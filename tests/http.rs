use flate2::{write::GzEncoder, Compression};
use reddit_clawler::http::{build_client, RequestBuilderExt};
use std::{io::Write, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use wreq::{cookie::Jar, StatusCode};

async fn serve(responses: Vec<Vec<u8>>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .expect("expected another request")
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let count = stream.read(&mut buf).await.unwrap();
                assert_ne!(count, 0, "connection closed before request headers");
                request.extend_from_slice(&buf[..count]);
                if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
            }
            requests.push(String::from_utf8(request).unwrap());
            stream.write_all(&response).await.unwrap();
            stream.shutdown().await.unwrap();
        }
        requests
    });
    (url, task)
}

fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

#[tokio::test]
async fn honors_retry_after_and_keeps_session_cookies() {
    let (url, server) = serve(vec![
        response(
            "429 Too Many Requests",
            "Retry-After: 1\r\nSet-Cookie: session=retry; Path=/\r\n",
            b"",
        ),
        response("200 OK", "", b"ok"),
    ])
    .await;
    let jar = Arc::new(Jar::default());
    jar.add("imported=present; Path=/", &url);
    let client = build_client(jar).unwrap();
    let started = std::time::Instant::now();
    let result = client.get(&url).send_with_retry().await.unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    assert!(started.elapsed() >= Duration::from_secs(1));
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("imported=present"));
    assert!(requests[1].contains("imported=present"));
    assert!(requests[1].contains("session=retry"));
}

#[tokio::test]
async fn bounds_transient_retries_and_returns_last_response() {
    let (url, server) = serve(vec![
        response("503 Service Unavailable", "", b"unavailable");
        4
    ])
    .await;
    let client = build_client(Arc::new(Jar::default())).unwrap();
    let result = client.get(url).send_with_retry().await.unwrap();
    assert_eq!(result.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(result.text().await.unwrap(), "unavailable");
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn returns_forbidden_and_long_retry_after_without_retrying() {
    for (status, headers, expected) in [
        ("403 Forbidden", "", StatusCode::FORBIDDEN),
        (
            "429 Too Many Requests",
            "Retry-After: 121\r\n",
            StatusCode::TOO_MANY_REQUESTS,
        ),
    ] {
        let (url, server) = serve(vec![response(status, headers, b"blocked")]).await;
        let client = build_client(Arc::new(Jar::default())).unwrap();
        let result = client.get(url).send_with_retry().await.unwrap();
        assert_eq!(result.status(), expected);
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn does_not_replay_post_requests() {
    let (url, server) = serve(vec![response("503 Service Unavailable", "", b"")]).await;
    let client = build_client(Arc::new(Jar::default())).unwrap();
    let result = client.post(url).send_with_retry().await.unwrap();
    assert_eq!(result.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn sends_consistent_browser_headers_and_decodes_gzip() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(br#"{"ok":true}"#).unwrap();
    let body = encoder.finish().unwrap();
    let (url, server) = serve(vec![response(
        "200 OK",
        "Content-Encoding: gzip\r\nContent-Type: application/json\r\n",
        &body,
    )])
    .await;
    let client = build_client(Arc::new(Jar::default())).unwrap();
    let result = client.get(url).send_with_retry().await.unwrap();
    assert_eq!(
        result.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({"ok": true})
    );
    let requests = server.await.unwrap();
    let headers = requests[0].to_ascii_lowercase();
    assert!(headers.contains("user-agent: mozilla/5.0 (iphone; cpu iphone os 18_1_1"));
    assert!(headers.contains("version/18.1.1 mobile/15e148 safari/604.1"));
    assert!(headers.contains("accept-encoding: gzip, deflate, br\r\n"));
    assert!(headers.contains("accept-language: en-us,en;q=0.9\r\n"));
}
