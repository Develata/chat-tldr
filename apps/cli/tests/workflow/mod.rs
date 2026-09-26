use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::json;

use super::*;

struct LocalModels {
    url: String,
    fail: Arc<AtomicBool>,
    empty_then_auth: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LocalModels {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fail = Arc::new(AtomicBool::new(false));
        let empty_then_auth = Arc::new(AtomicBool::new(false));
        let worker_empty_then_auth = Arc::clone(&empty_then_auth);
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_fail, worker_calls, worker_stop) =
            (Arc::clone(&fail), Arc::clone(&calls), Arc::clone(&stop));
        let worker = thread::spawn(move || {
            let mut classifications = 0;
            while !worker_stop.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("local HTTP accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(4)))
                    .unwrap();
                let body = read_request(&mut stream);
                worker_calls.fetch_add(1, Ordering::Relaxed);
                let empty_then_auth = worker_empty_then_auth.load(Ordering::Relaxed);
                let input: Value = serde_json::from_str(
                    body["messages"].as_array().unwrap().last().unwrap()["content"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                if empty_then_auth && input["questions"].is_object() {
                    classifications += 1;
                }
                let (status, response) = if worker_fail.load(Ordering::Relaxed)
                    || (empty_then_auth && classifications == 2)
                {
                    (401, json!({"error":"synthetic-key must never be shown"}))
                } else {
                    let mut response = respond(body);
                    if empty_then_auth
                        && input["instruction"]
                            .as_str()
                            .is_some_and(|instruction| instruction.starts_with("Group every"))
                    {
                        let content: Value = serde_json::from_str(
                            response["choices"][0]["message"]["content"]
                                .as_str()
                                .unwrap(),
                        )
                        .unwrap();
                        let template = &content["topics"][0];
                        let topics: Vec<_> = template["refs"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|reference| {
                                let mut topic = template.clone();
                                topic["refs"] = json!([reference]);
                                topic["items"] = json!([]);
                                topic["summary"] = json!("");
                                topic
                            })
                            .collect();
                        response["choices"][0]["message"]["content"] =
                            json!(json!({"topics":topics}).to_string());
                    }
                    (200, response)
                };
                let response = response.to_string();
                write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
        });
        Self {
            url,
            fail,
            empty_then_auth,
            calls,
            stop,
            worker: Some(worker),
        }
    }

    fn configure(&self, sandbox: &Sandbox) {
        fs::write(
            sandbox.0.join("local-models.toml"),
            format!(
                "[llm]\nbase_url='{}'\napi_key_env='LOCAL_TEST_MODEL_KEY'\ntimeout_secs=4\n",
                self.url
            ),
        )
        .unwrap();
    }

    fn analyze(&self, sandbox: &Sandbox, chat: &str, extra: &[&str], exit: i32) -> Vec<Value> {
        self.analyze_with_decider(sandbox, chat, "llm", extra, exit)
    }

    fn analyze_with_decider(
        &self,
        sandbox: &Sandbox,
        chat: &str,
        decider: &str,
        extra: &[&str],
        exit: i32,
    ) -> Vec<Value> {
        let mut command = sandbox.command();
        command
            .args([
                "--config",
                "local-models.toml",
                "analyze",
                "--chat",
                chat,
                "--decider",
                decider,
            ])
            .args(extra)
            .env(
                "LOCAL_TEST_MODEL_KEY",
                if self.fail.load(Ordering::Relaxed) {
                    "invalid-synthetic-key"
                } else {
                    "synthetic-key"
                },
            )
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost");
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(name);
        }
        let result = command.output().unwrap();
        assert!(!String::from_utf8_lossy(&result.stdout).contains("synthetic-key"));
        events(result, exit)
    }
}

#[test]
fn missing_jev_key_falls_back_once_and_keeps_working() {
    let sandbox = Sandbox::new();
    let chat = import_fixture(&sandbox);
    let server = LocalModels::start();
    server.configure(&sandbox);
    let analyzed = server.analyze_with_decider(&sandbox, &chat, "jev", &[], 0);
    assert_eq!(
        analyzed
            .iter()
            .filter(|event| event["event"] == "warning"
                && event["payload"]["code"] == "W_DECIDER_FALLBACK")
            .count(),
        1
    );
    assert_eq!(payload(&analyzed, "done")["finish_reason"], "done");
    assert!(server.calls.load(Ordering::Relaxed) >= 2);
}

impl Drop for LocalModels {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let length: usize = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, length)| length.trim().parse().unwrap())
        })
        .unwrap();
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}

fn respond(request: Value) -> Value {
    let input: Value = serde_json::from_str(
        request["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let content = if let Some(questions) = input["questions"].as_object() {
        let answers: serde_json::Map<_, _> = questions
            .iter()
            .map(|(key, question)| {
                let answer = match question["type"].as_str().unwrap() {
                    "noul" => {
                        json!({"type":"noul","p_yes":if key.contains("chitchat"){0.01}else{0.9}})
                    }
                    "score" => {
                        let count = question["criteria"].as_array().unwrap().len();
                        let probabilities: serde_json::Map<_, _> = (0..count)
                            .map(|index| {
                                (
                                    index.to_string(),
                                    json!(if index == count - 1 { 1.0 } else { 0.0 }),
                                )
                            })
                            .collect();
                        json!({"type":"score","probabilities":probabilities})
                    }
                    "choice" => {
                        let options = question["criteria"].as_object().unwrap();
                        let selected = if options.contains_key("new_topic") {
                            "new_topic"
                        } else {
                            options.keys().next().unwrap()
                        };
                        let probabilities: serde_json::Map<_, _> = options
                            .keys()
                            .map(|key| {
                                (key.clone(), json!(if key == selected { 1.0 } else { 0.0 }))
                            })
                            .collect();
                        json!({"type":"choice","probabilities":probabilities})
                    }
                    other => panic!("unexpected question {other}"),
                };
                (key.clone(), answer)
            })
            .collect();
        json!({"answers":answers})
    } else {
        let mut topic = json!({"title":"合成报告","summary":"合成群讨论报告提交。","items":[{
            "op":"new","existing_ref":null,"kind":"todo","title":"<script>提交报告</script>","summary":"提交合成报告。","assignee":"other","deadline_raw":"周五前","deadline_date_guess":null,
            "evidence":[{"ref":"n1","quote":"周五前交合成报告。"}]
        }]});
        if input["instruction"]
            .as_str()
            .unwrap()
            .starts_with("Group every")
        {
            topic["refs"] = Value::Array(
                input["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|row| {
                        row["ref"]
                            .as_str()
                            .filter(|reference| reference.starts_with('n'))
                            .map(Value::from)
                    })
                    .collect(),
            );
            json!({"topics":[topic]})
        } else {
            topic
        }
    };
    json!({"choices":[{"finish_reason":"stop","message":{"content":content.to_string()}}],"usage":{"prompt_tokens":120,"completion_tokens":80}})
}

#[test]
fn process_workflow_analyzes_evidence_exports_html_and_sets_state_idempotently() {
    let sandbox = Sandbox::new();
    let chat = import_fixture(&sandbox);
    let server = LocalModels::start();
    server.configure(&sandbox);
    let analyzed = server.analyze(&sandbox, &chat, &["--html", "analysis.html"], 0);
    assert_eq!(payload(&analyzed, "done")["finish_reason"], "done");
    assert_eq!(payload(&analyzed, "stats")["messages_analyzed"], 2);
    assert!(server.calls.load(Ordering::Relaxed) >= 2);
    let inbox = sandbox.run(&["inbox", "--chat", &chat, "--all"], 0);
    let todo = inbox
        .iter()
        .find(|event| event["event"] == "insight" && event["payload"]["insight"]["kind"] == "todo")
        .unwrap();
    let insight = &todo["payload"]["insight"];
    assert_eq!(insight["priority"], "P0");
    assert_eq!(insight["assignee"], "other");
    assert_eq!(insight["verification_status"], "verified");
    let id = insight["id"].as_str().unwrap();
    let html = fs::read_to_string(sandbox.0.join("analysis.html")).unwrap();
    assert!(html.contains("&lt;script&gt;提交报告&lt;/script&gt;"));
    assert!(!html.contains("<script>"));
    for (args, changed) in [
        (vec!["feedback", id, "--useful"], true),
        (vec!["feedback", id, "--useful"], false),
        (vec!["feedback", id, "--not-important"], true),
        (vec!["resolve", id, "--done"], true),
        (vec!["resolve", id, "--done"], false),
        (vec!["resolve", id, "--reopen"], true),
    ] {
        assert_eq!(payload(&sandbox.run(&args, 0), "ack")["changed"], changed);
    }
    let cursor = payload(&inbox, "inbox")["view_cursor"].as_str().unwrap();
    assert_eq!(
        payload(
            &sandbox.run(&["mark-read", "--chat", &chat, "--up-to", cursor], 0),
            "ack"
        )["changed"],
        true
    );
    assert_eq!(
        payload(
            &sandbox.run(&["mark-read", "--chat", &chat, "--up-to", cursor], 0),
            "ack"
        )["changed"],
        false
    );
    let calls = server.calls.load(Ordering::Relaxed);
    sandbox.run(&["analyze", "--chat", &chat], 0);
    assert_eq!(server.calls.load(Ordering::Relaxed), calls);
}

#[test]
fn authentication_failure_exits_four_and_replaced_key_resumes_messages() {
    let sandbox = Sandbox::new();
    let chat = import_fixture(&sandbox);
    let server = LocalModels::start();
    server.configure(&sandbox);
    server.fail.store(true, Ordering::Relaxed);
    let failed = server.analyze(&sandbox, &chat, &[], 4);
    assert_eq!(payload(&failed, "done")["status"], "failed");
    assert_eq!(payload(&failed, "error")["code"], "E_PROVIDER_AUTH");
    assert_eq!(server.calls.load(Ordering::Relaxed), 1);
    let inbox = sandbox.run(&["inbox", "--chat", &chat], 0);
    assert!(payload(&inbox, "inbox")["view_cursor"].is_null());
    server.fail.store(false, Ordering::Relaxed);
    let resumed = server.analyze(&sandbox, &chat, &[], 0);
    assert_eq!(payload(&resumed, "stats")["messages_analyzed"], 2);
}

#[test]
fn authentication_failure_after_empty_topic_commit_is_partial_and_keeps_checkpoint() {
    let sandbox = Sandbox::new();
    let mut fixture: Value = serde_json::from_slice(&fs::read(fixture()).unwrap()).unwrap();
    fixture["messages"][0]["content"] = json!({"text":"普通合成消息。","elements":[{"type":"text","data":{"text":"普通合成消息。"}}]});
    fs::write(sandbox.0.join("no-mentions.json"), fixture.to_string()).unwrap();
    let imported = sandbox.run(&["import", "no-mentions.json"], 0);
    let chat = payload(&imported, "ack")["detail"]["chat_ids"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let server = LocalModels::start();
    server.configure(&sandbox);
    server.empty_then_auth.store(true, Ordering::Relaxed);
    let failed = server.analyze(&sandbox, &chat, &[], 6);
    assert_eq!(payload(&failed, "done")["status"], "partial");
    assert_eq!(payload(&failed, "error")["code"], "E_PROVIDER_AUTH");
    assert!(!failed.iter().any(|event| event["event"] == "insight"));
    assert!(failed.iter().any(|event| event["event"] == "progress"
        && event["payload"]["stage"] == "store"
        && event["payload"]["current"] == 1));
    let inbox = sandbox.run(&["inbox", "--chat", &chat], 0);
    assert!(payload(&inbox, "inbox")["view_cursor"].is_string());
    let plan = sandbox.run(&["analyze", "--chat", &chat, "--dry-run"], 0);
    assert_eq!(payload(&plan, "ack")["detail"]["plan"]["messages"], 1);
    server.empty_then_auth.store(false, Ordering::Relaxed);
    let resumed = server.analyze(&sandbox, &chat, &[], 0);
    assert_eq!(payload(&resumed, "stats")["messages_analyzed"], 1);
}

#[test]
fn budget_stops_before_network_and_html_failure_keeps_committed_data() {
    let sandbox = Sandbox::new();
    let chat = import_fixture(&sandbox);
    let server = LocalModels::start();
    server.configure(&sandbox);
    let budget = server.analyze(&sandbox, &chat, &["--budget-usd", "0.000000001"], 6);
    assert_eq!(payload(&budget, "done")["finish_reason"], "budget_exceeded");
    assert_eq!(server.calls.load(Ordering::Relaxed), 0);
    let result = server.analyze(&sandbox, &chat, &["--html", "missing/result.html"], 6);
    assert_eq!(payload(&result, "error")["code"], "E_OUTPUT_WRITE");
    let inbox = sandbox.run(&["inbox", "--chat", &chat, "--all"], 0);
    assert!(inbox.iter().any(|event| event["event"] == "insight"));
}
