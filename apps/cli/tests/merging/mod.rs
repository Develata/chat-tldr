//! Keyless merge planning/no-work boundaries through the real CLI process.
use chat_tldr_core::{ChatId, ReplyRef, RunStatus, TopicState};
use chat_tldr_engine::store::{self, AnalysisSession, ImportOptions, TopicRecord};
use chrono::Duration;
use serde_json::json;

use super::*;

fn settled_pair(sandbox: &Sandbox, rejected: bool) -> ChatId {
    let database = sandbox.data_dir().join("chat-tldr.db");
    let mut batch = chat_tldr_qce::parse_qce_json(
        &fs::read(fixture()).unwrap(),
        &chat_tldr_qce::QceOptions::default(),
    )
    .unwrap();
    let template = batch.messages[0].clone();
    batch.messages = (0..3)
        .map(|index| {
            let mut message = template.clone();
            message.id = format!("m_cli_merge_{index}").into();
            message.source.identity = format!("qce:cli-merge-{index}");
            message.source.qce_id = Some(format!("cli-merge-{index}"));
            message.sent_at += Duration::seconds(index);
            message.text = format!("合成合并测试第 {index} 条消息。");
            message.mentions.clear();
            message.attachments.clear();
            message.forward = None;
            message.system = false;
            message.recalled = false;
            message.reply_to = (index > 0).then(|| ReplyRef {
                source_message_id: format!("cli-merge-{}", index - 1),
                resolved: Some(format!("m_cli_merge_{}", index - 1).into()),
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
    let mut session = AnalysisSession::begin(
        &database,
        &batch.chat.chat_id,
        &"r_cli_merge_seed".into(),
        &json!({}),
    )
    .unwrap();
    // A -> B -> A forms exactly two distinct cross-topic replies.
    for owner in 0..2 {
        let members: Vec<_> = batch
            .messages
            .iter()
            .enumerate()
            .filter(|(index, _)| index % 2 == owner)
            .map(|(_, message)| message)
            .collect();
        let topic = TopicRecord {
            id: format!("t_cli_merge_{owner}").into(),
            chat_id: batch.chat.chat_id.clone(),
            title: format!("合成话题 {owner}"),
            provisional: false,
            state: TopicState::Active,
            last_message_at: members.last().unwrap().sent_at,
            is_chitchat: Some(0.0),
        };
        let ids = members
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();
        session.assign(&topic, &ids, "synthetic").unwrap();
        session.commit_topic(&topic, &ids, vec![]).unwrap();
    }
    if rejected {
        session
            .reject_merge(&"t_cli_merge_0".into(), &"t_cli_merge_1".into())
            .unwrap();
    }
    session.finish(RunStatus::Complete).unwrap();
    batch.chat.chat_id
}

#[test]
fn merge_only_dry_run_without_keys_reports_work_and_preserves_database() {
    let sandbox = Sandbox::new();
    let chat = settled_pair(&sandbox, false);
    let database = sandbox.data_dir().join("chat-tldr.db");
    let before = fs::read(&database).unwrap();
    let result = sandbox.run(&["analyze", "--chat", chat.as_ref(), "--dry-run"], 0);
    let plan = &payload(&result, "ack")["detail"]["plan"];
    assert_eq!(plan["messages"], 0);
    assert_eq!(plan["merge_candidates"], 1);
    assert_eq!(plan["writes"], false);
    assert_eq!(plan["model_calls"], 0);
    assert_eq!(plan["readiness"]["llm_key_present"], false);
    assert_eq!(plan["readiness"]["jev_key_present"], false);
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(
        payload(&sandbox.run(&["stats"], 0), "stats")["counts"]["runs"],
        1
    );
}

#[test]
fn merge_only_live_run_without_keys_fails_before_writes_instead_of_noop_success() {
    let sandbox = Sandbox::new();
    let chat = settled_pair(&sandbox, false);
    let database = sandbox.data_dir().join("chat-tldr.db");
    let before = fs::read(&database).unwrap();
    let result = sandbox.run(&["analyze", "--chat", chat.as_ref()], 4);
    assert_eq!(payload(&result, "error")["code"], "E_CONFIG");
    assert_eq!(payload(&result, "done")["status"], "failed");
    assert!(!result.iter().any(|event| event["event"] == "stats"));
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(
        payload(&sandbox.run(&["stats"], 0), "stats")["counts"]["runs"],
        1
    );
}

#[test]
fn rejected_merge_only_run_needs_no_keys_and_creates_no_run_or_database_writes() {
    let sandbox = Sandbox::new();
    let chat = settled_pair(&sandbox, true);
    let database = sandbox.data_dir().join("chat-tldr.db");
    let before = fs::read(&database).unwrap();
    let dry_run = sandbox.run(&["analyze", "--chat", chat.as_ref(), "--dry-run"], 0);
    let plan = &payload(&dry_run, "ack")["detail"]["plan"];
    assert_eq!(plan["messages"], 0);
    assert_eq!(plan["merge_candidates"], 0);
    let result = sandbox.run(&["analyze", "--chat", chat.as_ref()], 0);
    let stats = payload(&result, "stats");
    assert_eq!(stats["messages_analyzed"], 0);
    assert_eq!(stats["topics_updated"], 0);
    assert_eq!(stats["cost_usd"], 0.0);
    assert!(stats["usage"].as_array().unwrap().is_empty());
    assert_eq!(payload(&result, "done")["status"], "complete");
    assert_eq!(payload(&result, "done")["finish_reason"], "done");
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(
        payload(&sandbox.run(&["stats"], 0), "stats")["counts"]["runs"],
        1
    );
}
