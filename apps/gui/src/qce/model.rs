//! Manager-specific protocol state. Never shares an inbox or a database handle.
use std::{collections::BTreeSet, path::PathBuf};

use chat_tldr_core::{EventBody, SCHEMA_VERSION};
use serde::Deserialize;

use crate::bridge::{BridgeEvent, BridgePayload, CommandKind, RequestTag};

#[derive(Clone, Deserialize)]
pub struct Contact {
    pub chat_type: String,
    pub peer_uid: String,
    pub display_name: String,
    #[serde(skip)]
    pub search_name: String,
}

#[derive(Clone, Deserialize)]
pub struct Status {
    pub qce_reachable: bool,
    pub authenticated: bool,
    pub qq_logged_in: Option<bool>,
    pub qce_ready: bool,
}

#[derive(Clone, Deserialize)]
pub struct Export {
    pub path: PathBuf,
    pub sha256: String,
    pub message_count: u64,
}

#[derive(Deserialize)]
struct Version {
    name: String,
    version: String,
    schema_version: String,
    capabilities: Vec<String>,
}

#[derive(Default)]
struct Pending {
    ack: bool,
    contacts: Vec<Contact>,
    keys: BTreeSet<(String, String)>,
    export: Option<Export>,
    invalid: bool,
}

#[derive(Default)]
pub struct Model {
    pub active: Option<RequestTag>,
    pub compatible: bool,
    pub status: Option<Status>,
    pub contacts: Vec<Contact>,
    pub export: Option<Export>,
    // QR content is represented only by the current matrix, never diagnostic text.
    pub qr: Option<qrcode::QrCode>,
    pub qr_updates: usize,
    pub error: Option<String>,
    pub progress: String,
    pending: Pending,
}

#[derive(Debug, PartialEq)]
pub enum Next {
    Status,
    Chats,
    Import(PathBuf),
}

impl Model {
    pub fn begin(&mut self, tag: RequestTag) {
        self.error = None;
        self.qr = None;
        self.progress = "正在连接…".into();
        self.pending = Pending::default();
        match tag.kind {
            CommandKind::QceVersion => self.compatible = false,
            CommandKind::QceStatus | CommandKind::QceLogin => {
                self.status = None;
                self.contacts.clear();
            }
            CommandKind::QceChats => self.contacts.clear(),
            CommandKind::QceExport => self.export = None,
            _ => {}
        }
        self.active = Some(tag);
    }

    pub fn apply(&mut self, event: BridgeEvent) -> Option<Next> {
        if self.active.as_ref() != Some(&event.request) {
            return None;
        }
        let kind = event.request.kind;
        match event.payload {
            // Manager stderr is deliberately not stored in the general activity log.
            BridgePayload::Stderr(_) => {}
            BridgePayload::Event(event) => {
                if self.pending.invalid || self.error.is_some() {
                    return None;
                }
                if let Err(()) = self.reduce(kind, event.body) {
                    self.pending.invalid = true;
                    self.qr = None;
                    self.error = Some("QCE 管理程序返回了无效或重复的数据，请更新组件。".into());
                }
            }
            BridgePayload::Finished(result) => {
                self.active = None;
                self.qr = None;
                self.progress.clear();
                if !result.is_success() || self.pending.invalid || self.error.is_some() {
                    if result.cancelled {
                        self.error = Some(
                            "已停止等待；QCE 服务端任务可能继续执行。本次不会自动导入。".into(),
                        );
                    } else if self.error.is_none() {
                        self.error = Some(
                            result
                                .error
                                .unwrap_or_else(|| "操作未成功，请检查连接后重试。".into()),
                        );
                    }
                    if kind == CommandKind::QceVersion {
                        self.compatible = false;
                    }
                    return None;
                }
                if kind != CommandKind::QceChats && !self.pending.ack {
                    self.error = Some("QCE 管理程序没有返回操作结果。".into());
                    self.compatible = false;
                    return None;
                }
                match kind {
                    CommandKind::QceVersion => {
                        self.compatible = true;
                        return Some(Next::Status);
                    }
                    CommandKind::QceStatus | CommandKind::QceLogin
                        if self.status.as_ref().is_some_and(|s| s.qce_ready) =>
                    {
                        return Some(Next::Chats);
                    }
                    CommandKind::QceChats => {
                        self.contacts = std::mem::take(&mut self.pending.contacts);
                        self.contacts.sort_by(|a, b| {
                            a.display_name
                                .cmp(&b.display_name)
                                .then_with(|| a.peer_uid.cmp(&b.peer_uid))
                        });
                    }
                    CommandKind::QceExport => {
                        self.export = self.pending.export.take();
                        return self
                            .export
                            .as_ref()
                            .map(|export| Next::Import(export.path.clone()));
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn reduce(&mut self, kind: CommandKind, body: EventBody) -> Result<(), ()> {
        match body {
            EventBody::Ack(ack) => {
                if self.pending.ack {
                    return Err(());
                }
                match (kind, ack.command.as_str()) {
                    (CommandKind::QceVersion, "version") => {
                        let version: Version =
                            serde_json::from_value(ack.detail).map_err(|_| ())?;
                        if version.name != "chat-tldr-qce-manager"
                            || version.version != env!("CARGO_PKG_VERSION")
                            || version.schema_version != SCHEMA_VERSION
                            || ["status", "login.qr-events", "chats", "export"].iter().any(
                                |required| !version.capabilities.iter().any(|cap| cap == required),
                            )
                        {
                            self.error = Some(
                                "QCE 管理程序版本不兼容，请使用与 GUI 同一版本的完整安装包。"
                                    .into(),
                            );
                            return Ok(());
                        }
                    }
                    (CommandKind::QceStatus, "status") => {
                        // Diagnostics are useful even when status ends with a nonzero code.
                        let status: Status = serde_json::from_value(ack.detail).map_err(|_| ())?;
                        if status.qce_ready
                            && (!status.qce_reachable
                                || !status.authenticated
                                || status.qq_logged_in != Some(true))
                        {
                            return Err(());
                        }
                        self.status = Some(status);
                    }
                    (CommandKind::QceLogin, "login") => {
                        if ack.detail.get("qce_ready").and_then(|v| v.as_bool()) != Some(true)
                            || ack.detail.get("qq_logged_in").and_then(|v| v.as_bool())
                                != Some(true)
                        {
                            return Err(());
                        }
                        self.status = Some(Status {
                            qce_reachable: true,
                            authenticated: true,
                            qq_logged_in: Some(true),
                            qce_ready: true,
                        });
                    }
                    (CommandKind::QceExport, "export") => {
                        let export: Export = serde_json::from_value(ack.detail).map_err(|_| ())?;
                        if !export.path.is_absolute()
                            || export.sha256.len() != 64
                            || !export.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                        {
                            return Err(());
                        }
                        self.pending.export = Some(export);
                    }
                    _ => return Err(()),
                }
                self.pending.ack = true;
            }
            EventBody::Unknown { event, payload } if event == "qce_chat" => {
                if kind != CommandKind::QceChats || self.pending.contacts.len() >= 2000 {
                    return Err(());
                }
                let mut row: Contact = serde_json::from_value(payload).map_err(|_| ())?;
                if !matches!(row.chat_type.as_str(), "group" | "private")
                    || row.peer_uid.is_empty()
                    || row.peer_uid.len() > 256
                    || row.peer_uid.chars().any(char::is_control)
                    || row.display_name.len() > 4096
                    || !self
                        .pending
                        .keys
                        .insert((row.chat_type.clone(), row.peer_uid.clone()))
                {
                    return Err(());
                }
                row.search_name = row.display_name.to_lowercase();
                self.pending.contacts.push(row);
            }
            EventBody::Unknown { event, payload } if event == "qce_login_qr" => {
                if kind != CommandKind::QceLogin
                    || payload.get("version").and_then(|v| v.as_u64()) != Some(1)
                {
                    return Err(());
                }
                let content = payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty() && s.len() <= 4096)
                    .ok_or(())?;
                self.qr = Some(qrcode::QrCode::new(content.as_bytes()).map_err(|_| ())?);
                self.qr_updates += 1;
            }
            EventBody::Progress(progress) => {
                if progress.stage == "logged_in" {
                    self.qr = None;
                }
                self.progress = progress.message.chars().take(512).collect();
            }
            EventBody::Error(error) => {
                self.qr = None;
                self.error = Some(format!(
                    "{}：{}",
                    error.code,
                    error.message.chars().take(2048).collect::<String>()
                ));
            }
            EventBody::Unknown { .. } | EventBody::Done(_) => {}
            _ => return Err(()),
        }
        Ok(())
    }
}
