use super::*;
use std::{fs, time::Instant};

fn tag(id: u64) -> RequestTag {
    RequestTag {
        id,
        kind: CommandKind::Version,
        chat: None,
    }
}

fn done(seq: u64) -> String {
    serde_json::to_string(&CliEvent::new(
        "r_gui_test".into(),
        seq,
        EventBody::Done(DonePayload {
            status: RunStatus::Complete,
            exit_code: 0,
            finish_reason: None,
            elapsed_ms: 1,
        }),
    ))
    .unwrap()
}

fn stream(source: &str) -> Stream {
    let (sender, _receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
    read_stdout(
        source.as_bytes(),
        &sender,
        &(Arc::new(|| {}) as Repaint),
        &tag(1),
    )
}

#[test]
fn command_passes_paths_as_argv_without_a_shell() {
    let settings = CliSettings {
        executable: "C:/with spaces/chat-tldr.exe".into(),
        data_dir: Some("data;echo unwanted".into()),
        config: None,
    };
    let request = Request {
        tag: tag(1),
        args: vec!["import".into(), "chat $(not-a-command).json".into()],
    };
    let command = command(&settings, &request);
    assert_eq!(command.get_program(), settings.executable.as_os_str());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![
            "--data-dir",
            "data;echo unwanted",
            "import",
            "chat $(not-a-command).json"
        ]
    );
}

#[test]
fn unknown_events_advance_sequence_but_malformed_known_events_and_truncation_fail() {
    let unknown =
        r#"{"schema_version":"1.1","run_id":"r_gui_test","seq":0,"event":"future","payload":{}}"#;
    let valid = stream(&format!("{unknown}\n{}\n", done(1)));
    assert!(valid.error.is_none());
    assert!(valid.validator.finish(0).is_ok());
    for source in [
        format!("{unknown}\n{}\n", done(2)),
        format!("{}\n{}\n", done(0), done(1)),
        r#"{"schema_version":"1.0","run_id":"r_gui_test","seq":0,"event":"inbox","payload":{}}"#
            .into(),
        "not json".into(),
    ] {
        assert!(stream(&source).error.is_some(), "{source}");
    }
    assert!(stream(unknown).validator.finish(0).is_err());
    assert!(stream(&done(0)).validator.finish(4).is_err());
    assert!(stream(&done(0).replace("1.0", "2.0")).error.is_some());
}

#[test]
fn oversized_lines_are_rejected_without_unbounded_allocation() {
    let bytes = vec![b'x'; MAX_STDOUT_LINE + 1];
    let mut input = io::Cursor::new(bytes);
    assert_eq!(
        limited_line(&mut input).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

fn finish(bridge: &mut Bridge) -> Completion {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while let Some(event) = bridge.try_recv() {
            if let BridgePayload::Finished(completion) = event.payload {
                return completion;
            }
        }
        assert!(Instant::now() < deadline, "background CLI did not finish");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn real_child_transport_drains_stderr_validates_exit_and_cancels_only_its_child() {
    let temp = tempfile::tempdir().unwrap();
    let helper_source = temp.path().join("helper.rs");
    let helper = temp.path().join(if cfg!(windows) {
        "helper.exe"
    } else {
        "helper"
    });
    fs::write(&helper_source, r#"
use std::{env,fs,io::{self,Write},thread,time::Duration};
fn main() {
 let args:Vec<_>=env::args().collect();
 let mode=&args[1];
 if mode=="pause" { eprintln!("ready"); thread::sleep(Duration::from_secs(20)); return; }
 if mode=="bad" { println!("bad json"); io::stdout().flush().unwrap(); thread::sleep(Duration::from_secs(20)); return; }
 if mode=="flood" { for _ in 0..5000 { eprintln!("{}", "诊断".repeat(300)); } }
 print!("{}",fs::read_to_string(&args[2]).unwrap());
 io::stdout().flush().unwrap();
 if mode=="mismatch" { std::process::exit(4); }
}
"#).unwrap();
    let result = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&helper_source)
        .arg("-o")
        .arg(&helper)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let fixture = temp.path().join("stream.jsonl");
    fs::write(&fixture, format!("{}\n", done(0))).unwrap();
    let mut bridge = Bridge::new(
        CliSettings {
            executable: helper,
            data_dir: None,
            config: None,
        },
        || {},
    );
    for (index, mode) in ["flood", "mismatch", "bad", "pause"]
        .into_iter()
        .enumerate()
    {
        let request = Request {
            tag: tag(index as u64),
            args: vec![mode.into(), fixture.as_os_str().to_owned()],
        };
        bridge.start(request.clone()).unwrap();
        assert!(bridge.start(request).is_err());
        if mode == "pause" {
            assert!(bridge.cancel());
        }
        let completion = finish(&mut bridge);
        assert!(!bridge.is_busy());
        assert_eq!(completion.is_success(), mode == "flood");
        if mode == "pause" {
            assert!(completion.cancelled);
            assert!(completion.error.is_some());
            assert_ne!(completion.exit_code, Some(130));
        } else if mode != "flood" {
            assert!(completion.error.is_some());
        }
    }
}
