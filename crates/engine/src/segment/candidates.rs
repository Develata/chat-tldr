//! Incremental, deterministic topic candidates. No model calls or persistence.

use std::collections::{BTreeMap, BTreeSet};

use chat_tldr_core::{ChatId, Cursor, MentionTarget, MessageId, TopicId, TopicState};

use crate::{
    config::SegmentConfig,
    store::{StoredMessage, TopicRecord},
};

struct IndexedMessage {
    chat_id: ChatId,
    sender: Option<String>,
    cursor: Cursor,
    topic_id: TopicId,
}

type SenderTopics = BTreeMap<String, BTreeMap<TopicId, BTreeSet<Cursor>>>;

/// Only identities, cursors and assignments are indexed; message bodies stay borrowed.
/// Building costs O(M log M), space O(M). Assignment touches only changed messages.
/// Selection visits T topics and relevant sender postings, never rescans M messages;
/// ranking costs O(T log T), and each reply lookup costs O(log M).
#[derive(Default)]
pub(crate) struct TopicLinks {
    messages: BTreeMap<MessageId, IndexedMessage>,
    senders: BTreeMap<ChatId, SenderTopics>,
}

impl TopicLinks {
    pub(crate) fn new(messages: &[StoredMessage]) -> Self {
        let mut links = Self::default();
        for message in messages {
            if let Some(topic) = &message.topic_id {
                links.insert(message, topic);
            }
        }
        links
    }

    /// Update immediately after a successful assignment. Repeating an assignment,
    /// changing its topic, or replacing it with a recalled message is safe.
    pub(crate) fn assign(&mut self, messages: &[StoredMessage], topic: &TopicId) {
        for message in messages {
            self.insert(message, topic);
        }
    }

    fn insert(&mut self, stored: &StoredMessage, topic: &TopicId) {
        let message = &stored.message;
        if let Some(previous) = self.messages.remove(&message.id)
            && let Some(sender) = previous.sender
            && let Some(senders) = self.senders.get_mut(&previous.chat_id)
        {
            if let Some(topics) = senders.get_mut(&sender) {
                if let Some(cursors) = topics.get_mut(&previous.topic_id) {
                    cursors.remove(&previous.cursor);
                    if cursors.is_empty() {
                        topics.remove(&previous.topic_id);
                    }
                }
                if topics.is_empty() {
                    senders.remove(&sender);
                }
            }
            if senders.is_empty() {
                self.senders.remove(&previous.chat_id);
            }
        }
        if message.recalled {
            return;
        }
        let sender = message
            .sender
            .as_ref()
            .strip_prefix("qq:")
            .filter(|identity| usable_identity(identity))
            .map(str::to_owned);
        if let Some(sender) = &sender {
            self.senders
                .entry(message.chat_id.clone())
                .or_default()
                .entry(sender.clone())
                .or_default()
                .entry(topic.clone())
                .or_default()
                .insert(stored.cursor);
        }
        self.messages.insert(
            message.id.clone(),
            IndexedMessage {
                chat_id: message.chat_id.clone(),
                sender,
                cursor: stored.cursor,
                topic_id: topic.clone(),
            },
        );
    }

    /// A shortcut needs at least one external reply and agreement among every
    /// external reply. Unresolved, unassigned, recalled or future targets reject it.
    /// Intra-burst replies do not yet identify an existing topic and are ignored.
    /// The caller must additionally check the returned topic's active/time window.
    pub(crate) fn reply_topic(&self, burst: &[StoredMessage]) -> Option<&TopicId> {
        let start = burst_start(burst)?;
        let local_ids: BTreeSet<_> = burst.iter().map(|m| &m.message.id).collect();
        let mut found = None;
        for stored in burst.iter().filter(|m| !m.message.recalled) {
            let Some(reply) = &stored.message.reply_to else {
                continue;
            };
            let target = reply.resolved.as_ref()?;
            if local_ids.contains(target) {
                continue;
            }
            let target = self.messages.get(target)?;
            if target.chat_id != start.message.chat_id || target.cursor >= start.cursor {
                return None;
            }
            if found.is_some_and(|topic| topic != &target.topic_id) {
                return None;
            }
            found = Some(&target.topic_id);
        }
        found
    }

    /// Return an ordered prefix: all candidates if within the count limit, top K
    /// otherwise. Runtime applies the serialized state/question token budget next.
    pub(crate) fn select<'a>(
        &self,
        burst: &[StoredMessage],
        topics: &'a [TopicRecord],
        config: &SegmentConfig,
    ) -> Vec<&'a TopicRecord> {
        let Some(start) = burst_start(burst) else {
            return vec![];
        };
        let edges = self.edges(burst, start);
        let mut ranked: Vec<_> = topics
            .iter()
            .filter(|topic| eligible_at(topic, start, config))
            .map(|topic| {
                let edge_count = edges.get(&topic.id).copied().unwrap_or(0);
                let decay = if config.topic_close_secs == 0 {
                    0.0
                } else {
                    distance_ms(topic, start) as f64 / (config.topic_close_secs as f64 * 1000.0)
                };
                // Embeddings are not configured: alpha is deliberately zero.
                let score = f64::from(config.beta) * f64::from(edge_count) / 3.0
                    - f64::from(config.gamma) * decay;
                (topic, score)
            })
            .collect();
        ranked.sort_unstable_by(|(left, left_score), (right, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| right.last_message_at.cmp(&left.last_message_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        if ranked.len() > config.all_candidates_max as usize {
            ranked.truncate(config.candidate_k as usize);
        }
        ranked.into_iter().map(|(topic, _)| topic).collect()
    }

    fn edges<'a>(
        &'a self,
        burst: &[StoredMessage],
        start: &StoredMessage,
    ) -> BTreeMap<&'a TopicId, u8> {
        let mut edges: BTreeMap<&TopicId, u8> = BTreeMap::new();
        let senders = self.senders.get(&start.message.chat_id);
        for stored in burst.iter().filter(|m| !m.message.recalled) {
            // One source message contributes at most one edge to each topic,
            // including repeated mentions and a reply plus @ to the same topic.
            let mut linked = BTreeSet::new();
            if let Some(target) = stored
                .message
                .reply_to
                .as_ref()
                .and_then(|reply| reply.resolved.as_ref())
                .and_then(|id| self.messages.get(id))
                && target.chat_id == start.message.chat_id
                && target.cursor < start.cursor
            {
                linked.insert(&target.topic_id);
            }
            for mention in &stored.message.mentions {
                let MentionTarget::User { uid, uin } = &mention.target else {
                    continue;
                };
                for identity in [uid.as_deref(), uin.as_deref()].into_iter().flatten() {
                    if !usable_identity(identity) {
                        continue;
                    }
                    if let Some(topics) = senders.and_then(|senders| senders.get(identity)) {
                        for (topic, cursors) in topics {
                            if cursors.first().is_some_and(|cursor| *cursor < start.cursor) {
                                linked.insert(topic);
                            }
                        }
                    }
                }
            }
            for topic in linked {
                let count = edges.entry(topic).or_default();
                *count = (*count + 1).min(3);
            }
        }
        edges
    }
}

/// Use absolute distance for historical backfills as well as forward ingestion.
/// Milliseconds avoid accepting a topic just beyond the configured boundary.
pub(crate) fn eligible(
    topic: &TopicRecord,
    burst: &[StoredMessage],
    config: &SegmentConfig,
) -> bool {
    burst_start(burst).is_some_and(|start| eligible_at(topic, start, config))
}

fn eligible_at(topic: &TopicRecord, start: &StoredMessage, config: &SegmentConfig) -> bool {
    topic.state == TopicState::Active
        && topic.chat_id == start.message.chat_id
        && distance_ms(topic, start) <= config.topic_close_secs.saturating_mul(1000)
}

fn distance_ms(topic: &TopicRecord, start: &StoredMessage) -> u64 {
    (start.message.sent_at - topic.last_message_at)
        .num_milliseconds()
        .unsigned_abs()
}

fn burst_start(burst: &[StoredMessage]) -> Option<&StoredMessage> {
    burst
        .iter()
        .filter(|m| !m.message.recalled)
        .min_by_key(|m| m.cursor)
}

fn usable_identity(identity: &str) -> bool {
    !identity.is_empty() && !matches!(identity, "unknown" | "system")
}

#[cfg(test)]
mod tests {
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
    fn active_window_uses_milliseconds_and_accepts_bounded_backfills() {
        let burst = [message(1, 10_000, "a", None)];
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
        assert_eq!(
            ids(TopicLinks::default().select(&burst, &topics, &config)),
            ["future_boundary", "past_boundary"]
        );
        assert!(eligible(&topics[0], &burst, &config));
        config.topic_close_secs = 0;
        assert!(!eligible(&topics[0], &burst, &config));
        let topics = [topic("same", 10_000), topic("one_ms", 10_001)];
        assert_eq!(
            ids(TopicLinks::default().select(&burst, &topics, &config)),
            ["same"]
        );
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
        let burst = [message(1, 1000, "writer", None)];
        let topics = [
            topic("z", 0),
            topic("b", 999),
            topic("a", 999),
            topic("c", 998),
        ];
        let links = TopicLinks::default();
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
    }

    #[test]
    fn cursor_order_excludes_same_time_future_and_empty_or_recalled_bursts() {
        let links = TopicLinks::new(&[
            message(1, 1000, "past", Some("past")),
            message(3, 1000, "future", Some("future")),
        ]);
        let mut burst = [message(2, 1000, "writer", None)];
        burst[0].message.mentions =
            vec![mention(Some("past"), None), mention(Some("future"), None)];
        let edges = links.edges(&burst, &burst[0]);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges.get(&TopicId::from("past")), Some(&1));
        let topics = [topic("past", 1000), topic("future", 1000)];
        assert!(links.select(&[], &topics, &config()).is_empty());
        burst[0].message.recalled = true;
        assert!(links.select(&burst, &topics, &config()).is_empty());
    }
}
