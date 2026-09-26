//! Coordinates UI actions, bounded CLI events and a separate file-I/O worker.
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

use chat_tldr_core::ChatId;
use eframe::egui;

use crate::{
    Args, appearance,
    bridge::{self, Bridge, CliSettings, CommandKind, Request, RequestTag},
    model::{FollowUp, GuiModel},
    prefs::{self, Preferences},
    ui::{self, Action, UiState},
};

pub struct Startup {
    prefs: Preferences,
    prefs_path: Option<PathBuf>,
    error: Option<String>,
    demo: bool,
    screenshot: Option<PathBuf>,
    quit_after_capture: bool,
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
        prefs.dark |= args.dark;
        Self {
            prefs,
            prefs_path,
            error,
            demo: args.demo,
            screenshot: args.screenshot,
            quit_after_capture: args.quit_after_capture,
        }
    }
}

enum IoRequest {
    PickFile,
    Save(PathBuf, Preferences),
    Screenshot(PathBuf, Arc<egui::ColorImage>),
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
    picking_file: bool,
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    quit_after_capture: bool,
    frames: usize,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> Self {
        appearance::install(&cc.egui_ctx, startup.prefs.dark);
        let bridge = make_bridge(&startup.prefs, &cc.egui_ctx);
        let (io_tx, io_rx) = io_worker(cc.egui_ctx.clone());
        let model = if startup.demo {
            GuiModel::demo()
        } else {
            GuiModel::default()
        };
        let mut app = Self {
            model,
            bridge,
            state: UiState::new(startup.prefs.clone()),
            prefs: startup.prefs,
            prefs_path: startup.prefs_path,
            sequence: 10,
            demo: startup.demo,
            io_tx,
            io_rx,
            picking_file: false,
            screenshot: startup.screenshot,
            screenshot_requested: false,
            quit_after_capture: startup.quit_after_capture,
            frames: 0,
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
        if self.demo || self.bridge.is_busy() {
            return;
        }
        let args: Vec<OsString> = args.into_iter().collect();
        if kind != CommandKind::Version
            && (!self.model.handshake_ok()
                || !args
                    .first()
                    .and_then(|arg| arg.to_str())
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
        if let Err(error) = self.bridge.start(Request {
            tag: tag.clone(),
            args,
        }) {
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
            let active = PathBuf::from(&self.prefs.data_dir).join("gui-state.json");
            // Keep the launch profile as a pointer to the selected data directory,
            // and make launching directly with --data-dir <selected> equivalent.
            if self.prefs_path.as_ref().is_some_and(|path| path != &active) {
                let _ = self.io_tx.send(IoRequest::Save(
                    self.prefs_path.clone().unwrap(),
                    self.prefs.clone(),
                ));
            }
            let _ = self.io_tx.send(IoRequest::Save(active, self.prefs.clone()));
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
        if self.demo || self.bridge.is_busy() || self.picking_file {
            return;
        }
        let chat = self.model.selected_chat.clone();
        match action {
            Action::Select(chat) => {
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
                    self.state.cloud_notice = true;
                }
            }
            Action::AcceptCloud => {
                self.state.cloud_notice = false;
                self.prefs.cloud_notice_accepted = true;
                self.state.draft.cloud_notice_accepted = true;
                self.save_preferences();
                self.analyze();
            }
            Action::MarkRead => {
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
                self.prefs = self.state.draft.clone();
                if let Err(error) = self.prefs.normalize_paths() {
                    self.model.last_error = Some(error);
                    return;
                }
                self.state.draft = self.prefs.clone();
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
            Action::Cancel => {}
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
            if let Some(follow_up) = self.model.apply(event) {
                match follow_up {
                    FollowUp::Chats => self.start(CommandKind::Chats, None, ["chats".into()]),
                    FollowUp::Inbox(chat) => self.inbox(chat),
                }
            }
        }
        if self.bridge.is_busy() {
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
                let _ = self.io_tx.send(IoRequest::Screenshot(path, image));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let (actions, displayed) = ui::render(
            ui,
            &mut self.state,
            &self.model,
            self.bridge.is_busy() || self.picking_file,
            self.demo,
        );
        if let Some(request_id) = displayed {
            self.model.mark_inbox_displayed(request_id);
        }
        for action in actions {
            self.act(action, ui.ctx());
        }
        self.frames += 1;
        if self.screenshot.is_some() && !self.screenshot_requested {
            if self.frames >= 4 && !self.bridge.is_busy() && !self.picking_file {
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

fn io_worker(ctx: egui::Context) -> (mpsc::Sender<IoRequest>, mpsc::Receiver<IoResult>) {
    let (tx, requests) = mpsc::channel();
    let (results, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for request in requests {
            let result = match request {
                IoRequest::PickFile => IoResult::File(
                    rfd::FileDialog::new()
                        .add_filter("QCE JSON", &["json"])
                        .pick_file(),
                ),
                IoRequest::Save(path, value) => IoResult::Saved(prefs::save(&path, &value)),
                IoRequest::Screenshot(path, image) => {
                    let bytes: Vec<u8> = image
                        .pixels
                        .iter()
                        .flat_map(|pixel| pixel.to_array())
                        .collect();
                    IoResult::Screenshot(
                        image::save_buffer(
                            &path,
                            &bytes,
                            image.width() as u32,
                            image.height() as u32,
                            image::ColorType::Rgba8,
                        )
                        .map_err(|e| format!("截图保存失败：{e}")),
                    )
                }
            };
            if results.send(result).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (tx, rx)
}
