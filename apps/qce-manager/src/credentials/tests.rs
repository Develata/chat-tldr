use super::*;
use clap::Parser;
use std::{cell::Cell, sync::atomic::AtomicBool};

struct FakeDocker {
    calls: Cell<usize>,
    result: Option<Vec<u8>>,
}
impl Docker for FakeDocker {
    fn run(&self, arguments: &[&str], _: Budget<'_>) -> Result<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(&arguments[..4], &["exec", "synthetic", "cat", "--"]);
        self.result
            .clone()
            .ok_or_else(|| Failure::new("E_QCE_DOCKER", 4, "synthetic failure"))
    }
}

#[test]
fn precedence_and_fallback_never_need_real_docker_or_environment() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("security.json"),
        r#"{"accessToken":"file-token","secretKey":"discard-me"}"#,
    )
    .unwrap();
    let cli = Cli::parse_from([
        "test",
        "--docker",
        "synthetic",
        "--qce-config-dir",
        root.path().to_str().unwrap(),
        "status",
    ]);
    let docker = FakeDocker {
        calls: Cell::new(0),
        result: Some(br#"{"accessToken":"docker-token","secretKey":"discard-me"}"#.to_vec()),
    };
    let cancelled = AtomicBool::new(false);
    let budget = Budget::new(&cancelled);
    let mut credentials = Credentials {
        cli: &cli,
        environment: Environment {
            qce_token: Some("env-token".into()),
            ..Environment::default()
        },
        docker: &docker,
    };
    assert_eq!(credentials.qce(budget).unwrap().0, "env-token");
    assert_eq!(docker.calls.get(), 0);
    credentials.environment.qce_token = None;
    assert_eq!(credentials.qce(budget).unwrap().0, "docker-token");
    let docker = FakeDocker {
        calls: Cell::new(0),
        result: None,
    };
    credentials.docker = &docker;
    assert_eq!(credentials.qce(budget).unwrap().0, "file-token");
    assert_eq!(docker.calls.get(), 2);
    let docker = FakeDocker {
        calls: Cell::new(0),
        result: Some(br#"{"secretKey":"discard-me"}"#.to_vec()),
    };
    credentials.docker = &docker;
    assert_eq!(credentials.qce(budget).unwrap().0, "file-token");
    std::fs::write(root.path().join("security.json"), "broken-json-discard-me").unwrap();
    let error = credentials.qce(budget).err().unwrap();
    assert_eq!(error.code, "E_QCE_AUTH");
    assert!(!error.message.contains("discard-me"));
    assert!(!error.message.contains(root.path().to_str().unwrap()));
}

#[test]
fn only_nonempty_named_string_fields_are_accepted() {
    for bytes in [
        b"{}".as_slice(),
        br#"{"accessToken":42}"#,
        br#"{"accessToken":" "}"#,
        b"bad",
        br#"{"token":"napcat-only"}"#,
    ] {
        assert!(parse(bytes, true).is_none());
    }
    assert_eq!(
        parse(
            br#"{"token":"webui","accessToken":"qce","secretKey":"ignored"}"#,
            false
        )
        .unwrap()
        .0,
        "webui"
    );
    assert!(parse(&vec![b' '; CONFIG_LIMIT as usize + 1], true).is_none());
}

#[test]
fn explicit_container_path_and_native_fallback_order() {
    let root = tempfile::tempdir().unwrap();
    let env_dir = root.path().join("config");
    let home = root.path().join("home");
    std::fs::create_dir_all(&env_dir).unwrap();
    std::fs::create_dir_all(home.join(".qq-chat-exporter")).unwrap();
    std::fs::write(
        env_dir.join("security.json"),
        r#"{"accessToken":"env-directory"}"#,
    )
    .unwrap();
    std::fs::write(
        home.join(".qq-chat-exporter/security.json"),
        r#"{"accessToken":"home-directory"}"#,
    )
    .unwrap();
    let cli = Cli::parse_from([
        "test",
        "--docker",
        "synthetic",
        "--security-json-path",
        "/custom/security.json",
        "status",
    ]);
    let docker = FakeDocker {
        calls: Cell::new(0),
        result: Some(b"broken".to_vec()),
    };
    let credentials = Credentials {
        cli: &cli,
        environment: Environment {
            qce_config: Some(env_dir.clone()),
            home: Some(home),
            ..Environment::default()
        },
        docker: &docker,
    };
    let cancelled = AtomicBool::new(false);
    assert_eq!(
        credentials.qce(Budget::new(&cancelled)).unwrap().0,
        "env-directory"
    );
    assert_eq!(docker.calls.get(), 1);
    std::fs::remove_file(env_dir.join("security.json")).unwrap();
    assert_eq!(
        credentials.qce(Budget::new(&cancelled)).unwrap().0,
        "home-directory"
    );
}
