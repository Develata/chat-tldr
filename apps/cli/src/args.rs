use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "chat-tldr",
    version,
    about = "Local chat import and analysis tools; business output is UTF-8 JSONL"
)]
pub struct Cli {
    /// Root directory for the database, configuration, and component files.
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,
    /// Use this configuration file without changing the data directory.
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Increase diagnostic verbosity. Logs always go to stderr.
    #[arg(short, action = ArgAction::Count, global = true)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the machine-readable protocol version and implemented capabilities.
    Version,
    /// Manage configuration files.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Check local configuration and paths without contacting model services.
    Doctor,
    /// Import one or more completed QCE JSON exports as one atomic batch.
    Import(ImportArgs),
    /// List imported chats without creating an empty database.
    Chats,
    /// List rendered messages in a chat, ordered by cursor.
    Messages(MessagesArgs),
    /// Analyze pending messages and retain completed checkpoints on failure.
    Analyze(AnalyzeArgs),
    /// Read a consistent inbox snapshot with evidence.
    Inbox(InboxArgs),
    /// Set the current preference vote for an insight.
    Feedback(FeedbackArgs),
    /// Set the lifecycle of an insight.
    Resolve(ResolveArgs),
    /// Mark only the safe prefix returned by inbox as reviewed.
    MarkRead(MarkReadArgs),
    /// Replay a saved run's controller decisions.
    Decisions(RunArgs),
    /// Export saved model answers and their subject mappings.
    JevLog(RunArgs),
    /// Query stored usage and counts without contacting providers.
    Stats(StatsArgs),
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Create the example configuration. Existing files are never overwritten.
    Init {
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    #[arg(required = true, num_args = 1.., value_name = "PATH")]
    pub paths: Vec<PathBuf>,
    /// Persist the current user's QQNT UID for every imported chat.
    #[arg(long)]
    pub self_uid: Option<String>,
    /// Persist the current user's QQ number for every imported chat.
    #[arg(long)]
    pub self_uin: Option<String>,
}

#[derive(Debug, Args)]
pub struct MessagesArgs {
    #[arg(long, value_name = "ID")]
    pub chat: String,
    /// Inclusive RFC 3339 timestamp, including an explicit timezone offset.
    #[arg(long, value_name = "TIME")]
    pub since: Option<String>,
    /// Exclusive RFC 3339 timestamp, including an explicit timezone offset.
    #[arg(long, value_name = "TIME")]
    pub until: Option<String>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[arg(long, value_name = "RUN_ID")]
    pub run: String,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    #[arg(long, value_name = "ID", conflicts_with = "run")]
    pub chat: Option<String>,
    #[arg(long, value_name = "RUN_ID")]
    pub run: Option<String>,
}

#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    #[arg(long, value_name = "ID")]
    pub chat: String,
    #[arg(long, value_name = "TIME")]
    pub since: Option<String>,
    #[arg(long, value_name = "TIME")]
    pub until: Option<String>,
    #[arg(long, default_value = "jev", value_parser = ["jev", "llm"])]
    pub decider: String,
    #[arg(long, default_value = "ours")]
    pub strategy: String,
    #[arg(long)]
    pub max_steps: Option<u32>,
    #[arg(long, allow_hyphen_values = true)]
    pub budget_usd: Option<f64>,
    #[arg(long, value_name = "FILE", conflicts_with = "dry_run")]
    pub html: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct InboxArgs {
    #[arg(long, value_name = "ID")]
    pub chat: String,
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub include_resolved: bool,
    #[arg(long)]
    pub include_rejected: bool,
    #[arg(long, value_name = "FILE")]
    pub html: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[group(skip)]
#[command(group(clap::ArgGroup::new("feedback_choice").required(true).multiple(false).args(["useful", "not_important"])))]
pub struct FeedbackArgs {
    #[arg(value_name = "INSIGHT_ID")]
    pub id: String,
    #[arg(long)]
    pub useful: bool,
    #[arg(long)]
    pub not_important: bool,
}

#[derive(Debug, Args)]
#[group(skip)]
#[command(group(clap::ArgGroup::new("resolve_choice").required(true).multiple(false).args(["done", "dismiss", "reopen"])))]
pub struct ResolveArgs {
    #[arg(value_name = "INSIGHT_ID")]
    pub id: String,
    #[arg(long)]
    pub done: bool,
    #[arg(long)]
    pub dismiss: bool,
    #[arg(long)]
    pub reopen: bool,
}

#[derive(Debug, Args)]
pub struct MarkReadArgs {
    #[arg(long, value_name = "ID")]
    pub chat: String,
    #[arg(long, value_name = "CURSOR")]
    pub up_to: String,
}
