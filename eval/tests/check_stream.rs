use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn new(contents: &[u8]) -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chat-tldr-stream-{}-{id}.jsonl",
            std::process::id()
        ));
        fs::write(&path, contents).unwrap();
        Self(path)
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn run(path: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
        .arg("check-stream")
        .arg(path)
        .args(extra)
        .output()
        .unwrap()
}

fn summary(output: &Output) -> Value {
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/jsonl")
        .join(name)
}

fn modified_stream(mut modify: impl FnMut(&mut Vec<Value>)) -> TemporaryFile {
    let source = fs::read_to_string(fixture("version.jsonl")).unwrap();
    let mut events: Vec<Value> = source
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    modify(&mut events);
    let stream = events
        .into_iter()
        .map(|event| event.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    TemporaryFile::new(stream.as_bytes())
}

#[test]
fn all_shared_synthetic_streams_are_accepted() {
    for name in [
        "version.jsonl",
        "chats.jsonl",
        "inbox.jsonl",
        "inbox-empty.jsonl",
        "analyze-complete.jsonl",
        "analyze-partial.jsonl",
    ] {
        let output = run(&fixture(name), &[]);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = summary(&output);
        assert_eq!(result["valid"], true);
        assert_eq!(result["exit_code_source"], "stream_only");
        assert!(result["process_exit_code"].is_null());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn recorded_process_exit_code_is_checked_without_confusing_run_failure_with_protocol_failure() {
    let matching = run(&fixture("analyze-partial.jsonl"), &["--exit-code", "6"]);
    assert!(matching.status.success());
    assert_eq!(summary(&matching)["process_exit_code"], 6);
    assert_eq!(summary(&matching)["exit_code_source"], "argument");
    let mismatching = run(&fixture("analyze-partial.jsonl"), &["--exit-code", "0"]);
    assert_eq!(mismatching.status.code(), Some(1));
    assert_eq!(summary(&mismatching)["valid"], false);
    assert!(!mismatching.stderr.is_empty());
}

#[test]
fn missing_done_or_event_after_done_fails() {
    let truncated = modified_stream(|events| {
        events.pop();
    });
    let output = run(&truncated.0, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        summary(&output)["error"]
            .as_str()
            .unwrap()
            .contains("without done")
    );
    let extra = modified_stream(|events| events.push(events[0].clone()));
    let output = run(&extra.0, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        summary(&output)["error"]
            .as_str()
            .unwrap()
            .contains("after done")
    );
}

#[test]
fn schema_sequence_and_run_changes_fail() {
    for field in ["schema_version", "seq", "run_id"] {
        let file = modified_stream(|events| {
            events[1][field] = match field {
                "schema_version" => json!("2.0"),
                "seq" => json!(9),
                _ => json!("r_other"),
            }
        });
        let output = run(&file.0, &[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(summary(&output)["valid"], false);
    }
}

#[test]
fn invalid_utf8_json_blank_lines_and_empty_file_fail() {
    for content in [b"\xff\n".as_slice(), b"{\n", b"\n", b""] {
        let file = TemporaryFile::new(content);
        let output = run(&file.0, &[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(summary(&output)["valid"], false);
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn missing_file_reports_a_json_failure() {
    let output = run(&fixture("intentionally-missing.jsonl"), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(summary(&output)["events"], 0);
}

#[test]
fn unknown_command_is_rejected_and_help_is_text() {
    let unknown = Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
        .arg("score")
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert!(!unknown.stderr.is_empty());
    let help = Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("check-stream"));
}
