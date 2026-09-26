use chat_tldr_core::*;
use serde_json::{Value, json};

fn event(body: EventBody, seq: u64) -> CliEvent {
    CliEvent::new("r_synthetic".into(), seq, body)
}

fn done(seq: u64) -> CliEvent {
    event(
        EventBody::Done(DonePayload {
            status: RunStatus::Complete,
            exit_code: 0,
            finish_reason: None,
            elapsed_ms: 1,
        }),
        seq,
    )
}

#[test]
fn scalar_wire_types_are_strings_and_sort_by_time_then_ordinal() {
    let cursor: Cursor = "1790416800000:2".parse().unwrap();
    assert_eq!(
        serde_json::to_value(cursor).unwrap(),
        json!("1790416800000:2")
    );
    assert_eq!(
        serde_json::from_value::<Cursor>(json!(cursor.to_string())).unwrap(),
        cursor
    );
    assert!(
        cursor
            < Cursor {
                ordinal: 3,
                ..cursor
            }
    );
    assert_eq!(
        serde_json::to_value(RenderProfile::default()).unwrap(),
        json!("r1")
    );
    assert_eq!(
        serde_json::to_value(ChatId::from("qq:group:synthetic")).unwrap(),
        json!("qq:group:synthetic")
    );
    for invalid in ["", "1", "1:2:3", "1:-1", "1:x", " 1:2", "01:2"] {
        assert!(invalid.parse::<Cursor>().is_err(), "{invalid}");
    }
    for invalid in ["1", "r0", "r-1", "r65536", "r01"] {
        assert!(invalid.parse::<RenderProfile>().is_err(), "{invalid}");
    }
    assert!("-1:1".parse::<Cursor>().is_ok());
}

#[test]
fn unknown_events_are_preserved_but_known_malformed_events_fail() {
    let future = json!({
        "schema_version": "1.9", "run_id": "r_synthetic", "seq": 0,
        "event": "future_event", "payload": {"future_field": [1, 2, 3]},
        "new_optional_envelope_field": true
    });
    let parsed: CliEvent = serde_json::from_value(future).unwrap();
    assert!(matches!(&parsed.body, EventBody::Unknown { event, .. } if event == "future_event"));
    assert_eq!(
        serde_json::to_value(&parsed).unwrap()["payload"]["future_field"],
        json!([1, 2, 3])
    );
    let mut validator = EventStreamValidator::new();
    validator.accept(&parsed).unwrap();
    validator.accept(&done(1)).unwrap();
    validator.finish(0).unwrap();

    for (name, payload) in [
        ("done", json!({"status": "complete", "elapsed_ms": 1})),
        ("error", json!({"code": "E_CONFIG"})),
        ("chat", json!({"chat_id": "qq:group:synthetic"})),
        ("future_event", json!([])),
    ] {
        let malformed = json!({"schema_version":"1.0", "run_id":"r_synthetic", "seq":0,
            "event": name, "payload": payload});
        assert!(
            serde_json::from_value::<CliEvent>(malformed).is_err(),
            "{name}"
        );
    }
}

#[test]
fn minor_version_enum_additions_and_unknown_fields_are_tolerated() {
    assert_eq!(
        serde_json::from_value::<InsightKind>(json!("future_kind")).unwrap(),
        InsightKind::Unknown
    );
    assert_eq!(
        serde_json::from_value::<Priority>(json!("P4")).unwrap(),
        Priority::Unknown
    );
    assert_eq!(
        serde_json::from_value::<MentionTarget>(json!({"type":"future_target","x":1})).unwrap(),
        MentionTarget::Unknown
    );
    assert_eq!(
        serde_json::from_value::<AgentAction>(json!({"action":"future_action","x":1})).unwrap(),
        AgentAction::Unknown
    );
    let mut known = serde_json::to_value(done(0)).unwrap();
    known["payload"]["future_optional"] = json!(true);
    assert_eq!(serde_json::from_value::<CliEvent>(known).unwrap(), done(0));
}

#[test]
fn framing_rejects_gaps_changed_runs_truncation_and_trailing_events() {
    let mut validator = EventStreamValidator::new();
    assert_eq!(
        validator.accept(&done(1)),
        Err(ProtocolError::Sequence {
            expected: 0,
            actual: 1
        })
    );
    assert_eq!(validator.finish(0), Err(ProtocolError::MissingDone));

    let mut unknown = event(
        EventBody::Unknown {
            event: "future".into(),
            payload: json!({}),
        },
        0,
    );
    validator.accept(&unknown).unwrap();
    unknown.seq = 1;
    unknown.run_id = "different".into();
    assert_eq!(validator.accept(&unknown), Err(ProtocolError::RunChanged));
    validator.accept(&done(1)).unwrap();
    assert_eq!(validator.accept(&done(2)), Err(ProtocolError::AfterDone));
    assert_eq!(
        validator.finish(6),
        Err(ProtocolError::ExitCode {
            expected: 0,
            actual: 6
        })
    );
    validator.finish(0).unwrap();

    let mut incompatible = done(0);
    incompatible.schema_version = "2.0".into();
    assert!(matches!(
        EventStreamValidator::new().accept(&incompatible),
        Err(ProtocolError::SchemaVersion(_))
    ));
}

#[test]
fn synthetic_client_fixtures_parse_roundtrip_and_finish() {
    let fixtures = [
        (include_str!("../../../fixtures/jsonl/version.jsonl"), 0),
        (include_str!("../../../fixtures/jsonl/chats.jsonl"), 0),
        (include_str!("../../../fixtures/jsonl/inbox-empty.jsonl"), 0),
        (include_str!("../../../fixtures/jsonl/inbox.jsonl"), 0),
        (
            include_str!("../../../fixtures/jsonl/analyze-complete.jsonl"),
            0,
        ),
        (
            include_str!("../../../fixtures/jsonl/analyze-partial.jsonl"),
            6,
        ),
    ];
    for (fixture, exit_code) in fixtures {
        let mut validator = EventStreamValidator::new();
        for line in fixture.lines() {
            let parsed: CliEvent = serde_json::from_str(line).unwrap();
            validator.accept(&parsed).unwrap();
            let wire = serde_json::to_value(&parsed).unwrap();
            assert_eq!(serde_json::from_value::<CliEvent>(wire).unwrap(), parsed);
            if let EventBody::Insight(payload) = parsed.body {
                assert_eq!(payload.insight.assignee, Assignee::Other);
                assert_eq!(payload.insight.priority, Priority::P0);
                assert!(payload.insight.deadline.is_some());
                for view in payload.evidence_view {
                    let [start, end] = view.highlight.unwrap();
                    let highlighted: String = view
                        .display_text
                        .chars()
                        .skip(start)
                        .take(end - start)
                        .collect();
                    assert_eq!(highlighted, payload.insight.evidence[0].quote);
                }
            }
        }
        validator.finish(exit_code).unwrap();
    }
}

#[test]
fn known_done_status_must_match_exit_code_and_unknown_stays_unknown() {
    for (status, exit_code) in [
        (RunStatus::Complete, 4),
        (RunStatus::Partial, 0),
        (RunStatus::Cancelled, 6),
        (RunStatus::Failed, 0),
        (RunStatus::Failed, 6),
        (RunStatus::Failed, 130),
    ] {
        let ending = event(
            EventBody::Done(DonePayload {
                status,
                exit_code,
                finish_reason: None,
                elapsed_ms: 0,
            }),
            0,
        );
        let mut validator = EventStreamValidator::new();
        assert_eq!(
            validator.accept(&ending),
            Err(ProtocolError::DoneStatus { status, exit_code })
        );
        assert_eq!(validator.finish(exit_code), Err(ProtocolError::MissingDone));
    }
    for (status, exit_code) in [
        (RunStatus::Complete, 0),
        (RunStatus::Partial, 6),
        (RunStatus::Cancelled, 130),
        (RunStatus::Failed, 4),
        (RunStatus::Unknown, 9),
    ] {
        let ending = event(
            EventBody::Done(DonePayload {
                status,
                exit_code,
                finish_reason: None,
                elapsed_ms: 0,
            }),
            0,
        );
        let mut validator = EventStreamValidator::new();
        validator.accept(&ending).unwrap();
        validator.finish(exit_code).unwrap();
        let parsed: CliEvent =
            serde_json::from_value(serde_json::to_value(&ending).unwrap()).unwrap();
        assert_eq!(parsed, ending);
    }
}

#[test]
fn import_stats_use_the_same_object_shape_as_documented_jsonl() {
    let stats = StatsPayload::Import(ImportStats {
        files: 1,
        chats: 1,
        seen: 2,
        inserted: 2,
        ..Default::default()
    });
    let wire: Value = serde_json::to_value(stats).unwrap();
    assert_eq!(wire["scope"], "import");
    assert_eq!(wire["inserted"], 2);
    assert!(wire.get("Import").is_none());
}
