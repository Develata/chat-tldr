//! Message-time expiry and historical attribution through public engine APIs.
use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, sync::atomic::AtomicBool};

use chat_tldr_core::*;
use chat_tldr_engine::{
    Config, EngineError,
    agent::{self, AnalysisResult, AnalyzeOptions, Models},
    decider::{Answer, Decider, DecisionRequest, DecisionResponse, Question, validate_response},
    llm::{LlmClient, LlmRequest, LlmResponse, MockLlm, ProviderError, Usage},
    store::{self, AnalysisSession, AnalysisSnapshot, ImportOptions, TopicRecord},
};
use chrono::{DateTime, Duration, FixedOffset};
use serde_json::{Value, json};

struct Workspace {
    _directory: tempfile::TempDir,
    path: PathBuf,
    template: ImportBatch,
    messages: Vec<UnifiedMessage>,
    config: Config,
}

impl Workspace {
    fn new(offsets_ms: &[i64]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lifecycle.db");
        let template = chat_tldr_qce::parse_qce_json(
            include_bytes!("../../../fixtures/qce/synthetic-group.json"),
            &chat_tldr_qce::QceOptions::default(),
        )
        .unwrap();
        let mut config = Config::parse("").unwrap();
        config.segment.topic_close_secs = 10;
        config.agent.direct_max = 100;
        config.agent.direct_interleave_max = 1.0;
        let mut workspace = Self {
            _directory: directory,
            path,
            template,
            messages: Vec::new(),
            config,
        };
        for offset in offsets_ms {
            workspace.import(*offset, None);
        }
        workspace
    }

    fn time(&self, offset_ms: i64) -> DateTime<FixedOffset> {
        self.template.messages[0].sent_at + Duration::milliseconds(offset_ms)
    }

    fn import(&mut self, offset_ms: i64, reply_index: Option<usize>) -> MessageId {
        let index = self.messages.len();
        let mut message = self.template.messages[0].clone();
        message.id = format!("m_lifecycle_{index}").into();
        message.source.identity = format!("qce:lifecycle-{index}");
        message.source.qce_id = Some(format!("lifecycle-{index}"));
        message.sent_at = self.time(offset_ms);
        message.text = format!("Synthetic lifecycle message {index}.");
        message.mentions.clear();
        message.attachments.clear();
        message.forward = None;
        message.reply_to = reply_index.map(|target| ReplyRef {
            source_message_id: self.messages[target].source.qce_id.clone().unwrap(),
            resolved: Some(self.messages[target].id.clone()),
        });
        message.recalled = false;
        message.system = false;
        let mut batch = self.template.clone();
        batch.messages = vec![message.clone()];
        store::import_batches(&self.path, &[batch], &ImportOptions::default()).unwrap();
        let id = message.id.clone();
        self.messages.push(message);
        id
    }

    fn seed(&self, id: &str, members: &[usize], closed: bool, item: bool) {
        let mut session = AnalysisSession::begin(
            &self.path,
            &self.template.chat.chat_id,
            &format!("r_seed_{id}").into(),
            &json!({}),
        )
        .unwrap();
        let topic = TopicRecord {
            id: id.into(),
            chat_id: self.template.chat.chat_id.clone(),
            title: id.into(),
            provisional: false,
            state: TopicState::Active,
            last_message_at: members
                .iter()
                .map(|index| self.messages[*index].sent_at)
                .max()
                .unwrap(),
            is_chitchat: Some(0.0),
        };
        let ids: Vec<_> = members
            .iter()
            .map(|index| self.messages[*index].id.clone())
            .collect();
        session.assign(&topic, &ids, "synthetic").unwrap();
        let items = if item {
            let message = &self.messages[members[0]];
            vec![(
                Insight {
                    id: "i_lifecycle".into(),
                    chat_id: topic.chat_id.clone(),
                    kind: InsightKind::Todo,
                    title: "Synthetic existing task".into(),
                    summary: String::new(),
                    priority: Priority::P1,
                    rank_score: 0.5,
                    confidence: Some(0.9),
                    assignee: Assignee::Other,
                    deadline: None,
                    evidence: vec![Evidence {
                        message_id: message.id.clone(),
                        quote: chat_tldr_engine::render::render(message),
                        render_profile: RenderProfile::default(),
                    }],
                    topic_id: Some(topic.id.clone()),
                    verification_status: VerificationStatus::Verified,
                    lifecycle: Lifecycle::Open,
                    created_in_run: session.run_id.clone(),
                    updated_at: message.sent_at,
                },
                0.5,
            )]
        } else {
            Vec::new()
        };
        session.commit_topic(&topic, &ids, items).unwrap();
        if closed {
            let rows = session
                .close_topics(
                    &[topic.id],
                    topic.last_message_at + Duration::seconds(11),
                    10,
                )
                .unwrap();
            assert_eq!(rows.len(), 1);
        }
        session.finish(RunStatus::Complete).unwrap();
        if item {
            store::resolve(&self.path, &"i_lifecycle".into(), Lifecycle::Done).unwrap();
            store::feedback(&self.path, &"i_lifecycle".into(), true).unwrap();
        }
    }

    fn snapshot(&self) -> AnalysisSnapshot {
        store::analysis_snapshot(&self.path, &self.template.chat.chat_id).unwrap()
    }

    fn topic(&self, id: &str) -> TopicRecord {
        self.snapshot()
            .topics
            .into_iter()
            .find(|topic| topic.id.as_ref() == id)
            .unwrap()
    }

    fn options(&self) -> AnalyzeOptions {
        let mut options = AnalyzeOptions::from_config(&self.config);
        options.budget_usd = 10.0;
        options
    }

    fn run(
        &self,
        run: &str,
        options: &AnalyzeOptions,
        llm: &dyn LlmClient,
        decider: &dyn Decider,
    ) -> (AnalysisResult, Vec<EventBody>) {
        let mut events = Vec::new();
        let result = agent::analyze(
            &self.path,
            &self.template.chat.chat_id,
            &run.into(),
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
}

#[derive(Default)]
struct EmptyExtraction {
    split_direct: bool,
    requests: RefCell<Vec<LlmRequest>>,
}

impl LlmClient for EmptyExtraction {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        self.requests.borrow_mut().push(request.clone());
        let input: Value = serde_json::from_str(&request.user).unwrap();
        let direct = input["instruction"]
            .as_str()
            .unwrap()
            .starts_with("Group every");
        let body = if direct {
            let refs: Vec<_> = input["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|row| row["ref"].as_str())
                .filter(|reference| reference.starts_with('n'))
                .collect();
            let groups = if self.split_direct {
                refs.into_iter().map(|reference| vec![reference]).collect()
            } else {
                vec![refs]
            };
            json!({"topics":groups.into_iter().enumerate().map(|(index, refs)| json!({"refs":refs,"title":format!("Direct topic {index}"),"summary":"","items":[]})).collect::<Vec<_>>()})
        } else {
            json!({"title":"Updated synthetic topic","summary":"","items":[]})
        };
        Ok(LlmResponse {
            content: body.to_string(),
            usage: Usage::default(),
        })
    }
}

struct SyntheticDecider {
    existing: bool,
    requests: RefCell<Vec<DecisionRequest>>,
}

impl SyntheticDecider {
    fn new(existing: bool) -> Self {
        Self {
            existing,
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl Decider for SyntheticDecider {
    fn name(&self) -> &str {
        "synthetic-lifecycle"
    }
    fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        self.requests.borrow_mut().push(request.clone());
        let answers = request
            .questions
            .iter()
            .map(|(key, question)| {
                let answer = match question {
                    Question::Noul { .. } => Answer::Noul { p_yes: 0.0 },
                    Question::Score { levels, .. } => Answer::Score {
                        score: 0.0,
                        confidence: 1.0,
                        probabilities: (0..levels.len())
                            .map(|index| (index.to_string(), if index == 0 { 1.0 } else { 0.0 }))
                            .collect(),
                    },
                    Question::Choice { options, .. } => {
                        let choice = if self.existing {
                            options
                                .keys()
                                .find(|key| key.as_str() != "new_topic")
                                .unwrap()
                        } else {
                            "new_topic"
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
fn later_message_closes_expired_topic_without_changing_its_members_or_user_state() {
    let mut workspace = Workspace::new(&[0]);
    workspace.seed("old", &[0], false, true);
    let before = workspace.snapshot();
    workspace.import(10_001, None);
    let (result, events) = workspace.run(
        "r_expire",
        &workspace.options(),
        &EmptyExtraction::default(),
        &SyntheticDecider::new(false),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(workspace.topic("old").state, TopicState::Closed);
    assert_eq!(workspace.topic("old").last_message_at, workspace.time(0));
    assert_eq!(result.stats.messages_analyzed, 1);
    assert_eq!(result.stats.topics_updated, 2);
    let after = workspace.snapshot();
    assert_eq!(
        after.messages[0].analysis_state,
        before.messages[0].analysis_state
    );
    assert_eq!(after.messages[0].cursor, before.messages[0].cursor);
    assert_eq!(after.insights[0].id, before.insights[0].id);
    assert_eq!(after.insights[0].lifecycle, Lifecycle::Done);
    assert_eq!(after.insights[0].evidence, before.insights[0].evidence);
    assert_eq!(after.insights[0].priority, before.insights[0].priority);
    assert!(events.iter().any(|event| matches!(event, EventBody::Topic(payload) if payload.topic_id.as_ref() == "old" && payload.state == TopicState::Closed)));
}

#[test]
fn exact_timeout_boundary_stays_active_despite_old_wall_clock_dates() {
    let mut workspace = Workspace::new(&[0]);
    workspace.seed("old", &[0], false, false);
    workspace.import(10_000, None);
    let (result, events) = workspace.run(
        "r_exact_ttl",
        &workspace.options(),
        &EmptyExtraction::default(),
        &SyntheticDecider::new(false),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(workspace.topic("old").state, TopicState::Active);
    assert_eq!(workspace.topic("old").last_message_at, workspace.time(0));
    assert!(!events.iter().any(|event| matches!(event, EventBody::Topic(payload) if payload.topic_id.as_ref() == "old" && payload.state == TopicState::Closed)));
}

#[test]
fn historical_reply_uses_closed_topic_predecessor_without_reopening_or_time_regression() {
    let mut workspace = Workspace::new(&[0, 100_000]);
    workspace.seed("historical", &[0, 1], true, false);
    let backfill = workspace.import(1_000, Some(0));
    let (result, events) = workspace.run(
        "r_backfill_closed",
        &workspace.options(),
        &EmptyExtraction::default(),
        &SyntheticDecider::new(true),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.topics_created, 0);
    let topic = workspace.topic("historical");
    assert_eq!(topic.state, TopicState::Closed);
    assert_eq!(topic.last_message_at, workspace.time(100_000));
    let snapshot = workspace.snapshot();
    let message = snapshot
        .messages
        .iter()
        .find(|message| message.message.id == backfill)
        .unwrap();
    assert_eq!(message.topic_id.as_ref(), Some(&topic.id));
    assert_eq!(message.analysis_state, "done");
    assert!(events.iter().any(|event| matches!(event, EventBody::Decision(payload) if matches!(payload.chosen, AgentAction::Segment { .. }))));
    assert!(!events.iter().any(|event| matches!(event, EventBody::Decision(payload) if matches!(payload.chosen, AgentAction::AnalyzeDirect { .. }))));
    assert!(
        events
            .iter()
            .filter_map(|event| match event {
                EventBody::Topic(payload) if payload.topic_id == topic.id => Some(payload),
                _ => None,
            })
            .all(|payload| payload.state == TopicState::Closed)
    );
}

#[test]
fn future_only_members_and_historical_activity_gaps_cannot_claim_backfilled_messages() {
    for future_only in [false, true] {
        let offsets = if future_only {
            vec![50_005]
        } else {
            vec![0, 50_005]
        };
        let mut workspace = Workspace::new(&offsets);
        workspace.seed(
            "unrelated",
            &(0..offsets.len()).collect::<Vec<_>>(),
            true,
            false,
        );
        let backfill = workspace.import(50_000, None);
        let decider = SyntheticDecider::new(true);
        let (result, _) = workspace.run(
            "r_gap_backfill",
            &workspace.options(),
            &EmptyExtraction::default(),
            &decider,
        );
        assert_eq!(result.status, RunStatus::Complete);
        assert_eq!(result.stats.topics_created, 1);
        let snapshot = workspace.snapshot();
        let assigned = snapshot
            .messages
            .iter()
            .find(|message| message.message.id == backfill)
            .unwrap()
            .topic_id
            .as_ref()
            .unwrap();
        assert_ne!(assigned.as_ref(), "unrelated");
        assert_eq!(workspace.topic("unrelated").state, TopicState::Closed);
        assert!(
            !decider
                .requests
                .borrow()
                .iter()
                .any(|request| request.questions.contains_key("topic"))
        );
    }
}

#[test]
fn direct_batch_can_close_an_earlier_topic_before_its_saved_draft_is_verified() {
    let workspace = Workspace::new(&[0, 20_000]);
    let llm = EmptyExtraction {
        split_direct: true,
        ..Default::default()
    };
    let (result, events) = workspace.run(
        "r_spanning_direct",
        &workspace.options(),
        &llm,
        &SyntheticDecider::new(false),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 2);
    assert_eq!(llm.requests.borrow().len(), 1);
    let snapshot = workspace.snapshot();
    assert!(
        snapshot
            .messages
            .iter()
            .all(|message| message.analysis_state == "done")
    );
    let first_topic = snapshot.messages[0].topic_id.as_ref().unwrap();
    assert_eq!(
        snapshot
            .topics
            .iter()
            .find(|topic| &topic.id == first_topic)
            .unwrap()
            .state,
        TopicState::Closed
    );
    let closed = events.iter().position(|event| matches!(event, EventBody::Topic(payload) if &payload.topic_id == first_topic && payload.state == TopicState::Closed)).unwrap();
    let verify = events.iter().position(|event| matches!(event, EventBody::Decision(payload) if matches!(payload.chosen, AgentAction::Verify { .. }))).unwrap();
    assert!(closed < verify);
}

#[test]
fn step_limited_direct_drafts_resume_from_cache_even_after_topic_closure() {
    let workspace = Workspace::new(&[0, 20_000]);
    let llm = EmptyExtraction {
        split_direct: true,
        ..Default::default()
    };
    let decider = SyntheticDecider::new(false);
    let mut options = workspace.options();
    options.max_steps = 1;
    let (first, _) = workspace.run("r_close_then_limit", &options, &llm, &decider);
    assert_eq!(first.status, RunStatus::Partial);
    assert_eq!(first.reason, FinishReason::MaxSteps);
    assert_eq!(first.stats.messages_analyzed, 0);
    assert_eq!(
        workspace
            .snapshot()
            .topics
            .iter()
            .filter(|topic| topic.state == TopicState::Closed)
            .count(),
        1
    );
    let no_llm = MockLlm::new(vec![]);
    let (second, _) = workspace.run(
        "r_resume_closed_draft",
        &workspace.options(),
        &no_llm,
        &decider,
    );
    assert_eq!(second.status, RunStatus::Complete);
    assert_eq!(second.stats.messages_analyzed, 2);
    assert!(no_llm.requests().is_empty());
    assert!(
        second
            .stats
            .usage
            .iter()
            .any(|usage| usage.stage == "extract" && usage.cache_hits > 0)
    );
    assert!(
        workspace
            .snapshot()
            .messages
            .iter()
            .all(|message| message.analysis_state == "done")
    );
    assert_eq!(
        workspace
            .snapshot()
            .topics
            .iter()
            .filter(|topic| topic.state == TopicState::Closed)
            .count(),
        1
    );
}

#[test]
fn imported_future_message_outside_analysis_range_does_not_close_the_current_topic() {
    let mut workspace = Workspace::new(&[0]);
    workspace.seed("current", &[0], false, false);
    workspace.import(1_000, Some(0));
    let future = workspace.import(100_000, None);
    let mut options = workspace.options();
    options.until = Some(workspace.time(2_000));
    let (result, _) = workspace.run(
        "r_scoped_current",
        &options,
        &EmptyExtraction::default(),
        &SyntheticDecider::new(true),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(workspace.topic("current").state, TopicState::Active);
    let snapshot = workspace.snapshot();
    let future = snapshot
        .messages
        .iter()
        .find(|message| message.message.id == future)
        .unwrap();
    assert_eq!(future.analysis_state, "pending");
    assert!(future.topic_id.is_none());
    let chat = store::list_chats(&workspace.path).unwrap().pop().unwrap();
    assert!(chat.last_analyzed.unwrap() < future.cursor);
}

#[test]
fn closed_topic_and_finish_sink_failures_preserve_committed_closure_and_statistics() {
    for stop_at_finish in [false, true] {
        let mut workspace = Workspace::new(&[0]);
        workspace.seed("old", &[0], false, false);
        workspace.import(10_001, None);
        let llm = EmptyExtraction::default();
        let decider = SyntheticDecider::new(false);
        let result = agent::analyze(
            &workspace.path,
            &workspace.template.chat.chat_id,
            &"r_closure_sink_failure".into(),
            &workspace.config,
            &workspace.options(),
            Models {
                llm: &llm,
                primary: Some(&decider),
                fallback: &decider,
            },
            &AtomicBool::new(false),
            &mut |event| {
                let stop = if stop_at_finish {
                    matches!(event, EventBody::Decision(payload) if matches!(payload.chosen, AgentAction::Finish { .. }))
                } else {
                    matches!(event, EventBody::Topic(payload) if payload.topic_id.as_ref() == "old" && payload.state == TopicState::Closed)
                };
                if stop {
                    Err(EngineError::Io(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "Synthetic closure sink failed",
                    )))
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(result, Err(EngineError::Io(_))));
        assert_eq!(workspace.topic("old").state, TopicState::Closed);
        let history = store::stats(
            &workspace.path,
            None,
            Some(&"r_closure_sink_failure".into()),
        )
        .unwrap();
        let StatsPayload::Run(stats) = &history.rows[0] else {
            panic!("run statistics required")
        };
        assert_eq!(stats.topics_updated, if stop_at_finish { 2 } else { 1 });
        assert!(
            !history
                .warnings
                .iter()
                .any(|warning| warning.code == "W_HISTORY_INCOMPLETE")
        );
        let (resumed, _) = workspace.run(
            "r_after_closure_sink_failure",
            &workspace.options(),
            &EmptyExtraction::default(),
            &decider,
        );
        assert_eq!(resumed.status, RunStatus::Complete);
        assert_eq!(workspace.topic("old").state, TopicState::Closed);
    }
}

#[test]
fn later_segment_or_outer_store_progress_failure_keeps_prior_closure_stats_partial() {
    for fail_store_progress in [false, true] {
        let mut workspace = Workspace::new(&[0]);
        workspace.config.segment.topic_close_secs = 6 * 60 * 60;
        workspace.config.agent.segment_batch = 1;
        workspace.config.agent.direct_max = 0;
        workspace.seed("old", &[0], false, false);
        workspace.import(7 * 60 * 60 * 1000, None);
        workspace.import(8 * 60 * 60 * 1000, None);
        let llm = EmptyExtraction::default();
        let decider = SyntheticDecider::new(false);
        let mut segments = 0;
        let mut store_progress = 0;
        let mut observed_close = false;
        let result = agent::analyze(
            &workspace.path,
            &workspace.template.chat.chat_id,
            &"r_failure_after_closure".into(),
            &workspace.config,
            &workspace.options(),
            Models {
                llm: &llm,
                primary: Some(&decider),
                fallback: &decider,
            },
            &AtomicBool::new(false),
            &mut |event| {
                match event {
                    EventBody::Topic(payload)
                        if payload.topic_id.as_ref() == "old"
                            && payload.state == TopicState::Closed =>
                    {
                        observed_close = true
                    }
                    EventBody::Decision(payload)
                        if matches!(payload.chosen, AgentAction::Segment { .. }) =>
                    {
                        segments += 1
                    }
                    EventBody::Progress(payload) if payload.stage == "store" => store_progress += 1,
                    _ => {}
                }
                if (!fail_store_progress && segments == 2)
                    || (fail_store_progress && store_progress == 2)
                {
                    assert!(
                        observed_close,
                        "failure must happen after the committed closure was emitted"
                    );
                    Err(EngineError::Io(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "Synthetic later sink failure",
                    )))
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(result, Err(EngineError::Io(_))));
        assert_eq!(workspace.topic("old").state, TopicState::Closed);
        assert_eq!(segments, 2);
        assert_eq!(store_progress, if fail_store_progress { 2 } else { 0 });
        let history = store::stats(
            &workspace.path,
            None,
            Some(&"r_failure_after_closure".into()),
        )
        .unwrap();
        assert_eq!(history.detail["run_status"], "partial");
        assert!(
            !history
                .warnings
                .iter()
                .any(|warning| warning.code == "W_HISTORY_INCOMPLETE")
        );
        let StatsPayload::Run(stats) = &history.rows[0] else {
            panic!("run statistics required")
        };
        assert_eq!(
            stats.topics_updated,
            if fail_store_progress { 2 } else { 1 }
        );
        assert_eq!(stats.messages_analyzed, u64::from(fail_store_progress));
        let (resumed, _) = workspace.run(
            "r_resume_later_sink_failure",
            &workspace.options(),
            &EmptyExtraction::default(),
            &decider,
        );
        assert_eq!(resumed.status, RunStatus::Complete);
        assert_eq!(workspace.topic("old").state, TopicState::Closed);
        assert!(
            workspace
                .snapshot()
                .messages
                .iter()
                .all(|message| message.analysis_state == "done")
        );
    }
}

fn closed_at_sixteen_hours() -> Workspace {
    let mut workspace = Workspace::new(&[9 * 60 * 60 * 1000]);
    workspace.config.segment.topic_close_secs = 6 * 60 * 60;
    workspace.seed("historical_cutoff", &[0], false, false);
    let mut session = AnalysisSession::begin(
        &workspace.path,
        &workspace.template.chat.chat_id,
        &"r_close_at_sixteen".into(),
        &json!({}),
    )
    .unwrap();
    let closed = session
        .close_topics(
            &["historical_cutoff".into()],
            workspace.time(16 * 60 * 60 * 1000),
            6 * 60 * 60,
        )
        .unwrap();
    assert_eq!(closed.len(), 1);
    session.finish(RunStatus::Complete).unwrap();
    workspace
}

#[test]
fn closed_topic_first_closure_time_still_blocks_live_messages_after_successful_backfill() {
    for later_hour in [16, 17] {
        let mut workspace = closed_at_sixteen_hours();
        let backfill = workspace.import(15 * 60 * 60 * 1000, Some(0));
        let llm = EmptyExtraction::default();
        let decider = SyntheticDecider::new(true);
        let (first, _) = workspace.run(
            "r_preclosure_backfill",
            &workspace.options(),
            &llm,
            &decider,
        );
        assert_eq!(first.status, RunStatus::Complete);
        assert_eq!(first.stats.topics_created, 0);
        assert_eq!(
            workspace.topic("historical_cutoff").state,
            TopicState::Closed
        );
        assert_eq!(
            workspace.topic("historical_cutoff").last_message_at,
            workspace.time(15 * 60 * 60 * 1000)
        );
        assert_eq!(
            workspace
                .snapshot()
                .messages
                .iter()
                .find(|message| message.message.id == backfill)
                .unwrap()
                .topic_id
                .as_ref()
                .unwrap()
                .as_ref(),
            "historical_cutoff"
        );
        let later = workspace.import(later_hour * 60 * 60 * 1000, Some(1));
        let (second, _) =
            workspace.run("r_postclosure_reply", &workspace.options(), &llm, &decider);
        assert_eq!(second.status, RunStatus::Complete);
        assert_eq!(second.stats.topics_created, 1);
        let snapshot = workspace.snapshot();
        let assigned = snapshot
            .messages
            .iter()
            .find(|message| message.message.id == later)
            .unwrap()
            .topic_id
            .as_ref()
            .unwrap();
        assert_ne!(assigned.as_ref(), "historical_cutoff");
        assert_eq!(
            workspace.topic("historical_cutoff").state,
            TopicState::Closed
        );
        assert_eq!(
            workspace.topic("historical_cutoff").last_message_at,
            workspace.time(15 * 60 * 60 * 1000)
        );
    }
}

#[test]
fn burst_spanning_first_closure_boundary_cannot_be_assigned_to_the_closed_topic() {
    let mut workspace = closed_at_sixteen_hours();
    workspace.config.agent.direct_max = 0;
    workspace.config.segment.weak_gap_secs = 3 * 60 * 60;
    workspace.config.segment.strong_gap_secs = 4 * 60 * 60;
    let before = workspace.import(15 * 60 * 60 * 1000, Some(0));
    let after = workspace.import(17 * 60 * 60 * 1000, Some(1));
    let (result, _) = workspace.run(
        "r_burst_across_cutoff",
        &workspace.options(),
        &EmptyExtraction::default(),
        &SyntheticDecider::new(true),
    );
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.topics_created, 1);
    let snapshot = workspace.snapshot();
    let assigned: Vec<_> = [&before, &after]
        .into_iter()
        .map(|id| {
            snapshot
                .messages
                .iter()
                .find(|message| &message.message.id == id)
                .unwrap()
                .topic_id
                .as_ref()
                .unwrap()
        })
        .collect();
    assert_eq!(assigned[0], assigned[1]);
    assert_ne!(assigned[0].as_ref(), "historical_cutoff");
    assert_eq!(
        workspace.topic("historical_cutoff").state,
        TopicState::Closed
    );
    assert_eq!(
        workspace.topic("historical_cutoff").last_message_at,
        workspace.time(9 * 60 * 60 * 1000)
    );
}

fn assert_assignment_survives_next_segment_failure(existing_topic: bool) {
    let mut workspace = if existing_topic {
        let mut workspace = Workspace::new(&[0]);
        workspace.seed("existing", &[0], false, false);
        workspace.import(1_000, Some(0));
        workspace.import(2_000, None);
        workspace
    } else {
        Workspace::new(&[0, 1_000])
    };
    workspace.config.agent.direct_max = 0;
    workspace.config.agent.segment_batch = 1;
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::new(true);
    let mut segments = 0;
    let mut assigned_topics = Vec::new();
    let result = agent::analyze(
        &workspace.path,
        &workspace.template.chat.chat_id,
        &"r_assignment_then_sink_failure".into(),
        &workspace.config,
        &workspace.options(),
        Models {
            llm: &llm,
            primary: Some(&decider),
            fallback: &decider,
        },
        &AtomicBool::new(false),
        &mut |event| {
            match event {
                EventBody::Topic(payload) => {
                    assert_eq!(payload.state, TopicState::Active);
                    assigned_topics.push(payload.topic_id);
                }
                EventBody::Decision(payload)
                    if matches!(payload.chosen, AgentAction::Segment { .. }) =>
                {
                    segments += 1;
                    if segments == 2 {
                        return Err(EngineError::Io(std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            "Synthetic failure after assignment checkpoint",
                        )));
                    }
                }
                _ => {}
            }
            Ok(())
        },
    );
    assert!(matches!(result, Err(EngineError::Io(_))));
    assert_eq!(segments, 2);
    assert_eq!(
        assigned_topics.len(),
        1,
        "successful assignment must emit its committed topic"
    );
    let snapshot = workspace.snapshot();
    let first = &snapshot.messages[usize::from(existing_topic)];
    let second = &snapshot.messages[usize::from(existing_topic) + 1];
    assert_eq!(first.topic_id.as_ref(), assigned_topics.first());
    assert_eq!(first.analysis_state, "pending");
    assert!(second.topic_id.is_none());
    assert_eq!(second.analysis_state, "pending");
    if existing_topic {
        assert_eq!(first.topic_id.as_ref().unwrap().as_ref(), "existing");
    }
    assert!(
        snapshot
            .topics
            .iter()
            .all(|topic| topic.state == TopicState::Active)
    );
    let history = store::stats(
        &workspace.path,
        None,
        Some(&"r_assignment_then_sink_failure".into()),
    )
    .unwrap();
    assert_eq!(history.detail["run_status"], "partial");
    assert!(
        !history
            .warnings
            .iter()
            .any(|warning| warning.code == "W_HISTORY_INCOMPLETE")
    );
    let StatsPayload::Run(stats) = &history.rows[0] else {
        panic!("run statistics required")
    };
    assert_eq!(stats.topics_created, u64::from(!existing_topic));
    assert_eq!(stats.messages_analyzed, 0);
    assert_eq!(stats.topics_updated, 0);
    assert!(llm.requests().is_empty());
}

#[test]
fn creating_a_topic_before_the_next_segment_sink_fails_is_a_persisted_partial_run() {
    assert_assignment_survives_next_segment_failure(false);
}

#[test]
fn assigning_an_existing_topic_is_partial_even_without_created_or_verified_counts() {
    assert_assignment_survives_next_segment_failure(true);
}
