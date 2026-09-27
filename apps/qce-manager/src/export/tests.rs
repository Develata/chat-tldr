use super::*;
use crate::{
    args::{Cli, Command},
    credentials::Environment,
    docker::SystemDocker,
    test_support::{Reply, Server},
};
use clap::Parser;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[test]
fn cancellation_during_polling_cleans_the_job_without_a_success_path() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let server = Server::new(move |request| match request.path.as_str() {
        "/api/messages/export" => Reply::json(json!({"data":{"taskId":"synthetic"}})),
        "/api/tasks/synthetic" => {
            flag.store(true, Ordering::Relaxed);
            Reply::json(json!({"data":{"status":"running","progress":0}}))
        }
        _ => panic!("cancelled task must not download"),
    });
    let root = tempfile::tempdir().unwrap();
    let cli = Cli::parse_from([
        "test",
        "export",
        "--type",
        "group",
        "--peer",
        "synthetic",
        "--since",
        "2026-09-01T00:00:00Z",
        "--until",
        "2026-09-02T00:00:00Z",
    ]);
    let Command::Export(args) = &cli.command else {
        unreachable!()
    };
    let credentials = Credentials {
        cli: &cli,
        environment: Environment {
            qce_token: Some("synthetic".into()),
            ..Environment::default()
        },
        docker: &SystemDocker,
    };
    let files = Files::new(Some(root.path())).unwrap();
    let http = Http::new(&server.url, 1, false).unwrap();
    let mut bytes = Vec::new();
    let error = run(
        args,
        &credentials,
        &http,
        &files,
        &mut Output::new(&mut bytes),
        Budget::new(&cancelled),
    )
    .unwrap_err();
    assert_eq!(error.code, "E_CANCELLED");
    assert_eq!(error.exit, 130);
    assert!(!String::from_utf8(bytes).unwrap().contains("\"path\""));
    assert_eq!(
        std::fs::read_dir(root.path().join("tmp/qce-manager"))
            .unwrap()
            .count(),
        0
    );
}
