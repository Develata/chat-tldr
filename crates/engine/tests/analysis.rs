//! Cross-module behavior tests: only synthetic input and in-process model doubles.
use std::{
    cell::Cell,
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use chat_tldr_core::{
    Assignee, ChatId, EventBody, FinishReason, InsightId, InsightKind, Lifecycle, Priority, RunId,
    RunStatus, TopicState, VerificationStatus,
};
use chat_tldr_engine::{
    Config,
    agent::{self, AnalysisResult, AnalyzeOptions, Models},
    decider::{Answer, Decider, DecisionRequest, DecisionResponse, Question, validate_response},
    extract::{self, Signals},
    llm::{LlmClient, LlmRequest, LlmResponse, MockLlm, ProviderError, Usage},
    store::{self, AnalysisSession, ImportOptions, InboxOptions, TopicRecord},
};
use chat_tldr_qce::{QceOptions, parse_qce_json};
use chrono::DateTime;
use serde_json::{Value, json};
use tempfile::TempDir;

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/qce/synthetic-group.json");

#[path = "analysis/scenario.rs"]
mod scenario;

struct Workspace {
    _directory: TempDir,
    database: PathBuf,
    chat: ChatId,
    config: Config,
}

impl Workspace {
    fn new() -> Self {
        Self::from_source(FIXTURE)
    }

    fn from_source(source: &[u8]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("chat-tldr.db");
        let batch = parse_qce_json(source, &QceOptions::default()).unwrap();
        let chat = batch.chat.chat_id.clone();
        store::import_batches(&database, &[batch], &ImportOptions::default()).unwrap();
        Self {
            _directory: directory,
            database,
            chat,
            config: Config::parse("").unwrap(),
        }
    }

    fn options(&self) -> AnalyzeOptions {
        let mut options = AnalyzeOptions::from_config(&self.config);
        options.budget_usd = 10.0;
        options
    }

    fn analyze(
        &self,
        run: &str,
        llm: &dyn LlmClient,
        decider: &dyn Decider,
        options: &AnalyzeOptions,
    ) -> (AnalysisResult, Vec<EventBody>) {
        let mut events = Vec::new();
        let result = agent::analyze(
            &self.database,
            &self.chat,
            &RunId::from(run),
            &self.config,
            options,
            Models {
                llm,
                primary: Some(decider),
                fallback: decider,
            },
            &AtomicBool::new(false),
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        (result, events)
    }

    fn inbox(&self, include_rejected: bool) -> store::InboxSnapshot {
        store::inbox(
            &self.database,
            &self.chat,
            &InboxOptions {
                all: false,
                include_resolved: false,
                include_rejected,
                now: DateTime::parse_from_rfc3339("2026-09-26T12:00:00+08:00").unwrap(),
            },
        )
        .unwrap()
    }

    fn states(&self) -> Vec<String> {
        store::analysis_snapshot(&self.database, &self.chat)
            .unwrap()
            .messages
            .into_iter()
            .map(|message| message.analysis_state)
            .collect()
    }
}

#[derive(Default)]
struct SyntheticDecider {
    calls: Cell<usize>,
}

impl Decider for SyntheticDecider {
    fn name(&self) -> &str {
        "synthetic-complete-decider"
    }

    fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        request.validate()?;
        self.calls.set(self.calls.get() + 1);
        let answers = request
            .questions
            .iter()
            .map(|(id, question)| {
                let answer = match question {
                    Question::Noul { .. } => Answer::Noul {
                        p_yes: if id == "chitchat" { 0.01 } else { 0.95 },
                    },
                    Question::Choice { options, .. } => {
                        let selected = options.keys().next().unwrap();
                        Answer::Choice {
                            choice: selected.clone(),
                            probabilities: options
                                .keys()
                                .map(|key| (key.clone(), if key == selected { 1.0 } else { 0.0 }))
                                .collect(),
                            confidence: 1.0,
                        }
                    }
                    Question::Score { levels, .. } => {
                        let selected = levels.len() - 1;
                        Answer::Score {
                            score: selected as f32,
                            probabilities: (0..levels.len())
                                .map(|index| {
                                    (index.to_string(), if index == selected { 1.0 } else { 0.0 })
                                })
                                .collect(),
                            confidence: 1.0,
                        }
                    }
                };
                (id.clone(), answer)
            })
            .collect::<BTreeMap<_, _>>();
        let response = DecisionResponse {
            model: self.name().into(),
            answers,
            usage: Usage::default(),
        };
        validate_response(request, &response)?;
        Ok(response)
    }
}

fn topic(quote: &str) -> Value {
    json!({
        "title":"合成报告", "summary":"", "items":[{
            "op":"new", "existing_ref":null, "kind":"todo", "title":"他人提交合成报告",
            "summary":"有截止日期的事项保留他人负责属性。", "assignee":"other",
            "deadline_raw":"周五前", "deadline_date_guess":null,
            "evidence":[{"ref":"n1","quote":quote}]
        }]
    })
}

fn direct(mut topic: Value) -> Value {
    topic["refs"] = json!(["n1", "n2"]);
    json!({"topics":[topic]})
}

fn response(value: Value) -> Result<LlmResponse, ProviderError> {
    Ok(LlmResponse {
        content: value.to_string(),
        usage: Usage {
            input_tokens: 100,
            output_tokens: 30,
            cost_usd: 0.0001,
        },
    })
}

fn successful_llm() -> MockLlm {
    MockLlm::new(vec![response(direct(topic("周五前交合成报告。")))])
}

#[test]
fn imported_messages_become_verified_other_assignee_p0_and_repeat_makes_no_model_calls() {
    let workspace = Workspace::new();
    let llm = successful_llm();
    let decider = SyntheticDecider::default();
    let (first, events) = workspace.analyze("r_complete", &llm, &decider, &workspace.options());
    assert_eq!(first.status, RunStatus::Complete);
    assert_eq!(first.reason, FinishReason::Done);
    assert_eq!(first.stats.messages_analyzed, 2);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
    let inbox = workspace.inbox(false);
    let item = inbox
        .insights
        .iter()
        .find(|payload| payload.insight.kind == InsightKind::Todo)
        .unwrap();
    assert_eq!(item.insight.priority, Priority::P0);
    assert_eq!(item.insight.assignee, Assignee::Other);
    assert_eq!(
        item.insight.verification_status,
        VerificationStatus::Verified
    );
    assert!(item.insight.deadline.as_ref().unwrap().bound_date.is_some());
    assert!(!item.evidence_view.is_empty());
    assert!(
        item.evidence_view
            .iter()
            .all(|evidence| evidence.ok && evidence.highlight.is_some())
    );
    assert!(inbox.meta.view_cursor.is_some());
    assert!(events.iter().any(|event| matches!(event, EventBody::Insight(payload) if payload.insight.id == item.insight.id)));
    let llm_calls = llm.requests().len();
    let decision_calls = decider.calls.get();
    let (second, _) = workspace.analyze("r_repeat", &llm, &decider, &workspace.options());
    assert_eq!(second.status, RunStatus::Complete);
    assert_eq!(second.stats.messages_analyzed, 0);
    assert_eq!(llm.requests().len(), llm_calls);
    assert_eq!(decider.calls.get(), decision_calls);
    assert_eq!(workspace.inbox(false).insights.len(), inbox.insights.len());
}

#[test]
fn empty_items_and_empty_summary_still_finish_the_topic() {
    let mut source: Value = serde_json::from_slice(FIXTURE).unwrap();
    source["messages"][0]["content"] = json!({"text":"合成普通讨论。","elements":[{"type":"text","data":{"text":"合成普通讨论。"}}]});
    let workspace = Workspace::from_source(&serde_json::to_vec(&source).unwrap());
    let llm = MockLlm::new(vec![response(direct(
        json!({"title":"无待办的合成话题","summary":"","items":[]}),
    ))]);
    let (result, _) = workspace.analyze(
        "r_empty",
        &llm,
        &SyntheticDecider::default(),
        &workspace.options(),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 2);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
    assert!(workspace.inbox(false).insights.is_empty());
    assert!(
        store::analysis_snapshot(&workspace.database, &workspace.chat)
            .unwrap()
            .insights
            .is_empty()
    );
}

#[test]
fn invented_quote_is_retried_then_rejected_and_hidden_from_default_inbox() {
    let workspace = Workspace::new();
    let unsupported = topic("完全虚构且不存在的引文");
    let llm = MockLlm::new(vec![
        response(direct(unsupported.clone())),
        response(unsupported),
    ]);
    let (result, _) = workspace.analyze(
        "r_rejected",
        &llm,
        &SyntheticDecider::default(),
        &workspace.options(),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(
        llm.requests().len(),
        2,
        "verification gets one correction attempt"
    );
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let rejected = snapshot
        .insights
        .iter()
        .find(|item| item.kind == InsightKind::Todo)
        .unwrap();
    assert_eq!(rejected.verification_status, VerificationStatus::Rejected);
    assert!(
        !workspace
            .inbox(false)
            .insights
            .iter()
            .any(|item| item.insight.id == rejected.id)
    );
    assert!(
        workspace
            .inbox(true)
            .insights
            .iter()
            .any(|item| item.insight.id == rejected.id)
    );
}

#[test]
fn provider_failure_is_partial_and_a_new_run_recovers_failed_messages() {
    let workspace = Workspace::new();
    let failed = MockLlm::new(vec![Err(ProviderError::new(
        "E_PROVIDER_TIMEOUT",
        true,
        "synthetic timeout",
    ))]);
    let decider = SyntheticDecider::default();
    let (first, events) = workspace.analyze("r_failure", &failed, &decider, &workspace.options());
    assert_eq!(first.status, RunStatus::Partial);
    assert_eq!(first.reason, FinishReason::Error);
    assert_eq!(workspace.states(), ["failed", "failed", "skipped"]);
    assert!(workspace.inbox(false).meta.view_cursor.is_none());
    assert!(events.iter().any(
        |event| matches!(event, EventBody::Error(payload) if payload.code == "E_PROVIDER_TIMEOUT")
    ));
    let (recovered, _) = workspace.analyze(
        "r_recovered",
        &successful_llm(),
        &decider,
        &workspace.options(),
    );
    assert_eq!(recovered.status, RunStatus::Complete);
    assert_eq!(recovered.stats.messages_analyzed, 2);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
}

#[test]
fn step_limit_keeps_unverified_messages_pending_and_next_run_resumes() {
    let workspace = Workspace::new();
    let mut options = workspace.options();
    options.max_steps = 1;
    let decider = SyntheticDecider::default();
    let (first, _) = workspace.analyze("r_step_limit", &successful_llm(), &decider, &options);
    assert_eq!(first.status, RunStatus::Partial);
    assert_eq!(first.reason, FinishReason::MaxSteps);
    assert_eq!(first.stats.messages_analyzed, 0);
    assert_eq!(workspace.states(), ["pending", "pending", "skipped"]);
    assert!(workspace.inbox(false).meta.view_cursor.is_none());
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    assert!(
        snapshot
            .messages
            .iter()
            .take(2)
            .all(|message| message.topic_id.is_some())
    );
    let llm = MockLlm::new(vec![response(topic("周五前交合成报告。"))]);
    let (resumed, _) = workspace.analyze("r_step_resume", &llm, &decider, &workspace.options());
    assert_eq!(resumed.status, RunStatus::Complete);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
    assert!(
        llm.requests().is_empty(),
        "the mapped direct draft must survive the checkpoint"
    );
}

#[test]
fn tiny_budget_stops_before_network_and_keeps_messages_pending() {
    let workspace = Workspace::new();
    let mut options = workspace.options();
    options.budget_usd = f64::EPSILON;
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::default();
    let (result, _) = workspace.analyze("r_budget", &llm, &decider, &options);
    assert_eq!(result.status, RunStatus::Partial);
    assert_eq!(result.reason, FinishReason::BudgetExceeded);
    assert_eq!(result.stats.messages_analyzed, 0);
    assert!(llm.requests().is_empty());
    assert_eq!(decider.calls.get(), 0);
    assert_eq!(workspace.states(), ["pending", "pending", "skipped"]);
}

#[test]
fn invalid_extraction_responses_do_not_poison_the_next_run() {
    let workspace = Workspace::new();
    let invalid = json!({"topics": []}); // Valid JSON, but it omits the new messages.
    let failed_llm = MockLlm::new(vec![response(invalid.clone()), response(invalid)]);
    let decider = SyntheticDecider::default();
    let (failed, _) = workspace.analyze(
        "r_invalid_cached_output",
        &failed_llm,
        &decider,
        &workspace.options(),
    );
    assert_eq!(failed.status, RunStatus::Partial);
    assert_eq!(failed_llm.requests().len(), 2);
    assert_eq!(workspace.states(), ["failed", "failed", "skipped"]);

    let recovered_llm = successful_llm();
    let (recovered, _) = workspace.analyze(
        "r_valid_after_invalid_output",
        &recovered_llm,
        &decider,
        &workspace.options(),
    );
    assert_eq!(recovered.status, RunStatus::Complete);
    assert_eq!(recovered_llm.requests().len(), 1);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
}

#[test]
fn legacy_invalid_extraction_cache_is_evicted_before_a_fresh_attempt() {
    let workspace = Workspace::new();
    let llm = MockLlm::new(vec![
        response(json!({"topics": []})),
        response(json!({"topics": []})),
    ]);
    let decider = SyntheticDecider::default();
    workspace.analyze(
        "r_failed_before_upgrade",
        &llm,
        &decider,
        &workspace.options(),
    );
    {
        let mut session = AnalysisSession::begin(
            &workspace.database,
            &workspace.chat,
            &"r_legacy_cache_seed".into(),
            &json!({}),
        )
        .unwrap();
        // Reproduce cache records created by the previous implementation, which
        // persisted both malformed attempts before checking message membership.
        for request in llm.requests() {
            let key = blake3::hash(
                format!(
                    "extract-v1:r1:{:?}:{}",
                    workspace.config.llm,
                    serde_json::to_string(&request).unwrap()
                )
                .as_bytes(),
            )
            .to_hex()
            .to_string();
            session
                .cache_put(
                    &key,
                    "extract",
                    "llm",
                    &workspace.config.llm.model,
                    &response(json!({"topics": []})).unwrap(),
                )
                .unwrap();
        }
        session.finish(RunStatus::Complete).unwrap();
    }
    let recovered_llm = successful_llm();
    let (recovered, _) = workspace.analyze(
        "r_recover_after_upgrade",
        &recovered_llm,
        &decider,
        &workspace.options(),
    );
    assert_eq!(recovered.status, RunStatus::Complete);
    assert_eq!(recovered_llm.requests().len(), 1);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
}

#[test]
fn jev_reservation_uses_the_configured_input_price() {
    let mut workspace = Workspace::new();
    workspace.config.llm.price_input_per_mtok = Some(0.0);
    workspace.config.llm.price_output_per_mtok = Some(0.0);
    workspace.config.jev.price_input_per_mtok = Some(1000.0);
    let mut options = workspace.options();
    options.budget_usd = 0.01;
    let decider = SyntheticDecider::default();
    let (result, _) = workspace.analyze(
        "r_configured_jev_budget",
        &successful_llm(),
        &decider,
        &options,
    );
    assert_eq!(result.reason, FinishReason::BudgetExceeded);
    assert_eq!(decider.calls.get(), 0);
    assert_eq!(workspace.states(), ["pending", "pending", "skipped"]);
}

#[test]
fn broken_event_sink_after_commit_preserves_exact_insight_history() {
    let workspace = Workspace::new();
    let run = RunId::from("r_broken_pipe_after_commit");
    let llm = successful_llm();
    let decider = SyntheticDecider::default();
    let result = agent::analyze(
        &workspace.database,
        &workspace.chat,
        &run,
        &workspace.config,
        &workspace.options(),
        Models {
            llm: &llm,
            primary: Some(&decider),
            fallback: &decider,
        },
        &AtomicBool::new(false),
        &mut |event| {
            if matches!(event, EventBody::Progress(ref progress) if progress.message == "Committed topic checkpoint")
            {
                return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into());
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    let saved = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    assert!(!saved.insights.is_empty());
    let history = store::stats(&workspace.database, None, Some(&run)).unwrap();
    assert_eq!(history.detail["run_status"], "partial");
    assert!(history.warnings.is_empty());
    let chat_tldr_core::StatsPayload::Run(stats) = &history.rows[0] else {
        panic!("expected exact run statistics");
    };
    assert_eq!(stats.messages_analyzed, 2);
    assert_eq!(stats.insights.created, saved.insights.len() as u64);
    assert_eq!(stats.insights.verified, saved.insights.len() as u64);
}

#[test]
fn same_chat_allows_only_one_live_analysis_session() {
    let workspace = Workspace::new();
    let first = AnalysisSession::begin(
        &workspace.database,
        &workspace.chat,
        &"r_locked_first".into(),
        &json!({}),
    )
    .unwrap();
    let error = AnalysisSession::begin(
        &workspace.database,
        &workspace.chat,
        &"r_locked_second".into(),
        &json!({}),
    )
    .err()
    .expect("a second live session must fail");
    assert_eq!(error.code(), "E_RUN_IN_PROGRESS");
    drop(first);
    let third = AnalysisSession::begin(
        &workspace.database,
        &workspace.chat,
        &"r_lock_released".into(),
        &json!({}),
    )
    .unwrap();
    drop(third);
}

struct ImportingLlm {
    database: PathBuf,
    imported: Cell<bool>,
    inner: MockLlm,
}

impl LlmClient for ImportingLlm {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        if !self.imported.replace(true) {
            let mut source: Value = serde_json::from_slice(FIXTURE).unwrap();
            let mut late = source["messages"][0].clone();
            late["id"] = json!("synthetic-late-during-analysis");
            late["seq"] = json!("4");
            late["timestamp"] = json!(1790400010000_i64);
            late["content"] = json!({"text":"分析期间新到的合成消息。","elements":[{"type":"text","data":{"text":"分析期间新到的合成消息。"}}]});
            source["messages"] = json!([late]);
            let batch = parse_qce_json(
                &serde_json::to_vec(&source).unwrap(),
                &QceOptions::default(),
            )
            .unwrap();
            // Success here also proves no SQLite write transaction is held while
            // waiting for the model response.
            store::import_batches(&self.database, &[batch], &ImportOptions::default()).unwrap();
        }
        self.inner.complete(request)
    }
}

#[test]
fn messages_imported_during_model_call_stay_outside_the_analysis_snapshot() {
    let workspace = Workspace::new();
    let llm = ImportingLlm {
        database: workspace.database.clone(),
        imported: Cell::new(false),
        inner: successful_llm(),
    };
    let (result, _) = workspace.analyze(
        "r_snapshot",
        &llm,
        &SyntheticDecider::default(),
        &workspace.options(),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 2);
    assert!(llm.imported.get());
    assert_eq!(workspace.states(), ["done", "done", "skipped", "pending"]);
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let late = snapshot.messages.last().unwrap();
    assert_eq!(
        late.message.source.qce_id.as_deref(),
        Some("synthetic-late-during-analysis")
    );
    assert!(late.topic_id.is_none());
    assert!(workspace.inbox(false).meta.view_cursor.unwrap() < late.cursor);
}

#[test]
fn time_window_is_half_open_and_leaves_unselected_messages_pending() {
    let workspace = Workspace::new();
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let mut options = workspace.options();
    options.since = Some(snapshot.messages[1].message.sent_at);
    options.until = Some(snapshot.messages[2].message.sent_at);
    // An external reply target does not establish interleaving in this window.
    // The single eligible message can therefore use direct extraction.
    let llm = MockLlm::new(vec![response(
        json!({"topics":[{"refs":["n1"],"title":"单条合成回复","summary":"","items":[]}]}),
    )]);
    let (result, _) = workspace.analyze(
        "r_time_window",
        &llm,
        &SyntheticDecider::default(),
        &options,
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 1);
    assert_eq!(workspace.states(), ["pending", "done", "skipped"]);
    assert!(
        workspace.inbox(false).meta.view_cursor.is_none(),
        "an unanalyzed prefix cannot be marked read"
    );
}

#[test]
fn interleaved_replies_inside_one_burst_select_segment_and_are_logged() {
    let mut source: Value = serde_json::from_slice(FIXTURE).unwrap();
    let template = source["messages"][0].clone();
    source["messages"] = Value::Array(["a", "c", "d", "e", "b"].iter().enumerate().map(|(index, sender)| {
        let mut message = template.clone();
        message["id"] = json!(format!("interleave-{index}"));
        message["seq"] = json!(index.to_string());
        message["timestamp"] = json!(1790400000000_i64 + index as i64 * 1000);
        message["sender"] = json!({"uid":format!("u_{sender}"),"name":sender});
        message["content"] = json!({"text":"Synthetic message", "elements":[{"type":"text","data":{"text":"Synthetic message"}}]});
        if index == 4 {
            message["content"]["elements"].as_array_mut().unwrap().push(json!({"type":"reply","data":{"referencedMessageId":"interleave-0"}}));
        }
        message
    }).collect());
    let workspace = Workspace::from_source(&serde_json::to_vec(&source).unwrap());
    let llm = MockLlm::new(vec![response(
        json!({"title":"Interleaved topic","summary":"","items":[]}),
    )]);
    let (result, events) = workspace.analyze(
        "r_interleave",
        &llm,
        &SyntheticDecider::default(),
        &workspace.options(),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 5);
    let decision = events
        .iter()
        .find_map(|event| {
            if let EventBody::Decision(decision) = event {
                Some(decision)
            } else {
                None
            }
        })
        .unwrap();
    assert!(matches!(
        decision.chosen,
        chat_tldr_core::AgentAction::Segment { .. }
    ));
    assert_eq!(decision.observation.interleave, 1.0);
}

#[test]
fn cross_burst_mention_keeps_linked_older_topic_when_candidate_count_is_limited() {
    let mut source: Value = serde_json::from_slice(FIXTURE).unwrap();
    source["messages"][2]["recalled"] = json!(false);
    source["messages"][2]["sender"] = json!({"uid":"u_third_fixture","name":"合成丙"});
    source["messages"][2]["content"] = json!({"text":"@合成甲 继续项目讨论。", "elements":[
        {"type":"at","data":{"uid":"u_alice_fixture","name":"合成甲"}},
        {"type":"text","data":{"text":" 继续项目讨论。"}}
    ]});
    let mut workspace = Workspace::from_source(&serde_json::to_vec(&source).unwrap());
    workspace.config.agent.direct_max = 0;
    workspace.config.segment.all_candidates_max = 1;
    workspace.config.segment.candidate_k = 1;
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let session = AnalysisSession::begin(
        &workspace.database,
        &workspace.chat,
        &"r_seed_topics".into(),
        &json!({}),
    )
    .unwrap();
    for (index, id) in ["older_linked", "newer_unlinked"].into_iter().enumerate() {
        let topic = TopicRecord {
            id: id.into(),
            chat_id: workspace.chat.clone(),
            title: id.into(),
            provisional: false,
            state: TopicState::Active,
            last_message_at: snapshot.messages[index].message.sent_at,
            is_chitchat: None,
        };
        let ids = [snapshot.messages[index].message.id.clone()];
        session.assign(&topic, &ids, "synthetic").unwrap();
        session.commit_topic(&topic, &ids, vec![]).unwrap();
    }
    drop(session);
    let llm = MockLlm::new(vec![response(
        json!({"title":"Continued discussion","summary":"","items":[]}),
    )]);
    let (result, _) = workspace.analyze(
        "r_linked_topic",
        &llm,
        &SyntheticDecider::default(),
        &workspace.options(),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 1);
    assert_eq!(result.stats.topics_created, 0);
    let after = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    assert_eq!(
        after.messages[2].topic_id.as_ref().unwrap().as_ref(),
        "older_linked"
    );
    assert_eq!(after.messages[1].topic_id, Some("newer_unlinked".into()));
    let history = store::jev_log(&workspace.database, &"r_linked_topic".into()).unwrap();
    let attribution = history
        .rows
        .iter()
        .find(|answer| answer.question_id == "topic")
        .unwrap();
    assert_eq!(attribution.subject.candidates, vec!["older_linked".into()]);
}

#[test]
fn until_excludes_an_ordinary_message_exactly_at_the_boundary() {
    let workspace = Workspace::new();
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let mut options = workspace.options();
    options.since = Some(snapshot.messages[0].message.sent_at);
    options.until = Some(snapshot.messages[1].message.sent_at);
    let llm = MockLlm::new(vec![response(
        json!({"topics":[{"refs":["n1"],"title":"边界内合成通知","summary":"","items":[]}]}),
    )]);
    let (result, _) = workspace.analyze(
        "r_until_boundary",
        &llm,
        &SyntheticDecider::default(),
        &options,
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 1);
    assert_eq!(workspace.states(), ["done", "pending", "skipped"]);
    assert_eq!(
        workspace.inbox(false).meta.view_cursor,
        Some(snapshot.messages[0].cursor)
    );
}

#[test]
fn reversed_direct_membership_preserves_evidence_message_identity() {
    let workspace = Workspace::new();
    let first_id = store::analysis_snapshot(&workspace.database, &workspace.chat)
        .unwrap()
        .messages[0]
        .message
        .id
        .clone();
    let mut output = direct(topic("周五前交合成报告。"));
    output["topics"][0]["refs"] = json!(["n2", "n1"]);
    let llm = MockLlm::new(vec![response(output)]);
    let (result, _) = workspace.analyze(
        "r_reversed_refs",
        &llm,
        &SyntheticDecider::default(),
        &workspace.options(),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(
        llm.requests().len(),
        1,
        "membership order must not force a correction request"
    );
    let inbox = workspace.inbox(false);
    let todo = inbox
        .insights
        .iter()
        .find(|item| item.insight.kind == InsightKind::Todo)
        .unwrap();
    assert_eq!(
        todo.insight.verification_status,
        VerificationStatus::Verified
    );
    assert_eq!(todo.insight.evidence[0].message_id, first_id);
}

#[derive(Default)]
struct FailingDecider {
    calls: Cell<usize>,
}

impl Decider for FailingDecider {
    fn name(&self) -> &str {
        "synthetic-unavailable-primary"
    }

    fn decide(&self, _: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        self.calls.set(self.calls.get() + 1);
        Err(ProviderError::new(
            "E_PROVIDER_TIMEOUT",
            true,
            "synthetic unavailable primary",
        ))
    }
}

#[test]
fn classifier_provider_failure_uses_fallback_and_reports_the_degradation() {
    let workspace = Workspace::new();
    let llm = successful_llm();
    let primary = FailingDecider::default();
    let fallback = SyntheticDecider::default();
    let mut events = Vec::new();
    let result = agent::analyze(
        &workspace.database,
        &workspace.chat,
        &"r_classify_fallback".into(),
        &workspace.config,
        &workspace.options(),
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &AtomicBool::new(false),
        &mut |event| {
            events.push(event);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(primary.calls.get(), 1);
    assert_eq!(fallback.calls.get(), 1);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
    assert!(events.iter().any(
        |event| matches!(event, EventBody::Warning(payload) if payload.code == "W_DECIDER_FALLBACK")
    ));
}

struct RecallingLlm {
    database: PathBuf,
    recalled: Cell<bool>,
    inner: MockLlm,
}

impl LlmClient for RecallingLlm {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        if !self.recalled.replace(true) {
            let mut source: Value = serde_json::from_slice(FIXTURE).unwrap();
            source["messages"][0]["recalled"] = json!(true);
            source["messages"] = json!([source["messages"][0].clone()]);
            let batch = parse_qce_json(
                &serde_json::to_vec(&source).unwrap(),
                &QceOptions::default(),
            )
            .unwrap();
            store::import_batches(&self.database, &[batch], &ImportOptions::default()).unwrap();
        }
        self.inner.complete(request)
    }
}

#[test]
fn recall_during_model_call_cannot_commit_a_verified_claim_from_stale_evidence() {
    let workspace = Workspace::new();
    let llm = RecallingLlm {
        database: workspace.database.clone(),
        recalled: Cell::new(false),
        inner: successful_llm(),
    };
    let (result, events) = workspace.analyze(
        "r_recall_race",
        &llm,
        &SyntheticDecider::default(),
        &workspace.options(),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert!(llm.recalled.get());
    assert_eq!(workspace.states(), ["skipped", "done", "skipped"]);
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    assert!(snapshot.messages[0].message.text.is_empty());
    assert!(
        snapshot
            .insights
            .iter()
            .filter(|item| item.kind == InsightKind::Todo)
            .all(|item| item.verification_status == VerificationStatus::Rejected)
    );
    assert!(
        !workspace
            .inbox(false)
            .insights
            .iter()
            .any(|item| item.insight.kind == InsightKind::Todo)
    );
    let emitted = events
        .iter()
        .find_map(|event| match event {
            EventBody::Insight(payload) if payload.insight.kind == InsightKind::Todo => {
                Some(payload)
            }
            _ => None,
        })
        .expect("rejected insight is still emitted for audit");
    assert_eq!(
        emitted.insight.verification_status,
        VerificationStatus::Rejected
    );
    assert!(!emitted.evidence_view.is_empty());
    assert!(
        emitted
            .evidence_view
            .iter()
            .all(|view| !view.ok && view.highlight.is_none() && view.display_text.is_empty())
    );
}

struct CancellingLlm<'a> {
    cancel: &'a AtomicBool,
    inner: MockLlm,
}

impl LlmClient for CancellingLlm<'_> {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        let result = self.inner.complete(request);
        self.cancel.store(true, Ordering::Relaxed);
        result
    }
}

#[test]
fn cancellation_during_model_call_does_not_commit_or_call_another_model() {
    let workspace = Workspace::new();
    let cancel = AtomicBool::new(false);
    let llm = CancellingLlm {
        cancel: &cancel,
        inner: successful_llm(),
    };
    let decider = SyntheticDecider::default();
    let mut events = Vec::new();
    let result = agent::analyze(
        &workspace.database,
        &workspace.chat,
        &"r_cancel_during_model".into(),
        &workspace.config,
        &workspace.options(),
        Models {
            llm: &llm,
            primary: Some(&decider),
            fallback: &decider,
        },
        &cancel,
        &mut |event| {
            events.push(event);
            Ok(())
        },
    )
    .unwrap();
    assert!(cancel.load(Ordering::Relaxed));
    assert_eq!(result.status, RunStatus::Cancelled);
    assert_eq!(result.reason, FinishReason::Cancelled);
    assert_eq!(result.stats.messages_analyzed, 0);
    assert_eq!(workspace.states(), ["pending", "pending", "skipped"]);
    assert!(workspace.inbox(false).insights.is_empty());
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EventBody::Insight(_)))
    );
    assert_eq!(llm.inner.requests().len(), 1);
    assert_eq!(decider.calls.get(), 0);
}

#[test]
fn cross_chat_or_topic_commit_rolls_back_items_topic_and_message_states() {
    let workspace = Workspace::new();
    let original = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let first_id = original.messages[0].message.id.clone();
    let second_id = original.messages[1].message.id.clone();
    let mut source: Value = serde_json::from_slice(FIXTURE).unwrap();
    source["chatInfo"]["peerUid"] = json!("synthetic-foreign-chat");
    let foreign = parse_qce_json(
        &serde_json::to_vec(&source).unwrap(),
        &QceOptions::default(),
    )
    .unwrap();
    let foreign_chat = foreign.chat.chat_id.clone();
    let foreign_id = foreign.messages[0].id.clone();
    store::import_batches(&workspace.database, &[foreign], &ImportOptions::default()).unwrap();
    let run = RunId::from("r_atomic_topic");
    let session =
        AnalysisSession::begin(&workspace.database, &workspace.chat, &run, &json!({})).unwrap();
    let first_topic = TopicRecord {
        id: "t_atomic_a".into(),
        chat_id: workspace.chat.clone(),
        title: "原始话题标题".into(),
        provisional: true,
        state: TopicState::Active,
        last_message_at: original.messages[0].message.sent_at,
        is_chitchat: None,
    };
    let mut second_topic = first_topic.clone();
    second_topic.id = "t_atomic_b".into();
    session
        .assign(&first_topic, std::slice::from_ref(&first_id), "direct")
        .unwrap();
    session
        .assign(&second_topic, std::slice::from_ref(&second_id), "direct")
        .unwrap();
    let before = session.snapshot().unwrap();
    let topics_before = serde_json::to_value(&before.topics).unwrap();
    let context = extract::request(&original.messages[..1], &[], &[], "", false);
    let output = extract::parse_topic(&topic("周五前交合成报告。").to_string(), &context).unwrap();
    let valid = extract::insights(
        &output,
        &context,
        &before.chat,
        &first_topic.id,
        &run,
        &Signals::default(),
        original.messages[0].message.sent_at,
    )
    .into_iter()
    .find(|(insight, _)| insight.kind == InsightKind::Todo)
    .unwrap();
    let mut attempted_topic = first_topic.clone();
    attempted_topic.title = "必须随错误一起回滚的标题".into();
    attempted_topic.provisional = false;
    let assert_rollback = || {
        let after = session.snapshot().unwrap();
        assert_eq!(serde_json::to_value(after.topics).unwrap(), topics_before);
        assert!(after.insights.is_empty());
        assert_eq!(workspace.states(), ["pending", "pending", "skipped"]);
        assert!(workspace.inbox(false).meta.view_cursor.is_none());
        let outside = store::analysis_snapshot(&workspace.database, &foreign_chat).unwrap();
        assert!(
            outside
                .messages
                .iter()
                .take(2)
                .all(|message| message.analysis_state == "pending")
        );
        assert!(outside.insights.is_empty());
    };

    let mut wrong_chat_topic = attempted_topic.clone();
    wrong_chat_topic.chat_id = foreign_chat.clone();
    assert!(
        session
            .commit_topic(
                &wrong_chat_topic,
                std::slice::from_ref(&first_id),
                vec![valid.clone()]
            )
            .is_err()
    );
    assert_rollback();
    for outside_id in [second_id, foreign_id] {
        assert!(
            session
                .commit_topic(
                    &attempted_topic,
                    &[first_id.clone(), outside_id],
                    vec![valid.clone()]
                )
                .is_err()
        );
        assert_rollback();
    }
    for cross_chat in [false, true] {
        let mut invalid = valid.clone();
        invalid.0.id = "i_atomic_invalid".into();
        if cross_chat {
            invalid.0.chat_id = foreign_chat.clone();
        } else {
            invalid.0.topic_id = Some(second_topic.id.clone());
        }
        // The first item and topic title have already been written when the
        // second invalid item aborts the transaction; neither may leak out.
        assert!(
            session
                .commit_topic(
                    &attempted_topic,
                    std::slice::from_ref(&first_id),
                    vec![valid.clone(), invalid]
                )
                .is_err()
        );
        assert_rollback();
    }
    let committed = session
        .commit_topic(&attempted_topic, &[first_id], vec![valid])
        .unwrap();
    assert_eq!(committed.len(), 1);
    assert_eq!(workspace.states(), ["done", "pending", "skipped"]);
}

struct UpdatingLlm {
    database: PathBuf,
    insight_id: InsightId,
    calls: Cell<usize>,
}

impl LlmClient for UpdatingLlm {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        self.calls.set(self.calls.get() + 1);
        let input: Value = serde_json::from_str(&request.user).unwrap();
        let existing_ref = input["open_items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["kind"] == "todo")
            .unwrap()["ref"]
            .clone();
        assert!(
            input["messages"].as_array().unwrap().iter().any(|message| {
                message["ref"].as_str().unwrap().starts_with('c')
                    && message["text"]
                        .as_str()
                        .unwrap()
                        .contains("周五前交合成报告。")
            }),
            "old evidence outside the five-message window must remain available"
        );
        assert!(store::resolve(&self.database, &self.insight_id, Lifecycle::Done).unwrap());
        let mut output = topic("合成报告周五前补充附件。");
        output["items"][0]["op"] = json!("update");
        output["items"][0]["existing_ref"] = existing_ref;
        output["items"][0]["title"] = json!("合成报告补充附件");
        response(output)
    }
}

#[test]
fn later_update_keeps_identity_old_evidence_and_concurrent_user_resolution() {
    let workspace = Workspace::new();
    let decider = SyntheticDecider::default();
    let (first, _) = workspace.analyze(
        "r_before_update",
        &successful_llm(),
        &decider,
        &workspace.options(),
    );
    assert_eq!(first.status, RunStatus::Complete);
    let before = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let previous = before
        .insights
        .iter()
        .find(|item| item.kind == InsightKind::Todo)
        .unwrap()
        .clone();
    let topic = before
        .topics
        .iter()
        .find(|topic| Some(&topic.id) == previous.topic_id.as_ref())
        .unwrap();
    let mut source: Value = serde_json::from_slice(FIXTURE).unwrap();
    let base = source["messages"][0].clone();
    let additions: Vec<Value> = (0..7)
        .map(|index| {
            let mut message = base.clone();
            message["id"] = json!(format!("synthetic-update-{index}"));
            message["seq"] = json!((index + 4).to_string());
            message["timestamp"] = json!(1790400100000_i64 + index * 1000);
            let text = if index == 6 {
                "合成报告周五前补充附件。"
            } else {
                "中间普通合成讨论。"
            };
            message["content"] =
                json!({"text":text,"elements":[{"type":"text","data":{"text":text}}]});
            message
        })
        .collect();
    source["messages"] = json!(additions);
    let batch = parse_qce_json(
        &serde_json::to_vec(&source).unwrap(),
        &QceOptions::default(),
    )
    .unwrap();
    let new_ids: Vec<_> = batch
        .messages
        .iter()
        .map(|message| message.id.clone())
        .collect();
    store::import_batches(&workspace.database, &[batch], &ImportOptions::default()).unwrap();
    {
        // Establish an existing topic with six intervening completed messages;
        // the next analyze run resumes only its final pending update.
        let mut session = AnalysisSession::begin(
            &workspace.database,
            &workspace.chat,
            &"r_seed_update".into(),
            &json!({}),
        )
        .unwrap();
        session.assign(topic, &new_ids, "direct").unwrap();
        session.commit_topic(topic, &new_ids[..6], vec![]).unwrap();
        session.finish(RunStatus::Complete).unwrap();
    }
    let llm = UpdatingLlm {
        database: workspace.database.clone(),
        insight_id: previous.id.clone(),
        calls: Cell::new(0),
    };
    let (updated, _) = workspace.analyze("r_after_update", &llm, &decider, &workspace.options());
    assert_eq!(updated.status, RunStatus::Complete);
    assert_eq!(updated.stats.messages_analyzed, 1);
    assert_eq!(llm.calls.get(), 1);
    let after = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    let todos: Vec<_> = after
        .insights
        .iter()
        .filter(|item| item.kind == InsightKind::Todo)
        .collect();
    assert_eq!(todos.len(), 1);
    let updated = todos[0];
    assert_eq!(updated.id, previous.id);
    assert_eq!(updated.created_in_run, previous.created_in_run);
    assert_eq!(updated.lifecycle, Lifecycle::Done);
    assert_eq!(updated.title, "合成报告补充附件");
    assert_eq!(updated.verification_status, VerificationStatus::Verified);
    assert_eq!(updated.evidence.len(), 2);
    assert!(updated.evidence.contains(&previous.evidence[0]));
    assert!(
        updated
            .evidence
            .iter()
            .any(|evidence| evidence.message_id == new_ids[6])
    );
    assert!(
        !workspace
            .inbox(false)
            .insights
            .iter()
            .any(|item| item.insight.id == previous.id)
    );
}
