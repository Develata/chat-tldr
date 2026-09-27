use super::*;
use crate::{
    args::Cli,
    credentials::Environment,
    docker::SystemDocker,
    test_support::{Reply, Request, Server},
};
use clap::Parser;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct Display {
    terminal: bool,
    codes: Vec<String>,
}
impl QrDisplay for Display {
    fn interactive(&self) -> bool {
        self.terminal
    }
    fn show(&mut self, content: &str) -> Result<()> {
        assert!(self.terminal);
        let rendered = render(content)?;
        assert!(rendered.contains('█') || rendered.contains('▀') || rendered.contains('▄'));
        self.codes.push(content.into());
        Ok(())
    }
}

fn authenticate(request: &Request) -> Option<Reply> {
    if request.path == "/api/auth/login" {
        assert_eq!(request.method, "POST");
        let expected =
            hex(ring::digest::digest(&ring::digest::SHA256, b"webui-test.napcat").as_ref());
        assert_eq!(request.body["hash"], expected);
        assert!(!request.headers.contains_key("authorization"));
        return Some(Reply::json(
            json!({"code":0,"data":{"Credential":"credential-test"}}),
        ));
    }
    let expected = if request.path.starts_with("/api/QQLogin") {
        "Bearer credential-test"
    } else {
        "Bearer qce-test"
    };
    assert_eq!(request.headers["authorization"], expected);
    None
}

fn invoke(server: &Server, display: &mut Display, max_wait: u64) -> (Result<()>, String) {
    invoke_mode(server, display, max_wait, false)
}

fn invoke_mode(
    server: &Server,
    display: &mut Display,
    max_wait: u64,
    qr_events: bool,
) -> (Result<()>, String) {
    let cli = Cli::parse_from(["test", "login"]);
    let credentials = Credentials {
        cli: &cli,
        environment: Environment {
            qce_token: Some("qce-test".into()),
            napcat_token: Some("webui-test".into()),
            ..Environment::default()
        },
        docker: &SystemDocker,
    };
    let qce = Http::new(&server.url, 1, false).unwrap();
    let napcat = Http::new(&server.url, 1, true).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut bytes = Vec::new();
    let result = run(
        Options {
            max_wait_secs: max_wait,
            qr_events,
        },
        &credentials,
        &qce,
        &napcat,
        display,
        &mut Output::new(&mut bytes),
        Budget::new(&cancelled),
    );
    let text = String::from_utf8(bytes).unwrap();
    for secret in ["qce-test", "webui-test", "credential-test"] {
        assert!(!text.contains(secret));
    }
    (result, text)
}

#[test]
fn explicit_gui_mode_refreshes_qr_without_terminal_output() {
    let mut probes = 0;
    let server = Server::new(move |request| {
        if let Some(reply) = authenticate(&request) {
            return reply;
        }
        match request.path.as_str() {
            "/api/QQLogin/CheckLoginStatus" => {
                probes += 1;
                Reply::json(json!({"data":{"isLogin":probes >= 4}}))
            }
            "/api/QQLogin/GetQQLoginQrcode" => Reply::json(
                json!({"data":{"qrcode":if probes < 3 {"synthetic-a"} else {"synthetic-b"}}}),
            ),
            "/api/system/status" => Reply::json(json!({"data":{"online":true}})),
            _ => panic!("unexpected route"),
        }
    });
    let mut display = Display::default();
    let (result, text) = invoke_mode(&server, &mut display, 4, true);
    assert!(result.is_ok(), "{result:?}");
    assert!(display.codes.is_empty());
    let codes: Vec<_> = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| row["event"] == "qce_login_qr")
        .collect();
    assert_eq!(codes.len(), 2);
    assert_eq!(
        codes[0]["payload"],
        json!({"version":1,"content":"synthetic-a"})
    );
    assert_eq!(codes[1]["payload"]["content"], "synthetic-b");
}

#[test]
fn existing_login_never_requests_a_qr_and_waits_for_qce() {
    let probes = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&probes);
    let server = Server::new(move |request| {
        if let Some(reply) = authenticate(&request) {
            return reply;
        }
        match request.path.as_str() {
            "/api/QQLogin/CheckLoginStatus" => Reply::json(json!({"data":{"isLogin":true}})),
            "/api/system/status" => {
                if count.fetch_add(1, Ordering::Relaxed) == 0 {
                    let mut r = Reply::bytes(Vec::new());
                    r.status = 503;
                    r
                } else {
                    Reply::json(json!({"data":{"online":true}}))
                }
            }
            _ => panic!("unexpected route"),
        }
    });
    let mut display = Display::default();
    // A 503 from a starting server is a bounded readiness condition too.
    let (result, _) = invoke(&server, &mut display, 3);
    assert!(result.is_ok(), "{result:?}");
    assert!(display.codes.is_empty());
    assert_eq!(probes.load(Ordering::Relaxed), 2);
}

#[test]
fn qr_refreshes_only_on_change_and_is_absent_from_jsonl() {
    let mut status = 0;
    let server = Server::new(move |request| {
        if let Some(reply) = authenticate(&request) {
            return reply;
        }
        match request.path.as_str() {
            "/api/QQLogin/CheckLoginStatus" => {
                status += 1;
                Reply::json(json!({"data":{"isLogin":status >= 4}}))
            }
            "/api/QQLogin/GetQQLoginQrcode" => Reply::json(
                json!({"data":{"qrcode":if status < 3 { "synthetic-qr-a" } else { "synthetic-qr-b" }}}),
            ),
            "/api/system/status" => Reply::json(json!({"data":{"online":true}})),
            _ => panic!("unexpected route"),
        }
    });
    let mut display = Display {
        terminal: true,
        ..Display::default()
    };
    let (result, output) = invoke(&server, &mut display, 4);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(display.codes, ["synthetic-qr-a", "synthetic-qr-b"]);
    assert!(!output.contains("synthetic-qr"));
    for line in output.lines() {
        serde_json::from_str::<Value>(line).unwrap();
    }
}

#[test]
fn unauthorized_body_reauthenticates_exactly_once() {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    let mut status = 0;
    let server = Server::new(move |request| {
        if let Some(reply) = authenticate(&request) {
            seen.fetch_add(1, Ordering::Relaxed);
            return reply;
        }
        match request.path.as_str() {
            "/api/QQLogin/CheckLoginStatus" => {
                status += 1;
                if status == 1 {
                    Reply::json(json!({"code":-1,"message":"Unauthorized"}))
                } else {
                    Reply::json(json!({"data":{"isLogin":true}}))
                }
            }
            "/api/system/status" => Reply::json(json!({"data":{"online":true}})),
            _ => panic!("unexpected route"),
        }
    });
    assert!(invoke(&server, &mut Display::default(), 3).0.is_ok());
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[test]
fn nonterminal_and_lost_existing_login_never_request_a_qr() {
    for initially_online in [false, true] {
        let mut count = 0;
        let server = Server::new(move |request| {
            if let Some(reply) = authenticate(&request) {
                return reply;
            }
            match request.path.as_str() {
                "/api/QQLogin/CheckLoginStatus" => {
                    count += 1;
                    Reply::json(json!({"data":{"isLogin":initially_online && count == 1}}))
                }
                "/api/system/status" => Reply::json(json!({"data":{"online":false}})),
                _ => panic!("QR must not be requested"),
            }
        });
        let mut display = Display {
            terminal: initially_online,
            ..Display::default()
        };
        assert_eq!(
            invoke(&server, &mut display, 3).0.unwrap_err().code,
            "E_QCE_LOGIN_REQUIRED"
        );
        assert!(display.codes.is_empty());
    }
}

#[test]
fn total_deadline_covers_qce_startup() {
    let server = Server::new(|request| {
        if let Some(reply) = authenticate(&request) {
            return reply;
        }
        match request.path.as_str() {
            "/api/QQLogin/CheckLoginStatus" => Reply::json(json!({"data":{"online":true}})),
            "/api/system/status" => Reply::json(json!({"data":{"online":false}})),
            _ => panic!("unexpected route"),
        }
    });
    assert_eq!(
        invoke(&server, &mut Display::default(), 1)
            .0
            .unwrap_err()
            .code,
        "E_QCE_LOGIN_TIMEOUT"
    );
}

#[test]
fn startup_reloads_token_created_after_login() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().to_path_buf();
    let mut statuses = 0;
    let server = Server::new(move |request| {
        if let Some(reply) = authenticate(&request) {
            return reply;
        }
        match request.path.as_str() {
            "/api/QQLogin/CheckLoginStatus" => {
                statuses += 1;
                if statuses == 2 {
                    std::fs::write(
                        config.join("security.json"),
                        r#"{"accessToken":"qce-test"}"#,
                    )
                    .unwrap();
                }
                Reply::json(json!({"data":{"isLogin":true}}))
            }
            "/api/system/status" => Reply::json(json!({"data":{"online":true}})),
            _ => panic!("unexpected route"),
        }
    });
    let cli = Cli::parse_from([
        "test",
        "--qce-config-dir",
        root.path().to_str().unwrap(),
        "login",
    ]);
    let credentials = Credentials {
        cli: &cli,
        environment: Environment {
            napcat_token: Some("webui-test".into()),
            ..Environment::default()
        },
        docker: &SystemDocker,
    };
    let qce = Http::new(&server.url, 1, false).unwrap();
    let napcat = Http::new(&server.url, 1, true).unwrap();
    run(
        Options {
            max_wait_secs: 3,
            qr_events: false,
        },
        &credentials,
        &qce,
        &napcat,
        &mut Display::default(),
        &mut Output::new(Vec::new()),
        Budget::new(&AtomicBool::new(false)),
    )
    .unwrap();
}

#[test]
fn authentication_connection_errors_retry_once_and_persistent_rejection_stops() {
    for persistent in [false, true] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let server = Server::new(move |request| {
            if request.path == "/api/auth/login" {
                if count.fetch_add(1, Ordering::Relaxed) == 0 || persistent {
                    let mut reply = Reply::bytes(Vec::new());
                    reply.status = if persistent { 401 } else { 503 };
                    return reply;
                }
                return Reply::json(json!({"data":{"Credential":"credential-test"}}));
            }
            match request.path.as_str() {
                "/api/QQLogin/CheckLoginStatus" => Reply::json(json!({"data":{"isLogin":true}})),
                "/api/system/status" => Reply::json(json!({"data":{"online":true}})),
                _ => panic!("unexpected route"),
            }
        });
        let result = invoke(&server, &mut Display::default(), 3).0;
        assert_eq!(result.is_ok(), !persistent);
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }
}
