use flate2::{write::GzEncoder, Compression};
use reddit_clawler::http::{build_client, RequestBuilderExt};
use reddit_clawler::{
    cli::{self, CliRedditCommand, CliSharedOptions, RedditCategoryFilter, RedditTimeframeFilter},
    clients::api_types::reddit::submitted_response::RedditSubmittedResponse,
    reddit_parser::RedditPostParser,
    reddit_parser::{RedditCrawlerPost, RedditMediaProviderType},
    utils::{
        download_crawler_post,
        state::{FileCacheItemLatest, FileCacheLatest, SharedState},
        DownloadPostResult,
    },
};
use std::{fs, io::Write, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
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

struct DownloadFolder(PathBuf);

impl DownloadFolder {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("reddit-clawler-{}", rand::random::<u64>()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for DownloadFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn image_post(url: String, provider: RedditMediaProviderType) -> RedditCrawlerPost {
    RedditCrawlerPost {
        author: "example".into(),
        created_utc: "2026-10-22T12:00:00Z".parse().unwrap(),
        extension: "webp".into(),
        id: "testpost".into(),
        index: None,
        provider,
        subreddit: "example".into(),
        title: "Image".into(),
        upvotes: 1,
        url,
        is_gallery: Some(false),
        media_id: None,
    }
}

#[tokio::test]
async fn media_downloads_override_navigation_accept_and_preserve_bytes() {
    let body =
        b"RIFF\x1a\0\0\0WEBPVP8L\x0d\0\0\0\x2f\0\0\0\0\x07\x10\x11\xfd\x0f\x44\x44\xff\x03\0";
    for provider in [
        RedditMediaProviderType::RedditImage,
        RedditMediaProviderType::RedditGalleryImage,
        RedditMediaProviderType::RedditGifVideo,
        RedditMediaProviderType::ImgurImage,
    ] {
        let (url, server) = serve(vec![
            response("302 Found", "Location: /image.webp\r\n", b""),
            response("200 OK", "Content-Type: image/webp\r\n", body),
        ])
        .await;
        let folder = DownloadFolder::new();
        let client = build_client(Arc::new(Jar::default())).unwrap();
        let state = Arc::new(Mutex::new(SharedState::default()));
        let result = download_crawler_post(
            &client,
            &state,
            folder.0.to_str().unwrap(),
            &image_post(url, provider),
        )
        .await
        .unwrap();
        assert!(matches!(result, DownloadPostResult::ReceivedBytes(n) if n == body.len() as f64));
        assert_eq!(
            fs::read(folder.0.join("1_example_testpost_2026-10-22.webp")).unwrap(),
            body
        );
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests {
            let headers = request.to_ascii_lowercase();
            let accept = headers
                .lines()
                .find(|line| line.starts_with("accept:"))
                .unwrap();
            assert!(accept.contains("image/webp"));
            assert!(!accept.contains("text/html"));
            assert!(headers.contains("user-agent: mozilla/5.0 (iphone; cpu iphone os 18_1_1"));
        }
    }
}

#[tokio::test]
async fn rejects_html_media_viewer_after_redirect_without_writing_a_file() {
    let (url, server) = serve(vec![
        response("307 Temporary Redirect", "Location: /media\r\n", b""),
        response(
            "200 OK",
            "Content-Type: text/html; charset=utf-8\r\n",
            b"<!doctype html><html>Media viewer</html>",
        ),
    ])
    .await;
    let folder = DownloadFolder::new();
    let client = build_client(Arc::new(Jar::default())).unwrap();
    let state = Arc::new(Mutex::new(SharedState::default()));
    let error = download_crawler_post(
        &client,
        &state,
        folder.0.to_str().unwrap(),
        &image_post(url, RedditMediaProviderType::RedditImage),
    )
    .await
    .err()
    .expect("HTML must not count as a successful media download");
    assert!(error.to_string().contains("text/html"), "{error}");
    assert_eq!(fs::read_dir(&folder.0).unwrap().count(), 0);
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn rejects_unsuccessful_or_empty_media_without_overwriting_existing_files() {
    for (status, headers, body) in [
        (
            "403 Forbidden",
            "Content-Type: image/webp\r\n",
            &b"blocked"[..],
        ),
        ("200 OK", "Content-Type: image/webp\r\n", &b""[..]),
        ("200 OK", "Content-Type: application/json\r\n", &b"{}"[..]),
    ] {
        let (url, server) = serve(vec![response(status, headers, body)]).await;
        let folder = DownloadFolder::new();
        let path = folder.0.join("1_example_testpost_2026-10-22.webp");
        fs::write(&path, b"existing image").unwrap();
        let client = build_client(Arc::new(Jar::default())).unwrap();
        let state = Arc::new(Mutex::new(SharedState::default()));
        assert!(download_crawler_post(
            &client,
            &state,
            folder.0.to_str().unwrap(),
            &image_post(url, RedditMediaProviderType::RedditImage),
        )
        .await
        .is_err());
        assert_eq!(fs::read(path).unwrap(), b"existing image");
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn force_redownloads_successfully_cached_posts_for_all_commands() {
    for command in ["user", "subreddit", "search"] {
        let (url, server) = serve(vec![response(
            "200 OK",
            "Content-Type: image/webp\r\n",
            b"replacement image",
        )])
        .await;
        let folder = DownloadFolder::new();
        let output = folder.0.join(command).join("example");
        fs::create_dir_all(&output).unwrap();
        let mock = folder.0.join("response.json");
        let mut mock_data: serde_json::Value = serde_json::from_str(include_str!(
            "mocks/reddit/submitted_response/reddit_image.json"
        ))
        .unwrap();
        mock_data[0]["data"]["children"][0]["data"]["url"] = url.into();
        fs::write(&mock, serde_json::to_vec(&mock_data).unwrap()).unwrap();
        let responses: Vec<RedditSubmittedResponse> = serde_json::from_value(mock_data).unwrap();
        let post = RedditPostParser::default().parse(&responses[0]).remove(0);
        let mut cache = FileCacheLatest::default();
        cache.upsert_item(FileCacheItemLatest {
            id: post.id.clone(),
            created_utc: post.created_utc,
            title: post.title.clone(),
            subreddit: post.subreddit.clone(),
            url: post.url.clone(),
            success: true,
            index: post.index,
            is_gallery: post.is_gallery,
            media_id: post.media_id.clone(),
        });
        fs::write(
            output.join("cache.json"),
            serde_json::to_vec(&cache).unwrap(),
        )
        .unwrap();
        let image = output.join(format!(
            "{}_{}_{}_{}.webp",
            post.upvotes,
            post.author,
            post.id,
            post.created_utc.format("%Y-%m-%d")
        ));
        fs::write(&image, b"").unwrap();
        let client = build_client(Arc::new(Jar::default())).unwrap();
        for force in [false, true] {
            let cmd = CliRedditCommand {
                resource: "example".into(),
                category: RedditCategoryFilter::New,
                timeframe: RedditTimeframeFilter::All,
                options: CliSharedOptions {
                    concurrency: 1,
                    mock: Some(mock.to_str().unwrap().into()),
                    output: folder.0.to_str().unwrap().into(),
                    cookies: None,
                    skip: false,
                    verbose: false,
                    limit: None,
                    force,
                    update: false,
                },
            };
            let state = Arc::new(Mutex::new(SharedState::default()));
            match command {
                "user" => cli::handle_user_command(cmd, &client, &state)
                    .await
                    .unwrap(),
                "subreddit" => cli::handle_subreddit_command(cmd, &client, &state)
                    .await
                    .unwrap(),
                "search" => cli::handle_search_command(cmd, &client, &state)
                    .await
                    .unwrap(),
                _ => unreachable!(),
            }
            assert_eq!(
                fs::read(&image).unwrap(),
                if force {
                    &b"replacement image"[..]
                } else {
                    &b""[..]
                }
            );
            let state = state.lock().await;
            assert_eq!(state.file_cache.files.len(), 1);
            assert!(state.file_cache.files[0].success);
        }
        assert_eq!(server.await.unwrap().len(), 1);
    }
}
