use super::*;
use chat_tldr_core::{AckPayload, DecisionMethod, DonePayload, Priority, StatsPayload};
use serde_json::json;

#[test]
fn provider_settings_require_matching_ack_success_and_current_request() {
    let mut model = GuiModel::default();
    for (id, valid) in [(1, false), (2, true)] {
        let tag = request(id, CommandKind::ConfigShow, None);
        model.begin(tag.clone()).unwrap();
        emit(
            &mut model,
            &tag,
            EventBody::Ack(AckPayload {
                command: "config show".into(),
                changed: false,
                target: None,
                detail: serde_json::to_value(crate::ui::settings::synthetic_snapshot()).unwrap(),
            }),
        );
        assert!(model.settings.is_none());
        end(
            &mut model,
            tag,
            completed(
                if valid {
                    RunStatus::Complete
                } else {
                    RunStatus::Failed
                },
                if valid { 0 } else { 4 },
            ),
        );
        assert_eq!(model.settings.is_some(), valid);
    }
    let tag = request(3, CommandKind::ConfigSet, None);
    model.begin(tag.clone()).unwrap();
    emit(
        &mut model,
        &tag,
        EventBody::Ack(AckPayload {
            command: "config show".into(),
            changed: false,
            target: None,
            detail: serde_json::to_value(crate::ui::settings::synthetic_snapshot()).unwrap(),
        }),
    );
    end(&mut model, tag, completed(RunStatus::Complete, 0));
    assert!(!model.last_completion.as_ref().unwrap().is_success());
    assert_eq!(model.settings.as_ref().unwrap().0, 2);
}

fn overview_ack(chat: &ChatId) -> AckPayload {
    AckPayload {
        command: "overview".into(),
        target: Some(chat.to_string()),
        changed: false,
        detail: json!({"overview":{
            "version":1,"chat_id":chat,"since":"2026-09-24T00:00:00+08:00","until":"2026-09-25T00:00:00+08:00","generated_at":"2026-09-26T00:00:00+08:00",
            "data_start":null,"data_end":null,"window_messages":0,"pending_messages":0,"window_pending":0,"last_reviewed":null,
            "hot_topics":[],"priority_topics":[],"related":[],"mentions":[],"deadlines":[],"unread_topics":[],"resources":[],"topics":[],"insights":[]
        },"counts":{"hot_topics":0,"priority_topics":0,"related":0,"mentions":0,"deadlines":0,"unread_topics":0,"resources":0,"topics":0,"insights":0}}),
    }
}

#[test]
fn overview_publishes_only_after_success_and_never_marks_the_inbox_displayed() {
    let mut model = GuiModel::demo();
    let chat = model.selected_chat.clone().unwrap();
    let tag = request(3, CommandKind::Overview, Some(chat.clone()));
    model.begin(tag.clone()).unwrap();
    emit(&mut model, &tag, EventBody::Ack(overview_ack(&chat)));
    assert!(model.overview.is_none());
    end(&mut model, tag, completed(RunStatus::Complete, 0));
    assert!(model.overview.is_some());
    assert!(model.mark_read_cursor().is_none());
    let tag = request(4, CommandKind::Overview, Some(chat.clone()));
    model.begin(tag.clone()).unwrap();
    emit(&mut model, &tag, EventBody::Ack(overview_ack(&chat)));
    end(&mut model, tag, completed(RunStatus::Failed, 7));
    assert!(model.overview.is_none());
}

#[test]
fn overview_rows_reassemble_topic_and_insight_references_before_publication() {
    let mut model = GuiModel::demo();
    let chat = model.selected_chat.clone().unwrap();
    let inbox = model.inbox.clone().unwrap();
    let mut ack = overview_ack(&chat);
    let mut report: Overview = serde_json::from_value(ack.detail["overview"].clone()).unwrap();
    report.topics = inbox.topics;
    report.insights = inbox.insights;
    let first = &report.insights[0].insight;
    report.priority_topics.push(chat_tldr_core::TopicDigest {
        topic_id: first.topic_id.clone(),
        priority: first.priority,
        insight_ids: vec![first.id.clone()],
        reasons: vec!["有截止事项".into()],
    });
    let counts = report.counts();
    let rows = report.drain_parts();
    ack.detail = json!({"overview":report,"counts":counts});
    let tag = request(3, CommandKind::Overview, Some(chat.clone()));
    model.begin(tag.clone()).unwrap();
    emit(&mut model, &tag, EventBody::Ack(ack));
    for row in rows {
        emit(
            &mut model,
            &tag,
            EventBody::Ack(AckPayload {
                command: "overview.rows".into(),
                target: Some(chat.to_string()),
                changed: false,
                detail: serde_json::to_value(row).unwrap(),
            }),
        );
    }
    assert!(model.overview.is_none());
    end(&mut model, tag, completed(RunStatus::Complete, 0));
    assert_eq!(model.overview.as_ref().unwrap().report.counts(), counts);
    assert!(model.mark_read_cursor().is_none());
}

#[test]
fn overview_missing_extra_or_dangling_rows_cannot_publish() {
    for case in 0..3 {
        let mut model = GuiModel::demo();
        let chat = model.selected_chat.clone().unwrap();
        let mut ack = overview_ack(&chat);
        let tag = request(3, CommandKind::Overview, Some(chat.clone()));
        model.begin(tag.clone()).unwrap();
        if case != 1 {
            ack.detail["counts"]["related"] = json!(1);
        }
        emit(&mut model, &tag, EventBody::Ack(ack));
        if case != 0 {
            emit(
                &mut model,
                &tag,
                EventBody::Ack(AckPayload {
                    command: "overview.rows".into(),
                    target: Some(chat.to_string()),
                    changed: false,
                    detail: json!({"section":"related","row":{"insight_id":"missing","reasons":[]}}),
                }),
            );
        }
        end(&mut model, tag, completed(RunStatus::Complete, 0));
        assert!(model.overview.is_none());
        assert!(model.last_error.is_some());
    }
}

#[test]
fn overview_rejects_wrong_versions_chats_and_duplicate_payloads() {
    for case in 0..3 {
        let mut model = GuiModel::demo();
        let chat = model.selected_chat.clone().unwrap();
        let mut ack = overview_ack(&chat);
        let tag = request(3, CommandKind::Overview, Some(chat));
        model.begin(tag.clone()).unwrap();
        if case == 0 {
            ack.detail["overview"]["version"] = json!(2);
        }
        if case == 1 {
            ack.detail["overview"]["chat_id"] = json!("another-chat");
        }
        emit(&mut model, &tag, EventBody::Ack(ack.clone()));
        if case == 2 {
            emit(&mut model, &tag, EventBody::Ack(ack));
        }
        end(&mut model, tag, completed(RunStatus::Complete, 0));
        assert!(model.overview.is_none());
        assert!(model.last_error.is_some());
    }
}

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
    assert_eq!(
        model
            .chats
            .iter()
            .find(|chat| chat.chat_id == view.meta.chat_id)
            .unwrap()
            .open_p0,
        view.meta.counts.p0
    );
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
fn detailed_partial_error_survives_refresh_until_explicit_retry() {
    let mut model = GuiModel::demo();
    let tag = request(3, CommandKind::Analyze, model.selected_chat.clone());
    model.begin(tag.clone()).unwrap();
    emit(
        &mut model,
        &tag,
        EventBody::Error(chat_tldr_core::ErrorPayload {
            stage: "extract".into(),
            code: "E_PROVIDER_TIMEOUT".into(),
            retryable: true,
            message: "合成服务超时".into(),
            topic_id: None,
        }),
    );
    end(&mut model, tag, completed(RunStatus::Partial, 6));
    let error = model.last_error.as_deref().unwrap();
    assert!(error.contains("E_PROVIDER_TIMEOUT"));
    assert!(error.contains("部分完成"));

    let refresh = request(4, CommandKind::Chats, None);
    model.begin(refresh.clone()).unwrap();
    assert!(
        model
            .last_error
            .as_deref()
            .unwrap()
            .contains("E_PROVIDER_TIMEOUT")
    );
    end(&mut model, refresh, completed(RunStatus::Complete, 0));
    assert!(
        model
            .last_error
            .as_deref()
            .unwrap()
            .contains("E_PROVIDER_TIMEOUT")
    );
    model
        .begin(request(
            5,
            CommandKind::Analyze,
            model.selected_chat.clone(),
        ))
        .unwrap();
    assert!(model.last_error.is_none());
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

#[test]
fn failed_chat_refresh_revokes_the_old_displayed_cursor() {
    let mut model = GuiModel::demo();
    let id = model.inbox.as_ref().unwrap().request_id;
    model.mark_inbox_displayed(id);
    assert!(model.mark_read_cursor().is_some());
    let tag = request(3, CommandKind::Chats, None);
    model.begin(tag.clone()).unwrap();
    assert!(model.mark_read_cursor().is_none());
    end(&mut model, tag, completed(RunStatus::Failed, 4));
    model.mark_inbox_displayed(id);
    assert!(model.mark_read_cursor().is_none());
}

#[test]
fn demo_keeps_fixture_request_sequence_and_has_consistent_four_priority_counts() {
    let model = GuiModel::demo();
    assert_eq!(model.latest_request, Some(2));
    let inbox = model.inbox.as_ref().unwrap();
    let actual = [Priority::P0, Priority::P1, Priority::P2, Priority::P3].map(|priority| {
        inbox
            .insights
            .iter()
            .filter(|row| row.insight.priority == priority)
            .count() as u64
    });
    assert_eq!(actual, [1, 1, 1, 1]);
    assert_eq!(
        actual,
        [
            inbox.meta.counts.p0,
            inbox.meta.counts.p1,
            inbox.meta.counts.p2,
            inbox.meta.counts.p3,
        ]
    );
    let chat = model
        .chats
        .iter()
        .find(|chat| chat.chat_id == inbox.meta.chat_id)
        .unwrap();
    assert_eq!(chat.open_p0, inbox.meta.counts.p0);
    assert_eq!(
        chat.unreviewed_messages,
        inbox
            .topics
            .iter()
            .map(|topic| topic.message_count)
            .sum::<u64>()
    );
}

#[test]
fn demo_covers_all_decision_methods_and_structured_mock_usage() {
    let model = GuiModel::demo();
    assert_eq!(
        model
            .decisions
            .iter()
            .map(|row| row.method)
            .collect::<Vec<_>>(),
        vec![
            DecisionMethod::Rule,
            DecisionMethod::Jev,
            DecisionMethod::Fallback,
        ]
    );
    let stats = model.stats.as_ref().unwrap();
    assert_eq!(stats.usage.len(), 2);
    assert!(stats.usage.iter().all(|row| row.provider == "mock"));
    assert!(
        stats
            .usage
            .iter()
            .all(|row| row.stage.contains("synthetic"))
    );
    assert!(matches!(
        model.history_stats.front(),
        Some(StatsPayload::Run(run)) if run == stats
    ));
}

#[test]
fn demo_overview_fills_six_tabs_and_keeps_every_reference_resolvable() {
    let model = GuiModel::demo();
    let view = model.overview.as_ref().unwrap();
    let report = &view.report;
    assert!(!report.hot_topics.is_empty());
    for topic in &report.hot_topics {
        assert!(topic.participants <= topic.meaningful_messages);
        assert!(topic.meaningful_messages <= topic.message_count);
    }
    assert!(!report.priority_topics.is_empty());
    assert!(!report.related.is_empty() || !report.mentions.is_empty());
    assert!(!report.deadlines.is_empty());
    assert!(!report.unread_topics.is_empty());
    assert!(!report.resources.is_empty());
    assert!(overview_consistent(report));
    assert_eq!(view.titles.len(), report.topics.len());
    assert_eq!(view.items.len(), report.insights.len());
}

#[test]
fn demo_evidence_highlights_use_valid_unicode_scalar_boundaries() {
    let model = GuiModel::demo();
    let inbox = model.inbox.as_ref().unwrap();
    let mut touches_both_boundaries = false;
    for row in &inbox.insights {
        for evidence in &row.evidence_view {
            let source = row
                .insight
                .evidence
                .iter()
                .find(|source| source.message_id == evidence.message_id)
                .expect("every evidence view references typed evidence");
            let [start, end] = evidence.highlight.expect("demo evidence is highlighted");
            let scalars = evidence.display_text.chars().count();
            assert!(start < end && end <= scalars);
            assert_eq!(
                evidence
                    .display_text
                    .chars()
                    .skip(start)
                    .take(end - start)
                    .collect::<String>(),
                source.quote
            );
            touches_both_boundaries |= start == 0 && end == scalars;
        }
    }
    assert!(touches_both_boundaries);
}

#[test]
fn failed_analysis_notice_survives_automatic_refresh() {
    let mut model = GuiModel::demo();
    model.capabilities = Some(Capabilities {
        commands: vec!["chats".into(), "inbox".into()],
        ..Default::default()
    });
    let chat = model.selected_chat.clone().unwrap();
    let tag = request(100, CommandKind::Analyze, Some(chat));
    model.begin(tag.clone()).unwrap();
    let mut result = completed(RunStatus::Partial, 6);
    result.error = Some("synthetic provider failure".into());
    assert_eq!(end(&mut model, tag, result), Some(FollowUp::Chats));
    assert!(model.last_error.is_some());
    model.begin(request(101, CommandKind::Chats, None)).unwrap();
    assert!(
        model.last_error.is_some(),
        "automatic refresh erased original analysis failure before painting"
    );
}

#[test]
fn demo_deadlines_respect_frozen_priority_policy() {
    let model = GuiModel::demo();
    for row in &model.inbox.as_ref().unwrap().insights {
        let item = &row.insight;
        if item.deadline.is_some() && item.kind != chat_tldr_core::InsightKind::TopicSummary {
            assert_eq!(item.priority, Priority::P0, "deadline item {}", item.id);
        }
    }
}

#[test]
fn switching_chats_discards_the_previous_chats_queried_stats() {
    let mut model = GuiModel::demo();
    assert!(!model.history_stats.is_empty());
    assert!(model.select_chat("qq:group:another-synthetic-chat".into()));
    assert!(model.history_stats.is_empty());
    assert!(model.stats.is_none());
}
