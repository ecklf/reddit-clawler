use super::state::SharedState;
use crate::{
    clients::{download_redgifs_media, RedgifsQuality},
    reddit_parser::{RedditCrawlerPost, RedditMediaProviderType},
};
use filetime::FileTime;
use std::{
    fs::{self, File},
    io::Write,
    sync::Arc,
};
use tokio::sync::Mutex;

pub fn prepare_output_folder(folder_path: &str) -> Result<(), anyhow::Error> {
    if fs::metadata(folder_path).is_err() {
        fs::create_dir_all(folder_path)?;
    }
    Ok(())
}

pub fn get_output_folder(path: &str, username: &str) -> String {
    format!("{}/{}", path, username)
}

pub async fn write_crawler_post(
    client: &reqwest_middleware::ClientWithMiddleware,
    shared_state: &Arc<Mutex<SharedState>>,
    folder_path: &str,
    media: &RedditCrawlerPost,
) -> Result<Option<f64>, anyhow::Error> {
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

    // let file_name = NON_ALPHANUMERIC_RE.replace_all(title, "_");
    // let file_name = MULTIPLE_UNDERSCORE_RE.replace_all(&file_name, "_");

    let response = match provider {
        RedditMediaProviderType::RedditImage
        | RedditMediaProviderType::RedditGalleryImage
        | RedditMediaProviderType::RedditVideo => Some(client.get(url).send().await?),
        RedditMediaProviderType::RedgifsImage | RedditMediaProviderType::RedgifsVideo => {
            Some(download_redgifs_media(client, shared_state, url, RedgifsQuality::HD).await?)
        }
        RedditMediaProviderType::YoutubeVideo => {
            println!("Not handling youtube download for: {}", &title);
            None
        }
        RedditMediaProviderType::ImgurImage => {
            println!("Not handling imgur download for: {}", &title);
            None
        }
        RedditMediaProviderType::None => {
            println!("Skipping unsupported provider: {}", &title);
            None
        }
    };

    if let Some(response) = response {
        let bytes = response.bytes().await?;

        let file_path = format!(
            "./{folder_path}/{file_name}.{extension}",
            folder_path = folder_path,
            file_name = file_name,
            extension = extension
        );

        let mut out = File::create(&file_path)?;
        let unix_timestamp = created_utc.timestamp();
        out.write_all(&bytes)?;

        let _ = tokio::task::spawn_blocking(move || -> Result<(), anyhow::Error> {
            let now = FileTime::from_unix_time(unix_timestamp, 0);
            filetime::set_file_handle_times(&out, Some(now), Some(now))?;
            Ok(())
        })
        .await?;

        return Ok(Some(bytes.len() as f64));
    }

    Ok(None)
}
