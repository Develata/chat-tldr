use super::*;
use crate::relations::identity;

struct Fixture {
    _temp: tempfile::TempDir,
    path: std::path::PathBuf,
    batch: ImportBatch,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("db.sqlite");
        let mut batch = chat_tldr_qce::parse_qce_json(
            include_bytes!("../../../../../fixtures/qce/synthetic-group.json"),
            &Default::default(),
        )
        .unwrap();
        let template = batch.messages[0].clone();
        batch.messages = [
            "地点和时间是什么？",
            "地点定在南门。",
            "最终定于周五九点南门。",
            "本次会议已取消。",
            "周四九点才对，尚未确认。",
        ]
        .iter()
        .enumerate()
        .map(|(n, text)| {
            let mut m = template.clone();
            m.id = format!("m_semantic_{n}").into();
            m.source.identity = format!("semantic:{n}");
            m.text = (*text).into();
            m.recalled = false;
            m.sent_at += chrono::Duration::minutes(n as i64);
            m
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
    fn session(&self) -> AnalysisSession {
        AnalysisSession::begin(
            &self.path,
            &self.batch.chat.chat_id,
            &"r_semantic".into(),
            &json!({}),
        )
        .unwrap()
    }
    fn topic(&self, session: &AnalysisSession, id: &str) -> TopicRecord {
        let topic = TopicRecord {
            id: id.into(),
            chat_id: self.batch.chat.chat_id.clone(),
            title: "合成关系".into(),
            provisional: false,
            state: TopicState::Closed,
            last_message_at: self.batch.messages[4].sent_at,
            is_chitchat: None,
        };
        session
            .assign(
                &topic,
                &self
                    .batch
                    .messages
                    .iter()
                    .map(|m| m.id.clone())
                    .collect::<Vec<_>>(),
                "direct",
            )
            .unwrap();
        topic
    }
    fn row(
        &self,
        topic: &TopicRecord,
        kind: RelationKind,
        a: usize,
        b: Option<usize>,
        completeness: Option<AnswerCompleteness>,
    ) -> SemanticRelation {
        let evidence = |n: usize| Evidence {
            message_id: self.batch.messages[n].id.clone(),
            quote: self.batch.messages[n].text.clone(),
            render_profile: RenderProfile::default(),
        };
        let mut row = SemanticRelation {
            id: String::new(),
            chat_id: topic.chat_id.clone(),
            topic_id: topic.id.clone(),
            kind,
            source: evidence(a),
            target: b.map(evidence),
            answer_completeness: completeness,
            verification_status: VerificationStatus::Unverified,
            created_in_run: "r_semantic".into(),
        };
        row.id = identity(&row);
        row
    }
    fn report(&self, until: Option<DateTime<FixedOffset>>) -> RelationsReport {
        relations(&self.path, &self.batch.chat.chat_id, None, until).unwrap()
    }
}

#[test]
fn temporal_answers_recall_duplicates_and_closed_topic_are_preserved() {
    let mut f = Fixture::new();
    let session = f.session();
    let topic = f.topic(&session, "t_a");
    let question = f.row(&topic, RelationKind::Question, 0, None, None);
    let partial = f.row(
        &topic,
        RelationKind::Answers,
        0,
        Some(1),
        Some(AnswerCompleteness::Partial),
    );
    let full = f.row(
        &topic,
        RelationKind::Answers,
        0,
        Some(2),
        Some(AnswerCompleteness::Full),
    );
    let rows = vec![question, full, partial]; // Intentionally not chronological.
    let ids = f
        .batch
        .messages
        .iter()
        .map(|m| m.id.clone())
        .collect::<Vec<_>>();
    session
        .commit_topic_with_relations(&topic, &ids, vec![], Some(&rows))
        .unwrap();
    session
        .commit_topic_with_relations(&topic, &ids, vec![], Some(&rows))
        .unwrap();
    assert_eq!(f.report(None).relations.len(), 3);
    assert_eq!(
        f.report(Some(f.batch.messages[1].sent_at)).questions[0].status,
        QuestionStatus::Pending
    );
    assert_eq!(
        f.report(Some(f.batch.messages[2].sent_at)).questions[0].status,
        QuestionStatus::PartiallyAnswered
    );
    assert_eq!(f.report(None).questions[0].status, QuestionStatus::Answered);
    assert_eq!(f.report(None).uncovered_messages, 0);
    assert_eq!(
        session.snapshot().unwrap().topics[0].state,
        TopicState::Closed
    );
    f.batch.messages[2].recalled = true;
    import_batches(
        &f.path,
        std::slice::from_ref(&f.batch),
        &ImportOptions::default(),
    )
    .unwrap();
    assert_eq!(
        f.report(None).questions[0].status,
        QuestionStatus::PartiallyAnswered
    );
    assert_eq!(
        f.report(None)
            .relations
            .iter()
            .filter(|r| r.verification_status == VerificationStatus::Rejected)
            .count(),
        1
    );
}

#[test]
fn changes_keep_both_ends_and_invalid_evidence_never_resolves_questions() {
    let f = Fixture::new();
    let session = f.session();
    let topic = f.topic(&session, "t_a");
    let mut unsupported = f.row(
        &topic,
        RelationKind::Answers,
        0,
        Some(1),
        Some(AnswerCompleteness::Full),
    );
    unsupported.target.as_mut().unwrap().quote = "伪造的完整答案".into();
    unsupported.id = identity(&unsupported);
    let rows = vec![
        f.row(&topic, RelationKind::Question, 0, None, None),
        unsupported,
        f.row(
            &topic,
            RelationKind::Answers,
            2,
            Some(0),
            Some(AnswerCompleteness::Full),
        ),
        f.row(&topic, RelationKind::Replaces, 1, Some(2), None),
        f.row(&topic, RelationKind::Cancels, 2, Some(3), None),
        f.row(&topic, RelationKind::Conflicts, 2, Some(4), None),
    ];
    let ids = f
        .batch
        .messages
        .iter()
        .map(|m| m.id.clone())
        .collect::<Vec<_>>();
    session
        .commit_topic_with_relations(&topic, &ids, vec![], Some(&rows))
        .unwrap();
    let report = f.report(None);
    assert_eq!(report.questions[0].status, QuestionStatus::Pending);
    assert_eq!(
        report
            .relations
            .iter()
            .filter(|r| r.verification_status == VerificationStatus::Rejected)
            .count(),
        2
    );
    assert_eq!(
        report
            .relations
            .iter()
            .find(|r| r.kind == RelationKind::Cancels)
            .unwrap()
            .source
            .quote,
        f.batch.messages[2].text
    );
}

#[test]
fn checkpoint_failure_rolls_back_relations_and_coverage() {
    let f = Fixture::new();
    let session = f.session();
    let topic = f.topic(&session, "t_a");
    let connection = super::super::analysis::open_write(&f.path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_relation_checkpoint BEFORE INSERT ON run_checkpoints BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let row = f.row(&topic, RelationKind::Question, 0, None, None);
    assert!(
        session
            .commit_topic_with_relations(
                &topic,
                &[f.batch.messages[0].id.clone()],
                vec![],
                Some(&[row])
            )
            .is_err()
    );
    assert!(f.report(None).relations.is_empty());
    assert_eq!(f.report(None).uncovered_messages, 5);
    assert_eq!(
        session.snapshot().unwrap().messages[0].analysis_state,
        "pending"
    );
}

#[test]
fn v1_reads_do_not_migrate_and_write_migration_preserves_existing_tables() {
    let f = Fixture::new();
    let session = f.session();
    let topic = f.topic(&session, "t_preserved");
    let item = Insight {
        id: "i_preserved".into(),
        chat_id: topic.chat_id.clone(),
        topic_id: Some(topic.id.clone()),
        kind: InsightKind::Todo,
        title: "保留用户状态".into(),
        summary: String::new(),
        priority: Priority::P2,
        rank_score: 0.5,
        confidence: None,
        assignee: Assignee::Other,
        deadline: None,
        evidence: vec![Evidence {
            message_id: f.batch.messages[0].id.clone(),
            quote: f.batch.messages[0].text.clone(),
            render_profile: RenderProfile::default(),
        }],
        verification_status: VerificationStatus::Unverified,
        lifecycle: Lifecycle::Open,
        created_in_run: session.run_id.clone(),
        updated_at: Utc::now().fixed_offset(),
    };
    session
        .commit_topic(
            &topic,
            &f.batch
                .messages
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
            vec![(item, 0.5)],
        )
        .unwrap();
    resolve(&f.path, &"i_preserved".into(), Lifecycle::Done).unwrap();
    feedback(&f.path, &"i_preserved".into(), true).unwrap();
    let last = session.snapshot().unwrap().messages.last().unwrap().cursor;
    mark_read(&f.path, &f.batch.chat.chat_id, last).unwrap();
    drop(session);
    let connection = Connection::open(&f.path).unwrap();
    let item_before: (String, String) = connection
        .query_row(
            "SELECT body_json,lifecycle FROM insights WHERE insight_id='i_preserved'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let feedback_before: String = connection
        .query_row(
            "SELECT label FROM feedback WHERE insight_id='i_preserved'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    connection.execute_batch("DROP TABLE relation_coverage; DROP TABLE semantic_relations; UPDATE meta SET value='1' WHERE key='db_version';").unwrap();
    drop(connection);
    let before = std::fs::read(&f.path).unwrap();
    let messages = list_messages(&f.path, &f.batch.chat.chat_id, None, None).unwrap();
    let chats = list_chats(&f.path).unwrap();
    assert_eq!(inspect(&f.path).unwrap(), Some(1));
    assert_eq!(f.report(None).uncovered_messages, 5);
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    let migrated = super::super::analysis::open_write(&f.path).unwrap();
    let item_after: (String, String) = migrated
        .query_row(
            "SELECT body_json,lifecycle FROM insights WHERE insight_id='i_preserved'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let feedback_after: String = migrated
        .query_row(
            "SELECT label FROM feedback WHERE insight_id='i_preserved'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(item_before, item_after);
    assert_eq!(feedback_before, feedback_after);
    drop(migrated);
    assert_eq!(inspect(&f.path).unwrap(), Some(2));
    assert_eq!(
        serde_json::to_value(list_messages(&f.path, &f.batch.chat.chat_id, None, None).unwrap())
            .unwrap(),
        serde_json::to_value(messages).unwrap()
    );
    assert_eq!(
        serde_json::to_value(list_chats(&f.path).unwrap()).unwrap(),
        serde_json::to_value(chats).unwrap()
    );
    assert_eq!(f.report(None).uncovered_messages, 5);
}

#[test]
fn failed_v1_migration_rolls_back_and_preserves_version_and_user_rows() {
    let f = Fixture::new();
    let connection = Connection::open(&f.path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE semantic_relations; UPDATE meta SET value='1' WHERE key='db_version';",
        )
        .unwrap();
    // An existing conflicting table makes migration fail after its first CREATE.
    assert!(super::super::analysis::open_write(&f.path).is_err());
    assert_eq!(inspect(&f.path).unwrap(), Some(1));
    let table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='semantic_relations')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!table);
    assert_eq!(
        list_messages(&f.path, &f.batch.chat.chat_id, None, None)
            .unwrap()
            .len(),
        5
    );
}
