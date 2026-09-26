//! Deterministic views over a single validated store snapshot. No SQL or model calls.
use crate::{
    extract,
    store::{InboxData, OverviewOptions},
};
use chat_tldr_core::*;
use chrono::{DateTime, FixedOffset};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Activity {
    raw: u64,
    meaningful: u64,
    senders: BTreeSet<PersonId>,
    seen: BTreeSet<(PersonId, i64, [u8; 32])>,
    buckets: BTreeMap<(PersonId, i64), u8>,
    score: f64,
    last: Option<DateTime<FixedOffset>>,
}

pub(crate) fn build(mut data: InboxData, chat: &ChatMeta, options: &OverviewOptions) -> Overview {
    let mut activity: BTreeMap<TopicId, Activity> = BTreeMap::new();
    let mut mention_reasons = BTreeMap::new();
    let mut mentions = Vec::new();
    let mut resources = Vec::new();
    let mut window_messages = 0;
    let mut window_pending = 0;
    let mut pending_messages = 0;
    let mut data_start = None;
    let mut data_end = None;
    let topics: BTreeSet<_> = data
        .snapshot
        .topics
        .iter()
        .filter(|topic| topic.state != TopicState::Merged)
        .map(|topic| topic.topic_id.clone())
        .collect();
    let mut ordered: Vec<_> = data.messages.iter().collect();
    ordered.sort_by_key(|(id, _)| data.positions[*id]);
    for (id, message) in ordered {
        data_start = Some(
            data_start.map_or(message.sent_at, |date: DateTime<FixedOffset>| {
                date.min(message.sent_at)
            }),
        );
        data_end = Some(
            data_end.map_or(message.sent_at, |date: DateTime<FixedOffset>| {
                date.max(message.sent_at)
            }),
        );
        let (state, topic) = &data.states[id];
        let analyzed = state == "done";
        let pending = matches!(state.as_str(), "pending" | "failed");
        pending_messages += u64::from(pending);
        if message.recalled || message.system {
            continue;
        }
        let reasons = relevance(message, chat);
        if !reasons.is_empty() {
            mention_reasons.insert(id, reasons.clone());
        }
        if message.sent_at < options.since || message.sent_at >= options.until {
            continue;
        }
        window_messages += 1;
        window_pending += u64::from(pending);
        let view = || OverviewMessage {
            message_id: id.clone(),
            topic_id: topic.clone(),
            sent_at: message.sent_at,
            sender_display: message.sender_display.clone(),
            text: crate::render::render(message),
            analyzed,
            reasons: reasons.clone(),
        };
        if !reasons.is_empty() {
            mentions.push(view());
        }
        let links = links(&message.text);
        if !message.attachments.is_empty() || !links.is_empty() {
            resources.push(ResourceItem {
                source: view(),
                attachments: message.attachments.clone(),
                links,
            });
        }
        let Some(topic) = topic.as_ref().filter(|topic| topics.contains(*topic)) else {
            continue;
        };
        if !analyzed {
            continue;
        }
        let stats = activity.entry(topic.clone()).or_default();
        stats.raw += 1;
        let Some(content) = meaningful_text(&message.text) else {
            continue;
        };
        stats.meaningful += 1;
        stats.senders.insert(message.sender.clone());
        stats.last = Some(
            stats
                .last
                .map_or(message.sent_at, |time| time.max(message.sent_at)),
        );
        // Per-sender, per-topic ten-minute buckets; both exact repetition and
        // differently worded flooding have bounded contribution. No body copies retained.
        let bucket = message.sent_at.timestamp().div_euclid(600);
        let fingerprint = *blake3::hash(content.as_bytes()).as_bytes();
        let count = stats
            .buckets
            .entry((message.sender.clone(), bucket))
            .or_default();
        if *count < 3
            && stats
                .seen
                .insert((message.sender.clone(), bucket, fingerprint))
        {
            *count += 1;
            let age_hours =
                (options.until - message.sent_at).num_milliseconds() as f64 / 3_600_000.0;
            stats.score += (-age_hours / 6.0).exp();
        }
    }
    let mut hot_topics: Vec<_> = activity
        .into_iter()
        .filter_map(|(topic_id, stats)| {
            Some(HotTopic {
                topic_id,
                message_count: stats.raw,
                meaningful_messages: stats.meaningful,
                participants: stats.senders.len() as u64,
                activity_score: stats.score + 2.0 * stats.senders.len() as f64,
                last_message_at: stats.last?,
            })
        })
        .collect();
    hot_topics.sort_by(|a, b| {
        b.activity_score
            .total_cmp(&a.activity_score)
            .then_with(|| b.last_message_at.cmp(&a.last_message_at))
            .then_with(|| a.topic_id.cmp(&b.topic_id))
    });
    // A time-bounded report must not leak an insight updated with future evidence.
    // These are current versions filtered by evidence, not historical version snapshots.
    data.snapshot.insights.retain(|item| {
        item.insight.evidence.iter().all(|e| {
            data.messages
                .get(&e.message_id)
                .is_some_and(|m| m.sent_at < options.until)
        })
    });
    let insights = &data.snapshot.insights;
    let mut related = Vec::new();
    let mut deadlines = Vec::new();
    for item in insights {
        let insight = &item.insight;
        let mut reasons = BTreeSet::new();
        match insight.assignee {
            Assignee::Me => {
                reasons.insert("分配给我".to_owned());
            }
            Assignee::All => {
                reasons.insert("分配给全体".to_owned());
            }
            _ => {}
        }
        for evidence in &insight.evidence {
            if let Some(mentions) = mention_reasons.get(&evidence.message_id) {
                reasons.extend(mentions.iter().cloned());
            }
        }
        if !reasons.is_empty() {
            related.push(RelatedInsight {
                insight_id: insight.id.clone(),
                reasons: reasons.into_iter().collect(),
            });
        }
        if let Some(deadline) = &insight.deadline {
            deadlines.push(DeadlineItem {
                insight_id: insight.id.clone(),
                status: deadline_status(deadline, options.until),
            });
        }
    }
    let by_id: BTreeMap<_, _> = insights
        .iter()
        .map(|item| (&item.insight.id, &item.insight))
        .collect();
    deadlines.sort_by(|a, b| {
        let left = by_id[&a.insight_id];
        let right = by_id[&b.insight_id];
        status_order(a.status)
            .cmp(&status_order(b.status))
            .then_with(|| {
                deadline_key(left.deadline.as_ref().unwrap())
                    .cmp(&deadline_key(right.deadline.as_ref().unwrap()))
            })
            .then_with(|| a.insight_id.cmp(&b.insight_id))
    });
    let priority_topics = group(insights.iter().map(|row| &row.insight));
    let unread_topics = group(
        insights
            .iter()
            .filter(|row| {
                row.insight.evidence.iter().any(|e| {
                    data.positions.get(&e.message_id).is_some_and(|cursor| {
                        data.snapshot
                            .meta
                            .last_reviewed
                            .is_none_or(|seen| *cursor > seen)
                    })
                })
            })
            .map(|row| &row.insight),
    );
    mentions.sort_by(|a, b| {
        b.sent_at
            .cmp(&a.sent_at)
            .then_with(|| a.message_id.cmp(&b.message_id))
    });
    resources.sort_by(|a, b| {
        b.source
            .sent_at
            .cmp(&a.source.sent_at)
            .then_with(|| a.source.message_id.cmp(&b.source.message_id))
    });
    Overview {
        version: 1,
        chat_id: chat.chat_id.clone(),
        since: options.since,
        until: options.until,
        generated_at: options.now,
        data_start,
        data_end,
        window_messages,
        pending_messages,
        window_pending,
        last_reviewed: data.snapshot.meta.last_reviewed,
        hot_topics,
        priority_topics,
        related,
        mentions,
        deadlines,
        unread_topics,
        resources,
        topics: data.snapshot.topics,
        insights: data.snapshot.insights,
    }
}

fn relevance(message: &UnifiedMessage, chat: &ChatMeta) -> Vec<String> {
    if !extract::mentions_me(message, chat) {
        return Vec::new();
    }
    let mut reasons = Vec::new();
    if message.mentions.iter().any(|mention| {
        matches!(&mention.target, MentionTarget::User { uid, uin }
        if chat.self_uid.as_ref().zip(uid.as_ref()).is_some_and(|(a,b)| a == b)
        || chat.self_uin.as_ref().zip(uin.as_ref()).is_some_and(|(a,b)| a == b)
        || chat.self_uin.as_ref().zip(uid.as_ref()).is_some_and(|(a,b)| a == b))
    }) {
        reasons.push("直接 @我".into());
    }
    if message
        .mentions
        .iter()
        .any(|mention| mention.target == MentionTarget::All)
    {
        reasons.push("@全体成员".into());
    }
    reasons
}

fn group<'a>(insights: impl Iterator<Item = &'a Insight>) -> Vec<TopicDigest> {
    let mut positions = BTreeMap::new();
    let mut groups: Vec<TopicDigest> = Vec::new();
    for item in insights {
        let index = *positions.entry(item.topic_id.clone()).or_insert_with(|| {
            groups.push(TopicDigest {
                topic_id: item.topic_id.clone(),
                priority: item.priority,
                insight_ids: Vec::new(),
                reasons: Vec::new(),
            });
            groups.len() - 1
        });
        let group = &mut groups[index];
        group.insight_ids.push(item.id.clone());
        for reason in [
            item.deadline.is_some().then_some("有截止事项"),
            (item.kind == InsightKind::Todo
                && matches!(item.assignee, Assignee::Me | Assignee::All))
            .then_some("需要我或全体处理"),
            (item.kind == InsightKind::MentionMe).then_some("有提及提醒"),
        ]
        .into_iter()
        .flatten()
        {
            if !group.reasons.iter().any(|r| r == reason) {
                group.reasons.push(reason.into());
            }
        }
    }
    groups
}

fn deadline_key(value: &TemporalConstraint) -> Option<DateTime<FixedOffset>> {
    value
        .bound_date?
        .and_time(
            value
                .bound_time
                .unwrap_or(chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap()),
        )
        .and_local_timezone(*value.anchor.offset())
        .single()
}

fn deadline_status(value: &TemporalConstraint, now: DateTime<FixedOffset>) -> DeadlineStatus {
    if value.normalized_by != NormalizedBy::Rule
        || !matches!(
            value.relation,
            TemporalRelation::Before | TemporalRelation::At
        )
    {
        return DeadlineStatus::Uncertain;
    }
    let Some(date) = value.bound_date else {
        return DeadlineStatus::Uncertain;
    };
    let past = if value.bound_time.is_some() {
        deadline_key(value).is_some_and(|bound| {
            if value.relation == TemporalRelation::Before {
                bound <= now
            } else {
                bound < now
            }
        })
    } else {
        date < now.with_timezone(value.anchor.offset()).date_naive()
    };
    if past {
        DeadlineStatus::Overdue
    } else {
        DeadlineStatus::Upcoming
    }
}

fn status_order(status: DeadlineStatus) -> u8 {
    match status {
        DeadlineStatus::Overdue => 0,
        DeadlineStatus::Upcoming => 1,
        _ => 2,
    }
}

fn meaningful_text(text: &str) -> Option<String> {
    let mut rest = text;
    let mut visible = String::with_capacity(text.len());
    while let Some(start) = rest.find('[') {
        visible.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find(']') else {
            visible.push_str(tail);
            rest = "";
            break;
        };
        let label = tail[1..end].split(':').next().unwrap_or_default();
        if !matches!(
            label,
            "图片"
                | "文件"
                | "语音"
                | "视频"
                | "合并转发"
                | "表情"
                | "系统"
                | "system"
                | "json"
                | "location"
        ) {
            visible.push_str(&tail[..=end]);
        }
        rest = &tail[end + 1..];
    }
    visible.push_str(rest);
    visible.retain(|c| !c.is_whitespace());
    visible
        .chars()
        .any(|c| c.is_alphanumeric())
        .then_some(visible)
}

fn links(text: &str) -> Vec<String> {
    let mut links = BTreeSet::new();
    let mut consumed = 0;
    for (start, _) in text.match_indices("http") {
        if start < consumed {
            continue;
        }
        let tail = &text[start..];
        if !tail.starts_with("https://") && !tail.starts_with("http://") {
            continue;
        }
        let end = tail
            .find(|c: char| c.is_whitespace() || "<>\"'，。；！？）】」』".contains(c))
            .unwrap_or(tail.len());
        consumed = start + end;
        let link = tail[..end].trim_end_matches(['.', ',', ';', ')', ']']);
        if link.len() > 8 {
            links.insert(link.to_owned());
        }
    }
    links.into_iter().collect()
}

#[cfg(test)]
#[path = "overview/tests.rs"]
mod tests;
