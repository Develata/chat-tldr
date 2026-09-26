use super::*;
use crate::store::{ImportOptions, feedback, import_batches, resolve};
use chrono::Duration;

struct Fixture {
    _temp: tempfile::TempDir,
    path: PathBuf,
    batch: ImportBatch,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lifecycle.db");
        let batch = chat_tldr_qce::parse_qce_json(
            include_bytes!("../../../../../../fixtures/qce/synthetic-group.json"),
            &chat_tldr_qce::QceOptions::default(),
        )
        .unwrap();
        import_batches(
            &path,
            std::slice::from_ref(&batch),
            &ImportOptions::default(),
        )
        .unwrap();
        Self {
            _temp: temp,
            path,
            batch,
        }
    }

    fn session(&self, run: &str) -> AnalysisSession {
        AnalysisSession::begin(
            &self.path,
            &self.batch.chat.chat_id,
            &run.into(),
            &serde_json::json!({}),
        )
        .unwrap()
    }

    fn topic(&self, session: &AnalysisSession, id: &str) -> TopicRecord {
        let topic = TopicRecord {
            id: id.into(),
            chat_id: self.batch.chat.chat_id.clone(),
            title: "Synthetic topic".into(),
            provisional: false,
            state: TopicState::Active,
            last_message_at: self.batch.messages[1].sent_at,
            is_chitchat: Some(0.2),
        };
        session.assign(&topic, &[], "direct").unwrap();
        topic
    }

    fn item(&self, session: &AnalysisSession, topic: &TopicRecord) -> Insight {
        let message = &self.batch.messages[0];
        Insight {
            id: "i_lifecycle".into(),
            chat_id: topic.chat_id.clone(),
            kind: InsightKind::Todo,
            title: "Synthetic action".into(),
            summary: "Local test only".into(),
            priority: Priority::P0,
            rank_score: 0.5,
            confidence: Some(0.9),
            assignee: Assignee::Other,
            deadline: None,
            evidence: vec![Evidence {
                message_id: message.id.clone(),
                quote: crate::render::render(message),
                render_profile: RenderProfile::default(),
            }],
            topic_id: Some(topic.id.clone()),
            verification_status: VerificationStatus::Verified,
            lifecycle: Lifecycle::Open,
            created_in_run: session.run_id.clone(),
            updated_at: message.sent_at,
        }
    }

    fn connection(&self) -> Connection {
        open_write(&self.path).unwrap()
    }

    fn rows(&self, table: &str) -> Vec<Vec<String>> {
        let connection = self.connection();
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|column| row.get_ref(column).map(|value| format!("{value:?}")))
                    .collect()
            })
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    }

    fn state(&self) -> Vec<Vec<Vec<String>>> {
        [
            "topics",
            "topic_messages",
            "messages",
            "chats",
            "insights",
            "evidence",
            "feedback",
            "preference_weights",
            "model_cache",
            "meta",
            "run_checkpoints",
        ]
        .into_iter()
        .map(|table| self.rows(table))
        .collect()
    }
}

#[test]
fn close_requires_strict_message_time_expiry_and_handles_offsets_and_huge_ttls() {
    let ttl = 6 * 60 * 60;
    for (elapsed, configured_ttl, closes) in [
        (Duration::seconds(-1), ttl, false),
        (Duration::zero(), 0, false),
        (Duration::nanoseconds(1), 0, true),
        (Duration::seconds(ttl as i64), ttl, false),
        (
            Duration::seconds(ttl as i64) + Duration::milliseconds(1),
            ttl,
            true,
        ),
        (Duration::days(3650), u64::MAX, false),
    ] {
        let fixture = Fixture::new();
        let session = fixture.session("r_close");
        let topic = fixture.topic(&session, "t_a");
        let at =
            (topic.last_message_at + elapsed).with_timezone(&FixedOffset::west_opt(10800).unwrap());
        let before = fixture.state();
        let closed = session
            .close_topics(std::slice::from_ref(&topic.id), at, configured_ttl)
            .unwrap();
        assert_eq!(closed.len(), usize::from(closes));
        if closes {
            assert_eq!(closed[0].state, TopicState::Closed);
            assert_eq!(closed[0].last_message_at, topic.last_message_at);
        } else {
            assert_eq!(fixture.state(), before);
        }
    }
}

#[test]
fn close_rechecks_current_activity_and_returns_the_latest_title() {
    let fixture = Fixture::new();
    let session = fixture.session("r_close");
    let old = fixture.topic(&session, "t_a");
    let mut current = old.clone();
    current.title = "Latest persisted title".into();
    current.last_message_at += Duration::hours(24);
    session.assign(&current, &[], "direct").unwrap();
    let before = fixture.state();
    assert!(
        session
            .close_topics(
                std::slice::from_ref(&old.id),
                old.last_message_at + Duration::hours(7),
                21600
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.state(), before);
    let closed = session
        .close_topics(
            std::slice::from_ref(&old.id),
            current.last_message_at + Duration::hours(7),
            21600,
        )
        .unwrap();
    assert_eq!(closed[0].title, current.title);
    assert_eq!(closed[0].last_message_at, current.last_message_at);
}

#[test]
fn duplicate_already_closed_and_merged_candidates_are_idempotent() {
    let fixture = Fixture::new();
    let session = fixture.session("r_close");
    let topic = fixture.topic(&session, "t_a");
    let merged = fixture.topic(&session, "t_merged");
    fixture
        .connection()
        .execute(
            "UPDATE topics SET state='merged',merged_into='t_a' WHERE topic_id='t_merged'",
            [],
        )
        .unwrap();
    let ids = [topic.id.clone(), merged.id, topic.id.clone()];
    let closed = session
        .close_topics(&ids, topic.last_message_at + Duration::hours(7), 21600)
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(fixture.rows("run_checkpoints").len(), 1);
    let before = fixture.state();
    assert!(
        session
            .close_topics(&ids, topic.last_message_at + Duration::days(7), 21600)
            .unwrap()
            .is_empty()
    );
    assert!(
        session
            .close_topics(&[], topic.last_message_at, 0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.state(), before);
}

#[test]
fn invalid_later_candidate_rolls_back_earlier_closures() {
    for foreign in [false, true] {
        let fixture = Fixture::new();
        let session = fixture.session("r_close");
        let topic = fixture.topic(&session, "t_a");
        let invalid: TopicId = "t_z_invalid".into();
        if foreign {
            let mut other = topic.clone();
            other.id = invalid.clone();
            other.chat_id = "qq:group:other".into();
            upsert_topic(&fixture.connection(), &other, &session.run_id).unwrap();
        }
        let before = fixture.state();
        assert!(
            session
                .close_topics(
                    &[topic.id, invalid],
                    topic.last_message_at + Duration::hours(7),
                    21600
                )
                .is_err()
        );
        assert_eq!(fixture.state(), before);
    }
}

#[test]
fn close_changes_no_messages_insights_user_choices_or_cached_work() {
    let fixture = Fixture::new();
    let session = fixture.session("r_close");
    let topic = fixture.topic(&session, "t_a");
    let ids: Vec<_> = fixture
        .batch
        .messages
        .iter()
        .take(2)
        .map(|m| m.id.clone())
        .collect();
    session.assign(&topic, &ids, "direct").unwrap();
    session
        .commit_topic(
            &topic,
            &ids[..1],
            vec![(fixture.item(&session, &topic), 0.5)],
        )
        .unwrap();
    session.fail_messages(&ids[1..]).unwrap();
    resolve(&fixture.path, &"i_lifecycle".into(), Lifecycle::Dismissed).unwrap();
    feedback(&fixture.path, &"i_lifecycle".into(), false).unwrap();
    session
        .cache_put(
            "synthetic-draft",
            "pending-extraction",
            "mock",
            "mock",
            &serde_json::json!({"title":"cached draft"}),
        )
        .unwrap();
    let preserved = [
        "topic_messages",
        "messages",
        "chats",
        "insights",
        "evidence",
        "feedback",
        "preference_weights",
        "model_cache",
    ];
    let before: Vec<_> = preserved.iter().map(|table| fixture.rows(table)).collect();
    assert_eq!(
        session
            .close_topics(
                std::slice::from_ref(&topic.id),
                topic.last_message_at + Duration::hours(7),
                21600
            )
            .unwrap()
            .len(),
        1
    );
    for (table, expected) in preserved.into_iter().zip(before) {
        assert_eq!(fixture.rows(table), expected, "{table} must not change");
    }
}

#[test]
fn stale_active_draft_can_resume_after_close_without_reopening_or_regressing_activity() {
    let fixture = Fixture::new();
    let mut session = fixture.session("r_close");
    let mut stale = fixture.topic(&session, "t_a");
    let first = &fixture.batch.messages[0];
    session
        .assign(&stale, std::slice::from_ref(&first.id), "direct")
        .unwrap();
    session
        .cache_put(
            "synthetic-draft",
            "pending-extraction",
            "mock",
            "mock",
            &serde_json::json!({"title":"cached draft"}),
        )
        .unwrap();
    let latest = stale.last_message_at;
    session
        .close_topics(
            std::slice::from_ref(&stale.id),
            latest + Duration::hours(7),
            21600,
        )
        .unwrap();
    session.finish(RunStatus::Cancelled).unwrap();
    drop(session);
    let resumed = fixture.session("r_resumed");
    assert!(
        resumed
            .cache_get::<serde_json::Value>("synthetic-draft")
            .unwrap()
            .is_some()
    );
    stale.last_message_at = first.sent_at;
    resumed
        .assign(&stale, std::slice::from_ref(&first.id), "rule_reply")
        .unwrap();
    resumed
        .commit_topic(
            &stale,
            std::slice::from_ref(&first.id),
            vec![(fixture.item(&resumed, &stale), 0.5)],
        )
        .unwrap();
    let snapshot = resumed.snapshot().unwrap();
    assert_eq!(snapshot.topics[0].state, TopicState::Closed);
    assert_eq!(snapshot.topics[0].last_message_at, latest);
    assert_eq!(
        snapshot
            .messages
            .iter()
            .find(|m| m.message.id == first.id)
            .unwrap()
            .analysis_state,
        "done"
    );
    assert_eq!(
        snapshot.insights[0].verification_status,
        VerificationStatus::Verified
    );
}

#[test]
fn ordinary_upserts_cannot_close_active_topics_or_reanimate_merged_topics() {
    let fixture = Fixture::new();
    let session = fixture.session("r_close");
    let mut stale = fixture.topic(&session, "t_a");
    stale.state = TopicState::Closed;
    session.assign(&stale, &[], "direct").unwrap();
    session.commit_topic(&stale, &[], vec![]).unwrap();
    assert_eq!(
        session.snapshot().unwrap().topics[0].state,
        TopicState::Active
    );
    fixture
        .connection()
        .execute(
            "UPDATE topics SET state='merged',merged_into='t_other' WHERE topic_id='t_a'",
            [],
        )
        .unwrap();
    stale.state = TopicState::Active;
    let before = fixture.state();
    assert!(session.assign(&stale, &[], "direct").is_err());
    assert!(session.commit_topic(&stale, &[], vec![]).is_err());
    assert_eq!(fixture.state(), before);
}

#[test]
fn first_closing_boundary_survives_backfill_repeated_close_and_session_restart() {
    let fixture = Fixture::new();
    let mut session = fixture.session("r_close");
    let topic = fixture.topic(&session, "t_a");
    let boundary = topic.last_message_at + Duration::hours(7);
    session
        .close_topics(std::slice::from_ref(&topic.id), boundary, 21600)
        .unwrap();
    assert_eq!(
        session.snapshot().unwrap().closed_at.get(&topic.id),
        Some(&boundary)
    );
    let mut backfill = topic.clone();
    backfill.last_message_at += Duration::hours(6);
    // Caller still has Active metadata, but neither ordinary write can reopen
    // the topic or extend its original message-time closing boundary.
    session.assign(&backfill, &[], "rule_reply").unwrap();
    session.commit_topic(&backfill, &[], vec![]).unwrap();
    assert!(
        session
            .close_topics(
                std::slice::from_ref(&topic.id),
                boundary + Duration::hours(20),
                21600
            )
            .unwrap()
            .is_empty()
    );
    let snapshot = session.snapshot().unwrap();
    assert_eq!(snapshot.topics[0].last_message_at, backfill.last_message_at);
    assert_eq!(snapshot.topics[0].state, TopicState::Closed);
    assert_eq!(
        snapshot.closed_at,
        BTreeMap::from([(topic.id.clone(), boundary)])
    );
    session.finish(RunStatus::Complete).unwrap();
    drop(session);
    assert_eq!(
        fixture.session("r_next").snapshot().unwrap().closed_at,
        BTreeMap::from([(topic.id, boundary)])
    );
}

#[test]
fn closing_snapshot_is_scoped_and_does_not_guess_legacy_boundaries() {
    let fixture = Fixture::new();
    let session = fixture.session("r_close");
    let legacy = fixture.topic(&session, "t_legacy");
    let active = fixture.topic(&session, "t_active");
    let mut foreign = fixture.topic(&session, "t_foreign");
    foreign.chat_id = "qq:group:other".into();
    let connection = fixture.connection();
    connection
        .execute(
            "UPDATE topics SET state='closed' WHERE topic_id='t_legacy'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE topics SET chat_id=?1,state='closed' WHERE topic_id='t_foreign'",
            [foreign.chat_id.as_ref()],
        )
        .unwrap();
    for id in [&active.id, &foreign.id] {
        connection
            .execute(
                "INSERT INTO meta(key,value) VALUES(?1,?2)",
                params![
                    format!("{CLOSED_AT_PREFIX}{id}"),
                    serde_json::to_string(&legacy.last_message_at).unwrap()
                ],
            )
            .unwrap();
    }
    let before = fixture.state();
    assert!(session.snapshot().unwrap().closed_at.is_empty());
    assert_eq!(
        fixture.state(),
        before,
        "read-only snapshot must not backfill legacy data"
    );
    let plans: Vec<String> = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {CLOSED_AT_SQL}"))
        .unwrap()
        .query_map(params![CLOSED_AT_PREFIX, legacy.chat_id.as_ref()], |row| {
            row.get(3)
        })
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert!(
        plans
            .iter()
            .any(|plan| plan.contains("SEARCH m USING INDEX") && plan.contains("(key=?)")),
        "closing metadata must use exact indexed keys: {plans:?}"
    );
    assert!(
        !plans.iter().any(|plan| plan.contains("SCAN m")),
        "must not scan unrelated metadata: {plans:?}"
    );
}

#[test]
fn closing_boundary_write_and_topic_state_roll_back_when_checkpoint_fails() {
    let fixture = Fixture::new();
    let session = fixture.session("r_close");
    let topic = fixture.topic(&session, "t_a");
    fixture.connection().execute_batch("CREATE TRIGGER fail_close_checkpoint BEFORE INSERT ON run_checkpoints WHEN NEW.stage='close' BEGIN SELECT RAISE(ABORT,'synthetic close failure'); END;").unwrap();
    let before = fixture.state();
    assert!(
        session
            .close_topics(
                std::slice::from_ref(&topic.id),
                topic.last_message_at + Duration::hours(7),
                21600
            )
            .is_err()
    );
    assert_eq!(fixture.state(), before);
    assert!(session.snapshot().unwrap().closed_at.is_empty());
}
