use clap::{builder::EnumValueParser, Arg, ArgAction, Command, ValueEnum};

#[derive(Debug)]
pub struct CliSharedOptions {
    pub concurrency: u16,
    pub mock: Option<String>,
    pub output: String,
    pub skip: bool,
    pub verbose: bool,
}

#[derive(Debug)]
pub struct CliUserCommand {
    pub username: String,
    pub options: CliSharedOptions,
}

#[derive(Debug)]
pub struct CliSearchCommand {
    pub term: String,
    pub category: RedditCategoryFilter,
    pub timeframe: RedditTimeframeFilter,
    pub options: CliSharedOptions,
}

#[derive(Debug)]
pub struct CliSubredditCommand {
    pub subreddit: String,
    pub category: RedditCategoryFilter,
    pub timeframe: RedditTimeframeFilter,
    pub options: CliSharedOptions,
}

#[derive(Debug)]
pub enum CliCommand {
    User(CliUserCommand),
    Search(CliSearchCommand),
    Subreddit(CliSubredditCommand),
}

#[derive(Debug, Clone, PartialEq, Eq, ValueEnum)]
pub enum RedditCategoryFilter {
    Hot,
    New,
    Top,
    Rising,
}

#[derive(Debug, Clone, PartialEq, Eq, ValueEnum)]
pub enum RedditTimeframeFilter {
    Hour,
    Day,
    Week,
    Month,
    Year,
    All,
}

pub fn run() -> CliCommand {
    let shared_args = &[
        Arg::new("verbose")
            .short('v')
            .long("verbose")
            .long_help("Print verbose output")
            .action(ArgAction::SetTrue),
        Arg::new("skip")
            .long("skip")
            .long_help("Skips download tasks for development purposes")
            .action(clap::ArgAction::SetTrue)
            .required(false)
            .hide(true),
        Arg::new("mock")
            .long("mock")
            .long_help("Pass a mock of a Reddit API response for development purposes")
            .action(clap::ArgAction::Set)
            .required(false)
            .hide(true),
        Arg::new("tasks")
            .short('t')
            .long("tasks")
            .long_help("Amount of tasks spawned for download [1-100]")
            .value_name("tasks")
            .value_parser(clap::value_parser!(u16).range(1..=100))
            .default_value("10")
            .action(clap::ArgAction::Set),
        Arg::new("output")
            .short('o')
            .long("output")
            .long_help("File download output directory")
            .value_name("PATH")
            .default_value("output")
            .action(clap::ArgAction::Set),
    ];

    let cmd = Command::new("reddit-clawler")
        .version("0.1.0")
        .about("Crawler for Reddit posts")
        .subcommand_required(true)
        .subcommand(
            Command::new("user")
                .about("Download posts from a specific user")
                .arg(Arg::new("username").required(true).index(1))
                .args(shared_args.clone()),
        )
        .subcommand(
            Command::new("search")
                .about("Download posts from a specific search term")
                .arg(Arg::new("search").required(true).index(1))
                .arg(
                    Arg::new("category")
                        .long("category")
                        .long_help("category for posts")
                        .value_name("hot|new|top|rising")
                        .value_parser(EnumValueParser::<RedditCategoryFilter>::new())
                        .required(true),
                )
                .arg(
                    Arg::new("timeframe")
                        .long("timeframe")
                        .long_help("Timeframe for posts")
                        .value_name("hour|day|week|month|year|all")
                        .value_parser(EnumValueParser::<RedditTimeframeFilter>::new())
                        .required(true),
                )
                .args(shared_args.clone()),
        )
        .subcommand(
            Command::new("subreddit")
                .about("Download posts from a specific subreddit")
                .arg(Arg::new("subreddit").required(true).index(1))
                .arg(
                    Arg::new("category")
                        .long("category")
                        .long_help("category for posts")
                        .value_name("hot|new|top|rising")
                        .value_parser(EnumValueParser::<RedditCategoryFilter>::new())
                        .required(true),
                )
                .arg(
                    Arg::new("timeframe")
                        .long("timeframe")
                        .long_help("Timeframe for posts")
                        .value_name("hour|day|week|month|year|all")
                        .value_parser(EnumValueParser::<RedditTimeframeFilter>::new())
                        .required(true),
                )
                .args(shared_args.clone()),
        );

    let matches = cmd.get_matches();

    let get_shared_options = |m: &clap::ArgMatches| {
        let concurrency = m.get_one::<u16>("tasks").unwrap().to_owned();
        let mock = m.get_one::<String>("mock").cloned();
        let output = m.get_one::<String>("output").unwrap().to_owned();
        let skip = m.get_one::<bool>("skip").unwrap().to_owned();
        let verbose = m.get_one::<bool>("verbose").unwrap().to_owned();

        CliSharedOptions {
            concurrency,
            mock,
            output,
            skip,
            verbose,
        }
    };

    match matches.subcommand() {
        Some(("user", m)) => {
            let username = m.get_one::<String>("username").unwrap().to_string();
            let shared_options = get_shared_options(m);

            CliCommand::User(CliUserCommand {
                username,
                options: shared_options
            })
        }
        Some(("subreddit", m)) => {
            let subreddit = m.get_one::<String>("subreddit").unwrap().to_string();
            let category = m.get_one::<RedditCategoryFilter>("category").unwrap().to_owned();
            let timeframe = m.get_one::<RedditTimeframeFilter>("timeframe").unwrap().to_owned();
            let shared_options = get_shared_options(m);
            CliCommand::Subreddit(CliSubredditCommand {
                subreddit,
                category,
                timeframe,
                options: shared_options
            })
        }
        Some(("search", m)) => {
            let search = m.get_one::<String>("search").unwrap().to_string();
            let category = m.get_one::<RedditCategoryFilter>("category").unwrap().to_owned();
            let timeframe = m.get_one::<RedditTimeframeFilter>("timeframe").unwrap().to_owned();
            let shared_options = get_shared_options(m);
            CliCommand::Search(CliSearchCommand {
                term: search,
                category,
                timeframe,
                options: shared_options
            })
        }
        _ => unreachable!(
            "Subcommand not found. Please file an issue: https://github.com/ecklf/reddit-clawler/issues/new"
        ),
    }
}
