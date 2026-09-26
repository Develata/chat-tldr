mod annotation;
mod output;
mod sheet;
mod stream;

use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "chat-tldr-eval",
    version,
    about = "Offline validation tools for chat-tldr evaluation artifacts"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a recorded CLI JSONL stream. This does not compute evaluation scores.
    CheckStream {
        /// UTF-8 JSONL file containing one complete CLI invocation.
        path: PathBuf,
        /// Actual subprocess exit code, if recorded by the caller.
        /// Without this, only the stream's internal consistency is checked.
        #[arg(long, allow_hyphen_values = true)]
        exit_code: Option<i32>,
    },
    /// Export a successful, complete messages JSONL stream as an unlabelled CSV.
    ExportSheet {
        #[arg(long)]
        messages: PathBuf,
        /// New CSV file; existing outputs are never overwritten.
        #[arg(long)]
        out: PathBuf,
        /// Actual process exit code, if recorded; otherwise check stream consistency only.
        #[arg(long, allow_hyphen_values = true)]
        exit_code: Option<i32>,
    },
    /// Validate a labelled CSV and publish messages.jsonl and items.jsonl together.
    ImportSheet {
        sheet: PathBuf,
        /// New gold directory; must not already exist.
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> ExitCode {
    let Args { command } = Args::parse();
    let summary = match command {
        Command::CheckStream { path, exit_code } => stream::check_stream(&path, exit_code),
        Command::ExportSheet {
            messages,
            out,
            exit_code,
        } => sheet::export(&messages, &out, exit_code)
            .unwrap_or_else(|error| json!({"command":"export-sheet","valid":false,"error":error})),
        Command::ImportSheet { sheet, out } => sheet::import(&sheet, &out)
            .unwrap_or_else(|error| json!({"command":"import-sheet","valid":false,"error":error})),
    };
    if let Some(error) = summary["error"].as_str() {
        eprintln!("{error}");
    }
    let mut stdout = io::stdout().lock();
    let written = serde_json::to_writer(&mut stdout, &summary)
        .map_err(io::Error::other)
        .and_then(|()| stdout.write_all(b"\n"))
        .and_then(|()| stdout.flush());
    if let Err(error) = written {
        eprintln!("cannot write validation summary: {error}");
        return ExitCode::FAILURE;
    }
    if summary["valid"] == true {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
