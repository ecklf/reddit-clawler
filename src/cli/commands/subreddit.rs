use crate::{
    cli::CliSubredditCommand,
    clients::{self},
    reddit_parser::RedditPostParser,
    utils::{
        self, download_crawler_post,
        state::{DownloadStats, FileCache, FileCacheItem, SharedState},
        DownloadProgress,
    },
};
use anyhow::anyhow;
use owo_colors::OwoColorize;
use spinoff::{spinners, Color, Spinner};
use std::{error::Error, fs, path::Path, sync::Arc, time::Duration};
use tokio::{
    sync::{oneshot, Mutex, Semaphore},
    time::sleep,
};

pub async fn handle_subreddit_command(
    cmd: CliSubredditCommand,
    client: &reqwest_middleware::ClientWithMiddleware,
    shared_state: &Arc<Mutex<SharedState>>,
) -> Result<(), Box<dyn Error>> {
    let CliSubredditCommand {
        subreddit,
        category,
        timeframe,
        options,
    } = cmd;

    let (tx, mut rx) = oneshot::channel::<bool>();
    let reddit_client = clients::RedditClient::default();
    let reddit_parser = RedditPostParser::default();

    let mut spinner = Spinner::new(
        spinners::Dots,
        format!("Fetching posts from {}{}", "/r/".bold(), subreddit.bold()),
        Color::TrueColor {
            r: 237,
            g: 106,
            b: 44,
        },
    );
    let output_folder = utils::get_output_folder(&options.output, &subreddit);
    utils::prepare_output_folder(&output_folder)?;
    let responses = reddit_client
        .get_subreddit_submissions(client, &subreddit, &category, &timeframe)
        .await?;

    let posts = responses
        .iter()
        .flat_map(|r| reddit_parser.parse(r))
        .collect::<Vec<_>>();

    let mut posts_to_download = posts.clone();
    let file_cache_path = format!("{}/cache.json", output_folder);

    if Path::new(&file_cache_path).exists() {
        let file_cache = fs::read_to_string(format!("{}/cache.json", output_folder)).unwrap();
        let file_cache =
            serde_json::from_str::<FileCache>(&file_cache).expect("Failed to parse cache file");

        let mut ss = shared_state.lock().await;
        ss.file_cache = file_cache.clone();

        posts_to_download = posts_to_download
            .into_iter()
            .filter(|p| {
                // Try to find the successfully downloaded post in the cache
                let found = file_cache.files.iter().any(|f| p.id == f.id && f.success);
                !found
            })
            .collect::<Vec<_>>();
    }

    spinner.success(&format!(
        "Done, trying to download {} posts. - Cached {}",
        posts_to_download.len(),
        posts.len() - posts_to_download.len()
    ));

    let download_stats: Arc<Mutex<DownloadStats>> = Arc::new(Mutex::new(DownloadStats::default()));
    let total_post_len: u64 = posts_to_download.len() as u64;
    let download_progress: Arc<Mutex<DownloadProgress>> =
        Arc::new(Mutex::new(DownloadProgress::new(total_post_len)));

    let semaphore = Arc::new(Semaphore::new(options.concurrency as usize));

    if options.skip {
        println!(
            "{}",
            format_args!("{} {}", "[FLAG]".red().bold(), "Download skipped".bold()),
        );
        return Ok(());
    }

    let clockwork_dp = Arc::clone(&download_progress);
    // Updates the progress bar so it runs smoothly
    let clockwork_orange = tokio::spawn(async move {
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            clockwork_dp.lock().await.control.tick();
            sleep(Duration::from_millis(100)).await;
        }
    });

    for post in posts_to_download {
        let client = client.clone();
        let output_folder = output_folder.clone();

        let dp_clone = Arc::clone(&download_progress);
        let ds_clone = Arc::clone(&download_stats);
        let ss_clone = Arc::clone(shared_state);
        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();

        tokio::spawn(async move {
            match download_crawler_post(&client, &ss_clone, &output_folder, &post).await {
                Ok(result) => {
                    match result {
                        utils::DownloadPostResult::ReceivedBytes(bytes) => {
                            let mut dl_stats = ds_clone.lock().await;
                            dl_stats.files_downloaded += 1;
                            dl_stats.bytes_downloaded += bytes;

                            ss_clone.lock().await.file_cache.files.push(FileCacheItem {
                                id: post.id.clone(),
                                created_utc: post.created_utc,
                                title: post.title.clone(),
                                subreddit: post.subreddit.clone(),
                                url: post.url.clone(),
                                success: true,
                                index: post.index,
                            });

                            dp_clone.lock().await.update_progress(
                                dl_stats.files_downloaded,
                                total_post_len,
                                dl_stats.bytes_downloaded,
                            );
                        }
                        utils::DownloadPostResult::ReceivedNotFound => {
                            ss_clone.lock().await.file_cache.files.push(FileCacheItem {
                                id: post.id.clone(),
                                created_utc: post.created_utc,
                                title: post.title.clone(),
                                subreddit: post.subreddit.clone(),
                                url: post.url.clone(),
                                success: false,
                                index: post.index,
                            });
                            let mut dl_stats = ds_clone.lock().await;
                            dl_stats.downloads_failed += 1;
                        }
                        utils::DownloadPostResult::ReceivedFailed => {
                            let mut dl_stats = ds_clone.lock().await;
                            dl_stats.downloads_failed += 1;
                        }

                        utils::DownloadPostResult::ReceivedUnhandled => {
                            // Do nothing
                        }
                    }
                }
                Err(_) => {
                    let mut dl_stats = ds_clone.lock().await;
                    dl_stats.downloads_failed += 1;
                }
            }
            drop(permit);
        })
        .await?;
    }

    tx.send(true)
        .map_err(|_| anyhow!("Failed sending to oneshot channel"))?;
    let dl_stats = download_stats.lock().await;
    download_progress.lock().await.post_report(
        dl_stats.files_downloaded,
        total_post_len,
        dl_stats.bytes_downloaded,
    );

    clockwork_orange.await?;

    let file_cache = &shared_state.lock().await.file_cache;
    let cache = serde_json::to_string(file_cache)?;
    fs::write(format!("{}/cache.json", output_folder), cache)?;

    Ok(())
}
