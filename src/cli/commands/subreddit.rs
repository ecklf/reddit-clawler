use crate::{
    cli::CliSubredditCommand,
    clients::{self},
    reddit_parser::RedditPostParser,
    utils::{
        self,
        state::{DownloadStats, SharedState},
        write_crawler_post, DownloadProgress,
    },
};
use owo_colors::OwoColorize;
use spinoff::{spinners, Color, Spinner};
use std::{error::Error, sync::Arc};
use tokio::sync::{Mutex, Semaphore};

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

    spinner.success(&format!("Done, trying to download {} posts", posts.len()));

    let download_stats: Arc<Mutex<DownloadStats>> = Arc::new(Mutex::new(DownloadStats::default()));
    let total_post_len = posts.len() as u64;
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

    for post in posts {
        let client = client.clone();
        let output_folder = output_folder.clone();

        let dp_clone = Arc::clone(&download_progress);
        let ds_clone = Arc::clone(&download_stats);
        let ss_clone = Arc::clone(shared_state);
        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();

        tokio::spawn(async move {
            match write_crawler_post(&client, &ss_clone, &output_folder, &post).await {
                Ok(bytes) => {
                    if let Some(bytes) = bytes {
                        let mut dl_stats = ds_clone.lock().await;
                        dl_stats.files_downloaded += 1;
                        dl_stats.bytes_downloaded += bytes;

                        dp_clone.lock().await.update_progress(
                            dl_stats.files_downloaded,
                            total_post_len,
                            dl_stats.bytes_downloaded,
                        );
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

    let dl_stats = download_stats.lock().await;
    download_progress.lock().await.post_report(
        dl_stats.files_downloaded,
        total_post_len,
        dl_stats.bytes_downloaded,
    );
    Ok(())
}
