use super::*;
use chat_tldr_core::{Overview, OverviewPart};

fn assemble(events: &[Value]) -> Overview {
    let ack = payload(events, "ack");
    let mut report: Overview = serde_json::from_value(ack["detail"]["overview"].clone()).unwrap();
    for event in events
        .iter()
        .filter(|e| e["event"] == "ack" && e["payload"]["command"] == "overview.rows")
    {
        serde_json::from_value::<OverviewPart>(event["payload"]["detail"].clone())
            .unwrap()
            .append(&mut report);
    }
    assert_eq!(
        serde_json::to_value(report.counts()).unwrap(),
        ack["detail"]["counts"]
    );
    report
}

fn source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/qce/scenario-analysis.json")
}
const CHAT: &str = "qq:group:synthetic-analysis-group";

#[test]
fn overview_is_offline_read_only_and_distinguishes_unanalyzed_mentions_from_heat() {
    let sandbox = Sandbox::new();
    sandbox.run(&["import", source().to_str().unwrap()], 0);
    let database = sandbox.data_dir().join("chat-tldr.db");
    let before = fs::read(&database).unwrap();
    let events = sandbox.run(
        &[
            "overview",
            "--chat",
            CHAT,
            "--since",
            "2026-09-24T09:00:00+08:00",
            "--until",
            "2026-09-24T09:06:00+08:00",
            "--html",
            "overview.html",
        ],
        0,
    );
    let ack = payload(&events, "ack");
    assert_eq!(ack["command"], "overview");
    assert_eq!(ack["changed"], false);
    let report = assemble(&events);
    assert_eq!(report.window_messages, 6);
    assert_eq!(report.window_pending, 6);
    assert_eq!(report.mentions.len(), 3);
    assert!(report.mentions.iter().all(|r| !r.analyzed));
    assert!(report.hot_topics.is_empty());
    assert!(report.insights.is_empty());
    assert_eq!(fs::read(&database).unwrap(), before);
    let html = fs::read_to_string(sandbox.0.join("overview.html")).unwrap();
    for label in [
        "最近热门话题",
        "优先话题",
        "与我有关",
        "截止事项",
        "未读回顾",
        "资料入口",
        "尚未完成分析",
    ] {
        assert!(html.contains(label));
    }
    assert!(!html.contains("<script"));
}

#[test]
fn overview_missing_database_invalid_windows_and_protected_html_fail_without_writes() {
    let sandbox = Sandbox::new();
    sandbox.run(&["overview", "--chat", CHAT], 3);
    assert!(!sandbox.data_dir().exists());
    sandbox.run(
        &[
            "overview",
            "--chat",
            CHAT,
            "--since",
            "2026-09-24T10:00:00Z",
            "--until",
            "2026-09-24T10:00:00Z",
        ],
        2,
    );
    sandbox.run(&["overview", "--chat", CHAT, "--until", "not-a-date"], 2);
    sandbox.run(&["import", source().to_str().unwrap()], 0);
    let db = sandbox.data_dir().join("chat-tldr.db");
    let before = fs::read(&db).unwrap();
    sandbox.run(
        &["overview", "--chat", CHAT, "--html", db.to_str().unwrap()],
        8,
    );
    assert_eq!(fs::read(db).unwrap(), before);
    let events = sandbox.run(
        &[
            "overview",
            "--chat",
            CHAT,
            "--until",
            "2026-09-24T09:00:00+08:00",
        ],
        0,
    );
    let report = assemble(&events);
    assert_eq!(report.until - report.since, chrono::Duration::hours(24));
    assert_eq!(report.window_messages, 0);
}

#[test]
fn large_overview_uses_individual_rows_below_the_gui_line_limit() {
    let sandbox = Sandbox::new();
    let mut input: Value = serde_json::from_slice(&fs::read(source()).unwrap()).unwrap();
    let mut prototype = input["messages"][0].clone();
    prototype["content"]["elements"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"type":"text","data":{"text":"x".repeat(6000)}}));
    input["messages"] = (0..200)
        .map(|index| {
            let mut row = prototype.clone();
            row["id"] = Value::String(format!("9300000000{index:06}"));
            row["seq"] = Value::String(index.to_string());
            row
        })
        .collect();
    input["statistics"]["totalMessages"] = serde_json::json!(200);
    let path = sandbox.0.join("large.json");
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    sandbox.run(&["import", path.to_str().unwrap()], 0);
    let output = sandbox
        .command()
        .args([
            "overview",
            "--chat",
            CHAT,
            "--since",
            "2026-09-24T09:00:00+08:00",
            "--until",
            "2026-09-24T10:00:00+08:00",
        ])
        .output()
        .unwrap();
    assert!(output.stdout.len() > 1024 * 1024);
    assert!(
        output
            .stdout
            .split(|b| *b == b'\n')
            .all(|line| line.len() < 1024 * 1024)
    );
    let stream = events(output, 0);
    assert_eq!(assemble(&stream).mentions.len(), 200);
}

#[test]
fn overview_html_escapes_untrusted_original_messages() {
    let sandbox = Sandbox::new();
    let mut input: Value = serde_json::from_slice(&fs::read(source()).unwrap()).unwrap();
    input["messages"][0]["content"]["elements"]
        .as_array_mut()
        .unwrap()
        .push(
            serde_json::json!({"type":"text","data":{"text":"<script>alert('x')</script> & raw"}}),
        );
    let path = sandbox.0.join("input.json");
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    sandbox.run(&["import", path.to_str().unwrap()], 0);
    sandbox.run(
        &[
            "overview",
            "--chat",
            CHAT,
            "--since",
            "2026-09-24T09:00:00+08:00",
            "--until",
            "2026-09-24T09:06:00+08:00",
            "--html",
            "view.html",
        ],
        0,
    );
    let html = fs::read_to_string(sandbox.0.join("view.html")).unwrap();
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
}
