use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const FIRST: &str = "m_0000000000000001";
const SECOND: &str = "m_0000000000000002";
const RECALLED: &str = "m_0000000000000003";

fn event(seq: u64, name: &str, payload: Value) -> Value {
    json!({"schema_version":"1.0","run_id":"r_synthetic_sheet","seq":seq,"event":name,"payload":payload})
}

fn events() -> Vec<Value> {
    let mut events = Vec::new();
    for (index, id) in [FIRST, SECOND, RECALLED].into_iter().enumerate() {
        events.push(event(index as u64, "message", json!({
            "message_id":id,"sender":"qq:u_synthetic","sender_display":"=合成昵称,\"甲\"",
            "sent_at":"2026-09-26T11:22:33-03:00","display_text":if index == 0 { "+中文,\"引号\"\r\n第二行😀" } else { "text:@合成原文" },
            "recalled":index == 2,"system":false,"reply_to":null,"mentions_me":false,
            "topic_id":null,"burst_id":null,"cursor":format!("1790400000000:{}", index + 1)
        })));
    }
    events.push(event(
        3,
        "done",
        json!({"status":"complete","exit_code":0,"elapsed_ms":1}),
    ));
    events
}

struct Workspace {
    _directory: TempDir,
    stream: PathBuf,
    sheet: PathBuf,
    gold: PathBuf,
}
impl Workspace {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let stream = directory.path().join("messages.jsonl");
        let sheet = directory.path().join("sheet.csv");
        let gold = directory.path().join("gold");
        let workspace = Self {
            _directory: directory,
            stream,
            sheet,
            gold,
        };
        workspace.write_stream(&events());
        workspace
    }
    fn write_stream(&self, events: &[Value]) {
        let text = events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&self.stream, text).unwrap();
    }
    fn export(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
            .arg("export-sheet")
            .arg("--messages")
            .arg(&self.stream)
            .arg("--out")
            .arg(&self.sheet)
            .output()
            .unwrap()
    }
    fn import(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
            .arg("import-sheet")
            .arg(&self.sheet)
            .arg("--out")
            .arg(&self.gold)
            .output()
            .unwrap()
    }
    fn import_checked(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
            .arg("import-sheet")
            .arg(&self.sheet)
            .arg("--messages")
            .arg(&self.stream)
            .arg("--out")
            .arg(&self.gold)
            .output()
            .unwrap()
    }
    fn labelled(&self) -> Vec<BTreeMap<String, String>> {
        success(&self.export());
        let mut rows = read_sheet(&self.sheet);
        for row in &mut rows {
            row.insert("thread".into(), "报告讨论,甲".into());
            row.insert("todo".into(), "false".into());
            row.insert("announcement".into(), "false".into());
            row.insert("items_json".into(), "[]".into());
        }
        rows
    }
}

fn success(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], true);
    value
}

fn failure(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], false);
    value["error"].as_str().unwrap().to_owned()
}

fn read_sheet(path: &Path) -> Vec<BTreeMap<String, String>> {
    csv::Reader::from_path(path)
        .unwrap()
        .deserialize()
        .map(Result::unwrap)
        .collect()
}

fn write_sheet(path: &Path, rows: &[BTreeMap<String, String>]) {
    let headers: Vec<_> = rows[0].keys().collect();
    let mut writer = csv::Writer::from_path(path).unwrap();
    writer.write_record(&headers).unwrap();
    for row in rows {
        writer
            .write_record(headers.iter().map(|name| &row[*name]))
            .unwrap();
    }
    writer.flush().unwrap();
}

fn gold(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn item() -> Value {
    json!({"item_id":"gold-甲","kind":"todo","assignee":"other","anchors":[FIRST, SECOND],
        "deadline":{"raw":"周五前","relation":"before","bound_date":"2026-09-25"},"importance":"P0"})
}

#[test]
fn checked_import_accepts_reordered_rows_and_columns_without_changing_sources() {
    let workspace = Workspace::new();
    let mut rows = workspace.labelled();
    rows.reverse();
    write_sheet(&workspace.sheet, &rows);
    let stream_before = fs::read(&workspace.stream).unwrap();
    let sheet_before = fs::read(&workspace.sheet).unwrap();

    let summary = success(&workspace.import_checked());
    assert_eq!(summary["source_checked"], true);
    assert_eq!(summary["messages"], 2);
    assert_eq!(summary["items"], 0);
    assert_eq!(fs::read(&workspace.stream).unwrap(), stream_before);
    assert_eq!(fs::read(&workspace.sheet).unwrap(), sheet_before);
    assert_eq!(gold(&workspace.gold.join("messages.jsonl")).len(), 2);
}

#[test]
fn changed_source_cells_fail_before_publication_without_echoing_private_values() {
    for (field, changed) in [
        ("sent_at", "text:2026-09-27T11:22:33-03:00"),
        ("sender", "text:qq:PRIVATE_CHANGED_SENDER"),
        ("sender_display", "text:PRIVATE_CHANGED_NAME"),
        ("display_text", "text:PRIVATE_CHANGED_TEXT"),
    ] {
        let workspace = Workspace::new();
        let mut rows = workspace.labelled();
        rows[0].insert(field.into(), changed.into());
        write_sheet(&workspace.sheet, &rows);
        let error = failure(&workspace.import_checked());
        assert!(error.contains(field), "{error}");
        assert!(!error.contains(changed));
        assert!(!error.contains("PRIVATE_CHANGED"));
        assert!(!workspace.gold.exists());
    }
}

#[test]
fn checked_import_requires_the_exact_non_recalled_message_set() {
    for mutation in ["missing", "added", "recalled"] {
        let workspace = Workspace::new();
        let mut rows = workspace.labelled();
        match mutation {
            "missing" => {
                rows.pop();
            }
            "added" => {
                let mut extra = rows[0].clone();
                extra.insert("message_id".into(), "m_0000000000000004".into());
                rows.push(extra);
            }
            "recalled" => {
                rows[0].insert("message_id".into(), RECALLED.into());
            }
            _ => unreachable!(),
        }
        write_sheet(&workspace.sheet, &rows);
        assert!(failure(&workspace.import_checked()).contains("reference"));
        assert!(!workspace.gold.exists());
    }
}

#[test]
fn checked_import_refuses_truncated_failed_or_duplicate_reference_streams() {
    for mutation in ["truncated", "failed", "duplicate"] {
        let workspace = Workspace::new();
        let rows = workspace.labelled();
        write_sheet(&workspace.sheet, &rows);
        let mut source = events();
        match mutation {
            "truncated" => {
                source.pop();
            }
            "failed" => {
                source.last_mut().unwrap()["payload"] =
                    json!({"status":"partial","exit_code":6,"elapsed_ms":1});
            }
            "duplicate" => source[1]["payload"]["message_id"] = json!(FIRST),
            _ => unreachable!(),
        }
        workspace.write_stream(&source);
        failure(&workspace.import_checked());
        assert!(!workspace.gold.exists());
    }
}

#[test]
fn checked_empty_import_and_legacy_import_report_their_validation_boundary() {
    let empty = Workspace::new();
    empty.write_stream(&[event(
        0,
        "done",
        json!({"status":"complete","exit_code":0,"elapsed_ms":1}),
    )]);
    success(&empty.export());
    let summary = success(&empty.import_checked());
    assert_eq!(summary["messages"], 0);
    assert_eq!(summary["source_checked"], true);

    let legacy = Workspace::new();
    let rows = legacy.labelled();
    write_sheet(&legacy.sheet, &rows);
    assert_eq!(success(&legacy.import())["source_checked"], false);
}

#[test]
fn unicode_quotes_multiline_formula_prefixes_and_source_prefix_are_lossless() {
    for text in [
        "=1+1",
        "+中文,\"引号\"\r\n第二行😀",
        "-1+2",
        "@SUM(A1)",
        "\t=1+1",
        "\r=1+1",
        "\n=1+1",
        "text:原有前缀",
    ] {
        let workspace = Workspace::new();
        let mut source = events();
        source[0]["payload"]["display_text"] = json!(text);
        workspace.write_stream(&source);
        let summary = success(&workspace.export());
        assert_eq!(summary["messages"], 2);
        assert_eq!(summary["recalled_skipped"], 1);
        let raw = fs::read(&workspace.sheet).unwrap();
        assert!(raw.starts_with(b"\xef\xbb\xbf"));
        let rows = read_sheet(&workspace.sheet);
        assert_eq!(rows[0]["display_text"].strip_prefix("text:"), Some(text));
        assert_eq!(rows[0]["sender_display"], "text:=合成昵称,\"甲\"");
        assert_eq!(rows[1]["display_text"], "text:text:@合成原文");
        for name in ["thread", "todo", "announcement", "items_json"] {
            assert!(rows[0][name].is_empty());
        }
        assert!(failure(&workspace.import()).contains("thread"));
        assert!(!workspace.gold.exists());
    }
}

#[test]
fn labelled_sheet_supports_multiple_items_and_shared_anchors_without_losing_values() {
    let workspace = Workspace::new();
    let mut rows = workspace.labelled();
    rows[0].insert("todo".into(), "true".into());
    rows[0].insert("announcement".into(), "true".into());
    let raw = "尽快,\"合成\"\r\n补充😀";
    let second_item = json!({"item_id":"g2","kind":"decision","assignee":"all","anchors":[FIRST],"deadline":{"raw":raw},"importance":"P1"});
    rows[0].insert(
        "items_json".into(),
        json!([item(), second_item.clone()]).to_string(),
    );
    write_sheet(&workspace.sheet, &rows);
    let summary = success(&workspace.import());
    assert_eq!(summary["messages"], 2);
    assert_eq!(summary["items"], 2);
    let messages = gold(&workspace.gold.join("messages.jsonl"));
    assert_eq!(
        messages[0],
        json!({"message_id":FIRST,"thread":"报告讨论,甲","todo":true,"announcement":true})
    );
    let items = gold(&workspace.gold.join("items.jsonl"));
    assert_eq!(items, [item(), second_item]);
    assert!(
        !workspace
            .gold
            .with_extension("chat-tldr-eval.lock")
            .exists()
    );
}

#[test]
fn missing_or_illegal_labels_never_produce_gold_or_default_false() {
    for (field, value) in [
        ("thread", ""),
        ("todo", ""),
        ("todo", "yes"),
        ("announcement", ""),
        ("items_json", ""),
        ("items_json", "null"),
        ("sheet_version", "2"),
        ("message_id", "m_bad"),
        ("sent_at", "text:not-a-date"),
        ("display_text", "=lost prefix"),
    ] {
        let workspace = Workspace::new();
        let mut rows = workspace.labelled();
        rows[1].insert(field.into(), value.into());
        write_sheet(&workspace.sheet, &rows);
        failure(&workspace.import());
        assert!(!workspace.gold.exists(), "invalid {field} created output");
    }
}

#[test]
fn duplicates_unknown_anchors_and_incomplete_items_are_rejected() {
    let variants = [
        "duplicate_message",
        "duplicate_item",
        "unknown_anchor",
        "recalled_anchor",
        "duplicate_anchor",
        "wrong_row",
        "empty_anchors",
        "missing_deadline",
        "missing_assignee",
        "unknown_field",
    ];
    for variant in variants {
        let workspace = Workspace::new();
        let mut rows = workspace.labelled();
        let mut annotated = item();
        match variant {
            "duplicate_message" => {
                rows[1] = rows[0].clone();
            }
            "duplicate_item" => {
                rows[1].insert("items_json".into(), json!([annotated.clone()]).to_string());
            }
            "unknown_anchor" => annotated["anchors"] = json!([FIRST, "m_0000000000000099"]),
            "recalled_anchor" => annotated["anchors"] = json!([FIRST, RECALLED]),
            "duplicate_anchor" => annotated["anchors"] = json!([FIRST, FIRST]),
            "wrong_row" => annotated["anchors"] = json!([SECOND]),
            "empty_anchors" => annotated["anchors"] = json!([]),
            "missing_deadline" => {
                annotated.as_object_mut().unwrap().remove("deadline");
            }
            "missing_assignee" => {
                annotated.as_object_mut().unwrap().remove("assignee");
            }
            "unknown_field" => annotated["assigneee"] = json!("me"),
            _ => unreachable!(),
        }
        rows[0].insert("items_json".into(), json!([annotated]).to_string());
        write_sheet(&workspace.sheet, &rows);
        failure(&workspace.import());
        assert!(!workspace.gold.exists(), "{variant} created gold");
    }
}

#[test]
fn enums_and_deadlines_are_checked_instead_of_silently_normalized() {
    for (field, value) in [
        ("kind", json!("task")),
        ("assignee", json!("myself")),
        ("importance", json!("p0")),
        (
            "deadline",
            json!({"raw":"明天","relation":"before","bound_date":"2026-02-30"}),
        ),
        ("deadline", json!({"raw":"明天","relation":"before"})),
        ("deadline", json!({"raw":"明天","bound_date":"2026-09-27"})),
        (
            "deadline",
            json!({"raw":"明天","relation":"until","bound_date":"2026-09-27"}),
        ),
        ("deadline", json!({"raw":""})),
        ("item_id", json!("")),
    ] {
        let workspace = Workspace::new();
        let mut rows = workspace.labelled();
        let mut annotated = item();
        annotated[field] = value;
        rows[0].insert("items_json".into(), json!([annotated]).to_string());
        write_sheet(&workspace.sheet, &rows);
        failure(&workspace.import());
        assert!(!workspace.gold.exists());
    }
}

#[test]
fn malformed_incomplete_failed_or_wrong_stream_cannot_export_a_sheet() {
    for variant in [
        "truncated",
        "seq",
        "duplicate",
        "bad_id",
        "partial",
        "wrong_event",
        "trailing",
    ] {
        let workspace = Workspace::new();
        let mut source = events();
        match variant {
            "truncated" => {
                source.pop();
            }
            "seq" => source[1]["seq"] = json!(42),
            "duplicate" => source[1]["payload"]["message_id"] = json!(FIRST),
            "bad_id" => source[0]["payload"]["message_id"] = json!("invalid"),
            "partial" => {
                source[3]["payload"]["status"] = json!("partial");
                source[3]["payload"]["exit_code"] = json!(6);
            }
            "wrong_event" => {
                source[0] = event(
                    0,
                    "ack",
                    json!({"command":"version","target":null,"changed":false,"detail":{}}),
                )
            }
            "trailing" => source.push(source[0].clone()),
            _ => unreachable!(),
        }
        workspace.write_stream(&source);
        failure(&workspace.export());
        assert!(
            !workspace.sheet.exists(),
            "{variant} created a partial sheet"
        );
    }
    let workspace = Workspace::new();
    let output = Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
        .arg("export-sheet")
        .arg("--messages")
        .arg(&workspace.stream)
        .arg("--out")
        .arg(&workspace.sheet)
        .args(["--exit-code", "6"])
        .output()
        .unwrap();
    failure(&output);
    assert!(!workspace.sheet.exists());
}

#[test]
fn unknown_minor_events_are_ignored_without_bypassing_sequence_validation() {
    let workspace = Workspace::new();
    let mut source = events();
    source.insert(1, event(1, "future_message_metadata", json!({"count":3})));
    for (seq, event) in source.iter_mut().enumerate() {
        event["seq"] = json!(seq);
        event["schema_version"] = json!("1.1");
    }
    workspace.write_stream(&source);
    assert_eq!(success(&workspace.export())["messages"], 2);

    let invalid = Workspace::new();
    source[1]["seq"] = json!(40);
    invalid.write_stream(&source);
    failure(&invalid.export());
    assert!(!invalid.sheet.exists());
}

#[test]
fn existing_outputs_and_incomplete_csv_are_never_overwritten() {
    let workspace = Workspace::new();
    let rows = workspace.labelled();
    let original = fs::read(&workspace.sheet).unwrap();
    failure(&workspace.export());
    assert_eq!(fs::read(&workspace.sheet).unwrap(), original);
    write_sheet(&workspace.sheet, &rows);
    fs::create_dir(&workspace.gold).unwrap();
    fs::write(workspace.gold.join("keep.txt"), "keep").unwrap();
    failure(&workspace.import());
    assert_eq!(
        fs::read_to_string(workspace.gold.join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(!workspace.gold.join("messages.jsonl").exists());

    for bytes in [
        b"".as_slice(),
        b"message_id,todo\nm_0000000000000001,true\n",
        b"\xff\n",
    ] {
        let workspace = Workspace::new();
        fs::write(&workspace.sheet, bytes).unwrap();
        failure(&workspace.import());
        assert!(!workspace.gold.exists());
    }
}

#[test]
fn empty_messages_stream_and_explicit_no_items_are_supported() {
    let workspace = Workspace::new();
    workspace.write_stream(&[event(
        0,
        "done",
        json!({"status":"complete","exit_code":0,"elapsed_ms":0}),
    )]);
    assert_eq!(success(&workspace.export())["messages"], 0);
    assert_eq!(success(&workspace.import())["items"], 0);
    assert!(
        fs::read(workspace.gold.join("messages.jsonl"))
            .unwrap()
            .is_empty()
    );
    assert!(
        fs::read(workspace.gold.join("items.jsonl"))
            .unwrap()
            .is_empty()
    );

    let workspace = Workspace::new();
    let mut rows = workspace.labelled();
    let mut annotated = item();
    annotated["deadline"] = Value::Null;
    rows[0].insert("items_json".into(), json!([annotated]).to_string());
    write_sheet(&workspace.sheet, &rows);
    success(&workspace.import());
    assert!(gold(&workspace.gold.join("items.jsonl"))[0]["deadline"].is_null());
}
