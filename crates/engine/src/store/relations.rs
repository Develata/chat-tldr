use super::*;
use crate::relations::{identity, valid};
use serde::Serialize;

#[derive(Serialize)]
pub struct RelationsReport {
    pub chat_id: ChatId,
    pub since: Option<DateTime<FixedOffset>>,
    pub until: Option<DateTime<FixedOffset>>,
    pub messages: u64,
    pub analyzed_messages: u64,
    pub uncovered_messages: u64,
    pub relations: Vec<SemanticRelation>,
    pub questions: Vec<QuestionState>,
}

fn sources<'a>(
    connection: &Connection,
    chat: &ChatId,
    ids: impl Iterator<Item = &'a MessageId>,
) -> Result<BTreeMap<MessageId, (UnifiedMessage, Cursor)>> {
    let mut statement = connection.prepare(
        "SELECT body_json,sent_at_ms,pk FROM messages WHERE message_id=?1 AND chat_id=?2",
    )?;
    let mut result = BTreeMap::new();
    for id in ids.collect::<BTreeSet<_>>() {
        let row: Option<(String, i64, i64)> = statement
            .query_row(params![id.as_ref(), chat.as_ref()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        if let Some((body, sent_at_ms, ordinal)) = row {
            result.insert(
                id.clone(),
                (
                    serde_json::from_str(&body)?,
                    Cursor {
                        sent_at_ms,
                        ordinal,
                    },
                ),
            );
        }
    }
    Ok(result)
}

pub(super) fn commit(
    connection: &Connection,
    chat: &ChatId,
    topic: &TopicId,
    run: &RunId,
    messages: &[MessageId],
    rows: &[SemanticRelation],
) -> Result<()> {
    let current = sources(
        connection,
        chat,
        rows.iter().flat_map(|r| {
            std::iter::once(&r.source.message_id).chain(r.target.iter().map(|e| &e.message_id))
        }),
    )?;
    for row in rows {
        if row.chat_id != *chat
            || row.topic_id != *topic
            || row.id != identity(row)
            || !current.contains_key(&row.source.message_id)
            || row
                .target
                .as_ref()
                .is_some_and(|e| !current.contains_key(&e.message_id))
        {
            return Err(EngineError::Input(
                "invalid relation identity or source chat".into(),
            ));
        }
        let mut row = row.clone();
        row.verification_status = if valid(&row, &current) {
            VerificationStatus::Verified
        } else {
            VerificationStatus::Rejected
        };
        connection.execute("INSERT INTO semantic_relations(relation_id,chat_id,topic_id,source_id,target_id,body_json) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(relation_id) DO NOTHING",
            params![row.id,chat.as_ref(),topic.as_ref(),row.source.message_id.as_ref(),row.target.as_ref().map(|e| e.message_id.as_ref()),serde_json::to_string(&row)?])?;
    }
    for id in messages {
        connection.execute("INSERT INTO relation_coverage(message_id,run_id) VALUES(?1,?2) ON CONFLICT(message_id) DO UPDATE SET run_id=excluded.run_id", params![id.as_ref(),run.as_ref()])?;
    }
    Ok(())
}

pub fn relations(
    path: &Path,
    chat: &ChatId,
    since: Option<DateTime<FixedOffset>>,
    until: Option<DateTime<FixedOffset>>,
) -> Result<RelationsReport> {
    if since.zip(until).is_some_and(|(a, b)| a > b) {
        return Err(EngineError::Usage(
            "--since must not be after --until".into(),
        ));
    }
    let Some(mut connection) = open_read(path)? else {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    };
    let tx = connection.transaction()?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM chats WHERE chat_id=?1)",
        [chat.as_ref()],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    }
    let messages: u64 = tx.query_row("SELECT count(*) FROM messages WHERE chat_id=?1 AND recalled=0 AND (?2 IS NULL OR sent_at_ms>=?2) AND (?3 IS NULL OR sent_at_ms<?3)",
        params![chat.as_ref(),since.map(ceil_millis),until.map(ceil_millis)], |r| r.get::<_,i64>(0).map(|n| n as u64))?;
    let mut result = RelationsReport {
        chat_id: chat.clone(),
        since,
        until,
        messages,
        analyzed_messages: 0,
        uncovered_messages: messages,
        relations: Vec::new(),
        questions: Vec::new(),
    };
    if check_version(&tx)? < 2 {
        return Ok(result);
    }
    result.analyzed_messages = tx.query_row("SELECT count(*) FROM messages m JOIN relation_coverage c ON c.message_id=m.message_id WHERE m.chat_id=?1 AND m.recalled=0 AND (?2 IS NULL OR m.sent_at_ms>=?2) AND (?3 IS NULL OR m.sent_at_ms<?3)",
        params![chat.as_ref(),since.map(ceil_millis),until.map(ceil_millis)], |r| r.get::<_,i64>(0).map(|n| n as u64))?;
    result.uncovered_messages -= result.analyzed_messages;
    // Read answers up to the observation boundary, regardless of import order.
    // A question before `since` is omitted, but an in-window correction can still
    // refer to its original out-of-window arrangement.
    let raw: Vec<String> = tx.prepare("SELECT r.body_json FROM semantic_relations r JOIN messages s ON s.message_id=r.source_id LEFT JOIN messages t ON t.message_id=r.target_id WHERE r.chat_id=?1 AND (?2 IS NULL OR (s.sent_at_ms<?2 AND (r.target_id IS NULL OR t.sent_at_ms<?2))) ORDER BY coalesce(t.sent_at_ms,s.sent_at_ms),coalesce(t.pk,s.pk),r.relation_id")?
        .query_map(params![chat.as_ref(),until.map(ceil_millis)], |r| r.get(0))?.collect::<std::result::Result<_,_>>()?;
    let mut rows: Vec<SemanticRelation> = raw
        .iter()
        .map(|s| serde_json::from_str(s))
        .collect::<std::result::Result<_, _>>()?;
    let current = sources(
        &tx,
        chat,
        rows.iter().flat_map(|r| {
            std::iter::once(&r.source.message_id).chain(r.target.iter().map(|e| &e.message_id))
        }),
    )?;
    for row in &mut rows {
        row.verification_status = if valid(row, &current) {
            VerificationStatus::Verified
        } else {
            VerificationStatus::Rejected
        };
    }
    let mut answers = BTreeMap::<(MessageId, String), Vec<&SemanticRelation>>::new();
    for row in &rows {
        if row.kind == RelationKind::Answers
            && row.verification_status == VerificationStatus::Verified
        {
            answers
                .entry((row.source.message_id.clone(), row.source.quote.clone()))
                .or_default()
                .push(row);
        }
    }
    for row in &rows {
        if row.kind == RelationKind::Question
            && row.verification_status == VerificationStatus::Verified
            && since.is_none_or(|t| current[&row.source.message_id].0.sent_at >= t)
        {
            let related = answers.get(&(row.source.message_id.clone(), row.source.quote.clone()));
            let status = match related {
                Some(rows)
                    if rows
                        .iter()
                        .any(|r| r.answer_completeness == Some(AnswerCompleteness::Full)) =>
                {
                    QuestionStatus::Answered
                }
                Some(_) => QuestionStatus::PartiallyAnswered,
                None => QuestionStatus::Pending,
            };
            result.questions.push(QuestionState {
                question: row.clone(),
                status,
                answer_ids: related
                    .into_iter()
                    .flatten()
                    .map(|r| r.id.clone())
                    .collect(),
            });
        }
    }
    result.relations = rows
        .into_iter()
        .filter(|r| {
            let end = r.target.as_ref().unwrap_or(&r.source);
            since.is_none_or(|t| {
                current
                    .get(&end.message_id)
                    .is_some_and(|(m, _)| m.sent_at >= t)
            })
        })
        .collect();
    Ok(result)
}

/// Preserve relation evidence and IDs while moving topic ownership atomically.
pub(super) fn merge(connection: &Connection, from: &TopicId, into: &TopicId) -> Result<()> {
    let raw: Vec<String> = connection
        .prepare("SELECT body_json FROM semantic_relations WHERE topic_id=?1")?
        .query_map([from.as_ref()], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for body in raw {
        let mut row: SemanticRelation = serde_json::from_str(&body)?;
        row.topic_id = into.clone();
        connection.execute(
            "UPDATE semantic_relations SET topic_id=?2,body_json=?3 WHERE relation_id=?1",
            params![row.id, into.as_ref(), serde_json::to_string(&row)?],
        )?;
    }
    Ok(())
}

impl AnalysisSession {
    /// Old unresolved questions remain available beyond the short context tail.
    pub(crate) fn pending_questions(&self, topic: &TopicId) -> Result<Vec<Evidence>> {
        let connection = open_read(&self.path)?
            .ok_or_else(|| EngineError::ChatNotFound(self.chat_id.to_string()))?;
        let raw: Vec<String> = connection
            .prepare("SELECT body_json FROM semantic_relations WHERE topic_id=?1")?
            .query_map([topic.as_ref()], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        let rows: Vec<SemanticRelation> = raw
            .iter()
            .map(|s| serde_json::from_str(s))
            .collect::<std::result::Result<_, _>>()?;
        let current = sources(
            &connection,
            &self.chat_id,
            rows.iter().flat_map(|r| {
                std::iter::once(&r.source.message_id).chain(r.target.iter().map(|e| &e.message_id))
            }),
        )?;
        let answered: BTreeSet<_> = rows
            .iter()
            .filter(|r| {
                r.kind == RelationKind::Answers
                    && r.answer_completeness == Some(AnswerCompleteness::Full)
                    && valid(r, &current)
            })
            .map(|r| (&r.source.message_id, &r.source.quote))
            .collect();
        Ok(rows
            .iter()
            .filter(|r| {
                r.kind == RelationKind::Question
                    && valid(r, &current)
                    && !answered.contains(&(&r.source.message_id, &r.source.quote))
            })
            .map(|r| r.source.clone())
            .collect())
    }
}

#[cfg(test)]
mod tests;
