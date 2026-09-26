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
