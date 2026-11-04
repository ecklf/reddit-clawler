use super::{RedditClient, RedditProviderError};
use crate::http::build_client;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::timeout,
};
use wreq::{cookie::Jar, StatusCode};

const TIMEOUT: Duration = Duration::from_secs(5);
const QUERY: &str = "q=author%3Aspez&after=t3_example&raw_json=1";
const JSON: &[u8] = br#"{"ok":true}"#;

// Each scripted response closes its connection. An extra fallback/retry cannot
// silently succeed: the listener is dropped after the last expected request.
async fn serve(responses: Vec<Vec<u8>>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        timeout(TIMEOUT, async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let count = stream.read(&mut buf).await.unwrap();
                    assert_ne!(count, 0, "connection closed before request headers");
                    request.extend_from_slice(&buf[..count]);
                    assert!(request.len() <= 64 * 1024, "request headers too large");
                    if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                stream.write_all(&response).await.unwrap();
                stream.shutdown().await.unwrap();
            }
            requests
        })
        .await
        .expect("mock server timed out waiting for the expected requests")
    });
    (base, task)
}

fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn json_response() -> Vec<u8> {
    response("200 OK", "Content-Type: application/json\r\n", JSON)
}

async fn fetch(
    client: &wreq::Client,
    url: &str,
    fallback: Option<&str>,
) -> Result<wreq::Response, RedditProviderError> {
    timeout(
        TIMEOUT,
        RedditClient::default().fetch_json_with_fallback(client, url, fallback),
    )
    .await
    .expect("fetch_json_with_fallback timed out")
}

async fn assert_json(result: wreq::Response) {
    assert_eq!(result.status(), StatusCode::OK);
    let body = timeout(TIMEOUT, result.json::<serde_json::Value>())
        .await
        .expect("reading JSON timed out")
        .unwrap();
    assert_eq!(body, serde_json::json!({"ok": true}));
}

async fn assert_requests(server: JoinHandle<Vec<String>>, targets: &[&str]) -> Vec<String> {
    let requests = timeout(TIMEOUT, server)
        .await
        .expect("joining mock server timed out")
        .unwrap();
    assert_eq!(requests.len(), targets.len());
    for (request, target) in requests.iter().zip(targets) {
        assert_eq!(
            request.lines().next().unwrap(),
            format!("GET {target} HTTP/1.1")
        );
        assert_eq!(header(request, "accept"), "application/json");
    }
    requests
}

fn header<'a>(request: &'a str, name: &str) -> &'a str {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .unwrap_or_else(|| panic!("missing {name} header"))
}

#[tokio::test]
async fn forbidden_falls_back_preserving_query_accept_and_jar_cookies() {
    let (base, server) = serve(vec![
        response(
            "403 Forbidden",
            "Set-Cookie: session=from-primary; Path=/\r\n",
            b"blocked",
        ),
        json_response(),
    ])
    .await;
    let primary_path = format!("/www/search.json?{QUERY}");
    let fallback_path = format!("/old/search.json?{QUERY}");
    let jar = Arc::new(Jar::default());
    // Deliberately use the same loopback host: this checks jar reuse, not
    // widening host-only cookies from www.reddit.com to old.reddit.com.
    jar.add("imported=present; Path=/", &base);
    let client = build_client(jar).unwrap();
    let result = fetch(
        &client,
        &format!("{base}{primary_path}"),
        Some(&format!("{base}{fallback_path}")),
    )
    .await
    .unwrap();
    assert_json(result).await;
    let requests = assert_requests(server, &[&primary_path, &fallback_path]).await;
    for request in &requests {
        assert!(header(request, "cookie")
            .split(';')
            .any(|cookie| cookie.trim() == "imported=present"));
    }
    assert!(header(&requests[1], "cookie")
        .split(';')
        .any(|cookie| cookie.trim() == "session=from-primary"));
}

#[tokio::test]
async fn challenge_redirect_falls_back_to_original_provided_endpoint() {
    let challenge = "/?js_challenge=1&jsc_token=primary-secret&solution=primary-solution";
    let (base, server) = serve(vec![
        response("302 Found", &format!("Location: {challenge}\r\n"), b""),
        response("200 OK", "Content-Type: text/html\r\n", b"challenge"),
        json_response(),
    ])
    .await;
    let primary_path = format!("/www/user/spez/submitted.json?{QUERY}");
    let fallback_path = format!("/old/user/spez/submitted.json?{QUERY}");
    let client = build_client(Arc::new(Jar::default())).unwrap();
    let result = fetch(
        &client,
        &format!("{base}{primary_path}"),
        Some(&format!("{base}{fallback_path}")),
    )
    .await
    .unwrap();
    assert_json(result).await;
    assert_requests(server, &[&primary_path, challenge, &fallback_path]).await;
}

#[tokio::test]
async fn successful_html_falls_back_to_json() {
    for content_type in ["text/html; charset=utf-8", "Application/XHTML+XML"] {
        let (base, server) = serve(vec![
            response(
                "200 OK",
                &format!("Content-Type: {content_type}\r\n"),
                b"<html>not JSON</html>",
            ),
            json_response(),
        ])
        .await;
        let client = build_client(Arc::new(Jar::default())).unwrap();
        let result = fetch(
            &client,
            &format!("{base}/www/about.json"),
            Some(&format!("{base}/old/about.json")),
        )
        .await
        .unwrap();
        assert_json(result).await;
        assert_requests(server, &["/www/about.json", "/old/about.json"]).await;
    }
}

#[tokio::test]
async fn final_challenge_is_classified_without_leaking_tokens() {
    let challenge = "/?js_challenge=1&jsc_token=secret-token&solution=secret-solution";
    for status in ["200 OK", "403 Forbidden"] {
        let (base, server) = serve(vec![
            response("403 Forbidden", "", b"blocked"),
            response("302 Found", &format!("Location: {challenge}\r\n"), b""),
            response(status, "Content-Type: text/html\r\n", b"challenge"),
        ])
        .await;
        let client = build_client(Arc::new(Jar::default())).unwrap();
        let error = fetch(
            &client,
            &format!("{base}/www/search.json"),
            Some(&format!("{base}/old/search.json")),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, RedditProviderError::BrowserChallenge));
        for rendered in [error.to_string(), format!("{error:?}")] {
            for sensitive in [
                "secret-token",
                "secret-solution",
                "jsc_token",
                "solution",
                &base,
            ] {
                assert!(!rendered.contains(sensitive), "error leaked {sensitive}");
            }
        }
        assert_requests(server, &["/www/search.json", "/old/search.json", challenge]).await;
    }
}

#[tokio::test]
async fn login_redirect_requires_authentication_without_fallback() {
    for status in ["200 OK", "403 Forbidden", "404 Not Found"] {
        let login = "/login/?dest=%2Fsearch.json";
        let (base, server) = serve(vec![
            response("302 Found", &format!("Location: {login}\r\n"), b""),
            response(status, "Content-Type: text/html\r\n", b"sign in"),
        ])
        .await;
        let client = build_client(Arc::new(Jar::default())).unwrap();
        let error = fetch(
            &client,
            &format!("{base}/www/search.json"),
            Some(&format!("{base}/must-not-fetch.json")),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, RedditProviderError::AuthenticationRequired));
        assert_requests(server, &["/www/search.json", login]).await;
    }
}

#[tokio::test]
async fn final_forbidden_is_returned_without_another_fallback() {
    let (base, server) = serve(vec![
        response("403 Forbidden", "", b"primary blocked"),
        response("403 Forbidden", "", b"fallback blocked"),
    ])
    .await;
    let client = build_client(Arc::new(Jar::default())).unwrap();
    let result = fetch(
        &client,
        &format!("{base}/www/search.json"),
        Some(&format!("{base}/old/search.json")),
    )
    .await
    .unwrap();
    assert_eq!(result.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        timeout(TIMEOUT, result.text()).await.unwrap().unwrap(),
        "fallback blocked"
    );
    assert_requests(server, &["/www/search.json", "/old/search.json"]).await;
}

#[tokio::test]
async fn not_found_and_rate_limited_do_not_fall_back() {
    for (status, headers, expected) in [
        ("404 Not Found", "", StatusCode::NOT_FOUND),
        (
            "429 Too Many Requests",
            "Retry-After: 121\r\n",
            StatusCode::TOO_MANY_REQUESTS,
        ),
    ] {
        for path in ["/www/search.json", "/?js_challenge=1&jsc_token=redacted"] {
            let (base, server) = serve(vec![response(status, headers, b"unchanged")]).await;
            let client = build_client(Arc::new(Jar::default())).unwrap();
            let result = fetch(
                &client,
                &format!("{base}{path}"),
                Some(&format!("{base}/must-not-fetch.json")),
            )
            .await;
            if expected == StatusCode::NOT_FOUND && path.starts_with("/?") {
                assert!(matches!(result, Err(RedditProviderError::BrowserChallenge)));
            } else {
                assert_eq!(result.unwrap().status(), expected);
            }
            assert_requests(server, &[path]).await;
        }
    }
}

#[tokio::test]
async fn html_not_found_is_not_proof_of_a_deleted_resource() {
    let (base, server) = serve(vec![response(
        "404 Not Found",
        "Content-Type: text/html\r\n",
        b"<html>interstitial</html>",
    )])
    .await;
    let client = build_client(Arc::new(Jar::default())).unwrap();
    let error = fetch(
        &client,
        &format!("{base}/www/user/spez/submitted.json"),
        Some(&format!("{base}/must-not-fetch.json")),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RedditProviderError::UnexpectedHtml));
    assert_requests(server, &["/www/user/spez/submitted.json"]).await;
}

#[tokio::test]
async fn final_successful_html_is_unexpected_html() {
    for content_type in ["text/html; charset=utf-8", "application/xhtml+xml"] {
        let (base, server) = serve(vec![
            response("403 Forbidden", "", b"blocked"),
            response(
                "200 OK",
                &format!("Content-Type: {content_type}\r\n"),
                b"<html>still not JSON</html>",
            ),
        ])
        .await;
        let client = build_client(Arc::new(Jar::default())).unwrap();
        let error = fetch(
            &client,
            &format!("{base}/www/search.json"),
            Some(&format!("{base}/old/search.json")),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, RedditProviderError::UnexpectedHtml));
        assert_requests(server, &["/www/search.json", "/old/search.json"]).await;
    }
}
