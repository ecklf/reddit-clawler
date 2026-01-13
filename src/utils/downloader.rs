use super::state::SharedState;
use crate::{
    clients::{download_redgifs_media, RedgifsQuality},
    reddit_parser::{RedditCrawlerPost, RedditMediaProviderType},
};
use chrono::{DateTime, Utc};
use filetime::FileTime;
use reqwest::Response;
use std::{
    fs::{self, File},
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
};
use tokio::sync::Mutex;

pub fn prepare_output_folder(folder_path: &str) -> Result<(), anyhow::Error> {
    if fs::metadata(folder_path).is_err() {
        fs::create_dir_all(folder_path)?;
    }
    Ok(())
}

pub fn get_output_folder(path: &str, stem: &str) -> String {
    format!("{}/{}", path, stem)
}

enum ProviderHandlerReturned {
    HttpResponse(Response),
    ThirdPartyResponse(String),
    NotFound,
    Unhandled,
}

pub async fn set_file_timestamp(
    file_path: File,
    created_utc: DateTime<Utc>,
) -> Result<(), anyhow::Error> {
    let unix_timestamp = created_utc.timestamp();
    let _ = tokio::task::spawn_blocking(move || -> Result<(), anyhow::Error> {
        let now = FileTime::from_unix_time(unix_timestamp, 0);
        filetime::set_file_handle_times(&file_path, Some(now), Some(now))?;
        Ok(())
    })
    .await?;
    Ok(())
}

pub enum DownloadPostResult {
    ReceivedBytes(f64),
    ReceivedFailed,
    ReceivedNotFound,
    ReceivedUnhandled,
}

pub async fn download_crawler_post(
    client: &reqwest_middleware::ClientWithMiddleware,
    shared_state: &Arc<Mutex<SharedState>>,
    folder_path: &str,
    media: &RedditCrawlerPost,
) -> Result<DownloadPostResult, anyhow::Error> {
    let RedditCrawlerPost {
        author,
        created_utc,
        extension,
        id,
        index,
        provider,
        subreddit: _subreddit,
        title,
        upvotes,
        url,
        is_gallery: _,
        media_id: _,
    } = media;

    let file_scheme = String::from("{UPVOTES}_{AUTHOR}_{POSTID}_{DATE}");
    let formatted_date = created_utc.format("%Y-%m-%d").to_string();

    let mut file_name = file_scheme
        .replace("{UPVOTES}", &upvotes.to_string())
        .replace("{AUTHOR}", &author.to_string())
        .replace("{POSTID}", &id.to_string())
        .replace("{DATE}", &formatted_date);

    if let Some(index) = index {
        file_name = format!("{}_{}", file_name, index);
    }

    let file_path = format!(
        "./{folder_path}/{file_name}.{extension}",
        folder_path = folder_path,
        file_name = file_name,
        extension = extension
    );

    let response = match provider {
        RedditMediaProviderType::RedditImage
        | RedditMediaProviderType::RedditGalleryImage
        | RedditMediaProviderType::RedditGifVideo => {
            ProviderHandlerReturned::HttpResponse(client.get(url).send().await?)
        }
        RedditMediaProviderType::RedditVideo => {
            let mut child = Command::new("yt-dlp")
                .arg(url)
                .arg("-o")
                .arg(&file_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("Spawning yt-dlp process failed");

            child.wait().expect("Download with yt-dlp process failed");
            ProviderHandlerReturned::ThirdPartyResponse(file_path.clone())
        }
        RedditMediaProviderType::RedgifsImage | RedditMediaProviderType::RedgifsVideo => {
            ProviderHandlerReturned::HttpResponse(
                download_redgifs_media(client, shared_state, url, RedgifsQuality::HD).await?,
            )
        }
        RedditMediaProviderType::YoutubeVideo => {
            let mut child = Command::new("yt-dlp")
                .arg(url)
                .arg("-f")
                .arg("bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best")
                .arg("-o")
                .arg(&file_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("Spawning yt-dlp process failed");

            child.wait().expect("Download with yt-dlp process failed");
            ProviderHandlerReturned::ThirdPartyResponse(file_path.clone())
        }
        RedditMediaProviderType::ImgurImage => {
            let response = client.get(url).send().await?;
            let content_type = response.headers().get("content-type");
            match content_type {
                Some(value) => match value.to_str() {
                    Ok(s) => {
                        // Imgur returns "text/html" when the post has been deleted
                        if s == "text/html" {
                            ProviderHandlerReturned::NotFound
                        } else {
                            ProviderHandlerReturned::HttpResponse(response)
                        }
                    }
                    Err(_) => ProviderHandlerReturned::HttpResponse(response),
                },
                _ => ProviderHandlerReturned::HttpResponse(response),
            }
        }
        RedditMediaProviderType::None => {
            println!("Skipping unsupported provider: {}", &title);
            ProviderHandlerReturned::Unhandled
        }
    };

    match response {
        ProviderHandlerReturned::HttpResponse(response) => {
            let bytes = response.bytes().await?;

            let mut out = File::create(&file_path)?;
            out.write_all(&bytes)?;
            set_file_timestamp(out, *created_utc).await?;

            Ok(DownloadPostResult::ReceivedBytes(bytes.len() as f64))
        }
        ProviderHandlerReturned::ThirdPartyResponse(fp) => {
            let bytes = fs::metadata(fp)?.len() as f64;
            set_file_timestamp(File::open(&file_path)?, *created_utc).await?;
            Ok(DownloadPostResult::ReceivedBytes(bytes))
        }
        ProviderHandlerReturned::NotFound => Ok(DownloadPostResult::ReceivedNotFound),
        ProviderHandlerReturned::Unhandled => Ok(DownloadPostResult::ReceivedUnhandled),
    }
}
