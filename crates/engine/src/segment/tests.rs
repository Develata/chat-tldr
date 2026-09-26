use super::*;
use chat_tldr_core::{Cursor, ReplyRef, UnifiedMessage};
use serde_json::json;

fn messages(senders: &[&str]) -> Vec<StoredMessage> {
    senders
        .iter()
        .enumerate()
        .map(|(index, sender)| {
            let message: UnifiedMessage = serde_json::from_value(json!({
                "id": format!("m{index}"), "chat_id": "test", "sender": sender,
                "sender_display": sender, "sent_at": "2026-09-26T12:00:00+00:00",
                "text": "synthetic", "mentions": [], "reply_to": null,
                "attachments": [], "forward": null, "recalled": false, "system": false,
                "source": {"format":"synthetic", "identity":format!("{index}"),
                    "qce_id":null, "qce_seq":null, "qce_type":null, "file_hash":"synthetic"}
            }))
            .unwrap();
            StoredMessage {
                cursor: Cursor {
                    sent_at_ms: message.sent_at.timestamp_millis(),
                    ordinal: index as i64,
                },
                message,
                analysis_state: "pending".into(),
                topic_id: None,
            }
        })
        .collect()
}

fn reply(messages: &mut [StoredMessage], source: usize, target: &str) {
    messages[source].message.reply_to = Some(ReplyRef {
        source_message_id: target.into(),
        resolved: Some(target.into()),
    });
}

#[test]
fn interleave_counts_other_participants_even_inside_one_temporal_burst() {
    let mut input = messages(&["a", "a", "c", "d", "e", "b", "b"]);
    reply(&mut input, 6, "m0");
    assert_eq!(
        bursts(&input, &crate::Config::parse("").unwrap().segment).len(),
        1
    );
    assert_eq!(interleave(&input), 1.0);
    // Removing one of the three intervening third-party messages crosses the threshold.
    input[3].message.recalled = true;
    assert_eq!(interleave(&input), 0.0);
}

#[test]
fn interleave_does_not_confuse_time_boundaries_or_external_replies_with_other_senders() {
    let mut input = messages(&["a", "a", "b", "a", "b"]);
    input[4].message.sent_at += chrono::Duration::hours(1);
    reply(&mut input, 4, "m0");
    reply(&mut input, 3, "outside-window");
    assert_eq!(interleave(&input), 0.0);
    assert_eq!(
        bursts(&input, &crate::Config::parse("").unwrap().segment).len(),
        2
    );
}

#[test]
fn interleave_denominator_uses_only_earlier_visible_in_window_targets() {
    let mut input = messages(&["a", "c", "d", "e", "a", "a", "a", "a"]);
    reply(&mut input, 4, "m0"); // three other people
    reply(&mut input, 5, "m4"); // adjacent
    reply(&mut input, 6, "outside-window");
    reply(&mut input, 7, "m7"); // self edge is not an earlier target
    assert_eq!(interleave(&input), 0.5);
    input[0].message.recalled = true;
    assert_eq!(interleave(&input), 0.0);
    assert_eq!(interleave(&[]), 0.0);
}

#[test]
fn interleave_long_same_sender_chain_does_not_scan_each_reply_interval() {
    let senders = vec!["a"; 20_000];
    let mut input = messages(&senders);
    for index in 1..input.len() {
        reply(&mut input, index, "m0");
    }
    assert_eq!(interleave(&input), 0.0);
}

#[test]
fn interleave_suffix_removes_edges_when_the_target_leaves_the_window() {
    let mut input = messages(&["a", "c", "d", "e", "b", "c", "d", "e", "a", "b"]);
    reply(&mut input, 4, "m0");
    reply(&mut input, 8, "m4");
    reply(&mut input, 9, "m8");
    let indexed = ReplyInterleave::new(&input);
    assert_eq!(indexed.remaining(0), 2.0 / 3.0);
    assert_eq!(indexed.remaining(1), 0.5);
    assert_eq!(indexed.remaining(4), 0.5);
    assert_eq!(indexed.remaining(5), 0.0);
    assert_eq!(indexed.remaining(input.len()), 0.0);
    assert_eq!(indexed.remaining(usize::MAX), 0.0);
    assert_eq!(ReplyInterleave::new(&[]).remaining(0), 0.0);
}

/// Intentionally scan each reply interval directly, independently of the indexed
/// implementation, to check all suffix boundaries on small synthetic snapshots.
fn naive_interleave(window: &[StoredMessage]) -> f32 {
    let mut total = 0;
    let mut cross = 0;
    for (source_index, source) in window.iter().enumerate() {
        if source.message.recalled {
            continue;
        }
        let Some(target_id) = source
            .message
            .reply_to
            .as_ref()
            .and_then(|r| r.resolved.as_ref())
        else {
            continue;
        };
        let Some(target_index) = window[..source_index]
            .iter()
            .position(|target| !target.message.recalled && &target.message.id == target_id)
        else {
            continue;
        };
        total += 1;
        let other_messages = window[target_index + 1..source_index]
            .iter()
            .filter(|message| {
                !message.message.recalled
                    && message.message.sender != source.message.sender
                    && message.message.sender != window[target_index].message.sender
            })
            .count();
        cross += usize::from(other_messages >= 3);
    }
    if total == 0 {
        0.0
    } else {
        cross as f32 / total as f32
    }
}

#[test]
fn every_indexed_suffix_matches_naive_and_independently_built_windows() {
    let names = ["a", "b", "c", "d", "e"];
    for seed in 0..64 {
        let senders: Vec<_> = (0..16)
            .map(|index| names[(index * (seed % 7 + 1) + seed) % names.len()])
            .collect();
        let mut input = messages(&senders);
        for index in 0..input.len() {
            input[index].message.recalled = (index * 3 + seed) % 17 == 0;
            match (index + seed) % 5 {
                0 => {}
                1 => reply(&mut input, index, "outside-window"),
                2 => {
                    reply(&mut input, index, "unresolved");
                    input[index].message.reply_to.as_mut().unwrap().resolved = None;
                }
                _ => {
                    let target = (index * 3 + seed) % input.len();
                    reply(&mut input, index, &format!("m{target}"));
                }
            }
        }
        let indexed = ReplyInterleave::new(&input);
        for prefix in 0..=input.len() {
            let window = &input[prefix..];
            let expected = naive_interleave(window);
            assert_eq!(
                indexed.remaining(prefix),
                expected,
                "seed={seed}, prefix={prefix}"
            );
            assert_eq!(
                interleave(window),
                expected,
                "fresh window seed={seed}, prefix={prefix}"
            );
        }
    }
}

#[test]
fn burst_boundaries_preserve_reply_and_inclusive_same_sender_rules() {
    let mut config = crate::Config::parse("").unwrap().segment;
    config.weak_gap_secs = 30;
    config.same_sender_join_secs = 60;
    let mut input = messages(&["a", "a"]);
    input[1].message.sent_at += chrono::Duration::seconds(60);
    assert_eq!(bursts(&input, &config).len(), 1);
    input[1].message.sent_at += chrono::Duration::milliseconds(1);
    assert_eq!(bursts(&input, &config).len(), 2);
    reply(&mut input, 1, "m0");
    assert_eq!(bursts(&input, &config).len(), 1);
    input[1].message.sent_at =
        input[0].message.sent_at + chrono::Duration::seconds(config.strong_gap_secs as i64);
    assert_eq!(bursts(&input, &config).len(), 2);
    config.strong_gap_secs = u64::MAX;
    config.weak_gap_secs = u64::MAX;
    assert_eq!(bursts(&input, &config).len(), 1);
    config.burst_max_messages = 1;
    assert_eq!(bursts(&input, &config).len(), 2);
}
