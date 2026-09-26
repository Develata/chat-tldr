//! Offline normalization checks for the handwritten scenario, not model-quality labels.
use std::collections::{BTreeMap, BTreeSet};

use chat_tldr_core::{AttachmentKind, ChatKind, ImportBatch, MentionTarget, UnifiedMessage};
use chat_tldr_qce::{QceOptions, parse_qce_json};
use chrono::{DateTime, FixedOffset};
use serde_json::Value;

const MAIN: &[u8] = include_bytes!("../../../fixtures/qce/scenario-analysis.json");
const BACKFILL: &[u8] = include_bytes!("../../../fixtures/qce/scenario-analysis-backfill.json");

fn source_id(number: u64) -> String {
    (9_200_000_000_000_000_000_u64 + number).to_string()
}

fn parse(bytes: &[u8]) -> ImportBatch {
    parse_qce_json(bytes, &QceOptions::default()).unwrap()
}

fn message(batch: &ImportBatch, number: u64) -> &UnifiedMessage {
    let source = source_id(number);
    batch
        .messages
        .iter()
        .find(|message| message.source.qce_id.as_deref() == Some(source.as_str()))
        .unwrap()
}

fn without_file_provenance(mut message: UnifiedMessage) -> UnifiedMessage {
    // The export file is different, while the complete logical message must
    // retain its identity, references, content, sender, and timestamp.
    message.source.file_hash.clear();
    message
}

#[test]
fn scenario_keeps_structured_self_all_other_and_plain_text_mentions_distinct() {
    let batch = parse(MAIN);
    assert_eq!(
        batch.chat.chat_id.as_ref(),
        "qq:group:synthetic-analysis-group"
    );
    assert_eq!(batch.chat.kind, ChatKind::Group);
    assert_eq!(batch.chat.self_uid.as_deref(), Some("u_scenario_self"));
    assert_eq!(batch.chat.self_uin.as_deref(), Some("290000001"));
    for number in [1, 2, 8] {
        let mentions = &message(&batch, number).mentions;
        assert_eq!(mentions.len(), 1);
        assert_eq!(
            mentions[0].target,
            MentionTarget::User {
                uid: Some("u_scenario_self".into()),
                uin: Some("290000001".into()),
            }
        );
    }
    assert_eq!(message(&batch, 3).mentions.len(), 1);
    assert_eq!(message(&batch, 3).mentions[0].target, MentionTarget::All);
    assert_eq!(
        message(&batch, 4).mentions[0].target,
        MentionTarget::User {
            uid: Some("u_scenario_wang".into()),
            uin: Some("290000003".into()),
        }
    );
    assert!(
        message(&batch, 5).mentions.is_empty(),
        "text that resembles @ must not invent a structured mention"
    );
    assert_eq!(
        message(&batch, 7).mentions[0].target,
        MentionTarget::User {
            uid: None,
            uin: Some("290000001".into()),
        }
    );
    assert_eq!(message(&batch, 8).mentions[0].display, "合成林舟旧昵称");
    assert_eq!(message(&batch, 9).mentions[0].display, "合成林舟");
    assert_eq!(
        message(&batch, 9).mentions[0].target,
        MentionTarget::User {
            uid: Some("u_scenario_same_name".into()),
            uin: Some("290000006".into()),
        }
    );
}

#[test]
fn scenario_preserves_source_reply_refs_without_crossing_the_store_resolution_boundary() {
    let batch = parse(MAIN);
    let reply = message(&batch, 10).reply_to.as_ref().unwrap();
    assert_eq!(reply.source_message_id, source_id(1));
    assert!(reply.resolved.is_none());
    assert_ne!(message(&batch, 1).id, message(&batch, 10).id);
    assert!(
        batch
            .messages
            .iter()
            .filter_map(|message| message.reply_to.as_ref())
            .all(|reply| reply.resolved.is_none()),
        "the pure adapter leaves cross-import resolution to engine::store"
    );
}

#[test]
fn scenario_recalled_payload_is_removed_and_system_identity_is_not_invented() {
    let batch = parse(MAIN);
    let raw: Value = serde_json::from_slice(MAIN).unwrap();
    assert_eq!(
        raw["messages"][62]["content"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|element| element["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["reply", "at", "text", "file"]
    );
    let recalled = message(&batch, 63);
    assert!(recalled.recalled);
    assert!(recalled.text.is_empty());
    assert!(recalled.mentions.is_empty());
    assert!(recalled.attachments.is_empty());
    assert!(recalled.reply_to.is_none());
    assert!(recalled.forward.is_none());
    assert_eq!(recalled.source.identity, format!("qce:{}", source_id(63)));
    let system = message(&batch, 62);
    assert!(system.system);
    assert!(!system.recalled);
    assert_eq!(system.sender.as_ref(), "qq:system");
    assert_eq!(system.text, "[system]");
}

#[test]
fn scenario_media_unknown_cards_and_forwarded_messages_keep_their_structural_limits() {
    let batch = parse(MAIN);
    for (number, kind, name, size, text) in [
        (
            64,
            AttachmentKind::Image,
            "synthetic-poster.png",
            128,
            "[图片]",
        ),
        (
            65,
            AttachmentKind::File,
            "synthetic-signup-template.csv",
            64,
            "附件仅是空白模板，文件名不表示群里已经安排交稿。[文件:synthetic-signup-template.csv]",
        ),
        (
            66,
            AttachmentKind::Audio,
            "synthetic-voice.ogg",
            96,
            "[语音]",
        ),
        (
            67,
            AttachmentKind::Video,
            "synthetic-demo.mp4",
            256,
            "[视频]",
        ),
    ] {
        let message = message(&batch, number);
        assert_eq!(message.text, text);
        let attachments = &message.attachments;
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].kind, kind);
        assert_eq!(attachments[0].name.as_deref(), Some(name));
        assert_eq!(attachments[0].size, Some(size));
    }
    assert_eq!(message(&batch, 68).text, "[json]");
    assert_eq!(message(&batch, 69).text, "[location]");
    assert!(message(&batch, 68).attachments.is_empty());
    assert!(message(&batch, 69).attachments.is_empty());
    let forward = message(&batch, 70).forward.as_ref().unwrap();
    assert_eq!(message(&batch, 70).text, "[合并转发:合成转发记录]");
    assert_eq!(forward.title, "合成转发记录");
    assert_eq!(forward.messages.len(), 2);
    assert_eq!(forward.messages[0].sender_display, "合成赵晴");
    assert_eq!(
        forward.messages[0].text,
        "转发内部示例：明天下午3点演练，尚未说明是否适用于本群。"
    );
    assert_eq!(forward.messages[1].sender_display, "合成苏棠");
    assert_eq!(
        forward.messages[1].text,
        "这份转发只供参考，本群安排需要另行确认。"
    );
    assert!(
        forward
            .messages
            .iter()
            .all(|message| message.forward.is_none())
    );
    assert_eq!(message(&batch, 72).text, "[表情:微笑][表情:合成鼓掌]");
    // Forwarded children stay inside their parent; the standalone row count is
    // unchanged and neither unsupported card contributes extracted text.
    assert_eq!(batch.messages.len(), 100);
    let codes: Vec<_> = batch
        .warnings
        .iter()
        .map(|warning| warning.split(':').next().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "W_UNKNOWN_ELEMENT",
            "W_UNKNOWN_ELEMENT",
            "W_UNKNOWN_ELEMENT"
        ]
    );
}

#[test]
fn all_scenario_ids_are_unique_and_parsing_is_deterministic_across_timezones() {
    let first = parse(MAIN);
    assert_eq!(first, parse(MAIN));
    assert_eq!(first.messages.len(), 100);
    assert_eq!(
        first
            .messages
            .iter()
            .map(|message| &message.id)
            .collect::<BTreeSet<_>>()
            .len(),
        100
    );
    assert_eq!(
        first
            .messages
            .iter()
            .map(|message| message.source.qce_id.clone().unwrap())
            .collect::<Vec<_>>(),
        (1..=100).map(source_id).collect::<Vec<_>>()
    );
    assert!(
        first
            .messages
            .windows(2)
            .all(|pair| pair[0].sent_at < pair[1].sent_at)
    );
    assert_eq!(
        first.messages[0].sent_at,
        DateTime::parse_from_rfc3339("2026-09-24T09:00:00+08:00").unwrap()
    );
    let shifted = parse_qce_json(
        MAIN,
        &QceOptions {
            timezone: FixedOffset::west_opt(3 * 3600).unwrap(),
            ..QceOptions::default()
        },
    )
    .unwrap();
    assert_eq!(first.chat, shifted.chat);
    assert_eq!(first.warnings, shifted.warnings);
    for (default, shifted) in first.messages.iter().zip(&shifted.messages) {
        assert_eq!(default.id, shifted.id);
        assert_eq!(default.source, shifted.source);
        assert_eq!(
            default.sent_at.timestamp_millis(),
            shifted.sent_at.timestamp_millis()
        );
        assert_eq!(shifted.sent_at.offset().local_minus_utc(), -10800);
        assert_eq!(default.text, shifted.text);
        assert_eq!(default.mentions, shifted.mentions);
        assert_eq!(default.reply_to, shifted.reply_to);
    }
}

#[test]
fn backfill_sorts_older_messages_and_preserves_duplicate_identity_and_full_normalization() {
    let main = parse(MAIN);
    let backfill = parse(BACKFILL);
    assert_eq!(backfill, parse(BACKFILL));
    assert_eq!(main.chat, backfill.chat);
    assert_eq!(backfill.messages.len(), 4);
    let raw_backfill: Value = serde_json::from_slice(BACKFILL).unwrap();
    assert_eq!(
        raw_backfill["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [101, 19, 102, 103].map(source_id)
    );
    assert_eq!(
        backfill
            .messages
            .iter()
            .map(|message| message.source.qce_id.clone().unwrap())
            .collect::<Vec<_>>(),
        [19, 101, 102, 103].map(source_id)
    );
    for (number, time) in [
        (101, "2026-09-24T12:15:00+08:00"),
        (102, "2026-09-24T12:16:00+08:00"),
        (103, "2026-09-24T12:17:00+08:00"),
    ] {
        assert_eq!(
            message(&backfill, number).sent_at,
            DateTime::parse_from_rfc3339(time).unwrap()
        );
    }
    for (number, target) in [(101, 20), (102, 101), (103, 18)] {
        let reply = message(&backfill, number).reply_to.as_ref().unwrap();
        assert_eq!(reply.source_message_id, source_id(target));
        assert!(reply.resolved.is_none());
    }
    assert_ne!(main.file_hash, backfill.file_hash);
    assert_eq!(
        without_file_provenance(message(&main, 19).clone()),
        without_file_provenance(message(&backfill, 19).clone())
    );
    let main_ids: BTreeSet<_> = main.messages.iter().map(|message| &message.id).collect();
    assert_eq!(
        backfill
            .messages
            .iter()
            .filter(|message| main_ids.contains(&message.id))
            .count(),
        1
    );
    assert_eq!(
        backfill
            .messages
            .iter()
            .filter(|message| !main_ids.contains(&message.id))
            .count(),
        3
    );

    let mut combined: Value = serde_json::from_slice(MAIN).unwrap();
    combined["messages"]
        .as_array_mut()
        .unwrap()
        .extend(raw_backfill["messages"].as_array().unwrap().iter().cloned());
    let together = parse(&serde_json::to_vec(&combined).unwrap());
    assert_eq!(
        together.messages.len(),
        104,
        "the adapter preserves rows; store performs import deduplication"
    );
    let expected: BTreeMap<_, _> = main
        .messages
        .iter()
        .chain(&backfill.messages)
        .map(|message| (message.id.clone(), without_file_provenance(message.clone())))
        .collect();
    assert_eq!(expected.len(), 103);
    assert!(
        together
            .messages
            .windows(2)
            .all(|pair| pair[0].sent_at <= pair[1].sent_at)
    );
    for message in together.messages {
        assert_eq!(message.source.file_hash, together.file_hash);
        assert_eq!(
            without_file_provenance(message.clone()),
            expected[&message.id]
        );
    }
}
