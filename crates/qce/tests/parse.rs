use chat_tldr_core::{AttachmentKind, MentionTarget};
use chat_tldr_qce::{QceError, QceOptions, parse_qce_json};
use chrono::FixedOffset;
use serde_json::{Value, json};

fn message(elements: Value) -> Value {
    json!({"id":"fixture-1","seq":"7","timestamp":1790400000000_i64,
        "sender":{"uid":"u_sender","name":"合成用户"},
        "content":{"text":"fallback identity text","elements":elements}})
}

fn export(messages: Value) -> Value {
    json!({"metadata":{"future":true},"statistics":{},
        "chatInfo":{"type":"group","peerUid":"fixture-group","name":"合成群"},
        "messages":messages,"unknownField":{"ignored":true}})
}

fn parse(value: Value) -> chat_tldr_core::ImportBatch {
    parse_qce_json(&serde_json::to_vec(&value).unwrap(), &QceOptions::default()).unwrap()
}

#[test]
fn shared_fixture_is_deterministic_and_keeps_reply_reference() {
    let bytes = include_bytes!("../../../fixtures/qce/synthetic-group.json");
    let first = parse_qce_json(bytes, &QceOptions::default()).unwrap();
    let second = parse_qce_json(bytes, &QceOptions::default()).unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    assert_eq!(first.messages.len(), 3);
    assert_eq!(first.chat.chat_id.as_ref(), "qq:group:synthetic-study");
    assert_eq!(first.chat.self_uid.as_deref(), Some("u_self_fixture"));
    assert!(matches!(
        first.messages[0].mentions[0].target,
        MentionTarget::All
    ));
    let reply = first.messages[1].reply_to.as_ref().unwrap();
    assert_eq!(reply.source_message_id, "synthetic-001");
    assert!(reply.resolved.is_none());
    assert!(first.messages[2].text.is_empty());
    assert!(first.messages[2].recalled);
    assert!(first.aliases.len() >= 3);
    let hash = blake3::hash(b"qq:group:synthetic-study\nqce:synthetic-001").to_hex();
    assert_eq!(first.messages[0].id.as_ref(), format!("m_{}", &hash[..16]));
}

#[test]
fn text_and_all_documented_element_placeholders_are_in_order() {
    let elements = json!([
        {"type":"text","data":{"text":"原文 😀"}},
        {"type":"face","data":{"name":"笑"}},
        {"type":"face","data":{}},
        {"type":"market_face","data":{"name":"猫"}},
        {"type":"image","data":{"filename":"demo.png","size":25}},
        {"type":"video","data":{"filename":"demo.mp4","size":50}},
        {"type":"audio","data":{"filename":"demo.ogg","size":10}},
        {"type":"file","data":{"filename":"报告.txt","size":20}},
        {"type":"system","data":{"text":"not yet supported"}},
        {"type":"json","data":{}},{"type":"location","data":{}},
        {"type":"future_shape","data":{"unknown":true}}
    ]);
    let batch = parse(export(json!([message(elements)])));
    let msg = &batch.messages[0];
    assert_eq!(
        msg.text,
        "原文 😀[表情:笑][表情][表情:猫][图片][视频][语音][文件:报告.txt][system][json][location][future_shape]"
    );
    assert_eq!(msg.attachments.len(), 4);
    assert!(matches!(msg.attachments[0].kind, AttachmentKind::Image));
    assert_eq!(msg.attachments[0].size, Some(25));
    assert_eq!(msg.attachments[3].name.as_deref(), Some("报告.txt"));
    assert_eq!(batch.warnings.len(), 4);
    assert!(
        batch
            .warnings
            .iter()
            .all(|warning| warning.starts_with("W_UNKNOWN_ELEMENT: "))
    );
}

#[test]
fn mentions_distinguish_nt_uid_numeric_uin_unknown_and_all() {
    let data = [
        json!({"uid":"u_me","uin":"12345","name":"我"}),
        json!({"uid":"12345","name":"另一个显示名"}),
        json!({"uid":"unknown","uin":"0","name":"未知"}),
        json!({"uid":"all","uin":"0","name":"全体成员"}),
    ];
    let elements: Vec<_> = data
        .into_iter()
        .map(|data| json!({"type":"at","data":data}))
        .collect();
    let batch = parse(export(json!([message(json!(elements))])));
    assert_eq!(
        batch.messages[0].mentions[0].target,
        MentionTarget::User {
            uid: Some("u_me".into()),
            uin: Some("12345".into())
        }
    );
    assert_eq!(
        batch.messages[0].mentions[1].target,
        MentionTarget::User {
            uid: None,
            uin: Some("12345".into())
        }
    );
    assert_eq!(
        batch.messages[0].mentions[2].target,
        MentionTarget::User {
            uid: None,
            uin: None
        }
    );
    assert!(matches!(
        batch.messages[0].mentions[3].target,
        MentionTarget::All
    ));
    assert!(batch.chat.self_uid.is_none());
    assert!(batch.chat.self_uin.is_none());
}

#[test]
fn reply_prefers_referenced_id_and_falls_back_to_message_id() {
    for (data, expected) in [
        (
            json!({"referencedMessageId":"preferred","messageId":"fallback"}),
            "preferred",
        ),
        (
            json!({"referencedMessageId":"","messageId":"fallback"}),
            "fallback",
        ),
    ] {
        let batch = parse(export(json!([message(
            json!([{"type":"reply","data":data}])
        )])));
        assert_eq!(
            batch.messages[0]
                .reply_to
                .as_ref()
                .unwrap()
                .source_message_id,
            expected
        );
        assert!(batch.messages[0].text.is_empty());
    }
}

#[test]
fn recalled_message_drops_all_content_even_if_export_keeps_it() {
    let mut raw = message(json!([
        {"type":"at","data":{"uid":"all"}},
        {"type":"image","data":{"filename":"secret.png"}},
        {"type":"reply","data":{"messageId":"target"}},
        {"type":"forward","data":{"title":"old contents"}}
    ]));
    raw["recalled"] = json!(true);
    let batch = parse(export(json!([raw])));
    let msg = &batch.messages[0];
    assert!(msg.text.is_empty());
    assert!(msg.mentions.is_empty());
    assert!(msg.attachments.is_empty());
    assert!(msg.reply_to.is_none());
    assert!(msg.forward.is_none());
}

#[test]
fn missing_id_uses_seq_then_hash_and_metadata_never_changes_message_identity() {
    let mut raw = message(json!([]));
    raw.as_object_mut().unwrap().remove("id");
    let with_seq = parse(export(json!([raw.clone()])));
    assert_eq!(with_seq.messages[0].source.identity, "seq:7@1790400000000");
    raw.as_object_mut().unwrap().remove("seq");
    let no_seq = parse(export(json!([raw.clone()])));
    assert!(no_seq.messages[0].source.identity.starts_with("hash:"));
    raw["sender"]["name"] = json!("changed display name");
    let renamed = parse(export(json!([raw])));
    assert_eq!(no_seq.messages[0].id, renamed.messages[0].id);
    assert_ne!(no_seq.file_hash, renamed.file_hash);
}

#[test]
fn explicit_integer_ids_remain_exact_without_float_conversion() {
    let mut raw = message(json!([]));
    raw["id"] = json!(18_000_000_000_000_000_001_u64);
    let batch = parse(export(json!([raw])));
    assert_eq!(
        batch.messages[0].source.qce_id.as_deref(),
        Some("18000000000000000001")
    );
}

#[test]
fn timestamp_sort_is_stable_and_offset_does_not_change_instant() {
    let mut first = message(json!([]));
    let mut second = first.clone();
    let mut third = first.clone();
    first["timestamp"] = json!(1790400002000_i64);
    first["id"] = json!("last");
    second["id"] = json!("equal-first");
    third["id"] = json!("equal-second");
    let bytes = serde_json::to_vec(&export(json!([first, second, third]))).unwrap();
    let opts = QceOptions {
        timezone: FixedOffset::west_opt(3 * 3600).unwrap(),
        ..QceOptions::default()
    };
    let batch = parse_qce_json(&bytes, &opts).unwrap();
    assert_eq!(
        batch
            .messages
            .iter()
            .map(|m| m.source.qce_id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["equal-first", "equal-second", "last"]
    );
    assert_eq!(batch.messages[0].sent_at.timestamp_millis(), 1790400000000);
    assert_eq!(batch.messages[0].sent_at.offset().local_minus_utc(), -10800);
}

#[test]
fn system_sender_can_be_absent_but_ordinary_sender_cannot() {
    let mut raw = message(json!([{"type":"system","data":{}}]));
    raw.as_object_mut().unwrap().remove("sender");
    let bytes = serde_json::to_vec(&export(json!([raw.clone()]))).unwrap();
    assert!(parse_qce_json(&bytes, &QceOptions::default()).is_err());
    raw["system"] = json!(true);
    assert_eq!(
        parse(export(json!([raw]))).messages[0].sender.as_ref(),
        "qq:system"
    );
}

fn nested_forward(depth: usize) -> Value {
    let elements = if depth == 0 {
        json!([{"type":"text","data":{"text":"deep text"}}])
    } else {
        json!([nested_forward(depth - 1)])
    };
    json!({"type":"forward","data":{"title":format!("level-{depth}"),"messages":[{
        "sender":{"name":"合成转发者"},"timestamp":1790400000000_i64,"content":{"elements":elements}
    }]}})
}

#[test]
fn forward_expansion_stops_after_two_levels_but_retains_next_title() {
    let batch = parse(export(json!([message(json!([nested_forward(4)]))])));
    let outer = batch.messages[0].forward.as_ref().unwrap();
    assert_eq!(outer.messages.len(), 1);
    let second = outer.messages[0].forward.as_ref().unwrap();
    assert_eq!(second.messages.len(), 1);
    let third = second.messages[0].forward.as_ref().unwrap();
    assert!(third.messages.is_empty());
    assert_eq!(third.title, "level-2");
    assert!(batch.warnings.iter().any(|w| w.contains("depth limit")));
}

#[test]
fn invalid_structures_and_time_ranges_return_errors_without_panicking() {
    for bytes in [
        b"".as_slice(),
        b"null",
        b"[]",
        b"{}",
        b"{\"messages\":false}",
    ] {
        assert!(parse_qce_json(bytes, &QceOptions::default()).is_err());
    }
    for replacement in [Value::Null, json!("not milliseconds"), json!(i64::MAX)] {
        let mut raw = message(json!([]));
        raw["timestamp"] = replacement;
        assert!(
            parse_qce_json(
                &serde_json::to_vec(&export(json!([raw]))).unwrap(),
                &QceOptions::default()
            )
            .is_err()
        );
    }
    let raw = message(json!([{"type":"text","data":{"text":17}}]));
    assert!(
        parse_qce_json(
            &serde_json::to_vec(&export(json!([raw]))).unwrap(),
            &QceOptions::default()
        )
        .is_err()
    );
}

#[test]
fn private_peer_uin_fallback_and_empty_export_are_supported() {
    let mut raw = export(json!([]));
    raw["chatInfo"] = json!({"type":"private","peerUin":"12345"});
    let batch = parse(raw);
    assert_eq!(batch.chat.chat_id.as_ref(), "qq:private:12345");
    assert!(batch.messages.is_empty());
}

#[test]
fn missing_optional_fields_and_unknown_fields_are_safe() {
    let raw =
        json!({"timestamp":1790400000000_i64,"sender":{"uid":"u_sender"},"newField":"ignored"});
    let batch = parse(export(json!([raw])));
    assert!(batch.messages[0].text.is_empty());
    assert!(batch.messages[0].sender_display.is_empty());
    assert!(batch.messages[0].source.identity.starts_with("hash:"));
}

#[test]
fn missing_elements_preserves_plain_text_with_warning() {
    let batch = parse(export(json!([message(json!([]))])));
    assert_eq!(batch.messages[0].text, "fallback identity text");
    assert!(
        batch
            .warnings
            .iter()
            .any(|w| w.contains("no content.elements"))
    );
}

#[test]
fn invalid_options_and_missing_chat_identity_fail() {
    let bytes = serde_json::to_vec(&export(json!([]))).unwrap();
    let opts = QceOptions {
        max_forward_depth: 3,
        ..QceOptions::default()
    };
    assert!(parse_qce_json(&bytes, &opts).is_err());
    for chat in [
        json!({"type":"group"}),
        json!({"type":"temp","peerUid":"x"}),
    ] {
        let mut raw = export(json!([]));
        raw["chatInfo"] = chat;
        assert!(
            parse_qce_json(&serde_json::to_vec(&raw).unwrap(), &QceOptions::default()).is_err()
        );
    }
}

#[test]
fn recognized_upstream_chunked_manifest_is_an_unsupported_format_not_a_parse_error() {
    let manifest = json!({
        "metadata":{"name":"QQChatExporter","version":"0.1.0"},
        "chatInfo":{"type":"group","peerUid":"synthetic-chunked"},
        "statistics":{"totalMessages":0},
        "chunked":{
            "format":"jsonl", "chunksDir":"chunks", "chunkFileExt":".jsonl",
            "maxMessagesPerChunk":50000, "maxBytesPerChunk":52428800, "chunks":[]
        }
    });
    let error = parse_qce_json(
        &serde_json::to_vec(&manifest).unwrap(),
        &QceOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(error, QceError::UnsupportedExport));
    let mut malformed = manifest;
    malformed["chunked"]["chunks"] = json!("not an array");
    assert!(matches!(
        parse_qce_json(
            &serde_json::to_vec(&malformed).unwrap(),
            &QceOptions::default()
        ),
        Err(QceError::Json(_))
    ));
}

#[test]
fn unfamiliar_chunked_fields_do_not_reclassify_a_valid_single_file_export() {
    let mut single = export(json!([]));
    single["chunked"] =
        json!({"format":"jsonl","chunksDir":"chunks","chunkFileExt":".jsonl","chunks":[]});
    assert!(
        parse_qce_json(
            &serde_json::to_vec(&single).unwrap(),
            &QceOptions::default()
        )
        .is_ok()
    );
}
