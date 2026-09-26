use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use chat_tldr_core::{
    ChatId, Cursor, EvidenceView, InboxPayload, Insight, InsightId, InsightKind, InsightPayload,
    Lifecycle, MessageId, Priority, PriorityCounts, TemporalConstraint, TopicId, TopicPayload,
    UnifiedMessage, VerificationStatus,
};
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{
    EngineError, Result, rank, render,
    verify::{self, DraftEvidence, VerifyInput},
};

use super::{
    analysis::{CurrentMessages, open_write, read_topics},
    open_read, parse_cursor, wire_string,
};

#[derive(Clone, Debug)]
pub struct InboxOptions {
    pub all: bool,
    pub include_resolved: bool,
    pub include_rejected: bool,
    pub now: DateTime<FixedOffset>,
}

#[derive(Clone, Debug)]
pub struct InboxSnapshot {
    pub meta: InboxPayload,
    pub topics: Vec<TopicPayload>,
    pub insights: Vec<InsightPayload>,
}

pub fn inbox(path: &Path, chat: &ChatId, options: &InboxOptions) -> Result<InboxSnapshot> {
    let Some(mut connection) = open_read(path)? else {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    };
    let tx = connection.transaction()?;
    let reviewed = last_reviewed(&tx, chat)?;
    type MessageRow = (i64, i64, String, String, Option<String>);
    let rows: Vec<MessageRow> = tx.prepare(
        "SELECT m.pk,m.sent_at_ms,m.analysis_state,m.body_json,t.topic_id FROM messages m \
         LEFT JOIN topic_messages t ON t.message_id=m.message_id WHERE m.chat_id=?1 ORDER BY m.sent_at_ms,m.pk"
    )?.query_map([chat.as_ref()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut positions = BTreeMap::new();
    let mut messages = BTreeMap::new();
    let mut topic_messages: BTreeMap<TopicId, Vec<MessageId>> = BTreeMap::new();
    let mut view_cursor = None;
    let mut safe_prefix = true;
    for (ordinal, sent_at_ms, state, body, topic) in rows {
        let cursor = Cursor {
            sent_at_ms,
            ordinal,
        };
        if safe_prefix && matches!(state.as_str(), "done" | "skipped") {
            view_cursor = Some(cursor);
        } else {
            safe_prefix = false;
        }
        let message: UnifiedMessage = serde_json::from_str(&body)?;
        if message.chat_id != *chat {
            return Err(EngineError::DatabaseFormat(
                "stored message belongs to another chat".into(),
            ));
        }
        if let Some(topic) = topic {
            topic_messages
                .entry(topic.into())
                .or_default()
                .push(message.id.clone());
        }
        positions.insert(message.id.clone(), cursor);
        messages.insert(message.id.clone(), message);
    }
    let lookup = CurrentMessages(messages);
    let raw: Vec<(String, f32)> = tx
        .prepare("SELECT body_json,rank_prior FROM insights WHERE chat_id=?1 ORDER BY insight_id")?
        .query_map([chat.as_ref()], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let weights = read_weights(&tx)?;
    let mut rejected_insights = 0;
    let mut insights = Vec::new();
    for (body, prior) in raw {
        let mut item: Insight = serde_json::from_str(&body)?;
        if item.chat_id != *chat {
            return Err(EngineError::DatabaseFormat(
                "stored insight belongs to another chat".into(),
            ));
        }
        let draft: Vec<_> = item
            .evidence
            .iter()
            .map(|evidence| DraftEvidence {
                message_id: evidence.message_id.clone(),
                quote: evidence.quote.clone(),
            })
            .collect();
        let profile = item
            .evidence
            .first()
            .map(|evidence| evidence.render_profile)
            .unwrap_or_default();
        let mut report = verify::verify(
            &VerifyInput {
                kind: item.kind,
                deadline_raw: item.deadline.as_ref().map(|deadline| deadline.raw.as_str()),
                evidence: &draft,
                profile,
            },
            &lookup,
        );
        for (index, evidence) in item.evidence.iter().enumerate() {
            if evidence.render_profile != profile {
                report.evidence_ok[index] = false;
                report.status = VerificationStatus::Rejected;
            }
        }
        item.verification_status = conservative_status(item.verification_status, report.status);
        if item.verification_status == VerificationStatus::Unverified
            && (item.kind != InsightKind::TopicSummary || item.deadline.is_some())
        {
            item.verification_status = VerificationStatus::Rejected;
        }
        if item.verification_status == VerificationStatus::Rejected {
            rejected_insights += 1;
            if !options.include_rejected {
                continue;
            }
        }
        if !options.include_resolved && item.lifecycle != Lifecycle::Open {
            continue;
        }
        let unseen_evidence = item.evidence.iter().any(|evidence| {
            positions
                .get(&evidence.message_id)
                .is_some_and(|position| reviewed.is_none_or(|seen| *position > seen))
        });
        let persistent = item.priority == Priority::P0 && item.lifecycle == Lifecycle::Open
            || item.priority == Priority::P1
                && item
                    .deadline
                    .as_ref()
                    .is_some_and(|deadline| not_expired(deadline, options.now));
        if !options.all && !unseen_evidence && !persistent {
            continue;
        }
        item.rank_score = score_from_weights(&tx, &item, prior, &weights)?;
        let evidence_view = item
            .evidence
            .iter()
            .zip(report.evidence_ok)
            .filter_map(|(evidence, ok)| {
                let message = lookup.0.get(&evidence.message_id)?;
                let display_text = render::render_with_profile(message, evidence.render_profile)
                    .unwrap_or_default();
                let highlight = if ok {
                    verify::find_quote(&display_text, &evidence.quote)
                        .map(|(start, end)| [start, end])
                } else {
                    None
                };
                Some(EvidenceView {
                    message_id: evidence.message_id.clone(),
                    sender_display: message.sender_display.clone(),
                    sent_at: message.sent_at,
                    display_text,
                    highlight,
                    ok,
                })
            })
            .collect();
        insights.push(InsightPayload {
            insight: item,
            evidence_view,
        });
    }
    insights.sort_by(|left, right| rank::compare_insights(&left.insight, &right.insight));
    let selected_topics: BTreeSet<_> = insights
        .iter()
        .filter_map(|item| item.insight.topic_id.clone())
        .collect();
    let merged: BTreeMap<String, Option<String>> = tx
        .prepare("SELECT topic_id,merged_into FROM topics WHERE chat_id=?1")?
        .query_map([chat.as_ref()], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let topics = read_topics(&tx, chat)?
        .into_iter()
        .filter(|topic| selected_topics.contains(&topic.id))
        .map(|topic| {
            let members: Vec<_> = topic_messages
                .get(&topic.id)
                .into_iter()
                .flatten()
                .filter_map(|id| lookup.0.get(id))
                .collect();
            TopicPayload {
                merged_into: merged
                    .get(topic.id.as_ref())
                    .cloned()
                    .flatten()
                    .map(Into::into),
                message_count: members.len() as u64,
                first_message_at: members
                    .iter()
                    .map(|message| message.sent_at)
                    .min()
                    .unwrap_or(topic.last_message_at),
                last_message_at: members
                    .iter()
                    .map(|message| message.sent_at)
                    .max()
                    .unwrap_or(topic.last_message_at),
                topic_id: topic.id,
                chat_id: topic.chat_id,
                title: topic.title,
                title_is_provisional: topic.provisional,
                state: topic.state,
                is_chitchat: topic.is_chitchat,
            }
        })
        .collect();
    let mut counts = PriorityCounts::default();
    for item in &insights {
        match item.insight.priority {
            Priority::P0 => counts.p0 += 1,
            Priority::P1 => counts.p1 += 1,
            Priority::P2 => counts.p2 += 1,
            Priority::P3 => counts.p3 += 1,
            Priority::Unknown => {}
        }
    }
    Ok(InboxSnapshot {
        meta: InboxPayload {
            chat_id: chat.clone(),
            view_cursor,
            last_reviewed: reviewed,
            counts,
            rejected_insights,
            generated_at: options.now,
        },
        topics,
        insights,
    })
}

fn conservative_status(
    stored: VerificationStatus,
    current: VerificationStatus,
) -> VerificationStatus {
    use VerificationStatus::*;
    match (stored, current) {
        (Rejected | Unknown, _) | (_, Rejected | Unknown) => Rejected,
        (Unverified, _) | (_, Unverified) => Unverified,
        (Verified, Verified) => Verified,
    }
}

fn not_expired(deadline: &TemporalConstraint, now: DateTime<FixedOffset>) -> bool {
    let Some(date) = deadline.bound_date else {
        return false;
    };
    if let Some(time) = deadline.bound_time {
        date.and_time(time)
            .and_local_timezone(*deadline.anchor.offset())
            .single()
            .is_some_and(|bound| bound >= now)
    } else {
        date >= now.with_timezone(deadline.anchor.offset()).date_naive()
    }
}

fn last_reviewed(connection: &Connection, chat: &ChatId) -> Result<Option<Cursor>> {
    let row: Option<Option<String>> = connection
        .query_row(
            "SELECT last_reviewed FROM chats WHERE chat_id=?1",
            [chat.as_ref()],
            |row| row.get(0),
        )
        .optional()?;
    parse_cursor(row.ok_or_else(|| EngineError::ChatNotFound(chat.to_string()))?)
}

fn safe_cursor(connection: &Connection, chat: &ChatId) -> Result<Option<Cursor>> {
    let mut statement = connection.prepare(
        "SELECT sent_at_ms,pk,analysis_state FROM messages WHERE chat_id=?1 ORDER BY sent_at_ms,pk",
    )?;
    let mut rows = statement.query([chat.as_ref()])?;
    let mut result = None;
    while let Some(row) = rows.next()? {
        let state: String = row.get(2)?;
        if !matches!(state.as_str(), "done" | "skipped") {
            break;
        }
        result = Some(Cursor {
            sent_at_ms: row.get(0)?,
            ordinal: row.get(1)?,
        });
    }
    Ok(result)
}

pub fn mark_read(path: &Path, chat: &ChatId, cursor: Cursor) -> Result<bool> {
    if !path.try_exists()? {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    }
    let mut connection = open_write(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let reviewed = last_reviewed(&tx, chat)?;
    let belongs: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE chat_id=?1 AND pk=?2 AND sent_at_ms=?3)",
        params![chat.as_ref(), cursor.ordinal, cursor.sent_at_ms],
        |row| row.get(0),
    )?;
    if !belongs {
        return Err(EngineError::CursorInvalid(
            "cursor does not belong to this chat".into(),
        ));
    }
    if reviewed.is_some_and(|prior| cursor <= prior) {
        return Ok(false);
    }
    if safe_cursor(&tx, chat)?.is_none_or(|safe| cursor > safe) {
        return Err(EngineError::CursorInvalid(
            "cursor crosses pending or failed messages".into(),
        ));
    }
    tx.execute(
        "UPDATE chats SET last_reviewed=?2,updated_at=?3 WHERE chat_id=?1",
        params![chat.as_ref(), cursor.to_string(), Utc::now().to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(true)
}

fn load_insight(connection: &Connection, id: &InsightId) -> Result<Insight> {
    let body: Option<String> = connection
        .query_row(
            "SELECT body_json FROM insights WHERE insight_id=?1",
            [id.as_ref()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&body.ok_or_else(|| {
        EngineError::InsightNotFound(id.to_string())
    })?)?)
}

pub fn resolve(path: &Path, id: &InsightId, lifecycle: Lifecycle) -> Result<bool> {
    if lifecycle == Lifecycle::Unknown {
        return Err(EngineError::Usage(
            "unknown lifecycle cannot be applied".into(),
        ));
    }
    if !path.try_exists()? {
        return Err(EngineError::InsightNotFound(id.to_string()));
    }
    let mut connection = open_write(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut item = load_insight(&tx, id)?;
    if item.lifecycle == lifecycle {
        return Ok(false);
    }
    item.lifecycle = lifecycle;
    item.updated_at = Utc::now().fixed_offset();
    tx.execute(
        "UPDATE insights SET lifecycle=?2,body_json=?3,updated_at=?4 WHERE insight_id=?1",
        params![
            id.as_ref(),
            wire_string(&lifecycle)?,
            serde_json::to_string(&item)?,
            item.updated_at.to_rfc3339()
        ],
    )?;
    tx.commit()?;
    Ok(true)
}

const DECAY_EVENTS_KEY: &str = "preference_decay_events_v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DecayEvent {
    at: DateTime<FixedOffset>,
    sequence: u64,
}

enum PreferenceEvent {
    Decay(DecayEvent),
    Vote {
        at: DateTime<FixedOffset>,
        id: String,
        positive: bool,
        features: Vec<String>,
    },
}

impl PreferenceEvent {
    fn at(&self) -> DateTime<FixedOffset> {
        match self {
            Self::Decay(event) => event.at,
            Self::Vote { at, .. } => *at,
        }
    }

    fn tie_key(&self) -> String {
        // When timestamps are identical, decay precedes votes; then use stable IDs.
        match self {
            Self::Decay(event) => format!("d:{:020}", event.sequence),
            Self::Vote { id, .. } => format!("v:{id}"),
        }
    }
}

fn decay_events(connection: &Connection) -> Result<Vec<DecayEvent>> {
    let body: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [DECAY_EVENTS_KEY],
            |row| row.get(0),
        )
        .optional()?;
    body.map(|body| serde_json::from_str(&body).map_err(Into::into))
        .transpose()
        .map(|events| events.unwrap_or_default())
}

fn features(connection: &Connection, item: &Insight) -> Result<Vec<String>> {
    let mut features = vec![
        format!("kind:{}", wire_string(&item.kind)?),
        format!("has_deadline:{}", item.deadline.is_some()),
    ];
    if let Some(topic) = &item.topic_id {
        features.push(format!("topic:{topic}"));
    }
    if let Some(evidence) = item.evidence.first() {
        let sender: Option<String> = connection
            .query_row(
                "SELECT sender FROM messages WHERE message_id=?1 AND chat_id=?2",
                params![evidence.message_id.as_ref(), item.chat_id.as_ref()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(sender) = sender {
            features.push(format!("sender:{sender}"));
        }
    }
    Ok(features)
}

pub(super) fn read_weights(connection: &Connection) -> Result<BTreeMap<String, f32>> {
    Ok(connection
        .prepare("SELECT feature,weight FROM preference_weights")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

pub(super) fn score_from_weights(
    connection: &Connection,
    item: &Insight,
    prior: f32,
    weights: &BTreeMap<String, f32>,
) -> Result<f32> {
    let weights: Vec<_> = features(connection, item)?
        .iter()
        .map(|feature| weights.get(feature).copied().unwrap_or(0.0))
        .collect();
    Ok(rank::apply_feedback(prior, &weights))
}

/// Apply already stored weights to a newly extracted/replaced insight before commit.
pub(super) fn personalized_score(
    connection: &Connection,
    item: &Insight,
    prior: f32,
) -> Result<f32> {
    score_from_weights(connection, item, prior, &read_weights(connection)?)
}

pub(super) fn rebuild_preferences(
    connection: &Connection,
    now: DateTime<FixedOffset>,
) -> Result<()> {
    let mut events: Vec<_> = decay_events(connection)?
        .into_iter()
        .map(PreferenceEvent::Decay)
        .collect();
    let votes: Vec<(String, String, String)> = connection
        .prepare("SELECT insight_id,label,created_at FROM feedback ORDER BY created_at,insight_id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (id, label, at) in votes {
        let positive = match label.as_str() {
            "useful" => true,
            "not_important" => false,
            _ => {
                return Err(EngineError::DatabaseFormat(
                    "unknown stored feedback label".into(),
                ));
            }
        };
        let item = load_insight(connection, &id.clone().into())?;
        let at = DateTime::parse_from_rfc3339(&at)
            .map_err(|_| EngineError::DatabaseFormat("invalid feedback timestamp".into()))?;
        events.push(PreferenceEvent::Vote {
            at,
            id,
            positive,
            features: features(connection, &item)?,
        });
    }
    events.sort_by_key(|event| (event.at(), event.tie_key()));
    let mut weights: BTreeMap<String, (f32, u64)> = BTreeMap::new();
    for event in events {
        match event {
            PreferenceEvent::Decay(_) => {
                for (weight, _) in weights.values_mut() {
                    *weight *= 0.95;
                }
            }
            PreferenceEvent::Vote {
                positive, features, ..
            } => {
                let direction = if positive { 1.0 } else { -1.0 };
                for feature in features {
                    let (weight, count) = weights.entry(feature).or_default();
                    *weight = (0.95 * *weight + 0.2 * direction).clamp(-0.5, 0.5);
                    *count += 1;
                }
            }
        }
    }
    connection.execute("DELETE FROM preference_weights", [])?;
    for (feature, (weight, count)) in &weights {
        let count = i64::try_from(*count).map_err(|_| {
            EngineError::DatabaseFormat("preference count exceeds SQLite integer range".into())
        })?;
        connection.execute(
            "INSERT INTO preference_weights(feature,weight,n,updated_at) VALUES(?1,?2,?3,?4)",
            params![feature, weight, count, now.to_rfc3339()],
        )?;
    }
    let weights: BTreeMap<_, _> = weights
        .into_iter()
        .map(|(feature, (weight, _))| (feature, weight))
        .collect();
    let items: Vec<(String, f32)> = connection
        .prepare("SELECT body_json,rank_prior FROM insights ORDER BY insight_id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (body, prior) in items {
        let mut item: Insight = serde_json::from_str(&body)?;
        let score = score_from_weights(connection, &item, prior, &weights)?;
        if score != item.rank_score {
            item.rank_score = score;
            connection.execute(
                "UPDATE insights SET body_json=?2 WHERE insight_id=?1",
                params![item.id.as_ref(), serde_json::to_string(&item)?],
            )?;
        }
    }
    Ok(())
}

/// Set the single current vote. Repeating the same setting performs no writes.
pub fn feedback(path: &Path, id: &InsightId, useful: bool) -> Result<bool> {
    if !path.try_exists()? {
        return Err(EngineError::InsightNotFound(id.to_string()));
    }
    let mut connection = open_write(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    load_insight(&tx, id)?;
    let label = if useful { "useful" } else { "not_important" };
    let previous: Option<String> = tx
        .query_row(
            "SELECT label FROM feedback WHERE insight_id=?1",
            [id.as_ref()],
            |row| row.get(0),
        )
        .optional()?;
    if previous.as_deref() == Some(label) {
        return Ok(false);
    }
    let now = Utc::now().fixed_offset();
    tx.execute("INSERT INTO feedback(insight_id,label,created_at) VALUES(?1,?2,?3) ON CONFLICT(insight_id) DO UPDATE SET label=excluded.label,created_at=excluded.created_at", params![id.as_ref(),label,now.to_rfc3339()])?;
    rebuild_preferences(&tx, now)?;
    tx.commit()?;
    Ok(true)
}

/// Append one durable decay event per real analysis start. Rebuilding after a
/// later vote change replays these events, so older remaining votes retain age.
pub fn decay_preferences(path: &Path) -> Result<()> {
    let mut connection = open_write(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut events = decay_events(&tx)?;
    let now = Utc::now().fixed_offset();
    let sequence = match events.last() {
        None => 0,
        Some(event) => event.sequence.checked_add(1).ok_or_else(|| {
            EngineError::DatabaseFormat("preference decay sequence overflow".into())
        })?,
    };
    events.push(DecayEvent { at: now, sequence });
    tx.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![DECAY_EVENTS_KEY,serde_json::to_string(&events)?])?;
    rebuild_preferences(&tx, now)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chat_tldr_core::{
        Assignee, Evidence, Granularity, ImportBatch, NormalizedBy, RenderProfile, TemporalRelation,
    };
    use chrono::Duration;

    use super::*;
    use crate::store::{ImportOptions, import_batches};

    struct Fixture {
        _temp: tempfile::TempDir,
        path: std::path::PathBuf,
        batch: ImportBatch,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("chat-tldr.db");
            let batch = chat_tldr_qce::parse_qce_json(
                include_bytes!("../../../../fixtures/qce/synthetic-group.json"),
                &chat_tldr_qce::QceOptions::default(),
            )
            .unwrap();
            import_batches(
                &path,
                std::slice::from_ref(&batch),
                &ImportOptions::default(),
            )
            .unwrap();
            let connection = Connection::open(&path).unwrap();
            connection
                .execute(
                    "UPDATE messages SET analysis_state='done' WHERE recalled=0",
                    [],
                )
                .unwrap();
            Self {
                _temp: temp,
                path,
                batch,
            }
        }

        fn chat(&self) -> &ChatId {
            &self.batch.chat.chat_id
        }

        fn connection(&self) -> Connection {
            Connection::open(&self.path).unwrap()
        }

        fn options(&self) -> InboxOptions {
            InboxOptions {
                all: false,
                include_resolved: false,
                include_rejected: false,
                now: self.batch.messages[0].sent_at,
            }
        }

        fn cursor(&self, index: usize) -> Cursor {
            self.connection()
                .query_row(
                    "SELECT sent_at_ms,pk FROM messages WHERE message_id=?1",
                    [self.batch.messages[index].id.as_ref()],
                    |row| {
                        Ok(Cursor {
                            sent_at_ms: row.get(0)?,
                            ordinal: row.get(1)?,
                        })
                    },
                )
                .unwrap()
        }

        fn seed(&self, id: &str, priority: Priority, message_index: usize) -> Insight {
            let message = &self.batch.messages[message_index];
            let item = Insight {
                id: id.into(),
                chat_id: self.chat().clone(),
                kind: InsightKind::Todo,
                title: "合成事项".into(),
                summary: "仅用于离线测试".into(),
                priority,
                rank_score: 1.0,
                confidence: Some(1.0),
                assignee: if priority == Priority::P0 {
                    Assignee::Me
                } else {
                    Assignee::Other
                },
                deadline: None,
                evidence: vec![Evidence {
                    message_id: message.id.clone(),
                    quote: render::render(message),
                    render_profile: RenderProfile::default(),
                }],
                topic_id: Some("t_synthetic".into()),
                verification_status: VerificationStatus::Verified,
                lifecycle: Lifecycle::Open,
                created_in_run: "r_synthetic".into(),
                updated_at: message.sent_at,
            };
            self.save(&item);
            item
        }

        fn save(&self, item: &Insight) {
            let connection = self.connection();
            connection.execute("INSERT INTO topics(topic_id,chat_id,title,title_is_provisional,state,last_message_at,created_in_run,updated_at) VALUES('t_synthetic',?1,'合成话题',0,'active',?2,'r_synthetic',?2) ON CONFLICT(topic_id) DO NOTHING", params![self.chat().as_ref(),item.updated_at.to_rfc3339()]).unwrap();
            connection.execute("INSERT INTO insights(insight_id,chat_id,topic_id,kind,priority,rank_prior,verification_status,lifecycle,body_json,created_in_run,updated_at) VALUES(?1,?2,?3,?4,?5,1.0,?6,?7,?8,'r_synthetic',?9) ON CONFLICT(insight_id) DO UPDATE SET kind=excluded.kind,priority=excluded.priority,verification_status=excluded.verification_status,lifecycle=excluded.lifecycle,body_json=excluded.body_json",
                params![item.id.as_ref(),item.chat_id.as_ref(),item.topic_id.as_ref().map(TopicId::as_ref),wire_string(&item.kind).unwrap(),wire_string(&item.priority).unwrap(),wire_string(&item.verification_status).unwrap(),wire_string(&item.lifecycle).unwrap(),serde_json::to_string(item).unwrap(),item.updated_at.to_rfc3339()]).unwrap();
            for evidence in &item.evidence {
                connection.execute("INSERT INTO topic_messages(topic_id,message_id,burst_id,method,run_id) VALUES('t_synthetic',?1,'b_synthetic','direct','r_synthetic') ON CONFLICT(message_id) DO NOTHING", [evidence.message_id.as_ref()]).unwrap();
            }
        }

        fn item(&self, id: &InsightId) -> Insight {
            load_insight(&self.connection(), id).unwrap()
        }
    }

    #[test]
    fn missing_store_queries_and_operations_do_not_create_a_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("absent.db");
        let options = InboxOptions {
            all: false,
            include_resolved: false,
            include_rejected: false,
            now: Utc::now().fixed_offset(),
        };
        assert!(matches!(
            inbox(&path, &"missing".into(), &options),
            Err(EngineError::ChatNotFound(_))
        ));
        assert!(matches!(
            mark_read(
                &path,
                &"missing".into(),
                Cursor {
                    sent_at_ms: 0,
                    ordinal: 1
                }
            ),
            Err(EngineError::ChatNotFound(_))
        ));
        assert!(matches!(
            resolve(&path, &"missing".into(), Lifecycle::Done),
            Err(EngineError::InsightNotFound(_))
        ));
        assert!(matches!(
            feedback(&path, &"missing".into(), true),
            Err(EngineError::InsightNotFound(_))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn safe_read_prefix_stops_before_failed_or_pending_messages() {
        let fixture = Fixture::new();
        let connection = fixture.connection();
        connection
            .execute(
                "UPDATE messages SET analysis_state='failed' WHERE message_id=?1",
                [fixture.batch.messages[1].id.as_ref()],
            )
            .unwrap();
        let snapshot = inbox(&fixture.path, fixture.chat(), &fixture.options()).unwrap();
        assert!(snapshot.insights.is_empty());
        assert_eq!(snapshot.meta.view_cursor, Some(fixture.cursor(0)));
        assert!(matches!(
            mark_read(&fixture.path, fixture.chat(), fixture.cursor(2)),
            Err(EngineError::CursorInvalid(_))
        ));
        assert!(mark_read(&fixture.path, fixture.chat(), fixture.cursor(0)).unwrap());
        assert!(!mark_read(&fixture.path, fixture.chat(), fixture.cursor(0)).unwrap());
        let forged = Cursor {
            sent_at_ms: fixture.cursor(0).sent_at_ms + 1,
            ..fixture.cursor(0)
        };
        assert!(matches!(
            mark_read(&fixture.path, fixture.chat(), forged),
            Err(EngineError::CursorInvalid(_))
        ));
        connection
            .execute(
                "UPDATE messages SET analysis_state='pending' WHERE message_id=?1",
                [fixture.batch.messages[0].id.as_ref()],
            )
            .unwrap();
        assert_eq!(
            inbox(&fixture.path, fixture.chat(), &fixture.options())
                .unwrap()
                .meta
                .view_cursor,
            None
        );
        // A formerly valid, already-reviewed cursor remains an idempotent no-op.
        assert!(!mark_read(&fixture.path, fixture.chat(), fixture.cursor(0)).unwrap());
        connection.execute("INSERT INTO chats(chat_id,kind,display_name,updated_at) VALUES('other','group','other','synthetic')", []).unwrap();
        assert!(matches!(
            mark_read(&fixture.path, &"other".into(), fixture.cursor(0)),
            Err(EngineError::CursorInvalid(_))
        ));
    }

    #[test]
    fn recalled_or_future_profile_evidence_is_rechecked_without_writing_the_store() {
        let fixture = Fixture::new();
        let item = fixture.seed("i_recall", Priority::P0, 0);
        assert_eq!(
            inbox(&fixture.path, fixture.chat(), &fixture.options())
                .unwrap()
                .insights
                .len(),
            1
        );
        let mut message = fixture.batch.messages[0].clone();
        message.recalled = true;
        message.text.clear();
        fixture.connection().execute("UPDATE messages SET recalled=1,text='',body_json=?2,analysis_state='skipped' WHERE message_id=?1", params![message.id.as_ref(),serde_json::to_string(&message).unwrap()]).unwrap();
        let snapshot = inbox(&fixture.path, fixture.chat(), &fixture.options()).unwrap();
        assert!(snapshot.insights.is_empty());
        assert_eq!(snapshot.meta.rejected_insights, 1);
        assert_eq!(
            fixture.item(&item.id).verification_status,
            VerificationStatus::Verified
        );
        let mut options = fixture.options();
        options.all = true;
        options.include_rejected = true;
        let rejected = inbox(&fixture.path, fixture.chat(), &options).unwrap();
        assert_eq!(
            rejected.insights[0].insight.verification_status,
            VerificationStatus::Rejected
        );
        assert!(!rejected.insights[0].evidence_view[0].ok);
        assert_eq!(rejected.insights[0].evidence_view[0].highlight, None);

        let mut mixed = fixture.seed("i_profile", Priority::P2, 1);
        let mut future = mixed.evidence[0].clone();
        future.render_profile.version = 2;
        mixed.evidence.push(future);
        fixture.save(&mixed);
        let snapshot = inbox(&fixture.path, fixture.chat(), &options).unwrap();
        let mixed = snapshot
            .insights
            .iter()
            .find(|value| value.insight.id == mixed.id)
            .unwrap();
        assert_eq!(
            mixed.insight.verification_status,
            VerificationStatus::Rejected
        );
        assert!(!mixed.evidence_view[1].ok);
    }

    #[test]
    fn all_resolved_and_rejected_filters_are_independent_and_counts_match_the_view() {
        let fixture = Fixture::new();
        let p0 = fixture.seed("i_p0", Priority::P0, 0);
        let p2 = fixture.seed("i_p2", Priority::P2, 0);
        let mut rejected = fixture.seed("i_rejected", Priority::P2, 0);
        rejected.verification_status = VerificationStatus::Rejected;
        fixture.save(&rejected);
        mark_read(&fixture.path, fixture.chat(), fixture.cursor(2)).unwrap();
        let snapshot = inbox(&fixture.path, fixture.chat(), &fixture.options()).unwrap();
        assert_eq!(
            snapshot
                .insights
                .iter()
                .map(|value| &value.insight.id)
                .collect::<Vec<_>>(),
            [&p0.id]
        );
        assert_eq!(snapshot.meta.counts.p0, 1);
        assert_eq!(snapshot.meta.counts.p2, 0);
        assert_eq!(snapshot.meta.rejected_insights, 1);
        assert_eq!(snapshot.topics[0].message_count, 1);
        let mut options = fixture.options();
        options.all = true;
        assert_eq!(
            inbox(&fixture.path, fixture.chat(), &options)
                .unwrap()
                .insights
                .len(),
            2
        );
        assert!(resolve(&fixture.path, &p0.id, Lifecycle::Done).unwrap());
        assert!(!resolve(&fixture.path, &p0.id, Lifecycle::Done).unwrap());
        let snapshot = inbox(&fixture.path, fixture.chat(), &options).unwrap();
        assert_eq!(snapshot.insights[0].insight.id, p2.id);
        options.include_resolved = true;
        options.include_rejected = true;
        let snapshot = inbox(&fixture.path, fixture.chat(), &options).unwrap();
        assert_eq!(snapshot.insights.len(), 3);
        assert_eq!(snapshot.meta.counts.p0, 1);
        assert_eq!(snapshot.meta.counts.p2, 2);
        assert_eq!(snapshot.meta.last_reviewed, Some(fixture.cursor(2)));
        assert!(resolve(&fixture.path, &rejected.id, Lifecycle::Done).unwrap());
        assert!(resolve(&fixture.path, &rejected.id, Lifecycle::Open).unwrap());
        assert_eq!(
            fixture.item(&rejected.id).verification_status,
            VerificationStatus::Rejected
        );
    }

    #[test]
    fn assigning_and_committing_backfill_preserve_the_latest_topic_activity() {
        use crate::store::analysis::{AnalysisSession, TopicRecord};
        use chat_tldr_core::{RunStatus, TopicState};

        let fixture = Fixture::new();
        let earlier = fixture
            .batch
            .messages
            .iter()
            .filter(|message| !message.recalled)
            .min_by_key(|message| message.sent_at)
            .unwrap();
        let later = fixture
            .batch
            .messages
            .iter()
            .filter(|message| !message.recalled)
            .max_by_key(|message| message.sent_at)
            .unwrap();
        assert!(earlier.sent_at < later.sent_at);
        let mut session = AnalysisSession::begin(
            &fixture.path,
            fixture.chat(),
            &"r_backfill_activity".into(),
            &serde_json::json!({}),
        )
        .unwrap();
        let mut topic = TopicRecord {
            id: "t_backfill_activity".into(),
            chat_id: fixture.chat().clone(),
            title: "合成回填话题".into(),
            provisional: false,
            state: TopicState::Active,
            last_message_at: later.sent_at,
            is_chitchat: None,
        };
        session
            .assign(&topic, std::slice::from_ref(&later.id), "direct")
            .unwrap();

        topic.last_message_at = earlier.sent_at;
        session
            .assign(&topic, std::slice::from_ref(&earlier.id), "rule_reply")
            .unwrap();
        let assigned = session.snapshot().unwrap();
        assert_eq!(assigned.topics[0].last_message_at, later.sent_at);

        session
            .commit_topic(&topic, std::slice::from_ref(&earlier.id), Vec::new())
            .unwrap();
        let committed = session.snapshot().unwrap();
        assert_eq!(committed.topics[0].last_message_at, later.sent_at);
        session.finish(RunStatus::Complete).unwrap();
    }

    #[test]
    fn a_p1_deadline_retains_its_window_only_while_it_is_not_expired() {
        let fixture = Fixture::new();
        let mut item = fixture.seed("i_old_p1", Priority::P1, 0);
        // The reader tolerates historical priorities; current ranking puts this in P0.
        item.deadline = Some(TemporalConstraint {
            raw: "周五前".into(),
            relation: TemporalRelation::Before,
            bound_date: Some(fixture.options().now.date_naive()),
            bound_time: None,
            granularity: Granularity::Day,
            anchor: fixture.options().now,
            normalized_by: NormalizedBy::Rule,
            confidence: 1.0,
        });
        fixture.save(&item);
        mark_read(&fixture.path, fixture.chat(), fixture.cursor(2)).unwrap();
        let mut options = fixture.options();
        assert_eq!(
            inbox(&fixture.path, fixture.chat(), &options)
                .unwrap()
                .insights
                .len(),
            1
        );
        options.now += Duration::days(1);
        assert!(
            inbox(&fixture.path, fixture.chat(), &options)
                .unwrap()
                .insights
                .is_empty()
        );
    }

    #[test]
    fn feedback_replaces_current_vote_without_changing_lifecycle_priority_or_review() {
        let fixture = Fixture::new();
        let item = fixture.seed("i_feedback", Priority::P0, 0);
        assert!(feedback(&fixture.path, &item.id, true).unwrap());
        let positive = fixture.item(&item.id);
        assert!((positive.rank_score - 1.2).abs() < 1e-6);
        assert!(!feedback(&fixture.path, &item.id, true).unwrap());
        assert_eq!(fixture.item(&item.id), positive);
        assert!(feedback(&fixture.path, &item.id, false).unwrap());
        assert!(!feedback(&fixture.path, &item.id, false).unwrap());
        let negative = fixture.item(&item.id);
        assert!((negative.rank_score - 0.8).abs() < 1e-6);
        assert_eq!(negative.priority, Priority::P0);
        assert_eq!(negative.lifecycle, Lifecycle::Open);
        assert_eq!(
            last_reviewed(&fixture.connection(), fixture.chat()).unwrap(),
            None
        );
        let votes: i64 = fixture
            .connection()
            .query_row("SELECT count(*) FROM feedback", [], |row| row.get(0))
            .unwrap();
        assert_eq!(votes, 1);
    }

    #[test]
    fn rebuilding_feedback_preserves_analysis_decay_for_other_effective_votes() {
        let fixture = Fixture::new();
        let first = fixture.seed("i_a", Priority::P0, 0);
        let second = fixture.seed("i_b", Priority::P0, 0);
        feedback(&fixture.path, &first.id, true).unwrap();
        decay_preferences(&fixture.path).unwrap();
        assert!((fixture.item(&first.id).rank_score - 1.19).abs() < 1e-6);
        feedback(&fixture.path, &second.id, false).unwrap();
        assert!((fixture.item(&first.id).rank_score - (1.0 + 0.95 * 0.19 - 0.2)).abs() < 1e-6);
        feedback(&fixture.path, &second.id, true).unwrap();
        let expected = 1.0 + 0.95 * 0.19 + 0.2;
        assert!((fixture.item(&first.id).rank_score - expected).abs() < 1e-6);
        assert!(
            (personalized_score(&fixture.connection(), &first, 1.0).unwrap() - expected).abs()
                < 1e-6
        );
        assert_eq!(decay_events(&fixture.connection()).unwrap().len(), 1);
        let count: i64 = fixture
            .connection()
            .query_row(
                "SELECT n FROM preference_weights WHERE feature='kind:todo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(fixture.item(&first.id).priority, Priority::P0);
    }
}
