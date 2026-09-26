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
        let path = temp.path().join("merge.db");
        let mut batch = chat_tldr_qce::parse_qce_json(
            include_bytes!("../../../../../../fixtures/qce/synthetic-group.json"),
            &chat_tldr_qce::QceOptions::default(),
        )
        .unwrap();
        let template = batch.messages[0].clone();
        batch.messages = (0..6)
            .map(|index| {
                let mut message = template.clone();
                message.id = format!("m_merge_{index}").into();
                message.source.identity = format!("qce:merge-{index}");
                message.source.qce_id = Some(format!("merge-{index}"));
                message.sent_at += Duration::seconds(index);
                message.reply_to = (index > 0).then(|| ReplyRef {
                    source_message_id: format!("merge-{}", index - 1),
                    resolved: Some(format!("m_merge_{}", index - 1).into()),
                });
                message
            })
            .collect();
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

    fn topic(&self, id: &str) -> TopicRecord {
        TopicRecord {
            id: id.into(),
            chat_id: self.batch.chat.chat_id.clone(),
            title: id.into(),
            provisional: false,
            state: TopicState::Active,
            last_message_at: self.batch.messages[0].sent_at,
            is_chitchat: Some(0.2),
        }
    }

    fn seed(&self, session: &AnalysisSession) -> (TopicRecord, TopicRecord) {
        let mut target = self.topic("t_a");
        let mut source = self.topic("t_b");
        target.last_message_at = self.batch.messages[4].sent_at;
        source.last_message_at = self.batch.messages[5].sent_at;
        for (parity, topic) in [&target, &source].into_iter().enumerate() {
            let ids: Vec<_> = self
                .batch
                .messages
                .iter()
                .enumerate()
                .filter(|(index, _)| index % 2 == parity)
                .map(|(_, message)| message.id.clone())
                .collect();
            session.assign(topic, &ids, "direct").unwrap();
            session.commit_topic(topic, &ids, Vec::new()).unwrap();
        }
        (target, source)
    }

    fn evidence(&self) -> Vec<MessageId> {
        self.batch.messages[..3]
            .iter()
            .map(|message| message.id.clone())
            .collect()
    }

    fn item(&self, session: &AnalysisSession, topic: &TopicRecord, id: &str) -> Insight {
        let message = &self.batch.messages[1];
        Insight {
            id: id.into(),
            chat_id: self.batch.chat.chat_id.clone(),
            kind: InsightKind::Todo,
            title: "Synthetic action".into(),
            summary: "Only local test data".into(),
            priority: Priority::P0,
            rank_score: 0.8,
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
            "insights",
            "evidence",
            "feedback",
            "preference_weights",
            "run_checkpoints",
            "meta",
            "messages",
            "chats",
        ]
        .into_iter()
        .map(|table| self.rows(table))
        .collect()
    }
}

#[test]
fn merge_preserves_insight_identity_lifecycle_evidence_votes_priority_and_cursors() {
    let fixture = Fixture::new();
    let session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    let items: Vec<_> = ["i_done", "i_dismissed", "i_open"]
        .into_iter()
        .map(|id| (fixture.item(&session, &source, id), 0.8))
        .collect();
    session.commit_topic(&source, &[], items).unwrap();
    resolve(&fixture.path, &"i_done".into(), Lifecycle::Done).unwrap();
    resolve(&fixture.path, &"i_dismissed".into(), Lifecycle::Dismissed).unwrap();
    feedback(&fixture.path, &"i_done".into(), true).unwrap();
    feedback(&fixture.path, &"i_dismissed".into(), false).unwrap();
    let before = session.snapshot().unwrap();
    let unchanged_tables = ["messages", "chats", "evidence", "feedback"];
    let unchanged: Vec<_> = unchanged_tables
        .iter()
        .map(|table| fixture.rows(table))
        .collect();
    let result = session
        .merge_topics(&target.id, &source.id, &fixture.evidence())
        .unwrap()
        .unwrap();
    assert_eq!(result.moved_insights, 3);
    assert_eq!(result.target.last_message_at, source.last_message_at);
    assert_eq!(result.source.state, TopicState::Merged);
    for (table, expected) in unchanged_tables.into_iter().zip(unchanged) {
        assert_eq!(fixture.rows(table), expected, "{table} changed");
    }
    let after = session.snapshot().unwrap();
    assert!(
        after
            .messages
            .iter()
            .all(|message| message.topic_id.as_ref() == Some(&target.id))
    );
    for (mut expected, actual) in before.insights.into_iter().zip(after.insights) {
        expected.topic_id = Some(target.id.clone());
        // Topic feedback now shares one identity, so only the personalized score
        // is permitted to change with that reference.
        expected.rank_score = actual.rank_score;
        assert_eq!(actual, expected);
    }
    let connection = fixture.connection();
    let merged_into: String = connection
        .query_row(
            "SELECT merged_into FROM topics WHERE topic_id=?1",
            [source.id.as_ref()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(merged_into, target.id.as_ref());
    let old_features: i64 = connection
        .query_row(
            "SELECT count(*) FROM preference_weights WHERE feature='topic:t_b'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let transferred_votes: i64 = connection
        .query_row(
            "SELECT n FROM preference_weights WHERE feature='topic:t_a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_features, 0);
    assert_eq!(transferred_votes, 2);
    let checkpoint: String = connection.query_row("SELECT status FROM run_checkpoints WHERE run_id='r_merge' AND topic_id='t_b' AND stage='merge'", [], |row| row.get(0)).unwrap();
    assert_eq!(checkpoint, "complete");
}

#[test]
fn merge_returns_current_titles_and_cannot_be_repeated_or_resurrected() {
    let fixture = Fixture::new();
    let session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    fixture
        .connection()
        .execute(
            "UPDATE topics SET title='current title' WHERE topic_id='t_a'",
            [],
        )
        .unwrap();
    let result = session
        .merge_topics(&target.id, &source.id, &fixture.evidence())
        .unwrap()
        .unwrap();
    assert_eq!(result.target.title, "current title");
    let state = fixture.state();
    assert!(
        session
            .merge_topics(&target.id, &source.id, &fixture.evidence())
            .is_err()
    );
    assert!(session.assign(&source, &[], "direct").is_err());
    assert!(session.commit_topic(&source, &[], vec![]).is_err());
    assert_eq!(fixture.state(), state);
}

#[test]
fn rejected_pairs_are_symmetric_durable_and_follow_a_merge() {
    let fixture = Fixture::new();
    let mut session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    let third = fixture.topic("t_c");
    session.assign(&third, &[], "direct").unwrap();
    session.reject_merge(&third.id, &source.id).unwrap();
    let rejected_state = fixture.state();
    session.reject_merge(&source.id, &third.id).unwrap();
    assert_eq!(fixture.state(), rejected_state);
    assert!(
        session
            .merge_topics(&third.id, &source.id, &fixture.evidence())
            .is_err()
    );
    session
        .merge_topics(&target.id, &source.id, &fixture.evidence())
        .unwrap()
        .unwrap();
    session.finish(RunStatus::Complete).unwrap();
    drop(session);
    let reopened = fixture.session("r_resume");
    assert_eq!(
        reopened.snapshot().unwrap().rejected_merges,
        BTreeSet::from([(target.id.clone(), third.id.clone())])
    );
    assert!(
        reopened
            .merge_topics(&target.id, &third.id, &fixture.evidence())
            .is_err()
    );
}

#[test]
fn malformed_later_insight_rolls_back_prior_insight_updates() {
    let fixture = Fixture::new();
    let session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    session
        .commit_topic(
            &source,
            &[],
            vec![
                (fixture.item(&session, &source, "i_a"), 0.8),
                (fixture.item(&session, &source, "i_z"), 0.8),
            ],
        )
        .unwrap();
    fixture
        .connection()
        .execute(
            "UPDATE insights SET body_json='malformed' WHERE insight_id='i_z'",
            [],
        )
        .unwrap();
    let state = fixture.state();
    assert!(
        session
            .merge_topics(&target.id, &source.id, &fixture.evidence())
            .is_err()
    );
    assert_eq!(fixture.state(), state);
}

#[test]
fn failure_at_checkpoint_rolls_back_all_merge_changes() {
    let fixture = Fixture::new();
    let session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    session
        .commit_topic(
            &source,
            &[],
            vec![(fixture.item(&session, &source, "i_vote"), 0.8)],
        )
        .unwrap();
    feedback(&fixture.path, &"i_vote".into(), true).unwrap();
    fixture.connection().execute_batch("CREATE TRIGGER fail_merge_checkpoint BEFORE INSERT ON run_checkpoints WHEN NEW.stage='merge' BEGIN SELECT RAISE(ABORT,'synthetic checkpoint failure'); END;").unwrap();
    let state = fixture.state();
    assert!(
        session
            .merge_topics(&target.id, &source.id, &fixture.evidence())
            .is_err()
    );
    assert_eq!(fixture.state(), state);
}

#[test]
fn invalid_pairs_and_cross_topic_insight_updates_have_no_side_effects() {
    let fixture = Fixture::new();
    let session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    session
        .commit_topic(
            &source,
            &[],
            vec![(fixture.item(&session, &source, "i_stays"), 0.8)],
        )
        .unwrap();
    let mut foreign = fixture.topic("t_foreign");
    foreign.chat_id = "qq:group:other".into();
    upsert_topic(&fixture.connection(), &foreign, &session.run_id).unwrap();
    let mut closed = fixture.topic("t_closed");
    closed.state = TopicState::Closed;
    upsert_topic(&fixture.connection(), &closed, &session.run_id).unwrap();
    let state = fixture.state();
    for (left, right) in [
        (&target.id, &target.id),
        (&target.id, &foreign.id),
        (&target.id, &closed.id),
        (&target.id, &TopicId::from("missing")),
    ] {
        assert!(
            session
                .merge_topics(left, right, &fixture.evidence())
                .is_err()
        );
        assert!(session.reject_merge(left, right).is_err());
    }
    assert!(
        session
            .commit_topic(
                &target,
                &[],
                vec![(fixture.item(&session, &target, "i_stays"), 0.8)]
            )
            .is_err()
    );
    assert_eq!(fixture.state(), state);
}

#[test]
fn recall_changed_assignment_and_changed_reply_invalidate_stale_support() {
    for update in [
        "UPDATE messages SET recalled=1 WHERE message_id='m_merge_0'",
        "UPDATE topic_messages SET topic_id='t_other' WHERE message_id='m_merge_0'",
        "UPDATE messages SET reply_resolved='m_merge_5' WHERE message_id='m_merge_1'",
        "UPDATE messages SET reply_resolved=NULL WHERE message_id='m_merge_1'",
        "UPDATE messages SET sent_at_ms=0 WHERE message_id='m_merge_1'",
    ] {
        let fixture = Fixture::new();
        let session = fixture.session("r_merge");
        let (target, source) = fixture.seed(&session);
        fixture.connection().execute(update, []).unwrap();
        let state = fixture.state();
        assert!(
            session
                .merge_topics(&target.id, &source.id, &fixture.evidence())
                .unwrap()
                .is_none(),
            "{update}"
        );
        assert_eq!(fixture.state(), state);
    }
}

#[test]
fn unfinished_messages_block_merge_even_outside_representative_evidence() {
    for status in ["pending", "failed"] {
        let fixture = Fixture::new();
        let session = fixture.session("r_merge");
        let (target, source) = fixture.seed(&session);
        fixture
            .connection()
            .execute(
                "UPDATE messages SET analysis_state=?1 WHERE message_id='m_merge_5'",
                [status],
            )
            .unwrap();
        let state = fixture.state();
        assert!(
            session
                .merge_topics(&target.id, &source.id, &fixture.evidence())
                .unwrap()
                .is_none()
        );
        assert_eq!(fixture.state(), state);
    }
}

#[test]
fn duplicate_missing_and_oversized_evidence_cannot_manufacture_reply_edges() {
    let fixture = Fixture::new();
    let session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    let state = fixture.state();
    for ids in [
        vec![],
        vec!["m_merge_0", "m_merge_1", "m_merge_1"],
        vec!["m_merge_0", "m_merge_1", "missing"],
    ] {
        let evidence: Vec<_> = ids.into_iter().map(Into::into).collect();
        assert!(
            session
                .merge_topics(&target.id, &source.id, &evidence)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        session
            .merge_topics(
                &target.id,
                &source.id,
                &vec![MessageId::from("m_merge_0"); 7]
            )
            .is_err()
    );
    assert_eq!(fixture.state(), state);
}

#[test]
fn legacy_v1_gets_merge_indexes_only_on_write_and_queries_use_them() {
    let fixture = Fixture::new();
    let old = Connection::open(&fixture.path).unwrap();
    let indexes = |connection: &Connection| -> Vec<String> {
        connection.prepare("SELECT name FROM sqlite_schema WHERE type='index' AND name IN ('idx_topic_messages_topic','idx_insights_topic') ORDER BY name")
            .unwrap().query_map([], |row| row.get(0)).unwrap().collect::<std::result::Result<_,_>>().unwrap()
    };
    assert!(indexes(&old).is_empty());
    analysis_snapshot(&fixture.path, &fixture.batch.chat.chat_id).unwrap();
    assert!(
        indexes(&old).is_empty(),
        "read-only snapshots install no indexes"
    );
    let connection = fixture.connection();
    assert_eq!(indexes(&connection).len(), 2);
    assert_eq!(super::super::super::check_version(&connection).unwrap(), 1);
    // Opening a second write connection is idempotent for existing DB v1.
    assert_eq!(indexes(&fixture.connection()), indexes(&connection));
    for (sql, index) in [
        (INVALID_ASSIGNMENT_SQL, "idx_topic_messages_topic"),
        (UNFINISHED_SQL, "idx_topic_messages_topic"),
        (MOVE_MESSAGES_SQL, "idx_topic_messages_topic"),
        (MOVED_INSIGHTS_SQL, "idx_insights_topic"),
        (MOVED_FEEDBACK_SQL, "idx_insights_topic"),
    ] {
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let values = vec!["synthetic"; statement.parameter_count()];
        let plans: Vec<String> = statement
            .query_map(rusqlite::params_from_iter(values), |row| row.get(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(
            plans
                .iter()
                .any(|plan| plan.contains("SEARCH") && plan.contains(index)),
            "missing indexed search for {sql}: {plans:?}"
        );
        assert!(
            !plans.iter().any(|plan| plan.contains("SCAN t")
                || plan.contains("SCAN insights")
                || plan.contains("SCAN i ")),
            "unexpected full table scan for {sql}: {plans:?}"
        );
    }
}

#[test]
fn unvoted_source_uses_target_weights_without_global_replay_and_matches_full_replay() {
    let fixture = Fixture::new();
    let session = fixture.session("r_merge");
    let (target, source) = fixture.seed(&session);
    session
        .commit_topic(
            &target,
            &[],
            vec![(fixture.item(&session, &target, "i_target"), 0.1)],
        )
        .unwrap();
    feedback(&fixture.path, &"i_target".into(), true).unwrap();
    crate::store::decay_preferences(&fixture.path).unwrap();
    session
        .commit_topic(
            &source,
            &[],
            vec![(fixture.item(&session, &source, "i_source"), 0.1)],
        )
        .unwrap();
    let before = session.snapshot().unwrap();
    let weights = fixture.rows("preference_weights");
    // An unrelated malformed body is deliberately not read on the fast path.
    // (It will be restored before comparing the full deterministic replay.)
    let connection = fixture.connection();
    let unrelated = fixture.item(&session, &target, "i_unrelated");
    connection.execute("INSERT INTO insights(insight_id,chat_id,topic_id,kind,priority,rank_prior,verification_status,lifecycle,body_json,created_in_run,updated_at) VALUES('i_unrelated',?1,NULL,'todo','P0',0.1,'verified','open','malformed','r_merge',?2)", params![target.chat_id.as_ref(), unrelated.updated_at.to_rfc3339()]).unwrap();
    session
        .merge_topics(&target.id, &source.id, &fixture.evidence())
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.rows("preference_weights"),
        weights,
        "unmoved feedback must not be replayed"
    );
    connection
        .execute("DELETE FROM insights WHERE insight_id='i_unrelated'", [])
        .unwrap();
    let optimized = session.snapshot().unwrap();
    let old_source = before
        .insights
        .iter()
        .find(|item| item.id.as_ref() == "i_source")
        .unwrap();
    let new_source = optimized
        .insights
        .iter()
        .find(|item| item.id.as_ref() == "i_source")
        .unwrap();
    assert_ne!(
        old_source.rank_score, new_source.rank_score,
        "target topic's learned preference must apply"
    );
    inbox::rebuild_preferences(&connection, Utc::now().fixed_offset()).unwrap();
    assert_eq!(session.snapshot().unwrap().insights, optimized.insights);
}
