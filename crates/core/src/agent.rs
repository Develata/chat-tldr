use serde::{Deserialize, Serialize};

use crate::{ChatId, Cursor, InsightId, TopicId};

wire_enum!(FinishReason {
    Done,
    MaxSteps,
    BudgetExceeded,
    Cancelled,
    Error
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AgentAction {
    AnalyzeDirect {
        chat_id: ChatId,
        range: MessageRange,
    },
    Segment {
        chat_id: ChatId,
        range: MessageRange,
    },
    AnalyzeTopic {
        topic_id: TopicId,
    },
    Verify {
        insight_ids: Vec<InsightId>,
    },
    MergeTopics {
        into: TopicId,
        from: Vec<TopicId>,
    },
    Finish {
        reason: FinishReason,
    },
    #[serde(other)]
    Unknown,
}

/// A cursor interval (after, up_to], distinct from CLI time filters [since, until).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRange {
    pub after: Option<Cursor>,
    pub up_to: Cursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentObservation {
    pub pending_messages: u32,
    pub interleave: f32,
    pub active_topics: u32,
    pub dirty_topics: u32,
    pub pending_verification: u32,
    pub merge_candidates: u32,
    pub steps_taken: u32,
    pub cost_usd: f64,
}
