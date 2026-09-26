//! Deterministic temporal bursts and reply interleaving; no I/O or model calls.
pub(crate) mod candidates;
pub(crate) mod merge;

use crate::store::StoredMessage;
use std::collections::BTreeMap;

pub(crate) fn bursts(
    messages: &[StoredMessage],
    config: &crate::config::SegmentConfig,
) -> Vec<Vec<StoredMessage>> {
    let mut groups: Vec<Vec<StoredMessage>> = Vec::new();
    for message in messages {
        let join = groups.last().is_some_and(|group| {
            let previous = group.last().expect("nonempty burst");
            let gap = (message.message.sent_at - previous.message.sent_at).num_milliseconds();
            let below = |seconds: u64| gap >= 0 && i128::from(gap) < i128::from(seconds) * 1000;
            let reply = message
                .message
                .reply_to
                .as_ref()
                .and_then(|r| r.resolved.as_ref())
                .is_some_and(|id| group.iter().any(|m| &m.message.id == id));
            group.len() < config.burst_max_messages as usize
                && below(config.strong_gap_secs)
                && (below(config.weak_gap_secs)
                    || reply
                    || message.message.sender == previous.message.sender
                        && gap >= 0
                        && i128::from(gap) <= i128::from(config.same_sender_join_secs) * 1000)
        });
        if join {
            groups.last_mut().expect("has group").push(message.clone())
        } else {
            groups.push(vec![message.clone()]);
        }
    }
    groups
}
/// Interleave ratios for every remaining suffix of one fixed ordered snapshot.
/// Build once in O(N log N) time and O(N) space, then query in O(1) after each
/// prefix drain. No message bodies are copied or retained by this index.
pub(crate) struct ReplyInterleave {
    total: Vec<usize>,
    cross: Vec<usize>,
}

impl ReplyInterleave {
    pub(crate) fn new(messages: &[StoredMessage]) -> Self {
        let mut positions = BTreeMap::new();
        let mut senders = BTreeMap::<_, Vec<usize>>::new();
        for (index, message) in messages.iter().enumerate() {
            if !message.message.recalled {
                positions.insert(&message.message.id, index);
                senders
                    .entry(&message.message.sender)
                    .or_default()
                    .push(index);
            }
        }
        // Recalled messages cannot contribute to the intervening conversation.
        let mut visible_prefix = Vec::with_capacity(messages.len() + 1);
        visible_prefix.push(0_usize);
        for message in messages {
            visible_prefix
                .push(visible_prefix.last().unwrap() + usize::from(!message.message.recalled));
        }
        let mut total = vec![0; messages.len() + 1];
        let mut cross = vec![0; messages.len() + 1];
        for (index, message) in messages.iter().enumerate() {
            if message.message.recalled {
                continue;
            }
            if let Some(reply) = message
                .message
                .reply_to
                .as_ref()
                .and_then(|r| r.resolved.as_ref())
                && let Some(&target) = positions.get(reply)
                && target < index
            {
                total[target] += 1;
                let count_between = |sender| {
                    let positions = &senders[sender];
                    positions.partition_point(|&p| p < index)
                        - positions.partition_point(|&p| p <= target)
                };
                let source_sender = &message.message.sender;
                let target_sender = &messages[target].message.sender;
                let mut others = visible_prefix[index]
                    - visible_prefix[target + 1]
                    - count_between(source_sender);
                if source_sender != target_sender {
                    others -= count_between(target_sender);
                }
                if others >= 3 {
                    cross[target] += 1;
                }
            }
        }
        // Every valid edge points backward: it remains inside a suffix exactly
        // when its target remains. Removing earlier messages cannot alter the
        // conversation between its target and source, so its cross flag is fixed.
        for index in (0..messages.len()).rev() {
            total[index] += total[index + 1];
            cross[index] += cross[index + 1];
        }
        Self { total, cross }
    }

    /// Fraction of remaining in-window reply edges with at least three visible
    /// intervening messages from people other than the two reply participants.
    /// Unknown/outside/future targets are excluded from both counts.
    pub(crate) fn remaining(&self, prefix_removed: usize) -> f32 {
        match self.total.get(prefix_removed) {
            Some(&total) if total > 0 => self.cross[prefix_removed] as f32 / total as f32,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
fn interleave(messages: &[StoredMessage]) -> f32 {
    ReplyInterleave::new(messages).remaining(0)
}

#[cfg(test)]
mod tests;
