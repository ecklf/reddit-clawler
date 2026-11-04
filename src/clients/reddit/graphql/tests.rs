//! Loopback-only protocol tests. No Reddit credentials or external services are used.
use super::*;
use crate::{
    cli::{CliRedditCommand, CliSharedOptions, RedditCategoryFilter, RedditTimeframeFilter},
    reddit_parser::RedditPostParser,
};
use std::{collections::VecDeque, future::Future};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};

const TIMEOUT: Duration = Duration::from_secs(5);
const OPERATION: &str = "TestPersistedOperation";
const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    timeout(TIMEOUT, future)
        .await
        .expect("loopback test timed out")
}

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn cookie(&self, name: &str) -> Option<&str> {
        self.header("cookie")?
            .split(';')
            .filter_map(|part| part.trim().split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("complete JSON request body")
    }
}

async fn read_request(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 2048];
        let count = stream.read(&mut buffer).await.unwrap();
        assert_ne!(count, 0, "connection closed before headers");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 1024 * 1024, "request too large");
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_owned();
    let path = first.next().unwrap().to_owned();
    let headers: Vec<_> = lines
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (key, value) = line.split_once(':').unwrap();
            (key.to_owned(), value.trim().to_owned())
        })
        .collect();
    assert!(
        !headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("transfer-encoding")),
        "these JSON requests should use Content-Length, not chunked encoding"
    );
    let length: usize = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.parse().unwrap())
        .unwrap_or(0);
    assert!(length <= 1024 * 1024);
    while bytes.len() < header_end + length {
        let mut buffer = [0; 2048];
        let count = stream.read(&mut buffer).await.unwrap();
        assert_ne!(count, 0, "connection closed before complete body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(bytes.len(), header_end + length);
    Request {
        method,
        path,
        headers,
        body: bytes[header_end..].to_vec(),
    }
}

struct Server {
    base: String,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Vec<Request>>,
}

impl Server {
    async fn start(responses: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            bounded(async move {
                let mut responses = VecDeque::from(responses);
                let mut requests = Vec::new();
                loop {
                    let (mut stream, _) = tokio::select! {
                        accepted = listener.accept() => accepted.unwrap(),
                        _ = &mut stopped => break,
                    };
                    requests.push(read_request(&mut stream).await);
                    // Keep accepting until the operation completes: unexpected retries are
                    // recorded and answered, not hidden by a refused connection.
                    let response = responses.pop_front().unwrap_or_else(|| {
                        response("418 Unexpected Request", "", "unexpected replay")
                    });
                    stream.write_all(&response).await.unwrap();
                    stream.shutdown().await.unwrap();
                }
                assert!(responses.is_empty(), "not all scripted requests were made");
                requests
            })
            .await
        });
        Self { base, stop, task }
    }

    async fn finish(self, targets: &[(&str, &str)]) -> Vec<Request> {
        self.stop.send(()).expect("mock server stopped early");
        let requests = bounded(self.task).await.unwrap();
        let actual: Vec<_> = requests
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(actual, targets, "unexpected refresh/replay/JSON endpoint");
        requests
    }
}

fn response(status: &str, headers: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn data(value: Value) -> Vec<u8> {
    response(
        "200 OK",
        "Content-Type: application/json\r\n",
        &json!({"data": value}).to_string(),
    )
}

fn head() -> Vec<u8> {
    response("200 OK", "", "")
}

fn issued(token: &str) -> Vec<u8> {
    response(
        "400 Bad Request",
        &format!("Set-Cookie: token_v2={token}; Path=/; HttpOnly\r\n"),
        "{}",
    )
}

fn jwt(payload: Value) -> String {
    format!(
        "e30.{}.",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    )
}

fn token(account: &str) -> String {
    jwt(json!({"exp": 4_102_444_800_u64, "lid": account}))
}

fn session(account: &str, nonce: &str) -> String {
    jwt(json!({"sub": account, "nonce": nonce}))
}

fn http_client(jar: Arc<Jar>) -> Client {
    Client::builder()
        .no_proxy()
        .cookie_provider(jar)
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .retry(wreq::retry::Policy::never())
        // Deliberately allow client-level redirects; the GraphQL requests must
        // override this policy before attaching any bearer/session credentials.
        .redirect(Policy::limited(5))
        .build()
        .unwrap()
}

fn graphql<'a>(
    client: &'a Client,
    jar: Arc<Jar>,
    base: &str,
    session: Option<&str>,
    token: Option<&str>,
) -> GraphQlClient<'a> {
    if let Some(session) = session {
        jar.add(format!("reddit_session={session}; Path=/"), base);
    }
    if let Some(token) = token {
        jar.add(format!("token_v2={token}; Path=/"), base);
    }
    let mut gql = GraphQlClient::new(client, jar);
    gql.web_url = format!("{base}/");
    gql.gql_url = format!("{base}/gql");
    gql
}

async fn execute(gql: &GraphQlClient<'_>) -> Result<Value> {
    bounded(gql.execute(
        OPERATION,
        HASH,
        json!({"after": " opaque+/== ", "ids": ["t3_abc"], "nested": {"enabled": true}}),
    ))
    .await
}

fn assert_operation(request: &Request, bearer: &str) {
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {bearer}").as_str())
    );
    assert_eq!(request.header("accept"), Some("application/json"));
    assert_eq!(
        request.json(),
        json!({
            "operationName": OPERATION,
            "variables": {"after": " opaque+/== ", "ids": ["t3_abc"], "nested": {"enabled": true}},
            "extensions": {"persistedQuery": {"version": 1, "sha256Hash": HASH}},
        })
    );
}

fn assert_refresh(requests: &[Request], expected_session: &str) {
    assert_eq!(requests[0].method, "HEAD");
    assert_eq!(requests[0].path, "/");
    assert!(requests[0].body.is_empty());
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/svc/shreddit/graphql");
    assert_eq!(requests[1].json(), json!({}));
    for request in &requests[..2] {
        assert_eq!(
            request.cookie("token_v2"),
            None,
            "stale token sent during renewal"
        );
        assert_eq!(request.cookie("reddit_session"), Some(expected_session));
        assert_eq!(request.header("authorization"), None);
        assert_eq!(request.header("cache-control"), Some("no-store"));
    }
}

#[tokio::test]
async fn blocked_page_priming_can_still_obtain_a_proxy_token() {
    let bearer = token("alice");
    let server = Server::start(vec![
        response("403 Forbidden", "", "page blocked"),
        issued(&bearer),
        data(json!({"ok": true})),
    ])
    .await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let gql = graphql(
        &client,
        jar,
        &server.base,
        Some(&session("alice", "imported")),
        None,
    );
    assert_eq!(execute(&gql).await.unwrap(), json!({"ok": true}));
    let requests = server
        .finish(&[
            ("HEAD", "/"),
            ("POST", "/svc/shreddit/graphql"),
            ("POST", "/gql"),
        ])
        .await;
    assert_operation(&requests[2], &bearer);
}

#[tokio::test]
async fn fresh_imported_token_is_reused_without_head_or_proxy() {
    let server = Server::start(vec![data(json!({"ok": true})), data(json!({"ok": true}))]).await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let token = token("alice");
    let session = session("alice", "imported");
    let gql = graphql(&client, jar, &server.base, Some(&session), Some(&token));
    for _ in 0..2 {
        assert_eq!(execute(&gql).await.unwrap(), json!({"ok": true}));
    }
    assert!(
        gql.issued.lock().await.is_none(),
        "imported token unexpectedly renewed"
    );
    let requests = server.finish(&[("POST", "/gql"), ("POST", "/gql")]).await;
    for request in requests {
        assert_operation(&request, &token);
        assert_eq!(request.cookie("reddit_session"), Some(session.as_str()));
    }
}

#[tokio::test]
async fn fractional_and_unknown_expiry_tokens_are_validated_by_reddit_before_renewal() {
    for payload in [
        json!({"exp": 4_102_444_800.0_f64, "lid": "alice"}),
        json!({"exp": 4_102_444_800.5_f64, "lid": "alice"}),
        json!({"lid": "alice"}),
        json!({"exp": "unrecognized-format"}),
    ] {
        let bearer = jwt(payload);
        let server =
            Server::start(vec![data(json!({"ok": true})), data(json!({"ok": true}))]).await;
        let jar = Arc::new(Jar::default());
        let client = http_client(jar.clone());
        let gql = graphql(
            &client,
            jar,
            &server.base,
            Some(&session("alice", "imported")),
            Some(&bearer),
        );
        for _ in 0..2 {
            assert_eq!(execute(&gql).await.unwrap(), json!({"ok": true}));
        }
        let requests = server.finish(&[("POST", "/gql"); 2]).await;
        for request in &requests {
            assert_operation(request, &bearer);
        }
    }
}

#[tokio::test]
async fn renewal_removes_domain_host_and_proxy_path_tokens_but_keeps_session() {
    let new = token("alice");
    let server = Server::start(vec![head(), issued(&new), data(json!({"ok": true}))]).await;
    let address: std::net::SocketAddr = server
        .base
        .strip_prefix("http://")
        .unwrap()
        .parse()
        .unwrap();
    let base = format!("http://www.reddit.test:{}", address.port());
    let jar = Arc::new(Jar::default());
    let stale = jwt(json!({"exp": 1, "lid": "alice"}));
    for scope in [
        "Domain=reddit.test; Path=/",
        "Path=/",
        "Domain=reddit.test; Path=/svc/shreddit",
    ] {
        jar.add(format!("token_v2={stale}; {scope}"), base.as_str());
    }
    jar.add("keep=unchanged; Domain=reddit.test; Path=/", base.as_str());
    let client = Client::builder()
        .no_proxy()
        .resolve("www.reddit.test", address)
        .cookie_provider(jar.clone())
        .timeout(TIMEOUT)
        .build()
        .unwrap();
    let session = session("alice", "imported");
    let gql = graphql(&client, jar.clone(), &base, Some(&session), None);
    assert_eq!(execute(&gql).await.unwrap(), json!({"ok": true}));
    let requests = server
        .finish(&[
            ("HEAD", "/"),
            ("POST", "/svc/shreddit/graphql"),
            ("POST", "/gql"),
        ])
        .await;
    assert_refresh(&requests, &session);
    assert_operation(&requests[2], &new);
    for request in &requests {
        assert_eq!(request.cookie("keep"), Some("unchanged"));
    }
    assert_eq!(
        jar.matches(format!("{base}/svc/shreddit/graphql"))
            .filter(|c| c.name() == "token_v2")
            .count(),
        1
    );
}

#[tokio::test]
async fn unknown_expiry_token_rejection_still_refreshes_once() {
    let old = jwt(json!({"lid": "alice"}));
    let new = token("alice");
    let server = Server::start(vec![
        response("401 Unauthorized", "", ""),
        head(),
        issued(&new),
        data(json!({"ok": true})),
    ])
    .await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let session = session("alice", "imported");
    let gql = graphql(&client, jar, &server.base, Some(&session), Some(&old));
    assert_eq!(execute(&gql).await.unwrap(), json!({"ok": true}));
    let requests = server
        .finish(&[
            ("POST", "/gql"),
            ("HEAD", "/"),
            ("POST", "/svc/shreddit/graphql"),
            ("POST", "/gql"),
        ])
        .await;
    assert_operation(&requests[0], &old);
    assert_refresh(&requests[1..], &session);
    assert_operation(&requests[3], &new);
}

#[tokio::test]
async fn missing_expired_and_wrong_account_tokens_are_renewed_without_old_cookie() {
    for old in [
        None,
        Some(jwt(json!({"exp": 1, "lid": "alice"}))),
        Some(token("mallory")),
    ] {
        let new = token("alice");
        let server = Server::start(vec![head(), issued(&new), data(json!({"ok": true}))]).await;
        let jar = Arc::new(Jar::default());
        let client = http_client(jar.clone());
        let session = session("alice", "imported");
        let gql = graphql(&client, jar, &server.base, Some(&session), old.as_deref());
        assert_eq!(execute(&gql).await.unwrap(), json!({"ok": true}));
        assert_eq!(gql.cookie("token_v2").as_deref(), Some(new.as_str()));
        let requests = server
            .finish(&[
                ("HEAD", "/"),
                ("POST", "/svc/shreddit/graphql"),
                ("POST", "/gql"),
            ])
            .await;
        assert_refresh(&requests, &session);
        assert_operation(&requests[2], &new);
    }
}

#[tokio::test]
async fn token_renewal_failure_reports_only_safe_metadata() {
    let secret = "PRIVATE_COOKIE_OR_BODY_DO_NOT_PRINT";
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for (status, content, cookie, expected_state, token_set) in [
        (
            "400 Bad Request",
            "application/json",
            String::new(),
            "missing",
            false,
        ),
        ("200 OK", "text/html", String::new(), "missing", false),
        (
            "400 Bad Request",
            "application/json",
            format!("token_v2={secret}; Path=/"),
            "unparseable JWT",
            true,
        ),
        (
            "400 Bad Request",
            "application/json",
            format!("token_v2={}; Path=/", jwt(json!({"exp": 1}))),
            "expired",
            true,
        ),
        (
            "400 Bad Request",
            "application/json",
            format!("token_v2={}; Path=/", jwt(json!({"exp": now + 60}))),
            "expires within five minutes",
            true,
        ),
        (
            "400 Bad Request",
            "application/json",
            format!("token_v2={secret}; Domain=invalid.example; Path=/"),
            "missing",
            true,
        ),
    ] {
        let mut headers = format!("Content-Type: {content}; charset=utf-8\r\n");
        if !cookie.is_empty() {
            headers.push_str(&format!("Set-Cookie: {cookie}\r\n"));
        }
        let server = Server::start(vec![head(), response(status, &headers, secret)]).await;
        let jar = Arc::new(Jar::default());
        let client = http_client(jar.clone());
        let gql = graphql(
            &client,
            jar,
            &server.base,
            Some(&session("alice", "imported")),
            None,
        );
        let error = execute(&gql).await.unwrap_err();
        let expected_status = if status.starts_with("200") { 200 } else { 400 };
        let expected_content = if content == "text/html" {
            "HTML"
        } else {
            "JSON"
        };
        assert!(matches!(error, RedditProviderError::TokenRefresh {
            imported: "missing", head_status: 200, proxy_status,
            content: actual_content, token_set: actual_set, stored
        } if proxy_status == expected_status && actual_content == expected_content
            && actual_set == token_set && stored == expected_state));
        let rendered = format!("{error}\n{error:?}");
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains(&server.base));
        if !cookie.is_empty() {
            assert!(!rendered.contains(&cookie));
        }
        server
            .finish(&[("HEAD", "/"), ("POST", "/svc/shreddit/graphql")])
            .await;
    }
}

#[tokio::test]
async fn imported_401_refreshes_once_and_replays_identical_persisted_operation() {
    let old = token("alice");
    let new = jwt(json!({"exp": 4_102_444_800_u64, "lid": "alice", "generation": 2}));
    let server = Server::start(vec![
        response("401 Unauthorized", "", "secret rejected body"),
        head(),
        issued(&new),
        data(json!({"ok": true})),
    ])
    .await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let session = session("alice", "imported");
    let gql = graphql(&client, jar, &server.base, Some(&session), Some(&old));
    assert_eq!(execute(&gql).await.unwrap(), json!({"ok": true}));
    let requests = server
        .finish(&[
            ("POST", "/gql"),
            ("HEAD", "/"),
            ("POST", "/svc/shreddit/graphql"),
            ("POST", "/gql"),
        ])
        .await;
    assert_operation(&requests[0], &old);
    assert_refresh(&requests[1..3], &session);
    assert_operation(&requests[3], &new);
    assert_eq!(requests[0].body, requests[3].body);
}

#[tokio::test]
async fn newly_issued_403_is_bounded_without_another_refresh_or_replay() {
    for token in [token("alice"), jwt(json!({"lid": "alice"}))] {
        let server = Server::start(vec![
            head(),
            issued(&token),
            response("403 Forbidden", "", "secret blocked body"),
        ])
        .await;
        let jar = Arc::new(Jar::default());
        let client = http_client(jar.clone());
        let session = session("alice", "imported");
        let gql = graphql(&client, jar, &server.base, Some(&session), None);
        assert!(
            matches!(execute(&gql).await, Err(RedditProviderError::GraphQl(message)) if message.contains("newly issued"))
        );
        let requests = server
            .finish(&[
                ("HEAD", "/"),
                ("POST", "/svc/shreddit/graphql"),
                ("POST", "/gql"),
            ])
            .await;
        assert_refresh(&requests, &session);
        assert_operation(&requests[2], &token);
    }
}

#[tokio::test]
async fn missing_session_never_attempts_network_even_with_fresh_token() {
    let server = Server::start(vec![]).await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let gql = graphql(&client, jar, &server.base, None, Some(&token("alice")));
    assert!(matches!(
        execute(&gql).await,
        Err(RedditProviderError::AuthenticationRequired)
    ));
    server.finish(&[]).await;
}

#[tokio::test]
async fn graphql_429_does_not_refresh_or_replay() {
    let server = Server::start(vec![response(
        "429 Too Many Requests",
        "Retry-After: 0\r\n",
        "secret rate limit body",
    )])
    .await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let token = token("alice");
    let gql = graphql(
        &client,
        jar,
        &server.base,
        Some(&session("alice", "imported")),
        Some(&token),
    );
    assert!(matches!(
        execute(&gql).await,
        Err(RedditProviderError::TooManyRequests)
    ));
    let requests = server.finish(&[("POST", "/gql")]).await;
    assert_operation(&requests[0], &token);
}

#[tokio::test]
async fn graphql_errors_and_non_json_never_echo_server_secrets() {
    let secret = "SERVER_BODY_SECRET_DO_NOT_ECHO";
    let token = token("alice");
    let session = session("alice", "private-session");
    for body in [
        json!({"data": {"partial": true}, "errors": [{"message": format!("{secret} {token} {session}"), "extensions": {"cookie": session}}]}).to_string(),
        format!("<html>{secret} Bearer {token} reddit_session={session}</html>"),
    ] {
        let is_json = body.starts_with('{');
        let server = Server::start(vec![response("200 OK", "Content-Type: application/json\r\n", &body)]).await;
        let jar = Arc::new(Jar::default());
        let client = http_client(jar.clone());
        let gql = graphql(&client, jar, &server.base, Some(&session), Some(&token));
        let error = execute(&gql).await.unwrap_err();
        if is_json {
            assert!(matches!(error, RedditProviderError::GraphQlRequest { operation: OPERATION, status: 200 }));
        } else {
            assert!(matches!(error, RedditProviderError::GraphQl("response was not JSON")));
        }
        let rendered = format!("{error}\n{error:?}");
        for private in [secret, token.as_str(), session.as_str(), body.as_str()] {
            assert!(!rendered.contains(private), "server-controlled secret leaked");
        }
        server.finish(&[("POST", "/gql")]).await;
    }
}

#[tokio::test]
async fn auth_redirects_never_follow_to_an_arbitrary_origin() {
    for refreshing in [false, true] {
        // A separate loopback origin stands in for an attacker-controlled host.
        // Following it fails the request-count assertion without real networking.
        let destination = Server::start(vec![]).await;
        let server = Server::start(vec![response(
            "302 Found",
            &format!("Location: {}/login/\r\n", destination.base),
            "",
        )])
        .await;
        let jar = Arc::new(Jar::default());
        let client = http_client(jar.clone());
        let token = token("alice");
        let gql = graphql(
            &client,
            jar,
            &server.base,
            Some(&session("alice", "imported")),
            if refreshing { None } else { Some(&token) },
        );
        let error = execute(&gql).await.unwrap_err();
        if refreshing {
            assert!(matches!(error, RedditProviderError::AuthenticationRequired));
            server.finish(&[("HEAD", "/")]).await;
        } else {
            assert!(matches!(
                error,
                RedditProviderError::GraphQlRequest { status: 302, .. }
            ));
            server.finish(&[("POST", "/gql")]).await;
        }
        destination.finish(&[]).await;
    }
}

#[tokio::test]
async fn rotated_session_with_same_known_sub_is_accepted_and_account_changes_rejected() {
    for (rotated_account, token_account, expected) in [
        ("alice", "alice", None),
        (
            "mallory",
            "mallory",
            Some("account changed during token refresh"),
        ),
        (
            "alice",
            "mallory",
            Some("token belongs to a different account"),
        ),
    ] {
        let before = session("alice", "before");
        let after = session(rotated_account, "after");
        let token = token(token_account);
        let mut responses = vec![
            response(
                "200 OK",
                &format!("Set-Cookie: reddit_session={after}; Path=/; HttpOnly\r\n"),
                "",
            ),
            issued(&token),
        ];
        if expected.is_none() {
            responses.push(data(json!({"ok": true})));
        }
        let server = Server::start(responses).await;
        let jar = Arc::new(Jar::default());
        let client = http_client(jar.clone());
        let gql = graphql(&client, jar, &server.base, Some(&before), None);
        let result = execute(&gql).await;
        if let Some(expected) = expected {
            assert!(
                matches!(result, Err(RedditProviderError::GraphQl(message)) if message == expected)
            );
        } else {
            assert_eq!(result.unwrap(), json!({"ok": true}));
        }
        let targets = if expected.is_none() {
            vec![
                ("HEAD", "/"),
                ("POST", "/svc/shreddit/graphql"),
                ("POST", "/gql"),
            ]
        } else {
            vec![("HEAD", "/"), ("POST", "/svc/shreddit/graphql")]
        };
        let requests = server.finish(&targets).await;
        assert_eq!(requests[0].cookie("reddit_session"), Some(before.as_str()));
        assert_eq!(requests[1].cookie("reddit_session"), Some(after.as_str()));
        assert_eq!(requests[1].json(), json!({}));
        for request in &requests[..2] {
            assert_eq!(request.cookie("token_v2"), None);
        }
        if expected.is_none() {
            assert_operation(&requests[2], &token);
        }
    }
}

fn command(limit: Option<u32>) -> CliRedditCommand {
    CliRedditCommand {
        resource: "Alice".into(),
        category: RedditCategoryFilter::New,
        timeframe: RedditTimeframeFilter::All,
        options: CliSharedOptions {
            concurrency: 1,
            mock: None,
            output: String::new(),
            cookies: None,
            skip: false,
            verbose: false,
            limit,
            force: false,
            update: false,
        },
    }
}

fn connection(nodes: Vec<Value>, cursor: Option<&str>) -> Value {
    json!({"edges": nodes.into_iter().map(|node| json!({"node": node})).collect::<Vec<_>>(),
        "pageInfo": {"hasNextPage": cursor.is_some(), "endCursor": cursor}})
}

fn image_post(id: &str) -> Value {
    json!({"__typename": "SubredditPost", "id": id, "createdAt": "2026-10-04T00:26:11Z",
        "postTitle": "Synthetic image", "subreddit": {"name": "pics"}, "authorInfo": {"name": "Alice"},
        "score": 42, "url": "https://i.redd.it/synthetic.jpg",
        "media": {"still": {"source": {"url": "https://i.redd.it/synthetic.jpg", "dimensions": {"width": 640, "height": 480}}}}})
}

fn assert_feed_operation(request: &Request, operation: &str, hash: &str) -> Value {
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/gql");
    let body = request.json();
    assert_eq!(body["operationName"], operation);
    assert_eq!(
        body["extensions"],
        json!({"persistedQuery": {"version": 1, "sha256Hash": hash}})
    );
    assert_eq!(body.as_object().unwrap().len(), 3);
    body["variables"].clone()
}

#[tokio::test]
async fn user_submissions_paginates_ad_only_sdui_hydrates_and_supplements_without_json_endpoints() {
    let cursor = " opaque+/==:SDUI cursor ";
    let search_cursor = " search+/==:cursor ";
    let server = Server::start(vec![
        data(json!({"redditorInfoByName": {"__typename": "Redditor"}})),
        data(json!({"profileFeed": {"elements": connection(vec![json!({"id": "t3_ad", "adPayload": {}})], Some(cursor))}})),
        data(json!({"profileFeed": {"elements": connection(vec![json!({"groupId": "t3_one"}), json!({"groupId": "t3_one"}), json!({"id": "t3_two"})], None)}})),
        data(json!({"postsInfoByIds": [image_post("t3_one"), image_post("t3_two")]})),
        data(json!({"search": {"general": {"posts": connection(vec![image_post("t3_two"), image_post("t3_three")], Some(search_cursor))}}})),
        data(json!({"search": {"general": {"posts": connection(vec![image_post("t3_three"), image_post("t3_four")], None)}}})),
    ]).await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let token = token("alice");
    let gql = graphql(
        &client,
        jar,
        &server.base,
        Some(&session("alice", "imported")),
        Some(&token),
    );
    let pages = bounded(gql.user_submissions(&command(None))).await.unwrap();
    let requests = server.finish(&[("POST", "/gql"); 6]).await;
    for request in &requests {
        assert_eq!(
            request.header("authorization"),
            Some(format!("Bearer {token}").as_str())
        );
    }
    let profile = assert_feed_operation(
        &requests[0],
        "UserProfile",
        "a05c628c5469c6b107c85988a8b1d0b79a2e7b6150ebd3a449c67e226e975ddc",
    );
    assert_eq!(
        profile,
        json!({"name": "Alice", "includePremiumAvatarTreatment": false})
    );
    let first = assert_feed_operation(
        &requests[1],
        "SubmittedPostsFeedSdui",
        "d973adb1955e30c12e200bd2dbe843172276aebbae0ff1a8dda76f0ad19a0605",
    );
    let second = assert_feed_operation(
        &requests[2],
        "SubmittedPostsFeedSdui",
        "d973adb1955e30c12e200bd2dbe843172276aebbae0ff1a8dda76f0ad19a0605",
    );
    assert_eq!(
        first,
        json!({"username": "Alice", "sort": "NEW", "time": "ALL"})
    );
    let mut continued = first;
    continued["after"] = json!(cursor);
    assert_eq!(second, continued);
    let hydration = assert_feed_operation(
        &requests[3],
        "PostsByIds",
        "b210ea1d852cdc2c25d9bff2d9fa8a406e1a968f36392798812f336694ac27f4",
    );
    assert_eq!(hydration["ids"], json!(["t3_one", "t3_two"]));
    assert_eq!(hydration["includeAwards"], true);
    assert_eq!(hydration["includeDeletedPosts"], true);
    let search = assert_feed_operation(
        &requests[4],
        "SearchPosts",
        "c3d31b88bfa15180fdcd0c8f6f13b8c0e131d0c981c9b9c8a7c537ddd0ed3348",
    );
    assert_eq!(search["query"], "author:Alice");
    assert_eq!(search["sort"], "NEW");
    assert_eq!(
        search["filters"],
        json!([{"key": "time_range", "value": "all"}])
    );
    assert!(search.get("afterCursor").is_none());
    let mut continued = search;
    continued["afterCursor"] = json!(search_cursor);
    assert_eq!(
        assert_feed_operation(
            &requests[5],
            "SearchPosts",
            "c3d31b88bfa15180fdcd0c8f6f13b8c0e131d0c981c9b9c8a7c537ddd0ed3348"
        ),
        continued
    );
    assert_eq!(pages.len(), 4);
    assert!(pages[0].data.children.is_empty());
    assert_eq!(pages[0].data.after.as_deref(), Some(cursor));
    assert_eq!(pages[2].data.after.as_deref(), Some(search_cursor));
    assert_eq!(
        pages
            .iter()
            .map(|page| page.data.children.len())
            .collect::<Vec<_>>(),
        [0, 2, 1, 1]
    );
    let parsed: Vec<_> = pages
        .iter()
        .flat_map(|page| RedditPostParser::default().parse(page))
        .collect();
    // These IDs become downloader/cache keys and must match the existing JSON
    // provider's bare IDs, even though GraphQL hydration uses t3_ fullnames.
    assert_eq!(
        parsed
            .iter()
            .map(|post| post.id.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three", "four"]
    );
}

#[tokio::test]
async fn subreddit_feed_without_page_info_pages_by_encoded_fullname() {
    let server = Server::start(vec![
        data(json!({"subredditFeed": {"elements": {"edges": [{"node": {"groupId": "t3_one"}}]}}})),
        data(json!({"postsInfoByIds": [image_post("t3_one")]})),
        data(json!({"subredditFeed": {"elements": {"edges": []}}})),
    ])
    .await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let gql = graphql(
        &client,
        jar,
        &server.base,
        Some(&session("alice", "imported")),
        Some(&token("alice")),
    );
    let mut cmd = command(None);
    cmd.resource = "pics".into();
    let pages = bounded(gql.subreddit_submissions(&cmd)).await.unwrap();
    let requests = server.finish(&[("POST", "/gql"); 3]).await;
    assert_eq!(requests[0].json()["operationName"], "SubredditFeedSdui");
    assert!(requests[0].json()["variables"].get("after").is_none());
    assert_eq!(requests[1].json()["operationName"], "PostsByIds");
    assert_eq!(
        requests[2].json()["variables"]["after"],
        base64::engine::general_purpose::STANDARD.encode("t3_one")
    );
    assert_eq!(pages[0].data.children[0].data.id, "one");
    assert!(pages[1].data.after.is_none());
}

#[tokio::test]
async fn user_crawl_keeps_posts_when_terminal_cursors_are_missing_or_empty() {
    let server = Server::start(vec![
        data(json!({"redditorInfoByName": {"__typename": "Redditor"}})),
        data(json!({"profileFeed": {"elements": {"edges": [{"node": {"groupId": "t3_kept"}}], "pageInfo": {"hasNextPage": false}}}})),
        data(json!({"postsInfoByIds": [image_post("t3_kept")]})),
        data(json!({"search": {"general": {"posts": {"edges": [], "pageInfo": {"hasNextPage": false, "endCursor": ""}}}}})),
    ]).await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let gql = graphql(
        &client,
        jar,
        &server.base,
        Some(&session("alice", "imported")),
        Some(&token("alice")),
    );
    let pages = bounded(gql.user_submissions(&command(None))).await.unwrap();
    let posts: Vec<_> = pages.iter().flat_map(|page| &page.data.children).collect();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].data.id, "kept");
    assert!(pages.iter().all(|page| page.data.after.is_none()));
    server.finish(&[("POST", "/gql"); 4]).await;
}

#[tokio::test]
async fn user_submissions_zero_limit_never_touches_network() {
    let server = Server::start(vec![]).await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let gql = graphql(&client, jar, &server.base, None, None);
    assert!(bounded(gql.user_submissions(&command(Some(0))))
        .await
        .unwrap()
        .is_empty());
    server.finish(&[]).await;
}

#[tokio::test]
async fn user_submissions_limit_counts_discovery_pages_even_ad_only_and_search_has_own_budget() {
    let sdui_cursor = "unconsumed sdui cursor";
    let search_cursor = "unconsumed search cursor";
    let server = Server::start(vec![
        data(json!({"redditorInfoByName": {"__typename": "Redditor"}})),
        data(json!({"profileFeed": {"elements": connection(vec![json!({"id": "t3_ad", "adPayload": {}})], Some(sdui_cursor))}})),
        data(json!({"search": {"general": {"posts": connection(vec![image_post("one"), image_post("two")], Some(search_cursor))}}})),
    ]).await;
    let jar = Arc::new(Jar::default());
    let client = http_client(jar.clone());
    let gql = graphql(
        &client,
        jar,
        &server.base,
        Some(&session("alice", "imported")),
        Some(&token("alice")),
    );
    let pages = bounded(gql.user_submissions(&command(Some(1))))
        .await
        .unwrap();
    let requests = server.finish(&[("POST", "/gql"); 3]).await;
    assert_eq!(
        requests
            .iter()
            .map(|r| r.json()["operationName"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        ["UserProfile", "SubmittedPostsFeedSdui", "SearchPosts"]
    );
    assert_eq!(pages.len(), 2);
    assert!(pages[0].data.children.is_empty());
    assert_eq!(pages[0].data.after.as_deref(), Some(sdui_cursor));
    assert_eq!(pages[1].data.after.as_deref(), Some(search_cursor));
    assert_eq!(pages[1].data.children.len(), 2, "limit is pages, not posts");
}
