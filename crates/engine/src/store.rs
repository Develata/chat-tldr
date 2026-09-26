//! The only module that opens SQLite. Read commands never initialize a database.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

use chat_tldr_core::*;
use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::json;

use crate::{EngineError, Result, render::render};

const MIGRATION: &str = include_str!("../migrations/0001_initial.sql");
mod analysis;
pub use analysis::*;
mod history;
pub use history::{History, decisions, jev_log, stats};
mod inbox;
pub(crate) use inbox::InboxData;
mod overview;
pub use inbox::{
    InboxOptions, InboxSnapshot, decay_preferences, feedback, inbox, mark_read, resolve,
};
pub use overview::{OverviewOptions, overview};
type ChatRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Default)]
pub struct ImportOptions {
    pub self_uid: Option<String>,
    pub self_uin: Option<String>,
    pub run_id: Option<RunId>,
    pub file_paths: Vec<String>,
}

pub struct ImportReport {
    pub chat_ids: Vec<ChatId>,
    pub changed: bool,
    pub stats: serde_json::Value,
    pub warnings: Vec<String>,
}

fn check_version(connection: &Connection) -> Result<u32> {
    let version: String =
        connection.query_row("SELECT value FROM meta WHERE key='db_version'", [], |row| {
            row.get(0)
        })?;
    let version = version
        .parse::<u32>()
        .map_err(|_| EngineError::DatabaseFormat("invalid db_version".into()))?;
    if version != DB_VERSION {
        return Err(EngineError::DatabaseFormat(format!(
            "database version {version}; this binary requires {DB_VERSION}"
        )));
    }
    Ok(version)
}

fn open_read(path: &Path) -> Result<Option<Connection>> {
    match path.try_exists()? {
        false => Ok(None),
        true => {
            let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            connection.busy_timeout(Duration::from_secs(5))?;
            check_version(&connection)?;
            Ok(Some(connection))
        }
    }
}

pub fn inspect(path: &Path) -> Result<Option<u32>> {
    Ok(open_read(path)?.map(|_| DB_VERSION))
}

pub fn import_batches(
    path: &Path,
    batches: &[ImportBatch],
    options: &ImportOptions,
) -> Result<ImportReport> {
    for value in [&options.self_uid, &options.self_uin].into_iter().flatten() {
        if value.trim().is_empty() {
            return Err(EngineError::Config("self identity cannot be empty".into()));
        }
    }
    if batches.is_empty() {
        return Err(EngineError::Input(
            "at least one import batch is required".into(),
        ));
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    let existing: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [],
        |r| r.get(0),
    )?;
    if existing != 0 {
        check_version(&connection)?;
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: i64 = tx.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [],
        |r| r.get(0),
    )?;
    if existing == 0 {
        tx.execute_batch(MIGRATION)?;
        tx.execute(
            "INSERT INTO meta(key,value) VALUES('db_version',?1)",
            [DB_VERSION.to_string()],
        )?;
    } else {
        check_version(&tx)?;
    }
    let now = Utc::now().to_rfc3339();
    let mut ids = BTreeSet::new();
    let mut warnings = Vec::new();
    let (mut seen, mut inserted, mut duplicate, mut backfilled, mut recalled) =
        (0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
    let mut changed = false;
    for (file_index, batch) in batches.iter().enumerate() {
        let chat = &batch.chat;
        ids.insert(chat.chat_id.clone());
        let prior: Option<(Option<String>, Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT self_uid,self_uin,last_analyzed FROM chats WHERE chat_id=?1",
                [chat.chat_id.as_ref()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let old_uid = prior.as_ref().and_then(|x| x.0.clone());
        let old_uin = prior.as_ref().and_then(|x| x.1.clone());
        let uid = identity(
            old_uid,
            options.self_uid.as_ref(),
            chat.self_uid.as_ref(),
            "self_uid",
        )?;
        let uin = identity(
            old_uin,
            options.self_uin.as_ref(),
            chat.self_uin.as_ref(),
            "self_uin",
        )?;
        let old_analyzed = prior
            .as_ref()
            .and_then(|x| x.2.as_ref())
            .map(|s| s.parse::<Cursor>())
            .transpose()
            .map_err(|_| EngineError::DatabaseFormat("invalid stored cursor".into()))?;
        changed |= tx.execute("INSERT INTO chats(chat_id,kind,display_name,self_uid,self_uin,updated_at) VALUES(?1,?2,?3,?4,?5,?6)
            ON CONFLICT(chat_id) DO UPDATE SET display_name=excluded.display_name,self_uid=excluded.self_uid,self_uin=excluded.self_uin,updated_at=excluded.updated_at
            WHERE chats.display_name IS NOT excluded.display_name OR chats.self_uid IS NOT excluded.self_uid OR chats.self_uin IS NOT excluded.self_uin",
            params![chat.chat_id.as_ref(), wire_string(&chat.kind)?, chat.display_name, uid, uin, now])? > 0;
        if uid.is_none() && uin.is_none() {
            warnings.push(format!(
                "W_SELF_ID_MISSING: {} has no self identity; @all still applies",
                chat.chat_id
            ));
        }
        let (start_inserted, start_duplicate, start_backfilled) = (inserted, duplicate, backfilled);
        for message in &batch.messages {
            if message.chat_id != chat.chat_id {
                return Err(EngineError::Input(
                    "message chat_id does not match batch".into(),
                ));
            }
            seen += 1;
            recalled += i64::from(message.recalled);
            let existing: Option<(String,bool)> = tx.query_row("SELECT message_id,recalled FROM messages WHERE chat_id=?1 AND source_identity=?2",
                params![chat.chat_id.as_ref(),message.source.identity],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((id, was_recalled)) = existing {
                if id != message.id.as_ref() {
                    return Err(EngineError::Input(
                        "source identity has a different stable message id".into(),
                    ));
                }
                duplicate += 1;
                // A later recall is a real business change; stale exports never resurrect it.
                if message.recalled && !was_recalled {
                    let old_body: String = tx.query_row(
                        "SELECT body_json FROM messages WHERE message_id=?1",
                        [&id],
                        |r| r.get(0),
                    )?;
                    let mut updated: UnifiedMessage = serde_json::from_str(&old_body)?;
                    updated.recalled = true;
                    updated.text.clear();
                    updated.mentions.clear();
                    let body = serde_json::to_string(&updated)?;
                    tx.execute("UPDATE messages SET recalled=1,text='',body_json=?2,analysis_state='skipped' WHERE message_id=?1",params![id,body])?;
                    tx.execute("DELETE FROM message_mentions WHERE message_id=?1", [&id])?;
                    changed = true;
                }
                continue;
            }
            tx.execute("INSERT INTO persons(person_id,first_seen) VALUES(?1,?2) ON CONFLICT(person_id) DO UPDATE SET first_seen=min(first_seen,excluded.first_seen)",params![message.sender.as_ref(),message.sent_at.with_timezone(&Utc).to_rfc3339()])?;
            tx.execute("INSERT INTO messages(message_id,chat_id,source_identity,sender,sender_display,sent_at_ms,text,reply_source_id,reply_resolved,recalled,system,body_json,analysis_state)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,NULL,?9,?10,?11,?12)", params![message.id.as_ref(),chat.chat_id.as_ref(),message.source.identity,message.sender.as_ref(),message.sender_display,
                    message.sent_at.timestamp_millis(),message.text,message.reply_to.as_ref().map(|r|&r.source_message_id),message.recalled,message.system,serde_json::to_string(message)?,if message.recalled {"skipped"} else {"pending"}])?;
            let cursor = Cursor {
                sent_at_ms: message.sent_at.timestamp_millis(),
                ordinal: tx.last_insert_rowid(),
            };
            if old_analyzed.is_some_and(|old| cursor < old) {
                backfilled += 1;
            }
            for mention in &message.mentions {
                let (kind, uid, uin) = match &mention.target {
                    MentionTarget::All => ("all", None, None),
                    MentionTarget::User { uid, uin } => ("user", uid.as_deref(), uin.as_deref()),
                    _ => ("unknown", None, None),
                };
                tx.execute("INSERT INTO message_mentions(message_id,target_kind,uid,uin) VALUES(?1,?2,?3,?4)",params![message.id.as_ref(),kind,uid,uin])?;
            }
            inserted += 1;
            changed = true;
        }
        let mut bounds =
            BTreeMap::<&PersonId, (DateTime<FixedOffset>, DateTime<FixedOffset>)>::new();
        for message in &batch.messages {
            bounds
                .entry(&message.sender)
                .and_modify(|(first, last)| {
                    *first = (*first).min(message.sent_at);
                    *last = (*last).max(message.sent_at);
                })
                .or_insert((message.sent_at, message.sent_at));
        }
        for alias in &batch.aliases {
            if let Some((first, last)) = bounds.get(&alias.person_id) {
                changed |= tx.execute("INSERT INTO person_aliases(person_id,chat_id,alias,alias_kind,first_seen,last_seen) VALUES(?1,?2,?3,?4,?5,?6)
                    ON CONFLICT(person_id,chat_id,alias,alias_kind) DO UPDATE SET first_seen=min(first_seen,excluded.first_seen),last_seen=max(last_seen,excluded.last_seen)
                    WHERE excluded.first_seen < first_seen OR excluded.last_seen > last_seen",params![alias.person_id.as_ref(),chat.chat_id.as_ref(),alias.alias,wire_string(&alias.kind)?,first.with_timezone(&Utc).to_rfc3339(),last.with_timezone(&Utc).to_rfc3339()])? > 0;
            }
        }
        changed |= resolve_replies(&tx, &chat.chat_id)?;
        update_cursors(&tx, &chat.chat_id)?;
        warnings.extend(batch.warnings.iter().cloned());
        let import_id = format!(
            "import_{}_{}_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            std::process::id(),
            file_index
        );
        tx.execute("INSERT INTO imports(import_id,file_path,file_hash,chat_id,run_id,seen,inserted,duplicate,backfilled,imported_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![import_id,options.file_paths.get(file_index).map(String::as_str).unwrap_or(""),batch.file_hash,chat.chat_id.as_ref(),options.run_id.as_ref().map(RunId::as_ref).unwrap_or(""),batch.messages.len() as i64,inserted-start_inserted,duplicate-start_duplicate,backfilled-start_backfilled,now])?;
    }
    if backfilled > 0 {
        warnings.push(format!(
            "W_BACKFILL: {backfilled} messages inserted before last_analyzed"
        ));
    }
    tx.commit()?;
    let chat_ids: Vec<_> = ids.into_iter().collect();
    Ok(ImportReport {
        stats: json!({"scope":"import","files":batches.len(),"seen":seen,"inserted":inserted,"duplicate":duplicate,"backfilled":backfilled,"recalled":recalled,"chats":chat_ids.len()}),
        chat_ids,
        changed,
        warnings,
    })
}

fn wire_string(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| EngineError::DatabaseFormat("expected string enum".into()))
}

fn identity(
    prior: Option<String>,
    incoming: Option<&String>,
    metadata: Option<&String>,
    field: &str,
) -> Result<Option<String>> {
    if let (Some(prior), Some(incoming)) = (&prior, incoming)
        && prior != incoming
    {
        return Err(EngineError::Config(format!(
            "{field} conflicts with the stored chat identity"
        )));
    }
    Ok(prior
        .or_else(|| incoming.cloned())
        .or_else(|| metadata.cloned()))
}

fn resolve_replies(connection: &Connection, chat: &ChatId) -> Result<bool> {
    let pairs:Vec<(String,String,String)>=connection.prepare("SELECT m.message_id,m.body_json,target.message_id FROM messages m JOIN messages target ON target.chat_id=m.chat_id AND target.source_identity='qce:'||m.reply_source_id WHERE m.chat_id=?1 AND m.reply_resolved IS NULL AND m.reply_source_id IS NOT NULL")?
        .query_map([chat.as_ref()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<std::result::Result<_,_>>()?;
    for (id, body, target) in &pairs {
        let mut message: UnifiedMessage = serde_json::from_str(body)?;
        if let Some(reply) = &mut message.reply_to {
            reply.resolved = Some(target.as_str().into());
        }
        connection.execute(
            "UPDATE messages SET reply_resolved=?2,body_json=?3 WHERE message_id=?1",
            params![id, target, serde_json::to_string(&message)?],
        )?;
    }
    Ok(!pairs.is_empty())
}

fn update_cursors(connection: &Connection, chat: &ChatId) -> Result<()> {
    let rows:Vec<(Cursor,String)>=connection.prepare("SELECT sent_at_ms,pk,analysis_state FROM messages WHERE chat_id=?1 ORDER BY sent_at_ms,pk")?
        .query_map([chat.as_ref()],|r|Ok((Cursor{sent_at_ms:r.get(0)?,ordinal:r.get(1)?},r.get(2)?)))?.collect::<std::result::Result<_,_>>()?;
    let ingested = rows.last().map(|x| x.0.to_string());
    let analyzed = rows
        .iter()
        .take_while(|(_, state)| state != "pending")
        .last()
        .map(|x| x.0.to_string());
    connection.execute(
        "UPDATE chats SET last_ingested=?2,last_analyzed=?3 WHERE chat_id=?1",
        params![chat.as_ref(), ingested, analyzed],
    )?;
    Ok(())
}

pub fn list_chats(path: &Path) -> Result<Vec<ChatPayload>> {
    let Some(mut connection) = open_read(path)? else {
        return Ok(Vec::new());
    };
    let tx = connection.transaction()?;
    let raw:Vec<ChatRow>=tx.prepare("SELECT chat_id,display_name,kind,last_ingested,last_analyzed,last_reviewed FROM chats ORDER BY chat_id")?
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?.collect::<std::result::Result<_,_>>()?;
    raw.into_iter().map(|(id,name,kind,ingested,analyzed,reviewed)| {
        let reviewed=parse_cursor(reviewed)?;
        let unread: i64=tx.query_row("SELECT count(*) FROM messages WHERE chat_id=?1 AND (?2 IS NULL OR sent_at_ms>?2 OR (sent_at_ms=?2 AND pk>?3))",params![id,reviewed.map(|c|c.sent_at_ms),reviewed.map(|c|c.ordinal)],|r|r.get(0))?;
        let open_p0: i64=tx.query_row("SELECT count(*) FROM insights WHERE chat_id=?1 AND priority='P0' AND lifecycle='open' AND verification_status='verified'",[&id],|r|r.get(0))?;
        Ok(ChatPayload{chat_id:id.into(),display_name:name,kind:serde_json::from_value(json!(kind))?,last_ingested:parse_cursor(ingested)?,last_analyzed:parse_cursor(analyzed)?,last_reviewed:reviewed,unreviewed_messages:unread as u64,open_p0:open_p0 as u64})
    }).collect()
}

fn parse_cursor(text: Option<String>) -> Result<Option<Cursor>> {
    text.map(|s| {
        s.parse()
            .map_err(|_| EngineError::DatabaseFormat("invalid stored cursor".into()))
    })
    .transpose()
}

pub fn list_messages(
    path: &Path,
    chat: &ChatId,
    since: Option<DateTime<FixedOffset>>,
    until: Option<DateTime<FixedOffset>>,
) -> Result<Vec<MessagePayload>> {
    let Some(mut connection) = open_read(path)? else {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    };
    let tx = connection.transaction()?;
    let identity: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT self_uid,self_uin FROM chats WHERE chat_id=?1",
            [chat.as_ref()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((uid, uin)) = identity else {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    };
    let raw:Vec<(i64,String,Option<String>,Option<String>)>=tx.prepare("SELECT m.pk,m.body_json,t.topic_id,t.burst_id FROM messages m LEFT JOIN topic_messages t ON t.message_id=m.message_id WHERE m.chat_id=?1 AND (?2 IS NULL OR m.sent_at_ms>=?2) AND (?3 IS NULL OR m.sent_at_ms<?3) ORDER BY m.sent_at_ms,m.pk")?
        .query_map(params![chat.as_ref(),since.map(ceil_millis),until.map(ceil_millis)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<std::result::Result<_,_>>()?;
    raw.into_iter()
        .map(|(pk, body, topic, burst)| {
            let message: UnifiedMessage = serde_json::from_str(&body)?;
            let mentions_me = !message.recalled
                && message.mentions.iter().any(|m| match &m.target {
                    MentionTarget::All => true,
                    MentionTarget::User {
                        uid: target_uid,
                        uin: target_uin,
                    } => {
                        uid.as_ref()
                            .zip(target_uid.as_ref())
                            .is_some_and(|(a, b)| a == b)
                            || uin
                                .as_ref()
                                .zip(target_uin.as_ref())
                                .is_some_and(|(a, b)| a == b)
                    }
                    _ => false,
                });
            Ok(MessagePayload {
                display_text: render(&message),
                cursor: Cursor {
                    sent_at_ms: message.sent_at.timestamp_millis(),
                    ordinal: pk,
                },
                message_id: message.id,
                sender: message.sender,
                sender_display: message.sender_display,
                sent_at: message.sent_at,
                recalled: message.recalled,
                system: message.system,
                reply_to: message.reply_to,
                mentions_me,
                topic_id: topic.map(Into::into),
                burst_id: burst,
            })
        })
        .collect()
}

// Messages occupy integer milliseconds. Ceiling both interval boundaries keeps
// [since, until) exact even when an RFC3339 argument contains nanoseconds.
fn ceil_millis(time: DateTime<FixedOffset>) -> i64 {
    time.timestamp_millis() + i64::from(!time.timestamp_subsec_nanos().is_multiple_of(1_000_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chat_tldr_qce::{QceOptions, parse_qce_json};

    fn batch() -> ImportBatch {
        parse_qce_json(
            include_bytes!("../../../fixtures/qce/synthetic-group.json"),
            &QceOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn empty_queries_never_create_directories_or_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("absent/chat-tldr.db");
        assert!(list_chats(&path).unwrap().is_empty());
        assert_eq!(inspect(&path).unwrap(), None);
        assert_eq!(
            list_messages(&path, &"qq:group:missing".into(), None, None)
                .unwrap_err()
                .code(),
            "E_CHAT_NOT_FOUND"
        );
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn import_is_idempotent_and_queries_preserve_cursor_and_mentions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("chat-tldr.db");
        let mut input = batch();
        input.chat.self_uid = None;
        input.chat.self_uin = None;
        input.messages[0].mentions = vec![Mention {
            target: MentionTarget::All,
            display: "全体成员".into(),
        }];
        let first = import_batches(&path, &[input.clone()], &ImportOptions::default()).unwrap();
        assert!(first.changed);
        assert_eq!(first.stats["inserted"], 3);
        assert_eq!(inspect(&path).unwrap(), Some(DB_VERSION));
        let messages = list_messages(&path, &input.chat.chat_id, None, None).unwrap();
        assert_eq!(messages.len(), 3);
        assert!(messages[0].mentions_me);
        assert!(messages.windows(2).all(|w| w[0].cursor < w[1].cursor));
        let second = import_batches(&path, &[input.clone()], &ImportOptions::default()).unwrap();
        assert!(!second.changed);
        assert_eq!(second.stats["duplicate"], 3);
        assert_eq!(
            messages,
            list_messages(&path, &input.chat.chat_id, None, None).unwrap()
        );
        let chats = list_chats(&path).unwrap();
        assert_eq!(chats[0].last_ingested, Some(messages[2].cursor));
        assert_eq!(chats[0].last_reviewed, None);
        assert_eq!(chats[0].unreviewed_messages, 3);
        let bounded = list_messages(
            &path,
            &input.chat.chat_id,
            Some(messages[0].sent_at),
            Some(messages[1].sent_at),
        )
        .unwrap();
        assert_eq!(bounded.len(), 1);
        let between = messages[0].sent_at + chrono::Duration::nanoseconds(1);
        assert_eq!(
            list_messages(&path, &input.chat.chat_id, Some(between), None)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            list_messages(&path, &input.chat.chat_id, None, Some(between))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_late_batch_error_rolls_back_every_earlier_batch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("chat-tldr.db");
        let original = batch();
        let options = ImportOptions {
            self_uid: Some("u_owner".into()),
            ..Default::default()
        };
        import_batches(&path, std::slice::from_ref(&original), &options).unwrap();
        let mut other = original.clone();
        other.chat.chat_id = "qq:group:new".into();
        for (i, message) in other.messages.iter_mut().enumerate() {
            message.chat_id = other.chat.chat_id.clone();
            message.id = format!("m_new_{i}").into();
        }
        let conflicting = ImportOptions {
            self_uid: Some("u_different".into()),
            ..Default::default()
        };
        let result = import_batches(&path, &[other, original.clone()], &conflicting);
        assert_eq!(result.err().unwrap().code(), "E_CONFIG");
        assert_eq!(list_chats(&path).unwrap().len(), 1);
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM imports", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        let mut changed_metadata = original;
        changed_metadata.chat.self_uid = Some("u_metadata".into());
        assert!(
            !import_batches(&path, &[changed_metadata], &ImportOptions::default())
                .unwrap()
                .changed
        );
    }

    #[test]
    fn replies_resolve_after_later_import_and_recalled_messages_do_not_resurrect() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("chat-tldr.db");
        let source = batch();
        let mut reply = source.clone();
        reply.messages = vec![source.messages[1].clone()];
        reply.messages[0].reply_to = Some(ReplyRef {
            source_message_id: source.messages[0].source.qce_id.clone().unwrap(),
            resolved: None,
        });
        import_batches(&path, &[reply.clone()], &ImportOptions::default()).unwrap();
        assert!(
            list_messages(&path, &source.chat.chat_id, None, None).unwrap()[0]
                .reply_to
                .as_ref()
                .unwrap()
                .resolved
                .is_none()
        );
        let mut target = source.clone();
        target.messages = vec![source.messages[0].clone()];
        import_batches(&path, &[target], &ImportOptions::default()).unwrap();
        let result = list_messages(&path, &source.chat.chat_id, None, None).unwrap();
        assert_eq!(
            result[1].reply_to.as_ref().unwrap().resolved,
            Some(source.messages[0].id.clone())
        );
        let mut recalled = reply.clone();
        let original_cursor = result[1].cursor;
        recalled.messages[0].sent_at += chrono::Duration::days(1);
        recalled.messages[0].recalled = true;
        recalled.messages[0].text.clear();
        recalled.messages[0].mentions.clear();
        assert!(
            import_batches(&path, &[recalled], &ImportOptions::default())
                .unwrap()
                .changed
        );
        assert!(
            !import_batches(&path, &[reply], &ImportOptions::default())
                .unwrap()
                .changed
        );
        let result = list_messages(&path, &source.chat.chat_id, None, None).unwrap();
        assert!(result[1].recalled);
        assert_eq!(result[1].cursor, original_cursor);
        assert_eq!(result[1].display_text, "");
        assert_eq!(
            result[1].reply_to.as_ref().unwrap().resolved,
            Some(source.messages[0].id.clone())
        );
    }

    #[test]
    fn backfill_rewinds_analyzed_prefix_without_marking_read() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("chat-tldr.db");
        let source = batch();
        let mut tail = source.clone();
        tail.messages = vec![source.messages[2].clone()];
        import_batches(&path, &[tail], &ImportOptions::default()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE messages SET analysis_state='done'", [])
            .unwrap();
        update_cursors(&conn, &source.chat.chat_id).unwrap();
        assert!(list_chats(&path).unwrap()[0].last_analyzed.is_some());
        let report = import_batches(&path, &[source], &ImportOptions::default()).unwrap();
        assert_eq!(report.stats["backfilled"], 2);
        let chat = &list_chats(&path).unwrap()[0];
        assert_eq!(chat.last_analyzed, None);
        assert_eq!(chat.last_reviewed, None);
    }

    #[test]
    fn newer_database_is_rejected_without_rewriting_version() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("chat-tldr.db");
        import_batches(&path, &[batch()], &ImportOptions::default()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value='999' WHERE key='db_version'", [])
            .unwrap();
        assert_eq!(inspect(&path).unwrap_err().code(), "E_DB");
        assert!(import_batches(&path, &[batch()], &ImportOptions::default()).is_err());
        assert_eq!(
            conn.query_row("SELECT value FROM meta WHERE key='db_version'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "999"
        );
    }
}
