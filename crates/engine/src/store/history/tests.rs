use super::*;
use crate::store::{AnalysisSession, ImportOptions, import_batches};
use chat_tldr_qce::{QceOptions, parse_qce_json};
use tempfile::TempDir;

struct Fixture {
    _directory: TempDir,
    path: std::path::PathBuf,
    chat: ChatId,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.db");
        let batch = parse_qce_json(
            include_bytes!("../../../../../fixtures/qce/synthetic-group.json"),
            &QceOptions::default(),
        )
        .unwrap();
        let chat = batch.chat.chat_id.clone();
        import_batches(&path, &[batch], &ImportOptions::default()).unwrap();
        Self {
            _directory: directory,
            path,
            chat,
        }
    }
    fn session(&self, run: &str) -> AnalysisSession {
        AnalysisSession::begin(&self.path, &self.chat, &run.into(), &json!({})).unwrap()
    }
    fn empty_stats(&self, run: &str) -> RunStats {
        RunStats {
            run_id: run.into(),
            chat_id: self.chat.clone(),
            messages_analyzed: 0,
            topics_created: 0,
            topics_updated: 0,
            insights: InsightStats::default(),
            usage: Vec::new(),
            cost_usd: 0.0,
            elapsed_ms: 123,
        }
    }
}

fn decision(step: u32) -> DecisionPayload {
    DecisionPayload {
        step,
        observation: AgentObservation {
            pending_messages: 0,
            interleave: 0.0,
            active_topics: 0,
            dirty_topics: 0,
            pending_verification: 0,
            merge_candidates: 0,
            steps_taken: step,
            cost_usd: 0.0,
        },
        allowed: vec![AgentAction::Finish {
            reason: FinishReason::Done,
        }],
        chosen: AgentAction::Finish {
            reason: FinishReason::Done,
        },
        method: DecisionMethod::Jev,
        probabilities: Some(BTreeMap::from([("finish".into(), 0.8)])),
        confidence: Some(0.8),
        reason: "synthetic choice".into(),
    }
}

#[test]
fn missing_store_reports_empty_global_or_not_found_without_creating_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("absent").join("db");
    let result = stats(&path, None, None).unwrap();
    let StatsPayload::Global(global) = &result.rows[0] else {
        panic!()
    };
    assert!(global.counts.values().all(|count| *count == 0));
    assert!(global.usage.is_empty());
    assert!(matches!(
        stats(&path, Some(&"missing".into()), None),
        Err(EngineError::ChatNotFound(_))
    ));
    assert!(matches!(
        stats(&path, None, Some(&"missing".into())),
        Err(EngineError::RunNotFound(_))
    ));
    assert!(matches!(
        decisions(&path, &"missing".into()),
        Err(EngineError::RunNotFound(_))
    ));
    assert!(matches!(
        jev_log(&path, &"missing".into()),
        Err(EngineError::RunNotFound(_))
    ));
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn replay_preserves_order_probabilities_confidence_and_frozen_subjects() {
    let fixture = Fixture::new();
    let mut session = fixture.session("r_replay");
    session.record_decision(&decision(3)).unwrap();
    session.record_decision(&decision(1)).unwrap();
    let payload = JevAnswerPayload {
        model: "synthetic-jev".into(),
        request_key: "request-a".into(),
        question_id: "topic".into(),
        qtype: "choice".into(),
        subject: AnswerSubject {
            kind: SubjectKind::Burst,
            id: "b_test".into(),
            message_ids: vec!["m_exact".into()],
            candidates: vec!["t_exact".into()],
        },
        answer: json!({"type":"choice","probabilities":{"T0":0.8,"new_topic":0.2},"choice":"T0","confidence":0.8}),
        confidence: Some(0.8),
    };
    session.record_answer(&payload, "typesafe", false).unwrap();
    session.record_answer(&payload, "typesafe", true).unwrap();
    session
        .finish_with_stats(RunStatus::Complete, &fixture.empty_stats("r_replay"))
        .unwrap();
    let before = std::fs::read(&fixture.path).unwrap();
    let decisions = decisions(&fixture.path, &session.run_id).unwrap();
    assert!(decisions.warnings.is_empty());
    assert_eq!(decisions.rows, vec![decision(1), decision(3)]);
    let answers = jev_log(&fixture.path, &session.run_id).unwrap();
    assert!(answers.warnings.is_empty());
    assert_eq!(answers.rows, vec![payload.clone(), payload]);
    assert_eq!(answers.detail["unaligned_answers"], 0);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
}

#[test]
fn legacy_rows_remain_readable_without_inventing_subject_or_confidence() {
    let fixture = Fixture::new();
    let mut session = fixture.session("r_legacy");
    session.record_decision(&decision(0)).unwrap();
    let connection = super::super::analysis::open_write(&fixture.path).unwrap();
    connection
        .execute(
            "UPDATE decisions SET observation_json=?1",
            [serde_json::to_string(&decision(0).observation).unwrap()],
        )
        .unwrap();
    connection.execute("INSERT INTO jev_answers(run_id,request_key,question_id,qtype,answer_json,confidence,subject_json) VALUES('r_legacy','old','n1_todo','noul','{\"type\":\"noul\",\"p_yes\":0.7}',NULL,'{\"new_messages\":[\"old text\"]}')",[]).unwrap();
    session.finish(RunStatus::Failed).unwrap();
    let replay = decisions(&fixture.path, &session.run_id).unwrap();
    assert_eq!(replay.rows[0].confidence, None);
    assert_eq!(replay.warnings[0].code, "W_HISTORY_INCOMPLETE");
    let answers = jev_log(&fixture.path, &session.run_id).unwrap();
    assert_eq!(answers.rows[0].subject.kind, SubjectKind::Unknown);
    assert_eq!(answers.rows[0].subject.id, "unavailable");
    assert!(answers.rows[0].subject.message_ids.is_empty());
    assert_eq!(answers.rows[0].model, "unknown");
    assert_eq!(answers.detail["unaligned_answers"], 1);
    let result = stats(&fixture.path, None, Some(&session.run_id)).unwrap();
    assert_eq!(result.detail["run_status"], "failed");
    assert_eq!(result.warnings[0].code, "W_HISTORY_INCOMPLETE");
}

#[test]
fn failed_and_empty_runs_keep_exact_counters_and_usage_excludes_cache_cost() {
    let fixture = Fixture::new();
    let mut failed = fixture.session("r_failed");
    let usage = UsageStats {
        stage: "decide".into(),
        provider: "typesafe".into(),
        model: "synthetic-jev".into(),
        calls: 1,
        cache_hits: 0,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: 0.0,
    };
    failed.record_usage(&usage).unwrap();
    let fallback = UsageStats {
        provider: "llm".into(),
        model: "synthetic-llm".into(),
        input_tokens: 100,
        output_tokens: 20,
        cost_usd: 0.003,
        ..usage.clone()
    };
    failed.record_usage(&fallback).unwrap();
    failed
        .record_usage(&UsageStats {
            calls: 0,
            cache_hits: 1,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            ..fallback
        })
        .unwrap();
    let saved = fixture.empty_stats("r_failed");
    failed.finish_with_stats(RunStatus::Failed, &saved).unwrap();
    drop(failed);
    let mut empty = fixture.session("r_empty");
    empty
        .finish_with_stats(RunStatus::Complete, &fixture.empty_stats("r_empty"))
        .unwrap();
    let result = stats(&fixture.path, None, Some(&"r_failed".into())).unwrap();
    assert!(result.warnings.is_empty());
    let StatsPayload::Run(run) = &result.rows[0] else {
        panic!()
    };
    assert_eq!(run.elapsed_ms, 123);
    assert_eq!(run.usage.iter().map(|u| u.calls).sum::<u64>(), 2);
    assert_eq!(run.usage.iter().map(|u| u.cache_hits).sum::<u64>(), 1);
    assert_eq!(run.usage.iter().map(|u| u.input_tokens).sum::<u64>(), 100);
    assert!((run.cost_usd - 0.003).abs() < 1e-9);
    let StatsPayload::Run(empty) = &stats(&fixture.path, None, Some(&"r_empty".into()))
        .unwrap()
        .rows[0]
    else {
        panic!()
    };
    assert!(empty.usage.is_empty());
    assert_eq!(empty.messages_analyzed, 0);
    let scoped = stats(&fixture.path, Some(&fixture.chat), None).unwrap();
    let StatsPayload::Global(global) = &scoped.rows[0] else {
        panic!()
    };
    assert_eq!(global.counts["runs"], 2);
    assert_eq!(global.counts["messages"], 3);
    assert_eq!(global.cost_usd, run.cost_usd);
}

#[test]
fn fallback_answers_keep_subjects_when_cancelled_work_resumes_from_cache() {
    use crate::{
        Config,
        agent::{self, AnalyzeOptions, Models},
        decider::{Answer, Decider, DecisionRequest, DecisionResponse, MockDecider, Question},
        llm::{LlmResponse, MockLlm, ProviderError, Usage},
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct CancelAfterAnswer<'a>(&'a AtomicBool);
    impl Decider for CancelAfterAnswer<'_> {
        fn name(&self) -> &str {
            "synthetic-fallback"
        }
        fn decide(
            &self,
            request: &DecisionRequest,
        ) -> std::result::Result<DecisionResponse, ProviderError> {
            let answers = request
                .questions
                .iter()
                .map(|(id, question)| {
                    let answer = match question {
                        Question::Noul { .. } => Answer::Noul { p_yes: 0.9 },
                        Question::Score { levels, .. } => Answer::Score {
                            score: (levels.len() - 1) as f32,
                            confidence: 1.0,
                            probabilities: (0..levels.len())
                                .map(|index| {
                                    (
                                        index.to_string(),
                                        if index == levels.len() - 1 { 1.0 } else { 0.0 },
                                    )
                                })
                                .collect(),
                        },
                        Question::Choice { .. } => panic!("classification only"),
                    };
                    (id.clone(), answer)
                })
                .collect();
            self.0.store(true, Ordering::Relaxed);
            Ok(DecisionResponse {
                model: self.name().into(),
                answers,
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 10,
                    cost_usd: 0.001,
                },
            })
        }
    }
    let fixture = Fixture::new();
    let config = Config::parse("").unwrap();
    let options = AnalyzeOptions::from_config(&config);
    let cancel = AtomicBool::new(false);
    let llm = MockLlm::new(vec![Ok(LlmResponse {
        content:
            json!({"topics":[{"refs":["n1","n2"],"title":"合成话题","summary":"","items":[]}]})
                .to_string(),
        usage: Usage::default(),
    })]);
    let primary = MockDecider::new(
        "synthetic-jev",
        vec![Err(ProviderError::new(
            "E_PROVIDER_UNAVAILABLE",
            true,
            "synthetic unavailable",
        ))],
    );
    let fallback = CancelAfterAnswer(&cancel);
    let mut events = Vec::new();
    let first = agent::analyze(
        &fixture.path,
        &fixture.chat,
        &"r_before_cache".into(),
        &config,
        &options,
        Models {
            llm: &llm,
            primary: Some(&primary),
            fallback: &fallback,
        },
        &cancel,
        &mut |event| {
            events.push(event);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(first.status, RunStatus::Cancelled, "{events:?}");
    let before = jev_log(&fixture.path, &"r_before_cache".into()).unwrap();
    assert!(!before.rows.is_empty());
    assert!(
        before
            .rows
            .iter()
            .all(|answer| answer.model == "synthetic-fallback"
                && answer.subject.kind != SubjectKind::Unknown)
    );
    cancel.store(false, Ordering::Relaxed);
    let second_fallback = MockDecider::new("synthetic-fallback", vec![]);
    let second_llm = MockLlm::new(vec![]);
    let second = agent::analyze(
        &fixture.path,
        &fixture.chat,
        &"r_after_cache".into(),
        &config,
        &options,
        Models {
            llm: &second_llm,
            primary: None,
            fallback: &second_fallback,
        },
        &cancel,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(second.status, RunStatus::Complete);
    assert!(second_fallback.requests().is_empty());
    let after = jev_log(&fixture.path, &"r_after_cache".into()).unwrap();
    assert_eq!(before.rows, after.rows);
    let historical = stats(&fixture.path, None, Some(&"r_after_cache".into())).unwrap();
    let StatsPayload::Run(stats) = &historical.rows[0] else {
        panic!()
    };
    assert!(stats.usage.iter().any(|usage| usage.stage == "decide"
        && usage.cache_hits == 1
        && usage.calls == 0
        && usage.cost_usd == 0.0));
}
