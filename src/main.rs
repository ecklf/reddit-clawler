use reddit_clawler::{
    cli,
    utils::{self, state::SharedState},
};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use std::{error::Error, sync::Arc};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Checks for dependencies that will be used in future versions
    utils::check_deps()?;
    // Checks for file_scheme that will be used in future version
    // let file_scheme = String::from("{UPVOTES}__ID}_{AUTHOR}_{POSTID}_{DATE}");
    // check_file_scheme(&file_scheme);
    let cli_request = cli::run();

    // Create client and state that is shared between tokio tasks
    // Retries up to 3 times with increasing intervals between attempts
    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);

    let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/96.0.4664.110 Safari/537.36";
    let client = ClientBuilder::new(
        reqwest::Client::builder()
            .user_agent(user_agent)
            .build()
            .unwrap(),
    )
    .with(RetryTransientMiddleware::new_with_policy(retry_policy))
    .build();

    // Shared state between tokio tasks e.g. caching an authorization token
    let shared_state: Arc<Mutex<SharedState>> = Arc::new(Mutex::new(SharedState::default()));

    match cli_request {
        cli::CliCommand::User(cmd) => {
            cli::handle_user_command(cmd, &client, &shared_state).await?;
        }

        cli::CliCommand::Subreddit(cmd) => {
            cli::handle_subreddit_command(cmd, &client, &shared_state).await?;
        }

        cli::CliCommand::Search(cmd) => {
            cli::handle_search_command(cmd, &client, &shared_state).await?;
        }
    }

    Ok(())
}
