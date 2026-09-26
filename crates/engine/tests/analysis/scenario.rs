//! Handwritten model answers test pipeline rules, not a real model's understanding.
use super::*;

#[test]
fn multi_scenario_backfill_is_idempotent_and_resolves_cross_export_replies() {
    let workspace = Workspace::from_source(include_bytes!(
        "../../../../fixtures/qce/scenario-analysis.json"
    ));
    let backfill = parse_qce_json(
        include_bytes!("../../../../fixtures/qce/scenario-analysis-backfill.json"),
        &QceOptions::default(),
    )
    .unwrap();
    let first = store::import_batches(
        &workspace.database,
        std::slice::from_ref(&backfill),
        &ImportOptions::default(),
    )
    .unwrap();
    assert!(first.changed);
    assert_eq!(first.stats["inserted"], 3);
    assert_eq!(first.stats["duplicate"], 1);
    let snapshot = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    assert_eq!(snapshot.messages.len(), 103);
    assert!(
        snapshot
            .messages
            .windows(2)
            .all(|pair| pair[0].cursor < pair[1].cursor)
    );
    let message = |suffix: u64| {
        let source = (9_200_000_000_000_000_000_u64 + suffix).to_string();
        snapshot
            .messages
            .iter()
            .find(|row| row.message.source.qce_id.as_deref() == Some(source.as_str()))
            .unwrap()
    };
    for (source, target) in [(19, 18), (101, 20), (102, 101), (103, 18)] {
        assert_eq!(
            message(source)
                .message
                .reply_to
                .as_ref()
                .unwrap()
                .resolved
                .as_ref(),
            Some(&message(target).message.id)
        );
    }
    for source in [101, 102, 103] {
        assert_eq!(message(source).analysis_state, "pending");
        assert!(message(source).topic_id.is_none());
    }
    let repeated =
        store::import_batches(&workspace.database, &[backfill], &ImportOptions::default()).unwrap();
    assert!(!repeated.changed);
    assert_eq!(repeated.stats["inserted"], 0);
    assert_eq!(repeated.stats["duplicate"], 4);
    let after = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    assert_eq!(after.messages.len(), 103);
    for (before, after) in snapshot.messages.iter().zip(&after.messages) {
        assert_eq!(before.message, after.message);
        assert_eq!(before.cursor, after.cursor);
        assert_eq!(before.analysis_state, after.analysis_state);
        assert_eq!(before.topic_id, after.topic_id);
    }
}

struct ScenarioDecider(SyntheticDecider);

impl Decider for ScenarioDecider {
    fn name(&self) -> &str {
        "synthetic-scenario-decider"
    }

    fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        let mut response = self.0.decide(request)?;
        for (id, answer) in &mut response.answers {
            if id.ends_with("_needs_action") {
                *answer = Answer::Noul {
                    p_yes: if id == "n1_needs_action" { 0.95 } else { 0.01 },
                };
            }
        }
        validate_response(request, &response)?;
        Ok(response)
    }
}

#[test]
fn multi_scenario_mentions_and_other_deadlines_survive_import_analysis_and_repeat() {
    let workspace = Workspace::from_source(include_bytes!(
        "../../../../fixtures/qce/scenario-analysis.json"
    ));
    let before = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    assert_eq!(before.messages.len(), 100);
    let id = |suffix: u64| {
        let source = (9_200_000_000_000_000_000_u64 + suffix).to_string();
        before
            .messages
            .iter()
            .find(|message| message.message.source.qce_id.as_deref() == Some(source.as_str()))
            .unwrap()
            .message
            .id
            .clone()
    };
    let mut options = workspace.options();
    options.since = Some(DateTime::parse_from_rfc3339("2026-09-24T09:00:00+08:00").unwrap());
    options.until = Some(DateTime::parse_from_rfc3339("2026-09-24T09:06:00+08:00").unwrap());
    for (suffix, expected) in [
        (1, true),
        (2, true),
        (3, true),
        (4, false),
        (5, false),
        (6, false),
    ] {
        let message = before
            .messages
            .iter()
            .find(|message| message.message.id == id(suffix))
            .unwrap();
        assert_eq!(
            extract::mentions_me(&message.message, &before.chat),
            expected
        );
    }
    let llm = MockLlm::new(vec![response(json!({"topics":[{
        "refs":["n1","n2","n3","n4","n5","n6"],
        "title":"合成课程事务", "summary":"", "items":[
            {"op":"new", "existing_ref":null, "kind":"todo",
             "title":"核对演示预算", "summary":"确认后发送结果。", "assignee":"me",
             "deadline_raw":"今天18:00前", "deadline_date_guess":null,
             "evidence":[{"ref":"n1","quote":"请核对合成演示预算，今天18:00前把确认结果发给我。"}]},
            {"op":"new", "existing_ref":null, "kind":"todo",
             "title":"王砚整理报名名单", "summary":"由合成王砚负责。", "assignee":"other",
             "deadline_raw":null, "deadline_date_guess":null,
             "evidence":[{"ref":"n4","quote":"请整理报名名单，负责人是你。"}]},
            {"op":"new", "existing_ref":null, "kind":"todo",
             "title":"王砚提交实验报告", "summary":"他人负责但有截止日期。", "assignee":"other",
             "deadline_raw":"2026年9月25日17:00前", "deadline_date_guess":null,
             "evidence":[{"ref":"n6","quote":"合成王砚负责实验报告，2026年9月25日17:00前提交给我。"}]}
        ]
    }]}))]);
    let decider = ScenarioDecider(SyntheticDecider::default());
    let (result, _) = workspace.analyze("r_scenario", &llm, &decider, &options);
    assert_eq!(result.status, RunStatus::Complete);
    assert_eq!(result.stats.messages_analyzed, 6);

    let inbox = workspace.inbox(false);
    assert_eq!(inbox.insights.len(), 5);
    let report = store::overview(
        &workspace.database,
        &workspace.chat,
        &store::OverviewOptions {
            since: options.since.unwrap(),
            until: options.until.unwrap(),
            now: options.until.unwrap(),
        },
    )
    .unwrap();
    assert_eq!(report.hot_topics.len(), 1);
    assert_eq!(report.hot_topics[0].meaningful_messages, 6);
    assert_eq!(report.priority_topics[0].priority, Priority::P0);
    assert_eq!(report.priority_topics[0].insight_ids.len(), 5);
    assert_eq!(report.related.len(), 3);
    assert_eq!(report.mentions.len(), 3);
    assert_eq!(report.deadlines.len(), 2);
    assert_eq!(report.unread_topics[0].insight_ids.len(), 5);
    let database_before = std::fs::read(&workspace.database).unwrap();
    assert_eq!(
        store::overview(
            &workspace.database,
            &workspace.chat,
            &store::OverviewOptions {
                since: options.since.unwrap(),
                until: options.until.unwrap(),
                now: options.until.unwrap(),
            }
        )
        .unwrap(),
        report
    );
    assert_eq!(std::fs::read(&workspace.database).unwrap(), database_before);
    let item_for = |suffix| {
        let expected = id(suffix);
        let matching: Vec<_> = inbox
            .insights
            .iter()
            .filter(|item| {
                item.insight
                    .evidence
                    .iter()
                    .any(|e| e.message_id == expected)
            })
            .collect();
        assert_eq!(matching.len(), 1, "exactly one item for source {suffix}");
        &matching[0].insight
    };
    assert_eq!(item_for(1).kind, InsightKind::Todo);
    assert_eq!(item_for(1).assignee, Assignee::Me);
    assert_eq!(item_for(1).priority, Priority::P0);
    assert_eq!(item_for(1).deadline.as_ref().unwrap().raw, "今天18:00前");
    for suffix in [2, 3] {
        assert_eq!(item_for(suffix).kind, InsightKind::MentionMe);
        assert_eq!(item_for(suffix).priority, Priority::P1);
    }
    assert_eq!(item_for(4).kind, InsightKind::Todo);
    assert_eq!(item_for(4).assignee, Assignee::Other);
    assert_eq!(item_for(4).priority, Priority::P2);
    assert_eq!(item_for(6).assignee, Assignee::Other);
    assert_eq!(item_for(6).priority, Priority::P0);
    assert!(item_for(6).deadline.is_some());
    assert!(inbox.insights.iter().all(|item| {
        item.insight.verification_status == VerificationStatus::Verified
            && item
                .evidence_view
                .iter()
                .all(|e| e.ok && e.highlight.is_some())
            && item.insight.evidence.iter().all(|e| e.message_id != id(5))
    }));
    assert_eq!(
        inbox
            .insights
            .iter()
            .map(|item| item.insight.priority)
            .collect::<Vec<_>>(),
        [
            Priority::P0,
            Priority::P0,
            Priority::P1,
            Priority::P1,
            Priority::P2
        ]
    );
    let after = store::analysis_snapshot(&workspace.database, &workspace.chat).unwrap();
    for message in &after.messages {
        if message.message.sent_at >= options.since.unwrap()
            && message.message.sent_at < options.until.unwrap()
        {
            assert_eq!(message.analysis_state, "done");
        } else {
            let previous = before
                .messages
                .iter()
                .find(|old| old.message.id == message.message.id)
                .unwrap();
            assert_eq!(message.analysis_state, previous.analysis_state);
            assert_eq!(message.topic_id, previous.topic_id);
        }
    }

    let decision_calls = decider.0.calls.get();
    let (repeat, _) = workspace.analyze("r_scenario_repeat", &llm, &decider, &options);
    assert_eq!(repeat.status, RunStatus::Complete);
    assert_eq!(repeat.stats.messages_analyzed, 0);
    assert_eq!(llm.requests().len(), 1);
    assert_eq!(decider.0.calls.get(), decision_calls);
    assert_eq!(workspace.inbox(false).insights.len(), 5);
    store::resolve(&workspace.database, &item_for(6).id, Lifecycle::Done).unwrap();
    store::mark_read(
        &workspace.database,
        &workspace.chat,
        inbox.meta.view_cursor.unwrap(),
    )
    .unwrap();
    let reviewed = store::overview(
        &workspace.database,
        &workspace.chat,
        &store::OverviewOptions {
            since: options.since.unwrap(),
            until: options.until.unwrap(),
            now: options.until.unwrap(),
        },
    )
    .unwrap();
    assert!(reviewed.unread_topics.is_empty());
    assert_eq!(reviewed.insights.len(), 4);
    assert_eq!(reviewed.deadlines.len(), 1);
    assert!(
        reviewed
            .insights
            .iter()
            .any(|row| row.insight.id == item_for(1).id)
    );
    assert!(
        !reviewed
            .insights
            .iter()
            .any(|row| row.insight.id == item_for(6).id)
    );
}
