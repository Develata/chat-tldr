//! Coordinates UI actions, bounded CLI events and a separate file-I/O worker.
#[path = "app/qce.rs"]
mod acquisition;
#[path = "app/qce_smoke.rs"]
pub(crate) mod acquisition_smoke;
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

use chat_tldr_core::ChatId;
use eframe::egui;

use crate::{
    Args, DemoView, appearance,
    bridge::{self, Bridge, CliSettings, CommandKind, Request, RequestTag},
    model::{FollowUp, GuiModel},
    prefs::{self, Preferences},
    ui::{self, Action, UiState},
};

pub struct Startup {
    settings: bool,
    prefs: Preferences,
    prefs_path: Option<PathBuf>,
    error: Option<String>,
    demo: bool,
    demo_view: Option<DemoView>,
    screenshot: Option<PathBuf>,
    smoke_report: Option<PathBuf>,
    quit_after_capture: bool,
    qce_smoke: Option<acquisition_smoke::Case>,
}

impl Startup {
    pub fn load(args: Args) -> Self {
        let mut error = None;
        let root = args
            .data_dir
            .clone()
            .map(Ok)
            .unwrap_or_else(prefs::default_data_dir);
        let prefs_path = match &root {
            Ok(root) => Some(root.join("gui-state.json")),
            Err(message) => {
                error = Some(message.clone());
                None
            }
        };
        let mut prefs = if args.demo {
            Preferences::default()
        } else {
            prefs_path
                .as_ref()
                .and_then(
                    |path| match prefs::load_profile(path, args.data_dir.is_none()) {
                        Ok(value) => Some(value),
                        Err(message) => {
                            error = Some(message);
                            None
                        }
                    },
                )
                .unwrap_or_default()
        };
        let accepted_connection = (
            prefs.cli.clone(),
            prefs.data_dir.clone(),
            prefs.config.clone(),
        );
        if let Some(path) = args.cli {
            prefs.cli = path.to_string_lossy().into_owned();
        }
        if prefs.cli.is_empty() {
            match bridge::default_cli_path() {
                Ok(path) => prefs.cli = path.to_string_lossy().into_owned(),
                Err(message) => error = Some(format!("无法定位 CLI：{message}")),
            }
        }
        if let Some(path) = args.data_dir {
            prefs.data_dir = path.to_string_lossy().into_owned();
        }
        if prefs.data_dir.is_empty()
            && let Ok(root) = root
        {
            prefs.data_dir = root.to_string_lossy().into_owned();
        }
        if let Some(path) = args.config {
            prefs.config = path.to_string_lossy().into_owned();
        }
        if let Err(message) = prefs.normalize_paths() {
            error = Some(message);
        }
        if (
            prefs.cli.clone(),
            prefs.data_dir.clone(),
            prefs.config.clone(),
        ) != accepted_connection
        {
            prefs.cloud_notice_accepted = false;
        }
        prefs.dark |= args.dark;
        Self {
            settings: args.settings,
            prefs,
            prefs_path,
            error,
            demo: args.demo,
            demo_view: args.demo_view,
            screenshot: args.screenshot,
            smoke_report: args.smoke_report,
            quit_after_capture: args.quit_after_capture,
            qce_smoke: args.qce_smoke,
        }
    }
}

enum IoRequest {
    PickFile,
    Save(Option<PathBuf>, Preferences),
    Screenshot(
        PathBuf,
        Arc<egui::ColorImage>,
        Option<(PathBuf, serde_json::Value)>,
    ),
    Stop,
}
enum IoResult {
    File(Option<PathBuf>),
    Saved(Result<(), String>),
    Screenshot(Result<(), String>),
}

pub struct App {
    model: GuiModel,
    bridge: Bridge,
    state: UiState,
    prefs: Preferences,
    prefs_path: Option<PathBuf>,
    sequence: u64,
    demo: bool,
    io_tx: mpsc::Sender<IoRequest>,
    io_rx: mpsc::Receiver<IoResult>,
    io_thread: Option<std::thread::JoinHandle<()>>,
    picking_file: bool,
    screenshot: Option<PathBuf>,
    smoke_report: Option<PathBuf>,
    screenshot_requested: bool,
    quit_after_capture: bool,
    frames: usize,
    qce: crate::qce::Wizard,
    qce_smoke: Option<acquisition_smoke::Script>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> Self {
        appearance::install(&cc.egui_ctx, startup.prefs.dark);
        let bridge = make_bridge(&startup.prefs, &cc.egui_ctx);
        let (io_tx, io_rx, io_thread) = io_worker(cc.egui_ctx.clone());
        let model = if startup.demo {
            GuiModel::demo()
        } else {
            GuiModel::default()
        };
        let mut state = UiState::new(startup.prefs.clone());
        state.settings = startup.settings;
        if startup.demo {
            state.configure_demo(startup.demo_view.unwrap_or_default());
        }
        let mut app = Self {
            model,
            bridge,
            state,
            prefs: startup.prefs,
            prefs_path: startup.prefs_path,
            sequence: 10,
            demo: startup.demo,
            io_tx,
            io_rx,
            io_thread: Some(io_thread),
            picking_file: false,
            screenshot: startup.screenshot,
            smoke_report: startup.smoke_report,
            screenshot_requested: false,
            quit_after_capture: startup.quit_after_capture,
            frames: 0,
            qce: crate::qce::Wizard::default(),
            qce_smoke: startup.qce_smoke.map(acquisition_smoke::Script::new),
        };
        if let Some(error) = startup.error {
            app.model.last_error = Some(error);
            app.state.settings = true;
        } else if !app.demo {
            app.start(CommandKind::Version, None, ["version".into()]);
        }
        app
    }

    fn start(
        &mut self,
        kind: CommandKind,
        chat: Option<ChatId>,
        args: impl IntoIterator<Item = OsString>,
    ) {
        self.start_with_input(kind, chat, args, None);
    }

    fn start_with_input(
        &mut self,
        kind: CommandKind,
        chat: Option<ChatId>,
        args: impl IntoIterator<Item = OsString>,
        input: Option<chat_tldr_core::settings::SecretString>,
    ) {
        if self.demo || self.bridge.is_busy() {
            return;
        }
        let args: Vec<OsString> = args.into_iter().collect();
        if kind != CommandKind::Version
            && (!self.model.handshake_ok()
                || !(match kind {
                    CommandKind::ConfigShow => Some("config show"),
                    CommandKind::ConfigSet => Some("config set"),
                    _ => args.first().and_then(|arg| arg.to_str()),
                })
                .is_some_and(|name| self.model.has_capability(name)))
        {
            self.model.last_error =
                Some("CLI 尚未通过版本检查，或不支持此操作；请检查设置。".into());
            return;
        }
        self.sequence += 1;
        let tag = RequestTag {
            id: self.sequence,
            kind,
            chat,
        };
        if let Err(error) = self.bridge.start_with_input(
            Request {
                tag: tag.clone(),
                args,
            },
            input,
        ) {
            self.model.last_error = Some(error);
            return;
        }
        if let Err(error) = self.model.begin(tag) {
            self.bridge.cancel();
            self.model.last_error = Some(error);
        }
    }

    fn inbox(&mut self, chat: ChatId) {
        let mut args: Vec<OsString> = vec!["inbox".into(), "--chat".into(), chat.0.clone().into()];
        if self.state.all {
            args.push("--all".into());
        }
        if self.state.resolved {
            args.push("--include-resolved".into());
        }
        self.start(CommandKind::Inbox, Some(chat), args);
    }

    fn save_preferences(&mut self) {
        if !self.demo && !self.prefs.data_dir.is_empty() {
            let _ = self
                .io_tx
                .send(IoRequest::Save(self.prefs_path.clone(), self.prefs.clone()));
        }
    }

    fn analyze(&mut self) {
        if let Some(chat) = self.model.selected_chat.clone() {
            self.start(
                CommandKind::Analyze,
                Some(chat.clone()),
                ["analyze".into(), "--chat".into(), chat.0.into()],
            );
        }
    }

    fn act(&mut self, action: Action, ctx: &egui::Context) {
        if action == Action::Cancel {
            self.bridge.cancel();
            return;
        }
        if action == Action::DismissError {
            self.model.last_error = None;
            return;
        }
        if self.demo || self.bridge.is_busy() || self.picking_file || self.qce.open {
            return;
        }
        let chat = self.model.selected_chat.clone();
        if matches!(
            action,
            Action::Select(_)
                | Action::Analyze
                | Action::Refresh
                | Action::PickImport
                | Action::Feedback(..)
                | Action::Resolve(..)
        ) {
            self.state.overview = false;
        }
        match action {
            Action::LoadProviders => {
                if self.state.providers.connection_matches(&self.state.draft) {
                    self.state.providers.attempted_load = true;
                    self.state.providers.saving = None;
                    self.state.providers.message = None;
                    self.start(
                        CommandKind::ConfigShow,
                        None,
                        ["config".into(), "show".into()],
                    );
                }
            }
            Action::SaveProvider(provider) => {
                if !self.state.providers.connection_matches(&self.state.draft) {
                    return;
                }
                let update = match self.state.providers.update(provider) {
                    Ok(update) => update,
                    Err(error) => {
                        self.model.last_error = Some(error);
                        return;
                    }
                };
                let json = match serde_json::to_string(&update) {
                    Ok(json) => chat_tldr_core::settings::SecretString::new(json),
                    Err(_) => {
                        self.model.last_error = Some("无法编码模型配置".into());
                        return;
                    }
                };
                self.state.providers.saving = Some(provider);
                self.state.providers.message = None;
                self.start_with_input(
                    CommandKind::ConfigSet,
                    None,
                    [
                        "config".into(),
                        "set".into(),
                        provider.into(),
                        "--request-stdin".into(),
                    ],
                    Some(json),
                );
            }
            Action::OpenQce => {
                self.state.cloud_notice = false;
                self.state.cloud_target = None;
                self.qce.open(&self.prefs, ctx);
            }
            Action::Overview(hours) => {
                if let Some(chat) = chat {
                    let until = chrono::Utc::now();
                    let since = until - chrono::Duration::hours(i64::from(hours));
                    self.start(
                        CommandKind::Overview,
                        Some(chat.clone()),
                        [
                            "overview".into(),
                            "--chat".into(),
                            chat.0.into(),
                            "--since".into(),
                            since.to_rfc3339().into(),
                            "--until".into(),
                            until.to_rfc3339().into(),
                        ],
                    );
                }
            }
            Action::Select(chat) => {
                self.state.cloud_notice = false;
                self.state.cloud_target = None;
                self.model.select_chat(chat.clone());
                self.inbox(chat);
            }
            Action::Refresh => self.start(CommandKind::Chats, None, ["chats".into()]),
            Action::PickImport => {
                self.picking_file = true;
                if self.io_tx.send(IoRequest::PickFile).is_err() {
                    self.picking_file = false;
                    self.model.last_error = Some("文件选择线程已停止".into());
                }
            }
            Action::Analyze => {
                if self.prefs.cloud_notice_accepted {
                    self.analyze();
                } else {
                    self.state.cloud_target = self.model.selected_chat.clone();
                    self.state.cloud_notice = true;
                }
            }
            Action::AcceptCloud(target) => {
                let matches = self.state.cloud_notice
                    && self.state.cloud_target.as_ref() == Some(&target)
                    && self.model.selected_chat.as_ref() == Some(&target);
                self.state.cloud_notice = false;
                self.state.cloud_target = None;
                if !matches {
                    self.model.last_error = Some("待分析群聊已变更，请重新点击分析并确认。".into());
                    return;
                }
                self.prefs.cloud_notice_accepted = true;
                self.state.draft.cloud_notice_accepted = true;
                self.save_preferences();
                self.analyze();
            }
            Action::MarkRead => {
                if self.state.overview {
                    return;
                }
                if let (Some(chat), Some(cursor)) = (chat, self.model.mark_read_cursor()) {
                    self.start(
                        CommandKind::MarkRead,
                        Some(chat.clone()),
                        [
                            "mark-read".into(),
                            "--chat".into(),
                            chat.0.into(),
                            "--up-to".into(),
                            cursor.to_string().into(),
                        ],
                    );
                }
            }
            Action::Feedback(id, useful) => self.start(
                CommandKind::Feedback,
                chat,
                [
                    "feedback".into(),
                    id.into(),
                    if useful {
                        "--useful"
                    } else {
                        "--not-important"
                    }
                    .into(),
                ],
            ),
            Action::Resolve(id, flag) => self.start(
                CommandKind::Resolve,
                chat,
                ["resolve".into(), id.into(), flag.into()],
            ),
            Action::SaveSettings => {
                let mut candidate = self.state.draft.clone();
                if let Err(error) = candidate.normalize_paths() {
                    self.model.last_error = Some(error);
                    return;
                }
                if candidate.cli != self.prefs.cli
                    || candidate.data_dir != self.prefs.data_dir
                    || candidate.config != self.prefs.config
                {
                    candidate.cloud_notice_accepted = false;
                }
                self.state.cloud_notice = false;
                self.state.cloud_target = None;
                self.prefs = candidate;
                self.state.draft = self.prefs.clone();
                self.state.providers = ui::settings::ProviderPanel::new(&self.prefs);
                self.save_preferences();
                self.bridge = make_bridge(&self.prefs, ctx);
                self.model = GuiModel::default();
                self.state.settings = false;
                self.start(CommandKind::Version, None, ["version".into()]);
            }
            Action::Stats => {
                let mut args = vec!["stats".into()];
                if let Some(chat) = &chat {
                    args.extend([OsString::from("--chat"), chat.0.clone().into()]);
                }
                self.start(CommandKind::Stats, chat, args);
            }
            Action::Decisions | Action::JevLog => {
                if let Some(stats) = &self.model.stats {
                    let (kind, name) = if action == Action::Decisions {
                        (CommandKind::Decisions, "decisions")
                    } else {
                        (CommandKind::JevLog, "jev-log")
                    };
                    self.start(
                        kind,
                        chat,
                        [name.into(), "--run".into(), stats.run_id.0.clone().into()],
                    );
                }
            }
            Action::Cancel | Action::DismissError => {}
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // A continuous producer cannot monopolize a UI frame.
        for _ in 0..128 {
            let Some(event) = self.bridge.try_recv() else {
                break;
            };
            let qce_import = self.model.active.as_ref() == Some(&event.request)
                && self.qce.importing
                && event.request.kind == CommandKind::Import
                && matches!(event.payload, crate::bridge::BridgePayload::Finished(_));
            self.observe_qce_import(&event);
            let config_saved = self.model.active.as_ref() == Some(&event.request)
                && event.request.kind == CommandKind::ConfigSet
                && matches!(event.payload, crate::bridge::BridgePayload::Finished(_));
            let follow_up = self.model.apply(event);
            if config_saved
                && self
                    .model
                    .last_completion
                    .as_ref()
                    .is_some_and(crate::bridge::Completion::is_success)
            {
                self.state.providers.sync(&self.model);
                self.prefs.cloud_notice_accepted = false;
                self.state.draft.cloud_notice_accepted = false;
                self.state.cloud_notice = false;
                self.state.cloud_target = None;
                self.save_preferences();
            }
            if qce_import {
                let success = self
                    .model
                    .last_completion
                    .as_ref()
                    .is_some_and(crate::bridge::Completion::is_success)
                    && self.qce.imported_chat.is_some();
                self.qce.import_finished(success);
                if !success && let Some(error) = &self.model.last_error {
                    self.qce.model.error =
                        Some(format!("导入未完成：{error}。导出文件已保留，可重试导入。"));
                }
                if success && let Some(chat) = self.qce.imported_chat.clone() {
                    self.model.select_chat(chat);
                }
            }
            if let Some(follow_up) = follow_up {
                match follow_up {
                    FollowUp::Chats => self.start(CommandKind::Chats, None, ["chats".into()]),
                    FollowUp::Inbox(chat) => self.inbox(chat),
                }
            }
        }
        if self.state.settings
            && !self.state.providers.attempted_load
            && !self.bridge.is_busy()
            && !self.qce.open
            && !self.demo
            && self.model.has_capability("config show")
            && self.state.providers.connection_matches(&self.state.draft)
        {
            self.act(Action::LoadProviders, ctx);
        }
        if let Some(path) = self.qce.tick() {
            self.import_qce(path);
        }
        self.advance_qce_smoke(ctx);
        if self.bridge.is_busy() || self.qce.busy() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        while let Ok(result) = self.io_rx.try_recv() {
            match result {
                IoResult::File(path) => {
                    self.picking_file = false;
                    if let Some(path) = path {
                        self.start(
                            CommandKind::Import,
                            None,
                            ["import".into(), path.into_os_string()],
                        );
                    }
                }
                IoResult::Saved(Err(error)) => self.model.last_error = Some(error),
                IoResult::Screenshot(result) => {
                    if let Err(error) = result {
                        eprintln!("{error}");
                        self.model.last_error = Some(error);
                    }
                    if self.quit_after_capture {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                IoResult::Saved(Ok(())) => {}
            }
        }
        let captures: Vec<_> = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::Screenshot { image, .. } = event {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
                .collect()
        });
        for image in captures {
            if let Some(path) = self.screenshot.take() {
                let report = self.smoke_report.take().map(|path| {
                    let mut report = crate::capture::report(&self.model, self.demo);
                    report["settings"] = serde_json::json!({
                        "open": self.state.settings,
                        "loaded": self.model.settings.is_some(),
                        "controls_visible": self.state.providers.key_visible && self.state.providers.save_visible,
                        "llm_format": self.model.settings.as_ref().map(|(_, s)| &s.llm.api_format),
                        "llm_key_available": self.model.settings.as_ref().is_some_and(|(_, s)| s.llm.credential.available),
                    });
                    if let Some(script) = &self.qce_smoke {
                        report["qce"] = script.report(&self.qce);
                    }
                    (path, report)
                });
                let _ = self.io_tx.send(IoRequest::Screenshot(path, image, report));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let (actions, displayed) = ui
            .add_enabled_ui(!self.qce.open, |ui| {
                ui::render(
                    ui,
                    &mut self.state,
                    &self.model,
                    self.bridge.is_busy(),
                    self.picking_file,
                    self.demo,
                )
            })
            .inner;
        if let Some(request_id) = displayed
            && !self.qce.open
        {
            self.model.mark_inbox_displayed(request_id);
        }
        for action in actions {
            self.act(action, ui.ctx());
        }
        if let Some(action) = self.qce.ui(
            ui.ctx(),
            !self.bridge.is_busy()
                && self.model.handshake_ok()
                && self.model.has_capability("import"),
        ) {
            self.act_qce(action, ui.ctx());
        }
        self.frames += 1;
        if self.screenshot.is_some() && !self.screenshot_requested {
            if self.frames >= 4
                && !self.bridge.is_busy()
                && self.model.active.is_none()
                && !self.picking_file
                && self
                    .qce_smoke
                    .as_ref()
                    .map_or(!self.qce.busy(), |s| s.ready)
            {
                self.screenshot_requested = true;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn make_bridge(prefs: &Preferences, ctx: &egui::Context) -> Bridge {
    let ctx = ctx.clone();
    Bridge::new(
        CliSettings {
            executable: PathBuf::from(&prefs.cli),
            data_dir: (!prefs.data_dir.is_empty()).then(|| PathBuf::from(&prefs.data_dir)),
            config: (!prefs.config.is_empty()).then(|| PathBuf::from(&prefs.config)),
        },
        move || ctx.request_repaint(),
    )
}

fn io_worker(
    ctx: egui::Context,
) -> (
    mpsc::Sender<IoRequest>,
    mpsc::Receiver<IoResult>,
    std::thread::JoinHandle<()>,
) {
    let (tx, requests) = mpsc::channel();
    let (results, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        for request in requests {
            let result = match request {
                IoRequest::PickFile => {
                    // A native modal chooser must not delay preference commits
                    // or shutdown of the file writer. It owns no CLI process.
                    let results = results.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let path = rfd::FileDialog::new()
                            .add_filter("QCE JSON", &["json"])
                            .pick_file();
                        let _ = results.send(IoResult::File(path));
                        ctx.request_repaint();
                    });
                    continue;
                }
                IoRequest::Save(path, value) => {
                    IoResult::Saved(prefs::save_profile(path.as_deref(), &value))
                }
                IoRequest::Screenshot(path, image, report) => {
                    IoResult::Screenshot(crate::capture::save(&path, &image, report))
                }
                IoRequest::Stop => break,
            };
            if results.send(result).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (tx, rx, worker)
}

impl Drop for App {
    fn drop(&mut self) {
        self.bridge.cancel();
        self.qce.cancel();
        // FIFO Stop waits for all requested atomic preference writes. The
        // picker runs separately, so an open dialog cannot block this drain.
        let _ = self.io_tx.send(IoRequest::Stop);
        if let Some(worker) = self.io_thread.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
#[path = "app/tests.rs"]
mod tests;
