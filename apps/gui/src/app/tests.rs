use super::*;
use crate::model::Capabilities;
use clap::Parser;

fn app(prefs: Preferences, launch: PathBuf) -> App {
    let ctx = egui::Context::default();
    let (io_tx, io_rx, worker) = io_worker(ctx.clone());
    let mut model = GuiModel::demo();
    model.capabilities = Some(Capabilities {
        commands: vec!["analyze".into(), "inbox".into()],
        strategies: vec!["ours".into()],
        deciders: vec!["jev".into()],
    });
    App {
        model,
        bridge: make_bridge(&prefs, &ctx),
        state: UiState::new(prefs.clone()),
        prefs,
        prefs_path: Some(launch),
        sequence: 10,
        demo: false,
        io_tx,
        io_rx,
        io_thread: Some(worker),
        picking_file: false,
        screenshot: None,
        screenshot_requested: false,
        quit_after_capture: false,
        frames: 0,
    }
}

fn profile(root: &std::path::Path) -> Preferences {
    Preferences {
        // Never run a real CLI or call a cloud provider from these tests.
        cli: root.join("missing-test-cli").to_string_lossy().into_owned(),
        data_dir: root.join("data").to_string_lossy().into_owned(),
        ..Default::default()
    }
}

#[test]
fn stale_cloud_confirmation_cannot_accept_or_start_analysis_for_another_chat() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(profile(dir.path()), dir.path().join("gui-state.json"));
    let ctx = egui::Context::default();
    let original = app.model.selected_chat.clone().unwrap();
    app.act(Action::Analyze, &ctx);
    assert_eq!(app.state.cloud_target.as_ref(), Some(&original));
    app.model.select_chat("another-chat".into());
    app.act(Action::AcceptCloud(original), &ctx);
    assert!(!app.prefs.cloud_notice_accepted);
    assert!(!app.bridge.is_busy());
    assert!(app.model.active.is_none());
    assert!(
        app.model
            .last_error
            .as_ref()
            .unwrap()
            .contains("群聊已变更")
    );
}

#[test]
fn changing_connection_cancels_confirmation_and_requires_a_new_cloud_notice() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = profile(dir.path());
    preferences.cloud_notice_accepted = true;
    let mut app = app(preferences, dir.path().join("gui-state.json"));
    app.state.cloud_notice = true;
    app.state.cloud_target = app.model.selected_chat.clone();
    app.state.draft.config = dir.path().join("other.toml").to_string_lossy().into_owned();
    app.act(Action::SaveSettings, &egui::Context::default());
    assert!(!app.prefs.cloud_notice_accepted);
    assert!(!app.state.draft.cloud_notice_accepted);
    assert!(!app.state.cloud_notice);
    assert!(app.state.cloud_target.is_none());
}

#[test]
fn closing_gui_flushes_preference_writes_before_worker_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let launch = dir.path().join("gui-state.json");
    let preferences = profile(dir.path());
    let active = PathBuf::from(&preferences.data_dir).join("gui-state.json");
    let mut app = app(preferences.clone(), launch.clone());
    app.save_preferences();
    drop(app);
    assert_eq!(prefs::load(&active).unwrap().data_dir, preferences.data_dir);
    assert_eq!(prefs::load(&launch).unwrap().data_dir, preferences.data_dir);
    assert_eq!(
        prefs::load_profile(&launch, true).unwrap().data_dir,
        preferences.data_dir
    );
}

#[test]
fn startup_config_override_does_not_reuse_another_connection_cloud_consent() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = profile(dir.path());
    preferences.cloud_notice_accepted = true;
    prefs::save_profile(None, &preferences).unwrap();
    let unchanged =
        Startup::load(Args::try_parse_from(["gui", "--data-dir", &preferences.data_dir]).unwrap());
    assert!(unchanged.prefs.cloud_notice_accepted);
    let changed = Startup::load(
        Args::try_parse_from([
            "gui",
            "--data-dir",
            &preferences.data_dir,
            "--config",
            &dir.path().join("other.toml").to_string_lossy(),
        ])
        .unwrap(),
    );
    assert!(!changed.prefs.cloud_notice_accepted);
}
