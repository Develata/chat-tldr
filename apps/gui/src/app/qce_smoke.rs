//! Explicit test automation. Real controls and the real two-child pipeline are reused.
//! The harness supplies only synthetic local services and an isolated data directory.
use super::App;
use crate::qce::{Action, Wizard};
use clap::ValueEnum;
use eframe::egui;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub(crate) enum Case {
    Connect,
    Login,
    Qr,
    Export,
    Cancel,
    RetryImport,
    LoginTimeout,
}

pub(crate) struct Script {
    case: Case,
    stage: u8,
    pub ready: bool,
}

impl Script {
    pub fn new(case: Case) -> Self {
        Self {
            case,
            stage: 0,
            ready: false,
        }
    }
    pub fn report(&self, wizard: &Wizard) -> Value {
        json!({"finished":self.ready,"compatible":wizard.model.compatible,"contacts":wizard.model.contacts.len(),
            "qr_updates":wizard.model.qr_updates,"qr_visible":wizard.model.qr.is_some(),"qr_fully_visible":wizard.qr_fully_visible,"exported":wizard.model.export.is_some(),
            "imported":wizard.imported,"import_attempts":wizard.import_attempts,"error":wizard.model.error})
    }
}

impl App {
    pub(super) fn advance_qce_smoke(&mut self, ctx: &egui::Context) {
        let Some(mut script) = self.qce_smoke.take() else {
            return;
        };
        if script.ready {
            self.qce_smoke = Some(script);
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        if std::env::var("CHAT_TLDR_QCE_TOKEN").as_deref() != Ok("synthetic-gui-smoke") {
            self.qce.model.error = Some("QCE smoke 仅允许隔离的合成测试环境。".into());
            script.ready = true;
        } else if script.stage == 0 && self.model.handshake_ok() && !self.bridge.is_busy() {
            self.qce.open(&self.prefs, ctx);
            script.stage = 1;
        } else if script.stage == 1 && !self.qce.busy() {
            match script.case {
                Case::Connect => script.ready = true,
                Case::Login | Case::Qr | Case::LoginTimeout => {
                    self.qce
                        .login_bounded(if script.case == Case::LoginTimeout {
                            2
                        } else {
                            15
                        });
                    script.stage = 2;
                }
                Case::Export | Case::Cancel | Case::RetryImport => {
                    self.qce.selected =
                        self.qce.model.contacts.iter().position(|c| {
                            c.peer_uid == "synthetic-study" && c.chat_type == "group"
                        });
                    if self.qce.selected.is_none() {
                        script.ready = true;
                    } else {
                        self.act_qce(Action::Export, ctx);
                        script.stage = 2;
                    }
                }
            }
        } else if script.stage == 2 {
            if script.case == Case::Qr && self.qce.model.qr_updates >= 2 {
                script.ready = true; // Capture the synthetic QR while login is still pending.
            } else if script.case == Case::Cancel && self.qce.model.progress.contains("等待 QCE")
            {
                self.act_qce(Action::Cancel, ctx);
                script.stage = 3;
            } else if !self.qce.busy() && !self.bridge.is_busy() {
                if script.case == Case::RetryImport && self.qce.retry_path().is_some() {
                    self.act_qce(Action::RetryImport, ctx);
                    script.stage = 3;
                } else {
                    script.ready = true;
                }
            }
        } else if script.stage == 3 && !self.qce.busy() && !self.bridge.is_busy() {
            script.ready = true;
        }
        self.qce_smoke = Some(script);
    }
}
