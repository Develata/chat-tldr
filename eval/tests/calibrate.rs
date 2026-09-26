use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn fixtures(root: &Path) -> Vec<Value> {
    fs::create_dir(root.join("gold")).unwrap();
    let labels:Vec<_>=(1..=4).map(|i| json!({"message_id":format!("m_{i:016x}"),"thread":"hand-specified","todo":i>2,"announcement":false})).collect();
    fs::write(
        root.join("gold/messages.jsonl"),
        labels
            .iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let mut payloads = vec![(
        "ack",
        json!({"command":"jev-log","target":"r_original","changed":false,"detail":{}}),
    )];
    for (i, p) in [0.0, 0.25, 0.75, 1.0].into_iter().enumerate() {
        payloads.push(("jev_answer",json!({"model":"hand-specified-probabilities","request_key":"request","question_id":format!("n{}_todo",i+1),"qtype":"noul",
            "subject":{"kind":"message","id":format!("m_{:016x}",i+1)},"answer":{"type":"noul","p_yes":p},"confidence":null})));
    }
    payloads.push(payloads[1].clone());
    let mut missing = payloads[2].1.clone();
    missing["question_id"] = json!("missing_todo");
    missing["subject"]["id"] = json!("unavailable");
    missing["subject"]["kind"] = json!("unknown");
    payloads.push(("jev_answer", missing));
    payloads.push((
        "done",
        json!({"status":"complete","exit_code":0,"elapsed_ms":1}),
    ));
    payloads.into_iter().enumerate().map(|(seq,(event,payload))| json!({"schema_version":"1.0","run_id":"r_log","seq":seq,"event":event,"payload":payload})).collect()
}
fn write(root: &Path, rows: &[Value]) {
    fs::write(
        root.join("jev.jsonl"),
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
}
fn run(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
        .args(["calibrate", "--gold"])
        .arg(root.join("gold"))
        .arg("--jev")
        .arg(root.join("jev.jsonl"))
        .arg("--out")
        .arg(root.join("result"))
        .output()
        .unwrap()
}

#[test]
fn publishes_real_plot_with_hand_computed_scores_and_exclusion_counts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, &fixtures(root));
    let result = run(root);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&fs::read(root.join("result/calibration.json")).unwrap()).unwrap();
    assert_eq!(report["included"], 4);
    assert_eq!(report["duplicate_answers"], 1);
    assert_eq!(report["excluded"]["unlabelled_subject"], 1);
    assert_eq!(report["groups"][0]["ece"], 0.125);
    assert_eq!(report["groups"][0]["binary_brier"], 0.03125);
    let plot = fs::read_to_string(root.join("result/reliability.svg")).unwrap();
    assert!(plot.contains("<svg") && plot.contains("Mean predicted probability"));
    assert!(!run(root).status.success());
}

#[test]
fn incomplete_conflicting_and_malformed_streams_publish_nothing() {
    for variant in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut rows = fixtures(root);
        match variant {
            0 => {
                rows.pop();
            }
            1 => rows[5]["payload"]["answer"]["p_yes"] = json!(0.9),
            _ => rows[2]["payload"]["answer"]["p_yes"] = json!("PRIVATE_CHAT"),
        };
        write(root, &rows);
        let result = run(root);
        assert!(!result.status.success());
        assert!(!root.join("result").exists());
        assert!(!String::from_utf8_lossy(&result.stdout).contains("PRIVATE_CHAT"));
    }
}
