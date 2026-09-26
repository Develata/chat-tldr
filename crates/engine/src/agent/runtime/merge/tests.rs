use super::*;
use crate::{
    decider::{Decider, DecisionResponse, LlmDecider, MockDecider},
    llm::{MockLlm, ProviderError},
    store::{self, ImportOptions},
};
use chat_tldr_qce::{QceOptions, parse_qce_json};
use std::{cell::Cell, path::PathBuf};

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    chat: ChatId,
    config: Config,
    topics: [TopicRecord; 2],
    evidence: Vec<StoredMessage>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("merge-confirmation.db");
        let batch = parse_qce_json(
            include_bytes!("../../../../../../fixtures/qce/synthetic-group.json"),
            &QceOptions::default(),
        )
        .unwrap();
        let chat = batch.chat.chat_id.clone();
        store::import_batches(&path, &[batch], &ImportOptions::default()).unwrap();
        let mut evidence: Vec<_> = store::analysis_snapshot(&path, &chat)
            .unwrap()
            .messages
            .into_iter()
            .filter(|message| !message.message.recalled)
            .take(2)
            .collect();
        assert_eq!(evidence.len(), 2);
        let topics = ["first", "second"].map(|id| TopicRecord {
            id: id.into(),
            chat_id: chat.clone(),
            title: id.into(),
            provisional: false,
            state: TopicState::Active,
            last_message_at: evidence[0].message.sent_at,
            is_chitchat: None,
        });
        for (message, topic) in evidence.iter_mut().zip(&topics) {
            message.topic_id = Some(topic.id.clone());
        }
        evidence[1].message.reply_to = Some(ReplyRef {
            source_message_id: "synthetic-001".into(),
            resolved: Some(evidence[0].message.id.clone()),
        });
        Self {
            _directory: directory,
            path,
            chat,
            config: Config::parse("").unwrap(),
            topics,
            evidence,
        }
    }

    fn run(
        &self,
        models: Models<'_>,
        budget: f64,
        cancel: &AtomicBool,
        test: impl FnOnce(&mut Runtime<'_, '_>),
    ) {
        let session = AnalysisSession::begin(
            &self.path,
            &self.chat,
            &"merge-confirmation".into(),
            &json!({}),
        )
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
        let mut options = AnalyzeOptions::from_config(&self.config);
        options.budget_usd = budget;
        let fallback_active = models.primary.is_none();
        test(&mut Runtime {
            session: &session,
            config: &self.config,
            models,
            options: &options,
            stats: &mut stats,
            reserved: 0.0,
            steps: 0,
            fallback_active,
            fallback_warned: false,
            cancel,
        });
    }

    fn confirm(&self, runtime: &mut Runtime<'_, '_>) -> Result<bool> {
        runtime.confirm_merge(&self.topics[0], &self.topics[1], &self.evidence)
    }
}

fn response(p_yes: f32) -> DecisionResponse {
    DecisionResponse {
        model: "synthetic-merge".into(),
        answers: BTreeMap::from([(QUESTION_ID.into(), Answer::Noul { p_yes })]),
        usage: Usage {
            input_tokens: 10,
            output_tokens: 2,
            cost_usd: 0.0001,
        },
    }
}

#[test]
fn merge_confirmation_uses_inclusive_half_probability_threshold() {
    for probability in [0.0, 0.49999, 0.5, 1.0] {
        let fixture = Fixture::new();
        let primary = MockDecider::new("jev", vec![Ok(response(probability))]);
        let fallback = MockDecider::new("llm", vec![]);
        let llm = MockLlm::new(vec![]);
        fixture.run(
            Models {
                llm: &llm,
                primary: Some(&primary),
                fallback: &fallback,
            },
            10.0,
            &AtomicBool::new(false),
            |runtime| assert_eq!(fixture.confirm(runtime).unwrap(), probability >= 0.5),
        );
        assert_eq!(primary.requests().len(), 1);
        assert!(fallback.requests().is_empty());
    }
}

#[test]
fn canonical_pair_and_evidence_order_share_cache_and_keep_auditable_reply_context() {
    let fixture = Fixture::new();
    let primary = MockDecider::new("jev", vec![Ok(response(0.6))]);
    let fallback = MockDecider::new("llm", vec![]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        10.0,
        &AtomicBool::new(false),
        |runtime| {
            assert!(fixture.confirm(runtime).unwrap());
            let reversed: Vec<_> = fixture.evidence.iter().rev().cloned().collect();
            assert!(
                runtime
                    .confirm_merge(&fixture.topics[1], &fixture.topics[0], &reversed)
                    .unwrap()
            );
            let history = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            assert_eq!(history.rows.len(), 2);
            assert_eq!(history.rows[0], history.rows[1]);
            assert_eq!(history.rows[0].subject.kind, SubjectKind::TopicPair);
            assert_eq!(
                history.rows[0].subject.candidates,
                vec!["first".into(), "second".into()]
            );
            assert_eq!(
                history.rows[0].subject.message_ids,
                fixture
                    .evidence
                    .iter()
                    .map(|m| m.message.id.clone())
                    .collect::<Vec<_>>()
            );
            assert_eq!(runtime.stats.usage[1].cache_hits, 1);
            assert_eq!(runtime.stats.usage[1].cost_usd, 0.0);
        },
    );
    assert_eq!(primary.requests().len(), 1);
    let request = &primary.requests()[0];
    assert_eq!(request.state["supporting_messages"][0]["ref"], "n1");
    assert_eq!(
        request.state["supporting_messages"][1]["reply_to_ref"],
        "n1"
    );
    assert_eq!(request.state["supporting_messages"][1]["topic_ref"], "T1");
    assert_eq!(request.state["topics"][0]["title"], "first");
}

#[test]
fn jev_failure_falls_back_to_the_configured_llm_with_the_same_subject() {
    let fixture = Fixture::new();
    let primary = MockDecider::new(
        "jev",
        vec![Err(ProviderError::new(
            "E_PROVIDER_UNAVAILABLE",
            true,
            "synthetic unavailable",
        ))],
    );
    let llm = MockLlm::new(vec![Ok(LlmResponse {
        content: json!({"answers":{"merge_topics":{"type":"noul","p_yes":0.7}}}).to_string(),
        usage: Usage::default(),
    })]);
    let fallback = LlmDecider::new(&llm, &fixture.config.llm.model);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        10.0,
        &AtomicBool::new(false),
        |runtime| {
            assert!(fixture.confirm(runtime).unwrap());
            assert!(runtime.fallback_active);
            let history = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            assert_eq!(history.rows.len(), 1);
            assert_eq!(history.rows[0].model, fallback.name());
            assert_eq!(history.rows[0].subject.kind, SubjectKind::TopicPair);
            assert!(
                runtime
                    .stats
                    .usage
                    .iter()
                    .any(|row| row.provider == "llm" && row.calls == 1)
            );
        },
    );
    let llm_request: serde_json::Value = serde_json::from_str(&llm.requests()[0].user).unwrap();
    assert_eq!(llm_request["state"], primary.requests()[0].state);
}

struct CancellingDecider<'a> {
    cancel: &'a AtomicBool,
    calls: Cell<u32>,
}

impl Decider for CancellingDecider<'_> {
    fn name(&self) -> &str {
        "cancelling-merge"
    }
    fn decide(&self, _: &DecisionRequest) -> std::result::Result<DecisionResponse, ProviderError> {
        self.calls.set(self.calls.get() + 1);
        self.cancel.store(true, Ordering::Relaxed);
        Ok(response(0.9))
    }
}

#[test]
fn cancelled_answer_is_retained_and_resumed_confirmation_reuses_it() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let primary = CancellingDecider {
        cancel: &cancel,
        calls: Cell::new(0),
    };
    let fallback = MockDecider::new("llm", vec![]);
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        10.0,
        &cancel,
        |runtime| {
            assert!(matches!(
                fixture.confirm(runtime),
                Err(EngineError::Cancelled)
            ));
            let before = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            assert_eq!(before.rows.len(), 1);
            assert_eq!(before.rows[0].subject.kind, SubjectKind::TopicPair);
            cancel.store(false, Ordering::Relaxed);
            assert!(fixture.confirm(runtime).unwrap());
            assert_eq!(runtime.stats.usage[1].cache_hits, 1);
            assert_eq!(primary.calls.get(), 1);
        },
    );
    assert!(fallback.requests().is_empty());
}

#[test]
fn request_money_budget_and_early_cancellation_do_not_call_models() {
    for cancelled in [false, true] {
        let fixture = Fixture::new();
        let primary = MockDecider::new("jev", vec![]);
        let fallback = MockDecider::new("llm", vec![]);
        let llm = MockLlm::new(vec![]);
        fixture.run(
            Models {
                llm: &llm,
                primary: Some(&primary),
                fallback: &fallback,
            },
            0.000000001,
            &AtomicBool::new(cancelled),
            |runtime| {
                let error = fixture.confirm(runtime).unwrap_err();
                assert!(matches!(
                    (cancelled, error),
                    (true, EngineError::Cancelled) | (false, EngineError::BudgetExceeded)
                ));
                assert!(runtime.stats.usage.is_empty());
            },
        );
        assert!(primary.requests().is_empty());
        assert!(fallback.requests().is_empty());
    }
}

#[test]
fn wire_state_budget_boundary_returns_deferred_error_instead_of_rejection() {
    for oversized in [false, true] {
        let mut fixture = Fixture::new();
        let (request, _) = merge_request(
            &fixture.chat,
            &fixture.topics[0],
            &fixture.topics[1],
            &fixture.evidence,
        )
        .unwrap();
        let wire = request.to_wire("");
        let size = serde_json::to_string(&wire["state"])
            .unwrap()
            .chars()
            .count()
            + serde_json::to_string(&wire["questions"][QUESTION_ID])
                .unwrap()
                .chars()
                .count();
        fixture.config.segment.state_token_budget = (size - usize::from(oversized)) as u32;
        let primary = MockDecider::new("jev", vec![Ok(response(0.8))]);
        let fallback = MockDecider::new("llm", vec![]);
        let llm = MockLlm::new(vec![]);
        fixture.run(
            Models {
                llm: &llm,
                primary: Some(&primary),
                fallback: &fallback,
            },
            10.0,
            &AtomicBool::new(false),
            |runtime| {
                if oversized {
                    let error = fixture.confirm(runtime).unwrap_err();
                    assert_eq!(error.code(), "E_MERGE_STATE_TOO_LARGE");
                    assert!(runtime.stats.usage.is_empty());
                    assert!(
                        store::jev_log(&fixture.path, &runtime.session.run_id)
                            .unwrap()
                            .rows
                            .is_empty()
                    );
                } else {
                    assert!(fixture.confirm(runtime).unwrap());
                }
            },
        );
        assert_eq!(primary.requests().len(), usize::from(!oversized));
    }
}

#[test]
fn provider_state_limit_applies_even_when_config_budget_is_larger() {
    let mut fixture = Fixture::new();
    fixture.evidence[0].message.text = "中".repeat(STATE_QUESTION_LIMIT);
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
        10.0,
        &AtomicBool::new(false),
        |runtime| {
            assert_eq!(
                fixture.confirm(runtime).unwrap_err().code(),
                "E_MERGE_STATE_TOO_LARGE"
            )
        },
    );
    assert!(primary.requests().is_empty());
}

#[test]
fn invalid_probabilities_and_unknown_question_ids_cannot_confirm_or_enter_cache() {
    let fixture = Fixture::new();
    let mut wrong_question = response(0.8);
    wrong_question.answers = BTreeMap::from([("other".into(), Answer::Noul { p_yes: 0.8 })]);
    let fallback = MockDecider::new(
        "llm",
        vec![
            Ok(response(f32::NAN)),
            Ok(response(1.1)),
            Ok(wrong_question),
            Ok(response(0.9)),
        ],
    );
    let llm = MockLlm::new(vec![]);
    fixture.run(
        Models {
            llm: &llm,
            primary: None,
            fallback: &fallback,
        },
        10.0,
        &AtomicBool::new(false),
        |runtime| {
            for _ in 0..3 {
                assert_eq!(
                    fixture.confirm(runtime).unwrap_err().code(),
                    "E_LLM_OUTPUT_INVALID"
                );
            }
            assert!(fixture.confirm(runtime).unwrap());
            let history = store::jev_log(&fixture.path, &runtime.session.run_id).unwrap();
            assert_eq!(history.rows.len(), 1);
        },
    );
    assert_eq!(fallback.requests().len(), 4);
}

#[test]
fn invalid_scope_or_stale_evidence_is_rejected_before_any_request() {
    let fixture = Fixture::new();
    for case in 0..7 {
        let mut topics = fixture.topics.clone();
        let mut evidence = fixture.evidence.clone();
        match case {
            0 => topics[1].id = topics[0].id.clone(),
            1 => topics[1].chat_id = "other".into(),
            2 => topics[1].state = TopicState::Merged,
            3 => evidence[0].message.recalled = true,
            4 => evidence[0].topic_id = None,
            5 => evidence.push(evidence[0].clone()),
            6 => evidence[1].topic_id = Some(topics[0].id.clone()),
            _ => unreachable!(),
        }
        assert!(matches!(
            merge_request(&fixture.chat, &topics[0], &topics[1], &evidence),
            Err(EngineError::Input(_))
        ));
    }
}
