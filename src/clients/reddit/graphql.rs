//! Authenticated, read-only persisted GraphQL requests. Endpoints and protocol
//! shapes were checked against Hydra; no credentials or response bodies are logged.
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use wreq::{cookie::Jar, header, redirect::Policy, Client, Response, StatusCode};

use super::{is_reddit_challenge_url, is_reddit_login_url, RedditProviderError};
use crate::http::RequestBuilderExt;

mod feeds;
mod posts;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, RedditProviderError>;
const WEB_URL: &str = "https://www.reddit.com/";
const GQL_URL: &str = "https://cf.gql-fed.reddit.com/";
const EXPIRY_MARGIN: u64 = 300;

pub(super) struct GraphQlClient<'a> {
    client: &'a Client,
    jar: Arc<Jar>,
    web_url: String,
    gql_url: String,
    issued: Mutex<Option<IssuedToken>>,
}

struct IssuedToken {
    token: String,
    session: String,
    at: Instant,
}

impl<'a> GraphQlClient<'a> {
    pub(super) fn new(client: &'a Client, jar: Arc<Jar>) -> Self {
        Self {
            client,
            jar,
            web_url: WEB_URL.into(),
            gql_url: GQL_URL.into(),
            issued: Mutex::new(None),
        }
    }

    fn cookie(&self, name: &str) -> Option<String> {
        self.jar
            .matches(self.web_url.as_str())
            .find(|cookie| cookie.name() == name && !cookie.value().is_empty())
            .map(|cookie| cookie.value().trim_matches('"').to_owned())
    }

    async fn token(&self, rejected: Option<&str>) -> Result<String> {
        // Serializes refreshes. The jar is shared with the main HTTP client,
        // so renewal Set-Cookies are available to subsequent requests.
        let mut issued = self.issued.lock().await;
        let session = self
            .cookie("reddit_session")
            .ok_or(RedditProviderError::AuthenticationRequired)?;
        let imported = token_state(self.cookie("token_v2").as_deref());
        if let Some(token) = self.cookie("token_v2") {
            let locally_issued = issued
                .as_ref()
                .is_some_and(|last| last.token == token && last.session == session);
            // An absent/unrecognized expiry isn't proof of rejection. Let
            // Reddit validate that token rather than forcing renewal first.
            if usable(&token) && (locally_issued || for_session(&token, &session)) {
                if rejected != Some(token.as_str()) {
                    return Ok(token);
                }
                if issued.as_ref().is_some_and(|last| {
                    last.token == token && last.at.elapsed() < Duration::from_secs(60)
                }) {
                    return Err(RedditProviderError::GraphQl(
                        "Reddit rejected a newly issued token; complete any browser challenge and export a fresh logged-in session",
                    ));
                }
            }
        }

        // Clear every applicable scope, not the session cookie. Reddit only
        // issues token_v2 when the request doesn't already carry a valid one.
        let tokens: Vec<_> = self
            .jar
            .matches(format!("{}svc/shreddit/graphql", self.web_url))
            .filter(|cookie| cookie.name() == "token_v2")
            .collect();
        for cookie in tokens {
            // Jar::remove ignores the cookie's Domain/Path attributes and
            // removes by the URI's exact storage scope, not request matching.
            // A .reddit.com cookie must be removed at reddit.com, not www.
            let mut scope = url::Url::parse(&self.web_url)
                .map_err(|_| RedditProviderError::GraphQl("invalid web endpoint"))?;
            if let Some(domain) = cookie.domain() {
                scope
                    .set_host(Some(domain.trim_start_matches('.')))
                    .map_err(|_| RedditProviderError::GraphQl("invalid cookie scope"))?;
            }
            scope.set_path(cookie.path().unwrap_or("/"));
            self.jar.remove(cookie, scope.as_str());
        }
        let response = self
            .client
            .head(&self.web_url)
            .cookie_provider(self.jar.clone())
            .redirect(Policy::none())
            .header(header::CACHE_CONTROL, "no-store")
            .send_with_retry()
            .await?;
        // Priming is best-effort: Reddit can block the page while still
        // allowing the proxy to issue a token (Hydra also ignores this status).
        // A rate limit is the exception: do not immediately issue another request.
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(RedditProviderError::TooManyRequests);
        }
        if response.status().is_redirection() {
            check_web_response(&response, false)?;
        }
        let head_status = response.status().as_u16();
        let mut token_set = sets_token_cookie(&response);
        drop(response);

        // Since October 2026, HEAD/page loads no longer issue token_v2. An
        // empty proxy request returns 400 but still sets the token cookie.
        let response = self
            .client
            .post(format!("{}svc/shreddit/graphql", self.web_url))
            .cookie_provider(self.jar.clone())
            .redirect(Policy::none())
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::ACCEPT, "application/json")
            .json(&json!({}))
            .send()
            .await?;
        check_web_response(&response, true)?;
        let proxy_status = response.status().as_u16();
        let content = response_content(&response);
        token_set |= sets_token_cookie(&response);
        drop(response);

        let new_session = self
            .cookie("reddit_session")
            .ok_or(RedditProviderError::AuthenticationRequired)?;
        // HEAD may rotate the session value. Compare known account claims,
        // falling back to exact equality when both formats are opaque.
        if !same_account(&session, &new_session) {
            return Err(RedditProviderError::GraphQl(
                "account changed during token refresh",
            ));
        }
        let token = self.cookie("token_v2");
        let stored = token_state(token.as_deref());
        let token = token
            .filter(|token| usable(token))
            .filter(|token| token_set || rejected != Some(token.as_str()))
            .ok_or(RedditProviderError::TokenRefresh {
                imported,
                head_status,
                proxy_status,
                content,
                token_set,
                stored,
            })?;
        if !for_session(&token, &new_session) {
            return Err(RedditProviderError::GraphQl(
                "token belongs to a different account",
            ));
        }
        *issued = Some(IssuedToken {
            token: token.clone(),
            session: new_session,
            at: Instant::now(),
        });
        Ok(token)
    }

    async fn send(
        &self,
        operation: &'static str,
        hash: &'static str,
        variables: &Value,
        token: &str,
    ) -> Result<Response> {
        Ok(self
            .client
            .post(&self.gql_url)
            .cookie_provider(self.jar.clone())
            // Never follow redirects while carrying a bearer token.
            .redirect(Policy::none())
            .bearer_auth(token)
            .header(header::ACCEPT, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .json(&json!({
                "operationName": operation,
                "variables": variables,
                "extensions": {"persistedQuery": {"version": 1, "sha256Hash": hash}},
            }))
            .send()
            .await?)
    }

    async fn execute(
        &self,
        operation: &'static str,
        hash: &'static str,
        variables: Value,
    ) -> Result<Value> {
        let token = self.token(None).await?;
        let mut response = self.send(operation, hash, &variables, &token).await?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            drop(response);
            let token = self.token(Some(&token)).await?;
            response = self.send(operation, hash, &variables, &token).await?;
        }
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(RedditProviderError::TooManyRequests);
        }
        if !status.is_success() {
            return Err(RedditProviderError::GraphQlRequest {
                operation,
                status: status.as_u16(),
            });
        }
        // Do not include arbitrary server error strings/body data: they may
        // echo authentication material. Reject partial-data errors so a failed
        // feed is never interpreted as an empty or deleted resource.
        let body = response.bytes().await?;
        let envelope: Value = serde_json::from_slice(&body)
            .map_err(|_| RedditProviderError::GraphQl("response was not JSON"))?;
        if envelope
            .get("errors")
            .is_some_and(|errors| !matches!(errors.as_array(), Some(errors) if errors.is_empty()))
        {
            return Err(RedditProviderError::GraphQlRequest {
                operation,
                status: status.as_u16(),
            });
        }
        envelope
            .get("data")
            .filter(|data| data.is_object())
            .cloned()
            .ok_or(RedditProviderError::GraphQl("response has no data object"))
    }
}

fn claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn usable(token: &str) -> bool {
    matches!(token_state(Some(token)), "fresh" | "missing/invalid expiry")
}

// Only canonical labels escape this module. Never put cookie values, claims,
// raw headers, URLs or response bodies into a diagnostic.
fn token_state(token: Option<&str>) -> &'static str {
    let Some(token) = token else { return "missing" };
    let Some(claims) = claims(token) else {
        return "unparseable JWT";
    };
    // JWT NumericDate permits non-integer JSON numbers. as_u64 rejects e.g.
    // 1793764800.0 even though Hydra/JavaScript accepts it as a numeric expiry.
    let Some(expiry) = claims["exp"].as_f64().filter(|expiry| expiry.is_finite()) else {
        return "missing/invalid expiry";
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    if expiry <= now {
        "expired"
    } else if expiry <= now + EXPIRY_MARGIN as f64 {
        "expires within five minutes"
    } else {
        "fresh"
    }
}

fn sets_token_cookie(response: &Response) -> bool {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .any(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.split_once('='))
                .is_some_and(|(name, _)| name.trim() == "token_v2")
        })
}

fn response_content(response: &Response) -> &'static str {
    let content = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next());
    match content.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("application/json") => "JSON",
        Some(value)
            if value.eq_ignore_ascii_case("text/html")
                || value.eq_ignore_ascii_case("application/xhtml+xml") =>
        {
            "HTML"
        }
        None => "no content type",
        _ => "other content type",
    }
}

fn account(session: &str) -> Option<String> {
    claims(session)?.get("sub")?.as_str().map(str::to_owned)
}

fn for_session(token: &str, session: &str) -> bool {
    let token_account = claims(token).and_then(|claims| claims["lid"].as_str().map(str::to_owned));
    match (account(session), token_account) {
        (Some(session), Some(token)) => session == token,
        // Decoded claims are only local hints, not authentication. When the
        // format is unknown, Reddit remains responsible for validating it.
        _ => true,
    }
}

fn same_account(before: &str, after: &str) -> bool {
    match (account(before), account(after)) {
        (Some(before), Some(after)) => before == after,
        _ => before == after,
    }
}

fn check_web_response(response: &Response, allow_bad_request: bool) -> Result<()> {
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        return Err(RedditProviderError::TooManyRequests);
    }
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<wreq::Uri>().ok());
    let uri = location.as_ref().unwrap_or(response.uri());
    if is_reddit_login_url(uri) || response.status() == StatusCode::UNAUTHORIZED {
        return Err(RedditProviderError::AuthenticationRequired);
    }
    if is_reddit_challenge_url(uri) {
        return Err(RedditProviderError::BrowserChallenge);
    }
    if response.status() == StatusCode::FORBIDDEN {
        return Err(RedditProviderError::GraphQl("Reddit blocked session-token renewal; complete the browser challenge and export fresh cookies"));
    }
    if response.status().is_success()
        || (allow_bad_request && response.status() == StatusCode::BAD_REQUEST)
    {
        return Ok(());
    }
    Err(RedditProviderError::GraphQl("session-token renewal failed"))
}
