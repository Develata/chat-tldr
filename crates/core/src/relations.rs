//! Evidence-backed semantic links; separate from user-controlled insight lifecycle.
use crate::{ChatId, Evidence, RunId, TopicId, VerificationStatus};
use serde::{Deserialize, Serialize};

wire_enum!(RelationKind {
    Question,
    Answers,
    Replaces,
    Cancels,
    Conflicts
});
wire_enum!(AnswerCompleteness { Full, Partial });
wire_enum!(QuestionStatus {
    Pending,
    PartiallyAnswered,
    Answered
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticRelation {
    pub id: String,
    pub chat_id: ChatId,
    pub topic_id: TopicId,
    pub kind: RelationKind,
    pub source: Evidence,
    pub target: Option<Evidence>,
    pub answer_completeness: Option<AnswerCompleteness>,
    pub verification_status: VerificationStatus,
    pub created_in_run: RunId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuestionState {
    pub question: SemanticRelation,
    pub status: QuestionStatus,
    pub answer_ids: Vec<String>,
}
