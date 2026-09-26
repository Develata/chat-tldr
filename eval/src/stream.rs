use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use chat_tldr_core::{CliEvent, EventBody, EventStreamValidator, MessagePayload, RunId, RunStatus};
use serde_json::{Value, json};

#[derive(Default)]
struct StreamState {
    events: u64,
    run_id: Option<RunId>,
    done_exit_code: Option<i32>,
}

fn visit(
    path: &Path,
    process_exit_code: Option<i32>,
    mut visitor: impl FnMut(EventBody) -> Result<(), String>,
) -> (StreamState, Result<(), String>) {
    let mut state = StreamState::default();
    let mut validator = EventStreamValidator::new();
    let result = (|| {
        let file =
            File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let number = index + 1;
            let line =
                line.map_err(|error| format!("line {number}: cannot read UTF-8 JSONL: {error}"))?;
            let event: CliEvent = serde_json::from_str(&line)
                .map_err(|error| format!("line {number}: invalid event JSON: {error}"))?;
            validator
                .accept(&event)
                .map_err(|error| format!("line {number}: {error}"))?;
            state.run_id.get_or_insert_with(|| event.run_id.clone());
            state.events += 1;
            if let EventBody::Done(done) = &event.body {
                state.done_exit_code = Some(done.exit_code);
            }
            visitor(event.body).map_err(|error| format!("line {number}: {error}"))?;
        }
        validator
            .finish(process_exit_code.or(state.done_exit_code).unwrap_or(0))
            .map_err(|error| error.to_string())
    })();
    (state, result)
}

pub fn check_stream(path: &Path, process_exit_code: Option<i32>) -> Value {
    let (state, result) = visit(path, process_exit_code, |_| Ok(()));
    json!({
        "command":"check-stream", "valid":result.is_ok(), "events":state.events,
        "run_id":state.run_id, "done_exit_code":state.done_exit_code,
        "process_exit_code":process_exit_code,
        "exit_code_source":if process_exit_code.is_some() { "argument" } else { "stream_only" },
        "error":result.err()
    })
}

pub fn messages(
    path: &Path,
    process_exit_code: Option<i32>,
) -> Result<Vec<MessagePayload>, String> {
    let mut messages = Vec::new();
    let (_, result) = visit(path, process_exit_code, |body| match body {
        EventBody::Message(message) => {
            messages.push(message);
            Ok(())
        }
        EventBody::Done(done) if done.status == RunStatus::Complete && done.exit_code == 0 => {
            Ok(())
        }
        // Future minor-version events still consume their validated sequence
        // number, but do not change the meaning of known message rows.
        EventBody::Unknown { .. } => Ok(()),
        _ => Err(
            "expected a successful messages stream containing only message and done events".into(),
        ),
    });
    result?;
    Ok(messages)
}
