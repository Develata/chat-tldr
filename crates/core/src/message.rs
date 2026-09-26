use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::{ChatId, MessageId, PersonId};

wire_enum!(ChatKind { Group, Private });
wire_enum!(AliasKind {
    Name,
    Nickname,
    GroupCard,
    Remark
});
wire_enum!(AttachmentKind {
    Image,
    Video,
    Audio,
    File,
    Other
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportBatch {
    pub chat: ChatMeta,
    pub messages: Vec<UnifiedMessage>,
    pub aliases: Vec<AliasObservation>,
    pub warnings: Vec<String>,
    pub file_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMeta {
    pub chat_id: ChatId,
    pub kind: ChatKind,
    pub display_name: String,
    pub self_uid: Option<String>,
    pub self_uin: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasObservation {
    pub person_id: PersonId,
    pub alias: String,
    pub kind: AliasKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnifiedMessage {
    pub id: MessageId,
    pub chat_id: ChatId,
    pub sender: PersonId,
    pub sender_display: String,
    pub sent_at: DateTime<FixedOffset>,
    pub text: String,
    pub mentions: Vec<Mention>,
    pub reply_to: Option<ReplyRef>,
    pub attachments: Vec<Attachment>,
    pub forward: Option<ForwardBundle>,
    pub recalled: bool,
    pub system: bool,
    pub source: SourceMeta,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mention {
    pub target: MentionTarget,
    pub display: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MentionTarget {
    User {
        uid: Option<String>,
        uin: Option<String>,
    },
    All,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyRef {
    pub source_message_id: String,
    pub resolved: Option<MessageId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub name: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardBundle {
    pub title: String,
    pub messages: Vec<ForwardedMessage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardedMessage {
    pub sender_display: String,
    pub sent_at: Option<DateTime<FixedOffset>>,
    pub text: String,
    pub forward: Option<Box<ForwardBundle>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMeta {
    pub format: String,
    pub identity: String,
    pub qce_id: Option<String>,
    pub qce_seq: Option<String>,
    pub qce_type: Option<String>,
    pub file_hash: String,
}
