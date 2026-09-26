use crate::{Failure, args::MessagesArgs, commands::parse_time, output::Output, paths::Paths};
use chat_tldr_core::{AckPayload, ChatId, EventBody};
use chat_tldr_engine::store;
use serde_json::json;
use std::io::Write;

pub fn run<W: Write>(
    args: MessagesArgs,
    paths: &Paths,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let since = parse_time(args.since.as_deref(), "--since")?;
    let until = parse_time(args.until.as_deref(), "--until")?;
    let report = store::relations(&paths.database, &ChatId(args.chat.clone()), since, until)?;
    let emit =
        |output: &mut Output<W>, command: &str, detail: serde_json::Value| -> Result<(), Failure> {
            output.emit(EventBody::Ack(AckPayload {
                command: command.into(),
                target: Some(args.chat.clone()),
                changed: false,
                detail,
            }))?;
            Ok(())
        };
    emit(
        output,
        "relations",
        json!({"since":since,"until":until,"messages":report.messages,
        "analyzed_messages":report.analyzed_messages,"uncovered_messages":report.uncovered_messages,
        "relations":report.relations.len(),"questions":report.questions.len(),
        "verification":"source_quotes_only","pending_scope":"analyzed_records_with_supplied_context"}),
    )?;
    for row in report.relations {
        emit(output, "relations.row", json!({"relation":row}))?;
    }
    for row in report.questions {
        emit(output, "relations.question", json!({"question":row}))?;
    }
    Ok(())
}
