use super::*;

#[test]
fn b0_calls_only_the_llm_and_retains_rejected_proposals_without_repair() {
    let workspace = Workspace::new();
    let mut options = workspace.options();
    options.strategy = "b0".into();
    let llm = MockLlm::new(vec![response(direct(topic("并不存在的证据。")))]);
    let decider = SyntheticDecider::default();
    let (result, events) = workspace.analyze("r_b0", &llm, &decider, &options);
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 2);
    assert_eq!(llm.requests().len(), 1);
    assert_eq!(decider.calls.get(), 0);
    assert_eq!(workspace.states(), ["done", "done", "skipped"]);
    let inbox = workspace.inbox(true);
    assert!(
        inbox
            .insights
            .iter()
            .any(|row| row.insight.kind == InsightKind::Todo
                && row.insight.verification_status == VerificationStatus::Rejected)
    );
    assert!(events.iter().any(|event| matches!(event,EventBody::Ack(ack) if ack.command=="analyze.strategy" && ack.detail["strategy"]=="b0")));
    assert!(agent::plan(&workspace.database, &workspace.chat, &options).is_err());
}

#[test]
fn b0_rejects_mixing_with_ours_and_honors_pre_request_budget() {
    let workspace = Workspace::new();
    let mut options = workspace.options();
    options.strategy = "b0".into();
    options.budget_usd = f64::EPSILON;
    let llm = MockLlm::new(vec![]);
    let decider = SyntheticDecider::default();
    let (result, _) = workspace.analyze("r_b0_no_budget", &llm, &decider, &options);
    assert_eq!(result.reason, FinishReason::BudgetExceeded);
    assert!(llm.requests().is_empty());
    assert_eq!(workspace.states(), ["pending", "pending", "skipped"]);
    workspace.analyze("r_ours", &successful_llm(), &decider, &workspace.options());
    assert!(agent::plan(&workspace.database, &workspace.chat, &options).is_err());
}

#[test]
fn b0_step_limit_keeps_uncommitted_messages_pending_and_reuses_extraction() {
    let workspace = Workspace::new();
    let mut options = workspace.options();
    options.strategy = "b0".into();
    options.max_steps = 1;
    let decider = SyntheticDecider::default();
    let (first, _) = workspace.analyze("r_b0_limited", &successful_llm(), &decider, &options);
    assert_eq!(first.reason, FinishReason::MaxSteps);
    assert_eq!(first.stats.messages_analyzed, 0);
    assert_eq!(workspace.states(), ["pending", "pending", "skipped"]);
    options.max_steps = 64;
    let unused = MockLlm::new(vec![]);
    let (second, _) = workspace.analyze("r_b0_resume", &unused, &decider, &options);
    assert_eq!(second.status, RunStatus::Complete);
    assert!(unused.requests().is_empty());
    assert_eq!(second.stats.messages_analyzed, 2);
}
