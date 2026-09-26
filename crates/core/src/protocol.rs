use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeMap};
use serde_json::Value;

use crate::{
    AgentAction, AgentObservation, ChatId, ChatKind, Cursor, FinishReason, Insight, MessageId,
    PersonId, ReplyRef, RunId, SCHEMA_VERSION, TopicId,
};

wire_enum!(DecisionMethod {
    Rule,
    Jev,
    Fallback
});
wire_enum!(TopicState {
    Active,
    Closed,
    Merged
});
wire_enum!(RunStatus {
    Complete,
    Partial,
    Failed,
    Cancelled
});
wire_enum!(SubjectKind {
    Message,
    Burst,
    TopicPair,
    Controller
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CliEvent {
    pub schema_version: String,
    pub run_id: RunId,
    pub seq: u64,
    #[serde(flatten)]
    pub body: EventBody,
}

impl CliEvent {
    pub fn new(run_id: RunId, seq: u64, body: EventBody) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            run_id,
            seq,
            body,
        }
    }
}

// Dispatch on the event name before parsing its payload. An untagged catch-all
// would accidentally swallow malformed *known* events as unknown events.
macro_rules! event_body {
    ($($variant:ident($payload:ty) => $name:literal),+ $(,)?) => {
        #[derive(Clone, Debug, PartialEq)]
        pub enum EventBody {
            $($variant($payload),)+
            Unknown { event: String, payload: Value },
        }

        impl EventBody {
            pub fn name(&self) -> &str {
                match self {
                    $(Self::$variant(_) => $name,)+
                    Self::Unknown { event, .. } => event,
                }
            }
        }

        impl Serialize for EventBody {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("event", self.name())?;
                match self {
                    $(Self::$variant(payload) => map.serialize_entry("payload", payload)?,)+
                    Self::Unknown { payload, .. } => map.serialize_entry("payload", payload)?,
                }
                map.end()
            }
        }

        impl<'de> Deserialize<'de> for EventBody {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                struct WireBody { event: String, payload: Value }
                let raw = WireBody::deserialize(deserializer)?;
                if !raw.payload.is_object() {
                    return Err(de::Error::custom("event payload must be a JSON object"));
                }
                match raw.event.as_str() {
                    $($name => serde_json::from_value(raw.payload)
                        .map(Self::$variant).map_err(de::Error::custom),)+
                    _ => Ok(Self::Unknown { event: raw.event, payload: raw.payload }),
                }
            }
        }
    };
}

event_body! {
    Progress(ProgressPayload) => "progress",
    Decision(DecisionPayload) => "decision",
    Chat(ChatPayload) => "chat",
    Message(MessagePayload) => "message",
    Topic(TopicPayload) => "topic",
    Insight(InsightPayload) => "insight",
    Inbox(InboxPayload) => "inbox",
    Stats(StatsPayload) => "stats",
    Ack(AckPayload) => "ack",
    JevAnswer(JevAnswerPayload) => "jev_answer",
    Warning(WarningPayload) => "warning",
    Error(ErrorPayload) => "error",
    Done(DonePayload) => "done",
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressPayload {
    pub stage: String,
    pub current: u64,
    pub total: Option<u64>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionPayload {
    pub step: u32,
    pub observation: AgentObservation,
    pub allowed: Vec<AgentAction>,
    pub chosen: AgentAction,
    pub method: DecisionMethod,
    pub probabilities: Option<BTreeMap<String, f32>>,
    pub confidence: Option<f32>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatPayload {
    pub chat_id: ChatId,
    pub display_name: String,
    pub kind: ChatKind,
    pub last_ingested: Option<Cursor>,
    pub last_analyzed: Option<Cursor>,
    pub last_reviewed: Option<Cursor>,
    pub unreviewed_messages: u64,
    pub open_p0: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessagePayload {
    pub message_id: MessageId,
    pub sender: PersonId,
    pub sender_display: String,
    pub sent_at: DateTime<FixedOffset>,
    pub display_text: String,
    pub recalled: bool,
    pub system: bool,
    pub reply_to: Option<ReplyRef>,
    pub mentions_me: bool,
    pub topic_id: Option<TopicId>,
    pub burst_id: Option<String>,
    pub cursor: Cursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicPayload {
    pub topic_id: TopicId,
    pub chat_id: ChatId,
    pub title: String,
    pub title_is_provisional: bool,
    pub state: TopicState,
    pub message_count: u64,
    pub first_message_at: DateTime<FixedOffset>,
    pub last_message_at: DateTime<FixedOffset>,
    pub is_chitchat: Option<f32>,
    pub merged_into: Option<TopicId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InsightPayload {
    pub insight: Insight,
    pub evidence_view: Vec<EvidenceView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceView {
    pub message_id: MessageId,
    pub sender_display: String,
    pub sent_at: DateTime<FixedOffset>,
    pub display_text: String,
    /// Unicode scalar indices, left inclusive and right exclusive (not bytes).
    pub highlight: Option<[usize; 2]>,
    pub ok: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboxPayload {
    pub chat_id: ChatId,
    /// Last position in a contiguous done/skipped prefix; null disables mark-read.
    pub view_cursor: Option<Cursor>,
    pub last_reviewed: Option<Cursor>,
    pub counts: PriorityCounts,
    pub rejected_insights: u64,
    pub generated_at: DateTime<FixedOffset>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityCounts {
    #[serde(rename = "P0")]
    pub p0: u64,
    #[serde(rename = "P1")]
    pub p1: u64,
    #[serde(rename = "P2")]
    pub p2: u64,
    #[serde(rename = "P3")]
    pub p3: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum StatsPayload {
    Import(ImportStats),
    Run(RunStats),
    Global(GlobalStats),
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportStats {
    pub files: u64,
    pub seen: u64,
    pub inserted: u64,
    pub duplicate: u64,
    pub backfilled: u64,
    pub recalled: u64,
    pub chats: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunStats {
    pub run_id: RunId,
    pub chat_id: ChatId,
    pub messages_analyzed: u64,
    pub topics_created: u64,
    pub topics_updated: u64,
    pub insights: InsightStats,
    pub usage: Vec<UsageStats>,
    pub cost_usd: f64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightStats {
    pub created: u64,
    pub updated: u64,
    pub verified: u64,
    pub unverified: u64,
    pub rejected: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageStats {
    pub stage: String,
    pub provider: String,
    pub model: String,
    pub calls: u64,
    pub cache_hits: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// The detailed global count set is extensible; count keys are table names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GlobalStats {
    pub counts: BTreeMap<String, u64>,
    pub usage: Vec<UsageStats>,
    pub cost_usd: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AckPayload {
    pub command: String,
    pub target: Option<String>,
    pub changed: bool,
    pub detail: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarningPayload {
    pub stage: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub stage: String,
    pub code: String,
    pub retryable: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<TopicId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DonePayload {
    pub status: RunStatus,
    pub exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JevAnswerPayload {
    pub model: String,
    pub request_key: String,
    pub question_id: String,
    pub qtype: String,
    pub subject: AnswerSubject,
    /// Provider-specific noul/choice distributions, retained for calibration.
    pub answer: Value,
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerSubject {
    pub kind: SubjectKind,
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_ids: Vec<MessageId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<TopicId>,
}

/// Shared subprocess framing checks for GUI and evaluation clients.
#[derive(Debug, Default)]
pub struct EventStreamValidator {
    run_id: Option<RunId>,
    next_seq: u64,
    done_exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("unsupported or invalid schema version: {0}")]
    SchemaVersion(String),
    #[error("expected sequence {expected}, received {actual}")]
    Sequence { expected: u64, actual: u64 },
    #[error("run_id changed within one invocation")]
    RunChanged,
    #[error("received an event after done")]
    AfterDone,
    #[error("stream ended without done")]
    MissingDone,
    #[error("done status {status:?} contradicts exit code {exit_code}")]
    DoneStatus { status: RunStatus, exit_code: i32 },
    #[error("process exit code {actual} differs from done exit code {expected}")]
    ExitCode { expected: i32, actual: i32 },
}

impl EventStreamValidator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accept(&mut self, event: &CliEvent) -> Result<(), ProtocolError> {
        if self.done_exit_code.is_some() {
            return Err(ProtocolError::AfterDone);
        }
        let valid_version = event
            .schema_version
            .split_once('.')
            .is_some_and(|(major, minor)| {
                major == "1" && !minor.is_empty() && minor.bytes().all(|byte| byte.is_ascii_digit())
            });
        if !valid_version {
            return Err(ProtocolError::SchemaVersion(event.schema_version.clone()));
        }
        if event.seq != self.next_seq {
            return Err(ProtocolError::Sequence {
                expected: self.next_seq,
                actual: event.seq,
            });
        }
        if self
            .run_id
            .as_ref()
            .is_some_and(|run_id| *run_id != event.run_id)
        {
            return Err(ProtocolError::RunChanged);
        }
        if let EventBody::Done(done) = &event.body {
            let valid_status = match done.status {
                RunStatus::Complete => done.exit_code == 0,
                RunStatus::Partial => done.exit_code == 6,
                RunStatus::Cancelled => done.exit_code == 130,
                RunStatus::Failed => !matches!(done.exit_code, 0 | 6 | 130),
                // A future status is a valid frame, but clients must display it
                // as unknown rather than interpret it as complete success.
                RunStatus::Unknown => true,
            };
            if !valid_status {
                return Err(ProtocolError::DoneStatus {
                    status: done.status,
                    exit_code: done.exit_code,
                });
            }
            self.done_exit_code = Some(done.exit_code);
        }
        self.run_id.get_or_insert_with(|| event.run_id.clone());
        self.next_seq += 1;
        Ok(())
    }

    pub fn finish(&self, process_exit_code: i32) -> Result<(), ProtocolError> {
        let expected = self.done_exit_code.ok_or(ProtocolError::MissingDone)?;
        if expected != process_exit_code {
            return Err(ProtocolError::ExitCode {
                expected,
                actual: process_exit_code,
            });
        }
        Ok(())
    }
}
