use super::*;
use chat_tldr_core::{AnswerSubject, SubjectKind};

fn answer() -> JevAnswerPayload {
    JevAnswerPayload {
        model: "synthetic-jev".into(),
        request_key: "request".into(),
        question_id: "n1_todo".into(),
        qtype: "noul".into(),
        answer: json!({"type":"noul","p_yes":0.8}),
        confidence: None,
        subject: AnswerSubject {
            kind: SubjectKind::Message,
            id: "m_0000000000000001".into(),
            message_ids: vec![],
            candidates: vec![],
        },
    }
}

#[test]
fn missing_labels_and_unknown_subjects_are_excluded_not_false() {
    let mut row = answer();
    assert_eq!(
        sample(&row, &BTreeMap::new(), &BTreeMap::new()).unwrap(),
        Err("missing_message_gold")
    );
    row.subject.kind = SubjectKind::Unknown;
    assert_eq!(
        sample(&row, &BTreeMap::new(), &BTreeMap::new()).unwrap(),
        Err("unlabelled_subject")
    );
}

#[test]
fn choice_uses_actual_top_probability_not_normalized_confidence() {
    let mut row = answer();
    row.qtype = "choice".into();
    row.confidence = Some(0.6);
    row.answer = json!({"type":"choice","choice":"T0","probabilities":{"T0":0.8,"new_topic":0.2},"confidence":0.6});
    let choices = BTreeMap::from([(
        (row.request_key.clone(), row.question_id.clone()),
        "new_topic".into(),
    )]);
    assert_eq!(
        sample(&row, &BTreeMap::new(), &choices).unwrap(),
        Ok(("choice_top_label".into(), 0.8, false))
    );
    row.answer["probabilities"]["T0"] = json!(0.7);
    assert!(sample(&row, &BTreeMap::new(), &choices).is_err());
}

#[test]
fn invalid_probabilities_are_rejected() {
    for value in [json!(-0.1), json!(1.1), json!(null), json!("PRIVATE_CHAT")] {
        assert!(probability(&value).is_err());
    }
}
