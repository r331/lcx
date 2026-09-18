mod cache;
mod client;
mod commands;
mod config;
mod lang;
mod render;
mod solution;
mod sets;
mod tui;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use crate::cache::Cache;
use crate::client::LeetCodeClient;
use crate::config::Config;

/// A command-line client for LeetCode. Run with no subcommand to open the
/// interactive TUI.
#[derive(Debug, Parser)]
#[command(name = "lcx", version, about = "A command-line client for LeetCode", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Save your LeetCode session credentials.
    Login(LoginArgs),
    /// Show the currently authenticated user.
    Whoami,
    /// List problems (cached). Use `cache --update` to refresh.
    List(ListArgs),
    /// Show a problem's description.
    Show(ShowArgs),
    /// Generate a solution file for a problem and open it.
    Pick(PickArgs),
    /// Open an existing (or newly generated) solution file in your editor.
    Edit(PickArgs),
    /// Run a solution against sample or custom test cases.
    Test(TestArgs),
    /// Submit a solution to LeetCode.
    Submit(SubmitArgs),
    /// Show today's daily challenge.
    Daily(DailyArgs),
    /// Manage the local problem cache.
    Cache(CacheArgs),
    /// View or change configuration.
    Config(ConfigArgs),
    /// Manage curated and custom problem sets.
    Sets(SetsArgs),
}

#[derive(Debug, Args)]
struct SetsArgs {
    #[command(subcommand)]
    action: SetsAction,
}

#[derive(Debug, Subcommand)]
enum SetsAction {
    /// List available sets.
    List,
    /// Create an empty custom or company set.
    Create { name: String },
    /// Import lines of slug,category or LeetCode problem URLs.
    Import { name: String, file: std::path::PathBuf },
    /// Add a LeetCode problem slug to a custom set.
    Add {
        name: String,
        slug: String,
        #[arg(long, default_value = "Other")]
        category: String,
    },
    /// Remove a problem from a custom set.
    Remove { name: String, slug: String },
    /// Delete a custom set.
    Delete { name: String },
}

#[derive(Debug, Args)]
struct LoginArgs {
    /// LEETCODE_SESSION cookie value.
    #[arg(long)]
    session: Option<String>,
    /// csrftoken cookie value.
    #[arg(long)]
    csrf: Option<String>,
    /// Optional Cloudflare cf_clearance cookie value.
    #[arg(long)]
    cf_clearance: Option<String>,
}

#[derive(Debug, Args)]
struct ListArgs {
    /// Filter by difficulty: easy, medium, hard.
    #[arg(short, long)]
    difficulty: Option<String>,
    /// Filter by topic tag slug (e.g. array, dynamic-programming).
    #[arg(short, long)]
    tag: Option<String>,
    /// Filter by status: solved, todo, attempted.
    #[arg(short, long)]
    status: Option<String>,
    /// Search text in title/slug/id.
    #[arg(short, long)]
    query: Option<String>,
    /// Maximum rows to display.
    #[arg(short, long, default_value_t = 50)]
    limit: i64,
}

#[derive(Debug, Args)]
struct ShowArgs {
    /// Problem id or title slug.
    key: String,
    /// Language slug override (e.g. rust, python3).
    #[arg(short, long)]
    lang: Option<String>,
    /// Also print the starter code snippet.
    #[arg(long)]
    code: bool,
}

#[derive(Debug, Args)]
struct PickArgs {
    /// Problem id or title slug.
    key: String,
    /// Language slug override (e.g. rust, python3).
    #[arg(short, long)]
    lang: Option<String>,
    /// Do not open the editor after generating.
    #[arg(long)]
    no_open: bool,
}

#[derive(Debug, Args)]
struct TestArgs {
    /// Problem id, slug, or path to a solution file.
    target: String,
    /// Custom test input (defaults to the problem's sample cases).
    #[arg(short, long)]
    case: Option<String>,
    /// Language slug override.
    #[arg(short, long)]
    lang: Option<String>,
}

#[derive(Debug, Args)]
struct SubmitArgs {
    /// Problem id, slug, or path to a solution file.
    target: String,
    /// Language slug override.
    #[arg(short, long)]
    lang: Option<String>,
}

#[derive(Debug, Args)]
struct DailyArgs {
    /// Generate and open a solution file for today's problem.
    #[arg(long)]
    pick: bool,
}

#[derive(Debug, Args)]
struct CacheArgs {
    /// Refresh the problem list from LeetCode.
    #[arg(long)]
    update: bool,
    /// Clear the cached problem list.
    #[arg(long)]
    clear: bool,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    #[command(subcommand)]
    action: Option<ConfigAction>,
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Set a configuration value (lang, editor, workspace).
    Set { key: String, value: String },
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return launch_tui();
    };
    match command {
        Command::Login(a) => commands::auth::login(a.session, a.csrf, a.cf_clearance).await,
        Command::Whoami => commands::auth::whoami().await,
        Command::List(a) => {
            commands::list::run(a.difficulty, a.tag, a.status, a.query, a.limit).await
        }
        Command::Show(a) => commands::show::run(&a.key, a.lang, a.code).await,
        Command::Pick(a) => commands::pick::run(&a.key, a.lang, !a.no_open).await,
        Command::Edit(a) => commands::pick::run(&a.key, a.lang, !a.no_open).await,
        Command::Test(a) => commands::judge::test(&a.target, a.case, a.lang).await,
        Command::Submit(a) => commands::judge::submit(&a.target, a.lang).await,
        Command::Daily(a) => commands::daily::run(a.pick).await,
        Command::Cache(a) => commands::cache_cmd::run(a.update, a.clear).await,
        Command::Config(a) => match a.action {
            Some(ConfigAction::Set { key, value }) => commands::config_cmd::set(&key, &value),
            None => commands::config_cmd::show(),
        },
        Command::Sets(a) => match a.action {
            SetsAction::List => {
                for set in sets::all()? {
                    println!("{} ({} problems)", set.name, set.entries.len());
                }
                Ok(())
            }
            SetsAction::Create { name } => sets::create(&name),
            SetsAction::Import { name, file } => {
                let count = sets::import_file(&name, &file)?;
                println!("Imported {count} problems into {name}.");
                Ok(())
            }
            SetsAction::Add {
                name,
                slug,
                category,
            } => sets::add(&name, &slug, &category),
            SetsAction::Remove { name, slug } => sets::remove(&name, &slug),
            SetsAction::Delete { name } => sets::delete(&name),
        },
    }
}

/// Launch the interactive TUI (default when no subcommand is given).
fn launch_tui() -> Result<()> {
    let cfg = Config::load()?;
    let client = LeetCodeClient::from_config(&cfg)?;
    let cache = Cache::open()?;
    tui::run(cfg, client, cache)
}
