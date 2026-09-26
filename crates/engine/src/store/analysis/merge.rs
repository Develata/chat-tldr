//! Transactional topic consolidation; no model calls or cursor advancement.
use super::super::inbox;
use super::*;

const INVALID_ASSIGNMENT_SQL: &str = "SELECT EXISTS(SELECT 1 FROM topic_messages t LEFT JOIN messages m ON m.message_id=t.message_id WHERE t.topic_id IN (?1,?2) AND (m.message_id IS NULL OR m.chat_id!=?3))";
const UNFINISHED_SQL: &str = "SELECT EXISTS(SELECT 1 FROM topic_messages t JOIN messages m ON m.message_id=t.message_id WHERE t.topic_id IN (?1,?2) AND m.analysis_state NOT IN ('done','skipped'))";
const MOVED_INSIGHTS_SQL: &str = "SELECT insight_id,chat_id,body_json,rank_prior FROM insights WHERE topic_id=?1 ORDER BY insight_id";
const MOVED_FEEDBACK_SQL: &str = "SELECT EXISTS(SELECT 1 FROM insights i JOIN feedback f ON f.insight_id=i.insight_id WHERE i.topic_id=?1)";
const MOVE_MESSAGES_SQL: &str = "UPDATE topic_messages SET topic_id=?1 WHERE topic_id=?2";

#[derive(Debug)]
pub struct MergeResult {
    pub target: TopicRecord,
    pub source: TopicRecord,
    pub moved_insights: u64,
}

fn rejection_key(chat: &ChatId) -> String {
    format!("analysis.rejected_merges.v1:{chat}")
}

fn canonical(left: TopicId, right: TopicId) -> (TopicId, TopicId) {
    if left < right {
        (left, right)
    } else {
        (right, left)
    }
}

pub(super) fn read_rejected(
    connection: &Connection,
    chat: &ChatId,
) -> Result<BTreeSet<(TopicId, TopicId)>> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [rejection_key(chat)],
            |row| row.get(0),
        )
        .optional()?;
    let rejected: BTreeSet<(TopicId, TopicId)> = raw
        .map(|body| serde_json::from_str(&body))
        .transpose()?
        .unwrap_or_default();
    if rejected.iter().any(|(left, right)| left >= right) {
        return Err(EngineError::DatabaseFormat(
            "invalid canonical rejected topic pair".into(),
        ));
    }
    Ok(rejected)
}

fn save_rejected(
    connection: &Connection,
    chat: &ChatId,
    rejected: &BTreeSet<(TopicId, TopicId)>,
) -> Result<()> {
    connection.execute(
        "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![rejection_key(chat), serde_json::to_string(rejected)?],
    )?;
    Ok(())
}

/// Resolve exactly two indexed rows, including their latest title and activity.
fn active_pair(
    connection: &Connection,
    chat: &ChatId,
    into: &TopicId,
    from: &TopicId,
) -> Result<(TopicRecord, TopicRecord)> {
    if into == from {
        return Err(EngineError::Input(
            "cannot merge a topic into itself".into(),
        ));
    }
    let mut statement = connection.prepare_cached(
        "SELECT chat_id,title,title_is_provisional,state,last_message_at,is_chitchat FROM topics WHERE topic_id=?1",
    )?;
    let mut load = |id: &TopicId| -> Result<TopicRecord> {
        type Row = (String, String, bool, String, String, Option<f32>);
        let raw: Option<Row> = statement
            .query_row([id.as_ref()], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .optional()?;
        let Some((stored_chat, title, provisional, state, last, is_chitchat)) = raw else {
            return Err(EngineError::Input("merge topic does not exist".into()));
        };
        if stored_chat != chat.as_ref() {
            return Err(EngineError::Input("cross-chat topic merge".into()));
        }
        if state != "active" {
            return Err(EngineError::Input(
                "only active topics can be merged".into(),
            ));
        }
        Ok(TopicRecord {
            id: id.clone(),
            chat_id: chat.clone(),
            title,
            provisional,
            state: TopicState::Active,
            last_message_at: DateTime::parse_from_rfc3339(&last)
                .map_err(|_| EngineError::DatabaseFormat("invalid topic timestamp".into()))?,
            is_chitchat,
        })
    };
    Ok((load(into)?, load(from)?))
}

impl AnalysisSession {
    /// Persist one symmetric rejection. Repeating it leaves the store untouched.
    pub fn reject_merge(&self, left: &TopicId, right: &TopicId) -> Result<()> {
        let mut connection = open_write(&self.path)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        active_pair(&tx, &self.chat_id, left, right)?;
        let mut rejected = read_rejected(&tx, &self.chat_id)?;
        if rejected.insert(canonical(left.clone(), right.clone())) {
            save_rejected(&tx, &self.chat_id, &rejected)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Consolidate two active topics atomically. Repeating a completed merge is an
    /// error with no writes; callers refresh their snapshot after each success.
    pub fn merge_topics(
        &self,
        into: &TopicId,
        from: &TopicId,
        evidence: &[MessageId],
    ) -> Result<Option<MergeResult>> {
        let mut connection = open_write(&self.path)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (mut target, mut source) = active_pair(&tx, &self.chat_id, into, from)?;
        let rejected = read_rejected(&tx, &self.chat_id)?;
        if rejected.contains(&canonical(into.clone(), from.clone())) {
            return Err(EngineError::Input("this topic pair was rejected".into()));
        }
        let invalid_assignment: bool = tx.query_row(
            INVALID_ASSIGNMENT_SQL,
            params![into.as_ref(), from.as_ref(), self.chat_id.as_ref()],
            |row| row.get(0),
        )?;
        if invalid_assignment {
            return Err(EngineError::DatabaseFormat(
                "invalid topic message ownership".into(),
            ));
        }
        let unfinished: bool = tx.query_row(
            UNFINISHED_SQL,
            params![into.as_ref(), from.as_ref()],
            |row| row.get(0),
        )?;
        if unfinished || !supports_merge(&tx, &self.chat_id, into, from, evidence)? {
            return Ok(None);
        }
        let items: Vec<(String, String, String, f32)> = tx
            .prepare(MOVED_INSIGHTS_SQL)?
            .query_map([from.as_ref()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let moved_feedback: bool =
            tx.query_row(MOVED_FEEDBACK_SQL, [from.as_ref()], |row| row.get(0))?;
        // Without a source vote, no event changes its features and all existing
        // weights remain valid. Read them once and rescore only migrated items.
        let weights = if !moved_feedback && !items.is_empty() {
            Some(inbox::read_weights(&tx)?)
        } else {
            None
        };
        let moved_insights = u64::try_from(items.len())
            .map_err(|_| EngineError::DatabaseFormat("insight count overflow".into()))?;
        let now = Utc::now().fixed_offset();
        for (id, chat, body, prior) in items {
            let mut item: Insight = serde_json::from_str(&body)?;
            if chat != self.chat_id.as_ref()
                || item.chat_id != self.chat_id
                || item.id.as_ref() != id
                || item.topic_id.as_ref() != Some(from)
            {
                return Err(EngineError::DatabaseFormat(
                    "invalid topic insight ownership".into(),
                ));
            }
            item.topic_id = Some(into.clone());
            if let Some(weights) = &weights {
                item.rank_score = inbox::score_from_weights(&tx, &item, prior, weights)?;
            }
            // Only topic identity and its personalized score change: lifecycle,
            // evidence, priority, confidence and insight timestamps stay intact.
            tx.execute(
                "UPDATE insights SET topic_id=?2,body_json=?3 WHERE insight_id=?1",
                params![id, into.as_ref(), serde_json::to_string(&item)?],
            )?;
        }
        tx.execute(MOVE_MESSAGES_SQL, params![into.as_ref(), from.as_ref()])?;
        target.last_message_at = target.last_message_at.max(source.last_message_at);
        tx.execute(
            "UPDATE topics SET last_message_at=?2,updated_at=?3 WHERE topic_id=?1",
            params![
                into.as_ref(),
                target.last_message_at.to_rfc3339(),
                now.to_rfc3339()
            ],
        )?;
        tx.execute(
            "UPDATE topics SET state='merged',merged_into=?2,updated_at=?3 WHERE topic_id=?1",
            params![from.as_ref(), into.as_ref(), now.to_rfc3339()],
        )?;
        source.state = TopicState::Merged;
        let transferred: BTreeSet<_> = rejected
            .into_iter()
            .map(|(left, right)| {
                canonical(
                    if left == *from { into.clone() } else { left },
                    if right == *from { into.clone() } else { right },
                )
            })
            .filter(|(left, right)| left != right)
            .collect();
        if !transferred.is_empty() {
            save_rejected(&tx, &self.chat_id, &transferred)?;
        }
        // Moving a voted insight changes the chronological topic-feature
        // history. Keep exact vote/decay replay for this less common path.
        if moved_feedback {
            inbox::rebuild_preferences(&tx, now)?;
        }
        tx.execute(
            "INSERT INTO run_checkpoints(run_id,topic_id,stage,status,updated_at) VALUES(?1,?2,'merge','complete',?3)",
            params![self.run_id.as_ref(), from.as_ref(), now.to_rfc3339()],
        )?;
        tx.execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2)",
            params![
                format!("analysis.topic_merge.v1:{}:{from}", self.run_id),
                serde_json::json!({"chat_id":self.chat_id,"into":into,"from":from,
                    "moved_insights":moved_insights,"completed_at":now})
                .to_string()
            ],
        )?;
        tx.commit()?;
        Ok(Some(MergeResult {
            target,
            source,
            moved_insights,
        }))
    }
}

/// The model saw only these representative rows. Recheck them under the same
/// write transaction used for consolidation, so recall/backfill cannot leave a
/// stale decision committing an unsupported merge. Each source has one reply.
fn supports_merge(
    connection: &Connection,
    chat: &ChatId,
    into: &TopicId,
    from: &TopicId,
    evidence: &[MessageId],
) -> Result<bool> {
    if evidence.len() > 6 {
        return Err(EngineError::Input(
            "merge evidence exceeds six messages".into(),
        ));
    }
    type Row = (String, String, bool, Cursor, Option<MessageId>);
    let mut statement = connection.prepare_cached(
        "SELECT m.chat_id,t.topic_id,m.recalled,m.sent_at_ms,m.pk,m.reply_resolved FROM messages m JOIN topic_messages t ON t.message_id=m.message_id WHERE m.message_id=?1",
    )?;
    let ids: BTreeSet<_> = evidence.iter().collect();
    let mut rows = BTreeMap::new();
    for id in ids {
        let row: Option<Row> = statement
            .query_row([id.as_ref()], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    Cursor {
                        sent_at_ms: row.get(3)?,
                        ordinal: row.get(4)?,
                    },
                    row.get::<_, Option<String>>(5)?.map(Into::into),
                ))
            })
            .optional()?;
        let Some((stored_chat, topic, recalled, cursor, reply)) = row else {
            return Ok(false);
        };
        if stored_chat != chat.as_ref()
            || recalled
            || (topic != into.as_ref() && topic != from.as_ref())
        {
            return Ok(false);
        }
        rows.insert(id, (topic, cursor, reply));
    }
    let edges = rows
        .values()
        .filter(|(topic, cursor, reply)| {
            reply.as_ref().and_then(|id| rows.get(id)).is_some_and(
                |(other_topic, other_cursor, _)| topic != other_topic && other_cursor < cursor,
            )
        })
        .count();
    Ok(edges >= 2)
}

#[cfg(test)]
mod tests;
