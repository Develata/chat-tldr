//! Finish settled topics before consolidating them; drafts keep their original context.
use super::*;
use crate::segment::merge::MergeGraph;

pub(super) fn assignments(snapshot: &AnalysisSnapshot) -> BTreeMap<MessageId, TopicId> {
    snapshot
        .messages
        .iter()
        .filter_map(|m| {
            m.topic_id
                .as_ref()
                .map(|t| (m.message.id.clone(), t.clone()))
        })
        .collect()
}

fn graph(
    snapshot: &AnalysisSnapshot,
    options: &AnalyzeOptions,
    assignments: &BTreeMap<MessageId, TopicId>,
    topics: &[TopicRecord],
    completed: &BTreeSet<MessageId>,
) -> MergeGraph {
    // Include unfinished members outside the requested window: their cached
    // drafts must not be invalidated by moving their topic underneath them.
    let unfinished: BTreeSet<_> = snapshot
        .messages
        .iter()
        .filter(|m| {
            !matches!(m.analysis_state.as_str(), "done" | "skipped")
                && !completed.contains(&m.message.id)
        })
        .filter_map(|m| assignments.get(&m.message.id))
        .collect();
    let settled: Vec<_> = topics
        .iter()
        .filter(|t| t.state == TopicState::Active && !unfinished.contains(&t.id))
        .cloned()
        .collect();
    let in_scope = snapshot
        .messages
        .iter()
        .filter(|m| {
            !m.message.recalled
                && options.since.is_none_or(|t| m.message.sent_at >= t)
                && options.until.is_none_or(|t| m.message.sent_at < t)
        })
        .map(|m| m.message.id.clone())
        .collect();
    MergeGraph::new(
        &snapshot.messages,
        assignments,
        &settled,
        &snapshot.rejected_merges,
        &in_scope,
    )
}

pub(super) fn candidate_count(snapshot: &AnalysisSnapshot, options: &AnalyzeOptions) -> usize {
    settled_candidate_count(
        snapshot,
        options,
        &assignments(snapshot),
        &snapshot.topics,
        &BTreeSet::new(),
    )
}

pub(super) fn settled_candidate_count(
    snapshot: &AnalysisSnapshot,
    options: &AnalyzeOptions,
    assignments: &BTreeMap<MessageId, TopicId>,
    topics: &[TopicRecord],
    completed: &BTreeSet<MessageId>,
) -> usize {
    graph(snapshot, options, assignments, topics, completed).count()
}

pub(super) struct MergeOutcome {
    pub(super) reason: FinishReason,
    pub(super) had_failure: bool,
    pub(super) remaining: usize,
}

/// Resolve only the few representative messages needed for each model request.
/// Path compression avoids rescanning every message after chained merges.
fn canonical(aliases: &mut BTreeMap<TopicId, TopicId>, id: &TopicId) -> TopicId {
    let mut current = id.clone();
    let mut path = Vec::new();
    while let Some(parent) = aliases.get(&current) {
        path.push(current.clone());
        current = parent.clone();
    }
    for child in path {
        aliases.insert(child, current.clone());
    }
    current
}

#[derive(Clone, Copy)]
struct Extent {
    count: u64,
    first: DateTime<FixedOffset>,
}

fn payload(topic: &TopicRecord, extent: Extent, merged_into: Option<TopicId>) -> TopicPayload {
    TopicPayload {
        topic_id: topic.id.clone(),
        chat_id: topic.chat_id.clone(),
        title: topic.title.clone(),
        title_is_provisional: topic.provisional,
        state: topic.state,
        message_count: extent.count,
        first_message_at: extent.first,
        last_message_at: topic.last_message_at,
        is_chitchat: topic.is_chitchat,
        merged_into,
    }
}

pub(super) fn process(
    snapshot: &AnalysisSnapshot,
    assignments: &BTreeMap<MessageId, TopicId>,
    completed: &BTreeSet<MessageId>,
    known_topics: &mut [TopicRecord],
    runtime: &mut Runtime<'_, '_>,
    sink: &mut dyn FnMut(EventBody) -> Result<()>,
) -> Result<MergeOutcome> {
    let mut graph = graph(
        snapshot,
        runtime.options,
        assignments,
        known_topics,
        completed,
    );
    let mut outcome = MergeOutcome {
        reason: FinishReason::Done,
        had_failure: false,
        remaining: 0,
    };
    if graph.count() == 0 {
        return Ok(outcome);
    }
    let sources: BTreeMap<_, _> = snapshot
        .messages
        .iter()
        .map(|m| (&m.message.id, m))
        .collect();
    let mut topics: BTreeMap<_, _> = known_topics.iter_mut().map(|t| (t.id.clone(), t)).collect();
    let mut extents = BTreeMap::<TopicId, Extent>::new();
    for m in &snapshot.messages {
        if let Some(id) = assignments.get(&m.message.id) {
            let extent = extents.entry(id.clone()).or_insert(Extent {
                count: 0,
                first: m.message.sent_at,
            });
            extent.count += 1;
            extent.first = extent.first.min(m.message.sent_at);
        }
    }
    let mut aliases = BTreeMap::new();
    let mut active_topics = topics
        .values()
        .filter(|t| t.state == TopicState::Active)
        .count();
    while let Some(candidate) = graph.next() {
        if runtime.cancel.load(Ordering::Relaxed) {
            outcome.reason = FinishReason::Cancelled;
            break;
        }
        if runtime.steps >= runtime.options.max_steps {
            outcome.reason = FinishReason::MaxSteps;
            break;
        }
        runtime.decision(
            AgentAction::MergeTopics {
                into: candidate.into.clone(),
                from: vec![candidate.from.clone()],
            },
            AgentObservation {
                pending_messages: 0,
                interleave: 0.0,
                active_topics: active_topics as u32,
                dirty_topics: 0,
                pending_verification: 0,
                merge_candidates: graph.count() as u32,
                steps_taken: runtime.steps,
                cost_usd: runtime.stats.cost_usd,
            },
            &format!(
                "Confirm a settled topic pair supported by {} reply edges",
                candidate.reply_edges
            ),
            sink,
        )?;
        let evidence: Vec<_> = candidate
            .evidence
            .iter()
            .map(|id| {
                let mut message = (*sources[id]).clone();
                message.topic_id = Some(canonical(&mut aliases, &assignments[id]));
                message
            })
            .collect();
        let confirmation =
            runtime.confirm_merge(topics[&candidate.into], topics[&candidate.from], &evidence);
        match confirmation {
            Ok(true) => {
                if runtime.cancel.load(Ordering::Relaxed) {
                    outcome.reason = FinishReason::Cancelled;
                    break;
                }
                if let Some(merged) = runtime.session.merge_topics(
                    &candidate.into,
                    &candidate.from,
                    &candidate.evidence,
                )? {
                    graph.merge(&candidate);
                    aliases.insert(candidate.from.clone(), candidate.into.clone());
                    active_topics -= 1;
                    let source_extent = extents
                        .remove(&candidate.from)
                        .expect("candidate has messages");
                    let target_extent = extents
                        .get_mut(&candidate.into)
                        .expect("candidate has messages");
                    target_extent.count += source_extent.count;
                    target_extent.first = target_extent.first.min(source_extent.first);
                    **topics
                        .get_mut(&candidate.into)
                        .expect("candidate topic exists") = merged.target.clone();
                    **topics
                        .get_mut(&candidate.from)
                        .expect("candidate topic exists") = merged.source.clone();
                    // The transaction has committed: count it before any fallible output.
                    runtime.stats.topics_updated += 2;
                    runtime.stats.insights.updated += merged.moved_insights;
                    sink(EventBody::Topic(payload(
                        &merged.target,
                        *target_extent,
                        None,
                    )))?;
                    sink(EventBody::Topic(payload(
                        &merged.source,
                        Extent {
                            count: 0,
                            ..source_extent
                        },
                        Some(candidate.into.clone()),
                    )))?;
                } else {
                    graph.reject(&candidate);
                    defer(&mut outcome, sink)?;
                }
            }
            Ok(false) => {
                runtime
                    .session
                    .reject_merge(&candidate.into, &candidate.from)?;
                graph.reject(&candidate);
            }
            Err(EngineError::Cancelled) => {
                outcome.reason = FinishReason::Cancelled;
                break;
            }
            Err(EngineError::BudgetExceeded) => {
                outcome.reason = FinishReason::BudgetExceeded;
                break;
            }
            Err(error) if error.code() == "E_MERGE_STATE_TOO_LARGE" => {
                graph.reject(&candidate);
                defer(&mut outcome, sink)?;
            }
            Err(error) if recoverable_topic_error(&error) => {
                graph.reject(&candidate);
                outcome.had_failure = true;
                emit_error(&error, "merge", None, sink)?;
            }
            Err(error) => return Err(error),
        }
        runtime.flush_warnings(sink)?;
    }
    // Transiently deferred pairs have been suppressed only in this run's graph;
    // the next plan rebuilds them from persisted memberships and rejection pairs.
    outcome.remaining = graph.count();
    Ok(outcome)
}

fn defer(outcome: &mut MergeOutcome, sink: &mut dyn FnMut(EventBody) -> Result<()>) -> Result<()> {
    outcome.had_failure = true;
    sink(EventBody::Warning(WarningPayload {
        stage: "merge".into(),
        code: "W_MERGE_DEFERRED".into(),
        message:
            "Topic merge deferred: its evidence changed or exceeds the configured state budget"
                .into(),
    }))
}
