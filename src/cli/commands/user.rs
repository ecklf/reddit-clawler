use crate::{
    cli::CliRedditCommand,
    clients::{self, api_types::reddit::submitted_response::RedditSubmittedResponse},
    reddit_parser::RedditPostParser,
    utils::{
        self, download_crawler_post,
        state::{
            DownloadStats, FileCacheItemLatest, FileCacheLatest, LastDownloadStatus,
            ResourceStatus, SharedState,
        },
        DownloadProgress,
    },
};
use anyhow::anyhow;
use owo_colors::OwoColorize;
use spinoff::{spinners, Color, Spinner};
use std::{error::Error, fs, mem, path::Path, str::FromStr, sync::Arc, time::Duration};
use tokio::{
    sync::{oneshot, Mutex, Semaphore},
    time::sleep,
};

pub async fn handle_user_command(
    cmd: CliRedditCommand,
    client: &reqwest_middleware::ClientWithMiddleware,
    shared_state: &Arc<Mutex<SharedState>>,
) -> Result<(), Box<dyn Error>> {
    let CliRedditCommand {
        resource: ref username,
        ref options,
        ..
    } = cmd;

    let (tx, mut rx) = oneshot::channel::<bool>();
    let reddit_client = clients::RedditClient::default();
    let reddit_parser = RedditPostParser::default();

    let mut spinner = Spinner::new(
        spinners::Dots,
        format!("Fetching posts from {}{}", "/u/".bold(), username.bold()),
        Color::TrueColor {
            r: 237,
            g: 106,
            b: 44,
        },
    );

    let stem = format!("user/{}", username);
    let output_folder = utils::get_output_folder(&options.output, &stem);

    utils::prepare_output_folder(&output_folder)?;

    let file_cache_path = format!("{}/cache.json", output_folder);

    if Path::new(&file_cache_path).exists() {
        let file_cache = fs::read_to_string(format!("{}/cache.json", output_folder)).unwrap();
        let file_cache = FileCacheLatest::from_str(&file_cache)?;

        let mut ss = shared_state.lock().await;
        ss.file_cache_path = Some(file_cache_path.clone());
        ss.file_cache = file_cache.clone();

        if !options.force
            && (file_cache.status.resource == ResourceStatus::Deleted
                || file_cache.status.resource == ResourceStatus::Suspended)
        {
            let issue = match file_cache.status.resource {
                ResourceStatus::Deleted => "deleted",
                ResourceStatus::Suspended => "suspended",
                _ => unreachable!(),
            };
            ss.file_cache.status.last_download = LastDownloadStatus::Success;
            fs::write(&file_cache_path, serde_json::to_string(&ss.file_cache)?)?;
            spinner.fail(&format!(
                "The user, {} has been marked as {} in cache. Skipping download",
                &username, issue
            ));
            return Ok(());
        }
    }

    let responses = match &options.mock {
        Some(mock_file) => {
            println!(
                "{}",
                format_args!("{} {}", "[FLAG]".red().bold(), "Mock mode enabled".bold()),
            );

            let file = fs::read_to_string(mock_file)
                .map_err(|e| format!("Failed to read mock file: {}", e))?;

            serde_json::from_str::<Vec<RedditSubmittedResponse>>(&file)
                .expect("Failed to parse mock file")
        }
        _ => {
            let response = reddit_client
                .get_user_submissions(client, shared_state, &cmd, options)
                .await;

            match response {
                Ok(responses) => {
                    let mut ss = shared_state.lock().await;
                    ss.file_cache.status.resource = ResourceStatus::Active;
                    ss.file_cache.status.last_download = LastDownloadStatus::Success;
                    fs::write(&file_cache_path, serde_json::to_string(&ss.file_cache)?)?;
                    responses
                }
                Err(e) => match e {
                    clients::RedditProviderError::NotFound => {
                        let mut ss = shared_state.lock().await;
                        ss.file_cache.status.resource = ResourceStatus::Deleted;
                        ss.file_cache.status.last_download = LastDownloadStatus::Success;
                        fs::write(&file_cache_path, serde_json::to_string(&ss.file_cache)?)?;
                        spinner.fail(&format!(
                            "The user, {} has been deleted. Skipping download - cache updated",
                            &username
                        ));
                        return Ok(());
                    }
                    clients::RedditProviderError::Suspended => {
                        let mut ss = shared_state.lock().await;
                        ss.file_cache.status.resource = ResourceStatus::Suspended;
                        ss.file_cache.status.last_download = LastDownloadStatus::Success;
                        fs::write(&file_cache_path, serde_json::to_string(&ss.file_cache)?)?;
                        spinner.fail(&format!(
                            "The user, {} has been suspended. Skipping download - cache updated",
                            &username
                        ));
                        return Ok(());
                    }
                    clients::RedditProviderError::TooManyRequests => {
                        let mut ss = shared_state.lock().await;
                        ss.file_cache.status.last_download = LastDownloadStatus::RateLimit;
                        fs::write(&file_cache_path, serde_json::to_string(&ss.file_cache)?)?;
                        return Err(Box::new(e));
                    }
                    clients::RedditProviderError::Forbidden => {
                        let mut ss = shared_state.lock().await;
                        ss.file_cache.status.last_download = LastDownloadStatus::Forbidden;
                        fs::write(&file_cache_path, serde_json::to_string(&ss.file_cache)?)?;
                        return Err(Box::new(e));
                    }
                    _ => {
                        let mut ss = shared_state.lock().await;
                        ss.file_cache.status.last_download = LastDownloadStatus::Error;
                        fs::write(&file_cache_path, serde_json::to_string(&ss.file_cache)?)?;
                        return Err(Box::new(e));
                    }
                },
            }
        }
    };

    let posts = responses
        .iter()
        .flat_map(|r| reddit_parser.parse(r))
        .collect::<Vec<_>>();

    let mut posts_to_download = posts.clone();

    if Path::new(&file_cache_path).exists() {
        let ss = shared_state.lock().await;
        posts_to_download = posts_to_download
            .into_iter()
            .filter(|p| {
                // Try to find the successfully downloaded post in the cache
                let found = ss
                    .file_cache
                    .files
                    .iter()
                    .any(|f| p.id == f.id && f.success);
                !found
            })
            .collect::<Vec<_>>();
    }

    let ss = shared_state.lock().await;
    spinner.success(&format!(
        "Done, trying to download {} posts. - cached {}",
        posts_to_download.len(),
        ss.file_cache.files.len()
    ));
    mem::drop(ss);

    let download_stats: Arc<Mutex<DownloadStats>> = Arc::new(Mutex::new(DownloadStats::default()));
    let total_post_len = posts_to_download.len() as u64;
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

                            ss_clone
                                .lock()
                                .await
                                .file_cache
                                .files
                                .push(FileCacheItemLatest {
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
                            ss_clone
                                .lock()
                                .await
                                .file_cache
                                .files
                                .push(FileCacheItemLatest {
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

    let ss = &shared_state.lock().await;
    let cache = serde_json::to_string(&ss.file_cache)?;
    fs::write(file_cache_path, cache)?;

    Ok(())
}
