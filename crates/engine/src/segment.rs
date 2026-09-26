//! Deterministic temporal bursts and reply interleaving; no I/O or model calls.
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
            let gap = (message.message.sent_at - previous.message.sent_at).num_seconds();
            let reply = message
                .message
                .reply_to
                .as_ref()
                .and_then(|r| r.resolved.as_ref())
                .is_some_and(|id| group.iter().any(|m| &m.message.id == id));
            group.len() < config.burst_max_messages as usize
                && gap < config.strong_gap_secs as i64
                && (gap < config.weak_gap_secs as i64
                    || reply
                    || message.message.sender == previous.message.sender
                        && gap < config.same_sender_join_secs as i64)
        });
        if join {
            groups.last_mut().expect("has group").push(message.clone())
        } else {
            groups.push(vec![message.clone()]);
        }
    }
    groups
}
pub(crate) fn interleave(messages: &[StoredMessage], config: &crate::config::SegmentConfig) -> f32 {
    let membership: BTreeMap<_, _> = bursts(messages, config)
        .into_iter()
        .enumerate()
        .flat_map(|(n, group)| group.into_iter().map(move |m| (m.message.id, n)))
        .collect();
    let mut cross = 0;
    let mut edges = 0;
    for message in messages {
        if let Some(reply) = message
            .message
            .reply_to
            .as_ref()
            .and_then(|r| r.resolved.as_ref())
        {
            edges += 1;
            if membership.get(reply) != membership.get(&message.message.id) {
                cross += 1
            }
        }
    }
    if edges == 0 {
        0.0
    } else {
        cross as f32 / edges as f32
    }
}
