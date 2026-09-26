use super::{InboxOptions, inbox::read_inbox, open_read};
use crate::{EngineError, Result};
use chat_tldr_core::{ChatId, ChatMeta, Overview};
use chrono::{DateTime, FixedOffset};
use std::path::Path;

pub struct OverviewOptions {
    pub since: DateTime<FixedOffset>,
    pub until: DateTime<FixedOffset>,
    pub now: DateTime<FixedOffset>,
}

/// One read transaction for membership, evidence, feedback, identity and statistics.
pub fn overview(path: &Path, chat: &ChatId, options: &OverviewOptions) -> Result<Overview> {
    if options.since >= options.until {
        return Err(EngineError::Usage("--since must precede --until".into()));
    }
    let Some(mut connection) = open_read(path)? else {
        return Err(EngineError::ChatNotFound(chat.to_string()));
    };
    let tx = connection.transaction()?;
    let data = read_inbox(
        &tx,
        chat,
        &InboxOptions {
            all: true,
            include_resolved: false,
            include_rejected: false,
            now: options.until,
        },
        true,
    )?;
    let meta = tx.query_row(
        "SELECT kind,display_name,self_uid,self_uin FROM chats WHERE chat_id=?1",
        [chat.as_ref()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        },
    )?;
    let identity = ChatMeta {
        chat_id: chat.clone(),
        kind: serde_json::from_value(serde_json::json!(meta.0))?,
        display_name: meta.1,
        self_uid: meta.2,
        self_uin: meta.3,
    };
    Ok(crate::overview::build(data, &identity, options))
}
