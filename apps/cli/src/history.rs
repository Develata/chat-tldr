//! Protocol framing for read-only history; SQLite remains owned by engine::store.
use std::io::Write;

use chat_tldr_core::{AckPayload, ChatId, EventBody, RunId};
use chat_tldr_engine::store::{self, History};

use crate::{
    Failure,
    args::{RunArgs, StatsArgs},
    output::Output,
    paths::Paths,
};

pub fn decisions<W: Write>(
    args: RunArgs,
    paths: &Paths,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let run = RunId(args.run);
    let history = store::decisions(&paths.database, &run)?;
    emit(
        "decisions",
        Some(run.to_string()),
        history,
        EventBody::Decision,
        output,
    )
}

pub fn jev_log<W: Write>(
    args: RunArgs,
    paths: &Paths,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let run = RunId(args.run);
    let history = store::jev_log(&paths.database, &run)?;
    emit(
        "jev-log",
        Some(run.to_string()),
        history,
        EventBody::JevAnswer,
        output,
    )
}

pub fn stats<W: Write>(
    args: StatsArgs,
    paths: &Paths,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let chat = args.chat.map(ChatId);
    let run = args.run.map(RunId);
    let history = store::stats(&paths.database, chat.as_ref(), run.as_ref())?;
    emit(
        "stats",
        chat.map(|chat| chat.to_string()),
        history,
        EventBody::Stats,
        output,
    )
}

fn emit<T, W: Write>(
    command: &str,
    target: Option<String>,
    history: History<T>,
    event: impl Fn(T) -> EventBody,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    if command != "stats" || target.is_some() {
        output.emit(EventBody::Ack(AckPayload {
            command: command.into(),
            target,
            changed: false,
            detail: history.detail,
        }))?;
    }
    for warning in history.warnings {
        output.emit(EventBody::Warning(warning))?;
    }
    for row in history.rows {
        output.emit(event(row))?;
    }
    Ok(())
}
