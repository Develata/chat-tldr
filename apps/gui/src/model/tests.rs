use super::*;
use chat_tldr_core::{AckPayload, DonePayload};
use serde_json::json;

fn request(id: u64, kind: CommandKind, chat: Option<ChatId>) -> RequestTag {
    RequestTag { id, kind, chat }
}

fn completed(status: RunStatus, code: i32) -> Completion {
    Completion {
        exit_code: Some(code),
        done: Some(DonePayload {
            status,
            exit_code: code,
            finish_reason: None,
            elapsed_ms: 1,
        }),
        error: None,
        cancelled: false,
    }
}

fn emit(model: &mut GuiModel, tag: &RequestTag, body: EventBody) {
    model.apply(BridgeEvent {
        request: tag.clone(),
        payload: BridgePayload::Event(Box::new(CliEvent::new("r_gui".into(), 0, body))),
    });
}

fn end(model: &mut GuiModel, tag: RequestTag, completion: Completion) -> Option<FollowUp> {
    model.apply(BridgeEvent {
        request: tag,
        payload: BridgePayload::Finished(completion),
    })
}

fn handshake(model: &mut GuiModel, version: &str) -> Option<FollowUp> {
    let tag = request(1, CommandKind::Version, None);
    model.begin(tag.clone()).unwrap();
    emit(
        model,
        &tag,
        EventBody::Ack(AckPayload {
            command: "version".into(),
            target: None,
            changed: false,
            detail: json!({"cli_version":"0.1.0","schema_version":version,"capabilities":{"commands":["chats","inbox","analyze","feedback","resolve","mark-read"]}}),
        }),
    );
    end(model, tag, completed(RunStatus::Complete, 0))
}

#[test]
fn version_handshake_requires_supported_payload_and_successful_process_exit() {
    let mut model = GuiModel::default();
    assert_eq!(handshake(&mut model, "1.42"), Some(FollowUp::Chats));
    assert!(model.handshake_ok());
    assert!(model.has_capability("inbox"));
    assert!(!model.has_capability("unimplemented"));
    let mut incompatible = GuiModel::default();
    assert_eq!(handshake(&mut incompatible, "2.0"), None);
    assert!(!incompatible.handshake_ok());
    assert!(incompatible.last_error.is_some());
    let mut missing = GuiModel::default();
    let tag = request(1, CommandKind::Version, None);
    missing.begin(tag.clone()).unwrap();
    assert_eq!(
        end(&mut missing, tag, completed(RunStatus::Complete, 0)),
        None
    );
    assert!(!missing.handshake_ok());
}

#[test]
fn only_a_complete_successfully_displayed_current_inbox_can_mark_read() {
    let mut model = GuiModel::demo();
    assert!(model.last_error.is_none(), "{:?}", model.last_error);
    let view = model.inbox.as_ref().unwrap();
    assert!(!view.insights.is_empty());
    let id = view.request_id;
    let cursor = view.meta.view_cursor;
    assert!(model.mark_read_cursor().is_none());
    model.mark_inbox_displayed(id + 1);
    assert!(model.mark_read_cursor().is_none());
    model.mark_inbox_displayed(id);
    assert_eq!(model.mark_read_cursor(), cursor);
    let tag = request(3, CommandKind::Inbox, model.selected_chat.clone());
    model.begin(tag.clone()).unwrap();
    assert!(model.mark_read_cursor().is_none());
    let mut failed = completed(RunStatus::Complete, 0);
    failed.error = Some("stream truncated after data".into());
    end(&mut model, tag, failed);
    model.mark_inbox_displayed(id);
    assert!(model.mark_read_cursor().is_none());
}

#[test]
fn partial_cancelled_and_mutation_completions_refresh_without_promoting_provisional_data() {
    for (kind, completion) in [
        (CommandKind::Analyze, completed(RunStatus::Partial, 6)),
        (
            CommandKind::Analyze,
            Completion {
                cancelled: true,
                error: Some("forced termination".into()),
                ..completed(RunStatus::Complete, 0)
            },
        ),
        (CommandKind::Feedback, completed(RunStatus::Complete, 0)),
        (CommandKind::Resolve, completed(RunStatus::Failed, 4)),
        (CommandKind::MarkRead, completed(RunStatus::Complete, 0)),
    ] {
        let mut model = GuiModel::demo();
        model.capabilities = Some(Capabilities {
            commands: vec!["inbox".into()],
            ..Capabilities::default()
        });
        let chat = model.selected_chat.clone().unwrap();
        let original = model.inbox.as_ref().unwrap().request_id;
        let tag = request(3, kind, Some(chat.clone()));
        model.begin(tag.clone()).unwrap();
        assert_eq!(
            end(&mut model, tag, completion),
            Some(FollowUp::Inbox(chat))
        );
        assert_eq!(model.inbox.as_ref().unwrap().request_id, original);
        model.mark_inbox_displayed(original);
        assert!(model.mark_read_cursor().is_none());
    }
}

#[test]
fn mutations_refresh_chat_summary_before_reloading_the_selected_inbox() {
    let mut model = GuiModel::demo();
    model.capabilities = Some(Capabilities {
        commands: vec!["chats".into(), "inbox".into()],
        ..Capabilities::default()
    });
    let chat_id = model.selected_chat.clone().unwrap();
    let mutation = request(3, CommandKind::MarkRead, Some(chat_id.clone()));
    model.begin(mutation.clone()).unwrap();
    assert_eq!(
        end(&mut model, mutation, completed(RunStatus::Complete, 0)),
        Some(FollowUp::Chats)
    );
    let mut updated = model
        .chats
        .iter()
        .find(|chat| chat.chat_id == chat_id)
        .unwrap()
        .clone();
    updated.unreviewed_messages = 0;
    updated.open_p0 = 0;
    let chats = request(4, CommandKind::Chats, None);
    model.begin(chats.clone()).unwrap();
    emit(&mut model, &chats, EventBody::Chat(updated));
    assert_eq!(
        end(&mut model, chats, completed(RunStatus::Complete, 0)),
        Some(FollowUp::Inbox(chat_id))
    );
    assert_eq!(model.chats[0].unreviewed_messages, 0);
    assert_eq!(model.chats[0].open_p0, 0);
    assert!(model.mark_read_cursor().is_none());
}

#[test]
fn stale_tags_and_chat_switches_never_publish_old_results() {
    let mut model = GuiModel::demo();
    model.capabilities = Some(Capabilities {
        commands: vec!["inbox".into()],
        ..Capabilities::default()
    });
    let old_chat = model.selected_chat.clone().unwrap();
    let tag = request(3, CommandKind::Inbox, Some(old_chat));
    model.begin(tag.clone()).unwrap();
    let stale = request(2, CommandKind::Inbox, tag.chat.clone());
    assert_eq!(
        end(&mut model, stale, completed(RunStatus::Complete, 0)),
        None
    );
    assert_eq!(model.active.as_ref(), Some(&tag));
    let next: ChatId = "qq:group:other".into();
    model.select_chat(next.clone());
    assert_eq!(
        end(&mut model, tag, completed(RunStatus::Complete, 0)),
        Some(FollowUp::Inbox(next))
    );
    assert!(model.inbox.is_none());
    assert!(model.mark_read_cursor().is_none());
}

#[test]
fn logs_are_bounded_by_count_and_unicode_character_length() {
    let mut model = GuiModel::default();
    let tag = request(1, CommandKind::Analyze, None);
    model.begin(tag.clone()).unwrap();
    for _ in 0..LOG_LIMIT + 50 {
        model.apply(BridgeEvent {
            request: tag.clone(),
            payload: BridgePayload::Stderr("中😀".repeat(3000)),
        });
    }
    assert_eq!(model.logs.len(), LOG_LIMIT);
    assert!(model.logs.iter().all(|line| line.chars().count() <= 2049));
}

#[test]
fn empty_and_unknown_completion_never_enable_mark_read() {
    let mut model = GuiModel::default();
    let chat: ChatId = "qq:group:synthetic-course".into();
    model.select_chat(chat.clone());
    let tag = request(1, CommandKind::Inbox, Some(chat));
    model.begin(tag.clone()).unwrap();
    let first = include_str!("../../../../fixtures/jsonl/inbox-empty.jsonl")
        .lines()
        .next()
        .unwrap();
    let mut meta = match serde_json::from_str::<CliEvent>(first).unwrap().body {
        EventBody::Inbox(meta) => meta,
        _ => panic!("fixture needs inbox metadata"),
    };
    meta.chat_id = model.selected_chat.clone().unwrap();
    emit(&mut model, &tag, EventBody::Inbox(meta));
    end(&mut model, tag, completed(RunStatus::Complete, 0));
    assert!(model.inbox.is_some());
    model.mark_inbox_displayed(1);
    assert!(model.mark_read_cursor().is_none());
    let tag = request(2, CommandKind::Inbox, model.selected_chat.clone());
    model.begin(tag.clone()).unwrap();
    let old = model.inbox.as_ref().unwrap().meta.clone();
    emit(&mut model, &tag, EventBody::Inbox(old));
    end(&mut model, tag, completed(RunStatus::Unknown, 0));
    assert!(!model.last_completion.as_ref().unwrap().is_success());
    assert!(model.last_error.as_ref().unwrap().contains("未知"));
}
