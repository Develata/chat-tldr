use super::{check_version, open_read, update_cursors, wire_string};
use crate::{
    EngineError, Result,
    verify::{self, DraftEvidence, MessageLookup, VerifyInput},
};
use chat_tldr_core::*;
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct StoredMessage {
    pub message: UnifiedMessage,
    pub cursor: Cursor,
    pub analysis_state: String,
    pub topic_id: Option<TopicId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopicRecord {
    pub id: TopicId,
    pub chat_id: ChatId,
    pub title: String,
    pub provisional: bool,
    pub state: TopicState,
    pub last_message_at: DateTime<FixedOffset>,
    pub is_chitchat: Option<f32>,
}

pub struct AnalysisSnapshot {
    pub chat: ChatMeta,
    pub messages: Vec<StoredMessage>,
    pub topics: Vec<TopicRecord>,
    pub insights: Vec<Insight>,
}

pub fn analysis_snapshot(path: &Path, chat: &ChatId) -> Result<AnalysisSnapshot> {
    let Some(mut connection) = open_read(path)? else {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    };
    let tx = connection.transaction()?;
    let row: Option<(String, String, Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT kind,display_name,self_uid,self_uin FROM chats WHERE chat_id=?1",
            [chat.as_ref()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((kind, display_name, self_uid, self_uin)) = row else {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    };
    let raw:Vec<(i64,String,String,Option<String>)>=tx.prepare("SELECT m.pk,m.body_json,m.analysis_state,t.topic_id FROM messages m LEFT JOIN topic_messages t ON t.message_id=m.message_id WHERE m.chat_id=?1 ORDER BY m.sent_at_ms,m.pk")?.query_map([chat.as_ref()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<std::result::Result<_,_>>()?;
    let messages = raw
        .into_iter()
        .map(|(ordinal, body, analysis_state, topic)| {
            let message: UnifiedMessage = serde_json::from_str(&body)?;
            Ok(StoredMessage {
                cursor: Cursor {
                    sent_at_ms: message.sent_at.timestamp_millis(),
                    ordinal,
                },
                message,
                analysis_state,
                topic_id: topic.map(Into::into),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let topics = read_topics(&tx, chat)?;
    let bodies: Vec<String> = tx
        .prepare("SELECT body_json FROM insights WHERE chat_id=?1 ORDER BY insight_id")?
        .query_map([chat.as_ref()], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let insights = bodies
        .into_iter()
        .map(|s| serde_json::from_str(&s).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    Ok(AnalysisSnapshot {
        chat: ChatMeta {
            chat_id: chat.clone(),
            kind: serde_json::from_value(serde_json::json!(kind))?,
            display_name,
            self_uid,
            self_uin,
        },
        messages,
        topics,
        insights,
    })
}

pub(super) fn read_topics(connection: &Connection, chat: &ChatId) -> Result<Vec<TopicRecord>> {
    type TopicRow = (String, String, bool, String, String, Option<f32>);
    let raw:Vec<TopicRow>=connection.prepare("SELECT topic_id,title,title_is_provisional,state,last_message_at,is_chitchat FROM topics WHERE chat_id=?1 ORDER BY topic_id")?.query_map([chat.as_ref()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?.collect::<std::result::Result<_,_>>()?;
    raw.into_iter()
        .map(|(id, title, provisional, state, last, chitchat)| {
            Ok(TopicRecord {
                id: id.into(),
                chat_id: chat.clone(),
                title,
                provisional,
                state: serde_json::from_value(serde_json::json!(state))?,
                last_message_at: DateTime::parse_from_rfc3339(&last)
                    .map_err(|_| EngineError::DatabaseFormat("invalid topic timestamp".into()))?,
                is_chitchat: chitchat,
            })
        })
        .collect()
}

pub(super) fn open_write(path: &Path) -> Result<Connection> {
    let connection =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    check_version(&connection)?;
    Ok(connection)
}

/// Holds an OS lock for the lifetime of one analysis; never unlink an active lock.
pub struct AnalysisSession {
    path: PathBuf,
    pub run_id: RunId,
    pub chat_id: ChatId,
    _lock: File,
    finished: bool,
}

impl AnalysisSession {
    pub fn begin(
        path: &Path,
        chat: &ChatId,
        run: &RunId,
        args: &serde_json::Value,
    ) -> Result<Self> {
        let path = path.canonicalize()?;
        let lock_dir = path
            .parent()
            .ok_or_else(|| EngineError::DatabaseFormat("database has no parent".into()))?
            .join("tmp/engine");
        std::fs::create_dir_all(&lock_dir)?;
        let key = blake3::hash(format!("{}\n{}", path.display(), chat).as_bytes()).to_hex();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_dir.join(format!("analyze-{key}.lock")))?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|e| {
            if e.kind() == std::io::ErrorKind::WouldBlock
                || e.raw_os_error() == fs2::lock_contended_error().raw_os_error()
            {
                EngineError::RunInProgress(chat.to_string())
            } else {
                EngineError::Io(e)
            }
        })?;
        let mut connection = open_write(&path)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM chats WHERE chat_id=?1)",
            [chat.as_ref()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(EngineError::ChatNotFound(chat.to_string()));
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("UPDATE runs SET status='cancelled',finished_at=?2 WHERE chat_id=?1 AND status='running'",params![chat.as_ref(),now])?;
        tx.execute("INSERT INTO runs(run_id,command,chat_id,status,pid,args_json,started_at,heartbeat_at) VALUES(?1,'analyze',?2,'running',?3,?4,?5,?5)",params![run.as_ref(),chat.as_ref(),std::process::id(),args.to_string(),now])?;
        tx.commit()?;
        Ok(Self {
            path,
            run_id: run.clone(),
            chat_id: chat.clone(),
            _lock: lock,
            finished: false,
        })
    }

    pub fn snapshot(&self) -> Result<AnalysisSnapshot> {
        analysis_snapshot(&self.path, &self.chat_id)
    }

    /// Read only the requested source rows from one consistent, read-only snapshot.
    /// Duplicate IDs are fetched once; missing and out-of-chat IDs are omitted.
    pub fn source_messages(
        &self,
        ids: &[MessageId],
    ) -> Result<BTreeMap<MessageId, UnifiedMessage>> {
        let Some(mut connection) = open_read(&self.path)? else {
            return Err(EngineError::ChatNotFound(self.chat_id.to_string()));
        };
        let tx = connection.transaction()?;
        read_source_messages(&tx, &self.chat_id, ids.iter())
    }

    pub fn record_decision(&self, payload: &DecisionPayload) -> Result<()> {
        let connection = open_write(&self.path)?;
        let mut observation = serde_json::to_value(&payload.observation)?;
        observation["_history_v1"] = serde_json::json!({"confidence":payload.confidence});
        connection.execute("INSERT INTO decisions(run_id,step,observation_json,allowed_json,chosen_json,method,probabilities_json,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![self.run_id.as_ref(),payload.step,observation.to_string(),serde_json::to_string(&payload.allowed)?,serde_json::to_string(&payload.chosen)?,wire_string(&payload.method)?,payload.probabilities.as_ref().map(serde_json::to_string).transpose()?,payload.reason,Utc::now().to_rfc3339()])?;
        Ok(())
    }

    pub fn assign(&self, topic: &TopicRecord, messages: &[MessageId], method: &str) -> Result<()> {
        if topic.chat_id != self.chat_id {
            return Err(EngineError::Input("cross-chat topic assignment".into()));
        }
        let mut connection = open_write(&self.path)?;
        let tx = connection.transaction()?;
        upsert_topic(&tx, topic, &self.run_id)?;
        let burst_id = format!(
            "b_{}",
            blake3::hash(serde_json::to_string(&(self.run_id.clone(), messages))?.as_bytes())
                .to_hex()
        );
        for id in messages {
            let belongs: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE message_id=?1 AND chat_id=?2)",
                params![id.as_ref(), self.chat_id.as_ref()],
                |r| r.get(0),
            )?;
            if !belongs {
                return Err(EngineError::Input(
                    "topic contains an out-of-chat message".into(),
                ));
            }
            tx.execute("INSERT INTO topic_messages(topic_id,message_id,burst_id,method,confidence,run_id) VALUES(?1,?2,?3,?4,NULL,?5) ON CONFLICT(message_id) DO NOTHING",params![topic.id.as_ref(),id.as_ref(),burst_id,method,self.run_id.as_ref()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Recheck evidence against current source rows within the commit transaction.
    pub fn commit_topic(
        &self,
        topic: &TopicRecord,
        messages: &[MessageId],
        items: Vec<(Insight, f32)>,
    ) -> Result<Vec<Insight>> {
        if topic.chat_id != self.chat_id {
            return Err(EngineError::Input("cross-chat topic commit".into()));
        }
        let mut connection = open_write(&self.path)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for id in messages {
            let belongs:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN topic_messages t ON t.message_id=m.message_id WHERE m.message_id=?1 AND m.chat_id=?2 AND t.topic_id=?3)",params![id.as_ref(),self.chat_id.as_ref(),topic.id.as_ref()],|r|r.get(0))?;
            if !belongs {
                return Err(EngineError::Input(
                    "checkpoint message is outside the assigned topic".into(),
                ));
            }
        }
        let lookup = CurrentMessages(read_source_messages(
            &tx,
            &self.chat_id,
            items
                .iter()
                .flat_map(|(item, _)| item.evidence.iter().map(|evidence| &evidence.message_id)),
        )?);
        upsert_topic(&tx, topic, &self.run_id)?;
        let mut committed = Vec::new();
        for (mut item, prior) in items {
            if item.chat_id != self.chat_id || item.topic_id.as_ref() != Some(&topic.id) {
                return Err(EngineError::Input("cross-topic insight commit".into()));
            }
            let draft: Vec<_> = item
                .evidence
                .iter()
                .map(|e| DraftEvidence {
                    message_id: e.message_id.clone(),
                    quote: e.quote.clone(),
                })
                .collect();
            let profile = item
                .evidence
                .first()
                .map(|e| e.render_profile)
                .unwrap_or_default();
            let report = verify::verify(
                &VerifyInput {
                    kind: item.kind,
                    deadline_raw: item.deadline.as_ref().map(|d| d.raw.as_str()),
                    evidence: &draft,
                    profile,
                },
                &lookup,
            );
            item.verification_status = if item.evidence.iter().any(|e| e.render_profile != profile)
            {
                VerificationStatus::Rejected
            } else {
                report.status
            };
            let existing: Option<(String, String)> = tx
                .query_row(
                    "SELECT chat_id,lifecycle FROM insights WHERE insight_id=?1",
                    [item.id.as_ref()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((chat, lifecycle)) = existing {
                if chat != self.chat_id.as_ref() {
                    return Err(EngineError::Input("cross-chat insight update".into()));
                }
                item.lifecycle = serde_json::from_value(serde_json::json!(lifecycle))?;
            }
            item.rank_score = super::inbox::personalized_score(&tx, &item, prior)?;
            tx.execute("INSERT INTO insights(insight_id,chat_id,topic_id,kind,priority,rank_prior,verification_status,lifecycle,body_json,created_in_run,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(insight_id) DO UPDATE SET kind=excluded.kind,priority=excluded.priority,rank_prior=excluded.rank_prior,verification_status=excluded.verification_status,lifecycle=excluded.lifecycle,body_json=excluded.body_json,updated_at=excluded.updated_at",params![item.id.as_ref(),item.chat_id.as_ref(),item.topic_id.as_ref().map(TopicId::as_ref),wire_string(&item.kind)?,wire_string(&item.priority)?,prior,wire_string(&item.verification_status)?,wire_string(&item.lifecycle)?,serde_json::to_string(&item)?,item.created_in_run.as_ref(),item.updated_at.to_rfc3339()])?;
            tx.execute(
                "DELETE FROM evidence WHERE insight_id=?1",
                [item.id.as_ref()],
            )?;
            for (evidence, ok) in item.evidence.iter().zip(report.evidence_ok) {
                tx.execute("INSERT INTO evidence(insight_id,message_id,quote,render_profile,ok) VALUES(?1,?2,?3,?4,?5)",params![item.id.as_ref(),evidence.message_id.as_ref(),evidence.quote,evidence.render_profile.to_string(),ok])?;
            }
            committed.push(item);
        }
        for id in messages {
            tx.execute("UPDATE messages SET analysis_state=CASE WHEN recalled=1 THEN 'skipped' ELSE 'done' END,analyzed_run=?2 WHERE message_id=?1 AND chat_id=?3",params![id.as_ref(),self.run_id.as_ref(),self.chat_id.as_ref()])?;
        }
        tx.execute("INSERT INTO run_checkpoints(run_id,topic_id,stage,status,updated_at) VALUES(?1,?2,'verify','complete',?3) ON CONFLICT(run_id,topic_id,stage) DO UPDATE SET status='complete',updated_at=excluded.updated_at",params![self.run_id.as_ref(),topic.id.as_ref(),Utc::now().to_rfc3339()])?;
        update_cursors(&tx, &self.chat_id)?;
        tx.commit()?;
        Ok(committed)
    }

    pub fn fail_messages(&self, ids: &[MessageId]) -> Result<()> {
        let mut connection = open_write(&self.path)?;
        let tx = connection.transaction()?;
        for id in ids {
            tx.execute("UPDATE messages SET analysis_state='failed' WHERE message_id=?1 AND chat_id=?2 AND recalled=0 AND analysis_state!='done'",params![id.as_ref(),self.chat_id.as_ref()])?;
        }
        update_cursors(&tx, &self.chat_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn cache_get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let connection = open_write(&self.path)?;
        let value: Option<String> = connection
            .query_row(
                "SELECT response_json FROM model_cache WHERE cache_key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }

    pub fn cache_put<T: Serialize>(
        &self,
        key: &str,
        stage: &str,
        provider: &str,
        model: &str,
        value: &T,
    ) -> Result<()> {
        let connection = open_write(&self.path)?;
        connection.execute("INSERT INTO model_cache(cache_key,stage,provider,model,response_json,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(cache_key) DO NOTHING",params![key,stage,provider,model,serde_json::to_string(value)?,Utc::now().to_rfc3339()])?;
        Ok(())
    }

    pub fn record_usage(&self, usage: &UsageStats) -> Result<()> {
        let connection = open_write(&self.path)?;
        connection.execute("INSERT INTO usage(run_id,stage,provider,model,calls,cache_hits,input_tokens,output_tokens,cost_usd) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(run_id,stage,provider,model) DO UPDATE SET calls=calls+excluded.calls,cache_hits=cache_hits+excluded.cache_hits,input_tokens=input_tokens+excluded.input_tokens,output_tokens=output_tokens+excluded.output_tokens,cost_usd=cost_usd+excluded.cost_usd",params![self.run_id.as_ref(),usage.stage,usage.provider,usage.model,usage.calls as i64,usage.cache_hits as i64,usage.input_tokens as i64,usage.output_tokens as i64,usage.cost_usd])?;
        Ok(())
    }

    pub fn record_answer(
        &self,
        payload: &JevAnswerPayload,
        provider: &str,
        cache_hit: bool,
    ) -> Result<()> {
        let connection = open_write(&self.path)?;
        let mut subject = serde_json::to_value(&payload.subject)?;
        subject["_history_v1"] =
            serde_json::json!({"model":payload.model,"provider":provider,"cache_hit":cache_hit});
        connection.execute("INSERT INTO jev_answers(run_id,request_key,question_id,qtype,answer_json,confidence,subject_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![self.run_id.as_ref(),payload.request_key,payload.question_id,payload.qtype,payload.answer.to_string(),payload.confidence,subject.to_string()])?;
        Ok(())
    }

    pub fn finish(&mut self, status: RunStatus) -> Result<()> {
        let connection = open_write(&self.path)?;
        connection.execute(
            "UPDATE runs SET status=?2,finished_at=?3,heartbeat_at=?3 WHERE run_id=?1",
            params![
                self.run_id.as_ref(),
                wire_string(&status)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        self.finished = true;
        Ok(())
    }

    /// Persist exact counters and terminal status atomically without changing DB v1.
    pub fn finish_with_stats(&mut self, status: RunStatus, stats: &RunStats) -> Result<()> {
        if stats.run_id != self.run_id || stats.chat_id != self.chat_id {
            return Err(EngineError::Input(
                "statistics belong to another run".into(),
            ));
        }
        let mut connection = open_write(&self.path)?;
        let tx = connection.transaction()?;
        tx.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("history.run_stats.v1:{}",self.run_id),serde_json::to_string(stats)?])?;
        tx.execute(
            "UPDATE runs SET status=?2,finished_at=?3,heartbeat_at=?3 WHERE run_id=?1",
            params![
                self.run_id.as_ref(),
                wire_string(&status)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        tx.commit()?;
        self.finished = true;
        Ok(())
    }
}

const SOURCE_MESSAGE_SQL: &str =
    "SELECT body_json FROM messages WHERE message_id=?1 AND chat_id=?2";

/// Reuse the unique message_id index and one prepared query for all distinct IDs.
/// The caller owns the transaction, including the commit-time verification snapshot.
fn read_source_messages<'a>(
    connection: &Connection,
    chat: &ChatId,
    ids: impl IntoIterator<Item = &'a MessageId>,
) -> Result<BTreeMap<MessageId, UnifiedMessage>> {
    let ids: BTreeSet<_> = ids.into_iter().collect();
    let mut messages = BTreeMap::new();
    if ids.is_empty() {
        return Ok(messages);
    }
    let mut statement = connection.prepare_cached(SOURCE_MESSAGE_SQL)?;
    for id in ids {
        let body: Option<String> = statement
            .query_row(params![id.as_ref(), chat.as_ref()], |row| row.get(0))
            .optional()?;
        if let Some(body) = body {
            let message: UnifiedMessage = serde_json::from_str(&body)?;
            if message.id != *id || message.chat_id != *chat {
                return Err(EngineError::DatabaseFormat(
                    "source message identity does not match its stored row".into(),
                ));
            }
            messages.insert(message.id.clone(), message);
        }
    }
    Ok(messages)
}

impl Drop for AnalysisSession {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish(RunStatus::Failed);
        }
    }
}

fn upsert_topic(connection: &Connection, topic: &TopicRecord, run: &RunId) -> Result<()> {
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT chat_id,last_message_at FROM topics WHERE topic_id=?1",
            [topic.id.as_ref()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let mut topic = topic.clone();
    if let Some((chat, last)) = existing {
        if chat != topic.chat_id.as_ref() {
            return Err(EngineError::Input("cross-chat topic update".into()));
        }
        let last = DateTime::parse_from_rfc3339(&last)
            .map_err(|_| EngineError::DatabaseFormat("invalid topic timestamp".into()))?;
        topic.last_message_at = topic.last_message_at.max(last);
    }
    connection.execute("INSERT INTO topics(topic_id,chat_id,title,title_is_provisional,state,last_message_at,is_chitchat,created_in_run,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(topic_id) DO UPDATE SET title=excluded.title,title_is_provisional=excluded.title_is_provisional,state=excluded.state,last_message_at=excluded.last_message_at,is_chitchat=excluded.is_chitchat,updated_at=excluded.updated_at",params![topic.id.as_ref(),topic.chat_id.as_ref(),topic.title,topic.provisional,wire_string(&topic.state)?,topic.last_message_at.to_rfc3339(),topic.is_chitchat,run.as_ref(),Utc::now().to_rfc3339()])?;
    Ok(())
}

pub(super) struct CurrentMessages(pub BTreeMap<MessageId, UnifiedMessage>);
impl MessageLookup for CurrentMessages {
    fn rendered(&self, id: &MessageId, profile: RenderProfile) -> Option<String> {
        self.0
            .get(id)
            .and_then(|m| crate::render::render_with_profile(m, profile))
    }
    fn is_recalled(&self, id: &MessageId) -> bool {
        self.0.get(id).is_some_and(|m| m.recalled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ImportOptions, import_batches};
    use chat_tldr_qce::{QceOptions, parse_qce_json};

    struct Fixture {
        _directory: tempfile::TempDir,
        path: PathBuf,
        batch: ImportBatch,
        foreign: ImportBatch,
    }

    impl Fixture {
        fn new() -> Self {
            let source = include_bytes!("../../../../fixtures/qce/synthetic-group.json");
            let batch = parse_qce_json(source, &QceOptions::default()).unwrap();
            let mut foreign_source: serde_json::Value = serde_json::from_slice(source).unwrap();
            foreign_source["chatInfo"]["peerUid"] = serde_json::json!("other-chat");
            let foreign = parse_qce_json(
                &serde_json::to_vec(&foreign_source).unwrap(),
                &QceOptions::default(),
            )
            .unwrap();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("analysis.db");
            import_batches(
                &path,
                &[batch.clone(), foreign.clone()],
                &ImportOptions::default(),
            )
            .unwrap();
            Self {
                _directory: directory,
                path,
                batch,
                foreign,
            }
        }

        fn session(&self) -> AnalysisSession {
            AnalysisSession::begin(
                &self.path,
                &self.batch.chat.chat_id,
                &"r_sources".into(),
                &serde_json::json!({}),
            )
            .unwrap()
        }

        fn corrupt_unrelated_body(&self) {
            open_write(&self.path)
                .unwrap()
                .execute(
                    "UPDATE messages SET body_json='not json' WHERE message_id=?1",
                    [self.batch.messages[1].id.as_ref()],
                )
                .unwrap();
        }
    }

    #[test]
    fn source_messages_fetch_only_distinct_requested_rows_in_the_session_chat() {
        let fixture = Fixture::new();
        let session = fixture.session();
        fixture.corrupt_unrelated_body();
        let first = &fixture.batch.messages[0];
        let ids = [
            first.id.clone(),
            first.id.clone(),
            fixture.foreign.messages[0].id.clone(),
            "missing".into(),
        ];
        let before = std::fs::read(&fixture.path).unwrap();
        let sources = session.source_messages(&ids).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[&first.id], *first);
        assert!(session.source_messages(&[]).unwrap().is_empty());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    }

    #[test]
    fn commit_checks_only_evidence_sources_and_rejects_missing_foreign_and_recalled_rows() {
        let fixture = Fixture::new();
        let session = fixture.session();
        let first = &fixture.batch.messages[0];
        let topic = TopicRecord {
            id: "t_sources".into(),
            chat_id: fixture.batch.chat.chat_id.clone(),
            title: "合成话题".into(),
            provisional: false,
            state: TopicState::Active,
            last_message_at: first.sent_at,
            is_chitchat: None,
        };
        session
            .assign(&topic, std::slice::from_ref(&first.id), "direct")
            .unwrap();
        fixture.corrupt_unrelated_body();
        let sources = [
            first.id.clone(),
            "missing".into(),
            fixture.foreign.messages[0].id.clone(),
            fixture.batch.messages[2].id.clone(),
        ];
        let items = sources
            .into_iter()
            .enumerate()
            .map(|(index, source)| {
                (
                    Insight {
                        id: format!("i_source_{index}").into(),
                        chat_id: topic.chat_id.clone(),
                        kind: InsightKind::Todo,
                        title: "合成事项".into(),
                        summary: "合成证据".into(),
                        priority: Priority::P1,
                        rank_score: 1.0,
                        confidence: Some(1.0),
                        assignee: Assignee::Other,
                        deadline: None,
                        evidence: vec![Evidence {
                            message_id: source,
                            quote: crate::render::render(first),
                            render_profile: RenderProfile::default(),
                        }],
                        topic_id: Some(topic.id.clone()),
                        verification_status: VerificationStatus::Verified,
                        lifecycle: Lifecycle::Open,
                        created_in_run: session.run_id.clone(),
                        updated_at: first.sent_at,
                    },
                    1.0,
                )
            })
            .collect();
        let committed = session
            .commit_topic(&topic, std::slice::from_ref(&first.id), items)
            .unwrap();
        assert_eq!(
            committed[0].verification_status,
            VerificationStatus::Verified
        );
        assert!(
            committed[1..]
                .iter()
                .all(|item| item.verification_status == VerificationStatus::Rejected)
        );
        let connection = open_read(&fixture.path).unwrap().unwrap();
        let state: String = connection
            .query_row(
                "SELECT analysis_state FROM messages WHERE message_id=?1",
                [first.id.as_ref()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "done");
    }

    #[test]
    fn indexed_source_reads_keep_one_snapshot_until_the_transaction_ends() {
        let fixture = Fixture::new();
        let writer = open_write(&fixture.path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        let mut connection = open_read(&fixture.path).unwrap().unwrap();
        let first = &fixture.batch.messages[0];
        let tx = connection.transaction().unwrap();
        let ids = std::slice::from_ref(&first.id);
        assert!(
            !read_source_messages(&tx, &fixture.batch.chat.chat_id, ids).unwrap()[&first.id]
                .recalled
        );
        let mut recalled = first.clone();
        recalled.recalled = true;
        writer
            .execute(
                "UPDATE messages SET body_json=?2,recalled=1 WHERE message_id=?1",
                params![first.id.as_ref(), serde_json::to_string(&recalled).unwrap()],
            )
            .unwrap();
        assert!(
            !read_source_messages(&tx, &fixture.batch.chat.chat_id, ids).unwrap()[&first.id]
                .recalled
        );
        tx.commit().unwrap();
        assert!(
            read_source_messages(&connection, &fixture.batch.chat.chat_id, ids).unwrap()[&first.id]
                .recalled
        );

        let plan: String = connection
            .query_row(
                &format!("EXPLAIN QUERY PLAN {SOURCE_MESSAGE_SQL}"),
                params![first.id.as_ref(), fixture.batch.chat.chat_id.as_ref()],
                |row| row.get(3),
            )
            .unwrap();
        assert!(plan.contains("SEARCH messages USING INDEX"), "{plan}");
        assert!(plan.contains("message_id=?"), "{plan}");
        assert!(!plan.contains("SCAN"), "{plan}");
    }
}
