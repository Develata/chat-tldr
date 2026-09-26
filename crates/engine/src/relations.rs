//! Model proposals are resolved only through the supplied extraction context.
use crate::extract::{ExtractedEvidence, ExtractionContext};
use chat_tldr_core::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExtractedRelation {
    #[schemars(with = "KindSchema")]
    pub kind: String,
    pub source: ExtractedEvidence,
    pub target: Option<ExtractedEvidence>,
    #[schemars(with = "Option<CompletenessSchema>")]
    pub answer_completeness: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum KindSchema {
    Question,
    Answers,
    Replaces,
    Cancels,
    Conflicts,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum CompletenessSchema {
    Full,
    Partial,
}

#[derive(JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum RelationSchema {
    Question {
        source: ExtractedEvidence,
        target: (),
        answer_completeness: (),
    },
    Answers {
        source: ExtractedEvidence,
        target: ExtractedEvidence,
        answer_completeness: CompletenessSchema,
    },
    Replaces {
        source: ExtractedEvidence,
        target: ExtractedEvidence,
        answer_completeness: (),
    },
    Cancels {
        source: ExtractedEvidence,
        target: ExtractedEvidence,
        answer_completeness: (),
    },
    Conflicts {
        source: ExtractedEvidence,
        target: ExtractedEvidence,
        answer_completeness: (),
    },
}

impl ExtractedRelation {
    pub fn evidence(&self) -> impl Iterator<Item = &ExtractedEvidence> {
        std::iter::once(&self.source).chain(self.target.iter())
    }

    pub fn validate(&self, context: &ExtractionContext) -> Result<(), String> {
        let shape = match self.kind.as_str() {
            "question" => self.target.is_none() && self.answer_completeness.is_none(),
            "answers" => {
                self.target.is_some()
                    && matches!(
                        self.answer_completeness.as_deref(),
                        Some("full" | "partial")
                    )
            }
            "replaces" | "cancels" | "conflicts" => {
                self.target.is_some() && self.answer_completeness.is_none()
            }
            _ => false,
        };
        if !shape {
            return Err("question requires target=null and answer_completeness=null; answers requires target evidence and full/partial; replaces/cancels/conflicts requires target evidence and answer_completeness=null".into());
        }
        if self
            .evidence()
            .any(|e| !context.messages.contains_key(&e.reference))
        {
            return Err("relation contains an unknown message ref".into());
        }
        Ok(())
    }
}

pub fn proposals(
    rows: &[ExtractedRelation],
    context: &ExtractionContext,
    chat: &ChatId,
    topic: &TopicId,
    run: &RunId,
) -> Vec<SemanticRelation> {
    let evidence = |e: &ExtractedEvidence| Evidence {
        message_id: context.messages[&e.reference].id.clone(),
        quote: e.quote.clone(),
        render_profile: RenderProfile::default(),
    };
    let mut result = std::collections::BTreeMap::new();
    for row in rows {
        // validate_topic already checked refs and the wire enum spellings.
        let mut relation = SemanticRelation {
            id: String::new(),
            chat_id: chat.clone(),
            topic_id: topic.clone(),
            kind: serde_json::from_value(serde_json::json!(row.kind)).expect("validated kind"),
            source: evidence(&row.source),
            target: row.target.as_ref().map(evidence),
            answer_completeness: row.answer_completeness.as_ref().map(|v| {
                serde_json::from_value(serde_json::json!(v)).expect("validated completeness")
            }),
            verification_status: VerificationStatus::Unverified,
            created_in_run: run.clone(),
        };
        relation.id = identity(&relation);
        result.insert(relation.id.clone(), relation.clone());
        // An explicit answers proposal also identifies its question. It is still
        // subject to the same source/quote validation at commit and query time.
        if relation.kind == RelationKind::Answers {
            relation.kind = RelationKind::Question;
            relation.target = None;
            relation.answer_completeness = None;
            relation.id = identity(&relation);
            result.insert(relation.id.clone(), relation);
        }
    }
    result.into_values().collect()
}

pub fn identity(row: &SemanticRelation) -> String {
    let bytes = serde_json::to_vec(&(
        &row.chat_id,
        row.kind,
        &row.source,
        &row.target,
        row.answer_completeness,
    ))
    .expect("relation identity serializes");
    format!("rel_{}", blake3::hash(&bytes).to_hex())
}

/// Quote validity is a provenance check, not a semantic accuracy claim.
pub fn valid(
    row: &SemanticRelation,
    sources: &std::collections::BTreeMap<MessageId, (UnifiedMessage, Cursor)>,
) -> bool {
    let check = |e: &Evidence| {
        sources.get(&e.message_id).filter(|(m, _)| {
            m.chat_id == row.chat_id
                && !m.recalled
                && e.render_profile == RenderProfile::default()
                && e.quote
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .take(3)
                    .count()
                    == 3
                && crate::verify::find_quote(&crate::render::render(m), &e.quote).is_some()
        })
    };
    let Some((_, source_order)) = check(&row.source) else {
        return false;
    };
    match row.kind {
        RelationKind::Question => row.target.is_none() && row.answer_completeness.is_none(),
        RelationKind::Answers
        | RelationKind::Replaces
        | RelationKind::Cancels
        | RelationKind::Conflicts => {
            let Some((_, target_order)) = row.target.as_ref().and_then(check) else {
                return false;
            };
            source_order < target_order
                && match row.kind {
                    RelationKind::Answers => matches!(
                        row.answer_completeness,
                        Some(AnswerCompleteness::Full | AnswerCompleteness::Partial)
                    ),
                    _ => row.answer_completeness.is_none(),
                }
        }
        _ => false,
    }
}
