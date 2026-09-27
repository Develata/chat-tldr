//! Optional QQ acquisition workflow; uses the manager and main CLI as separate children.
mod model;
mod ui;
pub use ui::Action;
#[cfg(test)]
mod tests;

use crate::{
    bridge::{Bridge, CliSettings, CommandKind, Request, RequestTag},
    prefs::Preferences,
};
use chrono::{DateTime, Local, SecondsFormat};
use eframe::egui;
use model::{Model, Next};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub executable: String,
    pub base_url: String,
    pub napcat_url: String,
    pub docker: String,
    pub qce_config_dir: String,
    pub napcat_config_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            executable: String::new(),
            base_url: "http://127.0.0.1:40653".into(),
            napcat_url: "http://127.0.0.1:6099".into(),
            docker: String::new(),
            qce_config_dir: String::new(),
            napcat_config_dir: String::new(),
        }
    }
}

impl Settings {
    fn validate_urls(&self) -> Result<(), String> {
        for value in [&self.base_url, &self.napcat_url] {
            let error = || {
                "服务地址须为本机 HTTP(S) 地址，不得包含用户名、密码、路径或查询参数。".to_owned()
            };
            let url = url::Url::parse(value).map_err(|_| error())?;
            let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
            if !matches!(url.scheme(), "http" | "https")
                || !(host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback()))
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err(error());
            }
        }
        Ok(())
    }
}

pub struct Wizard {
    pub open: bool,
    pub model: Model,
    pub settings: Settings,
    connected_settings: Settings,
    connected_data_dir: String,
    bridge: Option<Bridge>,
    sequence: u64,
    pub selected: Option<usize>,
    pub search: String,
    pub groups_only: bool,
    pub since: String,
    pub until: String,
    default_range: (String, String),
    pub importing: bool,
    pub imported: bool,
    pub import_attempts: usize,
    pub imported_chat: Option<chat_tldr_core::ChatId>,
    pub qr_fully_visible: bool,
    cancel_requested: bool,
}

impl Default for Wizard {
    fn default() -> Self {
        let until = Local::now();
        let since = until - chrono::Duration::hours(24);
        let since = since.to_rfc3339_opts(SecondsFormat::Millis, false);
        let until = until.to_rfc3339_opts(SecondsFormat::Millis, false);
        Self {
            open: false,
            model: Model::default(),
            settings: Settings::default(),
            connected_settings: Settings::default(),
            connected_data_dir: String::new(),
            bridge: None,
            sequence: 0,
            selected: None,
            search: String::new(),
            groups_only: true,
            default_range: (since.clone(), until.clone()),
            since,
            until,
            importing: false,
            imported: false,
            import_attempts: 0,
            imported_chat: None,
            qr_fully_visible: false,
            cancel_requested: false,
        }
    }
}

impl Wizard {
    pub fn busy(&self) -> bool {
        self.importing || self.bridge.as_ref().is_some_and(Bridge::is_busy)
    }

    pub fn settings_changed(&self) -> bool {
        self.settings != self.connected_settings
    }

    pub fn open(&mut self, prefs: &Preferences, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        self.open = true;
        if self.connected_data_dir == prefs.data_dir
            && self.settings == prefs.qce
            && self.model.export.is_some()
            && !self.imported
        {
            return; // Closing the window does not discard a recoverable export.
        }
        self.settings = prefs.qce.clone();
        self.connect(prefs, ctx);
    }

    pub fn connect(&mut self, prefs: &Preferences, ctx: &egui::Context) -> bool {
        if self.busy() {
            return false;
        }
        if let Err(error) = self.settings.validate_urls() {
            self.model.error = Some(error);
            return false;
        }
        for value in [
            &mut self.settings.executable,
            &mut self.settings.qce_config_dir,
            &mut self.settings.napcat_config_dir,
        ] {
            if !value.trim().is_empty() {
                match std::path::absolute(value.trim()) {
                    Ok(path) => *value = path.to_string_lossy().into_owned(),
                    Err(_) => {
                        self.model.error = Some("连接配置中的路径无效。".into());
                        return false;
                    }
                }
            }
        }
        let executable = if self.settings.executable.trim().is_empty() {
            std::env::current_exe().map(|p| {
                p.with_file_name(if cfg!(windows) {
                    "chat-tldr-qce-manager.exe"
                } else {
                    "chat-tldr-qce-manager"
                })
            })
        } else {
            std::path::absolute(&self.settings.executable)
        };
        let executable = match executable {
            Ok(value) => value,
            Err(_) => {
                self.model.error = Some("无法定位 QCE 管理程序，请检查高级设置。".into());
                return false;
            }
        };
        let ctx = ctx.clone();
        self.bridge = Some(Bridge::new(
            CliSettings {
                executable,
                data_dir: Some(PathBuf::from(&prefs.data_dir)),
                config: None,
            },
            move || ctx.request_repaint(),
        ));
        self.connected_settings = self.settings.clone();
        self.connected_data_dir = prefs.data_dir.clone();
        // Refresh an untouched default when starting a new acquisition or
        // checking the connection. Never rewrite an explicitly edited range.
        if self.since == self.default_range.0 && self.until == self.default_range.1 {
            let until = Local::now();
            self.since =
                (until - chrono::Duration::hours(24)).to_rfc3339_opts(SecondsFormat::Millis, false);
            self.until = until.to_rfc3339_opts(SecondsFormat::Millis, false);
            self.default_range = (self.since.clone(), self.until.clone());
        }
        self.model = Model::default();
        self.selected = None;
        self.imported = false;
        self.start(CommandKind::QceVersion, vec!["version".into()]);
        true
    }

    fn start(&mut self, kind: CommandKind, mut args: Vec<OsString>) {
        if self.busy() || (kind != CommandKind::QceVersion && !self.model.compatible) {
            return;
        }
        let Some(bridge) = &mut self.bridge else {
            return;
        };
        args.extend([
            "--base-url".into(),
            self.connected_settings.base_url.clone().into(),
            "--napcat-url".into(),
            self.connected_settings.napcat_url.clone().into(),
        ]);
        for (flag, value) in [
            ("--docker", &self.connected_settings.docker),
            ("--qce-config-dir", &self.connected_settings.qce_config_dir),
            (
                "--napcat-config-dir",
                &self.connected_settings.napcat_config_dir,
            ),
        ] {
            if !value.trim().is_empty() {
                args.extend([flag.into(), value.clone().into()]);
            }
        }
        self.sequence += 1;
        let tag = RequestTag {
            id: self.sequence,
            kind,
            chat: None,
        };
        match bridge.start(Request {
            tag: tag.clone(),
            args,
        }) {
            Ok(()) => {
                self.cancel_requested = false;
                self.model.begin(tag);
            }
            Err(error) => self.model.error = Some(error),
        }
    }

    pub fn login(&mut self) {
        self.login_bounded(180);
    }

    pub fn login_bounded(&mut self, seconds: u64) {
        if self.settings_changed() {
            return;
        }
        self.selected = None;
        self.start(
            CommandKind::QceLogin,
            vec![
                "login".into(),
                "--qr-events".into(),
                "--max-wait-secs".into(),
                seconds.to_string().into(),
            ],
        );
    }

    pub fn export(&mut self) {
        if self.settings_changed() || !self.model.status.as_ref().is_some_and(|s| s.qce_ready) {
            return;
        }
        let Some(contact) = self
            .selected
            .and_then(|index| self.model.contacts.get(index))
        else {
            return;
        };
        let (since, until) = match time_range(&self.since, &self.until) {
            Ok(range) => range,
            Err(error) => {
                self.model.error = Some(error);
                return;
            }
        };
        let args = vec![
            "export".into(),
            "--type".into(),
            contact.chat_type.clone().into(),
            "--peer".into(),
            contact.peer_uid.clone().into(),
            "--since".into(),
            since.into(),
            "--until".into(),
            until.into(),
        ];
        self.imported = false;
        self.start(CommandKind::QceExport, args);
    }

    pub fn cancel(&mut self) {
        if let Some(bridge) = &self.bridge {
            bridge.cancel();
        }
        self.cancel_requested = true;
        self.model.qr = None;
    }

    pub fn tick(&mut self) -> Option<PathBuf> {
        for _ in 0..128 {
            let Some(event) = self.bridge.as_mut().and_then(Bridge::try_recv) else {
                break;
            };
            let next = self.model.apply(event);
            if self.cancel_requested {
                self.model.qr = None;
                continue;
            }
            match next {
                Some(Next::Status) => self.start(CommandKind::QceStatus, vec!["status".into()]),
                Some(Next::Chats) => self.start(CommandKind::QceChats, vec!["chats".into()]),
                Some(Next::Import(path)) if self.open => return Some(path),
                _ => {}
            }
        }
        None
    }

    pub fn retry_path(&self) -> Option<PathBuf> {
        (!self.busy() && !self.imported)
            .then(|| self.model.export.as_ref().map(|e| e.path.clone()))
            .flatten()
    }

    pub fn import_started(&mut self) {
        self.importing = true;
        self.imported_chat = None;
        self.import_attempts += 1;
        self.model.error = None;
    }

    pub fn import_finished(&mut self, success: bool) {
        self.importing = false;
        self.imported = success;
        if !success {
            self.model.error =
                Some("导出文件已保留，导入未完成。请查看主窗口错误后重试导入。".into());
        }
    }
}

fn time_range(since: &str, until: &str) -> Result<(String, String), String> {
    let parse = |s: &str| {
        DateTime::parse_from_rfc3339(s.trim())
            .map_err(|_| "时间须为带时区的 RFC3339，例如 2026-09-27T09:00:00-03:00".to_owned())
    };
    let since = parse(since)?;
    let until = parse(until)?;
    if since > until
        || [since, until]
            .iter()
            .any(|t| t.timestamp_subsec_nanos() % 1_000_000 != 0)
    {
        return Err("开始时间不得晚于结束时间，精度最多到毫秒。".into());
    }
    Ok((
        since.to_rfc3339_opts(SecondsFormat::Millis, false),
        until.to_rfc3339_opts(SecondsFormat::Millis, false),
    ))
}
