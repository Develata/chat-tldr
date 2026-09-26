use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use chat_tldr_core::{
    ChatId, EventBody, InboxPayload, Insight, InsightKind, Lifecycle, MessagePayload, Priority,
    PriorityCounts, RunStats, RunStatus, StatsPayload, TemporalRelation, VerificationStatus,
};
use serde::de::DeserializeOwned;

use crate::{
    annotation::{GoldItem, GoldMessage, message_id},
    stream,
};

pub struct Inputs {
    pub gold: Vec<GoldItem>,
    pub messages: Vec<MessagePayload>,
    pub insights: Vec<Insight>,
    pub stats: Option<RunStats>,
    pub warnings: Vec<String>,
}

pub fn load(gold: &Path, run: &Path) -> Result<Inputs, String> {
    let labels: Vec<GoldMessage> = read_gold(&gold.join("messages.jsonl"), "gold messages")?;
    let mut gold_ids = BTreeSet::new();
    for label in &labels {
        label.validate()?;
        if !gold_ids.insert(label.message_id.as_str()) {
            return Err("duplicate message_id in gold messages".into());
        }
    }
    let gold: Vec<GoldItem> = read_gold(&gold.join("items.jsonl"), "gold items")?;
    let mut item_ids = BTreeSet::new();
    for item in &gold {
        item.validate()?;
        if !item_ids.insert(item.item_id.as_str()) {
            return Err("duplicate item_id in gold items".into());
        }
        if item
            .anchors
            .iter()
            .any(|id| !gold_ids.contains(id.as_str()))
        {
            return Err("gold item anchor is absent from gold messages".into());
        }
    }

    let messages = read_messages(&run.join("messages.jsonl"))?;
    let message_ids: BTreeSet<_> = messages.iter().map(|row| row.message_id.as_ref()).collect();
    if message_ids != gold_ids {
        return Err("non-recalled run message IDs must exactly match gold messages".into());
    }
    let (header, insights) = read_inbox(&run.join("inbox.jsonl"), &message_ids)?;
    let (stats, warnings) = read_analysis(&run.join("analyze.jsonl"), &header.chat_id)?;
    Ok(Inputs {
        gold,
        messages,
        insights,
        stats,
        warnings,
    })
}

fn read_gold<T: DeserializeOwned>(path: &Path, label: &str) -> Result<Vec<T>, String> {
    let file = File::open(path).map_err(|_| format!("cannot open {label} JSONL"))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let number = index + 1;
            let line =
                line.map_err(|_| format!("{label} line {number}: cannot read UTF-8 JSONL"))?;
            serde_json::from_str(&line)
                .map_err(|_| format!("{label} line {number}: invalid fields or JSON"))
        })
        .collect()
}

// Existing stream diagnostics can contain a deserializer's offending value.
// Keep the line number, but never copy that value into aggregate score output.
fn safe_stream_error(label: &str, error: String) -> String {
    let line = error
        .strip_prefix("line ")
        .and_then(|tail| tail.split_once(':'))
        .map(|(number, _)| number)
        .filter(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()));
    match line {
        Some(number) => format!("{label} line {number}: invalid or inconsistent event stream"),
        None => format!("{label}: cannot read a complete successful event stream"),
    }
}

fn read_messages(path: &Path) -> Result<Vec<MessagePayload>, String> {
    let rows =
        stream::messages(path, None).map_err(|error| safe_stream_error("run messages", error))?;
    let mut seen = BTreeSet::new();
    let mut messages = Vec::new();
    for row in rows {
        message_id(row.message_id.as_ref())?;
        if !seen.insert(row.message_id.clone()) {
            return Err("duplicate message_id in run messages".into());
        }
        if !row.recalled {
            messages.push(row);
        }
    }
    Ok(messages)
}

fn read_inbox(
    path: &Path,
    messages: &BTreeSet<&str>,
) -> Result<(InboxPayload, Vec<Insight>), String> {
    let mut header: Option<InboxPayload> = None;
    let mut insights: Vec<Insight> = Vec::new();
    let mut ids = BTreeSet::new();
    let mut topic_ids = BTreeSet::new();
    let mut counts = PriorityCounts::default();
    let mut rejected = 0;
    let (_, result) = stream::visit(path, None, |body| {
        if matches!(body, EventBody::Unknown { .. }) {
            return Ok(());
        }
        if header.is_none() {
            if let EventBody::Inbox(value) = body {
                valid_id(value.chat_id.as_ref())?;
                header = Some(value);
                return Ok(());
            }
            return Err("first known event must be an inbox header".into());
        }
        let chat = &header.as_ref().expect("header checked above").chat_id;
        match body {
            EventBody::Topic(topic) => {
                if topic.chat_id != *chat || !topic_ids.insert(topic.topic_id) {
                    return Err("duplicate topic or mismatched chat".into());
                }
            }
            EventBody::Insight(payload) => {
                let insight = payload.insight;
                validate_insight(&insight, messages)?;
                if insight.chat_id != *chat || !ids.insert(insight.id.clone()) {
                    return Err("duplicate insight or mismatched chat".into());
                }
                if let Some(previous) = insights.last()
                    && compare_insights(previous, &insight).is_gt()
                {
                    return Err("insights are not in inbox rank order".into());
                }
                // The engine counts every emitted row, including rejected rows
                // when --include-rejected is requested.
                match insight.priority {
                    Priority::P0 => counts.p0 += 1,
                    Priority::P1 => counts.p1 += 1,
                    Priority::P2 => counts.p2 += 1,
                    Priority::P3 => counts.p3 += 1,
                    Priority::Unknown => unreachable!("validated above"),
                }
                rejected += u64::from(insight.verification_status == VerificationStatus::Rejected);
                insights.push(insight);
            }
            EventBody::Done(done) if done.status == RunStatus::Complete && done.exit_code == 0 => {}
            _ => return Err("unexpected inbox event".into()),
        }
        Ok(())
    });
    result.map_err(|error| safe_stream_error("inbox", error))?;
    let header = header.ok_or("inbox header is missing")?;
    if header.counts != counts {
        return Err("inbox priority counts do not match emitted insights".into());
    }
    if header.rejected_insights != rejected {
        return Err("inbox rejected count does not match emitted rejected insights; export with --all --include-resolved --include-rejected".into());
    }
    Ok((header, insights))
}

fn compare_insights(left: &Insight, right: &Insight) -> std::cmp::Ordering {
    left.priority
        .cmp(&right.priority)
        .then_with(|| right.rank_score.total_cmp(&left.rank_score))
        .then_with(|| left.id.cmp(&right.id))
}

fn valid_id(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        Err("invalid identifier".into())
    } else {
        Ok(())
    }
}

fn probability(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn validate_insight(insight: &Insight, messages: &BTreeSet<&str>) -> Result<(), String> {
    valid_id(insight.id.as_ref())?;
    valid_id(insight.created_in_run.as_ref())?;
    if insight.kind == InsightKind::Unknown
        || insight.priority == Priority::Unknown
        || insight.verification_status == VerificationStatus::Unknown
        || insight.lifecycle == Lifecycle::Unknown
    {
        return Err(
            "unknown insight kind, priority, verification or lifecycle cannot be scored".into(),
        );
    }
    if !insight.rank_score.is_finite()
        || insight.confidence.is_some_and(|value| !probability(value))
    {
        return Err("invalid insight rank or confidence".into());
    }
    if let Some(deadline) = &insight.deadline
        && (deadline.raw.trim().is_empty()
            || !probability(deadline.confidence)
            || (deadline.bound_date.is_some() && deadline.relation == TemporalRelation::Unknown)
            || (deadline.bound_time.is_some() && deadline.bound_date.is_none()))
    {
        return Err("invalid deadline fields".into());
    }
    if insight.verification_status == VerificationStatus::Unverified
        && (insight.kind != InsightKind::TopicSummary || insight.deadline.is_some())
    {
        return Err("only topic summaries without deadlines may be unverified".into());
    }
    if insight.verification_status != VerificationStatus::Rejected {
        // Check reference coverage only; the offline evaluator does not rerun
        // engine verification. Unverified summaries can retain failed evidence
        // alongside a supported reference, including missing/recalled messages.
        let referenced = insight
            .evidence
            .iter()
            .filter(|evidence| {
                !evidence.quote.trim().is_empty() && messages.contains(evidence.message_id.as_ref())
            })
            .count();
        if referenced == 0
            || (insight.verification_status == VerificationStatus::Verified
                && referenced != insight.evidence.len())
        {
            return Err(
                "insight evidence does not satisfy its verification reference coverage".into(),
            );
        }
    }
    Ok(())
}

fn read_analysis(path: &Path, chat: &ChatId) -> Result<(Option<RunStats>, Vec<String>), String> {
    let mut stats = None;
    let mut warnings = BTreeSet::new();
    let (state, result) = stream::visit(path, None, |body| {
        match body {
            EventBody::Stats(StatsPayload::Run(value)) => {
                if stats.is_some() || value.chat_id != *chat {
                    return Err("duplicate run stats or mismatched chat".into());
                }
                validate_stats(&value)?;
                stats = Some(value);
            }
            EventBody::Topic(topic) if topic.chat_id == *chat => {}
            EventBody::Insight(payload) if payload.insight.chat_id == *chat => {}
            EventBody::Warning(warning) => {
                let code = if warning.code.starts_with("W_")
                    && warning.code.len() <= 80
                    && warning.code.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    }) {
                    warning.code
                } else {
                    "W_UNRECOGNIZED_WARNING_CODE".into()
                };
                warnings.insert(code);
            }
            EventBody::Progress(_) | EventBody::Decision(_) | EventBody::Unknown { .. } => {}
            EventBody::Done(done) if done.status == RunStatus::Complete && done.exit_code == 0 => {}
            // A dry run's ack is a plan, not evidence of analysis.
            _ => return Err("expected a successful analyze stream".into()),
        }
        Ok(())
    });
    result.map_err(|error| safe_stream_error("analyze", error))?;
    if let Some(value) = &stats
        && Some(&value.run_id) != state.run_id.as_ref()
    {
        return Err("analyze stats run_id does not match its event envelope".into());
    }
    if warnings.contains("W_HISTORY_INCOMPLETE") {
        stats = None;
    } else if stats.is_none() {
        warnings.insert("W_RUN_STATS_MISSING".into());
    }
    Ok((stats, warnings.into_iter().collect()))
}

fn validate_stats(stats: &RunStats) -> Result<(), String> {
    if !stats.cost_usd.is_finite()
        || stats.cost_usd < 0.0
        || stats
            .usage
            .iter()
            .any(|usage| !usage.cost_usd.is_finite() || usage.cost_usd < 0.0)
    {
        return Err("run costs must be finite and nonnegative".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use chat_tldr_core::{CliEvent, InsightStats};
    use serde_json::{Value, json};

    fn inbox_fixture() -> Vec<Value> {
        include_str!("../../../fixtures/jsonl/inbox.jsonl")
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn write_events(rows: &[Value]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for (index, row) in rows.iter().enumerate() {
            let mut row = row.clone();
            row["seq"] = json!(index);
            serde_json::to_writer(file.as_file_mut(), &row).unwrap();
            writeln!(file.as_file_mut()).unwrap();
        }
        file
    }

    fn stats() -> RunStats {
        RunStats {
            run_id: "r_test".into(),
            chat_id: "qq:group:synthetic-course".into(),
            messages_analyzed: 1,
            topics_created: 0,
            topics_updated: 0,
            insights: InsightStats::default(),
            usage: Vec::new(),
            cost_usd: 0.0,
            elapsed_ms: 1,
        }
    }

    fn analysis_rows(stats: RunStats) -> Vec<Value> {
        vec![
            serde_json::to_value(CliEvent::new(
                "r_test".into(),
                0,
                EventBody::Stats(StatsPayload::Run(stats)),
            ))
            .unwrap(),
            json!({"schema_version":"1.0","run_id":"r_test","seq":1,
                "event":"done","payload":{"status":"complete","exit_code":0,"elapsed_ms":1}}),
        ]
    }

    #[test]
    fn rejected_rows_are_counted_and_may_reference_missing_evidence() {
        let mut rows = inbox_fixture();
        rows[0]["payload"]["rejected_insights"] = json!(1);
        rows[2]["payload"]["insight"]["verification_status"] = json!("rejected");
        rows[2]["payload"]["insight"]["evidence"][0]["message_id"] = json!("m_ffffffffffffffff");
        let file = write_events(&rows);
        let (header, items) = read_inbox(file.path(), &BTreeSet::new()).unwrap();
        assert_eq!(header.counts.p0, 1);
        assert_eq!(items.len(), 1);

        rows[0]["payload"]["counts"]["P0"] = json!(0);
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &BTreeSet::new()).is_err());
    }

    #[test]
    fn missing_rejected_rows_and_invalid_rank_order_are_rejected() {
        let mut rows = inbox_fixture();
        let messages = BTreeSet::from(["m_0000000000000001"]);
        rows[0]["payload"]["rejected_insights"] = json!(1);
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &messages).is_err());
        rows[0]["payload"]["rejected_insights"] = json!(0);
        rows[0]["payload"]["counts"]["P0"] = json!(2);
        let mut item = rows[2].clone();
        item["payload"]["insight"]["id"] = json!("i_000000000002");
        item["payload"]["insight"]["rank_score"] = json!(3.0);
        rows.insert(3, item);
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &messages).is_err());
    }

    #[test]
    fn unknown_minor_events_preserve_known_inbox_validation() {
        let mut rows = inbox_fixture();
        rows.insert(0, json!({"schema_version":"1.2","run_id":"r_20260926T120300_0004","seq":0,"event":"future","payload":{}}));
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &BTreeSet::from(["m_0000000000000001"])).is_ok());
    }

    #[test]
    fn unknown_assignee_and_raw_deadline_remain_valid() {
        let mut rows = inbox_fixture();
        let item = &mut rows[2]["payload"]["insight"];
        item["assignee"] = json!("unknown");
        item["deadline"]["relation"] = json!("unknown");
        item["deadline"]["bound_date"] = Value::Null;
        item["deadline"]["bound_time"] = Value::Null;
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &BTreeSet::from(["m_0000000000000001"])).is_ok());
        rows[2]["payload"]["insight"]["kind"] = json!("future_kind");
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &BTreeSet::from(["m_0000000000000001"])).is_err());
    }

    #[test]
    fn unverified_summary_keeps_its_row_with_partial_evidence_coverage() {
        let mut rows = inbox_fixture();
        let item = &mut rows[2]["payload"]["insight"];
        item["kind"] = json!("topic_summary");
        item["deadline"] = Value::Null;
        item["verification_status"] = json!("unverified");
        let evidence = item["evidence"].as_array_mut().unwrap();
        let mut missing = evidence[0].clone();
        missing["message_id"] = json!("m_ffffffffffffffff");
        evidence.push(missing);
        let mut empty = evidence[0].clone();
        empty["quote"] = json!("");
        evidence.push(empty);
        let file = write_events(&rows);
        let (_, insights) =
            read_inbox(file.path(), &BTreeSet::from(["m_0000000000000001"])).unwrap();
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].kind, InsightKind::TopicSummary);
        assert_eq!(insights[0].evidence.len(), 3);

        rows[2]["payload"]["insight"]["verification_status"] = json!("verified");
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &BTreeSet::from(["m_0000000000000001"])).is_err());
    }

    #[test]
    fn unverified_summary_allows_recalled_extra_evidence_but_needs_one_live_reference() {
        let mut message_rows = vec![
            json!({"schema_version":"1.0","run_id":"r_test","seq":0,"event":"message",
                "payload":{"message_id":"m_0000000000000001","sender":"qq:synthetic",
                    "sender_display":"合成用户","sent_at":"2026-09-26T10:00:00+08:00",
                    "display_text":"合成消息","recalled":false,"system":false,"reply_to":null,
                    "mentions_me":false,"topic_id":null,"burst_id":null,"cursor":"1790388000000:1"}}),
            analysis_rows(stats()).pop().unwrap(),
        ];
        let mut recalled = message_rows[0].clone();
        recalled["payload"]["message_id"] = json!("m_ffffffffffffffff");
        recalled["payload"]["recalled"] = json!(true);
        message_rows.insert(1, recalled);
        let messages_file = write_events(&message_rows);
        let messages = read_messages(messages_file.path()).unwrap();
        let message_ids = messages.iter().map(|row| row.message_id.as_ref()).collect();

        let mut rows = inbox_fixture();
        let item = &mut rows[2]["payload"]["insight"];
        item["kind"] = json!("topic_summary");
        item["deadline"] = Value::Null;
        item["verification_status"] = json!("unverified");
        let evidence = item["evidence"].as_array_mut().unwrap();
        let mut recalled = evidence[0].clone();
        recalled["message_id"] = json!("m_ffffffffffffffff");
        evidence.push(recalled);
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &message_ids).is_ok());

        rows[2]["payload"]["insight"]["evidence"][0]["quote"] = json!("  ");
        let file = write_events(&rows);
        assert!(read_inbox(file.path(), &message_ids).is_err());
    }

    #[test]
    fn statistics_require_the_original_run_and_incomplete_history_is_unavailable() {
        let mut wrong = stats();
        wrong.run_id = "r_another".into();
        let file = write_events(&analysis_rows(wrong));
        assert!(read_analysis(file.path(), &stats().chat_id).is_err());

        let mut rows = analysis_rows(stats());
        rows.insert(0, json!({"schema_version":"1.0","run_id":"r_test","seq":0,
            "event":"warning","payload":{"stage":"store","code":"W_HISTORY_INCOMPLETE","message":"PRIVATE_CHAT"}}));
        let file = write_events(&rows);
        let (stats, warnings) = read_analysis(file.path(), &stats().chat_id).unwrap();
        assert!(stats.is_none());
        assert_eq!(warnings, ["W_HISTORY_INCOMPLETE"]);
    }

    #[test]
    fn malformed_payload_errors_do_not_disclose_chat_values() {
        let mut rows = inbox_fixture();
        rows[2]["payload"]["insight"]["confidence"] = json!("PRIVATE_CHAT");
        let file = write_events(&rows);
        let error = read_inbox(file.path(), &BTreeSet::new()).unwrap_err();
        assert!(error.contains("line 3"));
        assert!(!error.contains("PRIVATE_CHAT"));
    }

    #[test]
    fn gold_fields_are_complete_and_unknown_fields_are_rejected() {
        assert!(
            serde_json::from_value::<GoldMessage>(json!({
                "message_id":"m_0000000000000001","thread":"th1","todo":false
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<GoldMessage>(json!({
                "message_id":"m_0000000000000001","thread":"th1","todo":false,
                "announcement":false,"unreviewed":true
            }))
            .is_err()
        );
    }
}
