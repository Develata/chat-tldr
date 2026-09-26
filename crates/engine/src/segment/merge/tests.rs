use chat_tldr_core::{Cursor, ReplyRef, UnifiedMessage};
use serde_json::json;

use super::*;

struct Fixture {
    messages: Vec<StoredMessage>,
    topics: Vec<TopicRecord>,
    assignments: BTreeMap<MessageId, TopicId>,
    rejected: BTreeSet<Pair>,
    in_scope: BTreeSet<MessageId>,
}

fn id(index: usize) -> MessageId {
    format!("m_{index:04}").into()
}

impl Fixture {
    fn new(rows: &[(&str, Option<usize>)]) -> Self {
        let messages: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(index, (topic, reply))| {
                let mut message: UnifiedMessage = serde_json::from_value(json!({
                    "id": id(index), "chat_id": "synthetic", "sender": "synthetic",
                    "sender_display": "synthetic", "sent_at": "2026-09-26T12:00:00+00:00",
                    "text": "synthetic", "mentions": [], "reply_to": null,
                    "attachments": [], "forward": null, "recalled": false, "system": false,
                    "source": {"format":"synthetic", "identity":index.to_string(),
                        "qce_id":null, "qce_seq":null, "qce_type":null, "file_hash":"synthetic"}
                }))
                .unwrap();
                message.reply_to = reply.map(|target| ReplyRef {
                    source_message_id: target.to_string(),
                    resolved: Some(id(target)),
                });
                StoredMessage {
                    cursor: Cursor {
                        sent_at_ms: message.sent_at.timestamp_millis(),
                        ordinal: index as i64,
                    },
                    message,
                    analysis_state: "done".into(),
                    topic_id: Some((*topic).into()),
                }
            })
            .collect();
        let topics = rows
            .iter()
            .map(|(topic, _)| *topic)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|topic| TopicRecord {
                id: topic.into(),
                chat_id: "synthetic".into(),
                title: "synthetic topic".into(),
                provisional: false,
                state: TopicState::Active,
                last_message_at: messages[0].message.sent_at,
                is_chitchat: None,
            })
            .collect();
        let assignments = messages
            .iter()
            .map(|m| (m.message.id.clone(), m.topic_id.clone().unwrap()))
            .collect();
        Self {
            in_scope: messages.iter().map(|m| m.message.id.clone()).collect(),
            messages,
            topics,
            assignments,
            rejected: BTreeSet::new(),
        }
    }

    fn graph(&self) -> MergeGraph {
        MergeGraph::new(
            &self.messages,
            &self.assignments,
            &self.topics,
            &self.rejected,
            &self.in_scope,
        )
    }
}

#[test]
fn two_distinct_one_direction_replies_are_enough_but_duplicate_rows_are_not() {
    let mut fixture = Fixture::new(&[("a", None), ("b", Some(0))]);
    fixture.messages.push(fixture.messages[1].clone());
    assert_eq!(fixture.graph().count(), 0);
    fixture.messages.pop();
    let fixture = Fixture::new(&[("a", None), ("b", Some(0)), ("b", Some(0))]);
    let candidate = fixture.graph().next().unwrap();
    assert_eq!(candidate.into.as_ref(), "a");
    assert_eq!(candidate.from.as_ref(), "b");
    assert_eq!(candidate.reply_edges, 2);
    assert_eq!(candidate.evidence, [id(1), id(0), id(2)]);
}

#[test]
fn assignments_are_authoritative_and_missing_assignments_do_not_use_stored_topics() {
    let mut fixture = Fixture::new(&[("a", None), ("b", Some(0)), ("b", Some(0))]);
    for message in &mut fixture.messages {
        message.topic_id = None;
    }
    assert_eq!(fixture.graph().count(), 1);
    fixture.assignments.remove(&id(1));
    assert_eq!(fixture.graph().count(), 0);
    fixture.assignments.insert(id(1), "a".into());
    assert_eq!(fixture.graph().count(), 0);
}

#[test]
fn only_reply_source_in_scope_can_admit_historical_support() {
    let mut fixture = Fixture::new(&[("a", None), ("b", Some(0)), ("b", Some(0))]);
    fixture.in_scope.clear();
    assert_eq!(fixture.graph().count(), 0);
    fixture.in_scope.insert(id(0));
    assert_eq!(fixture.graph().count(), 0);
    fixture.in_scope.insert(id(2));
    assert_eq!(fixture.graph().next().unwrap().reply_edges, 2);
}

#[test]
fn recalled_cross_chat_inactive_unresolved_and_nonpast_replies_do_not_count() {
    for invalid_case in 0..9 {
        let mut fixture = Fixture::new(&[("a", None), ("b", Some(0)), ("b", Some(0))]);
        match invalid_case {
            0 => fixture.messages[1].message.recalled = true,
            1 => fixture.messages[0].message.recalled = true,
            2 => fixture.messages[1].message.chat_id = "other".into(),
            3 => fixture.topics[1].chat_id = "other".into(),
            4 => fixture.topics[1].state = TopicState::Closed,
            5 => {
                fixture.messages[1]
                    .message
                    .reply_to
                    .as_mut()
                    .unwrap()
                    .resolved = None
            }
            6 => {
                fixture.messages[1]
                    .message
                    .reply_to
                    .as_mut()
                    .unwrap()
                    .resolved = Some(id(99))
            }
            7 => fixture.messages[0].cursor.ordinal = 3,
            8 => fixture.messages[1].cursor = fixture.messages[0].cursor,
            _ => unreachable!(),
        }
        assert_eq!(fixture.graph().count(), 0, "case {invalid_case}");
    }
}

#[test]
fn deterministic_ranking_uses_weight_then_ordered_topic_pair() {
    let mut fixture = Fixture::new(&[
        ("a", None),
        ("b", Some(0)),
        ("b", Some(0)),
        ("c", Some(0)),
        ("c", Some(0)),
        ("d", Some(0)),
        ("d", Some(0)),
        ("d", Some(0)),
    ]);
    let mut graph = fixture.graph();
    fixture.messages.reverse();
    fixture.topics.reverse();
    assert_eq!(graph.next(), fixture.graph().next());
    assert_eq!(graph.count(), 3);
    let first = graph.next().unwrap();
    assert_eq!(first.from.as_ref(), "d");
    graph.reject(&first);
    assert_eq!(graph.next().unwrap().from.as_ref(), "b");
    graph.reject(&graph.next().unwrap());
    assert_eq!(graph.next().unwrap().from.as_ref(), "c");
}

#[test]
fn evidence_is_bounded_deduplicated_and_keeps_in_scope_support() {
    let mut fixture = Fixture::new(&[
        ("a", None),
        ("b", Some(0)),
        ("b", Some(0)),
        ("b", Some(0)),
        ("b", Some(0)),
        ("b", Some(0)),
    ]);
    fixture.in_scope = BTreeSet::from([id(5)]);
    let candidate = fixture.graph().next().unwrap();
    assert_eq!(candidate.reply_edges, 5);
    assert_eq!(candidate.evidence, [id(5), id(0), id(1), id(2)]);
}

#[test]
fn merging_three_topics_accumulates_below_threshold_edges_without_recounting() {
    let fixture = Fixture::new(&[
        ("a", None),
        ("b", Some(0)),
        ("b", Some(0)),
        ("c", Some(0)),
        ("c", Some(1)),
    ]);
    let mut graph = fixture.graph();
    assert_eq!(graph.count(), 1);
    let first = graph.next().unwrap();
    graph.merge(&first);
    let combined = graph.next().unwrap();
    assert_eq!(combined.into.as_ref(), "a");
    assert_eq!(combined.from.as_ref(), "c");
    assert_eq!(combined.reply_edges, 2);
    assert_eq!(combined.evidence, [id(3), id(0), id(4), id(1)]);
    graph.merge(&combined);
    assert_eq!(graph.count(), 0);
    graph.merge(&first); // A stale accepted candidate cannot revive removed B.
    assert_eq!(graph.count(), 0);
}

#[test]
fn scope_support_is_not_transferred_from_removed_internal_edges() {
    let mut fixture = Fixture::new(&[
        ("a", None),
        ("b", Some(0)),
        ("b", Some(0)),
        ("c", Some(0)),
        ("c", Some(1)),
    ]);
    fixture.in_scope = BTreeSet::from([id(1)]);
    let mut graph = fixture.graph();
    graph.merge(&graph.next().unwrap());
    assert_eq!(graph.count(), 0);
}

#[test]
fn persisted_rejections_without_support_inherit_across_merges() {
    let mut fixture = Fixture::new(&[
        ("a", None),
        ("b", Some(0)),
        ("b", Some(0)),
        ("c", Some(0)),
        ("c", Some(0)),
    ]);
    // Reversed persisted pairs are normalized; B-C has no graph edge yet.
    fixture.rejected.insert(("c".into(), "b".into()));
    let mut graph = fixture.graph();
    assert_eq!(graph.count(), 2);
    graph.merge(&graph.next().unwrap());
    assert_eq!(graph.count(), 0);
}

#[test]
fn rejected_candidates_survive_merges_and_old_aliases_cannot_reintroduce_them() {
    let fixture = Fixture::new(&[
        ("a", None),
        ("b", Some(0)),
        ("b", Some(0)),
        ("c", Some(1)),
        ("c", Some(1)),
    ]);
    let mut graph = fixture.graph();
    let initial_merge = graph.next().unwrap();
    graph.reject(&initial_merge);
    let old_bc = graph.next().unwrap();
    graph.merge(&old_bc);
    assert_eq!(graph.count(), 0);
    graph.reject(&old_bc);
    assert_eq!(graph.count(), 0);

    let mut graph = fixture.graph();
    // Take a candidate from the same initial graph, but apply its rejection after
    // one endpoint was merged. It must block the canonical A-C pair.
    graph.merge(&initial_merge);
    assert_eq!(graph.count(), 1);
    graph.reject(&old_bc);
    assert_eq!(graph.count(), 0);
}
