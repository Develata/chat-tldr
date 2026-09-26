use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const A: &str = "m_0000000000000001";
const B: &str = "m_0000000000000002";
const C: &str = "m_0000000000000003";
const D: &str = "m_0000000000000004";
const PRIVATE: &str = "synthetic-private-body-must-not-appear-in-scores";

fn frame(run: &str, seq: usize, event: &str, payload: Value) -> Value {
    json!({"schema_version":"1.0","run_id":run,"seq":seq,"event":event,"payload":payload})
}
fn done(run: &str, seq: usize) -> Value {
    frame(
        run,
        seq,
        "done",
        json!({"status":"complete","exit_code":0,"elapsed_ms":9}),
    )
}
fn write_jsonl(path: &Path, rows: &[Value]) {
    fs::write(
        path,
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
}
fn insight(
    id: &str,
    kind: &str,
    priority: &str,
    anchor: &str,
    rejected: bool,
    deadline: bool,
) -> Value {
    json!({"insight":{
        "id":id,"chat_id":"qq:group:synthetic-score","kind":kind,
        "title":PRIVATE,"summary":PRIVATE,"priority":priority,"rank_score":0.8,
        "confidence":0.9,"assignee":"unknown",
        "deadline":if deadline { json!({"raw":"周五前","relation":"before","bound_date":"2026-10-02",
            "bound_time":null,"granularity":"day","anchor":"2026-09-26T11:00:00-03:00",
            "normalized_by":"rule","confidence":1.0}) } else { Value::Null },
        "evidence":[{"message_id":anchor,"quote":PRIVATE,"render_profile":"r1"}],
        "topic_id":"t_synthetic","verification_status":if rejected { "rejected" } else { "verified" },
        "lifecycle":"open","created_in_run":"r_analysis","updated_at":"2026-09-26T11:00:00-03:00"
    },"evidence_view":[]})
}

struct Case {
    dir: TempDir,
    inbox: Vec<Value>,
    messages: Vec<Value>,
    analyze: Vec<Value>,
}
impl Case {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("gold")).unwrap();
        fs::create_dir(dir.path().join("run")).unwrap();
        let gold_messages: Vec<_> = [A, B, C, D]
            .iter()
            .map(|id| {
                json!({
                    "message_id":id,"thread":"th1","todo":*id == A,"announcement":*id == B
                })
            })
            .collect();
        write_jsonl(&dir.path().join("gold/messages.jsonl"), &gold_messages);
        write_jsonl(
            &dir.path().join("gold/items.jsonl"),
            &[
                json!({"item_id":"g1","kind":"todo","assignee":"other","anchors":[A],"deadline":{
                "raw":"周五前","relation":"before","bound_date":"2026-10-02"},"importance":"P0"}),
                json!({"item_id":"g2","kind":"announcement","assignee":"all","anchors":[B],"deadline":null,"importance":"P1"}),
                json!({"item_id":"g3","kind":"decision","assignee":"unknown","anchors":[D],"deadline":null,"importance":"P0"}),
            ],
        );
        let mut messages: Vec<_> = [A,B,C,D].iter().enumerate().map(|(i,id)| frame("r_messages",i,"message",json!({
            "message_id":id,"sender":"qq:synthetic","sender_display":PRIVATE,"sent_at":"2026-09-26T11:00:00-03:00",
            "display_text":PRIVATE,"recalled":false,"system":false,"reply_to":null,"mentions_me":false,
            "topic_id":"t_synthetic","burst_id":"b_synthetic","cursor":format!("1790400000000:{}",i+1)
        }))).collect();
        messages.push(done("r_messages", 4));
        let inbox = vec![
            frame(
                "r_inbox",
                0,
                "inbox",
                json!({"chat_id":"qq:group:synthetic-score","view_cursor":null,"last_reviewed":null,
                "counts":{"P0":2,"P1":2,"P2":0,"P3":0},"rejected_insights":1,"generated_at":"2026-09-26T11:01:00-03:00"}),
            ),
            frame(
                "r_inbox",
                1,
                "insight",
                insight("i_1", "todo", "P0", A, false, true),
            ),
            frame(
                "r_inbox",
                2,
                "insight",
                insight("i_2", "todo", "P0", C, false, false),
            ),
            frame(
                "r_inbox",
                3,
                "insight",
                insight("i_3", "announcement", "P1", B, true, false),
            ),
            frame(
                "r_inbox",
                4,
                "insight",
                insight("i_4", "decision", "P1", D, false, false),
            ),
            done("r_inbox", 5),
        ];
        let analyze = vec![
            frame(
                "r_analysis",
                0,
                "stats",
                json!({"scope":"run","run_id":"r_analysis","chat_id":"qq:group:synthetic-score",
                "messages_analyzed":4,"topics_created":1,"topics_updated":0,
                "insights":{"created":4,"updated":0,"verified":3,"unverified":0,"rejected":1},
                "usage":[{"stage":"extract","provider":"synthetic","model":"mock","calls":2,"cache_hits":1,"input_tokens":120,"output_tokens":30,"cost_usd":0.01}],
                "cost_usd":0.01,"elapsed_ms":9}),
            ),
            done("r_analysis", 1),
        ];
        Self {
            dir,
            inbox,
            messages,
            analyze,
        }
    }
    fn run(&self) -> Output {
        let root = self.dir.path();
        write_jsonl(&root.join("run/messages.jsonl"), &self.messages);
        write_jsonl(&root.join("run/inbox.jsonl"), &self.inbox);
        write_jsonl(&root.join("run/analyze.jsonl"), &self.analyze);
        Command::new(env!("CARGO_BIN_EXE_chat-tldr-eval"))
            .arg("score")
            .arg("--gold")
            .arg(root.join("gold"))
            .arg("--run")
            .arg(root.join("run"))
            .arg("--out")
            .arg(root.join("scores.csv"))
            .output()
            .unwrap()
    }
    fn scores(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        csv::Reader::from_path(self.dir.path().join("scores.csv"))
            .unwrap()
            .deserialize()
            .map(|row| {
                let row: BTreeMap<String, String> = row.unwrap();
                (row["metric"].clone(), row)
            })
            .collect()
    }
}
fn pass(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["valid"], true);
    summary
}
fn fail(case: &Case) {
    let output = case.run();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["valid"], false);
    assert!(!case.dir.path().join("scores.csv").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(PRIVATE));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(PRIVATE));
}

#[test]
fn scores_nonperfect_snapshot_with_gold_ideal_and_single_run_cost() {
    let case = Case::new();
    let gold_before = fs::read(case.dir.path().join("gold/items.jsonl")).unwrap();
    let summary = pass(&case.run());
    assert_eq!(summary["snapshot_items"], 4);
    assert_eq!(summary["scored_items"], 3);
    assert_eq!(summary["exit_code_source"], "stream_only");
    let scores = case.scores();
    let number = |key: &str| scores[key]["value"].parse::<f64>().unwrap();
    assert_eq!(number("todo_precision"), 0.5);
    assert_eq!(number("todo_recall"), 1.0);
    assert!((number("todo_f1") - 2.0 / 3.0).abs() < 1e-12);
    assert_eq!(number("announcement_recall"), 0.0);
    assert_eq!(scores["announcement_precision"]["status"], "undefined");
    assert_eq!(number("deadline_f1"), 1.0);
    assert_eq!(number("p0_recall_at_5"), 1.0);
    let ndcg = 10.5 / (7.0 + 7.0 / 3_f64.log2() + 1.5);
    assert!((number("ndcg_at_5") - ndcg).abs() < 1e-12);
    assert_eq!(number("snapshot_rejected_rate"), 0.25);
    assert_eq!(number("run_input_tokens"), 120.0);
    assert_eq!(number("run_estimated_cost_usd"), 0.01);
    assert_eq!(
        scores["raw_unsupported_rate"]["status"],
        "unavailable_raw_proposals"
    );
    assert_eq!(number("thread_one_to_one"), 1.0);
    assert_eq!(number("thread_exact_f1"), 1.0);
    assert_eq!(number("ari"), 1.0);
    assert_eq!(number("nmi"), 1.0);
    let csv = fs::read_to_string(case.dir.path().join("scores.csv")).unwrap();
    assert!(!csv.contains(PRIVATE));
    assert!(!csv.contains(A));
    assert_eq!(
        gold_before,
        fs::read(case.dir.path().join("gold/items.jsonl")).unwrap()
    );
    assert!(!case.dir.path().join("run/profile").exists());
}

#[test]
fn b0_retains_rejections_and_scores_model_order_instead_of_inbox_order() {
    let mut case = Case::new();
    let mut rows = vec![frame(
        "r_analysis",
        0,
        "ack",
        json!({"command":"analyze.strategy","target":"qq:group:synthetic-score","changed":false,
        "detail":{"strategy":"b0","ranking":"model_order"}}),
    )];
    // Reverse the model order so an accidental shared inbox sort is observable.
    for row in case
        .inbox
        .iter()
        .rev()
        .filter(|row| row["event"] == "insight")
    {
        rows.push(frame(
            "r_analysis",
            rows.len(),
            "insight",
            row["payload"].clone(),
        ));
    }
    rows.push(frame(
        "r_analysis",
        rows.len(),
        "stats",
        case.analyze[0]["payload"].clone(),
    ));
    rows.push(done("r_analysis", rows.len()));
    case.analyze = rows;
    let report = pass(&case.run());
    assert_eq!(report["system"], "b0");
    assert_eq!(report["scored_items"], 4);
    let scores = case.scores();
    assert_eq!(
        scores["snapshot_rejected_rate_after_filter"]["value"],
        "0.25"
    );
    assert_eq!(scores["todo_precision"]["value"], "0.5");
    assert_eq!(scores["announcement_recall"]["value"], "1");
    assert_eq!(scores["run_calls"]["system"], "b0");
}

#[test]
fn refuses_existing_output_without_changing_bytes() {
    let case = Case::new();
    fs::write(case.dir.path().join("scores.csv"), "keep existing report").unwrap();
    assert_eq!(case.run().status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(case.dir.path().join("scores.csv")).unwrap(),
        "keep existing report"
    );
}

#[test]
fn refuses_partial_truncated_mixed_chat_or_incomplete_snapshots() {
    for variant in 0..9 {
        let mut case = Case::new();
        match variant {
            0 => {
                case.analyze[1]["payload"]["status"] = json!("partial");
                case.analyze[1]["payload"]["exit_code"] = json!(6);
            }
            1 => {
                case.messages.pop();
            }
            2 => {
                case.inbox[2]["payload"]["insight"]["chat_id"] = json!("qq:group:other");
            }
            3 => {
                case.inbox[0]["payload"]["rejected_insights"] = json!(2);
            }
            4 => {
                case.messages[1]["payload"]["message_id"] = json!(A);
            }
            5 => {
                case.messages[1]["payload"]["message_id"] = json!("m_0000000000000009");
            }
            6 => {
                case.analyze[0]["payload"]["run_id"] = json!("r_wrong");
            }
            7 => {
                case.inbox[2]["payload"]["insight"]["id"] = json!("i_1");
            }
            8 => {
                case.inbox[1]["payload"]["insight"]["kind"] = json!(PRIVATE);
            }
            _ => unreachable!(),
        }
        fail(&case);
    }
}

#[test]
fn rejects_invalid_gold_and_unknown_anchors_without_leaking_content() {
    for variant in 0..3 {
        let case = Case::new();
        let path = case.dir.path().join("gold/items.jsonl");
        let text = fs::read_to_string(&path).unwrap();
        let mut items: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        match variant {
            0 => items[0]["anchors"] = json!(["m_0000000000000009"]),
            1 => items[0]["kind"] = json!(PRIVATE),
            2 => items[0]["deadline"]["bound_date"] = json!("2026-02-30"),
            _ => unreachable!(),
        }
        write_jsonl(&path, &items);
        fail(&case);
    }
}

#[test]
fn compatible_unknown_minor_events_keep_protocol_validation() {
    let mut case = Case::new();
    case.inbox.insert(
        2,
        frame("r_inbox", 2, "future_note", json!({"text":PRIVATE})),
    );
    for (seq, row) in case.inbox.iter_mut().enumerate() {
        row["seq"] = json!(seq);
        row["schema_version"] = json!("1.1");
    }
    pass(&case.run());
}

#[test]
fn missing_stats_are_unavailable_not_zero() {
    let mut case = Case::new();
    case.analyze = vec![done("r_analysis", 0)];
    let summary = pass(&case.run());
    assert_eq!(summary["run_stats_available"], false);
    assert_eq!(
        case.scores()["run_calls"]["status"],
        "unavailable_run_stats"
    );
    assert_eq!(case.scores()["run_calls"]["value"], "");
}

#[test]
fn empty_evaluation_is_explicitly_undefined() {
    let mut case = Case::new();
    write_jsonl(&case.dir.path().join("gold/messages.jsonl"), &[]);
    write_jsonl(&case.dir.path().join("gold/items.jsonl"), &[]);
    case.messages = vec![done("r_messages", 0)];
    case.inbox.truncate(1);
    case.inbox[0]["payload"]["counts"] = json!({"P0":0,"P1":0,"P2":0,"P3":0});
    case.inbox[0]["payload"]["rejected_insights"] = json!(0);
    case.inbox.push(done("r_inbox", 1));
    case.analyze = vec![done("r_analysis", 0)];
    pass(&case.run());
    let scores = case.scores();
    for metric in [
        "todo_f1",
        "ndcg_at_5",
        "p0_recall_at_10",
        "snapshot_rejected_rate",
    ] {
        assert_eq!(scores[metric]["status"], "undefined");
        assert_eq!(scores[metric]["value"], "");
    }
}

#[test]
fn overflowed_usage_and_negative_costs_do_not_publish_scores() {
    for negative_cost in [false, true] {
        let mut case = Case::new();
        if negative_cost {
            case.analyze[0]["payload"]["cost_usd"] = json!(-0.01);
        } else {
            let mut usage = case.analyze[0]["payload"]["usage"][0].clone();
            usage["calls"] = json!(u64::MAX);
            case.analyze[0]["payload"]["usage"]
                .as_array_mut()
                .unwrap()
                .push(usage);
        }
        fail(&case);
    }
}
