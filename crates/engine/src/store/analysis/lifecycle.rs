//! Message-time topic expiry; closing never changes members or pending work.
use super::*;

const CLOSED_AT_PREFIX: &str = "analysis.topic_closed_at.v1:";
const CLOSED_AT_SQL: &str = "SELECT t.topic_id,m.value FROM topics t JOIN meta m ON m.key=?1 || t.topic_id WHERE t.chat_id=?2 AND t.state='closed' ORDER BY t.topic_id";

/// One joined snapshot query; legacy closed rows have no inferred boundary.
pub(super) fn read_closed_at(
    connection: &Connection,
    chat: &ChatId,
) -> Result<BTreeMap<TopicId, DateTime<FixedOffset>>> {
    let rows: Vec<(String, String)> = connection
        .prepare(CLOSED_AT_SQL)?
        .query_map(params![CLOSED_AT_PREFIX, chat.as_ref()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    rows.into_iter()
        .map(|(id, raw)| {
            let at = serde_json::from_str(&raw).map_err(|_| {
                EngineError::DatabaseFormat("invalid topic closing timestamp".into())
            })?;
            Ok((id.into(), at))
        })
        .collect()
}

impl AnalysisSession {
    /// Close still-expired active candidates in one transaction, using their
    /// latest stored activity. Equality with the timeout remains active.
    /// Closed/merged candidates are unchanged; invalid scope rolls back all.
    pub fn close_topics(
        &self,
        candidates: &[TopicId],
        at: DateTime<FixedOffset>,
        close_secs: u64,
    ) -> Result<Vec<TopicRecord>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let mut connection = open_write(&self.path)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut closed = Vec::new();
        let now = Utc::now().to_rfc3339();
        let ids: BTreeSet<_> = candidates.iter().collect();
        for id in ids {
            let Some(mut topic) = active_candidate(&tx, &self.chat_id, id)? else {
                continue;
            };
            let elapsed = at.signed_duration_since(topic.last_message_at);
            // Compare whole seconds and the positive fractional remainder. This
            // avoids overflowing a duration for an arbitrarily large u64 TTL,
            // and keeps sub-millisecond DateTime values strictly ordered too.
            if elapsed <= chrono::TimeDelta::zero() {
                continue;
            }
            let seconds = elapsed.num_seconds() as u64;
            if seconds < close_secs || (seconds == close_secs && elapsed.subsec_nanos() == 0) {
                continue;
            }
            tx.execute(
                "UPDATE topics SET state='closed',updated_at=?2 WHERE topic_id=?1",
                params![id.as_ref(), now],
            )?;
            tx.execute(
                "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
                params![
                    format!("{CLOSED_AT_PREFIX}{id}"),
                    serde_json::to_string(&at)?
                ],
            )?;
            tx.execute(
                "INSERT INTO run_checkpoints(run_id,topic_id,stage,status,updated_at) VALUES(?1,?2,'close','complete',?3) ON CONFLICT(run_id,topic_id,stage) DO NOTHING",
                params![self.run_id.as_ref(), id.as_ref(), now],
            )?;
            topic.state = TopicState::Closed;
            closed.push(topic);
        }
        tx.commit()?;
        Ok(closed)
    }
}

fn active_candidate(
    connection: &Connection,
    chat: &ChatId,
    id: &TopicId,
) -> Result<Option<TopicRecord>> {
    type Row = (String, String, bool, String, String, Option<f32>);
    let raw: Option<Row> = connection
        .prepare_cached("SELECT chat_id,title,title_is_provisional,state,last_message_at,is_chitchat FROM topics WHERE topic_id=?1")?
        .query_row([id.as_ref()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .optional()?;
    let Some((stored_chat, title, provisional, state, last, is_chitchat)) = raw else {
        return Err(EngineError::Input("close topic does not exist".into()));
    };
    if stored_chat != chat.as_ref() {
        return Err(EngineError::Input("cross-chat topic close".into()));
    }
    match state.as_str() {
        "closed" | "merged" => return Ok(None),
        "active" => {}
        _ => return Err(EngineError::DatabaseFormat("unknown topic state".into())),
    }
    Ok(Some(TopicRecord {
        id: id.clone(),
        chat_id: chat.clone(),
        title,
        provisional,
        state: TopicState::Active,
        last_message_at: DateTime::parse_from_rfc3339(&last)
            .map_err(|_| EngineError::DatabaseFormat("invalid topic timestamp".into()))?,
        is_chitchat,
    }))
}

#[cfg(test)]
mod tests;
