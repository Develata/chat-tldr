use super::*;
use crate::store::InboxSnapshot;
use chrono::{Duration, NaiveDate, NaiveTime};

fn time(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).unwrap()
}
fn options() -> OverviewOptions {
    OverviewOptions {
        since: time("2026-09-24T09:00:00+08:00"),
        until: time("2026-09-24T11:00:00+08:00"),
        now: time("2026-09-26T20:00:00+08:00"),
    }
}

fn fixture() -> (InboxData, ChatMeta) {
    let batch = chat_tldr_qce::parse_qce_json(
        include_bytes!("../../../../fixtures/qce/scenario-analysis.json"),
        &chat_tldr_qce::QceOptions::default(),
    )
    .unwrap();
    let data = InboxData {
        snapshot: InboxSnapshot {
            meta: InboxPayload {
                chat_id: batch.chat.chat_id.clone(),
                view_cursor: None,
                last_reviewed: None,
                counts: PriorityCounts::default(),
                rejected_insights: 0,
                generated_at: options().now,
            },
            topics: Vec::new(),
            insights: Vec::new(),
        },
        positions: batch
            .messages
            .iter()
            .enumerate()
            .map(|(n, m)| {
                (
                    m.id.clone(),
                    Cursor {
                        sent_at_ms: m.sent_at.timestamp_millis(),
                        ordinal: n as i64 + 1,
                    },
                )
            })
            .collect(),
        states: batch
            .messages
            .iter()
            .map(|m| (m.id.clone(), ("pending".into(), None)))
            .collect(),
        messages: batch
            .messages
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect(),
    };
    (data, batch.chat)
}
fn id(data: &InboxData, n: u64) -> MessageId {
    let source = (9_200_000_000_000_000_000 + n).to_string();
    data.messages
        .values()
        .find(|m| m.source.qce_id.as_ref() == Some(&source))
        .unwrap()
        .id
        .clone()
}
fn assign(data: &mut InboxData, name: &str, numbers: &[u64], state: TopicState) {
    let ids: Vec<_> = numbers.iter().map(|n| id(data, *n)).collect();
    for id in &ids {
        data.states
            .insert(id.clone(), ("done".into(), Some(name.into())));
    }
    let first = ids
        .iter()
        .map(|id| data.messages[id].sent_at)
        .min()
        .unwrap();
    let last = ids
        .iter()
        .map(|id| data.messages[id].sent_at)
        .max()
        .unwrap();
    data.snapshot.topics.push(TopicPayload {
        topic_id: name.into(),
        chat_id: data.snapshot.meta.chat_id.clone(),
        title: name.into(),
        title_is_provisional: false,
        state,
        message_count: ids.len() as u64,
        first_message_at: first,
        last_message_at: last,
        is_chitchat: None,
        merged_into: None,
    });
}
fn item(
    data: &mut InboxData,
    source: u64,
    topic: &str,
    priority: Priority,
    assignee: Assignee,
) -> InsightId {
    let id = id(data, source);
    let message = &data.messages[&id];
    let item_id = InsightId(format!("i_{source}"));
    data.snapshot.insights.push(InsightPayload {
        insight: Insight {
            id: item_id.clone(),
            chat_id: message.chat_id.clone(),
            kind: InsightKind::Todo,
            title: "合成事项".into(),
            summary: String::new(),
            priority,
            rank_score: 1.0,
            confidence: Some(1.0),
            assignee,
            deadline: None,
            evidence: vec![Evidence {
                message_id: id,
                quote: message.text.clone(),
                render_profile: RenderProfile::default(),
            }],
            topic_id: Some(topic.into()),
            verification_status: VerificationStatus::Verified,
            lifecycle: Lifecycle::Open,
            created_in_run: "r_test".into(),
            updated_at: options().now,
        },
        evidence_view: Vec::new(),
    });
    item_id
}

#[test]
fn popular_casual_chat_is_separate_from_a_quiet_high_priority_topic() {
    let (mut data, chat) = fixture();
    assign(
        &mut data,
        "hot",
        &(21..=40).collect::<Vec<_>>(),
        TopicState::Closed,
    );
    assign(&mut data, "urgent", &[6], TopicState::Closed);
    let task = item(&mut data, 6, "urgent", Priority::P0, Assignee::Other);
    let result = build(data, &chat, &options());
    assert_eq!(result.hot_topics[0].topic_id.as_ref(), "hot");
    assert!(result.hot_topics[0].participants > 1);
    assert_eq!(result.hot_topics[0].meaningful_messages, 20);
    assert_eq!(
        result.priority_topics[0]
            .topic_id
            .as_ref()
            .unwrap()
            .as_ref(),
        "urgent"
    );
    assert_eq!(result.priority_topics[0].insight_ids, [task]);
    assert!(result.related.is_empty());
}

#[test]
fn topic_assignment_alone_recalled_system_and_placeholders_do_not_create_heat() {
    let (mut data, chat) = fixture();
    assign(&mut data, "excluded", &[1, 2, 3, 64], TopicState::Active);
    data.states.get_mut(&id(&data, 1)).unwrap().0 = "pending".into();
    data.messages.get_mut(&id(&data, 2)).unwrap().recalled = true;
    data.messages.get_mut(&id(&data, 3)).unwrap().system = true;
    let opts = OverviewOptions {
        until: time("2026-09-27T00:00:00+08:00"),
        ..options()
    };
    let result = build(data, &chat, &opts);
    assert!(result.hot_topics.is_empty());
    assert!(result.window_pending > 0);
}

#[test]
fn flooding_is_capped_and_same_name_different_people_remain_distinct() {
    let (mut data, chat) = fixture();
    assign(
        &mut data,
        "spam",
        &(21..=40).collect::<Vec<_>>(),
        TopicState::Active,
    );
    let template = data.messages[&id(&data, 21)].clone();
    for n in 21..=40 {
        let id = id(&data, n);
        let row = data.messages.get_mut(&id).unwrap();
        row.sent_at = template.sent_at;
        row.sender = template.sender.clone();
        row.text = format!("不同的刷屏内容 {n}");
    }
    assign(&mut data, "people", &[8, 9], TopicState::Active);
    let me = data.messages[&id(&data, 1)].mentions[0].target.clone();
    let other = data.messages[&id(&data, 9)].mentions[0].target.clone();
    assert_ne!(me, other);
    for (n, sender) in [(8, "qq:a"), (9, "qq:b")] {
        let id = id(&data, n);
        let row = data.messages.get_mut(&id).unwrap();
        row.sender = sender.into();
        row.sender_display = "同名".into();
    }
    let result = build(data, &chat, &options());
    let spam = result
        .hot_topics
        .iter()
        .find(|t| t.topic_id.as_ref() == "spam")
        .unwrap();
    assert_eq!(spam.meaningful_messages, 20);
    assert_eq!(spam.participants, 1);
    assert!(spam.activity_score <= 5.0);
    assert_eq!(
        result
            .hot_topics
            .iter()
            .find(|t| t.topic_id.as_ref() == "people")
            .unwrap()
            .participants,
        2
    );
}

#[test]
fn exact_duplicates_contribute_once_and_time_window_is_half_open() {
    let (mut data, chat) = fixture();
    assign(&mut data, "topic", &[1, 2, 3, 4], TopicState::Active);
    let first = data.messages[&id(&data, 1)].clone();
    for n in [2, 3, 4] {
        let key = id(&data, n);
        let row = data.messages.get_mut(&key).unwrap();
        row.text = first.text.clone();
        row.sender = first.sender.clone();
    }
    let opts = OverviewOptions {
        until: first.sent_at + Duration::minutes(3),
        ..options()
    };
    let result = build(data, &chat, &opts);
    assert_eq!(result.window_messages, 3);
    assert_eq!(result.hot_topics[0].meaningful_messages, 3);
    assert!(result.hot_topics[0].activity_score <= 3.0);
}

#[test]
fn related_insight_recovers_covered_mentions_and_unread_uses_evidence_cursor() {
    let (mut data, chat) = fixture();
    assign(&mut data, "topic", &[1, 3, 4, 5, 9], TopicState::Active);
    let direct = item(&mut data, 1, "topic", Priority::P0, Assignee::Other);
    let all = item(&mut data, 3, "topic", Priority::P1, Assignee::Other);
    item(&mut data, 4, "topic", Priority::P2, Assignee::Other);
    item(&mut data, 5, "topic", Priority::P2, Assignee::Other);
    item(&mut data, 9, "topic", Priority::P2, Assignee::Other);
    data.snapshot.meta.last_reviewed = Some(data.positions[&id(&data, 4)]);
    let result = build(data, &chat, &options());
    assert_eq!(result.related.len(), 2);
    assert_eq!(result.related[0].insight_id, direct);
    assert!(result.related[0].reasons.iter().any(|r| r == "直接 @我"));
    assert_eq!(result.related[1].insight_id, all);
    assert!(result.related[1].reasons.iter().any(|r| r == "@全体成员"));
    assert_eq!(result.unread_topics[0].insight_ids.len(), 2);
    assert_eq!(result.priority_topics[0].insight_ids.len(), 5);
}

#[test]
fn old_priority_remains_visible_but_future_evidence_is_excluded() {
    let (mut data, chat) = fixture();
    assign(&mut data, "old", &[6], TopicState::Closed);
    assign(&mut data, "future", &[100], TopicState::Active);
    let old = item(&mut data, 6, "old", Priority::P0, Assignee::Other);
    item(&mut data, 100, "future", Priority::P0, Assignee::Other);
    let opts = OverviewOptions {
        since: time("2026-09-25T00:00:00+08:00"),
        until: time("2026-09-26T00:00:00+08:00"),
        ..options()
    };
    let result = build(data, &chat, &opts);
    assert!(result.hot_topics.is_empty());
    assert_eq!(result.insights.len(), 1);
    assert_eq!(result.insights[0].insight.id, old);
}

#[test]
fn resources_are_metadata_and_source_text_without_reading_attachments() {
    let (data, chat) = fixture();
    let opts = OverviewOptions {
        until: time("2026-09-27T00:00:00+08:00"),
        ..options()
    };
    let report = build(data, &chat, &opts);
    assert!(
        report
            .resources
            .iter()
            .any(|r| r.attachments.iter().any(|a| a.kind == AttachmentKind::File))
    );
    assert!(report.resources.iter().all(|r| !r.source.analyzed));
    assert_eq!(
        links(
            "资料：https://example.invalid/a，下一句 javascript:bad http://example.invalid/b?x=1&y=2"
        ),
        [
            "http://example.invalid/b?x=1&y=2",
            "https://example.invalid/a"
        ]
    );
    assert!(meaningful_text("[图片][文件:作业]").is_none());
    assert!(meaningful_text("[图片] 请检查第二项").is_some());
    assert_eq!(meaningful_text("[重要通知]"), Some("[重要通知]".into()));
    assert!(meaningful_text("[没有闭合的讨论").is_some());
}

#[test]
fn deadline_status_respects_precision_anchor_offset_and_uncertainty() {
    let mut d = TemporalConstraint {
        raw: "今天18:00前".into(),
        relation: TemporalRelation::Before,
        bound_date: NaiveDate::from_ymd_opt(2026, 9, 24),
        bound_time: NaiveTime::from_hms_opt(18, 0, 0),
        granularity: Granularity::Minute,
        anchor: options().since,
        normalized_by: NormalizedBy::Rule,
        confidence: 1.0,
    };
    assert_eq!(
        deadline_status(&d, time("2026-09-24T09:59:59Z")),
        DeadlineStatus::Upcoming
    );
    assert_eq!(
        deadline_status(&d, time("2026-09-24T10:00:00Z")),
        DeadlineStatus::Overdue
    );
    d.bound_time = None;
    assert_eq!(
        deadline_status(&d, time("2026-09-24T15:59:59Z")),
        DeadlineStatus::Upcoming
    );
    assert_eq!(
        deadline_status(&d, time("2026-09-24T16:00:00Z")),
        DeadlineStatus::Overdue
    );
    d.normalized_by = NormalizedBy::Llm;
    assert_eq!(
        deadline_status(&d, options().now),
        DeadlineStatus::Uncertain
    );
    d.normalized_by = NormalizedBy::Rule;
    d.relation = TemporalRelation::After;
    assert_eq!(
        deadline_status(&d, options().now),
        DeadlineStatus::Uncertain
    );
}
