use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
    process::ExitCode,
};

use chat_tldr_core::{CliEvent, EventBody, EventStreamValidator};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

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
}

fn main() -> ExitCode {
    let Args { command } = Args::parse();
    let Command::CheckStream { path, exit_code } = command;
    let summary = check_stream(&path, exit_code);
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

fn check_stream(path: &std::path::Path, process_exit_code: Option<i32>) -> Value {
    let mut validator = EventStreamValidator::new();
    let mut events = 0_u64;
    let mut run_id = None;
    let mut done_exit_code = None;
    let result = (|| -> Result<(), String> {
        let file =
            File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line_number = index + 1;
            let line = line
                .map_err(|error| format!("line {line_number}: cannot read UTF-8 JSONL: {error}"))?;
            let event: CliEvent = serde_json::from_str(&line)
                .map_err(|error| format!("line {line_number}: invalid event JSON: {error}"))?;
            validator
                .accept(&event)
                .map_err(|error| format!("line {line_number}: {error}"))?;
            run_id.get_or_insert_with(|| event.run_id.clone());
            events += 1;
            if let EventBody::Done(done) = event.body {
                done_exit_code = Some(done.exit_code);
            }
        }
        // The fallback checks file consistency only. With no done event, finish
        // reports MissingDone before consulting this sentinel exit code.
        validator
            .finish(process_exit_code.or(done_exit_code).unwrap_or(0))
            .map_err(|error| error.to_string())
    })();
    json!({
        "command": "check-stream",
        "valid": result.is_ok(),
        "events": events,
        "run_id": run_id,
        "done_exit_code": done_exit_code,
        "process_exit_code": process_exit_code,
        "exit_code_source": if process_exit_code.is_some() { "argument" } else { "stream_only" },
        "error": result.err(),
    })
}
