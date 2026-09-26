mod args;
mod commands;
mod output;
mod paths;

use std::io;
use std::process::ExitCode;

use chat_tldr_core::{ErrorPayload, EventBody};
use clap::{Parser, error::ErrorKind};

use args::Cli;
use output::Output;

#[derive(Debug)]
pub struct Failure {
    code: &'static str,
    exit_code: u8,
    stage: &'static str,
    retryable: bool,
    message: String,
}

impl Failure {
    fn new(code: &'static str, exit_code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            exit_code,
            stage: "cli",
            retryable: false,
            message: message.into(),
        }
    }
}

impl From<chat_tldr_engine::EngineError> for Failure {
    fn from(error: chat_tldr_engine::EngineError) -> Self {
        Self {
            code: error.code(),
            exit_code: error.exit_code(),
            stage: if error.code().starts_with("E_DB") {
                "store"
            } else {
                "cli"
            },
            retryable: matches!(error.code(), "E_DB_BUSY" | "E_RUN_IN_PROGRESS"),
            message: error.to_string(),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::new(
            "E_OUTPUT_WRITE",
            8,
            format!("Cannot write command output: {error}"),
        )
    }
}

fn main() -> ExitCode {
    let cli = Cli::try_parse();
    if let Err(error) = &cli
        && matches!(
            error.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
        )
    {
        return match error.print() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }

    let stdout = io::stdout();
    let mut output = Output::new(stdout.lock());
    let result = match cli {
        Ok(cli) => commands::run(cli, &mut output),
        Err(error) => {
            let _ = error.print();
            Err(Failure::new("E_USAGE", 2, error.to_string()))
        }
    };
    let exit_code = match result {
        Ok(()) => 0,
        Err(error) => {
            let code = error.exit_code;
            if output
                .emit(EventBody::Error(ErrorPayload {
                    stage: error.stage.to_owned(),
                    code: error.code.to_owned(),
                    retryable: error.retryable,
                    message: error.message,
                    topic_id: None,
                }))
                .is_err()
            {
                eprintln!("Cannot write JSONL output; the output pipe may be closed.");
                return ExitCode::from(code);
            }
            code
        }
    };
    if output.done(exit_code).is_err() {
        eprintln!("Cannot write the final JSONL event; the output pipe may be closed.");
        return ExitCode::from(8);
    }
    ExitCode::from(exit_code)
}
