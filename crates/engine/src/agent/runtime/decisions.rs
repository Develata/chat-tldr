//! Typed decisions share validation, cache, budget and audit handling across stages.
use super::*;
use crate::decider::DecisionResponse;

#[derive(Clone, Copy)]
enum Stage {
    Decide,
    TopicReview,
    Controller,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Decide => "decide",
            Self::TopicReview => "topic_review",
            Self::Controller => "controller",
        }
    }
}

impl Runtime<'_, '_> {
    pub(super) fn decide_controller(
        &mut self,
        request: &DecisionRequest,
        subjects: &BTreeMap<String, AnswerSubject>,
    ) -> Result<DecisionResponse> {
        self.decide_stage(request, subjects, Stage::Controller)
    }

    pub(super) fn decide(
        &mut self,
        request: &DecisionRequest,
        subjects: &BTreeMap<String, AnswerSubject>,
    ) -> Result<DecisionResponse> {
        self.decide_stage(request, subjects, Stage::Decide)
    }

    pub(super) fn review_topic(
        &mut self,
        request: &DecisionRequest,
        subjects: &BTreeMap<String, AnswerSubject>,
    ) -> Result<DecisionResponse> {
        self.decide_stage(request, subjects, Stage::TopicReview)
    }

    fn decide_stage(
        &mut self,
        request: &DecisionRequest,
        subjects: &BTreeMap<String, AnswerSubject>,
        stage: Stage,
    ) -> Result<DecisionResponse> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        request.validate()?;
        if !request.questions.keys().eq(subjects.keys()) {
            return Err(EngineError::Input(
                "decision questions are missing their source mapping".into(),
            ));
        }
        // A review uses the LLM for this request only. Successful Jev decisions
        // afterwards must not be mistaken for a permanent provider fallback.
        let primary = if self.fallback_active || matches!(stage, Stage::TopicReview) {
            None
        } else {
            self.models.primary
        };
        let decider = primary.unwrap_or(self.models.fallback);
        let provider = if primary.is_some() { "typesafe" } else { "llm" };
        let config = if primary.is_some() {
            &self.config.jev
        } else {
            &self.config.llm
        };
        let prepared = if primary.is_none() {
            Some(crate::decider::LlmDecider::new(self.models.llm, &config.model).prepare(request)?)
        } else {
            None
        };
        let prompt = match &prepared {
            Some(value) => serde_json::to_string(value)?,
            None => serde_json::to_string(request)?,
        };
        let key = blake3::hash(
            format!(
                "{}-v1:r1:{provider}:{config:?}:{}:{prompt}",
                stage.name(),
                decider.name()
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        let cached = match self.session.cache_get::<DecisionResponse>(&key) {
            Ok(value) => value,
            Err(EngineError::Json(_)) => {
                self.session.cache_remove(&key)?;
                None
            }
            Err(error) => return Err(error),
        };
        let cached = match cached {
            Some(response) if crate::decider::validate_response(request, &response).is_ok() => {
                Some(response)
            }
            Some(_) => {
                self.session.cache_remove(&key)?;
                None
            }
            None => None,
        };
        let (response, cache_hit) = if let Some(response) = cached {
            self.usage(
                stage.name(),
                provider,
                decider.name(),
                &response.usage,
                true,
            )?;
            (response, true)
        } else {
            self.reserve(
                prompt.len() + 512,
                prepared.as_ref().map_or(0, |p| p.max_tokens),
                primary.is_some(),
            )?;
            let response = match decider.decide(request) {
                Ok(response) => response,
                Err(error) if primary.is_some() => {
                    self.usage(
                        stage.name(),
                        provider,
                        decider.name(),
                        &error.usage(),
                        false,
                    )?;
                    if matches!(stage, Stage::Controller) {
                        // The controller's documented fallback is the safe rule choice.
                        // A failed scheduling request must not switch all topic decisions.
                        return Err(error.into());
                    }
                    self.fallback_active = true;
                    self.fallback_cause = Some(error.code());
                    return self.decide_stage(request, subjects, stage);
                }
                Err(error) => {
                    self.usage(
                        stage.name(),
                        provider,
                        decider.name(),
                        &error.usage(),
                        false,
                    )?;
                    return Err(error.into());
                }
            };
            self.usage(
                stage.name(),
                provider,
                decider.name(),
                &response.usage,
                false,
            )?;
            crate::decider::validate_response(request, &response)?;
            self.session
                .cache_put(&key, stage.name(), provider, decider.name(), &response)?;
            (response, false)
        };
        for (id, answer) in &response.answers {
            let (qtype, confidence) = match answer {
                Answer::Noul { .. } => ("noul", None),
                Answer::Choice { confidence, .. } => ("choice", Some(*confidence)),
                Answer::Score { confidence, .. } => ("score", Some(*confidence)),
            };
            self.session.record_answer(
                &JevAnswerPayload {
                    model: response.model.clone(),
                    request_key: key.clone(),
                    question_id: id.clone(),
                    qtype: qtype.into(),
                    answer: serde_json::to_value(answer)?,
                    confidence,
                    subject: subjects[id].clone(),
                },
                provider,
                cache_hit,
            )?;
        }
        // The request has completed and may already be billed. Preserve its
        // validated result and source mapping before honoring cancellation, so
        // a resumed run can reuse it without assigning work after cancellation.
        if self.cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        Ok(response)
    }
}
