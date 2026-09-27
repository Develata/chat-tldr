#[path = "../src/test_support.rs"]
mod support;

use chat_tldr_core::{CliEvent, EventStreamValidator};
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use support::{Reply, Request, Server};

const TOKEN: &str = "synthetic-qce-token-keep-private";

fn command(root: &Path, url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_chat-tldr-qce-manager"));
    command.arg("--data-dir").arg(root).args([
        "--base-url",
        url,
        "--napcat-url",
        url,
        "--timeout-secs",
        "1",
    ]);
    command
        .env("CHAT_TLDR_QCE_TOKEN", TOKEN)
        .env("CHAT_TLDR_NAPCAT_TOKEN", "synthetic-webui-token");
    command
}

fn stream(output: &Output) -> Vec<Value> {
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    for secret in [
        TOKEN,
        "synthetic-webui-token",
        "synthetic-credential",
        "synthetic-secret-key",
    ] {
        assert!(!stdout.contains(secret));
        assert!(!stderr.contains(secret));
    }
    let mut validator = EventStreamValidator::new();
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| {
            let event: CliEvent = serde_json::from_str(line).unwrap();
            validator.accept(&event).unwrap();
            serde_json::from_str(line).unwrap()
        })
        .collect();
    validator.finish(output.status.code().unwrap()).unwrap();
    events
}

fn export(root: &Path, url: &str) -> Output {
    command(root, url)
        .args([
            "export",
            "--type",
            "group",
            "--peer",
            "synthetic-group",
            "--since",
            "2026-09-01T00:00:00Z",
            "--until",
            "2026-09-02T00:00:00Z",
            "--max-wait-secs",
            "2",
        ])
        .output()
        .unwrap()
}

fn payload() -> Vec<u8> {
    serde_json::to_vec(&json!({"metadata":{},"chatInfo":{"type":"group","peerUid":"synthetic-group"},"messages":[{"id":"one","content":{"text":"合成内容"}},{"id":"two"}]})).unwrap()
}

fn route(request: Request, failure: &'static str) -> Reply {
    assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
    if failure == "auth" {
        let mut reply = Reply::json(json!({"message":TOKEN}));
        reply.status = 401;
        return reply;
    }
    match request.path.as_str() {
        "/api/messages/export" => {
            assert_eq!(request.method, "POST");
            assert_eq!(request.body["peer"]["chatType"], 2);
            assert_eq!(request.body["filter"]["startTime"], 1788220800000_i64);
            assert_eq!(request.body["format"], "JSON");
            assert_eq!(request.body["options"]["exportAsZip"], false);
            assert_eq!(
                request.body["options"]["skipDownloadResourceTypes"],
                json!(["image", "video", "audio", "file"])
            );
            Reply::json(json!({"success":true,"data":{"taskId":"synthetic-task"}}))
        }
        "/api/tasks/synthetic-task" => Reply::json(json!({"success":true,"data":{
            "status":match failure {"task"=>"failed","timeout"=>"running",_=>"completed"},
            "progress":100,"downloadUrl":if failure=="cross-origin" { "http://127.0.0.1:1/cross-origin" } else { "/download" }
        }})),
        "/download" => {
            let mut reply = Reply::bytes(match failure {
                "json" => b"{malformed".to_vec(),
                "chunked" => br#"{"chunked":{"format":"jsonl"}}"#.to_vec(),
                "messages" => br#"{"messages":{}}"#.to_vec(),
                "trailing" => [payload(), b"false".to_vec()].concat(),
                _ => payload(),
            });
            if failure == "truncated" {
                reply.length = Some(reply.body.len() + 100);
            }
            if failure == "redirect" {
                reply.status = 302;
                reply.location = Some("http://127.0.0.1:1/redirect-target".into());
            }
            reply
        }
        _ => panic!("unexpected route"),
    }
}

fn empty_temp(root: &Path) {
    let path = root.join("tmp/qce-manager");
    assert!(!path.exists() || fs::read_dir(path).unwrap().count() == 0);
}

#[test]
fn export_atomically_publishes_manifest_hash_and_two_distinct_directories() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("中文 数据目录");
    let server = Server::new(|request| route(request, "ok"));
    let mut paths = Vec::new();
    for _ in 0..2 {
        let result = export(&data, &server.url);
        let events = stream(&result);
        assert!(result.status.success(), "{:?}", events);
        let detail =
            &events.iter().find(|event| event["event"] == "ack").unwrap()["payload"]["detail"];
        let path = Path::new(detail["path"].as_str().unwrap());
        assert!(path.is_absolute());
        assert_eq!(fs::read(path).unwrap(), payload());
        let manifest: Value =
            serde_json::from_slice(&fs::read(detail["manifest"].as_str().unwrap()).unwrap())
                .unwrap();
        assert_eq!(manifest["message_count"], 2);
        assert_eq!(manifest["file_size"], payload().len());
        assert_eq!(manifest["sha256"], detail["sha256"]);
        let digest = ring::digest::digest(&ring::digest::SHA256, &payload());
        let expected: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(manifest["sha256"], expected);
        assert_eq!(manifest["schema_version"], 1);
        for key in [
            "export_id",
            "created_at",
            "tool_version",
            "qce_base_url",
            "chat_type",
            "peer_uid",
            "since",
            "until",
            "task_id",
        ] {
            assert!(manifest.get(key).is_some());
        }
        assert!(!manifest.to_string().contains(TOKEN));
        paths.push(path.to_owned());
        empty_temp(&data);
    }
    assert_ne!(paths[0], paths[1]);
    let result = command(&data, &server.url).arg("clean").output().unwrap();
    stream(&result);
    assert!(result.status.success());
    assert!(paths.iter().all(|path| path.exists()));
}

#[test]
fn failures_leave_no_path_or_temporary_files() {
    for failure in [
        "auth",
        "task",
        "timeout",
        "truncated",
        "json",
        "chunked",
        "messages",
        "trailing",
        "cross-origin",
        "redirect",
    ] {
        let root = tempfile::tempdir().unwrap();
        let server = Server::new(move |request| route(request, failure));
        let result = export(root.path(), &server.url);
        let events = stream(&result);
        assert!(!result.status.success(), "{failure}");
        assert!(
            !events
                .iter()
                .any(|v| v["payload"]["detail"]["path"].is_string()),
            "{failure}"
        );
        empty_temp(root.path());
        let exports = root.path().join("sources/qce/exports");
        assert!(!exports.exists() || fs::read_dir(exports).unwrap().count() == 0);
    }
}

#[test]
fn refused_connection_is_reported_without_creating_a_success_path() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let root = tempfile::tempdir().unwrap();
    let output = export(root.path(), &url);
    let events = stream(&output);
    assert!(events.iter().any(|v| matches!(
        v["payload"]["code"].as_str(),
        Some("E_QCE_UNREACHABLE" | "E_QCE_TIMEOUT")
    )));
    empty_temp(root.path());
}

#[test]
fn every_command_uses_the_shared_protocol_and_never_requires_real_services() {
    let root = tempfile::tempdir().unwrap();
    let qr_calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&qr_calls);
    let server = Server::new(move |request| match request.path.as_str() {
        "/api/system/status" => Reply::json(json!({"data":{"online":true}})),
        "/api/recent-contacts?includeAll=true&limit=2000" => Reply::json(
            json!({"data":{"contacts":[{"chatType":2,"peerUid":"synthetic-peer","name":"合成群"},{"chatType":3,"peerUid":"skip-temp"}]}}),
        ),
        "/api/auth/login" => Reply::json(json!({"data":{"Credential":"synthetic-credential"}})),
        "/api/QQLogin/CheckLoginStatus" => Reply::json(json!({"data":{"isLogin":true}})),
        _ => {
            seen.fetch_add(1, Ordering::Relaxed);
            panic!("unexpected request");
        }
    });
    for args in [
        vec!["version"],
        vec!["status"],
        vec!["login"],
        vec!["chats"],
        vec!["clean", "--dry-run"],
        vec!["clean"],
        vec!["invalid-command"],
    ] {
        let result = command(root.path(), &server.url)
            .args(&args)
            .output()
            .unwrap();
        let events = stream(&result);
        assert_eq!(
            result.status.code(),
            Some(if args[0] == "invalid-command" { 2 } else { 0 })
        );
        if args[0] == "chats" {
            assert_eq!(
                events.iter().filter(|v| v["event"] == "qce_chat").count(),
                1
            );
        }
        if args[0] == "status" {
            assert!(
                events[0]["payload"]["detail"]["login_hint"]
                    .as_str()
                    .unwrap()
                    .contains(&server.url)
            );
        }
    }
    assert_eq!(qr_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn redirected_stderr_never_receives_or_requests_a_login_qr() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::new(|request| match request.path.as_str() {
        "/api/auth/login" => Reply::json(json!({"data":{"Credential":"synthetic-credential"}})),
        "/api/QQLogin/CheckLoginStatus" => {
            Reply::json(json!({"data":{"isLogin":false,"qrcodeurl":"synthetic-qr-do-not-print"}}))
        }
        _ => panic!("noninteractive login must not request a QR"),
    });
    let result = command(root.path(), &server.url)
        .arg("login")
        .output()
        .unwrap();
    let events = stream(&result);
    assert_eq!(result.status.code(), Some(4));
    assert!(
        events
            .iter()
            .any(|v| v["payload"]["code"] == "E_QCE_LOGIN_REQUIRED")
    );
    assert!(!String::from_utf8_lossy(&result.stderr).contains("synthetic-qr"));
}

#[test]
fn remote_credentials_paths_and_invalid_time_are_rejected_before_network_io() {
    let root = tempfile::tempdir().unwrap();
    for url in [
        "http://192.0.2.1:40653",
        "http://localhost.example:40653",
        "http://user:password@localhost",
        "http://localhost/?token=hidden",
        "file:///tmp/export",
        "http://localhost/path",
    ] {
        let result = command(root.path(), url).arg("chats").output().unwrap();
        stream(&result);
        assert_eq!(result.status.code(), Some(2));
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let server = Server::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        Reply::json(json!({}))
    });
    let result = command(root.path(), &server.url)
        .args([
            "export",
            "--type",
            "group",
            "--peer",
            "synthetic",
            "--since",
            "2026-09-01T00:00:00.000001Z",
            "--until",
            "2026-09-02T00:00:00Z",
        ])
        .output()
        .unwrap();
    stream(&result);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
