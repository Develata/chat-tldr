use chat_tldr_core::{Mention, ReplyRef, SourceMeta, UnifiedMessage};
use chrono::{DateTime, Duration, FixedOffset};

use super::*;

fn time(milliseconds: i64) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-26T10:00:00+08:00").unwrap()
        + Duration::milliseconds(milliseconds)
}

fn message(id: i64, milliseconds: i64, sender: &str, topic: Option<&str>) -> StoredMessage {
    StoredMessage {
        message: UnifiedMessage {
            id: format!("m_{id}").into(),
            chat_id: "qq:group:synthetic".into(),
            sender: format!("qq:{sender}").into(),
            sender_display: "same display name".into(),
            sent_at: time(milliseconds),
            text: "synthetic message".into(),
            mentions: vec![],
            reply_to: None,
            attachments: vec![],
            forward: None,
            recalled: false,
            system: false,
            source: SourceMeta {
                format: "synthetic".into(),
                identity: format!("synthetic:{id}"),
                qce_id: None,
                qce_seq: None,
                qce_type: None,
                file_hash: "synthetic".into(),
            },
        },
        cursor: Cursor {
            sent_at_ms: time(milliseconds).timestamp_millis(),
            ordinal: id,
        },
        analysis_state: "pending".into(),
        topic_id: topic.map(Into::into),
    }
}

fn reply(message: &mut StoredMessage, target: Option<i64>) {
    message.message.reply_to = Some(ReplyRef {
        source_message_id: "synthetic".into(),
        resolved: target.map(|id| format!("m_{id}").into()),
    });
}

fn mention(uid: Option<&str>, uin: Option<&str>) -> Mention {
    Mention {
        target: MentionTarget::User {
            uid: uid.map(Into::into),
            uin: uin.map(Into::into),
        },
        display: "same display name".into(),
    }
}

fn topic(id: &str, milliseconds: i64) -> TopicRecord {
    TopicRecord {
        id: id.into(),
        chat_id: "qq:group:synthetic".into(),
        title: "synthetic topic".into(),
        provisional: false,
        state: TopicState::Active,
        last_message_at: time(milliseconds),
        is_chitchat: None,
    }
}

fn config() -> SegmentConfig {
    crate::Config::parse("").unwrap().segment
}

fn ids(topics: Vec<&TopicRecord>) -> Vec<&str> {
    topics.into_iter().map(|topic| topic.id.as_ref()).collect()
}

#[test]
fn reply_shortcut_requires_every_external_reply_to_resolve_and_agree() {
    let links = TopicLinks::new(&[
        message(1, 0, "a", Some("first")),
        message(2, 1, "b", Some("first")),
        message(3, 2, "c", Some("second")),
        message(4, 3, "d", None),
    ]);
    let mut burst = vec![message(5, 1000, "e", None), message(6, 1001, "f", None)];
    assert_eq!(links.reply_topic(&burst), None);
    reply(&mut burst[0], Some(1));
    reply(&mut burst[1], Some(2));
    assert_eq!(links.reply_topic(&burst), Some(&TopicId::from("first")));
    reply(&mut burst[1], Some(5)); // Internal replies do not prevent the shortcut.
    assert_eq!(links.reply_topic(&burst), Some(&TopicId::from("first")));
    for target in [None, Some(3), Some(4), Some(99)] {
        reply(&mut burst[1], target);
        assert_eq!(links.reply_topic(&burst), None);
    }
}

#[test]
fn historical_window_uses_member_milliseconds_and_accepts_closed_backfills() {
    let burst = [message(10, 10_000, "a", None)];
    let mut config = config();
    config.topic_close_secs = 2;
    let mut closed = topic("closed", 10_000);
    closed.state = TopicState::Closed;
    let mut other_chat = topic("other", 10_000);
    other_chat.chat_id = "qq:group:other".into();
    let topics = [
        topic("past_boundary", 8000),
        topic("past_outside", 7999),
        topic("future_boundary", 12_000),
        topic("future_outside", 12_001),
        closed,
        other_chat,
    ];
    let mut foreign = message(6, 9000, "other", Some("other"));
    foreign.message.chat_id = "qq:group:other".into();
    let mut links = TopicLinks::new(&[
        message(1, 8000, "past", Some("past_boundary")),
        message(2, 7999, "past", Some("past_outside")),
        message(3, 12_000, "future", Some("future_boundary")),
        message(4, 12_001, "future", Some("future_outside")),
        message(5, 9500, "closed", Some("closed")),
        foreign,
    ]);
    links.mark_closed(&"closed".into(), time(20_000));
    assert_eq!(
        ids(links.select(&burst, &topics, &config)),
        ["closed", "past_boundary"]
    );
    assert!(links.eligible(&topics[0], &burst, &config));
    assert!(links.has_candidates(&burst, &topics, &config));
    assert!(!links.has_candidates(&burst, &topics[2..4], &config));
    config.topic_close_secs = 0;
    assert!(!links.eligible(&topics[0], &burst, &config));
    assert!(!links.has_candidates(&burst, &topics, &config));
    let topics = [topic("same", 10_000), topic("one_ms", 10_001)];
    let links = TopicLinks::new(&[
        message(1, 10_000, "past", Some("same")),
        message(2, 10_001, "future", Some("one_ms")),
    ]);
    assert_eq!(ids(links.select(&burst, &topics, &config)), ["same"]);
}

#[test]
fn exact_uid_and_uin_edges_rank_ahead_of_more_recent_unlinked_topics() {
    let links = TopicLinks::new(&[
        message(1, 0, "u_alice", Some("uid")),
        message(2, 0, "123456", Some("uin")),
        message(3, 0, "other", Some("recent")),
    ]);
    let mut burst = [message(4, 1000, "writer", None)];
    burst[0].message.mentions = vec![
        mention(Some("u_alice"), None),
        mention(None, Some("123456")),
    ];
    let topics = [topic("recent", 999), topic("uin", 0), topic("uid", 0)];
    assert_eq!(
        ids(links.select(&burst, &topics, &config())),
        ["uid", "uin", "recent"]
    );
}

#[test]
fn mentions_do_not_guess_names_use_future_or_recalled_messages_or_expand_all() {
    let mut recalled = message(3, 0, "recalled", Some("recalled"));
    recalled.message.recalled = true;
    let links = TopicLinks::new(&[
        message(1, 2000, "future", Some("future")),
        message(2, 0, "u_actual", Some("name")),
        recalled,
        message(4, 0, "unknown", Some("unknown")),
    ]);
    let mut burst = [message(5, 1000, "writer", None)];
    burst[0].message.mentions = vec![
        mention(Some("future"), None),
        mention(Some("recalled"), None),
        mention(Some("unknown"), None),
        mention(Some("same display name"), None),
        Mention {
            target: MentionTarget::All,
            display: "everyone".into(),
        },
    ];
    assert!(links.edges(&burst, &burst[0]).is_empty());
    reply(&mut burst[0], Some(1));
    assert_eq!(links.reply_topic(&burst), None);
    reply(&mut burst[0], Some(3));
    assert_eq!(links.reply_topic(&burst), None);
}

#[test]
fn repeated_mentions_and_reply_from_one_source_count_as_one_edge() {
    let links = TopicLinks::new(&[
        message(1, 0, "u_alice", Some("topic")),
        message(2, 1, "u_alice", Some("topic")),
        message(3, 2, "u_bob", Some("topic")),
    ]);
    let mut burst = [message(4, 1000, "writer", None)];
    burst[0].message.mentions = vec![
        mention(Some("u_alice"), Some("u_alice")),
        mention(Some("u_alice"), None),
        mention(Some("u_bob"), None),
    ];
    reply(&mut burst[0], Some(1));
    assert_eq!(
        links.edges(&burst, &burst[0]).get(&TopicId::from("topic")),
        Some(&1)
    );
}

#[test]
fn count_limit_preserves_all_or_best_k_with_stable_ties() {
    let mut config = config();
    config.all_candidates_max = 3;
    config.candidate_k = 2;
    let burst = [message(10, 1000, "writer", None)];
    let topics = [
        topic("z", 0),
        topic("b", 999),
        topic("a", 999),
        topic("c", 998),
    ];
    let links = TopicLinks::new(&[
        message(1, 0, "writer", Some("z")),
        message(2, 999, "writer", Some("b")),
        message(3, 999, "writer", Some("a")),
        message(4, 998, "writer", Some("c")),
    ]);
    assert_eq!(
        ids(links.select(&burst, &topics[..3], &config)),
        ["a", "b", "z"]
    );
    assert_eq!(ids(links.select(&burst, &topics, &config)), ["a", "b"]);
    config.gamma = 0.0;
    config.alpha = 1000.0; // No embedding feature may influence selection.
    assert_eq!(ids(links.select(&burst, &topics, &config)), ["a", "b"]);
}

#[test]
fn links_are_capped_at_three_before_combining_with_time_decay() {
    let links = TopicLinks::new(&[
        message(1, 0, "old", Some("old")),
        message(2, 1, "recent", Some("recent")),
    ]);
    let mut burst: Vec<_> = (3..7)
        .map(|id| message(id, 1000 + id, "writer", None))
        .collect();
    for (index, message) in burst.iter_mut().enumerate() {
        message.message.mentions.push(mention(Some("old"), None));
        if index < 3 {
            message.message.mentions.push(mention(Some("recent"), None));
        }
    }
    let topics = [topic("old", 0), topic("recent", 1)];
    assert_eq!(
        ids(links.select(&burst, &topics, &config())),
        ["recent", "old"]
    );
    assert!(
        links
            .edges(&burst, &burst[0])
            .values()
            .all(|count| *count == 3)
    );
}

#[test]
fn incremental_assignment_reassignment_and_recall_remove_stale_edges() {
    let mut assigned = message(1, 0, "u_alice", None);
    let mut links = TopicLinks::new(std::slice::from_ref(&assigned));
    let mut burst = [message(2, 1000, "writer", None)];
    burst[0].message.mentions = vec![mention(Some("u_alice"), None)];
    reply(&mut burst[0], Some(1));
    assert_eq!(links.reply_topic(&burst), None);
    links.assign(std::slice::from_ref(&assigned), &"first".into());
    links.assign(std::slice::from_ref(&assigned), &"first".into());
    assert_eq!(links.reply_topic(&burst), Some(&TopicId::from("first")));
    links.assign(std::slice::from_ref(&assigned), &"second".into());
    assert_eq!(links.reply_topic(&burst), Some(&TopicId::from("second")));
    let edges = links.edges(&burst, &burst[0]);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges.get(&TopicId::from("second")), Some(&1));
    assigned.message.recalled = true;
    links.assign(&[assigned], &"second".into());
    assert_eq!(links.reply_topic(&burst), None);
    assert!(links.edges(&burst, &burst[0]).is_empty());
    assert!(links.senders.is_empty());
    assert!(links.activity.is_empty());
}

#[test]
fn cursor_order_excludes_same_time_future_and_empty_or_recalled_bursts() {
    let links = TopicLinks::new(&[
        message(1, 1000, "past", Some("past")),
        message(3, 1000, "future", Some("future")),
    ]);
    let mut burst = [message(2, 1000, "writer", None)];
    burst[0].message.mentions = vec![mention(Some("past"), None), mention(Some("future"), None)];
    let edges = links.edges(&burst, &burst[0]);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges.get(&TopicId::from("past")), Some(&1));
    let topics = [topic("past", 1000), topic("future", 1000)];
    assert_eq!(ids(links.select(&burst, &topics, &config())), ["past"]);
    assert!(links.eligible(&topics[0], &burst, &config()));
    assert!(!links.eligible(&topics[1], &burst, &config()));
    assert!(links.select(&[], &topics, &config()).is_empty());
    assert!(!links.has_candidates(&[], &topics, &config()));
    burst[0].message.recalled = true;
    assert!(links.select(&burst, &topics, &config()).is_empty());
    assert!(!links.has_candidates(&burst, &topics, &config()));
}

#[test]
fn future_activity_cannot_override_the_historical_predecessor_or_create_a_candidate() {
    let links = TopicLinks::new(&[
        message(1, 1000, "old", Some("old")),
        message(2, 5000, "recent", Some("recent")),
        message(3, 20_000, "future", Some("old")),
        message(4, 30_000, "future", Some("only_future")),
    ]);
    let burst = [message(5, 6000, "writer", None)];
    let topics = [
        topic("old", 20_000),
        topic("recent", 5000),
        topic("only_future", 30_000),
        topic("without_members", 5999),
    ];
    assert_eq!(
        ids(links.select(&burst, &topics, &config())),
        ["recent", "old"]
    );
    let mut tie_config = config();
    tie_config.beta = 0.0;
    tie_config.gamma = 0.0;
    assert_eq!(
        ids(links.select(&burst, &topics, &tie_config)),
        ["recent", "old"]
    );
    assert!(!links.has_candidates(&burst, &topics[2..], &config()));
    assert!(!TopicLinks::default().has_candidates(&burst, &topics, &config()));
}

#[test]
fn merged_topics_never_return_and_closed_topics_require_an_actual_recent_predecessor() {
    let mut links = TopicLinks::new(&[
        message(1, 1000, "writer", Some("closed")),
        message(2, 6000, "writer", Some("closed")),
        message(3, 4999, "writer", Some("merged")),
    ]);
    links.mark_closed(&"closed".into(), time(7000));
    let mut closed = topic("closed", 6000);
    closed.state = TopicState::Closed;
    let mut merged = topic("merged", 4999);
    merged.state = TopicState::Merged;
    let topics = [closed, merged];
    let mut options = config();
    options.topic_close_secs = 2;
    let before_gap = [message(4, 3000, "writer", None)];
    assert_eq!(
        ids(links.select(&before_gap, &topics, &options)),
        ["closed"]
    );
    let in_gap = [message(5, 5000, "writer", None)];
    assert!(links.select(&in_gap, &topics, &options).is_empty());
    assert!(!links.has_candidates(&in_gap, &topics, &options));
    // Even exact reply links cannot waive topic lifecycle/time eligibility.
    let mut reply_burst = [message(6, 5000, "writer", None)];
    reply(&mut reply_burst[0], Some(3));
    assert_eq!(links.reply_topic(&reply_burst), Some(&topics[1].id));
    assert!(!links.eligible(&topics[1], &reply_burst, &options));
}

#[test]
fn incremental_activity_tracks_reassignment_time_changes_and_recalls_without_sender_identity() {
    let mut stored = message(1, 1000, "unknown", Some("first"));
    let mut links = TopicLinks::new(std::slice::from_ref(&stored));
    let topics = [topic("first", 1000), topic("second", 1000)];
    let burst = [message(2, 2000, "writer", None)];
    assert_eq!(ids(links.select(&burst, &topics, &config())), ["first"]);
    assert!(links.senders.is_empty());
    links.assign(std::slice::from_ref(&stored), &topics[1].id);
    links.assign(std::slice::from_ref(&stored), &topics[1].id);
    assert_eq!(ids(links.select(&burst, &topics, &config())), ["second"]);
    assert!(links.senders.is_empty());
    stored.cursor.sent_at_ms = time(3000).timestamp_millis();
    stored.message.sent_at = time(3000);
    links.assign(std::slice::from_ref(&stored), &topics[1].id);
    assert!(!links.has_candidates(&burst, &topics, &config()));
    stored.cursor.sent_at_ms = time(1000).timestamp_millis();
    stored.message.sent_at = time(1000);
    links.assign(std::slice::from_ref(&stored), &topics[0].id);
    assert!(links.eligible(&topics[0], &burst, &config()));
    stored.message.recalled = true;
    links.assign(&[stored], &topics[0].id);
    assert!(!links.has_candidates(&burst, &topics, &config()));
    assert!(links.activity.is_empty());
    assert!(links.senders.is_empty());
    assert!(links.messages.is_empty());
}

#[test]
fn activity_uses_the_predecessor_from_the_same_chat_and_falls_back_after_recall() {
    let mut newest = message(2, 1900, "writer", Some("topic"));
    let older = message(1, 1000, "writer", Some("topic"));
    let mut foreign = message(3, 1999, "writer", Some("topic"));
    foreign.message.chat_id = "qq:group:other".into();
    let mut links = TopicLinks::new(&[older, newest.clone(), foreign]);
    let topics = [topic("topic", 1999)];
    let burst = [message(4, 2500, "writer", None)];
    let mut options = config();
    options.topic_close_secs = 1;
    assert!(links.has_candidates(&burst, &topics, &options));
    newest.message.recalled = true;
    links.assign(&[newest], &topics[0].id);
    assert!(!links.has_candidates(&burst, &topics, &options));
}

#[test]
fn backfills_cannot_extend_closed_topics_into_live_or_later_unrelated_history() {
    const HOUR: i64 = 3_600_000;
    let mut links = TopicLinks::new(&[
        message(1, 9 * HOUR, "past", Some("closed")),
        message(2, 16 * HOUR, "other", Some("other")),
    ]);
    let mut closed = topic("closed", 9 * HOUR);
    closed.state = TopicState::Closed;
    let mut backfill = message(3, 15 * HOUR, "past", None);
    // A legacy Closed record without an explicit historical boundary is not a
    // candidate: neither global last activity nor unrelated topics may guess it.
    assert!(!links.eligible(&closed, std::slice::from_ref(&backfill), &config()));
    links.mark_closed(&closed.id, time(16 * HOUR));
    assert!(links.eligible(&closed, std::slice::from_ref(&backfill), &config()));
    backfill.topic_id = Some(closed.id.clone());
    links.assign(&[backfill], &closed.id);
    closed.last_message_at = time(15 * HOUR);
    let after_close = [message(4, 17 * HOUR, "past", None)];
    assert!(!links.eligible(&closed, &after_close, &config()));
    assert!(!links.has_candidates(&after_close, std::slice::from_ref(&closed), &config()));
    links.assign(&[message(5, 20 * HOUR, "other", None)], &"other".into());
    assert!(
        links
            .select(&after_close, std::slice::from_ref(&closed), &config())
            .is_empty()
    );
    links.mark_closed(&closed.id, time(24 * HOUR));
    links.mark_closed(&closed.id, time(14 * HOUR));
    assert!(!links.eligible(&closed, &after_close, &config()));
    assert!(links.eligible(
        &closed,
        &[message(6, 15 * HOUR + 1, "past", None)],
        &config()
    ));
    // Persisted closure metadata never constrains an explicitly Active topic.
    closed.state = TopicState::Active;
    assert!(links.eligible(&closed, &after_close, &config()));
}

#[test]
fn closed_candidate_rejects_the_entire_burst_at_or_across_its_boundary() {
    let mut links = TopicLinks::new(&[message(1, 1000, "past", Some("closed"))]);
    let mut closed = topic("closed", 1000);
    closed.state = TopicState::Closed;
    links.mark_closed(&closed.id, time(3000));
    let before = message(2, 2999, "writer", None);
    let boundary = message(3, 3000, "writer", None);
    let after = message(4, 3001, "writer", None);
    assert!(links.eligible(&closed, std::slice::from_ref(&before), &config()));
    assert!(!links.eligible(&closed, std::slice::from_ref(&boundary), &config()));
    let mut burst = [after, before]; // The guard must not depend on slice order.
    assert!(!links.eligible(&closed, &burst, &config()));
    assert!(!links.has_candidates(&burst, std::slice::from_ref(&closed), &config()));
    assert!(
        links
            .select(&burst, std::slice::from_ref(&closed), &config())
            .is_empty()
    );
    burst[0].message.recalled = true;
    assert!(links.eligible(&closed, &burst, &config()));
}
