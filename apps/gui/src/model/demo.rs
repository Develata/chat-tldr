use std::collections::{BTreeMap, VecDeque};

use chat_tldr_core::{
    AgentAction, AgentObservation, Assignee, Attachment, AttachmentKind, ChatId, DeadlineItem,
    DeadlineStatus, DecisionMethod, DecisionPayload, Evidence, EvidenceView, FinishReason,
    Granularity, HotTopic, Insight, InsightKind, InsightPayload, InsightStats, Lifecycle,
    MessageRange, NormalizedBy, Overview, OverviewMessage, Priority, PriorityCounts,
    RelatedInsight, RenderProfile, ResourceItem, RunId, RunStats, StatsPayload, TemporalConstraint,
    TemporalRelation, TopicDigest, TopicPayload, TopicState, UsageStats, VerificationStatus,
};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};

use super::{GuiModel, OverviewView, overview_consistent};

const DEMO_RUN: &str = "r_synthetic_20260927_080500";

/// Add a complete, deterministic screenshot scenario after the checked-in CLI
/// fixtures have exercised the normal stream parser and reducer.
pub(super) fn complete(model: &mut GuiModel) -> Result<(), String> {
    let (chat_id, view_cursor, topics, insights, counts, unreviewed_messages) = {
        let inbox = model
            .inbox
            .as_mut()
            .ok_or_else(|| "合成演示收件箱夹具加载失败".to_owned())?;
        let chat_id = inbox.meta.chat_id.clone();

        label_fixture_rows(inbox.topics.as_mut_slice(), inbox.insights.as_mut_slice())?;
        inbox.topics.extend(additional_topics(&chat_id));
        inbox.insights.extend(additional_insights(&chat_id));
        inbox.meta.counts = priority_counts(&inbox.insights);
        inbox.meta.generated_at = time("2026-09-27T08:05:00+08:00");

        (
            chat_id,
            inbox.meta.view_cursor,
            inbox.topics.clone(),
            inbox.insights.clone(),
            inbox.meta.counts.clone(),
            inbox.topics.iter().map(|topic| topic.message_count).sum(),
        )
    };

    let chat = model
        .chats
        .iter_mut()
        .find(|chat| chat.chat_id == chat_id)
        .ok_or_else(|| "合成演示会话夹具加载失败".to_owned())?;
    chat.display_name = "合成演示群（仅用于截图）".into();
    chat.unreviewed_messages = unreviewed_messages;
    chat.open_p0 = counts.p0;

    model.decisions = demo_decisions(&chat_id, view_cursor, &topics);
    let stats = demo_stats(&chat_id);
    model.stats = Some(stats.clone());
    model.history_stats = VecDeque::from([StatsPayload::Run(stats)]);

    let report = demo_overview(&chat_id, &topics, &insights);
    if !overview_consistent(&report) {
        return Err("合成演示总览引用不完整".into());
    }
    model.overview = Some(OverviewView::new(report));
    model
        .logs
        .push_back("【合成演示】以下卡片、决策、统计和总览均为固定测试数据。".into());
    Ok(())
}

fn label_fixture_rows(
    topics: &mut [TopicPayload],
    insights: &mut [InsightPayload],
) -> Result<(), String> {
    let topic = topics
        .first_mut()
        .ok_or_else(|| "合成演示话题夹具为空".to_owned())?;
    topic.title = "【合成示例】课程报告提交".into();

    let row = insights
        .first_mut()
        .ok_or_else(|| "合成演示结论夹具为空".to_owned())?;
    let text = "【合成示例】张三请在9月27日23:59之前提交课程报告。";
    let quote = "9月27日23:59之前";
    row.insight.title = "【合成示例】提交课程报告".into();
    row.insight.summary = "合成演示：张三需要在固定截止时间前提交课程报告。".into();
    row.insight.created_in_run = DEMO_RUN.into();
    row.insight.deadline = Some(TemporalConstraint {
        raw: quote.into(),
        relation: TemporalRelation::Before,
        bound_date: NaiveDate::from_ymd_opt(2026, 9, 27),
        bound_time: NaiveTime::from_hms_opt(23, 59, 0),
        granularity: Granularity::Minute,
        anchor: time("2026-09-26T10:00:00+08:00"),
        normalized_by: NormalizedBy::Rule,
        confidence: 1.0,
    });
    row.insight
        .evidence
        .first_mut()
        .ok_or_else(|| "合成演示结论夹具缺少证据".to_owned())?
        .quote = quote.into();
    let evidence = row
        .evidence_view
        .first_mut()
        .ok_or_else(|| "合成演示结论夹具缺少证据视图".to_owned())?;
    evidence.sender_display = "合成课代表".into();
    evidence.display_text = text.into();
    evidence.highlight = scalar_range(text, quote);
    Ok(())
}

fn additional_topics(chat_id: &ChatId) -> Vec<TopicPayload> {
    [
        (
            "t_synthetic_rehearsal",
            "【合成示例】功能彩排",
            3,
            "2026-09-26T11:20:00+08:00",
            "2026-09-26T11:32:00+08:00",
        ),
        (
            "t_synthetic_layout",
            "【合成示例】窄窗口排版",
            2,
            "2026-09-26T12:08:00+08:00",
            "2026-09-26T12:10:00+08:00",
        ),
        (
            "t_synthetic_resources",
            "【合成示例】茶歇闲聊",
            1,
            "2026-09-26T13:20:00+08:00",
            "2026-09-26T13:20:00+08:00",
        ),
    ]
    .into_iter()
    .map(|(id, title, message_count, first, last)| TopicPayload {
        topic_id: id.into(),
        chat_id: chat_id.clone(),
        title: title.into(),
        title_is_provisional: false,
        state: TopicState::Active,
        message_count,
        first_message_at: time(first),
        last_message_at: time(last),
        is_chitchat: Some(if id == "t_synthetic_resources" {
            0.95
        } else {
            0.02
        }),
        merged_into: None,
    })
    .collect()
}

fn additional_insights(chat_id: &ChatId) -> Vec<InsightPayload> {
    vec![
        insight(
            chat_id,
            "i_synthetic_p1",
            "t_synthetic_rehearsal",
            "m_synthetic_rehearsal",
            InsightKind::Announcement,
            Priority::P1,
            Assignee::All,
            "【合成示例】功能彩排已完成",
            "合成演示：面向全体的进展通知，无待办和截止日期。",
            "【合成示例】@全体成员 功能彩排已完成，演示材料已经整理好。",
            "功能彩排已完成",
            "合成项目负责人",
            "2026-09-26T11:32:00+08:00",
            None,
        ),
        insight(
            chat_id,
            "i_synthetic_p2",
            "t_synthetic_layout",
            "m_synthetic_layout",
            InsightKind::Todo,
            Priority::P2,
            Assignee::Other,
            "【合成示例】确认窄窗口排版",
            "合成演示：由设计同学确认排版，未指定截止日期。",
            "【合成示例】设计同学负责确认窄窗口里的中文排版 🙂",
            "【合成示例】设计同学负责确认窄窗口里的中文排版 🙂",
            "合成设计同学",
            "2026-09-26T12:10:00+08:00",
            None,
        ),
        insight(
            chat_id,
            "i_synthetic_p3",
            "t_synthetic_resources",
            "m_synthetic_resources",
            InsightKind::TopicSummary,
            Priority::P3,
            Assignee::Unknown,
            "【合成示例】茶歇闲聊",
            "合成演示：大家在茶歇分享趣味读物，没有行动要求。",
            "【合成示例】茶歇趣味读物：https://example.invalid/synthetic-ui.pdf",
            "https://example.invalid/synthetic-ui.pdf",
            "合成资料机器人",
            "2026-09-26T13:20:00+08:00",
            None,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn insight(
    chat_id: &ChatId,
    insight_id: &str,
    topic_id: &str,
    message_id: &str,
    kind: InsightKind,
    priority: Priority,
    assignee: Assignee,
    title: &str,
    summary: &str,
    text: &str,
    quote: &str,
    sender: &str,
    sent_at: &str,
    deadline: Option<TemporalConstraint>,
) -> InsightPayload {
    InsightPayload {
        insight: Insight {
            id: insight_id.into(),
            chat_id: chat_id.clone(),
            kind,
            title: title.into(),
            summary: summary.into(),
            priority,
            rank_score: match priority {
                Priority::P1 => 1.8,
                Priority::P2 => 1.1,
                _ => 0.4,
            },
            confidence: (kind != InsightKind::TopicSummary).then_some(0.94),
            assignee,
            deadline,
            evidence: vec![Evidence {
                message_id: message_id.into(),
                quote: quote.into(),
                render_profile: RenderProfile::default(),
            }],
            topic_id: Some(topic_id.into()),
            verification_status: VerificationStatus::Verified,
            lifecycle: Lifecycle::Open,
            created_in_run: RunId(DEMO_RUN.into()),
            updated_at: time("2026-09-27T08:05:00+08:00"),
        },
        evidence_view: vec![EvidenceView {
            message_id: message_id.into(),
            sender_display: sender.into(),
            sent_at: time(sent_at),
            display_text: text.into(),
            highlight: scalar_range(text, quote),
            ok: true,
        }],
    }
}

fn priority_counts(insights: &[InsightPayload]) -> PriorityCounts {
    let mut counts = PriorityCounts::default();
    for row in insights {
        match row.insight.priority {
            Priority::P0 => counts.p0 += 1,
            Priority::P1 => counts.p1 += 1,
            Priority::P2 => counts.p2 += 1,
            Priority::P3 => counts.p3 += 1,
            Priority::Unknown => {}
        }
    }
    counts
}

fn demo_decisions(
    chat_id: &ChatId,
    up_to: Option<chat_tldr_core::Cursor>,
    topics: &[TopicPayload],
) -> VecDeque<DecisionPayload> {
    let range = MessageRange {
        after: None,
        up_to: up_to.expect("non-empty synthetic inbox has a cursor"),
    };
    let first_topic = topics
        .first()
        .expect("synthetic fixture has a topic")
        .topic_id
        .clone();
    let segment = AgentAction::Segment {
        chat_id: chat_id.clone(),
        range: range.clone(),
    };
    let analyze = AgentAction::AnalyzeTopic {
        topic_id: first_topic,
    };
    let finish = AgentAction::Finish {
        reason: FinishReason::Done,
    };
    VecDeque::from([
        DecisionPayload {
            step: 1,
            observation: observation(8, 0, 0),
            allowed: vec![
                segment.clone(),
                AgentAction::AnalyzeDirect {
                    chat_id: chat_id.clone(),
                    range,
                },
            ],
            chosen: segment,
            method: DecisionMethod::Rule,
            probabilities: None,
            confidence: Some(1.0),
            reason: "【合成演示·规则】消息交错度较高，先按话题分段。".into(),
        },
        DecisionPayload {
            step: 2,
            observation: observation(0, 4, 0),
            allowed: vec![analyze.clone(), finish.clone()],
            chosen: analyze,
            method: DecisionMethod::Jev,
            probabilities: Some(BTreeMap::from([
                ("analyze_topic".into(), 0.91),
                ("finish".into(), 0.09),
            ])),
            confidence: Some(0.91),
            reason: "【合成演示·JEV】优先分析含固定截止时间的话题。".into(),
        },
        DecisionPayload {
            step: 3,
            observation: observation(0, 0, 0),
            allowed: vec![finish.clone()],
            chosen: finish,
            method: DecisionMethod::Fallback,
            probabilities: None,
            confidence: None,
            reason: "【合成演示·回退】没有剩余候选动作，安全结束。".into(),
        },
    ])
}

fn observation(pending_messages: u32, active_topics: u32, steps_taken: u32) -> AgentObservation {
    AgentObservation {
        pending_messages,
        interleave: 0.72,
        active_topics,
        dirty_topics: active_topics,
        pending_verification: 0,
        merge_candidates: 0,
        steps_taken,
        cost_usd: 0.0,
    }
}

fn demo_stats(chat_id: &ChatId) -> RunStats {
    RunStats {
        run_id: DEMO_RUN.into(),
        chat_id: chat_id.clone(),
        messages_analyzed: 8,
        topics_created: 4,
        topics_updated: 0,
        insights: InsightStats {
            created: 4,
            updated: 0,
            verified: 4,
            unverified: 0,
            rejected: 0,
        },
        usage: vec![
            UsageStats {
                stage: "synthetic-segment".into(),
                provider: "mock".into(),
                model: "synthetic-segmenter".into(),
                calls: 1,
                cache_hits: 0,
                input_tokens: 640,
                output_tokens: 128,
                cost_usd: 0.0012,
            },
            UsageStats {
                stage: "synthetic-insight".into(),
                provider: "mock".into(),
                model: "synthetic-insight-extractor".into(),
                calls: 2,
                cache_hits: 1,
                input_tokens: 1_280,
                output_tokens: 320,
                cost_usd: 0.0028,
            },
        ],
        cost_usd: 0.004,
        elapsed_ms: 1_842,
    }
}

fn demo_overview(
    chat_id: &ChatId,
    topics: &[TopicPayload],
    insights: &[InsightPayload],
) -> Overview {
    let topic = |index: usize| topics[index].topic_id.clone();
    let insight = |priority| {
        insights
            .iter()
            .find(|row| row.insight.priority == priority)
            .expect("synthetic demo has every priority")
            .insight
            .id
            .clone()
    };
    Overview {
        version: 1,
        chat_id: chat_id.clone(),
        since: time("2026-09-26T08:00:00+08:00"),
        until: time("2026-09-27T08:00:00+08:00"),
        generated_at: time("2026-09-27T08:05:00+08:00"),
        data_start: Some(time("2026-09-26T10:00:00+08:00")),
        data_end: Some(time("2026-09-26T13:20:00+08:00")),
        window_messages: topics.iter().map(|row| row.message_count).sum(),
        pending_messages: 2,
        window_pending: 1,
        last_reviewed: None,
        hot_topics: vec![HotTopic {
            topic_id: topic(1),
            message_count: 3,
            meaningful_messages: 3,
            participants: 4,
            activity_score: 7.6,
            last_message_at: time("2026-09-26T11:32:00+08:00"),
        }],
        priority_topics: vec![
            TopicDigest {
                topic_id: Some(topic(0)),
                priority: Priority::P0,
                insight_ids: vec![insight(Priority::P0)],
                reasons: vec!["【合成演示】固定截止时间临近".into()],
            },
            TopicDigest {
                topic_id: Some(topic(1)),
                priority: Priority::P1,
                insight_ids: vec![insight(Priority::P1)],
                reasons: vec!["【合成演示】面向全体的进展通知".into()],
            },
        ],
        related: vec![RelatedInsight {
            insight_id: insight(Priority::P1),
            reasons: vec!["【合成演示】通知面向全体成员".into()],
        }],
        mentions: vec![OverviewMessage {
            message_id: "m_synthetic_rehearsal".into(),
            topic_id: Some(topic(1)),
            sent_at: time("2026-09-26T11:32:00+08:00"),
            sender_display: "合成项目负责人".into(),
            text: "【合成示例】@全体成员 功能彩排已完成，演示材料已经整理好。".into(),
            analyzed: true,
            reasons: vec!["【合成演示】原文中 @全体成员".into()],
        }],
        deadlines: vec![DeadlineItem {
            insight_id: insight(Priority::P0),
            status: DeadlineStatus::Upcoming,
        }],
        unread_topics: vec![
            TopicDigest {
                topic_id: Some(topic(2)),
                priority: Priority::P2,
                insight_ids: vec![insight(Priority::P2)],
                reasons: vec!["【合成演示】与界面验收有关".into()],
            },
            TopicDigest {
                topic_id: Some(topic(3)),
                priority: Priority::P3,
                insight_ids: vec![insight(Priority::P3)],
                reasons: vec!["【合成演示】包含资料入口".into()],
            },
        ],
        resources: vec![ResourceItem {
            source: OverviewMessage {
                message_id: "m_synthetic_resources".into(),
                topic_id: Some(topic(3)),
                sent_at: time("2026-09-26T13:20:00+08:00"),
                sender_display: "合成资料机器人".into(),
                text: "【合成示例】茶歇趣味读物：https://example.invalid/synthetic-ui.pdf".into(),
                analyzed: true,
                reasons: vec!["【合成演示】检测到链接和附件元数据".into()],
            },
            attachments: vec![Attachment {
                kind: AttachmentKind::File,
                name: Some("synthetic-ui-checklist.pdf".into()),
                size: Some(24_576),
            }],
            links: vec!["https://example.invalid/synthetic-ui.pdf".into()],
        }],
        topics: topics.to_vec(),
        insights: insights.to_vec(),
    }
}

fn scalar_range(text: &str, quote: &str) -> Option<[usize; 2]> {
    let start_byte = text.find(quote)?;
    let end_byte = start_byte + quote.len();
    Some([
        text[..start_byte].chars().count(),
        text[..end_byte].chars().count(),
    ])
}

fn time(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).expect("synthetic demo timestamps are valid")
}
