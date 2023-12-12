use crate::{
    cli::CliUserCommand,
    clients::{self, api_types::reddit::submitted_response::RedditSubmittedResponse},
    reddit_parser::RedditPostParser,
    utils::{
        self, download_crawler_post,
        state::{DownloadStats, SharedState},
        DownloadProgress,
    },
};
use anyhow::anyhow;
use owo_colors::OwoColorize;
use spinoff::{spinners, Color, Spinner};
use std::{error::Error, fs, sync::Arc, time::Duration};
use tokio::{
    sync::{oneshot, Mutex, Semaphore},
    time::sleep,
};

pub async fn handle_user_command(
    cmd: CliUserCommand,
    client: &reqwest_middleware::ClientWithMiddleware,
    shared_state: &Arc<Mutex<SharedState>>,
) -> Result<(), Box<dyn Error>> {
    let CliUserCommand { username, options } = cmd;

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
    let output_folder = utils::get_output_folder(&options.output, &username);
    utils::prepare_output_folder(&output_folder)?;

    let responses = match options.mock {
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
            reddit_client
                .get_user_submissions(client, &username)
                .await?
        }
    };

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

    for post in posts {
        let client = client.clone();
        let output_folder = output_folder.clone();

        let dp_clone = Arc::clone(&download_progress);
        let ds_clone = Arc::clone(&download_stats);
        let ss_clone = Arc::clone(shared_state);
        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();

        tokio::spawn(async move {
            match download_crawler_post(&client, &ss_clone, &output_folder, &post).await {
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

                Err(_) => println!("Failed - {}", &output_folder),
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

    Ok(())
}
