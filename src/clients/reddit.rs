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

mod graphql;

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
    #[error("Reddit requires authentication; sign in in a browser and export fresh Reddit cookies with --cookies")]
    AuthenticationRequired,
    #[error("Reddit returned a browser/JavaScript challenge instead of JSON; complete the challenge in a browser on this network, then export fresh Reddit cookies with --cookies")]
    BrowserChallenge,
    #[error(
        "Reddit returned HTML instead of JSON; refresh your browser session and exported cookies"
    )]
    UnexpectedHtml,
    #[error("Reddit GraphQL error: {0}")]
    GraphQl(&'static str),
    #[error("Reddit token renewal failed (imported token: {imported}; HEAD: {head_status}; proxy: {proxy_status}/{content}; token_v2 Set-Cookie: {token_set}; stored token: {stored}). No usable bearer token is available. If the export lacks a fresh token_v2, export it from the logged-in browser along with reddit_session; session-only renewal was not successful")]
    TokenRefresh {
        imported: &'static str,
        head_status: u16,
        proxy_status: u16,
        content: &'static str,
        token_set: bool,
        stored: &'static str,
    },
    #[error("Reddit GraphQL operation {operation} failed (HTTP {status}); the session may be blocked or the persisted query may have changed")]
    GraphQlRequest {
        operation: &'static str,
        status: u16,
    },
}

fn is_reddit_login_url(url: &wreq::Uri) -> bool {
    url.path() == "/login/"
}

fn json_fallback_url(url: &str) -> Option<String> {
    let mut url = url::Url::parse(url).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.reddit.com")
        || !url.path().ends_with(".json")
    {
        return None;
    }
    url.set_host(Some("old.reddit.com")).ok()?;
    Some(url.into())
}

fn is_reddit_challenge_url(uri: &wreq::Uri) -> bool {
    uri.query().is_some_and(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .any(|(name, value)| name == "js_challenge" && value == "1")
    })
}

fn is_html(response: &wreq::Response) -> bool {
    response
        .headers()
        .get(wreq::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value.trim().eq_ignore_ascii_case("text/html")
                || value.trim().eq_ignore_ascii_case("application/xhtml+xml")
        })
}

fn is_terminal_json_status(status: wreq::StatusCode) -> bool {
    matches!(
        status,
        wreq::StatusCode::NOT_FOUND | wreq::StatusCode::TOO_MANY_REQUESTS
    )
}

fn needs_json_fallback(response: &wreq::Response) -> bool {
    !is_terminal_json_status(response.status())
        && !is_reddit_login_url(response.uri())
        && (response.status() == wreq::StatusCode::FORBIDDEN
            || is_reddit_challenge_url(response.uri())
            || (response.status().is_success() && is_html(response)))
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
    async fn graphql_client<'a>(
        &self,
        client: &'a wreq::Client,
        shared_state: &Arc<Mutex<SharedState>>,
        options: &CliSharedOptions,
    ) -> Result<Option<graphql::GraphQlClient<'a>>, RedditProviderError> {
        if options.cookies.is_none() {
            return Ok(None);
        }
        let jar = shared_state.lock().await.reddit_cookie_jar.clone().ok_or(
            RedditProviderError::GraphQl("browser cookie jar is not configured"),
        )?;
        Ok(Some(graphql::GraphQlClient::new(client, jar)))
    }

    async fn filter_cached_graphql_posts(
        &self,
        responses: &mut Vec<RedditSubmittedResponse>,
        shared_state: &Arc<Mutex<SharedState>>,
        options: &CliSharedOptions,
    ) {
        if options.update || options.force {
            return;
        }
        let state = shared_state.lock().await;
        for response in responses.iter_mut() {
            response.data.children.retain(|child| {
                let cached: Vec<_> = state
                    .file_cache
                    .files
                    .iter()
                    .filter(|file| file.id == child.data.id)
                    .collect();
                cached.is_empty() || cached.iter().any(|file| !file.success)
            });
        }
        responses.retain(|response| !response.data.children.is_empty());
    }

    async fn fetch_json(
        &self,
        client: &wreq::Client,
        url: &str,
    ) -> Result<wreq::Response, RedditProviderError> {
        let fallback = json_fallback_url(url);
        self.fetch_json_with_fallback(client, url, fallback.as_deref())
            .await
    }

    async fn fetch_json_with_fallback(
        &self,
        client: &wreq::Client,
        url: &str,
        fallback: Option<&str>,
    ) -> Result<wreq::Response, RedditProviderError> {
        let mut response = client
            .get(url)
            .headers(self.headers.clone())
            .header(wreq::header::ACCEPT, "application/json")
            .send_with_retry()
            .await?;

        // Use the original JSON endpoint, not the redirected challenge URL.
        // The same jar is reused; host-only cookies must not be widened to old.
        if needs_json_fallback(&response) {
            if let Some(fallback) = fallback {
                drop(response);
                response = client
                    .get(fallback)
                    .headers(self.headers.clone())
                    .header(wreq::header::ACCEPT, "application/json")
                    .send_with_retry()
                    .await?;
            }
        }

        // Never turn a rate limit into an authentication/challenge error.
        if response.status() == wreq::StatusCode::TOO_MANY_REQUESTS {
            return Ok(response);
        }

        // Classify redirects before status/JSON handling. Challenge URLs can
        // include tokens, so don't embed them in errors or logs.
        if is_reddit_login_url(response.uri()) {
            return Err(RedditProviderError::AuthenticationRequired);
        }
        if is_reddit_challenge_url(response.uri()) {
            return Err(RedditProviderError::BrowserChallenge);
        }
        // An HTML 404 is not proof that a user was deleted: it may be an
        // interstitial/error page. Only let API 404s reach resource caching.
        if (response.status().is_success() || response.status() == wreq::StatusCode::NOT_FOUND)
            && is_html(&response)
        {
            return Err(RedditProviderError::UnexpectedHtml);
        }
        Ok(response)
    }

    async fn seed_anonymous_session(
        &self,
        client: &wreq::Client,
    ) -> Result<(), RedditProviderError> {
        // Prime the same host used for JSON requests, without downloading HTML.
        client
            .head("https://www.reddit.com/")
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
        let res = self
            .fetch_json(
                client,
                &format!("https://www.reddit.com/user/{}/about.json?raw_json=1", user),
            )
            .await?;

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
        if let Some(graphql) = self.graphql_client(client, shared_state, options).await? {
            return graphql.user_submissions(cmd).await;
        }
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

            let res = self.fetch_json(client, &url).await?;

            if res.status() == wreq::StatusCode::TOO_MANY_REQUESTS {
                return Err(RedditProviderError::TooManyRequests);
            }

            if res.status() == wreq::StatusCode::NOT_FOUND {
                return Err(RedditProviderError::NotFound);
            }

            if res.status() == wreq::StatusCode::FORBIDDEN {
                let about =
                    self.gen_user_about_url(client, user)
                        .await
                        .map_err(|error| match error {
                            RedditProviderError::BrowserChallenge
                            | RedditProviderError::UnexpectedHtml
                            | RedditProviderError::AuthenticationRequired => error,
                            _ => RedditProviderError::Forbidden,
                        })?;

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
        if let Some(graphql) = self.graphql_client(client, shared_state, options).await? {
            let mut responses = graphql.subreddit_submissions(cmd).await?;
            self.filter_cached_graphql_posts(&mut responses, shared_state, options)
                .await;
            return Ok(responses);
        }
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

            let res = self.fetch_json(client, &url).await?;

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

        if let Some(graphql) = self.graphql_client(client, shared_state, options).await? {
            let mut responses = graphql
                .search_submissions(term, category, timeframe, options.limit)
                .await?;
            self.filter_cached_graphql_posts(&mut responses, shared_state, options)
                .await;
            return Ok(responses);
        }

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

            let res = self.fetch_json(client, &url).await?;

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
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_fallback_preserves_endpoint_query_and_pagination() {
        for url in [
            "https://www.reddit.com/search.json?q=author%3Aspez&include_over_18=on&limit=100&sort=new&t=all&after=t3_example&raw_json=1",
            "https://www.reddit.com/user/spez/submitted.json?after=t3_example&raw_json=1",
            "https://www.reddit.com/user/spez/about.json?raw_json=1",
            "https://www.reddit.com/r/redpandas/new.json?after=t3_example&raw_json=1",
        ] {
            assert_eq!(
                json_fallback_url(url).unwrap(),
                url.replacen("www.reddit.com", "old.reddit.com", 1)
            );
        }
    }

    #[test]
    fn json_fallback_is_restricted_to_www_json_endpoints() {
        for url in [
            "https://old.reddit.com/search.json?q=test",
            "https://example.com/search.json?q=test",
            "https://www.reddit.com/",
            "https://www.reddit.com/login/",
            "http://www.reddit.com/search.json?q=test",
        ] {
            assert!(json_fallback_url(url).is_none());
        }
    }

    #[test]
    fn detects_challenge_query_without_matching_unrelated_values() {
        for (query, expected) in [
            ("solution=redacted&js_challenge=1&jsc_token=redacted", true),
            ("js_challenge=0", false),
            ("q=js_challenge%3D1", false),
            ("", false),
        ] {
            let uri = format!("https://www.reddit.com/?{query}").parse().unwrap();
            assert_eq!(is_reddit_challenge_url(&uri), expected);
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
