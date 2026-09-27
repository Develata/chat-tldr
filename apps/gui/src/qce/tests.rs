use super::{
    model::{Model, Next},
    *,
};
use crate::bridge::{BridgeEvent, BridgePayload, Completion};
use chat_tldr_core::{AckPayload, CliEvent, DonePayload, EventBody, RunStatus};
use serde_json::{Value, json};

#[path = "ui_tests.rs"]
mod ui_tests;

#[test]
fn connection_settings_reject_remote_or_credential_bearing_urls_before_saving() {
    for url in [
        "https://example.com",
        "http://secret@localhost:6099",
        "http://localhost/?token=secret",
        "http://127.0.0.1/path",
        "file:///tmp/private",
    ] {
        let settings = Settings {
            base_url: url.into(),
            ..Default::default()
        };
        let error = settings.validate_urls().unwrap_err();
        assert!(!error.contains("secret"));
    }
    for url in [
        "http://127.0.0.1:40654",
        "http://[::1]:6099",
        "https://localhost:12345",
    ] {
        assert!(
            Settings {
                base_url: url.into(),
                ..Default::default()
            }
            .validate_urls()
            .is_ok()
        );
    }
}

fn begin(model: &mut Model, kind: CommandKind, id: u64) -> RequestTag {
    let tag = RequestTag {
        id,
        kind,
        chat: None,
    };
    model.begin(tag.clone());
    tag
}
fn event(model: &mut Model, tag: &RequestTag, body: EventBody) -> Option<Next> {
    model.apply(BridgeEvent {
        request: tag.clone(),
        payload: BridgePayload::Event(Box::new(CliEvent::new("test-run".into(), 0, body))),
    })
}
fn ack(model: &mut Model, tag: &RequestTag, command: &str, detail: Value) {
    event(
        model,
        tag,
        EventBody::Ack(AckPayload {
            command: command.into(),
            target: None,
            changed: false,
            detail,
        }),
    );
}
fn finish(model: &mut Model, tag: &RequestTag, code: i32, cancelled: bool) -> Option<Next> {
    model.apply(BridgeEvent {
        request: tag.clone(),
        payload: BridgePayload::Finished(Completion {
            exit_code: Some(code),
            done: Some(DonePayload {
                status: if code == 0 {
                    RunStatus::Complete
                } else {
                    RunStatus::Failed
                },
                exit_code: code,
                finish_reason: None,
                elapsed_ms: 1,
            }),
            error: None,
            cancelled,
        }),
    })
}
fn export_detail() -> Value {
    json!({"path":std::env::temp_dir().join("synthetic messages.json"),"sha256":"ab".repeat(32),"message_count":3})
}

#[test]
fn export_requires_ack_and_successful_process_exit_before_import() {
    for (code, cancelled) in [(5, false), (0, true), (0, false)] {
        let mut model = Model::default();
        let tag = begin(&mut model, CommandKind::QceExport, 1);
        ack(&mut model, &tag, "export", export_detail());
        assert!(model.export.is_none());
        let result = finish(&mut model, &tag, code, cancelled);
        assert_eq!(
            matches!(result, Some(Next::Import(_))),
            code == 0 && !cancelled
        );
        assert_eq!(model.export.is_some(), code == 0 && !cancelled);
    }
    let mut model = Model::default();
    let tag = begin(&mut model, CommandKind::QceExport, 1);
    assert!(finish(&mut model, &tag, 0, false).is_none());
    assert!(model.error.is_some());
}

#[test]
fn duplicate_ack_and_relative_paths_cannot_be_imported() {
    for duplicate in [true, false] {
        let mut model = Model::default();
        let tag = begin(&mut model, CommandKind::QceExport, 1);
        let mut detail = export_detail();
        if !duplicate {
            detail["path"] = "relative.json".into();
        }
        ack(&mut model, &tag, "export", detail.clone());
        if duplicate {
            ack(&mut model, &tag, "export", detail);
        }
        assert!(finish(&mut model, &tag, 0, false).is_none());
        assert!(model.export.is_none());
    }
}

#[test]
fn incompatible_manager_and_stale_events_do_not_enable_actions() {
    let mut model = Model::default();
    let old = begin(&mut model, CommandKind::QceVersion, 1);
    let current = begin(&mut model, CommandKind::QceVersion, 2);
    let version = json!({"name":"chat-tldr-qce-manager","version":env!("CARGO_PKG_VERSION"),"schema_version":"1.0","capabilities":["status","login.qr-events","chats","export"]});
    ack(&mut model, &old, "version", version.clone());
    assert!(finish(&mut model, &old, 0, false).is_none());
    assert!(!model.compatible);
    let mut old_version = version.clone();
    old_version["capabilities"] = json!(["status", "login", "chats", "export"]);
    ack(&mut model, &current, "version", old_version);
    assert!(finish(&mut model, &current, 0, false).is_none());
    assert!(!model.compatible);
    let tag = begin(&mut model, CommandKind::QceVersion, 3);
    ack(&mut model, &tag, "version", version);
    assert_eq!(finish(&mut model, &tag, 0, false), Some(Next::Status));
    assert!(model.compatible);
}

#[test]
fn qr_replaces_in_memory_and_clears_on_login_or_failure() {
    let mut model = Model::default();
    let tag = begin(&mut model, CommandKind::QceLogin, 1);
    for content in ["synthetic-first", "synthetic-refreshed"] {
        event(
            &mut model,
            &tag,
            EventBody::Unknown {
                event: "qce_login_qr".into(),
                payload: json!({"version":1,"content":content}),
            },
        );
    }
    assert!(model.qr.is_some());
    assert_eq!(model.qr_updates, 2);
    event(
        &mut model,
        &tag,
        EventBody::Progress(chat_tldr_core::ProgressPayload {
            stage: "logged_in".into(),
            current: 0,
            total: None,
            message: "已登录".into(),
        }),
    );
    assert!(model.qr.is_none());
    event(
        &mut model,
        &tag,
        EventBody::Unknown {
            event: "qce_login_qr".into(),
            payload: json!({"version":1,"content":"synthetic-only"}),
        },
    );
    finish(&mut model, &tag, 5, false);
    assert!(model.qr.is_none());
    assert!(!model.error.unwrap().contains("synthetic-only"));
}

#[test]
fn status_failure_preserves_diagnostics_without_loading_contacts() {
    let mut model = Model::default();
    let tag = begin(&mut model, CommandKind::QceStatus, 1);
    ack(
        &mut model,
        &tag,
        "status",
        json!({"qce_reachable":false,"authenticated":false,"qq_logged_in":null,"qce_ready":false}),
    );
    assert!(finish(&mut model, &tag, 5, false).is_none());
    assert!(model.status.is_some());
    assert!(model.contacts.is_empty());
}

#[test]
fn contacts_publish_only_after_complete_and_duplicates_are_errors() {
    let mut model = Model::default();
    let tag = begin(&mut model, CommandKind::QceChats, 1);
    let contact = EventBody::Unknown {
        event: "qce_chat".into(),
        payload: json!({"chat_type":"group","peer_uid":"synthetic","display_name":"测试群"}),
    };
    event(&mut model, &tag, contact.clone());
    assert!(model.contacts.is_empty());
    event(&mut model, &tag, contact);
    assert!(finish(&mut model, &tag, 0, false).is_none());
    assert!(model.contacts.is_empty());
    assert!(model.error.is_some());
}

#[test]
fn failed_import_retains_exact_export_for_retry_and_time_is_explicit() {
    let mut wizard = Wizard::default();
    wizard.model.export = Some(serde_json::from_value(export_detail()).unwrap());
    let path = wizard.retry_path().unwrap();
    let prefs = Preferences {
        data_dir: "synthetic-data-dir".into(),
        ..Default::default()
    };
    wizard.connected_data_dir = prefs.data_dir.clone();
    wizard.open(&prefs, &egui::Context::default());
    assert_eq!(wizard.retry_path(), Some(path.clone()));
    assert!(
        wizard.bridge.is_none(),
        "reopening must not restart export or contact the service"
    );
    wizard.import_started();
    assert!(wizard.retry_path().is_none());
    wizard.import_finished(false);
    assert_eq!(wizard.retry_path(), Some(path));
    wizard.import_started();
    wizard.import_finished(true);
    assert!(wizard.retry_path().is_none());
    assert!(time_range("2026-09-27T09:00:00-03:00", "2026-09-27T12:00:00Z").is_ok());
    assert!(time_range("2026-09-27T09:00:00", "2026-09-27T10:00:00Z").is_err());
    assert!(time_range("2026-09-27T09:00:00.0001Z", "2026-09-27T10:00:00Z").is_err());
    assert!(time_range("2026-09-28T09:00:00Z", "2026-09-27T10:00:00Z").is_err());
}
