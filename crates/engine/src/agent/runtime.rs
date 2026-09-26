//! Model orchestration, cache identity, bounded cost estimates, and decision signals.
use super::{AnalyzeOptions, Models, warning};
use crate::{
    Config, EngineError, Result,
    decider::{Answer, DecisionRequest, DecisionResponse, Question},
    extract::{self, ExtractionContext, Signals, TopicExtraction},
    llm::{LlmResponse, Usage},
    store::{AnalysisSession, StoredMessage, TopicRecord},
};
use chat_tldr_core::*;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) struct Runtime<'a, 'b> {
    pub(super) session: &'a AnalysisSession,
    pub(super) config: &'a Config,
    pub(super) models: Models<'a>,
    pub(super) options: &'a AnalyzeOptions,
    pub(super) stats: &'b mut RunStats,
    pub(super) reserved: f64,
    pub(super) steps: u32,
    pub(super) fallback_active: bool,
    pub(super) fallback_warned: bool,
    pub(super) cancel: &'a AtomicBool,
}
impl Runtime<'_, '_> {
    pub(super) fn flush_warnings(
        &mut self,
        sink: &mut dyn FnMut(EventBody) -> Result<()>,
    ) -> Result<()> {
        if self.fallback_active && self.options.decider != "llm" && !self.fallback_warned {
            warning(
                "W_DECIDER_FALLBACK",
                "Jev is unavailable; remaining decisions use the configured LLM",
                sink,
            )?;
            self.fallback_warned = true;
        }
        Ok(())
    }
    pub(super) fn decision(
        &mut self,
        action: AgentAction,
        observation: AgentObservation,
        reason: &str,
        sink: &mut dyn FnMut(EventBody) -> Result<()>,
    ) -> Result<()> {
        let payload = DecisionPayload {
            step: self.steps,
            observation,
            allowed: vec![action.clone()],
            chosen: action,
            method: DecisionMethod::Rule,
            probabilities: None,
            confidence: None,
            reason: reason.into(),
        };
        self.session.record_decision(&payload)?;
        sink(EventBody::Decision(payload))?;
        self.steps += 1;
        Ok(())
    }
    fn reserve(&mut self, input: usize, output: u32, jev: bool) -> Result<()> {
        let cost = if jev {
            input as f64 * 0.042 / 1e6 * 4.0
        } else {
            (input as f64 * self.config.llm.price_input_per_mtok.unwrap_or(0.30)
                + output as f64 * self.config.llm.price_output_per_mtok.unwrap_or(1.20))
                / 1e6
                * 8.0
        };
        if self.reserved + cost > self.options.budget_usd {
            return Err(EngineError::BudgetExceeded);
        }
        self.reserved += cost;
        Ok(())
    }
    fn usage(
        &mut self,
        stage: &str,
        provider: &str,
        model: &str,
        usage: &Usage,
        cache: bool,
    ) -> Result<()> {
        let row = UsageStats {
            stage: stage.into(),
            provider: provider.into(),
            model: model.into(),
            calls: u64::from(!cache),
            cache_hits: u64::from(cache),
            input_tokens: if cache { 0 } else { usage.input_tokens },
            output_tokens: if cache { 0 } else { usage.output_tokens },
            cost_usd: if cache { 0.0 } else { usage.cost_usd },
        };
        self.session.record_usage(&row)?;
        self.stats.cost_usd += row.cost_usd;
        self.stats.usage.push(row);
        Ok(())
    }
    fn llm(&mut self, request: &crate::llm::LlmRequest) -> Result<String> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        let key = blake3::hash(
            format!(
                "extract-v1:r1:{:?}:{}",
                self.config.llm,
                serde_json::to_string(request)?
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        if let Some(response) = self.session.cache_get::<LlmResponse>(&key)? {
            self.usage(
                "extract",
                "llm",
                &self.config.llm.model.clone(),
                &response.usage,
                true,
            )?;
            return Ok(response.content);
        }
        self.reserve(
            request.system.len() + request.user.len() + 256,
            request.max_tokens,
            false,
        )?;
        let response = match self.models.llm.complete(request) {
            Ok(r) => r,
            Err(e) => {
                self.usage(
                    "extract",
                    "llm",
                    &self.config.llm.model.clone(),
                    &e.usage(),
                    false,
                )?;
                return Err(e.into());
            }
        };
        self.usage(
            "extract",
            "llm",
            &self.config.llm.model.clone(),
            &response.usage,
            false,
        )?;
        self.session
            .cache_put(&key, "extract", "llm", &self.config.llm.model, &response)?;
        Ok(response.content)
    }
    fn draft_key(&self, context: &ExtractionContext) -> Result<String> {
        Ok(blake3::hash(
            format!(
                "pending-v1:r1:{:?}:{}",
                self.config.llm,
                serde_json::to_string(&context.request)?
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string())
    }
    pub(super) fn save_draft(
        &self,
        context: &ExtractionContext,
        output: &TopicExtraction,
    ) -> Result<()> {
        self.session.cache_put(
            &self.draft_key(context)?,
            "pending-extraction",
            "llm",
            &self.config.llm.model,
            output,
        )
    }
    pub(super) fn restore_or_extract(
        &mut self,
        context: &ExtractionContext,
    ) -> Result<TopicExtraction> {
        if let Some(output) = self
            .session
            .cache_get::<TopicExtraction>(&self.draft_key(context)?)?
        {
            self.usage(
                "extract",
                "llm",
                &self.config.llm.model.clone(),
                &Usage::default(),
                true,
            )?;
            return Ok(output);
        }
        let output = self.extract_topic(context)?;
        self.save_draft(context, &output)?;
        Ok(output)
    }
    pub(super) fn extract_topic(&mut self, context: &ExtractionContext) -> Result<TopicExtraction> {
        let mut request = context.request.clone();
        for attempt in 0..2 {
            let content = self.llm(&request)?;
            match extract::parse_topic(&content, context) {
                Ok(value) => return Ok(value),
                Err(error) if attempt == 0 => request.user.push_str(&format!(
                    "\nValidation error: {error}. Return corrected json."
                )),
                Err(error) => {
                    return Err(crate::llm::ProviderError::new(
                        "E_LLM_OUTPUT_INVALID",
                        true,
                        error,
                    )
                    .into());
                }
            }
        }
        unreachable!("bounded retry returns")
    }
    pub(super) fn extract_direct(
        &mut self,
        context: &ExtractionContext,
    ) -> Result<extract::DirectExtraction> {
        let mut request = context.request.clone();
        for attempt in 0..2 {
            let content = self.llm(&request)?;
            match extract::parse_direct(&content, context) {
                Ok(value) => return Ok(value),
                Err(error) if attempt == 0 => request.user.push_str(&format!(
                    "\nValidation error: {error}. Return corrected json."
                )),
                Err(error) => {
                    return Err(crate::llm::ProviderError::new(
                        "E_LLM_OUTPUT_INVALID",
                        true,
                        error,
                    )
                    .into());
                }
            }
        }
        unreachable!("bounded retry returns")
    }
    fn decide(&mut self, request: &DecisionRequest) -> Result<DecisionResponse> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        let primary = if self.fallback_active {
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
                "decide-v1:r1:{provider}:{config:?}:{}:{prompt}",
                decider.name()
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        let response = if let Some(response) = self.session.cache_get::<DecisionResponse>(&key)? {
            self.usage("decide", provider, decider.name(), &response.usage, true)?;
            response
        } else {
            self.reserve(
                prompt.len() + 512,
                prepared.as_ref().map_or(0, |p| p.max_tokens),
                primary.is_some(),
            )?;
            let response = match decider.decide(request) {
                Ok(response) => response,
                Err(error) if primary.is_some() => {
                    self.usage("decide", provider, decider.name(), &error.usage(), false)?;
                    self.fallback_active = true;
                    return self.decide(request);
                }
                Err(error) => {
                    self.usage("decide", provider, decider.name(), &error.usage(), false)?;
                    return Err(error.into());
                }
            };
            self.usage("decide", provider, decider.name(), &response.usage, false)?;
            crate::decider::validate_response(request, &response)?;
            self.session
                .cache_put(&key, "decide", provider, decider.name(), &response)?;
            response
        };
        for (id, answer) in &response.answers {
            let (qtype, confidence) = match answer {
                Answer::Noul { .. } => ("noul", None),
                Answer::Choice { confidence, .. } => ("choice", Some(*confidence)),
                Answer::Score { confidence, .. } => ("score", Some(*confidence)),
            };
            self.session.record_answer(
                &key,
                id,
                qtype,
                &serde_json::to_value(answer)?,
                confidence,
                &request.state,
            )?;
        }
        Ok(response)
    }
    pub(super) fn classify(
        &mut self,
        messages: &[StoredMessage],
        chat: &ChatMeta,
    ) -> Result<Signals> {
        let mut signals = Signals::default();
        for batch in messages.chunks(40) {
            let next = self.classify_batch(batch, chat)?;
            signals.todo.extend(next.todo);
            signals.announcement.extend(next.announcement);
            signals.needs_action.extend(next.needs_action);
            signals.urgency = signals
                .urgency
                .into_iter()
                .chain(next.urgency)
                .reduce(f32::max);
            signals.chitchat = signals
                .chitchat
                .into_iter()
                .chain(next.chitchat)
                .reduce(f32::min);
        }
        Ok(signals)
    }
    fn classify_batch(&mut self, messages: &[StoredMessage], chat: &ChatMeta) -> Result<Signals> {
        let mut questions = BTreeMap::new();
        let mut rows = Vec::new();
        for (n, message) in messages.iter().enumerate() {
            let reference = format!("n{}", n + 1);
            rows.push(json!({"ref":reference,"message_id":message.message.id,"text":crate::render::render(&message.message)}));
            for label in ["todo", "announcement"] {
                questions.insert(
                    format!("{reference}_{label}"),
                    Question::Noul {
                        instructions: format!(
                            "Does message {reference} contain a concrete {label}?"
                        ),
                        criteria: None,
                    },
                );
            }
            if extract::mentions_me(&message.message, chat) {
                questions.insert(
                    format!("{reference}_needs_action"),
                    Question::Noul {
                        instructions: format!(
                            "Does message {reference} require action from the mentioned people?"
                        ),
                        criteria: None,
                    },
                );
            }
        }
        questions.insert(
            "chitchat".into(),
            Question::Noul {
                instructions: "Is this casual chat with no substantive information?".into(),
                criteria: None,
            },
        );
        questions.insert(
            "urgency".into(),
            Question::Score {
                instructions: "How urgent are these messages?".into(),
                levels: vec![
                    "Not time-sensitive".into(),
                    "Within days".into(),
                    "Within hours".into(),
                ],
            },
        );
        let response = self.decide(&DecisionRequest {
            state: json!({"new_messages":rows}),
            questions,
        })?;
        let mut signals = Signals::default();
        for (n, message) in messages.iter().enumerate() {
            for (label, map) in [
                ("todo", &mut signals.todo),
                ("announcement", &mut signals.announcement),
                ("needs_action", &mut signals.needs_action),
            ] {
                if let Some(Answer::Noul { p_yes }) =
                    response.answers.get(&format!("n{}_{label}", n + 1))
                {
                    map.insert(message.message.id.clone(), *p_yes);
                }
            }
        }
        if let Some(Answer::Noul { p_yes }) = response.answers.get("chitchat") {
            signals.chitchat = Some(*p_yes)
        }
        if let Some(Answer::Score { score, .. }) = response.answers.get("urgency") {
            signals.urgency = Some(*score)
        }
        Ok(signals)
    }
    pub(super) fn choose_topic(
        &mut self,
        messages: &[StoredMessage],
        topics: &[TopicRecord],
    ) -> Result<Option<TopicRecord>> {
        if topics.is_empty() {
            return Ok(None);
        }
        let candidates: Vec<_> = topics
            .iter()
            .rev()
            .take(self.config.segment.all_candidates_max.min(254) as usize)
            .collect();
        let mut options: BTreeMap<_, _> = candidates
            .iter()
            .enumerate()
            .map(|(n, _)| (format!("T{n}"), format!("Continue candidate T{n}")))
            .collect();
        options.insert("new_topic".into(), "Start a new topic".into());
        let request = DecisionRequest {
            state: json!({"candidate_topics":candidates.iter().enumerate().map(|(n,t)|json!({"ref":format!("T{n}"),"title":t.title})).collect::<Vec<_>>(),"new_messages":messages.iter().map(|m|crate::render::render(&m.message)).collect::<Vec<_>>()}),
            questions: BTreeMap::from([(
                "topic".into(),
                Question::Choice {
                    instructions: "Which candidate topic do these messages continue?".into(),
                    options,
                },
            )]),
        };
        let response = self.decide(&request)?;
        if let Some(Answer::Choice {
            choice, confidence, ..
        }) = response.answers.get("topic")
            && *confidence >= self.config.segment.tau_high
            && let Some(index) = choice
                .strip_prefix('T')
                .and_then(|s| s.parse::<usize>().ok())
        {
            return Ok(candidates.get(index).map(|t| (*t).clone()));
        }
        Ok(None)
    }
}
