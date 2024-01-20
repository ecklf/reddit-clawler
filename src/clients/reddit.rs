use crate::{
    cli::{SubredditCategoryFilter, SubredditTimeframeFilter},
    clients::api_types::reddit::submitted_response::RedditSubmittedResponse,
};
use reqwest::header::HeaderMap;
use thiserror::Error;
const MAX_SUBMISSIONS_PER_REQUEST: u32 = 100;

#[derive(Error, Debug)]
pub enum RedditParserError {
    #[error("ReqwestMiddleware error: {0}")]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("JSON deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
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
                "https://www.reddit.com/user/{}/submitted.json?limit={}&sort=new&after={}&raw_json=1",
                user, MAX_SUBMISSIONS_PER_REQUEST, after
            ),
            None => format!(
                "https://www.reddit.com/user/{}/submitted.json?limit={}&sort=new&raw_json=1",
                user, MAX_SUBMISSIONS_PER_REQUEST
            ),
        }
    }

    pub async fn get_user_submissions(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        user: &str,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditParserError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let url = match after {
                Some(after) => self.gen_user_submitted_url(user, Some(&after)),
                None => self.gen_user_submitted_url(user, None),
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

    fn get_category_str(&self, category: &SubredditCategoryFilter) -> String {
        match category {
            SubredditCategoryFilter::Hot => "hot".to_string(),
            SubredditCategoryFilter::New => "new".to_string(),
            SubredditCategoryFilter::Top => "top".to_string(),
            SubredditCategoryFilter::Rising => "rising".to_string(),
        }
    }

    fn get_timeframe_str(&self, timeframe: &SubredditTimeframeFilter) -> String {
        match timeframe {
            SubredditTimeframeFilter::Hour => "hour".to_string(),
            SubredditTimeframeFilter::Day => "day".to_string(),
            SubredditTimeframeFilter::Week => "week".to_string(),
            SubredditTimeframeFilter::Month => "month".to_string(),
            SubredditTimeframeFilter::Year => "year".to_string(),
            SubredditTimeframeFilter::All => "all".to_string(),
        }
    }

    fn gen_subreddit_submitted_url(
        &self,
        subreddit: &str,
        after: Option<&str>,
        category: &SubredditCategoryFilter,
        timeframe: &SubredditTimeframeFilter,
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
        category: &SubredditCategoryFilter,
        timeframe: &SubredditTimeframeFilter,
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
}
