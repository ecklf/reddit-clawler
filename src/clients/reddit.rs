use std::{collections::HashSet, sync::Arc};

use crate::{
    cli::{CliRedditCommand, CliSharedOptions, RedditCategoryFilter, RedditTimeframeFilter},
    clients::api_types::reddit::{
        submitted_response::RedditSubmittedResponse, user_about::RedditUserAbout,
    },
    http::RequestBuilderExt,
    utils::state::SharedState,
};
use thiserror::Error;
use tokio::sync::Mutex;
use wreq::header::HeaderMap;
const MAX_SUBMISSIONS_PER_REQUEST: u32 = 100;

#[derive(Error, Debug)]
pub enum RedditProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] wreq::Error),
    #[error("JSON deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Reddit returned a Not Found status")]
    NotFound,
    #[error("Reddit returned a Suspended status")]
    Suspended,
    #[error("Reddit returned a 429 Too Many Requests error")]
    TooManyRequests,
    #[error("Reddit requires authentication or returned a 403 Forbidden error")]
    Forbidden,
    #[error("Reddit requires authentication")]
    AuthenticationRequired,
}

fn is_reddit_login_url(url: &wreq::Uri) -> bool {
    url.path() == "/login/"
}

fn search_fallback_url(url: &str) -> Option<String> {
    let mut url = url::Url::parse(url).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.reddit.com")
        || url.path() != "/search.json"
    {
        return None;
    }
    url.set_host(Some("old.reddit.com")).ok()?;
    Some(url.into())
}

pub struct RedditClient {
    headers: HeaderMap,
}

impl Default for RedditClient {
    fn default() -> Self {
        let mut map = HeaderMap::new();
        // Browser identity comes from the shared transport's emulation profile.
        map.insert(
            wreq::header::CACHE_CONTROL,
            wreq::header::HeaderValue::from_static("no-cache"),
        );
        map.insert(
            wreq::header::PRAGMA,
            wreq::header::HeaderValue::from_static("no-cache"),
        );

        Self { headers: map }
    }
}

impl RedditClient {
    async fn fetch_search(
        &self,
        client: &wreq::Client,
        url: &str,
    ) -> Result<wreq::Response, RedditProviderError> {
        let response = client
            .get(url)
            .headers(self.headers.clone())
            .send_with_retry()
            .await?;

        // Reddit can block www search while allowing the same authenticated
        // JSON endpoint on old. Do not retry other statuses or unrelated hosts.
        if response.status() == wreq::StatusCode::FORBIDDEN {
            if let Some(fallback) = search_fallback_url(url) {
                drop(response);
                return Ok(client
                    .get(fallback)
                    .headers(self.headers.clone())
                    .send_with_retry()
                    .await?);
            }
        }
        Ok(response)
    }

    async fn seed_anonymous_session(
        &self,
        client: &wreq::Client,
    ) -> Result<(), RedditProviderError> {
        client
            .head("https://old.reddit.com/")
            .headers(self.headers.to_owned())
            .send_with_retry()
            .await
            .map_err(RedditProviderError::Http)?;
        Ok(())
    }

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
                user, MAX_SUBMISSIONS_PER_REQUEST, category, timeframe, after
            ),
            None => format!(
                "https://www.reddit.com/user/{}/submitted.json?include_over_18=on&limit={}&sort={}&t={}&raw_json=1",
                user, MAX_SUBMISSIONS_PER_REQUEST, category, timeframe
            ),
        }
    }

    pub async fn gen_user_about_url(
        &self,
        client: &wreq::Client,
        user: &str,
    ) -> Result<RedditUserAbout, RedditProviderError> {
        let res = client
            .get(format!(
                "https://www.reddit.com/user/{}/about.json?raw_json=1",
                user
            ))
            .headers(self.headers.to_owned())
            .send_with_retry()
            .await
            .map_err(RedditProviderError::Http)?;

        if res.status() == wreq::StatusCode::TOO_MANY_REQUESTS {
            return Err(RedditProviderError::TooManyRequests);
        }

        if res.status() == wreq::StatusCode::NOT_FOUND {
            return Err(RedditProviderError::NotFound);
        }

        if is_reddit_login_url(res.uri()) {
            return Err(RedditProviderError::AuthenticationRequired);
        }

        if res.status() == wreq::StatusCode::FORBIDDEN {
            return Err(RedditProviderError::Forbidden);
        }

        res.error_for_status()?
            .json::<RedditUserAbout>()
            .await
            .map_err(RedditProviderError::Http)
    }

    pub async fn get_user_submissions(
        &self,
        client: &wreq::Client,
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

        self.seed_anonymous_session(client).await?;

        loop {
            let url = match after {
                Some(after) => self.gen_user_submitted_url(user, Some(&after), category, timeframe),
                None => self.gen_user_submitted_url(user, None, category, timeframe),
            };

            let res = client
                .get(&url)
                .headers(self.headers.to_owned())
                .send_with_retry()
                .await
                .map_err(RedditProviderError::Http)?;

            if res.status() == wreq::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditProviderError::TooManyRequests);
            }

            if res.status() == wreq::StatusCode::NOT_FOUND {
                return Err(RedditProviderError::NotFound);
            }

            if is_reddit_login_url(res.uri()) {
                return Err(RedditProviderError::AuthenticationRequired);
            }

            if res.status() == wreq::StatusCode::FORBIDDEN {
                let about = self
                    .gen_user_about_url(client, user)
                    .await
                    .map_err(|_| RedditProviderError::Forbidden)?;

                match about.data.is_suspended {
                    true => return Err(RedditProviderError::Suspended),
                    false => return Err(RedditProviderError::Forbidden),
                }
            }

            let res: RedditSubmittedResponse = res
                .error_for_status()?
                .json()
                .await
                .map_err(RedditProviderError::Http)?;

            // Keep cached posts in the response so the caller can still update
            // metadata such as last_post_tz. The download phase filters already
            // downloaded items before attempting any downloads.
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

        let search_term = format!("author:{}", user);
        let mut search_responses = self
            .get_search_submissions_for_term(
                client,
                shared_state,
                &search_term,
                category,
                timeframe,
                options,
            )
            .await?;
        responses.append(&mut search_responses);

        let mut seen_ids = HashSet::new();
        for response in &mut responses {
            response
                .data
                .children
                .retain(|child| seen_ids.insert(child.data.id.clone()));
        }
        responses.retain(|response| !response.data.children.is_empty());

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
        client: &wreq::Client,
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

        self.seed_anonymous_session(client).await?;

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
                .send_with_retry()
                .await
                .map_err(RedditProviderError::Http)?;

            if res.status() == wreq::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditProviderError::TooManyRequests);
            }

            if res.status() == wreq::StatusCode::NOT_FOUND {
                return Err(RedditProviderError::NotFound);
            }

            if is_reddit_login_url(res.uri()) {
                return Err(RedditProviderError::AuthenticationRequired);
            }

            if res.status() == wreq::StatusCode::FORBIDDEN {
                return Err(RedditProviderError::Forbidden);
            }

            let mut res: RedditSubmittedResponse = res
                .error_for_status()?
                .json()
                .await
                .map_err(RedditProviderError::Http)?;

            // Skip filtering in update mode to get all posts for cache refresh
            if !options.update {
                let file_cache = &shared_state.lock().await.file_cache;

                // Filter out posts only if ALL items for that post ID are successfully downloaded.
                // This ensures gallery posts with some failed items are re-fetched.
                let non_downloaded = res
                    .data
                    .children
                    .into_iter()
                    .filter(|rc| {
                        let cached_items: Vec<_> = file_cache
                            .files
                            .iter()
                            .filter(|f| f.id == rc.data.id)
                            .collect();

                        // Keep the post if:
                        // 1. No items cached for this ID, OR
                        // 2. Any cached item has success == false
                        cached_items.is_empty() || cached_items.iter().any(|f| !f.success)
                    })
                    .collect::<Vec<_>>();
                res.data.children = non_downloaded;
            }

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
        let mut url = url::Url::parse("https://www.reddit.com/search.json").unwrap();
        let mut query = url.query_pairs_mut();
        query
            .append_pair("q", term)
            .append_pair("include_over_18", "on")
            .append_pair("limit", &MAX_SUBMISSIONS_PER_REQUEST.to_string())
            .append_pair("sort", &category.to_string())
            .append_pair("t", &timeframe.to_string());
        if let Some(after) = after {
            query.append_pair("after", after);
        }
        query.append_pair("raw_json", "1");
        drop(query);

        url.into()
    }

    pub async fn get_search_submissions(
        &self,
        client: &wreq::Client,
        shared_state: &Arc<Mutex<SharedState>>,
        cmd: &CliRedditCommand,
        options: &CliSharedOptions,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditProviderError> {
        let CliRedditCommand {
            resource: term,
            category,
            timeframe,
            ..
        } = cmd;

        self.get_search_submissions_for_term(
            client,
            shared_state,
            term,
            category,
            timeframe,
            options,
        )
        .await
    }

    async fn get_search_submissions_for_term(
        &self,
        client: &wreq::Client,
        shared_state: &Arc<Mutex<SharedState>>,
        term: &str,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
        options: &CliSharedOptions,
    ) -> Result<Vec<RedditSubmittedResponse>, RedditProviderError> {
        let mut responses: Vec<RedditSubmittedResponse> = Vec::new();
        let mut after: Option<String> = None;
        let mut request_count: u32 = 0;
        let CliSharedOptions { limit, .. } = options;

        self.seed_anonymous_session(client).await?;

        loop {
            let url = match after {
                Some(after) => self.gen_search_url(term, Some(&after), category, timeframe),
                None => self.gen_search_url(term, None, category, timeframe),
            };

            let res = self.fetch_search(client, &url).await?;

            if res.status() == wreq::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditProviderError::TooManyRequests);
            }

            if res.status() == wreq::StatusCode::NOT_FOUND {
                return Err(RedditProviderError::NotFound);
            }

            if is_reddit_login_url(res.uri()) {
                return Err(RedditProviderError::AuthenticationRequired);
            }

            if res.status() == wreq::StatusCode::FORBIDDEN {
                return Err(RedditProviderError::Forbidden);
            }

            let mut res: RedditSubmittedResponse = res
                .error_for_status()?
                .json()
                .await
                .map_err(RedditProviderError::Http)?;

            // Skip filtering in update mode to get all posts for cache refresh
            if !options.update {
                let file_cache = &shared_state.lock().await.file_cache;

                // Filter out posts only if ALL items for that post ID are successfully downloaded.
                // This ensures gallery posts with some failed items are re-fetched.
                let non_downloaded = res
                    .data
                    .children
                    .into_iter()
                    .filter(|rc| {
                        let cached_items: Vec<_> = file_cache
                            .files
                            .iter()
                            .filter(|f| f.id == rc.data.id)
                            .collect();

                        // Keep the post if:
                        // 1. No items cached for this ID, OR
                        // 2. Any cached item has success == false
                        cached_items.is_empty() || cached_items.iter().any(|f| !f.success)
                    })
                    .collect::<Vec<_>>();
                res.data.children = non_downloaded;
            }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_fallback_preserves_query_and_pagination() {
        let url = "https://www.reddit.com/search.json?q=author%3Aspez&include_over_18=on&limit=100&sort=new&t=all&after=t3_example&raw_json=1";
        assert_eq!(
            search_fallback_url(url).unwrap(),
            url.replacen("www.reddit.com", "old.reddit.com", 1)
        );
    }

    #[test]
    fn search_fallback_is_restricted_to_www_search() {
        for url in [
            "https://old.reddit.com/search.json?q=test",
            "https://example.com/search.json?q=test",
            "https://www.reddit.com/user/spez/submitted.json",
            "http://www.reddit.com/search.json?q=test",
        ] {
            assert!(search_fallback_url(url).is_none());
        }
    }

    #[test]
    fn detects_reddit_login_url() {
        let url = "https://old.reddit.com/login/?reason=lor2&dest=https%3A%2F%2Fold.reddit.com%2F"
            .parse::<wreq::Uri>()
            .unwrap();

        assert!(is_reddit_login_url(&url));
    }

    #[test]
    fn generates_user_submissions_url() {
        let client = RedditClient::default();

        let url = client.gen_user_submitted_url(
            "spez",
            None,
            &RedditCategoryFilter::New,
            &RedditTimeframeFilter::All,
        );

        assert_eq!(
            url,
            "https://www.reddit.com/user/spez/submitted.json?include_over_18=on&limit=100&sort=new&t=all&raw_json=1"
        );
    }

    #[test]
    fn generates_paginated_user_submissions_url() {
        let client = RedditClient::default();

        let url = client.gen_user_submitted_url(
            "spez",
            Some("t3_example"),
            &RedditCategoryFilter::Top,
            &RedditTimeframeFilter::Year,
        );

        assert_eq!(
            url,
            "https://www.reddit.com/user/spez/submitted.json?include_over_18=on&limit=100&sort=top&t=year&after=t3_example&raw_json=1"
        );
    }

    #[test]
    fn generates_author_search_url() {
        let client = RedditClient::default();

        let url = client.gen_search_url(
            "author:spez",
            None,
            &RedditCategoryFilter::New,
            &RedditTimeframeFilter::All,
        );

        assert_eq!(
            url,
            "https://www.reddit.com/search.json?q=author%3Aspez&include_over_18=on&limit=100&sort=new&t=all&raw_json=1"
        );
    }
}
