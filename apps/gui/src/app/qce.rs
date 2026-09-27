use super::App;
use crate::{
    bridge::{BridgeEvent, BridgePayload, CommandKind},
    qce::Action,
};
use chat_tldr_core::{ChatId, EventBody};
use eframe::egui;
use std::path::PathBuf;

impl App {
    pub(super) fn import_qce(&mut self, path: PathBuf) {
        if self.bridge.is_busy()
            || !self.model.handshake_ok()
            || !self.model.has_capability("import")
        {
            self.qce.model.error = Some("主 CLI 尚未就绪；导出文件已保留，请稍后重试导入。".into());
            return;
        }
        self.state.overview = false;
        self.start(
            CommandKind::Import,
            None,
            ["import".into(), path.into_os_string()],
        );
        if self.bridge.is_busy() {
            self.qce.import_started();
        }
    }

    pub(super) fn observe_qce_import(&mut self, event: &BridgeEvent) {
        if !self.qce.importing
            || event.request.kind != CommandKind::Import
            || self.model.active.as_ref() != Some(&event.request)
        {
            return;
        }
        if let BridgePayload::Event(event) = &event.payload
            && let EventBody::Ack(ack) = &event.body
            && ack.command == "import"
            && let Some(value) = ack.detail.get("chat_ids")
            && let Ok(chats) = serde_json::from_value::<Vec<ChatId>>(value.clone())
            && chats.len() == 1
        {
            self.qce.imported_chat = chats.into_iter().next();
        }
    }

    pub(super) fn act_qce(&mut self, action: Action, ctx: &egui::Context) {
        if matches!(action, Action::Cancel | Action::Close) {
            self.qce.cancel();
            if self.qce.importing {
                self.bridge.cancel();
            }
            if action == Action::Close {
                self.qce.open = false;
            }
            return;
        }
        if self.bridge.is_busy() || self.qce.busy() || self.demo {
            return;
        }
        match action {
            Action::Connect => {
                if self.qce.connect(&self.prefs, ctx) {
                    self.prefs.qce = self.qce.settings.clone();
                    self.state.draft.qce = self.prefs.qce.clone();
                    self.save_preferences();
                }
            }
            Action::Login => self.qce.login(),
            Action::Export => self.qce.export(),
            Action::RetryImport => {
                if let Some(path) = self.qce.retry_path() {
                    self.import_qce(path);
                }
            }
            Action::Cancel | Action::Close => {}
        }
    }
}
