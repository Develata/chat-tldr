use super::*;
use crate::{
    decider::{Decider, DecisionResponse, LlmDecider, MockDecider},
    llm::{MockLlm, ProviderError},
    store::{self, ImportOptions},
};
use chat_tldr_qce::{QceOptions, parse_qce_json};
use std::path::PathBuf;

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    chat: ChatId,
    config: Config,
    messages: Vec<StoredMessage>,
    topics: Vec<TopicRecord>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("attribution.db");
        let batch = parse_qce_json(
            include_bytes!("../../../../../../fixtures/qce/synthetic-group.json"),
            &QceOptions::default(),
        )
        .unwrap();
        let chat = batch.chat.chat_id.clone();
        store::import_batches(&path, &[batch], &ImportOptions::default()).unwrap();
        let messages = store::analysis_snapshot(&path, &chat).unwrap().messages;
        let topics = ["first", "second"]
            .into_iter()
            .map(|id| TopicRecord {
                id: id.into(),
                chat_id: chat.clone(),
                title: id.into(),
                provisional: false,
                state: TopicState::Active,
                last_message_at: messages[0].message.sent_at,
                is_chitchat: None,
            })
            .collect();
        Self {
            _directory: directory,
            path,
            chat,
            config: Config::parse("").unwrap(),
            messages,
            topics,
        }
    }

    fn options(&self) -> AnalyzeOptions {
        let mut options = AnalyzeOptions::from_config(&self.config);
        options.budget_usd = 10.0;
        options
    }

    fn run(
        &self,
        models: Models<'_>,
        options: &AnalyzeOptions,
        cancel: &AtomicBool,
        test: impl FnOnce(&mut Runtime<'_, '_>),
    ) {
        let session =
            AnalysisSession::begin(&self.path, &self.chat, &"attribution".into(), &json!({}))
                .unwrap();
        let mut stats = RunStats {
            run_id: session.run_id.clone(),
            chat_id: self.chat.clone(),
            messages_analyzed: 0,
            topics_created: 0,
            topics_updated: 0,
            insights: InsightStats::default(),
            usage: Vec::new(),
            cost_usd: 0.0,
            elapsed_ms: 0,
        };
        let fallback_active = options.decider == "llm" || models.primary.is_none();
        let mut runtime = Runtime {
            session: &session,
            config: &self.config,
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
        test(&mut runtime);
    }

    fn choose(&self, runtime: &mut Runtime<'_, '_>) -> Result<Option<(TopicRecord, &'static str)>> {
        runtime.choose_topic(&self.messages, &self.topics.iter().collect::<Vec<_>>())
    }
}

fn answer(choice: &str, confidence: f32) -> DecisionResponse {
    let selected = (confidence * 2.0 + 1.0) / 3.0;
    DecisionResponse {
        model: "synthetic".into(),
        answers: BTreeMap::from([(
            "topic".into(),
            Answer::Choice {
                choice: choice.into(),
                confidence,
                probabilities: ["T0", "T1", "new_topic"]
                    .into_iter()
                    .map(|key| {
                        (
                            key.into(),
                            if key == choice {
                                selected
                            } else {
                                (1.0 - selected) / 2.0
                            },
                        )
                    })
                    .collect(),
            },
        )]),
        usage: Usage::default(),
    }
}

#[test]
fn attribution_threshold_boundaries_and_new_topic_bypass_review() {
    for (choice, confidence, expected, method, reviews) in [
        ("T0", 0.60, Some("first"), "jev", 0),
        ("T0", 1.0, Some("first"), "jev", 0),
        ("T0", 0.249, None, "", 0),
        ("T0", 0.25, Some("second"), "llm_review", 1),
        ("T0", 0.599, Some("second"), "llm_review", 1),
        ("new_topic", 0.4, None, "", 0),
        ("new_topic", 0.9, None, "", 0),
    ] {
        let fixture = Fixture::new();
        let primary = MockDecider::new("jev", vec![Ok(answer(choice, confidence))]);
        let fallback = MockDecider::new("llm", vec![Ok(answer("T1", 0.1))]);
        let llm = MockLlm::new(vec![]);
        fixture.run(
            Models {
                llm: &llm,
                primary: Some(&primary),
                fallback: &fallback,
            },
            &fixture.options(),
            &AtomicBool::new(false),
            |runtime| {
                let actual = fixture.choose(runtime).unwrap();
                assert_eq!(
                    actual.as_ref().map(|(topic, _)| topic.id.as_ref()),
                    expected
                );
                if let Some((_, actual_method)) = actual {
                    assert_eq!(actual_method, method);
                }
                assert!(!runtime.fallback_active);
            },
        );
        assert_eq!(primary.requests().len(), 1);
        assert_eq!(fallback.requests().len(), reviews);
    }
}

#[test]
fn llm_review_keeps_candidates_and_history_and_reuses_only_its_own_cache() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![Ok(answer("T0", 0.4))]);
    let llm = MockLlm::new(vec![Ok(LlmResponse {
        content: json!({"answers":{"topic":{"type":"choice","probabilities":{"T0":0.30,"T1":0.40,"new_topic":0.30}}}}).to_string(),
        usage: Usage { input_tokens: 10, output_tokens: 20, cost_usd: 0.001 },
    })]);
    let fallback = LlmDecider::new(&llm, &fixture.config.llm.model);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &AtomicBool::new(false),
        |runtime| {
            for _ in 0..2 {
                let (topic, method) = fixture.choose(runtime).unwrap().unwrap();
                assert_eq!(topic.id.as_ref(), "second");
                assert_eq!(method, "llm_review");
            }
            assert!(!runtime.fallback_active);
            let review_usage: Vec<_> = runtime
                .stats
                .usage
                .iter()
                .filter(|row| row.stage == "topic_review")
                .collect();
            assert_eq!(review_usage.len(), 2);
            assert_eq!(review_usage[0].calls, 1);
            assert_eq!(review_usage[1].cache_hits, 1);
            assert_eq!(review_usage[1].cost_usd, 0.0);
            let history = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            assert_eq!(history.rows.len(), 4);
            for row in &history.rows {
                assert_eq!(row.subject.kind, SubjectKind::Burst);
                assert_eq!(
                    row.subject.candidates,
                    vec!["first".into(), "second".into()]
                );
                assert_eq!(row.subject.message_ids.len(), fixture.messages.len());
            }
            assert_eq!(history.rows[0].subject, history.rows[1].subject);
            assert_ne!(history.rows[0].request_key, history.rows[1].request_key);
        },
    );
    assert_eq!(primary.requests().len(), 1);
    assert_eq!(llm.requests().len(), 1);
    let review: serde_json::Value = serde_json::from_str(&llm.requests()[0].user).unwrap();
    assert_eq!(review["state"], primary.requests()[0].state);
}

#[test]
fn llm_review_can_choose_new_topic_without_changing_provider_mode() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![Ok(answer("T0", 0.4))]);
    let fallback = MockDecider::new("llm", vec![Ok(answer("new_topic", 0.1))]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &AtomicBool::new(false),
        |runtime| {
            assert!(fixture.choose(runtime).unwrap().is_none());
            assert!(!runtime.fallback_active);
        },
    );
    assert_eq!(primary.requests()[0], fallback.requests()[0]);
}

#[test]
fn primary_failure_still_falls_back_and_high_confidence_llm_is_labeled_correctly() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![Err(ProviderError::invalid_output())]);
    let fallback = MockDecider::new("llm", vec![Ok(answer("T1", 0.9))]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &AtomicBool::new(false),
        |runtime| {
            let (topic, method) = fixture.choose(runtime).unwrap().unwrap();
            assert_eq!(topic.id.as_ref(), "second");
            assert_eq!(method, "llm");
            assert!(runtime.fallback_active);
        },
    );
    assert_eq!(fallback.requests().len(), 1);
}

#[test]
fn review_budget_and_cancellation_stop_before_the_second_request() {
    for cancelled in [false, true] {
        let fixture = Fixture::new();
        let primary = MockDecider::new("jev", vec![Ok(answer("T0", 0.4))]);
        let fallback = MockDecider::new("llm", vec![]);
        let llm = MockLlm::new(vec![]);
        let mut options = fixture.options();
        options.budget_usd = 0.001;
        fixture.run(
            Models {
                llm: &llm,
                primary: Some(&primary),
                fallback: &fallback,
            },
            &options,
            &AtomicBool::new(cancelled),
            |runtime| {
                let error = fixture.choose(runtime).unwrap_err();
                assert!(matches!(
                    (cancelled, error),
                    (true, EngineError::Cancelled) | (false, EngineError::BudgetExceeded)
                ));
            },
        );
        assert_eq!(primary.requests().len(), usize::from(!cancelled));
        assert!(fallback.requests().is_empty());
    }
}

struct CancellingDecider<'a> {
    cancelled: &'a AtomicBool,
}

impl Decider for CancellingDecider<'_> {
    fn name(&self) -> &str {
        "cancelling"
    }
    fn decide(&self, _: &DecisionRequest) -> std::result::Result<DecisionResponse, ProviderError> {
        self.cancelled.store(true, Ordering::Relaxed);
        Ok(answer("T0", 0.4))
    }
}

#[test]
fn cancellation_during_primary_response_prevents_review_or_assignment() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let primary = CancellingDecider { cancelled: &cancel };
    let fallback = MockDecider::new("llm", vec![]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &cancel,
        |runtime| {
            assert!(matches!(
                fixture.choose(runtime),
                Err(EngineError::Cancelled)
            ));
            assert_eq!(runtime.stats.usage[0].calls, 1);
            let history = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            assert_eq!(history.rows.len(), 1);
            assert_eq!(history.rows[0].subject.kind, SubjectKind::Burst);
            assert_eq!(
                history.rows[0].subject.message_ids.len(),
                fixture.messages.len()
            );
            let cached = runtime
                .session
                .cache_get::<DecisionResponse>(&history.rows[0].request_key)
                .unwrap();
            assert!(cached.is_some());
        },
    );
    assert!(fallback.requests().is_empty());
}

#[test]
fn invalid_review_response_is_not_cached_and_valid_retry_reuses_primary() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![Ok(answer("T0", 0.4))]);
    let fallback = MockDecider::new("llm", vec![Ok(answer("T99", 0.9)), Ok(answer("T1", 0.9))]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &AtomicBool::new(false),
        |runtime| {
            assert!(fixture.choose(runtime).is_err());
            assert_eq!(
                fixture.choose(runtime).unwrap().unwrap().0.id.as_ref(),
                "second"
            );
        },
    );
    assert_eq!(primary.requests().len(), 1);
    assert_eq!(fallback.requests().len(), 2);
}

#[test]
fn invalid_persisted_review_is_evicted_and_queried_again() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![Ok(answer("T0", 0.4))]);
    let fallback = MockDecider::new("llm", vec![Ok(answer("T1", 0.9)), Ok(answer("T0", 0.9))]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &AtomicBool::new(false),
        |runtime| {
            assert_eq!(
                fixture.choose(runtime).unwrap().unwrap().0.id.as_ref(),
                "second"
            );
            let history = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            let review_key = &history.rows[1].request_key;
            runtime.session.cache_remove(review_key).unwrap();
            runtime
                .session
                .cache_put(
                    review_key,
                    "topic_review",
                    "llm",
                    "llm",
                    &answer("T99", 0.9),
                )
                .unwrap();
            assert_eq!(
                fixture.choose(runtime).unwrap().unwrap().0.id.as_ref(),
                "first"
            );
        },
    );
    assert_eq!(primary.requests().len(), 1);
    assert_eq!(fallback.requests().len(), 2);
}

#[test]
fn candidate_budget_trimming_preserves_rank_and_exact_json_character_limit() {
    let mut fixture = Fixture::new();
    fixture.topics = (0..12)
        .map(|n| {
            let mut topic = fixture.topics[0].clone();
            topic.id = format!("topic-{n}").into();
            topic.title = format!("标题 \"\\\n {n}");
            topic
        })
        .collect();
    let refs: Vec<_> = fixture.topics.iter().collect();
    let full = bounded_request(&fixture.messages, &refs, &fixture.config.segment)
        .unwrap()
        .unwrap()
        .0;
    let full_size = wire_size(&full);
    for budget in (1..=full_size + 1).step_by(31) {
        fixture.config.segment.state_token_budget = budget as u32;
        if let Some((request, count)) =
            bounded_request(&fixture.messages, &refs, &fixture.config.segment).unwrap()
        {
            let actual = wire_size(&request);
            assert!(actual <= budget);
            assert!(count <= fixture.config.segment.candidate_k as usize || count == refs.len());
            assert_eq!(
                request.state["candidate_topics"][0]["title"],
                fixture.topics[0].title
            );
            let Question::Choice { options, .. } = &request.questions["topic"] else {
                panic!()
            };
            assert_eq!(options.len(), count + 1);
        }
    }
}

fn wire_size(request: &DecisionRequest) -> usize {
    let wire = request.to_wire("");
    serde_json::to_string(&wire["state"])
        .unwrap()
        .chars()
        .count()
        + serde_json::to_string(&wire["questions"]["topic"])
            .unwrap()
            .chars()
            .count()
}

#[test]
fn exact_wire_budget_boundary_keeps_or_trims_a_candidate() {
    let mut fixture = Fixture::new();
    fixture.config.segment.candidate_k = 1;
    let refs: Vec<_> = fixture.topics.iter().collect();
    let full = bounded_request(&fixture.messages, &refs, &fixture.config.segment)
        .unwrap()
        .unwrap()
        .0;
    let size = wire_size(&full);
    fixture.config.segment.state_token_budget = size as u32;
    assert_eq!(
        bounded_request(&fixture.messages, &refs, &fixture.config.segment)
            .unwrap()
            .unwrap()
            .1,
        2
    );
    fixture.config.segment.state_token_budget -= 1;
    let (trimmed, count) = bounded_request(&fixture.messages, &refs, &fixture.config.segment)
        .unwrap()
        .unwrap();
    assert_eq!(count, 1);
    let trimmed_size = wire_size(&trimmed);
    fixture.config.segment.state_token_budget = trimmed_size as u32;
    assert_eq!(
        bounded_request(&fixture.messages, &refs, &fixture.config.segment)
            .unwrap()
            .unwrap()
            .1,
        1
    );
    fixture.config.segment.state_token_budget -= 1;
    assert!(
        bounded_request(&fixture.messages, &refs, &fixture.config.segment)
            .unwrap()
            .is_none()
    );
}

#[test]
fn llm_mode_review_is_a_distinct_request_and_has_no_primary_side_effects() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![]);
    let fallback = MockDecider::new("llm", vec![Ok(answer("T0", 0.4)), Ok(answer("T1", 0.1))]);
    let llm = MockLlm::new(vec![]);
    let mut options = fixture.options();
    options.decider = "llm".into();
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &options,
        &AtomicBool::new(false),
        |runtime| {
            let (topic, method) = fixture.choose(runtime).unwrap().unwrap();
            assert_eq!(topic.id.as_ref(), "second");
            assert_eq!(method, "llm_review");
            assert!(runtime.fallback_active);
        },
    );
    assert!(primary.requests().is_empty());
    assert_eq!(fallback.requests().len(), 2);
}

#[test]
fn malformed_cached_review_schema_is_evicted_instead_of_becoming_database_failure() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![Ok(answer("T0", 0.4))]);
    let fallback = MockDecider::new("llm", vec![Ok(answer("T1", 0.9)), Ok(answer("T0", 0.9))]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &AtomicBool::new(false),
        |runtime| {
            fixture.choose(runtime).unwrap();
            let history = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            let key = &history.rows[1].request_key;
            runtime.session.cache_remove(key).unwrap();
            runtime
                .session
                .cache_put(key, "topic_review", "llm", "llm", &json!({"invalid":true}))
                .unwrap();
            assert_eq!(
                fixture.choose(runtime).unwrap().unwrap().0.id.as_ref(),
                "first"
            );
        },
    );
    assert_eq!(fallback.requests().len(), 2);
}

#[test]
fn oversized_burst_does_not_call_any_model_even_with_large_configured_budget() {
    let mut fixture = Fixture::new();
    fixture.messages[0].message.text = "中".repeat(JEV_STATE_QUESTION_LIMIT);
    fixture.config.segment.state_token_budget = u32::MAX;
    let primary = MockDecider::new("jev", vec![]);
    let fallback = MockDecider::new("llm", vec![]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &fixture.options(),
        &AtomicBool::new(false),
        |runtime| assert!(fixture.choose(runtime).unwrap().is_none()),
    );
    assert!(primary.requests().is_empty());
    assert!(fallback.requests().is_empty());
}
