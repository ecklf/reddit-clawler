use std::sync::Arc;

use crate::{
    cli::{CliRedditCommand, CliSharedOptions, RedditCategoryFilter, RedditTimeframeFilter},
    clients::api_types::reddit::{
        submitted_response::RedditSubmittedResponse, user_about::RedditUserAbout,
    },
    utils::state::SharedState,
};
use reqwest::header::HeaderMap;
use thiserror::Error;
use tokio::sync::Mutex;
const MAX_SUBMISSIONS_PER_REQUEST: u32 = 100;

#[derive(Error, Debug)]
pub enum RedditProviderError {
    #[error("ReqwestMiddleware error: {0}")]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("JSON deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Reddit returned a Not Found status")]
    NotFound,
    #[error("Reddit returned a Suspended status")]
    Suspended,
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
    fn gen_user_submitted_url(
        &self,
        user: &str,
        after: Option<&str>,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
    ) -> String {
        let category = category.to_string();
        let timeframe = timeframe.to_string();

        match after {
            Some(after) => format!(
                "https://www.reddit.com/user/{}/submitted.json?include_over_18=on&limit={}&sort={}&t={}&after={}&raw_json=1",
                user, category, timeframe, MAX_SUBMISSIONS_PER_REQUEST, after
            ),
            None => format!(
                "https://www.reddit.com/user/{}/submitted.json?include_over_18=on&limit={}&sort={}&t={}&raw_json=1",
                user, category, timeframe, MAX_SUBMISSIONS_PER_REQUEST
            ),
        }
    }

    pub async fn gen_user_about_url(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        user: &str,
    ) -> Result<RedditUserAbout, RedditProviderError> {
        let res = client
            .get(format!(
                "https://www.reddit.com/user/{}/about.json?raw_json=1",
                user
            ))
            .headers(self.headers.to_owned())
            .send()
            .await
            .map_err(RedditProviderError::ReqwestMiddleware)?;

        if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(RedditProviderError::TooManyRequests);
        }

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(RedditProviderError::NotFound);
        }

        res.json::<RedditUserAbout>()
            .await
            .map_err(RedditProviderError::Reqwest)
    }

    pub async fn get_user_submissions(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        shared_state: &Arc<Mutex<SharedState>>,
        cmd: &CliRedditCommand,
        options: &CliSharedOptions,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditProviderError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;
        let mut request_count: u32 = 0;

        let CliRedditCommand {
            resource: user,
            category,
            timeframe,
            ..
        } = cmd;

        let CliSharedOptions { limit, .. } = options;

        loop {
            let url = match after {
                Some(after) => self.gen_user_submitted_url(user, Some(&after), category, timeframe),
                None => self.gen_user_submitted_url(user, None, category, timeframe),
            };

            let res = client
                .get(&url)
                .headers(self.headers.to_owned())
                .send()
                .await
                .map_err(RedditProviderError::ReqwestMiddleware)?;

            if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditProviderError::TooManyRequests);
            }

            if res.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(RedditProviderError::NotFound);
            }

            if res.status() == reqwest::StatusCode::FORBIDDEN {
                let about = self
                    .gen_user_about_url(client, user)
                    .await
                    .map_err(|_| RedditProviderError::Forbidden)?;

                match about.data.is_suspended {
                    true => return Err(RedditProviderError::Suspended),
                    false => return Err(RedditProviderError::Forbidden),
                }
            }

            let mut res: RedditSubmittedResponse =
                res.json().await.map_err(RedditProviderError::Reqwest)?;

            let file_cache = &shared_state.lock().await.file_cache;

            let non_downloaded = res
                .data
                .children
                .into_iter()
                .filter(|rc| !file_cache.files.iter().any(|f| f.id == rc.data.id))
                .collect::<Vec<_>>();
            res.data.children = non_downloaded;

            if !res.data.children.is_empty() {
                responses.push(res.to_owned());
            }

            request_count += 1;
            match res.data.after {
                Some(a) => {
                    // Skip downloading if limit is reached
                    if let Some(l) = limit {
                        if request_count >= *l {
                            break;
                        }
                    }
                    after = Some(a);
                }
                None => {
                    break;
                }
            }
        }

        Ok(responses)
    }

    fn gen_subreddit_submitted_url(
        &self,
        subreddit: &str,
        after: Option<&str>,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
    ) -> String {
        let category = category.to_string();
        let timeframe = timeframe.to_string();

        match after {
            Some(after) => format!(
                "https://www.reddit.com/r/{}/{}.json?include_over_18=on&limit=100&t={}&after={}&raw_json=1",
                subreddit, category, timeframe, after
            ),
            None => format!(
                "https://www.reddit.com/r/{}/{}.json?include_over_18=on&limit=100&t={}&raw_json=1",
                subreddit, category, timeframe
            ),
        }
    }

    pub async fn get_subreddit_submissions(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        shared_state: &Arc<Mutex<SharedState>>,
        cmd: &CliRedditCommand,
        options: &CliSharedOptions,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditProviderError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;
        let mut request_count: u32 = 0;

        let CliRedditCommand {
            resource: subreddit,
            category,
            timeframe,
            ..
        } = cmd;

        let CliSharedOptions { limit, .. } = options;

        loop {
            let url = match after {
                Some(after) => {
                    self.gen_subreddit_submitted_url(subreddit, Some(&after), category, timeframe)
                }
                None => self.gen_subreddit_submitted_url(subreddit, None, category, timeframe),
            };

            let res = client
                .get(&url)
                .headers(self.headers.to_owned())
                .send()
                .await
                .map_err(RedditProviderError::ReqwestMiddleware)?;

            if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditProviderError::TooManyRequests);
            }

            if res.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(RedditProviderError::NotFound);
            }

            if res.status() == reqwest::StatusCode::FORBIDDEN {
                return Err(RedditProviderError::Forbidden);
            }

            let mut res: RedditSubmittedResponse =
                res.json().await.map_err(RedditProviderError::Reqwest)?;

            let file_cache = &shared_state.lock().await.file_cache;

            let non_downloaded = res
                .data
                .children
                .into_iter()
                .filter(|rc| !file_cache.files.iter().any(|f| f.id == rc.data.id))
                .collect::<Vec<_>>();
            res.data.children = non_downloaded;

            if !res.data.children.is_empty() {
                responses.push(res.to_owned());
            }

            request_count += 1;
            match res.data.after {
                Some(a) => {
                    // Skip downloading if limit is reached
                    if let Some(l) = limit {
                        if request_count >= *l {
                            break;
                        }
                    }
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
        let category = category.to_string();
        let timeframe = timeframe.to_string();

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

    pub async fn get_search_submissions(
        &self,
        client: &reqwest_middleware::ClientWithMiddleware,
        shared_state: &Arc<Mutex<SharedState>>,
        cmd: &CliRedditCommand,
        options: &CliSharedOptions,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditProviderError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;
        let mut request_count: u32 = 0;

        let CliRedditCommand {
            resource: term,
            category,
            timeframe,
            ..
        } = cmd;

        let CliSharedOptions { limit, .. } = options;

        loop {
            let url = match after {
                Some(after) => self.gen_search_url(term, Some(&after), category, timeframe),
                None => self.gen_search_url(term, None, category, timeframe),
            };

            let res = client
                .get(&url)
                .headers(self.headers.to_owned())
                .send()
                .await
                .map_err(RedditProviderError::ReqwestMiddleware)?;

            if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditProviderError::TooManyRequests);
            }

            if res.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(RedditProviderError::NotFound);
            }

            if res.status() == reqwest::StatusCode::FORBIDDEN {
                return Err(RedditProviderError::Forbidden);
            }

            let mut res: RedditSubmittedResponse =
                res.json().await.map_err(RedditProviderError::Reqwest)?;

            let file_cache = &shared_state.lock().await.file_cache;

            let non_downloaded = res
                .data
                .children
                .into_iter()
                .filter(|rc| !file_cache.files.iter().any(|f| f.id == rc.data.id))
                .collect::<Vec<_>>();
            res.data.children = non_downloaded;

            if !res.data.children.is_empty() {
                responses.push(res.to_owned());
            }

            request_count += 1;
            match res.data.after {
                Some(a) => {
                    // Skip downloading if limit is reached
                    if let Some(l) = limit {
                        if request_count >= *l {
                            break;
                        }
                    }
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
