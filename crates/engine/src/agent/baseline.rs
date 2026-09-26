//! B0: one whole-window LLM extraction, without Jev, repair or topic merging.
//! Common evidence verification labels proposals after generation; it never
//! removes rejected proposals. Evaluation retains their original model order.
use super::*;

const MAX_INPUT_CHARS: usize = 96_000;

fn selection(
    snapshot: &AnalysisSnapshot,
    options: &AnalyzeOptions,
) -> Result<(Vec<StoredMessage>, usize)> {
    if !snapshot.topics.is_empty() || !snapshot.insights.is_empty() {
        return Err(EngineError::Usage("B0 requires a fresh, separately imported data directory; do not mix baseline and Ours state".into()));
    }
    let mut messages = eligible(snapshot, options);
    let original = messages.len();
    let sizes: Vec<_> = messages
        .iter()
        .map(|message| crate::render::render(&message.message).chars().count() + 256)
        .collect();
    let mut length: usize = sizes.iter().sum();
    let mut first = 0;
    while length > MAX_INPUT_CHARS && first < sizes.len() {
        length -= sizes[first];
        first += 1;
    }
    messages.drain(..first);
    if original > 0 && messages.is_empty() {
        return Err(EngineError::Input(
            "even the newest baseline message exceeds the input limit".into(),
        ));
    }
    Ok((messages, first))
}

pub(super) fn plan(
    snapshot: &AnalysisSnapshot,
    options: &AnalyzeOptions,
) -> Result<serde_json::Value> {
    let (messages, truncated) = selection(snapshot, options)?;
    Ok(
        json!({"strategy":"b0", "messages":messages.len(), "truncated_messages":truncated,
        "input_character_limit":MAX_INPUT_CHARS, "merge_candidates":0,
        "max_steps":options.max_steps, "budget_usd":options.budget_usd, "model_calls":0, "writes":false}),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn analyze(
    path: &Path,
    chat: &ChatId,
    run: &RunId,
    config: &Config,
    options: &AnalyzeOptions,
    models: Models<'_>,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(EventBody) -> Result<()>,
) -> Result<AnalysisResult> {
    let start = Instant::now();
    let mut session = AnalysisSession::begin(path, chat, run, &serde_json::to_value(options)?)?;
    let snapshot = session.snapshot()?;
    let (messages, truncated) = selection(&snapshot, options)?;
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
    let mut order = Vec::new();
    let mut assigned = false;
    let mut runtime = Runtime {
        session: &session,
        config,
        models,
        options,
        stats: &mut stats,
        budget: Default::default(),
        steps: 0,
        fallback_active: true,
        fallback_warned: false,
        fallback_cause: None,
        cancel,
    };
    let processing: Result<FinishReason> = (|| {
        sink(EventBody::Ack(AckPayload {
            command: "analyze.strategy".into(),
            target: Some(chat.to_string()),
            changed: false,
            detail: json!({"strategy":"b0", "ranking":"model_order",
                "input_messages":messages.len() + truncated, "selected_messages":messages.len(), "truncated_messages":truncated}),
        }))?;
        if truncated > 0 {
            warning(
                "W_BASELINE_TRUNCATED",
                &format!(
                    "B0 omitted {truncated} oldest messages; they remain pending, and must be reported as missing topic coverage"
                ),
                sink,
            )?;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(EngineError::Cancelled);
        }
        let Some(last) = messages.last() else {
            return Ok(FinishReason::Done);
        };
        let mut context = extract::request(&messages, &[], &[], "", true);
        context.identify_viewer(&snapshot.chat);
        context.request.max_tokens = 16384;
        context.request.system.push_str(" Baseline single-pass summarization: order topics, then items within topics, from most important to least important for the viewer. Do not omit topics without action items.");
        runtime.decision(
            AgentAction::AnalyzeDirect {
                chat_id: chat.clone(),
                range: MessageRange {
                    after: None,
                    up_to: last.cursor,
                },
            },
            observation(messages.len(), &VecDeque::new(), &[], 0, 0.0, 0.0),
            "B0 whole-window extraction",
            sink,
        )?;
        let output = runtime.extract_direct(&context)?;
        for (index, direct) in output.topics.into_iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(EngineError::Cancelled);
            }
            if runtime.steps >= options.max_steps {
                return Ok(FinishReason::MaxSteps);
            }
            let ids: BTreeSet<_> = direct
                .refs
                .iter()
                .map(|reference| context.messages[reference].id.clone())
                .collect();
            let members: Vec<_> = messages
                .iter()
                .filter(|message| ids.contains(&message.message.id))
                .cloned()
                .collect();
            let references: Vec<_> = messages
                .iter()
                .enumerate()
                .filter(|(_, message)| ids.contains(&message.message.id))
                .map(|(index, _)| format!("n{}", index + 1))
                .collect();
            let extraction = remap_extraction(direct.extraction, &references);
            let mut topic = new_topic(chat, run, index as u32, &members, extraction.title.clone());
            topic.provisional = false;
            let local = extract::request(&members, &[], &[], &topic.title, false);
            let items: Vec<_> = extract::insights(
                &extraction,
                &local,
                &snapshot.chat,
                &topic.id,
                run,
                &extract::Signals::default(),
                Utc::now().fixed_offset(),
            )
            .into_iter()
            .filter(|(item, _)| item.kind != InsightKind::MentionMe)
            .collect();
            runtime.decision(
                AgentAction::Verify {
                    insight_ids: items.iter().map(|(item, _)| item.id.clone()).collect(),
                },
                observation(
                    0,
                    &VecDeque::new(),
                    &[],
                    runtime.steps,
                    runtime.stats.cost_usd,
                    0.0,
                ),
                "B0 post-hoc evidence labels only; no corrective generation or filtering",
                sink,
            )?;
            let ids: Vec<_> = ids.into_iter().collect();
            session.assign(&topic, &ids, "baseline_b0")?;
            assigned = true;
            let relations = extraction
                .relations
                .as_ref()
                .map(|rows| crate::relations::proposals(rows, &local, chat, &topic.id, run));
            let committed =
                session.commit_topic_with_relations(&topic, &ids, items, relations.as_deref())?;
            runtime.stats.messages_analyzed += ids.len() as u64;
            runtime.stats.topics_created += 1;
            for item in &committed {
                runtime.stats.insights.created += 1;
                match item.verification_status {
                    VerificationStatus::Verified => runtime.stats.insights.verified += 1,
                    VerificationStatus::Unverified => runtime.stats.insights.unverified += 1,
                    _ => runtime.stats.insights.rejected += 1,
                }
                order.push(item.id.clone());
            }
            sink(EventBody::Progress(ProgressPayload {
                stage: "store".into(),
                current: runtime.stats.messages_analyzed,
                total: Some(messages.len() as u64),
                message: "Committed B0 topic".into(),
            }))?;
        }
        Ok(FinishReason::Done)
    })();
    let reason = match processing {
        Ok(reason) => reason,
        Err(EngineError::BudgetExceeded) => FinishReason::BudgetExceeded,
        Err(EngineError::Cancelled) => FinishReason::Cancelled,
        Err(error) => {
            stats.elapsed_ms = start.elapsed().as_millis() as u64;
            session.finish_with_stats(
                if assigned {
                    RunStatus::Partial
                } else {
                    RunStatus::Failed
                },
                &stats,
            )?;
            return Err(error);
        }
    };
    let status = match reason {
        FinishReason::Done => RunStatus::Complete,
        FinishReason::Cancelled => RunStatus::Cancelled,
        _ => RunStatus::Partial,
    };
    stats.elapsed_ms = start.elapsed().as_millis() as u64;
    // Finalize before emitting optional presentation events; an output failure
    // cannot erase committed baseline counters or require another cloud request.
    session.finish_with_stats(status, &stats)?;
    let inbox = store::inbox(
        path,
        chat,
        &store::InboxOptions {
            all: true,
            include_resolved: true,
            include_rejected: true,
            now: Utc::now().fixed_offset(),
        },
    )?;
    for topic in inbox.topics {
        sink(EventBody::Topic(topic))?;
    }
    let mut items: BTreeMap<_, _> = inbox
        .insights
        .into_iter()
        .map(|payload| (payload.insight.id.clone(), payload))
        .collect();
    for id in order {
        if let Some(payload) = items.remove(&id) {
            sink(EventBody::Insight(payload))?;
        }
    }
    Ok(AnalysisResult {
        status,
        reason,
        stats,
    })
}
