use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::{ChatId, InsightId, MessageId, RenderProfile, RunId, TopicId};

wire_enum!(InsightKind {
    MentionMe,
    Todo,
    Announcement,
    Decision,
    TopicSummary
});
wire_enum!(Assignee { Me, All, Other });
wire_enum!(VerificationStatus {
    Verified,
    Unverified,
    Rejected
});
wire_enum!(Lifecycle {
    Open,
    Done,
    Dismissed
});
wire_enum!(TemporalRelation { Before, At, After });
wire_enum!(Granularity {
    Minute,
    Hour,
    HalfDay,
    Day,
    Week
});
wire_enum!(NormalizedBy { Rule, Llm, None });

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
    #[serde(other, rename = "unknown")]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Insight {
    pub id: InsightId,
    pub chat_id: ChatId,
    pub kind: InsightKind,
    pub title: String,
    pub summary: String,
    pub priority: Priority,
    pub rank_score: f32,
    pub confidence: Option<f32>,
    pub assignee: Assignee,
    pub deadline: Option<TemporalConstraint>,
    pub evidence: Vec<Evidence>,
    pub topic_id: Option<TopicId>,
    pub verification_status: VerificationStatus,
    pub lifecycle: Lifecycle,
    pub created_in_run: RunId,
    pub updated_at: DateTime<FixedOffset>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalConstraint {
    pub raw: String,
    pub relation: TemporalRelation,
    pub bound_date: Option<NaiveDate>,
    pub bound_time: Option<NaiveTime>,
    pub granularity: Granularity,
    pub anchor: DateTime<FixedOffset>,
    pub normalized_by: NormalizedBy,
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub message_id: MessageId,
    pub quote: String,
    pub render_profile: RenderProfile,
}
