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
 if mode=="listen" {
   let _listener=std::net::TcpListener::bind(&args[2]).unwrap();
   eprintln!("listener-ready");
   for seq in 0..5000 {
     println!("{{\"schema_version\":\"1.0\",\"run_id\":\"r_gui_test\",\"seq\":{seq},\"event\":\"future\",\"payload\":{{}}}}");
   }
   io::stdout().flush().unwrap();
   thread::sleep(Duration::from_secs(20)); return;
 }
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
    // A real kernel resource verifies that Drop reaps the child before returning,
    // including while stdout is blocked on this GUI's bounded event queue.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    bridge
        .start(Request {
            tag: tag(10),
            args: vec!["listen".into(), address.to_string().into()],
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(BridgeEvent {
            payload: BridgePayload::Stderr(text),
            ..
        }) = bridge.try_recv()
            && text.contains("listener-ready")
        {
            break;
        }
        assert!(Instant::now() < deadline, "helper did not own its port");
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(100));
    drop(bridge);
    let _released = std::net::TcpListener::bind(address)
        .expect("dropping GUI bridge must terminate its CLI before returning");
}

#[test]
fn cancelling_wrapper_does_not_wait_for_inherited_descendant_pipes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("wrapper.rs");
    let helper = temp.path().join(if cfg!(windows) {
        "wrapper.exe"
    } else {
        "wrapper"
    });
    fs::write(
        &source,
        r#"
use std::{env,fs,io::{self,Write},process::{Command,Stdio},thread,time::{Duration,Instant}};
fn main() {
 let args:Vec<_>=env::args_os().collect();
 if args[1]=="direct" {
   print!("{}", fs::read_to_string(&args[2]).unwrap());
   io::stdout().flush().unwrap(); return;
 }
 if args[1]=="descendant" {
   fs::write(&args[2], "ready").unwrap();
   let until=Instant::now()+Duration::from_secs(5);
   while !std::path::Path::new(&args[3]).exists() && Instant::now()<until {
     thread::sleep(Duration::from_millis(10));
   }
   print!("{}", fs::read_to_string(&args[5]).unwrap());
   let _=io::stdout().flush();
   fs::write(&args[4], "released").unwrap();
   return;
 }
 let mut command=Command::new(env::current_exe().unwrap());
 command.arg("descendant").args(&args[2..]).stdout(Stdio::inherit()).stderr(Stdio::inherit());
 #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x0800_0000); }
 let _child=command.spawn().unwrap();
 if args[1]=="active-wrapper" { thread::sleep(Duration::from_secs(20)); }
}
"#,
    )
    .unwrap();
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&helper)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let fixture = temp.path().join("stream.jsonl");
    fs::write(&fixture, format!("{}\n", done(0))).unwrap();
    for mode in [
        "active-wrapper",
        "exited-wrapper",
        "cancel-wrapper",
        "complete-wrapper",
    ] {
        let ready = temp.path().join(format!("{mode}-ready"));
        let release = temp.path().join(format!("{mode}-release"));
        let released = temp.path().join(format!("{mode}-released"));
        let cleanup = DescendantCleanup {
            release: release.clone(),
            released,
        };
        let mut bridge = Bridge::new(
            CliSettings {
                executable: helper.clone(),
                data_dir: None,
                config: None,
            },
            || {},
        );
        bridge
            .start(Request {
                tag: tag(1),
                args: vec![
                    mode.into(),
                    ready.clone().into_os_string(),
                    release.into_os_string(),
                    cleanup.released.clone().into_os_string(),
                    fixture.clone().into_os_string(),
                ],
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "descendant failed to start");
            thread::sleep(Duration::from_millis(5));
        }
        // Give the exited-wrapper branch time to enter reader draining before
        // cancelling. Its descendant keeps both pipes open independently.
        thread::sleep(Duration::from_millis(100));
        if mode == "complete-wrapper" {
            // A naturally exited wrapper is not enough for success: the stream
            // is still incomplete until the inherited writer supplies done.
            assert!(bridge.is_busy());
            while let Some(event) = bridge.try_recv() {
                assert!(!matches!(event.payload, BridgePayload::Finished(_)));
            }
            fs::write(&cleanup.release, "release").unwrap();
            assert!(finish(&mut bridge).is_success());
            drop(cleanup);
            continue;
        }
        if mode == "cancel-wrapper" {
            let start = Instant::now();
            bridge.cancel();
            let completion = finish(&mut bridge);
            assert!(completion.cancelled);
            assert!(!completion.is_success());
            assert!(start.elapsed() < Duration::from_secs(1));
            // Start a new request before releasing the old inherited writer.
            // Its late done must keep the original tag, so GuiModel can ignore
            // it (covered independently by stale_tags_and_chat_switches...).
            bridge
                .start(Request {
                    tag: tag(2),
                    args: vec!["direct".into(), fixture.clone().into_os_string()],
                })
                .unwrap();
            drop(cleanup);
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut old_done = false;
            let mut new_done = false;
            let mut new_finished = false;
            while !(old_done && new_done && new_finished) {
                while let Some(event) = bridge.try_recv() {
                    match event.payload {
                        BridgePayload::Event(wire) if matches!(wire.body, EventBody::Done(_)) => {
                            if event.request == tag(1) {
                                old_done = true;
                            } else {
                                assert_eq!(event.request, tag(2));
                                new_done = true;
                            }
                        }
                        BridgePayload::Finished(completion) => {
                            assert_eq!(event.request, tag(2));
                            assert!(completion.is_success());
                            new_finished = true;
                        }
                        _ => {}
                    }
                }
                assert!(Instant::now() < deadline, "tagged streams did not finish");
                thread::sleep(Duration::from_millis(5));
            }
            continue;
        }
        let start = Instant::now();
        drop(bridge);
        let elapsed = start.elapsed();
        drop(cleanup);
        assert!(
            elapsed < Duration::from_secs(1),
            "{mode}: GUI exit waited {elapsed:?} for descendant-owned pipes"
        );
    }
}

struct DescendantCleanup {
    release: PathBuf,
    released: PathBuf,
}

impl Drop for DescendantCleanup {
    fn drop(&mut self) {
        // The helper also has its own five-second bound in case the test aborts.
        let _ = fs::write(&self.release, "release");
        let deadline = Instant::now() + Duration::from_secs(6);
        while !self.released.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
    }
}
