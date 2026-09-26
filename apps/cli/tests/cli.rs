use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use chat_tldr_core::{CliEvent, EventStreamValidator};
use serde_json::Value;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let name = format!(
            "chat-tldr-cli-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        );
        let path = std::env::temp_dir().join(name);
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn data_dir(&self) -> PathBuf {
        self.0.join("data")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_chat-tldr"));
        command
            .current_dir(&self.0)
            .arg("--data-dir")
            .arg(self.data_dir())
            .env_remove("TYPESAFE_API_KEY")
            .env_remove("CHAT_TLDR_LLM_API_KEY");
        command
    }

    fn run(&self, args: &[&str], expected_exit: i32) -> Vec<Value> {
        let output = self.command().args(args).output().unwrap();
        events(output, expected_exit)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn events(output: Output, expected_exit: i32) -> Vec<Value> {
    assert_eq!(
        output.status.code(),
        Some(expected_exit),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert!(stdout.ends_with('\n'));
    let mut validator = EventStreamValidator::new();
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| {
            let event: CliEvent = serde_json::from_str(line).unwrap();
            validator.accept(&event).unwrap();
            serde_json::from_str(line).unwrap()
        })
        .collect();
    validator.finish(expected_exit).unwrap();
    assert_eq!(events.last().unwrap()["event"], "done");
    assert_eq!(
        events.last().unwrap()["payload"]["exit_code"],
        expected_exit
    );
    events
}

fn payload<'a>(events: &'a [Value], event: &str) -> &'a Value {
    &events.iter().find(|value| value["event"] == event).unwrap()["payload"]
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/qce/synthetic-group.json")
}

#[test]
fn version_bypasses_configuration_and_reports_only_working_commands() {
    let sandbox = Sandbox::new();
    let events = sandbox.run(&["--config", "does-not-exist.toml", "version"], 0);
    assert_eq!(events.len(), 2);
    let version = payload(&events, "ack");
    assert_eq!(version["detail"]["schema_version"], "1.0");
    let commands = version["detail"]["capabilities"]["commands"]
        .as_array()
        .unwrap();
    assert!(commands.contains(&Value::from("import")));
    assert!(!commands.contains(&Value::from("analyze")));
    assert!(!sandbox.data_dir().exists());
}

#[test]
fn argument_errors_are_jsonl_and_help_is_plain_text() {
    let sandbox = Sandbox::new();
    let events = sandbox.run(&["import"], 2);
    assert_eq!(payload(&events, "error")["code"], "E_USAGE");
    let output = sandbox.command().arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Usage:"));
    assert!(help.contains("--data-dir"));
    assert!(!sandbox.data_dir().exists());
}

#[test]
fn empty_queries_never_initialize_the_database() {
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.run(&["chats"], 0).len(), 1);
    let events = sandbox.run(&["messages", "--chat", "qq:group:missing"], 3);
    assert_eq!(payload(&events, "error")["code"], "E_CHAT_NOT_FOUND");
    assert!(!sandbox.data_dir().exists());
}

#[test]
fn explicit_invalid_configuration_is_an_error() {
    let sandbox = Sandbox::new();
    let events = sandbox.run(&["chats", "--config", "missing.toml"], 4);
    assert_eq!(payload(&events, "error")["code"], "E_CONFIG");
    fs::write(sandbox.0.join("bad.toml"), "timezone = [").unwrap();
    let events = sandbox.run(&["doctor", "--config", "bad.toml"], 4);
    assert_eq!(payload(&events, "error")["code"], "E_CONFIG");
    assert!(!sandbox.data_dir().exists());
}

#[test]
fn configuration_creation_never_overwrites_an_existing_file() {
    let sandbox = Sandbox::new();
    let events = sandbox.run(&["config", "init"], 0);
    assert_eq!(payload(&events, "ack")["changed"], true);
    let path = sandbox.data_dir().join("config.toml");
    let original = fs::read(&path).unwrap();
    sandbox.run(&["doctor"], 4);
    let events = sandbox.run(&["config", "init"], 4);
    assert_eq!(payload(&events, "error")["code"], "E_CONFIG");
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!sandbox.data_dir().join("chat-tldr.db").exists());
}

#[test]
fn config_out_and_global_arguments_resolve_from_the_callers_directory() {
    let sandbox = Sandbox::new();
    let output = Command::new(env!("CARGO_BIN_EXE_chat-tldr"))
        .current_dir(&sandbox.0)
        .args([
            "config",
            "init",
            "--out",
            "space in path/example.toml",
            "--data-dir",
            "relative-data",
        ])
        .output()
        .unwrap();
    let events = events(output, 0);
    let out = payload(&events, "ack")["detail"]["config_file"]
        .as_str()
        .unwrap();
    assert!(Path::new(out).is_absolute());
    assert!(sandbox.0.join("space in path/example.toml").is_file());
    assert!(!sandbox.0.join("relative-data").exists());
}

#[test]
fn doctor_is_offline_and_does_not_expose_credentials() {
    let sandbox = Sandbox::new();
    let without_keys = sandbox.run(&["doctor"], 4);
    let detail = &payload(&without_keys, "ack")["detail"];
    assert_eq!(detail["readiness"]["read"], true);
    assert_eq!(detail["readiness"]["import"], true);
    assert_eq!(detail["providers"]["llm"]["key_present"], false);
    let output = sandbox
        .command()
        .arg("doctor")
        .env("CHAT_TLDR_LLM_API_KEY", "synthetic-secret-must-not-appear")
        .env("TYPESAFE_API_KEY", "synthetic-secret-must-not-appear")
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-secret-must-not-appear"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-secret-must-not-appear"));
    let events = events(output, 0);
    let detail = &payload(&events, "ack")["detail"];
    assert_eq!(detail["providers"]["llm"]["key_present"], true);
    assert_eq!(detail["remote_checked"], false);
    assert_eq!(detail["analyze_implemented"], false);
    assert!(Path::new(detail["paths"]["database"].as_str().unwrap()).is_absolute());
    assert!(!sandbox.data_dir().exists());
}

#[test]
fn an_explicit_config_does_not_relocate_the_data_directory() {
    let sandbox = Sandbox::new();
    sandbox.run(&["config", "init", "--out", "outside.toml"], 0);
    let events = sandbox.run(&["--config", "outside.toml", "config", "init"], 0);
    assert_eq!(payload(&events, "ack")["changed"], true);
    assert!(sandbox.data_dir().join("config.toml").exists());
    assert!(sandbox.0.join("outside.toml").exists());
    assert!(!sandbox.data_dir().join("chat-tldr.db").exists());
}

#[test]
fn invalid_time_filters_fail_before_querying_storage() {
    let sandbox = Sandbox::new();
    for time in ["2026-09-26T12:00:00", "not-a-date"] {
        let events = sandbox.run(&["messages", "--chat", "missing", "--since", time], 2);
        assert_eq!(payload(&events, "error")["code"], "E_USAGE");
    }
    let events = sandbox.run(
        &[
            "messages",
            "--chat",
            "missing",
            "--since",
            "2026-09-27T00:00:00Z",
            "--until",
            "2026-09-26T00:00:00Z",
        ],
        2,
    );
    assert_eq!(payload(&events, "error")["code"], "E_USAGE");
    assert!(!sandbox.data_dir().exists());
}

#[test]
fn import_is_idempotent_and_messages_do_not_need_the_original_file() {
    let sandbox = Sandbox::new();
    let local = sandbox.0.join("export with spaces.json");
    fs::copy(fixture(), &local).unwrap();
    let first = events(
        sandbox
            .command()
            .arg("import")
            .arg(&local)
            .output()
            .unwrap(),
        0,
    );
    assert_eq!(payload(&first, "ack")["changed"], true);
    assert!(payload(&first, "stats")["inserted"].as_u64().unwrap() > 0);
    let chat = payload(&first, "ack")["detail"]["chat_ids"][0]
        .as_str()
        .unwrap();
    let second = events(
        sandbox
            .command()
            .arg("import")
            .arg(&local)
            .output()
            .unwrap(),
        0,
    );
    assert_eq!(payload(&second, "ack")["changed"], false);
    assert_eq!(payload(&second, "stats")["inserted"], 0);
    fs::remove_file(&local).unwrap();
    let chats = sandbox.run(&["chats"], 0);
    assert_eq!(payload(&chats, "chat")["chat_id"], chat);
    let messages = sandbox.run(&["messages", "--chat", chat], 0);
    assert!(messages.iter().any(|event| event["event"] == "message"));
}

#[test]
fn malformed_second_input_rolls_back_the_entire_batch() {
    let sandbox = Sandbox::new();
    let bad = sandbox.0.join("bad.json");
    fs::write(&bad, b"not json").unwrap();
    let events = events(
        sandbox
            .command()
            .arg("import")
            .arg(fixture())
            .arg(&bad)
            .output()
            .unwrap(),
        3,
    );
    assert_eq!(payload(&events, "error")["code"], "E_INPUT_PARSE");
    assert!(!events.iter().any(|event| event["event"] == "ack"));
    assert_eq!(sandbox.run(&["chats"], 0).len(), 1);
}

#[test]
fn identity_warning_has_a_stable_code_and_everyone_mentions_still_work() {
    let sandbox = Sandbox::new();
    let mut data: Value = serde_json::from_slice(&fs::read(fixture()).unwrap()).unwrap();
    let info = data["chatInfo"].as_object_mut().unwrap();
    info.remove("selfUid");
    info.remove("selfUin");
    let input = sandbox.0.join("no-self.json");
    fs::write(&input, serde_json::to_vec(&data).unwrap()).unwrap();
    let imported = events(
        sandbox.command().arg("import").arg(input).output().unwrap(),
        0,
    );
    assert_eq!(payload(&imported, "warning")["code"], "W_SELF_ID_MISSING");
    let chat = payload(&imported, "ack")["detail"]["chat_ids"][0]
        .as_str()
        .unwrap();
    let messages = sandbox.run(&["messages", "--chat", chat], 0);
    assert_eq!(payload(&messages, "message")["mentions_me"], true);
}
