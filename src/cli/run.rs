use clap::{builder::EnumValueParser, Arg, ArgAction, Command, ValueEnum};
use owo_colors::OwoColorize;
use std::fmt;

#[derive(Debug)]
pub struct CliSharedOptions {
    pub concurrency: u16,
    pub mock: Option<String>,
    pub output: String,
    pub skip: bool,
    pub verbose: bool,
    pub limit: Option<u32>,
}

#[derive(Debug)]
pub struct CliRedditCommand {
    pub resource: String,
    pub category: RedditCategoryFilter,
    pub timeframe: RedditTimeframeFilter,
    pub options: CliSharedOptions,
}

#[derive(Debug)]
pub enum CliCommand {
    User(CliRedditCommand),
    Search(CliRedditCommand),
    Subreddit(CliRedditCommand),
}

#[derive(Debug, Clone, PartialEq, Eq, ValueEnum)]
pub enum RedditCategoryFilter {
    Hot,
    New,
    Top,
    Rising,
    Controversial,
}

impl fmt::Display for RedditCategoryFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let category_str = match self {
            RedditCategoryFilter::Hot => "hot",
            RedditCategoryFilter::New => "new",
            RedditCategoryFilter::Top => "top",
            RedditCategoryFilter::Rising => "rising",
            RedditCategoryFilter::Controversial => "controversial",
        };
        write!(f, "{}", category_str)
    }
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

impl fmt::Display for RedditTimeframeFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let timeframe_str = match self {
            RedditTimeframeFilter::Hour => "hour".to_string(),
            RedditTimeframeFilter::Day => "day".to_string(),
            RedditTimeframeFilter::Week => "week".to_string(),
            RedditTimeframeFilter::Month => "month".to_string(),
            RedditTimeframeFilter::Year => "year".to_string(),
            RedditTimeframeFilter::All => "all".to_string(),
        };
        write!(f, "{}", timeframe_str)
    }
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
        Arg::new("limit")
            .short('l')
            .long("limit")
            .long_help("Limit of fetch requests")
            .value_name("limit")
            .value_parser(clap::value_parser!(u32))
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
                .arg(Arg::new("resource").required(true).index(1))
                .arg(
                    Arg::new("category")
                        .long("category")
                        .long_help("Category for posts")
                        .value_name("hot|new|rising|top|controversial")
                        .value_parser(EnumValueParser::<RedditCategoryFilter>::new())
                        .required(true),
                )
                .arg(
                    Arg::new("timeframe")
                        .long("timeframe")
                        .long_help(
                            "Timeframe for posts - needed when using category top|controversial",
                        )
                        .value_name("hour|day|week|month|year|all")
                        .value_parser(EnumValueParser::<RedditTimeframeFilter>::new())
                        .required_if_eq("category", "top")
                        .required_if_eq("category", "controversial"),
                )
                .args(shared_args.clone()),
        )
        .subcommand(
            Command::new("search")
                .about("Download posts from a specific search term")
                .arg(Arg::new("resource").required(true).index(1))
                .arg(
                    Arg::new("category")
                        .long("category")
                        .long_help("Category for posts")
                        .value_name("hot|new|rising|top|controversial")
                        .value_parser(EnumValueParser::<RedditCategoryFilter>::new())
                        .required(true),
                )
                .arg(
                    Arg::new("timeframe")
                        .long("timeframe")
                        .long_help(
                            "Timeframe for posts - needed when using category top|controversial",
                        )
                        .value_name("hour|day|week|month|year|all")
                        .value_parser(EnumValueParser::<RedditTimeframeFilter>::new())
                        .required_if_eq("category", "top")
                        .required_if_eq("category", "controversial"),
                )
                .args(shared_args.clone()),
        )
        .subcommand(
            Command::new("subreddit")
                .about("Download posts from a specific subreddit")
                .arg(Arg::new("resource").required(true).index(1))
                .arg(
                    Arg::new("category")
                        .long("category")
                        .long_help("Category for posts")
                        .value_name("hot|new|rising|top|controversial")
                        .value_parser(EnumValueParser::<RedditCategoryFilter>::new())
                        .required(true),
                )
                .arg(
                    Arg::new("timeframe")
                        .long("timeframe")
                        .long_help(
                            "Timeframe for posts - needed when using category top|controversial",
                        )
                        .value_name("hour|day|week|month|year|all")
                        .value_parser(EnumValueParser::<RedditTimeframeFilter>::new())
                        .required_if_eq("category", "top")
                        .required_if_eq("category", "controversial"),
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
        let limit = m.get_one::<u32>("limit").copied();

        CliSharedOptions {
            concurrency,
            mock,
            output,
            skip,
            verbose,
            limit,
        }
    };

    let get_inputs = |m: &clap::ArgMatches| -> (
        String,
        RedditCategoryFilter,
        RedditTimeframeFilter,
        CliSharedOptions,
    ) {
        let resource = m.get_one::<String>("resource").unwrap().to_string();
        let category = m
            .get_one::<RedditCategoryFilter>("category")
            .unwrap()
            .to_owned();

        let timeframe = match category {
            RedditCategoryFilter::Hot
            | RedditCategoryFilter::New
            | RedditCategoryFilter::Rising => {
                let category = category.to_string();
                if let Some(tf) = m.get_one::<RedditTimeframeFilter>("timeframe") {
                    println!(
                        "Unncessary timeframe {} for category {} provided - ignoring",
                        tf.bold(),
                        category.bold()
                    );
                };
                RedditTimeframeFilter::All
            }
            _ => m
                .get_one::<RedditTimeframeFilter>("timeframe")
                .unwrap()
                .to_owned(),
        };

        let shared_options = get_shared_options(m);
        (resource, category, timeframe, shared_options)
    };

    match matches.subcommand() {
        Some(("user", m)) => {
            let (resource, category, timeframe, options)= get_inputs(m);
            CliCommand::User(CliRedditCommand {
                resource,
                category,
                timeframe,
                options
            })
        }
        Some(("subreddit", m)) => {
            let (resource, category, timeframe, options)= get_inputs(m);
            CliCommand::Subreddit(CliRedditCommand {
                resource,
                category,
                timeframe,
                options
            })
        }
        Some(("search", m)) => {
            let (resource, category, timeframe, options)= get_inputs(m);
            CliCommand::Search(CliRedditCommand {
                resource,
                category,
                timeframe,
                options
            })
        }
        _ => unreachable!(
            "Subcommand not found. Please file an issue: https://github.com/ecklf/reddit-clawler/issues/new"
        ),
    }
}
