use std::sync::Arc;

use crate::{
    cli::{RedditCategoryFilter, RedditTimeframeFilter},
    clients::api_types::reddit::submitted_response::RedditSubmittedResponse,
    utils::state::SharedState,
};
use reqwest::header::HeaderMap;
use thiserror::Error;
use tokio::sync::Mutex;
const MAX_SUBMISSIONS_PER_REQUEST: u32 = 500;

#[derive(Error, Debug)]
pub enum RedditParserError {
    #[error("ReqwestMiddleware error: {0}")]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("JSON deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Reddit returned a 404 Not Found error")]
    NotFound,
    #[error("Reddit returned a 429 Too Many Requests error")]
    TooManyRequests,
    #[error("Reddit returned a 403 Forbidden error")]
    Forbidden,
}

pub struct RedditClient {
    headers: HeaderMap,
}

impl Default for RedditClient {
    fn default() -> Self {
        let mut map: HeaderMap = reqwest::header::HeaderMap::new();
        map.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("Reddit-User-Analysis"),
        );

        Self { headers: map }
    }
}

impl RedditClient {
    fn gen_user_submitted_url(&self, user: &str, after: Option<&str>) -> String {
        match after {
            Some(after) => format!(
                "https://www.reddit.com/user/{}/submitted.json?limit={}&sort=top&t=all&after={}&raw_json=1",
                user, MAX_SUBMISSIONS_PER_REQUEST, after
            ),
            None => format!(
                "https://www.reddit.com/user/{}/submitted.json?limit={}&sort=top&t=all&raw_json=1",
                user, MAX_SUBMISSIONS_PER_REQUEST
            ),
        }
    }

    pub async fn get_user_submissions(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        user: &str,
        shared_state: &Arc<Mutex<SharedState>>,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditParserError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let url = match after {
                Some(after) => self.gen_user_submitted_url(user, Some(&after)),
                None => self.gen_user_submitted_url(user, None),
            };

            let res = client
                .get(&url)
                .headers(self.headers.to_owned())
                .send()
                .await
                .map_err(RedditParserError::ReqwestMiddleware)?;

            if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditParserError::TooManyRequests);
            }

            if res.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(RedditParserError::NotFound);
            }

            if res.status() == reqwest::StatusCode::FORBIDDEN {
                return Err(RedditParserError::Forbidden);
            }

            let mut res: RedditSubmittedResponse =
                res.json().await.map_err(RedditParserError::Reqwest)?;

            let file_cache = &shared_state.lock().await.file_cache;

            let non_downloaded = res
                .data
                .children
                .into_iter()
                .filter(|rc| !file_cache.files.iter().any(|f| f.id == rc.data.id))
                .collect::<Vec<_>>();
            res.data.children = non_downloaded;

            match res.data.children.is_empty() {
                true => {
                    // If we already have all posts from the last request in the file_cache
                    // We assume we already finished downloading all posts
                    break;
                }
                false => {
                    responses.push(res.to_owned());
                }
            }

            match res.data.after {
                Some(a) => {
                    after = Some(a);
                }
                None => {
                    break;
                }
            }
        }

        Ok(responses)
    }

    fn get_category_str(&self, category: &RedditCategoryFilter) -> String {
        match category {
            RedditCategoryFilter::Hot => "hot".to_string(),
            RedditCategoryFilter::New => "new".to_string(),
            RedditCategoryFilter::Top => "top".to_string(),
            RedditCategoryFilter::Rising => "rising".to_string(),
        }
    }

    fn get_timeframe_str(&self, timeframe: &RedditTimeframeFilter) -> String {
        match timeframe {
            RedditTimeframeFilter::Hour => "hour".to_string(),
            RedditTimeframeFilter::Day => "day".to_string(),
            RedditTimeframeFilter::Week => "week".to_string(),
            RedditTimeframeFilter::Month => "month".to_string(),
            RedditTimeframeFilter::Year => "year".to_string(),
            RedditTimeframeFilter::All => "all".to_string(),
        }
    }

    fn gen_subreddit_submitted_url(
        &self,
        subreddit: &str,
        after: Option<&str>,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
    ) -> String {
        let category = self.get_category_str(category);
        let timeframe = self.get_timeframe_str(timeframe);

        match after {
            Some(after) => format!(
                "https://www.reddit.com/r/{}/{}.json?limit=100&t={}&after={}&raw_json=1",
                subreddit, category, timeframe, after
            ),
            None => format!(
                "https://www.reddit.com/r/{}/{}.json?limit=100&t={}&raw_json=1",
                subreddit, category, timeframe
            ),
        }
    }

    pub async fn get_subreddit_submissions(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        subreddit: &str,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditParserError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let url = match after {
                Some(after) => {
                    self.gen_subreddit_submitted_url(subreddit, Some(&after), category, timeframe)
                }
                None => self.gen_subreddit_submitted_url(subreddit, None, category, timeframe),
            };

            let res: RedditSubmittedResponse = client
                .get(&url)
                .headers(self.headers.to_owned())
                .send()
                .await
                .map_err(RedditParserError::ReqwestMiddleware)?
                .json()
                .await
                .map_err(RedditParserError::Reqwest)?;

            responses.push(res.to_owned());

            match res.data.after {
                Some(a) => {
                    after = Some(a);
                }
                None => {
                    break;
                }
            }
        }

        Ok(responses)
    }

    fn gen_search_url(
        &self,
        term: &str,
        after: Option<&str>,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
    ) -> String {
        let category = self.get_category_str(category);
        let timeframe = self.get_timeframe_str(timeframe);

        match after {
            Some(after) => format!(
                "https://www.reddit.com/search.json?q={}&include_over_18=on&count=100&sort={}&t={}&after={}&raw_json=1",
                term, category, timeframe, after
            ),
            None => format!(
                "https://www.reddit.com/search.json?q={}&include_over_18=on&count=100&sort={}&t={}&raw_json=1",
                term, category, timeframe
            ),
        }
    }

    pub async fn get_reddit_search(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        term: &str,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditParserError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let url = match after {
                Some(after) => self.gen_search_url(term, Some(&after), category, timeframe),
                None => self.gen_search_url(term, None, category, timeframe),
            };

            let res: RedditSubmittedResponse = client
                .get(&url)
                .headers(self.headers.to_owned())
                .send()
                .await
                .map_err(RedditParserError::ReqwestMiddleware)?
                .json()
                .await
                .map_err(RedditParserError::Reqwest)?;

            responses.push(res.to_owned());

            match res.data.after {
                Some(a) => {
                    after = Some(a);
                }
                None => {
                    break;
                }
            }
        }

        Ok(responses)
    }
}
