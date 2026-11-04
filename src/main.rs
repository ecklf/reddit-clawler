use reddit_clawler::{
    cli, http,
    utils::{self, state::SharedState},
};
use std::{error::Error, process::ExitCode, sync::Arc};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    // Checks for dependencies that will be used in future versions
    utils::check_deps()?;
    // Checks for file_scheme that will be used in future version
    // let file_scheme = String::from("{UPVOTES}__ID}_{AUTHOR}_{POSTID}_{DATE}");
    // check_file_scheme(&file_scheme);
    let cli_request = cli::run();

    let cookies_path = match &cli_request {
        cli::CliCommand::User(cmd)
        | cli::CliCommand::Search(cmd)
        | cli::CliCommand::Subreddit(cmd) => cmd.options.cookies.as_deref(),
    };
    let cookie_jar = match cookies_path {
        Some(path) => Arc::new(utils::load_cookie_file(path)?),
        None => Arc::new(wreq::cookie::Jar::default()),
    };

    let client = http::build_client(cookie_jar.clone())?;

    // Share the jar so GraphQL token renewal and media requests see the same
    // session without re-reading or writing the exported credentials file.
    let shared_state: Arc<Mutex<SharedState>> = Arc::new(Mutex::new(SharedState {
        reddit_cookie_jar: Some(cookie_jar),
        ..SharedState::default()
    }));

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
