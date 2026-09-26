use super::*;
use crate::{
    decider::{DecisionResponse, MockDecider},
    llm::{MockLlm, ProviderError, Usage},
    store::{self, AnalysisSession, ImportOptions},
};
use chat_tldr_qce::{QceOptions, parse_qce_json};

fn response() -> DecisionResponse {
    DecisionResponse {
        model: "synthetic-jev".into(),
        usage: Usage {
            input_tokens: 10,
            output_tokens: 10,
            cost_usd: 0.00001,
        },
        answers: BTreeMap::from([(
            "next_action".into(),
            Answer::Choice {
                choice: "a1".into(),
                probabilities: BTreeMap::from([("a0".into(), 0.1), ("a1".into(), 0.9)]),
                confidence: 0.8,
            },
        )]),
    }
}

#[test]
fn jev_choice_is_logged_cached_bounded_and_rule_fallback_does_not_switch_provider() {
    for success in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.db");
        let source = include_bytes!("../../../../../../fixtures/qce/synthetic-group.json");
        let batch = parse_qce_json(source, &QceOptions::default()).unwrap();
        let chat = batch.chat.chat_id.clone();
        store::import_batches(&path, &[batch], &ImportOptions::default()).unwrap();
        let session =
            AnalysisSession::begin(&path, &chat, &"r_controller".into(), &json!({})).unwrap();
        let config = Config::parse("").unwrap();
        let options = AnalyzeOptions::from_config(&config);
        let primary = MockDecider::new(
            "synthetic-jev",
            vec![if success {
                Ok(response())
            } else {
                Err(ProviderError::new("E_PROVIDER_TIMEOUT", true, "synthetic"))
            }],
        );
        let fallback = MockDecider::new("unused", vec![]);
        let llm = MockLlm::new(vec![]);
        let cancel = AtomicBool::new(false);
        let mut stats = RunStats {
            run_id: session.run_id.clone(),
            chat_id: chat,
            messages_analyzed: 0,
            topics_created: 0,
            topics_updated: 0,
            insights: InsightStats::default(),
            usage: vec![],
            cost_usd: 0.0,
            elapsed_ms: 0,
        };
        let mut runtime = Runtime {
            session: &session,
            config: &config,
            models: Models {
                llm: &llm,
                primary: Some(&primary),
                fallback: &fallback,
            },
            options: &options,
            stats: &mut stats,
            budget: Default::default(),
            steps: 0,
            fallback_active: false,
            fallback_warned: false,
            fallback_cause: None,
            cancel: &cancel,
        };
        let allowed = vec![
            AgentAction::AnalyzeTopic {
                topic_id: "first".into(),
            },
            AgentAction::AnalyzeTopic {
                topic_id: "second".into(),
            },
        ];
        let descriptions = vec!["ordinary topic".into(), "urgent topic".into()];
        let observation = AgentObservation {
            pending_messages: 0,
            interleave: 0.0,
            active_topics: 2,
            dirty_topics: 2,
            pending_verification: 0,
            merge_candidates: 0,
            steps_taken: 0,
            cost_usd: 0.0,
        };
        let mut events = vec![];
        let chosen = runtime
            .select_action(
                allowed.clone(),
                descriptions.clone(),
                observation.clone(),
                &mut |event| {
                    events.push(event);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(chosen, allowed[usize::from(success)]);
        assert_eq!(runtime.steps, 1);
        assert!(!runtime.fallback_active);
        assert!(fallback.requests().is_empty());
        let EventBody::Decision(decision) = &events[0] else {
            panic!("missing decision")
        };
        assert_eq!(decision.allowed, allowed);
        assert_eq!(
            decision.method,
            if success {
                DecisionMethod::Jev
            } else {
                DecisionMethod::Fallback
            }
        );
        let request = &primary.requests()[0];
        assert!(
            request
                .state
                .as_object()
                .unwrap()
                .values()
                .all(|value| value.is_string())
        );
        if success {
            runtime
                .select_action(allowed, descriptions, observation, &mut |_| Ok(()))
                .unwrap();
            assert_eq!(primary.requests().len(), 1);
            assert_eq!(runtime.stats.usage.last().unwrap().cache_hits, 1);
        }
    }
}
