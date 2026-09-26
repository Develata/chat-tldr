//! Pure adapter for single-file QCE JSON exports; no file or network access.

mod input;
mod normalize;

use chat_tldr_core::{
    AliasKind, AliasObservation, ChatId, ChatKind, ChatMeta, ImportBatch, MessageId, PersonId,
    SourceMeta, UnifiedMessage,
};
use chrono::{DateTime, FixedOffset};
use input::{Export, RawMessage};

/// Import-time normalization options. At most two forwarded-message levels are kept.
#[derive(Debug, Clone)]
pub struct QceOptions {
    pub timezone: FixedOffset,
    pub max_forward_depth: u8,
}

impl Default for QceOptions {
    fn default() -> Self {
        Self {
            timezone: FixedOffset::east_opt(8 * 3600).expect("constant valid offset"),
            max_forward_depth: 2,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QceError {
    #[error("invalid QCE JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("missing or invalid QCE field: {0}")]
    Field(String),
    #[error("unsupported QCE chat type (expected group or private)")]
    ChatType,
    #[error("QCE chunked JSONL manifests are unsupported; export a single JSON file")]
    UnsupportedExport,
    #[error("max_forward_depth must be between 0 and 2")]
    ForwardDepth,
}

/// Parse one complete export. Missing identity/time information is an error;
/// optional metadata and unknown fields are tolerated without inventing identity.
pub fn parse_qce_json(bytes: &[u8], opts: &QceOptions) -> Result<ImportBatch, QceError> {
    if opts.max_forward_depth > 2 {
        return Err(QceError::ForwardDepth);
    }
    let raw: Export = match serde_json::from_slice(bytes) {
        Ok(raw) => raw,
        Err(error) => {
            // Inspect only the failure path: successful single-file imports
            // should not allocate a second complete JSON tree.
            if serde_json::from_slice::<serde_json::Value>(bytes)
                .ok()
                .is_some_and(|value| is_chunked_manifest(&value))
            {
                return Err(QceError::UnsupportedExport);
            }
            return Err(QceError::Json(error));
        }
    };
    let kind = match raw.chat_info.kind.as_str() {
        "group" => ChatKind::Group,
        "private" => ChatKind::Private,
        _ => return Err(QceError::ChatType),
    };
    let peer = input::nonempty(raw.chat_info.peer_uid.as_deref())
        .or_else(|| input::nonempty(raw.chat_info.peer_uin.as_deref()))
        .ok_or_else(|| QceError::Field("chatInfo.peerUid/peerUin".into()))?;
    let chat_id = ChatId::from(format!("qq:{}:{peer}", raw.chat_info.kind));
    let file_hash = format!("blake3:{}", blake3::hash(bytes).to_hex());
    let chat = ChatMeta {
        chat_id,
        kind,
        display_name: raw.chat_info.name,
        self_uid: input::nonempty(raw.chat_info.self_uid.as_deref()).map(str::to_owned),
        self_uin: input::nonempty(raw.chat_info.self_uin.as_deref()).map(str::to_owned),
    };
    let mut messages = Vec::with_capacity(raw.messages.len());
    let mut aliases = Vec::new();
    let mut warnings = Vec::new();
    for (index, msg) in raw.messages.into_iter().enumerate() {
        messages.push(normalize_message(
            msg,
            index,
            &chat,
            &file_hash,
            opts,
            &mut aliases,
            &mut warnings,
        )?);
    }
    // Stable sort preserves file order for messages sharing a timestamp.
    messages.sort_by_key(|m| m.sent_at.timestamp_millis());
    Ok(ImportBatch {
        chat,
        messages,
        aliases,
        warnings,
        file_hash,
    })
}

fn is_chunked_manifest(value: &serde_json::Value) -> bool {
    // Exact discriminator and shape from upstream 7fcca888, json_exporter.rs
    // ChunkedJsonlManifest / ChunkedSection. Filenames alone are not evidence.
    let Some(chunked) = value.get("chunked") else {
        return false;
    };
    value.get("messages").is_none()
        && ["metadata", "chatInfo", "statistics"]
            .iter()
            .all(|key| value.get(key).is_some_and(serde_json::Value::is_object))
        && chunked.get("format").and_then(serde_json::Value::as_str) == Some("jsonl")
        && ["chunksDir", "chunkFileExt"]
            .iter()
            .all(|key| chunked.get(key).is_some_and(serde_json::Value::is_string))
        && chunked
            .get("chunks")
            .is_some_and(serde_json::Value::is_array)
}

fn normalize_message(
    raw: RawMessage,
    index: usize,
    chat: &ChatMeta,
    file_hash: &str,
    opts: &QceOptions,
    aliases: &mut Vec<AliasObservation>,
    warnings: &mut Vec<String>,
) -> Result<UnifiedMessage, QceError> {
    let path = format!("messages[{index}]");
    let timestamp = raw
        .timestamp
        .ok_or_else(|| QceError::Field(format!("{path}.timestamp")))?;
    let sent_at = DateTime::from_timestamp_millis(timestamp)
        .ok_or_else(|| QceError::Field(format!("{path}.timestamp")))?
        .with_timezone(&opts.timezone);
    let sender_uid = input::nonempty(raw.sender.uid.as_deref())
        .filter(|uid| !matches!(*uid, "unknown" | "未知" | "0"));
    let uid = match sender_uid {
        Some(uid) => uid,
        None if raw.system => "system",
        None => return Err(QceError::Field(format!("{path}.sender.uid"))),
    };
    let sender = PersonId::from(format!("qq:{uid}"));
    for (alias, kind) in [
        (Some(raw.sender.name.as_str()), AliasKind::Name),
        (raw.sender.nickname.as_deref(), AliasKind::Nickname),
        (raw.sender.group_card.as_deref(), AliasKind::GroupCard),
        (raw.sender.remark.as_deref(), AliasKind::Remark),
    ] {
        if let Some(alias) = input::nonempty(alias) {
            aliases.push(AliasObservation {
                person_id: sender.clone(),
                alias: alias.to_owned(),
                kind,
            });
        }
    }
    let qce_id = input::nonempty(raw.id.as_deref()).map(str::to_owned);
    let qce_seq = input::nonempty(raw.seq.as_deref()).map(str::to_owned);
    let identity = if let Some(id) = &qce_id {
        format!("qce:{id}")
    } else if let Some(seq) = &qce_seq {
        format!("seq:{seq}@{timestamp}")
    } else {
        let key = serde_json::to_vec(&(uid, timestamp, &raw.content.text))?;
        warnings.push(format!(
            "{path}: missing id and seq; using content hash identity"
        ));
        format!("hash:{}", blake3::hash(&key).to_hex())
    };
    let digest = blake3::hash(format!("{}\n{identity}", chat.chat_id).as_bytes());
    let id = MessageId::from(format!("m_{}", &digest.to_hex()[..16]));
    // A recalled export can still carry its old content; discard all of it.
    let content = if raw.recalled {
        normalize::Content::default()
    } else {
        normalize::content(&raw.content, 0, opts, warnings, &path)?
    };
    Ok(UnifiedMessage {
        id,
        chat_id: chat.chat_id.clone(),
        sender,
        sender_display: raw.sender.name,
        sent_at,
        text: content.text,
        mentions: content.mentions,
        reply_to: content.reply_to,
        attachments: content.attachments,
        forward: content.forward,
        recalled: raw.recalled,
        system: raw.system,
        source: SourceMeta {
            format: "qce-json".into(),
            identity,
            qce_id,
            qce_seq,
            qce_type: input::nonempty(raw.kind.as_deref()).map(str::to_owned),
            file_hash: file_hash.to_owned(),
        },
    })
}
