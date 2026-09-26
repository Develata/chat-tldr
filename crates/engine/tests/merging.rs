//! Cross-module merge behavior over synthetic messages and in-process models.
use std::{
    cell::Cell,
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use chat_tldr_core::*;
use chat_tldr_engine::{
    Config, EngineError,
    agent::{self, AnalysisResult, AnalyzeOptions, Models},
    decider::{Answer, Decider, DecisionRequest, DecisionResponse, Question, validate_response},
    llm::{LlmClient, LlmResponse, MockLlm, ProviderError, Usage},
    store::{self, AnalysisSession, AnalysisSnapshot, ImportOptions, TopicRecord},
};
use chrono::Duration;
use serde_json::json;

struct Workspace {
    _directory: tempfile::TempDir,
    database: PathBuf,
    batch: ImportBatch,
    owners: Vec<usize>,
    config: Config,
}

impl Workspace {
    fn new(owners: &[usize]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("merging.db");
        let mut batch = chat_tldr_qce::parse_qce_json(
            include_bytes!("../../../fixtures/qce/synthetic-group.json"),
            &chat_tldr_qce::QceOptions::default(),
        )
        .unwrap();
        let template = batch.messages[0].clone();
        batch.messages = owners
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let mut message = template.clone();
                message.id = format!("m_integration_merge_{index}").into();
                message.source.identity = format!("qce:integration-merge-{index}");
                message.source.qce_id = Some(format!("integration-merge-{index}"));
                message.sent_at += Duration::seconds(index as i64);
                message.text = format!("合成讨论第 {index} 条。");
                message.mentions.clear();
                message.attachments.clear();
                message.forward = None;
                message.system = false;
                message.recalled = false;
                message.reply_to = (index > 0).then(|| ReplyRef {
                    source_message_id: format!("integration-merge-{}", index - 1),
                    resolved: Some(format!("m_integration_merge_{}", index - 1).into()),
                });
                message
            })
            .collect();
        store::import_batches(
            &database,
            std::slice::from_ref(&batch),
            &ImportOptions::default(),
        )
        .unwrap();
        Self {
            _directory: directory,
            database,
            batch,
            owners: owners.to_vec(),
            config: Config::parse("").unwrap(),
        }
    }

    fn seed(&self, unfinished_first: Option<&str>) {
        let mut session = AnalysisSession::begin(
            &self.database,
            &self.batch.chat.chat_id,
            &"r_seed".into(),
            &json!({}),
        )
        .unwrap();
        for owner in 0..=*self.owners.iter().max().unwrap() {
            let rows: Vec<_> = self
                .batch
                .messages
                .iter()
                .enumerate()
                .filter(|(index, _)| self.owners[*index] == owner)
                .collect();
            let topic = TopicRecord {
                id: format!("t_{owner}").into(),
                chat_id: self.batch.chat.chat_id.clone(),
                title: format!("合成话题 {owner}"),
                provisional: false,
                state: TopicState::Active,
                last_message_at: rows.last().unwrap().1.sent_at,
                is_chitchat: Some(0.0),
            };
            session
                .assign(
                    &topic,
                    &rows
                        .iter()
                        .map(|(_, message)| message.id.clone())
                        .collect::<Vec<_>>(),
                    "synthetic",
                )
                .unwrap();
            let completed: Vec<_> = rows
                .iter()
                .filter(|(index, _)| *index != 0 || unfinished_first.is_none())
                .map(|(_, message)| message.id.clone())
                .collect();
            session.commit_topic(&topic, &completed, vec![]).unwrap();
        }
        if unfinished_first == Some("failed") {
            session
                .fail_messages(&[self.batch.messages[0].id.clone()])
                .unwrap();
        }
        session.finish(RunStatus::Complete).unwrap();
    }

    fn options(&self) -> AnalyzeOptions {
        let mut options = AnalyzeOptions::from_config(&self.config);
        options.budget_usd = 10.0;
        options
    }

    fn snapshot(&self) -> AnalysisSnapshot {
        store::analysis_snapshot(&self.database, &self.batch.chat.chat_id).unwrap()
    }

    fn plan(&self, options: &AnalyzeOptions) -> serde_json::Value {
        agent::plan(&self.database, &self.batch.chat.chat_id, options).unwrap()
    }

    fn run(
        &self,
        run: &str,
        options: &AnalyzeOptions,
        llm: &dyn LlmClient,
        decider: &dyn Decider,
        cancel: &AtomicBool,
    ) -> (AnalysisResult, Vec<EventBody>) {
        let mut events = Vec::new();
        let result = agent::analyze(
            &self.database,
            &self.batch.chat.chat_id,
            &run.into(),
            &self.config,
            options,
            Models {
                llm,
                primary: Some(decider),
                fallback: decider,
            },
            cancel,
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .unwrap();
        (result, events)
    }

    fn active_topics(&self) -> usize {
        self.snapshot()
            .topics
            .iter()
            .filter(|topic| topic.state == TopicState::Active)
            .count()
    }

    fn import_followup(&self, reply_to: Option<&UnifiedMessage>) -> MessageId {
        let mut batch = self.batch.clone();
        let mut message = batch.messages.last().unwrap().clone();
        message.id = "m_integration_followup".into();
        message.source.identity = "qce:integration-followup".into();
        message.source.qce_id = Some("integration-followup".into());
        message.sent_at += Duration::seconds(60);
        message.text = "合成后续消息。".into();
        message.reply_to = reply_to.map(|target| ReplyRef {
            source_message_id: target.source.qce_id.clone().unwrap(),
            resolved: Some(target.id.clone()),
        });
        let id = message.id.clone();
        batch.messages = vec![message];
        store::import_batches(&self.database, &[batch], &ImportOptions::default()).unwrap();
        id
    }
}

struct SyntheticDecider<'a> {
    p_yes: f32,
    merge_calls: Cell<usize>,
    calls: Cell<usize>,
    on_merge: Option<&'a dyn Fn()>,
    fail_merge: bool,
}

impl SyntheticDecider<'_> {
    fn new(p_yes: f32) -> Self {
        Self {
            p_yes,
            merge_calls: Cell::new(0),
            calls: Cell::new(0),
            on_merge: None,
            fail_merge: false,
        }
    }
}

impl Decider for SyntheticDecider<'_> {
    fn name(&self) -> &str {
        "synthetic-merge-integration"
    }
    fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        self.calls.set(self.calls.get() + 1);
        if request.questions.contains_key("merge_topics") {
            self.merge_calls.set(self.merge_calls.get() + 1);
            if let Some(callback) = self.on_merge {
                callback();
            }
            if self.fail_merge {
                return Err(ProviderError::new(
                    "E_PROVIDER_UNAVAILABLE",
                    true,
                    "Synthetic transient failure",
                ));
            }
        }
        let answers = request
            .questions
            .iter()
            .map(|(key, question)| {
                let answer = match question {
                    Question::Noul { .. } => Answer::Noul {
                        p_yes: if key == "merge_topics" {
                            self.p_yes
                        } else {
                            0.0
                        },
                    },
                    Question::Score { levels, .. } => Answer::Score {
                        score: 0.0,
                        confidence: 1.0,
                        probabilities: (0..levels.len())
                            .map(|index| (index.to_string(), if index == 0 { 1.0 } else { 0.0 }))
                            .collect(),
                    },
                    Question::Choice { options, .. } => {
                        let choice = if options.contains_key("new_topic") {
                            "new_topic"
                        } else {
                            options.keys().next().unwrap()
                        };
                        Answer::Choice {
                            choice: choice.into(),
                            confidence: 1.0,
                            probabilities: options
                                .keys()
                                .map(|key| (key.clone(), if key == choice { 1.0 } else { 0.0 }))
                                .collect(),
                        }
                    }
                };
                (key.clone(), answer)
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

#[test]
fn completed_messages_merge_without_new_extraction_and_repeat_has_no_work_or_calls() {
    let workspace = Workspace::new(&[0, 1, 0, 1, 0, 1]);
    workspace.seed(None);
    let plan = workspace.plan(&workspace.options());
    assert_eq!(plan["messages"], 0);
    assert_eq!(plan["merge_candidates"], 1);
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::new(0.5);
    let (result, events) = workspace.run(
        "r_merge_only",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 0);
    assert_eq!(result.stats.topics_updated, 2);
    assert_eq!(workspace.active_topics(), 1);
    assert!(
        workspace
            .snapshot()
            .messages
            .iter()
            .all(|message| message.analysis_state == "done")
    );
    assert!(events.iter().any(|event| matches!(event, EventBody::Decision(payload) if matches!(payload.chosen, AgentAction::MergeTopics { .. }))));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, EventBody::Topic(_)))
            .count(),
        2
    );
    assert_eq!(workspace.plan(&workspace.options())["merge_candidates"], 0);
    let (repeat, _) = workspace.run(
        "r_merge_no_work",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(repeat.status, RunStatus::Complete);
    assert_eq!(repeat.stats.topics_updated, 0);
    assert_eq!(decider.merge_calls.get(), 1);
    assert!(llm.requests().is_empty());
}

#[test]
fn reply_source_scope_filters_candidates_and_negative_decision_persists_across_runs() {
    let workspace = Workspace::new(&[0, 1, 0, 1, 0, 1]);
    workspace.seed(None);
    let llm = MockLlm::new(vec![]);
    let negative = SyntheticDecider::new(0.49);
    let mut options = workspace.options();
    options.until = Some(workspace.batch.messages[1].sent_at);
    assert_eq!(workspace.plan(&options)["merge_candidates"], 0);
    workspace.run(
        "r_outside_source_scope",
        &options,
        &llm,
        &negative,
        &AtomicBool::new(false),
    );
    assert_eq!(negative.calls.get(), 0);
    options.until = None;
    options.since = Some(workspace.batch.messages[5].sent_at);
    assert_eq!(workspace.plan(&options)["merge_candidates"], 1);
    let (result, _) = workspace.run(
        "r_reject",
        &options,
        &llm,
        &negative,
        &AtomicBool::new(false),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(workspace.active_topics(), 2);
    assert_eq!(workspace.snapshot().rejected_merges.len(), 1);
    let positive = SyntheticDecider::new(1.0);
    workspace.run(
        "r_rejection_persists",
        &workspace.options(),
        &llm,
        &positive,
        &AtomicBool::new(false),
    );
    assert_eq!(positive.calls.get(), 0);
    assert_eq!(workspace.plan(&workspace.options())["merge_candidates"], 0);
}

#[test]
fn step_limit_commits_one_merge_and_next_run_finishes_remaining_candidate() {
    let workspace = Workspace::new(&[0, 1, 0, 1, 2, 1, 2]);
    workspace.seed(None);
    assert_eq!(workspace.plan(&workspace.options())["merge_candidates"], 2);
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::new(0.9);
    let mut options = workspace.options();
    options.max_steps = 1;
    let (first, _) = workspace.run(
        "r_one_step",
        &options,
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(first.status, RunStatus::Partial);
    assert_eq!(first.reason, FinishReason::MaxSteps);
    assert_eq!(first.stats.topics_updated, 2);
    assert_eq!(workspace.active_topics(), 2);
    assert_eq!(decider.merge_calls.get(), 1);
    let (second, _) = workspace.run(
        "r_resume_steps",
        &options,
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(second.status, RunStatus::Complete);
    assert_eq!(workspace.active_topics(), 1);
    assert_eq!(decider.merge_calls.get(), 2);
}

#[test]
fn exhausted_money_budget_defers_without_rejection_and_later_run_merges() {
    let workspace = Workspace::new(&[0, 1, 0]);
    workspace.seed(None);
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::new(0.8);
    let mut options = workspace.options();
    options.budget_usd = 0.000000001;
    let (first, _) = workspace.run(
        "r_no_money",
        &options,
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(first.status, RunStatus::Partial);
    assert_eq!(first.reason, FinishReason::BudgetExceeded);
    assert!(workspace.snapshot().rejected_merges.is_empty());
    assert_eq!(decider.calls.get(), 0);
    let (second, _) = workspace.run(
        "r_more_money",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(second.status, RunStatus::Complete);
    assert_eq!(workspace.active_topics(), 1);
}

#[test]
fn cancellation_after_confirmation_preserves_answer_for_cross_run_recovery() {
    let workspace = Workspace::new(&[0, 1, 0]);
    workspace.seed(None);
    let cancel = AtomicBool::new(false);
    let callback = || cancel.store(true, Ordering::Relaxed);
    let mut first_decider = SyntheticDecider::new(0.9);
    first_decider.on_merge = Some(&callback);
    let llm = MockLlm::new(vec![]);
    let (first, _) = workspace.run(
        "r_cancelled_merge",
        &workspace.options(),
        &llm,
        &first_decider,
        &cancel,
    );
    assert_eq!(first.status, RunStatus::Cancelled);
    assert_eq!(workspace.active_topics(), 2);
    assert!(workspace.snapshot().rejected_merges.is_empty());
    let first_history = store::jev_log(&workspace.database, &"r_cancelled_merge".into()).unwrap();
    assert_eq!(first_history.rows.len(), 1);
    assert_eq!(first_history.rows[0].subject.kind, SubjectKind::TopicPair);
    let resumed = MockDeciderWithoutCalls;
    cancel.store(false, Ordering::Relaxed);
    let (second, _) = workspace.run(
        "r_resume_cancelled_merge",
        &workspace.options(),
        &llm,
        &resumed,
        &cancel,
    );
    assert_eq!(second.status, RunStatus::Complete);
    assert_eq!(workspace.active_topics(), 1);
    assert!(
        second
            .stats
            .usage
            .iter()
            .any(|usage| usage.cache_hits == 1 && usage.calls == 0)
    );
    assert_eq!(
        store::jev_log(&workspace.database, &"r_resume_cancelled_merge".into())
            .unwrap()
            .rows,
        first_history.rows
    );
}

struct MockDeciderWithoutCalls;
impl Decider for MockDeciderWithoutCalls {
    fn name(&self) -> &str {
        "synthetic-merge-integration"
    }
    fn decide(&self, _: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        panic!("a resumed confirmed pair must use its cache")
    }
}

#[test]
fn pending_or_failed_members_outside_scope_prevent_merge_of_other_done_members() {
    for state in ["pending", "failed"] {
        let workspace = Workspace::new(&[0, 1, 0, 1, 0, 1]);
        workspace.seed(Some(state));
        let mut options = workspace.options();
        options.since = Some(workspace.batch.messages[1].sent_at);
        let plan = workspace.plan(&options);
        assert_eq!(plan["messages"], 0);
        assert_eq!(plan["merge_candidates"], 0);
        let llm = MockLlm::new(vec![]);
        let decider = SyntheticDecider::new(0.9);
        let (result, _) = workspace.run(
            "r_unfinished_block",
            &options,
            &llm,
            &decider,
            &AtomicBool::new(false),
        );
        assert_eq!(result.status, RunStatus::Complete);
        assert_eq!(workspace.active_topics(), 2);
        assert_eq!(workspace.snapshot().messages[0].analysis_state, state);
        assert_eq!(decider.calls.get(), 0);
    }
}

#[test]
fn newly_extracted_topics_merge_only_after_all_members_pass_verify() {
    let mut workspace = Workspace::new(&[0, 1, 0, 1, 0, 1]);
    workspace.config.agent.direct_max = 100;
    workspace.config.agent.direct_interleave_max = 1.0;
    let llm = MockLlm::new(vec![Ok(LlmResponse {
        content: json!({"topics":[
            {"refs":["n1","n3","n5"],"title":"Synthetic first","summary":"","items":[]},
            {"refs":["n2","n4","n6"],"title":"Synthetic second","summary":"","items":[]}
        ]})
        .to_string(),
        usage: Usage::default(),
    })]);
    let callback = || {
        assert!(
            workspace
                .snapshot()
                .messages
                .iter()
                .all(|message| message.analysis_state == "done")
        )
    };
    let mut decider = SyntheticDecider::new(0.9);
    decider.on_merge = Some(&callback);
    let (result, events) = workspace.run(
        "r_extract_then_merge",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 6);
    assert_eq!(result.stats.topics_created, 2);
    assert_eq!(workspace.active_topics(), 1);
    assert_eq!(decider.merge_calls.get(), 1);
    let actions: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            EventBody::Decision(payload) => Some(&payload.chosen),
            _ => None,
        })
        .collect();
    let last_verify = actions
        .iter()
        .rposition(|action| matches!(action, AgentAction::Verify { .. }))
        .unwrap();
    let first_merge = actions
        .iter()
        .position(|action| matches!(action, AgentAction::MergeTopics { .. }))
        .unwrap();
    assert!(last_verify < first_merge);
}

#[test]
fn transient_model_failure_and_oversized_state_are_partial_without_permanent_rejection() {
    for oversized in [false, true] {
        let mut workspace = Workspace::new(&[0, 1, 0]);
        workspace.seed(None);
        let mut decider = SyntheticDecider::new(0.9);
        if oversized {
            workspace.config.segment.state_token_budget = 1;
        } else {
            decider.fail_merge = true;
        }
        let llm = MockLlm::new(vec![]);
        let (first, events) = workspace.run(
            "r_deferred_merge",
            &workspace.options(),
            &llm,
            &decider,
            &AtomicBool::new(false),
        );
        assert_eq!(first.status, RunStatus::Partial);
        assert!(workspace.snapshot().rejected_merges.is_empty());
        assert_eq!(workspace.active_topics(), 2);
        if oversized {
            assert!(events.iter().any(|event| matches!(event, EventBody::Warning(payload) if payload.code == "W_MERGE_DEFERRED")));
            assert_eq!(decider.calls.get(), 0);
        }
        workspace.config.segment.state_token_budget = 24_000;
        decider.fail_merge = false;
        let (second, _) = workspace.run(
            "r_retry_deferred_merge",
            &workspace.options(),
            &llm,
            &decider,
            &AtomicBool::new(false),
        );
        assert_eq!(second.status, RunStatus::Complete);
        assert_eq!(workspace.active_topics(), 1);
    }
}

#[test]
fn evidence_recalled_during_confirmation_defers_the_merge() {
    let workspace = Workspace::new(&[0, 1, 0]);
    workspace.seed(None);
    let callback = || {
        let mut recalled = workspace.batch.clone();
        for message in &mut recalled.messages {
            message.recalled = true;
            message.text.clear();
        }
        store::import_batches(&workspace.database, &[recalled], &ImportOptions::default()).unwrap();
    };
    let mut decider = SyntheticDecider::new(0.9);
    decider.on_merge = Some(&callback);
    let llm = MockLlm::new(vec![]);
    let (result, events) = workspace.run(
        "r_recall_merge_support",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(result.status, RunStatus::Partial);
    assert_eq!(workspace.active_topics(), 2);
    assert!(workspace.snapshot().rejected_merges.is_empty());
    assert!(events.iter().any(
        |event| matches!(event, EventBody::Warning(payload) if payload.code == "W_MERGE_DEFERRED")
    ));
}

#[test]
fn topic_event_failure_keeps_committed_merge_and_persists_its_statistics() {
    let workspace = Workspace::new(&[0, 1, 0]);
    workspace.seed(None);
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::new(0.9);
    let result = agent::analyze(
        &workspace.database,
        &workspace.batch.chat.chat_id,
        &"r_broken_topic_sink".into(),
        &workspace.config,
        &workspace.options(),
        Models {
            llm: &llm,
            primary: Some(&decider),
            fallback: &decider,
        },
        &AtomicBool::new(false),
        &mut |event| {
            if matches!(event, EventBody::Topic(_)) {
                Err(EngineError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "synthetic sink closed",
                )))
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(EngineError::Io(_))));
    assert_eq!(workspace.active_topics(), 1);
    let history = store::stats(
        &workspace.database,
        None,
        Some(&"r_broken_topic_sink".into()),
    )
    .unwrap();
    let StatsPayload::Run(stats) = &history.rows[0] else {
        panic!("run stats expected")
    };
    assert_eq!(stats.topics_updated, 2);
    assert_eq!(stats.messages_analyzed, 0);
    let (repeat, _) = workspace.run(
        "r_after_broken_topic_sink",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(repeat.status, RunStatus::Complete);
    assert_eq!(decider.merge_calls.get(), 1);
}

#[test]
fn finish_event_failure_keeps_committed_merge_and_persists_its_statistics() {
    let workspace = Workspace::new(&[0, 1, 0]);
    workspace.seed(None);
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::new(0.9);
    let delivered_topics = Cell::new(0);
    let result = agent::analyze(
        &workspace.database,
        &workspace.batch.chat.chat_id,
        &"r_broken_finish_sink".into(),
        &workspace.config,
        &workspace.options(),
        Models {
            llm: &llm,
            primary: Some(&decider),
            fallback: &decider,
        },
        &AtomicBool::new(false),
        &mut |event| {
            if matches!(event, EventBody::Topic(_)) {
                delivered_topics.set(delivered_topics.get() + 1);
            }
            if matches!(event, EventBody::Decision(payload) if matches!(payload.chosen, AgentAction::Finish { .. }))
            {
                Err(EngineError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "synthetic finish sink closed",
                )))
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(EngineError::Io(_))));
    assert_eq!(delivered_topics.get(), 2);
    assert_eq!(workspace.active_topics(), 1);
    let history = store::stats(
        &workspace.database,
        None,
        Some(&"r_broken_finish_sink".into()),
    )
    .unwrap();
    let StatsPayload::Run(stats) = &history.rows[0] else {
        panic!("run stats expected")
    };
    assert_eq!(stats.topics_updated, 2);
    assert_eq!(stats.messages_analyzed, 0);
    assert!(
        !history
            .warnings
            .iter()
            .any(|warning| warning.code == "W_HISTORY_INCOMPLETE")
    );
    let (repeat, _) = workspace.run(
        "r_after_broken_finish_sink",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(repeat.status, RunStatus::Complete);
    assert_eq!(decider.merge_calls.get(), 1);
}

#[test]
fn followup_verify_counts_active_topics_without_the_merged_source() {
    let mut workspace = Workspace::new(&[0, 1, 0]);
    workspace.seed(None);
    let decider = SyntheticDecider::new(0.9);
    workspace.run(
        "r_merge_before_followup",
        &workspace.options(),
        &MockLlm::new(vec![]),
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(workspace.active_topics(), 1);
    assert_eq!(workspace.snapshot().topics.len(), 2);
    let followup = workspace.import_followup(Some(&workspace.batch.messages[0]));
    workspace.config.agent.direct_max = 0;
    let llm = MockLlm::new(vec![Ok(LlmResponse {
        content: json!({"title":"Synthetic continued topic","summary":"","items":[]}).to_string(),
        usage: Usage::default(),
    })]);
    let (result, events) = workspace.run(
        "r_verify_followup",
        &workspace.options(),
        &llm,
        &decider,
        &AtomicBool::new(false),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 1);
    let snapshot = workspace.snapshot();
    let target = snapshot
        .topics
        .iter()
        .find(|topic| topic.state == TopicState::Active)
        .unwrap();
    let message = snapshot
        .messages
        .iter()
        .find(|message| message.message.id == followup)
        .unwrap();
    assert_eq!(message.topic_id.as_ref(), Some(&target.id));
    assert_eq!(message.analysis_state, "done");
    let verify: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            EventBody::Decision(payload)
                if matches!(payload.chosen, AgentAction::Verify { .. }) =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect();
    assert_eq!(verify.len(), 1);
    assert_eq!(verify[0].observation.active_topics, 1);
    assert_eq!(decider.merge_calls.get(), 1);
}

#[test]
fn ordinary_phase_limits_keep_settled_merge_candidates_in_finish_observation() {
    for money_limited in [false, true] {
        let mut workspace = Workspace::new(&[0, 1, 0]);
        workspace.seed(None);
        workspace.import_followup(None);
        workspace.config.agent.direct_max = 0;
        let mut options = workspace.options();
        if money_limited {
            options.budget_usd = 0.000000001;
        } else {
            options.max_steps = 1;
        }
        let initial = workspace.plan(&options);
        assert_eq!(initial["messages"], 1);
        assert_eq!(initial["merge_candidates"], 1);
        let llm = MockLlm::new(vec![]);
        let decider = SyntheticDecider::new(0.9);
        let (result, events) = workspace.run(
            "r_ordinary_limit",
            &options,
            &llm,
            &decider,
            &AtomicBool::new(false),
        );
        assert_eq!(result.status, RunStatus::Partial);
        assert_eq!(
            result.reason,
            if money_limited {
                FinishReason::BudgetExceeded
            } else {
                FinishReason::MaxSteps
            }
        );
        assert_eq!(decider.merge_calls.get(), 0);
        assert!(llm.requests().is_empty());
        let finish = events
            .iter()
            .find_map(|event| match event {
                EventBody::Decision(payload)
                    if matches!(payload.chosen, AgentAction::Finish { .. }) =>
                {
                    Some(payload)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(finish.observation.merge_candidates, 1);
        assert_eq!(workspace.plan(&workspace.options())["merge_candidates"], 1);
    }
}
