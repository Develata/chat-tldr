use super::*;
use serde_json::json;
use std::{io::Write, process::Stdio};

fn update(sandbox: &Sandbox, value: &Value, exit: i32) -> Vec<Value> {
    let mut child = sandbox
        .command()
        .args(["config", "set", "llm", "--request-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    for bytes in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains("synthetic-persisted-secret"));
    }
    events(output, exit)
}

#[test]
fn config_roundtrip_restart_redaction_and_clear_need_no_environment() {
    let sandbox = Sandbox::new();
    let initial = sandbox.run(&["config", "show"], 0);
    assert!(!sandbox.data_dir().exists());
    let revision = payload(&initial, "ack")["detail"]["revision"].clone();
    let saved = update(
        &sandbox,
        &json!({
            "expected_revision": revision,
            "api_format":"anthropic", "base_url":"https://models.example.test", "model":"synthetic-model",
            "key":"synthetic-persisted-secret"
        }),
        0,
    );
    assert_eq!(
        payload(&saved, "ack")["detail"]["llm"]["credential"]["source"],
        "config"
    );
    assert!(
        fs::read_to_string(sandbox.data_dir().join("config.toml"))
            .unwrap()
            .contains("synthetic-persisted-secret")
    );
    let restarted = sandbox.run(&["doctor"], 0);
    assert_eq!(
        payload(&restarted, "ack")["detail"]["providers"]["llm"]["key_present"],
        true
    );
    let shown = sandbox.run(&["config", "show"], 0);
    assert!(
        !serde_json::to_string(&shown)
            .unwrap()
            .contains("synthetic-persisted-secret")
    );
    assert_eq!(
        payload(&shown, "ack")["detail"]["llm"]["extra_body"],
        json!({})
    );
    update(
        &sandbox,
        &json!({"expected_revision":revision,"model":"stale"}),
        4,
    );
    update(&sandbox, &json!({"clear_key":true}), 0);
    sandbox.run(&["doctor"], 4);
    assert!(!sandbox.data_dir().join("chat-tldr.db").exists());
}

#[test]
fn cli_flags_and_environment_migration_keep_keys_out_of_output() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args([
            "config",
            "set",
            "llm",
            "--api-format",
            "openai",
            "--model",
            "configured-model",
            "--base-url",
            "https://example.test/v1",
            "--key-from-env",
            "SYNTHETIC_MIGRATION_KEY",
        ])
        .env("SYNTHETIC_MIGRATION_KEY", "synthetic-persisted-secret")
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-persisted-secret"));
    events(output, 0);
    sandbox.run(&["doctor"], 0);
    let stored =
        chat_tldr_engine::Config::load(&sandbox.data_dir().join("config.toml"), true).unwrap();
    assert_eq!(stored.llm.model, "configured-model");
    assert_eq!(
        stored.llm.api_key.unwrap().expose(),
        "synthetic-persisted-secret"
    );
    update(
        &sandbox,
        &json!({"key":"synthetic-persisted-secret", "typo":"unexpected"}),
        4,
    );
}
