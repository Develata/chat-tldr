//! Bounded, synchronous analysis. Provider calls happen outside write transactions.
mod baseline;
mod lifecycle;
mod merge;
mod runtime;
use crate::segment::{ReplyInterleave, bursts, candidates::TopicLinks};
use crate::{
    Config, EngineError, Result,
    decider::Decider,
    extract::{self, TopicExtraction},
    llm::LlmClient,
    store::{self, AnalysisSession, AnalysisSnapshot, StoredMessage, TopicRecord},
};
use chat_tldr_core::*;
use chrono::{DateTime, FixedOffset, Utc};
use runtime::Runtime;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

#[derive(Clone, Debug, serde::Serialize)]
pub struct AnalyzeOptions {
    pub since: Option<DateTime<FixedOffset>>,
    pub until: Option<DateTime<FixedOffset>>,
    pub max_steps: u32,
    pub budget_usd: f64,
    pub decider: String,
    pub strategy: String,
}
impl AnalyzeOptions {
    pub fn from_config(config: &Config) -> Self {
        Self {
            since: None,
            until: None,
            max_steps: config.agent.max_steps,
            budget_usd: config.agent.budget_usd,
            decider: "jev".into(),
            strategy: "ours".into(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.max_steps == 0
            || !self.budget_usd.is_finite()
            || self.budget_usd <= 0.0
            || self.since.zip(self.until).is_some_and(|(a, b)| a > b)
            || !matches!(self.decider.as_str(), "jev" | "llm")
            || !matches!(self.strategy.as_str(), "ours" | "b0")
        {
            return Err(EngineError::Usage(
                "invalid analysis bounds, budget, step count or decider".into(),
            ));
        }
        Ok(())
    }
}

pub struct AnalysisResult {
    pub status: RunStatus,
    pub reason: FinishReason,
    pub stats: RunStats,
}
pub fn plan(path: &Path, chat: &ChatId, options: &AnalyzeOptions) -> Result<serde_json::Value> {
    options.validate()?;
    let snapshot = store::analysis_snapshot(path, chat)?;
    if options.strategy == "b0" {
        return baseline::plan(&snapshot, options);
    }
    let pending: Vec<_> = eligible(&snapshot, options);
    let dirty: BTreeSet<_> = pending.iter().filter_map(|m| m.topic_id.clone()).collect();
    let merges = merge::candidate_count(&snapshot, options);
    Ok(
        json!({"chat_id":chat,"messages":pending.len(),"unassigned_messages":pending.iter().filter(|m|m.topic_id.is_none()).count(),"dirty_topics":dirty.len(),"merge_candidates":merges,"snapshot_up_to":snapshot.messages.last().map(|m|m.cursor),"since":options.since,"until":options.until,"max_steps":options.max_steps,"budget_usd":options.budget_usd,"model_calls":0,"writes":false}),
    )
}

fn eligible(snapshot: &AnalysisSnapshot, options: &AnalyzeOptions) -> Vec<StoredMessage> {
    snapshot
        .messages
        .iter()
        .filter(|m| {
            !m.message.recalled
                && matches!(m.analysis_state.as_str(), "pending" | "failed")
                && options.since.is_none_or(|t| m.message.sent_at >= t)
                && options.until.is_none_or(|t| m.message.sent_at < t)
        })
        .cloned()
        .collect()
}

pub struct Models<'a> {
    pub llm: &'a dyn LlmClient,
    pub primary: Option<&'a dyn Decider>,
    pub fallback: &'a dyn Decider,
}
struct PendingTopic {
    topic: TopicRecord,
    messages: Vec<StoredMessage>,
    output: Option<TopicExtraction>,
}

/// The sink must not call back into analysis. Only committed business records are emitted.
#[allow(clippy::too_many_arguments)] // Keep execution inputs explicit at the engine boundary.
pub fn analyze(
    path: &Path,
    chat: &ChatId,
    run: &RunId,
    config: &Config,
    options: &AnalyzeOptions,
    models: Models<'_>,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(EventBody) -> Result<()>,
) -> Result<AnalysisResult> {
    options.validate()?;
    if options.strategy == "b0" {
        return baseline::analyze(path, chat, run, config, options, models, cancel, sink);
    }
    let start = Instant::now();
    let preflight = store::analysis_snapshot(path, chat)?;
    let mut stats = RunStats {
        run_id: run.clone(),
        chat_id: chat.clone(),
        messages_analyzed: 0,
        topics_created: 0,
        topics_updated: 0,
        insights: InsightStats::default(),
        usage: Vec::new(),
        cost_usd: 0.0,
        elapsed_ms: 0,
    };
    if eligible(&preflight, options).is_empty() && merge::candidate_count(&preflight, options) == 0
    {
        return Ok(AnalysisResult {
            status: RunStatus::Complete,
            reason: FinishReason::Done,
            stats,
        });
    }
    let mut session = AnalysisSession::begin(path, chat, run, &serde_json::to_value(options)?)?;
    let snapshot = session.snapshot()?;
    let messages = eligible(&snapshot, options);
    let mut unassigned: VecDeque<_> = messages
        .iter()
        .filter(|m| m.topic_id.is_none())
        .cloned()
        .collect();
    let backlog_len = unassigned.len();
    let interleaving = ReplyInterleave::new(unassigned.make_contiguous());
    let mut groups = BTreeMap::<TopicId, Vec<StoredMessage>>::new();
    for message in messages.iter().filter(|m| m.topic_id.is_some()) {
        groups
            .entry(message.topic_id.clone().expect("filtered"))
            .or_default()
            .push(message.clone());
    }
    let mut queue: VecDeque<_> = snapshot
        .topics
        .iter()
        .filter_map(|t| {
            groups.remove(&t.id).map(|m| PendingTopic {
                topic: t.clone(),
                messages: m,
                output: None,
            })
        })
        .collect();
    let mut known_topics = snapshot.topics.clone();
    let mut lifecycle = lifecycle::TopicLifecycle::new(&snapshot);
    let mut assignments = merge::assignments(&snapshot);
    let mut completed = BTreeSet::new();
    let mut links = TopicLinks::new(&snapshot.messages);
    for (id, at) in &snapshot.closed_at {
        links.mark_closed(id, *at);
    }
    let fallback_active = options.decider == "llm" || models.primary.is_none();
    store::decay_preferences(path)?;
    let mut runtime = Runtime {
        session: &session,
        config,
        models,
        options,
        stats: &mut stats,
        budget: Default::default(),
        steps: 0,
        fallback_active,
        fallback_warned: false,
        fallback_cause: None,
        cancel,
    };
    // Every execution error, including event sinks between checkpoints, leaves
    // through one finalizer so persisted work never loses its exact counters.
    let mut assigned_any = false;
    let processing: Result<(RunStatus, FinishReason)> = (|| {
        let mut reason = FinishReason::Done;
        let mut had_failure = false;
        let mut topic_number = 0_u32;
        while !unassigned.is_empty() || !queue.is_empty() {
            if cancel.load(Ordering::Relaxed) {
                reason = FinishReason::Cancelled;
                break;
            }
            if runtime.steps >= options.max_steps {
                reason = FinishReason::MaxSteps;
                break;
            }
            let observation = observation(
                unassigned.len(),
                &queue,
                &known_topics,
                runtime.steps,
                runtime.stats.cost_usd,
                interleaving.remaining(backlog_len - unassigned.len()),
            );
            if !unassigned.is_empty() {
                let direct_allowed = unassigned.len() <= config.agent.direct_max as usize
                // Direct extraction creates fresh topics. Existing historical
                // candidates must get attribution even for a small backlog.
                && !unassigned.iter().any(|message| {
                    links.has_candidates(std::slice::from_ref(message), &known_topics, &config.segment)
                });
                let direct =
                    direct_allowed && observation.interleave < config.agent.direct_interleave_max;
                let count = if direct {
                    unassigned.len()
                } else {
                    (config.agent.segment_batch as usize).min(unassigned.len())
                };
                let range = MessageRange {
                    after: None,
                    up_to: unassigned[count - 1].cursor,
                };
                let action = if direct {
                    AgentAction::AnalyzeDirect {
                        chat_id: chat.clone(),
                        range,
                    }
                } else {
                    AgentAction::Segment {
                        chat_id: chat.clone(),
                        range,
                    }
                };
                let mut allowed = vec![action];
                let mut descriptions = vec![
                    if direct {
                        "Extract all messages as a small coherent batch"
                    } else {
                        "Segment messages before extracting individual topics"
                    }
                    .into(),
                ];
                if direct_allowed && !direct {
                    allowed.push(AgentAction::AnalyzeDirect {
                        chat_id: chat.clone(),
                        range: MessageRange {
                            after: None,
                            up_to: unassigned.back().expect("nonempty").cursor,
                        },
                    });
                    descriptions.push("Extract all small-backlog messages directly, grouping their topics in one request".into());
                }
                let chosen = match runtime.select_action(allowed, descriptions, observation, sink) {
                    Ok(action) => action,
                    Err(EngineError::BudgetExceeded) => {
                        reason = FinishReason::BudgetExceeded;
                        break;
                    }
                    Err(EngineError::Cancelled) => {
                        reason = FinishReason::Cancelled;
                        break;
                    }
                    Err(error) => return Err(error),
                };
                let direct = matches!(chosen, AgentAction::AnalyzeDirect { .. });
                let count = if direct { unassigned.len() } else { count };
                let batch: Vec<_> = unassigned.drain(..count).collect();
                let outcome: Result<()> = if direct {
                    let mut context = extract::request(&batch, &[], &[], "", true);
                    context.identify_viewer(&snapshot.chat);
                    (|| {
                        let direct_output = runtime.extract_direct(&context)?;
                        for output in direct_output.topics {
                            let member_ids: BTreeSet<_> = output
                                .refs
                                .iter()
                                .map(|r| context.messages[r].id.clone())
                                .collect();
                            let members: Vec<_> = batch
                                .iter()
                                .filter(|m| member_ids.contains(&m.message.id))
                                .cloned()
                                .collect();
                            topic_number += 1;
                            let topic = new_topic(
                                chat,
                                run,
                                topic_number,
                                &members,
                                output.extraction.title.clone(),
                            );
                            // New context numbering follows source order, not model membership order.
                            let ordered_refs: Vec<_> = batch
                                .iter()
                                .enumerate()
                                .filter(|(_, m)| {
                                    members
                                        .iter()
                                        .any(|member| member.message.id == m.message.id)
                                })
                                .map(|(n, _)| format!("n{}", n + 1))
                                .collect();
                            let extraction = remap_extraction(output.extraction, &ordered_refs);
                            let mut topic_context =
                                extract::request(&members, &[], &[], &topic.title, false);
                            topic_context.identify_viewer(&snapshot.chat);
                            runtime.save_draft(&topic_context, &extraction)?;
                            session.assign(
                                &topic,
                                &member_ids.into_iter().collect::<Vec<_>>(),
                                "direct",
                            )?;
                            assigned_any = true;
                            links.assign(&members, &topic.id);
                            for member in &members {
                                assignments.insert(member.message.id.clone(), topic.id.clone());
                            }
                            let update = lifecycle.assigned(&topic, known_topics.len(), &members);
                            known_topics.push(topic.clone());
                            runtime.stats.topics_created += 1;
                            sink(EventBody::Topic(update))?;

                            queue.push_back(PendingTopic {
                                topic,
                                messages: members,
                                output: Some(extraction),
                            });
                        }
                        lifecycle.close_at(
                            batch.last().expect("nonempty batch").message.sent_at,
                            &mut known_topics,
                            &mut links,
                            &mut runtime,
                            sink,
                        )?;
                        Ok(())
                    })()
                } else {
                    (|| {
                        for burst in bursts(&batch, &config.segment) {
                            let rule_topic = links.reply_topic(&burst).and_then(|id| {
                                known_topics
                                    .iter()
                                    .find(|topic| {
                                        &topic.id == id
                                            && links.eligible(topic, &burst, &config.segment)
                                    })
                                    .cloned()
                            });
                            let chosen = if let Some(topic) = rule_topic {
                                Some((topic, "rule_reply"))
                            } else {
                                let candidates =
                                    links.select(&burst, &known_topics, &config.segment);
                                runtime.choose_topic(&burst, &candidates)?
                            };
                            let created = chosen.is_none();
                            let (mut topic, method) = if let Some(pair) = chosen {
                                pair
                            } else {
                                topic_number += 1;
                                let topic = new_topic(
                                    chat,
                                    run,
                                    topic_number,
                                    &burst,
                                    crate::render::render(&burst[0].message)
                                        .chars()
                                        .take(20)
                                        .collect(),
                                );
                                known_topics.push(topic.clone());
                                (topic, "new_topic")
                            };
                            topic.last_message_at = topic
                                .last_message_at
                                .max(burst.last().expect("nonempty burst").message.sent_at);
                            let position = known_topics
                                .iter()
                                .position(|known| known.id == topic.id)
                                .expect("selected or newly added topic exists");
                            let ids: Vec<_> = burst.iter().map(|m| m.message.id.clone()).collect();
                            session.assign(&topic, &ids, method)?;
                            assigned_any = true;
                            runtime.stats.topics_created += u64::from(created);
                            known_topics[position] = topic.clone();
                            let update = lifecycle.assigned(&topic, position, &burst);
                            links.assign(&burst, &topic.id);
                            for member in &burst {
                                assignments.insert(member.message.id.clone(), topic.id.clone());
                            }
                            sink(EventBody::Topic(update))?;
                            let activity_at = burst.last().expect("nonempty burst").message.sent_at;
                            if let Some(pending) = queue.iter_mut().find(|p| p.topic.id == topic.id)
                            {
                                pending.topic = topic;
                                pending.messages.extend(burst)
                            } else {
                                queue.push_back(PendingTopic {
                                    topic,
                                    messages: burst,
                                    output: None,
                                })
                            }
                            lifecycle.close_at(
                                activity_at,
                                &mut known_topics,
                                &mut links,
                                &mut runtime,
                                sink,
                            )?;
                        }
                        Ok(())
                    })()
                };
                if let Err(error) = outcome {
                    if matches!(error, EngineError::Cancelled) {
                        reason = FinishReason::Cancelled;
                        break;
                    }
                    if matches!(error, EngineError::BudgetExceeded) {
                        reason = FinishReason::BudgetExceeded;
                        break;
                    }
                    if !recoverable_topic_error(&error) {
                        return Err(error);
                    }
                    session.fail_messages(
                        &batch
                            .iter()
                            .map(|m| m.message.id.clone())
                            .collect::<Vec<_>>(),
                    )?;
                    had_failure = true;
                    emit_error(&error, "segment", None, sink)?;
                }
                runtime.flush_warnings(sink)?;
                continue;
            }
            // Ready drafts must verify first, including empty extractions. Otherwise
            // Jev chooses among a bounded prefix of executable dirty-topic actions.
            let index = if let Some(index) =
                queue.iter().position(|pending| pending.output.is_some())
            {
                index
            } else {
                let candidates: Vec<_> = queue.iter().take(16).collect();
                let allowed = candidates
                    .iter()
                    .map(|pending| AgentAction::AnalyzeTopic {
                        topic_id: pending.topic.id.clone(),
                    })
                    .collect();
                let descriptions = candidates
                    .iter()
                    .map(|pending| {
                        format!(
                            "Extract pending messages for topic: {}",
                            pending.topic.title
                        )
                    })
                    .collect();
                let chosen =
                    match runtime.select_action(allowed, descriptions, observation.clone(), sink) {
                        Ok(action) => action,
                        Err(EngineError::BudgetExceeded) => {
                            reason = FinishReason::BudgetExceeded;
                            break;
                        }
                        Err(EngineError::Cancelled) => {
                            reason = FinishReason::Cancelled;
                            break;
                        }
                        Err(error) => return Err(error),
                    };
                queue.iter().position(|pending| matches!(&chosen, AgentAction::AnalyzeTopic { topic_id } if *topic_id == pending.topic.id)).expect("validated controller candidate")
            };
            let mut pending = queue.remove(index).expect("work exists");
            // A later burst may have closed an earlier topic while its draft was
            // still queued. Refresh metadata without changing the draft or members.
            if let Some(current) = lifecycle.current(&known_topics, &pending.topic.id) {
                pending.topic = current.clone();
            }
            let ids: Vec<_> = pending
                .messages
                .iter()
                .map(|m| m.message.id.clone())
                .collect();
            let mut context_messages: Vec<_> = snapshot
                .messages
                .iter()
                .filter(|m| {
                    assignments.get(&m.message.id) == Some(&pending.topic.id)
                        && !ids.contains(&m.message.id)
                        && !m.message.recalled
                })
                .rev()
                .take(5)
                .cloned()
                .collect();
            let existing: Vec<_> = snapshot
                .insights
                .iter()
                .filter(|i| i.topic_id.as_ref() == Some(&pending.topic.id))
                .cloned()
                .collect();
            let pending_questions = session.pending_questions(&pending.topic.id)?;
            let required_sources: BTreeSet<_> = existing
                .iter()
                .flat_map(|i| &i.evidence)
                .map(|e| e.message_id.clone())
                .chain(pending_questions.iter().map(|e| e.message_id.clone()))
                .collect();
            // Updating an item appends evidence. Include its current source rows even
            // when older than the short conversational context window.
            for source in snapshot
                .messages
                .iter()
                .filter(|m| required_sources.contains(&m.message.id) && !m.message.recalled)
            {
                if !ids.contains(&source.message.id)
                    && !context_messages
                        .iter()
                        .any(|m| m.message.id == source.message.id)
                {
                    context_messages.push(source.clone());
                }
            }
            let mut context = extract::request(
                &pending.messages,
                &context_messages,
                &existing,
                &pending.topic.title,
                false,
            );
            context.identify_viewer(&snapshot.chat);
            let refs: BTreeMap<_, _> = context.messages.iter().map(|(r, m)| (&m.id, r)).collect();
            let questions: Vec<_> = pending_questions
                .iter()
                .filter_map(|q| {
                    refs.get(&q.message_id)
                        .map(|r| json!({"source_ref":r,"quote":q.quote}))
                })
                .collect();
            let mut request: serde_json::Value = serde_json::from_str(&context.request.user)?;
            request["pending_questions"] = json!(questions);
            context.request.user = request.to_string();
            let result: Result<()> = (|| {
                if pending.output.is_none() {
                    pending.output = Some(runtime.restore_or_extract(&context)?);
                }
                if cancel.load(Ordering::Relaxed) {
                    reason = FinishReason::Cancelled;
                    return Ok(());
                }
                if runtime.steps >= options.max_steps {
                    reason = FinishReason::MaxSteps;
                    return Ok(());
                }
                let signals = runtime.classify(&pending.messages, &snapshot.chat)?;
                runtime.flush_warnings(sink)?;
                if cancel.load(Ordering::Relaxed) {
                    reason = FinishReason::Cancelled;
                    return Ok(());
                }
                let mut output = pending
                    .output
                    .take()
                    .expect("extraction exists even when items is empty");
                let now = Utc::now().fixed_offset();
                let mut insights = extract::insights(
                    &output,
                    &context,
                    &snapshot.chat,
                    &pending.topic.id,
                    run,
                    &signals,
                    now,
                );
                let rejected = insights
                    .iter()
                    .any(|(i, _)| i.verification_status == VerificationStatus::Rejected);
                if rejected {
                    context.request.user.push_str("\nVerification failed. Replace unsupported claims or use exact evidence quotes of at least three non-whitespace characters from the supplied records. Return corrected json.");
                    match runtime.extract_topic(&context) {
                        Ok(corrected) => {
                            output = corrected;
                            insights = extract::insights(
                                &output,
                                &context,
                                &snapshot.chat,
                                &pending.topic.id,
                                run,
                                &signals,
                                now,
                            )
                        }
                        Err(EngineError::BudgetExceeded) => {
                            return Err(EngineError::BudgetExceeded);
                        }
                        Err(error) if !recoverable_topic_error(&error) => return Err(error),
                        Err(error) => {
                            emit_error(&error, "verify", Some(pending.topic.id.clone()), sink)?;
                            had_failure = true;
                        }
                    }
                }
                if cancel.load(Ordering::Relaxed) {
                    reason = FinishReason::Cancelled;
                    return Ok(());
                }
                let observation = AgentObservation {
                    pending_messages: 0,
                    interleave: 0.0,
                    active_topics: known_topics
                        .iter()
                        .filter(|t| t.state == TopicState::Active)
                        .count() as u32,
                    dirty_topics: queue.len() as u32 + 1,
                    pending_verification: insights.len() as u32,
                    merge_candidates: 0,
                    steps_taken: runtime.steps,
                    cost_usd: runtime.stats.cost_usd,
                };
                runtime.decision(
                    AgentAction::Verify {
                        insight_ids: insights.iter().map(|(i, _)| i.id.clone()).collect(),
                    },
                    observation,
                    "Verify and atomically commit the pending topic, including empty extraction",
                    sink,
                )?;
                let relations = output.relations.as_ref().map(|rows| {
                    crate::relations::proposals(rows, &context, chat, &pending.topic.id, run)
                });
                if relations.is_none() {
                    warning(
                        "W_RELATIONS_UNCOVERED",
                        "Model response omitted relations; this checkpoint is not counted as relation coverage",
                        sink,
                    )?;
                }
                pending.topic.title = output.title;
                pending.topic.provisional = false;
                pending.topic.is_chitchat = signals.chitchat;
                pending.topic.last_message_at = pending.topic.last_message_at.max(
                    pending
                        .messages
                        .iter()
                        .map(|m| m.message.sent_at)
                        .max()
                        .expect("nonempty topic"),
                );
                let committed = session.commit_topic_with_relations(
                    &pending.topic,
                    &ids,
                    insights,
                    relations.as_deref(),
                )?;
                completed.extend(ids.iter().cloned());
                if let Some(known) = known_topics.iter_mut().find(|t| t.id == pending.topic.id) {
                    *known = pending.topic.clone();
                }
                runtime.stats.messages_analyzed += ids.len() as u64;
                runtime.stats.topics_updated += 1;
                // Count the whole committed checkpoint before writing to an event
                // sink, which can fail (for example when a JSONL consumer exits).
                let existing_ids: BTreeSet<_> = existing.iter().map(|item| &item.id).collect();
                for insight in &committed {
                    if existing_ids.contains(&insight.id) {
                        runtime.stats.insights.updated += 1;
                    } else {
                        runtime.stats.insights.created += 1;
                    }
                    match insight.verification_status {
                        VerificationStatus::Verified => runtime.stats.insights.verified += 1,
                        VerificationStatus::Unverified => runtime.stats.insights.unverified += 1,
                        _ => runtime.stats.insights.rejected += 1,
                    }
                }
                lifecycle.committed(&pending.topic);
                lifecycle.close_at(
                    pending
                        .messages
                        .iter()
                        .map(|m| m.message.sent_at)
                        .max()
                        .expect("nonempty topic"),
                    &mut known_topics,
                    &mut links,
                    &mut runtime,
                    sink,
                )?;
                sink(EventBody::Progress(ProgressPayload {
                    stage: "store".into(),
                    current: runtime.stats.messages_analyzed,
                    total: Some(messages.len() as u64),
                    message: "Committed topic checkpoint".into(),
                }))?;
                let source_ids: Vec<_> = committed
                    .iter()
                    .flat_map(|item| item.evidence.iter().map(|e| e.message_id.clone()))
                    .collect();
                let current_sources = session.source_messages(&source_ids)?;
                for insight in committed {
                    if matches!(
                        insight.verification_status,
                        VerificationStatus::Rejected | VerificationStatus::Unknown
                    ) {
                        warning(
                            "W_INSIGHT_REJECTED",
                            "An extracted item did not pass evidence verification",
                            sink,
                        )?;
                    }
                    let evidence_view = insight
                        .evidence
                        .iter()
                        .filter_map(|e| {
                            current_sources.get(&e.message_id).map(|m| {
                                let display_text = crate::render::render(m);
                                let highlight = crate::verify::find_quote(&display_text, &e.quote)
                                    .map(|(a, b)| [a, b]);
                                EvidenceView {
                                    message_id: m.id.clone(),
                                    sender_display: m.sender_display.clone(),
                                    sent_at: m.sent_at,
                                    display_text,
                                    ok: highlight.is_some()
                                        && !m.recalled
                                        && e.quote
                                            .chars()
                                            .filter(|c| !c.is_whitespace())
                                            .take(3)
                                            .count()
                                            == 3,
                                    highlight,
                                }
                            })
                        })
                        .collect();
                    sink(EventBody::Insight(InsightPayload {
                        insight,
                        evidence_view,
                    }))?;
                }
                Ok(())
            })();
            if reason != FinishReason::Done {
                break;
            }
            if let Err(error) = result {
                if matches!(error, EngineError::Cancelled) {
                    reason = FinishReason::Cancelled;
                    break;
                }
                if matches!(error, EngineError::BudgetExceeded) {
                    reason = FinishReason::BudgetExceeded;
                    break;
                }
                if !recoverable_topic_error(&error) {
                    return Err(error);
                }
                session.fail_messages(&ids)?;
                had_failure = true;
                emit_error(&error, "extract", Some(pending.topic.id), sink)?;
            }
            sink(EventBody::Progress(ProgressPayload {
                stage: "store".into(),
                current: runtime.stats.messages_analyzed,
                total: Some(messages.len() as u64),
                message: "Topic processing checkpoint".into(),
            }))?;
        }
        let remaining_merges = if reason == FinishReason::Done {
            let outcome = merge::process(
                &snapshot,
                &assignments,
                &completed,
                &mut known_topics,
                &mut runtime,
                sink,
            )?;
            reason = outcome.reason;
            had_failure |= outcome.had_failure;
            outcome.remaining
        } else {
            merge::settled_candidate_count(
                &snapshot,
                options,
                &assignments,
                &known_topics,
                &completed,
            )
        };
        if had_failure && reason == FinishReason::Done {
            reason = FinishReason::Error
        }
        let status = match reason {
            FinishReason::Done => RunStatus::Complete,
            FinishReason::Cancelled => RunStatus::Cancelled,
            _ => RunStatus::Partial,
        };
        let mut observation = observation(
            unassigned.len(),
            &queue,
            &known_topics,
            runtime.steps,
            runtime.stats.cost_usd,
            interleaving.remaining(backlog_len - unassigned.len()),
        );
        observation.merge_candidates = remaining_merges as u32;
        runtime.flush_warnings(sink)?;
        runtime.decision(
            AgentAction::Finish { reason },
            observation,
            "Finish at a bounded checkpoint",
            sink,
        )?;
        Ok((status, reason))
    })();
    stats.elapsed_ms = start.elapsed().as_millis() as u64;
    let (status, reason) = match processing {
        Ok(finished) => finished,
        Err(error) => {
            session.finish_with_stats(
                if assigned_any || stats.messages_analyzed > 0 || stats.topics_updated > 0 {
                    RunStatus::Partial
                } else {
                    RunStatus::Failed
                },
                &stats,
            )?;
            return Err(error);
        }
    };
    session.finish_with_stats(status, &stats)?;
    Ok(AnalysisResult {
        status,
        reason,
        stats,
    })
}

fn remap_extraction(mut output: TopicExtraction, refs: &[String]) -> TopicExtraction {
    let mapping: BTreeMap<_, _> = refs
        .iter()
        .enumerate()
        .map(|(n, r)| (r.as_str(), format!("n{}", n + 1)))
        .collect();
    for item in &mut output.items {
        for evidence in &mut item.evidence {
            if let Some(new) = mapping.get(evidence.reference.as_str()) {
                evidence.reference = new.clone();
            }
        }
    }
    for relation in output.relations.iter_mut().flatten() {
        for evidence in std::iter::once(&mut relation.source).chain(relation.target.iter_mut()) {
            if let Some(new) = mapping.get(evidence.reference.as_str()) {
                evidence.reference = new.clone();
            }
        }
    }
    output
}
fn new_topic(
    chat: &ChatId,
    run: &RunId,
    index: u32,
    messages: &[StoredMessage],
    title: String,
) -> TopicRecord {
    TopicRecord {
        id: format!(
            "t_{}",
            &blake3::hash(format!("{chat}\n{run}\n{index}").as_bytes()).to_hex()[..12]
        )
        .into(),
        chat_id: chat.clone(),
        title,
        provisional: true,
        state: TopicState::Active,
        last_message_at: messages.last().expect("nonempty topic").message.sent_at,
        is_chitchat: None,
    }
}
fn observation(
    pending_messages: usize,
    queue: &VecDeque<PendingTopic>,
    topics: &[TopicRecord],
    steps: u32,
    cost: f64,
    interleave: f32,
) -> AgentObservation {
    AgentObservation {
        pending_messages: pending_messages as u32,
        interleave,
        active_topics: topics
            .iter()
            .filter(|t| t.state == TopicState::Active)
            .count() as u32,
        dirty_topics: queue.len() as u32,
        pending_verification: queue
            .iter()
            .filter_map(|p| p.output.as_ref())
            .map(|o| o.items.len() as u32)
            .sum(),
        merge_candidates: 0,
        steps_taken: steps,
        cost_usd: cost,
    }
}

fn warning(code: &str, message: &str, sink: &mut dyn FnMut(EventBody) -> Result<()>) -> Result<()> {
    sink(EventBody::Warning(WarningPayload {
        stage: "decide".into(),
        code: code.into(),
        message: message.into(),
    }))
}
fn recoverable_topic_error(error: &EngineError) -> bool {
    matches!(error, EngineError::Provider(provider) if provider.retryable())
}
fn emit_error(
    error: &EngineError,
    stage: &str,
    topic: Option<TopicId>,
    sink: &mut dyn FnMut(EventBody) -> Result<()>,
) -> Result<()> {
    sink(EventBody::Error(ErrorPayload {
        stage: stage.into(),
        code: error.code().into(),
        retryable: matches!(error,EngineError::Provider(e) if e.retryable()),
        message: error.to_string(),
        topic_id: topic,
    }))
}
