//! Incremental, deterministic topic candidates. No model calls or persistence.

use std::collections::{BTreeMap, BTreeSet};

use chat_tldr_core::{ChatId, Cursor, MentionTarget, MessageId, TopicId, TopicState};
use chrono::{DateTime, FixedOffset};

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

type TopicCursors = BTreeMap<TopicId, BTreeSet<Cursor>>;
type SenderTopics = BTreeMap<String, TopicCursors>;

/// Only identities, cursors and assignments are indexed; message bodies stay borrowed.
/// Building costs O(M log M); space is O(M + C), including C recorded close times.
/// Assignment touches only changed messages; each close marker costs O(log C).
/// Selection visits T topics and relevant sender postings, never rescans M messages;
/// each historical activity lookup costs O(log M), ranking costs O(T log T).
#[derive(Default)]
pub(crate) struct TopicLinks {
    messages: BTreeMap<MessageId, IndexedMessage>,
    senders: BTreeMap<ChatId, SenderTopics>,
    activity: BTreeMap<ChatId, TopicCursors>,
    closed_at: BTreeMap<TopicId, DateTime<FixedOffset>>,
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

    /// Remember the first committed message-time boundary. Backfills and later
    /// activity in other topics must never slide a closed topic's boundary.
    pub(crate) fn mark_closed(&mut self, topic: &TopicId, at: DateTime<FixedOffset>) {
        self.closed_at.entry(topic.clone()).or_insert(at);
    }

    fn insert(&mut self, stored: &StoredMessage, topic: &TopicId) {
        let message = &stored.message;
        if let Some(previous) = self.messages.remove(&message.id) {
            if let Some(topics) = self.activity.get_mut(&previous.chat_id) {
                remove_cursor(topics, &previous.topic_id, previous.cursor);
                if topics.is_empty() {
                    self.activity.remove(&previous.chat_id);
                }
            }
            if let Some(sender) = previous.sender
                && let Some(senders) = self.senders.get_mut(&previous.chat_id)
            {
                if let Some(topics) = senders.get_mut(&sender) {
                    remove_cursor(topics, &previous.topic_id, previous.cursor);
                    if topics.is_empty() {
                        senders.remove(&sender);
                    }
                }
                if senders.is_empty() {
                    self.senders.remove(&previous.chat_id);
                }
            }
        }
        if message.recalled {
            return;
        }
        self.activity
            .entry(message.chat_id.clone())
            .or_default()
            .entry(topic.clone())
            .or_default()
            .insert(stored.cursor);
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
    /// The caller must additionally check the returned topic's historical window.
    pub(crate) fn reply_topic(&self, burst: &[StoredMessage]) -> Option<&TopicId> {
        let start = burst_window(burst)?.start;
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

    pub(crate) fn eligible(
        &self,
        topic: &TopicRecord,
        burst: &[StoredMessage],
        config: &SegmentConfig,
    ) -> bool {
        burst_window(burst)
            .is_some_and(|window| self.activity_before(topic, &window, config).is_some())
    }

    /// Check whether direct extraction would bypass a possible historical topic.
    /// No sorting or reply/@ scoring is needed; stop at the first eligible topic.
    pub(crate) fn has_candidates(
        &self,
        burst: &[StoredMessage],
        topics: &[TopicRecord],
        config: &SegmentConfig,
    ) -> bool {
        let Some(window) = burst_window(burst) else {
            return false;
        };
        topics
            .iter()
            .any(|topic| self.activity_before(topic, &window, config).is_some())
    }

    /// A topic is active at this burst only when it has a recent non-recalled
    /// member strictly before the burst cursor. Closed topics additionally need
    /// every non-recalled burst member before their first durable closing time.
    /// Merged topics and future-only membership never qualify.
    fn activity_before(
        &self,
        topic: &TopicRecord,
        window: &BurstWindow<'_>,
        config: &SegmentConfig,
    ) -> Option<Cursor> {
        if !matches!(topic.state, TopicState::Active | TopicState::Closed)
            || topic.chat_id != window.start.message.chat_id
        {
            return None;
        }
        if topic.state == TopicState::Closed && window.last_at >= *self.closed_at.get(&topic.id)? {
            return None;
        }
        let start = window.start;
        let cursor = *self
            .activity
            .get(&topic.chat_id)?
            .get(&topic.id)?
            .range(..start.cursor)
            .next_back()?;
        (start.cursor.sent_at_ms.abs_diff(cursor.sent_at_ms)
            <= config.topic_close_secs.saturating_mul(1000))
        .then_some(cursor)
    }

    /// Return an ordered prefix: all candidates if within the count limit, top K
    /// otherwise. Runtime applies the serialized state/question token budget next.
    pub(crate) fn select<'a>(
        &self,
        burst: &[StoredMessage],
        topics: &'a [TopicRecord],
        config: &SegmentConfig,
    ) -> Vec<&'a TopicRecord> {
        let Some(window) = burst_window(burst) else {
            return vec![];
        };
        let start = window.start;
        let edges = self.edges(burst, start);
        let mut ranked: Vec<_> = topics
            .iter()
            .filter_map(|topic| {
                let cursor = self.activity_before(topic, &window, config)?;
                let edge_count = edges.get(&topic.id).copied().unwrap_or(0);
                let decay = if config.topic_close_secs == 0 {
                    0.0
                } else {
                    start.cursor.sent_at_ms.abs_diff(cursor.sent_at_ms) as f64
                        / (config.topic_close_secs as f64 * 1000.0)
                };
                // Embeddings are not configured: alpha is deliberately zero.
                let score = f64::from(config.beta) * f64::from(edge_count) / 3.0
                    - f64::from(config.gamma) * decay;
                Some((topic, score, cursor.sent_at_ms))
            })
            .collect();
        ranked.sort_unstable_by(
            |(left, left_score, left_time), (right, right_score, right_time)| {
                right_score
                    .total_cmp(left_score)
                    .then_with(|| right_time.cmp(left_time))
                    .then_with(|| left.id.cmp(&right.id))
            },
        );
        if ranked.len() > config.all_candidates_max as usize {
            ranked.truncate(config.candidate_k as usize);
        }
        ranked.into_iter().map(|(topic, _, _)| topic).collect()
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

fn remove_cursor(topics: &mut TopicCursors, topic: &TopicId, cursor: Cursor) {
    if let Some(cursors) = topics.get_mut(topic) {
        cursors.remove(&cursor);
        if cursors.is_empty() {
            topics.remove(topic);
        }
    }
}

struct BurstWindow<'a> {
    start: &'a StoredMessage,
    last_at: DateTime<FixedOffset>,
}

fn burst_window(burst: &[StoredMessage]) -> Option<BurstWindow<'_>> {
    let mut messages = burst.iter().filter(|m| !m.message.recalled);
    let first = messages.next()?;
    let mut window = BurstWindow {
        start: first,
        last_at: first.message.sent_at,
    };
    for message in messages {
        if message.cursor < window.start.cursor {
            window.start = message;
        }
        window.last_at = window.last_at.max(message.message.sent_at);
    }
    Some(window)
}

fn usable_identity(identity: &str) -> bool {
    !identity.is_empty() && !matches!(identity, "unknown" | "system")
}

#[cfg(test)]
mod tests;
