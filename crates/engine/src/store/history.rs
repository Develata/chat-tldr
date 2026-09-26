//! Read-only audit queries. DB v1 JSON extensions preserve older records verbatim.
use std::{collections::BTreeMap, path::Path};

use chat_tldr_core::*;
use chrono::DateTime;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::open_read;
use crate::{EngineError, Result};

pub struct History<T> {
    pub rows: Vec<T>,
    pub warnings: Vec<WarningPayload>,
    pub detail: Value,
}

impl<T> History<T> {
    fn new(rows: Vec<T>, detail: Value) -> Self {
        Self {
            rows,
            warnings: Vec::new(),
            detail,
        }
    }

    fn warn(&mut self, code: &str, message: &str) {
        if !self.warnings.iter().any(|warning| warning.code == code) {
            self.warnings.push(WarningPayload {
                stage: "history".into(),
                code: code.into(),
                message: message.into(),
            });
        }
    }
}

struct Run {
    chat: Option<ChatId>,
    status: String,
    started: String,
    ended: String,
}

fn read_run(connection: &Connection, run: &RunId) -> Result<Run> {
    connection.query_row(
        "SELECT chat_id,status,started_at,COALESCE(finished_at,heartbeat_at) FROM runs WHERE run_id=?1",
        [run.as_ref()],
        |row| Ok(Run { chat: row.get::<_,Option<String>>(0)?.map(Into::into), status: row.get(1)?, started: row.get(2)?, ended: row.get(3)? }),
    ).optional()?.ok_or_else(|| EngineError::RunNotFound(run.to_string()))
}

pub fn decisions(path: &Path, run: &RunId) -> Result<History<DecisionPayload>> {
    let Some(mut connection) = open_read(path)? else {
        return Err(EngineError::RunNotFound(run.to_string()));
    };
    let tx = connection.transaction()?;
    let record = read_run(&tx, run)?;
    let mut result = History::new(Vec::new(), json!({"run_status":record.status}));
    let mut statement = tx.prepare("SELECT step,observation_json,allowed_json,chosen_json,method,probabilities_json,reason FROM decisions WHERE run_id=?1 ORDER BY step")?;
    let mut rows = statement.query([run.as_ref()])?;
    while let Some(row) = rows.next()? {
        let observation: Value = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let extension = observation.get("_history_v1");
        if extension.is_none() {
            result.warn(
                "W_HISTORY_INCOMPLETE",
                "Legacy decisions did not persist confidence; unavailable values are null.",
            );
        }
        result.rows.push(DecisionPayload {
            step: row.get(0)?,
            confidence: extension
                .and_then(|value| value.get("confidence"))
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .flatten(),
            observation: serde_json::from_value(observation)?,
            allowed: serde_json::from_str(&row.get::<_, String>(2)?)?,
            chosen: serde_json::from_str(&row.get::<_, String>(3)?)?,
            method: serde_json::from_value(json!(row.get::<_, String>(4)?))?,
            probabilities: row
                .get::<_, Option<String>>(5)?
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            reason: row.get(6)?,
        });
    }
    Ok(result)
}

pub fn jev_log(path: &Path, run: &RunId) -> Result<History<JevAnswerPayload>> {
    let Some(mut connection) = open_read(path)? else {
        return Err(EngineError::RunNotFound(run.to_string()));
    };
    let tx = connection.transaction()?;
    let record = read_run(&tx, run)?;
    let mut result = History::new(
        Vec::new(),
        json!({"run_status":record.status,"unaligned_answers":0}),
    );
    let mut statement = tx.prepare("SELECT j.request_key,j.question_id,j.qtype,j.answer_json,j.confidence,j.subject_json,c.model FROM jev_answers j LEFT JOIN model_cache c ON c.cache_key=j.request_key WHERE j.run_id=?1 ORDER BY j.rowid")?;
    let mut rows = statement.query([run.as_ref()])?;
    let mut unaligned = 0_u64;
    while let Some(row) = rows.next()? {
        let source: Value = serde_json::from_str(&row.get::<_, String>(5)?)?;
        let subject = match serde_json::from_value::<AnswerSubject>(source.clone()) {
            Ok(subject) if subject.kind != SubjectKind::Unknown && !subject.id.is_empty() => {
                subject
            }
            _ => {
                unaligned += 1;
                result.warn("W_SUBJECT_UNAVAILABLE", "Legacy answers have no reliable subject mapping; kind=unknown/id=unavailable cannot be used for calibration alignment.");
                AnswerSubject {
                    kind: SubjectKind::Unknown,
                    id: "unavailable".into(),
                    message_ids: Vec::new(),
                    candidates: Vec::new(),
                }
            }
        };
        let cached_model: Option<String> = row.get(6)?;
        let model = source["_history_v1"]["model"]
            .as_str()
            .map(str::to_owned)
            .or(cached_model)
            .unwrap_or_else(|| "unknown".into());
        if model == "unknown" {
            result.warn("W_HISTORY_INCOMPLETE", "The original model name is unavailable for some legacy answers; it is reported as unknown.");
        }
        result.rows.push(JevAnswerPayload {
            model,
            request_key: row.get(0)?,
            question_id: row.get(1)?,
            qtype: row.get(2)?,
            answer: serde_json::from_str(&row.get::<_, String>(3)?)?,
            confidence: row.get(4)?,
            subject,
        });
    }
    result.detail["unaligned_answers"] = json!(unaligned);
    Ok(result)
}

pub fn stats(
    path: &Path,
    chat: Option<&ChatId>,
    run: Option<&RunId>,
) -> Result<History<StatsPayload>> {
    if chat.is_some() && run.is_some() {
        return Err(EngineError::Usage(
            "--chat and --run are mutually exclusive".into(),
        ));
    }
    let Some(mut connection) = open_read(path)? else {
        if let Some(run) = run {
            return Err(EngineError::RunNotFound(run.to_string()));
        }
        if let Some(chat) = chat {
            return Err(EngineError::ChatNotFound(chat.to_string()));
        }
        return Ok(History::new(
            vec![StatsPayload::Global(GlobalStats {
                counts: COUNT_TABLES
                    .iter()
                    .map(|(name, _)| ((*name).into(), 0))
                    .collect(),
                usage: Vec::new(),
                cost_usd: 0.0,
            })],
            json!({}),
        ));
    };
    let tx = connection.transaction()?;
    if let Some(run) = run {
        return run_stats(&tx, run);
    }
    if let Some(chat) = chat {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM chats WHERE chat_id=?1)",
            [chat.as_ref()],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(EngineError::ChatNotFound(chat.to_string()));
        }
    }
    let mut counts = BTreeMap::new();
    for (table, scoped) in COUNT_TABLES {
        let count = if let Some(chat) = chat {
            let Some(query) = scoped else {
                continue;
            };
            tx.query_row(query, [chat.as_ref()], |row| count_column(row, 0))?
        } else {
            tx.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                count_column(row, 0)
            })?
        };
        counts.insert((*table).into(), count);
    }
    let usage = read_usage(&tx, chat, None)?;
    let cost_usd = total_cost(&usage)?;
    Ok(History::new(
        vec![StatsPayload::Global(GlobalStats {
            counts,
            usage,
            cost_usd,
        })],
        json!({"chat_id":chat}),
    ))
}

fn run_stats(connection: &Connection, run: &RunId) -> Result<History<StatsPayload>> {
    let record = read_run(connection, run)?;
    let chat = record
        .chat
        .ok_or_else(|| EngineError::DatabaseFormat("run has no chat for run statistics".into()))?;
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [format!("history.run_stats.v1:{run}")],
            |row| row.get(0),
        )
        .optional()?;
    let mut result = History::new(Vec::new(), json!({"run_status":record.status}));
    let mut stats: RunStats = if let Some(body) = stored {
        let stats: RunStats = serde_json::from_str(&body)?;
        if stats.run_id != *run || stats.chat_id != chat {
            return Err(EngineError::DatabaseFormat(
                "stored run statistics identity mismatch".into(),
            ));
        }
        stats
    } else {
        result.warn("W_HISTORY_INCOMPLETE", "Exact counters were not saved for this run. Message/created counts reflect surviving database rows; insight update/verification counts are unavailable and reported as 0. Elapsed time ends at the saved finish/heartbeat time.");
        let count = |sql: &str| -> Result<u64> {
            Ok(connection.query_row(sql, [run.as_ref()], |row| count_column(row, 0))?)
        };
        let started = DateTime::parse_from_rfc3339(&record.started)
            .map_err(|_| EngineError::DatabaseFormat("invalid run start timestamp".into()))?;
        let ended = DateTime::parse_from_rfc3339(&record.ended)
            .map_err(|_| EngineError::DatabaseFormat("invalid run end timestamp".into()))?;
        RunStats {
            run_id: run.clone(),
            chat_id: chat,
            messages_analyzed: count("SELECT COUNT(*) FROM messages WHERE analyzed_run=?1")?,
            topics_created: count("SELECT COUNT(*) FROM topics WHERE created_in_run=?1")?,
            topics_updated: count(
                "SELECT COUNT(*) FROM run_checkpoints WHERE run_id=?1 AND stage='verify' AND status='complete'",
            )?,
            insights: InsightStats {
                created: count("SELECT COUNT(*) FROM insights WHERE created_in_run=?1")?,
                ..Default::default()
            },
            usage: Vec::new(),
            cost_usd: 0.0,
            elapsed_ms: (ended - started).num_milliseconds().max(0) as u64,
        }
    };
    stats.usage = read_usage(connection, None, Some(run))?;
    stats.cost_usd = total_cost(&stats.usage)?;
    result.rows.push(StatsPayload::Run(stats));
    Ok(result)
}

fn read_usage(
    connection: &Connection,
    chat: Option<&ChatId>,
    run: Option<&RunId>,
) -> Result<Vec<UsageStats>> {
    let mut statement = connection.prepare("SELECT u.stage,u.provider,u.model,SUM(u.calls),SUM(u.cache_hits),SUM(u.input_tokens),SUM(u.output_tokens),SUM(u.cost_usd) FROM usage u LEFT JOIN runs r ON r.run_id=u.run_id WHERE (?1 IS NULL OR r.chat_id=?1) AND (?2 IS NULL OR u.run_id=?2) GROUP BY u.stage,u.provider,u.model ORDER BY u.stage,u.provider,u.model")?;
    let rows = statement
        .query_map(
            params![chat.map(ChatId::as_ref), run.map(RunId::as_ref)],
            |row| {
                Ok(UsageStats {
                    stage: row.get(0)?,
                    provider: row.get(1)?,
                    model: row.get(2)?,
                    calls: count_column(row, 3)?,
                    cache_hits: count_column(row, 4)?,
                    input_tokens: count_column(row, 5)?,
                    output_tokens: count_column(row, 6)?,
                    cost_usd: row.get(7)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows
        .iter()
        .any(|row| !row.cost_usd.is_finite() || row.cost_usd < 0.0)
    {
        return Err(EngineError::DatabaseFormat(
            "invalid recorded usage cost".into(),
        ));
    }
    Ok(rows)
}

fn count_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    value
        .try_into()
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn total_cost(usage: &[UsageStats]) -> Result<f64> {
    let cost: f64 = usage.iter().map(|row| row.cost_usd).sum();
    if !cost.is_finite() {
        return Err(EngineError::DatabaseFormat(
            "recorded usage cost overflow".into(),
        ));
    }
    Ok(cost)
}

// Static SQL only. Global-only tables are omitted from chat-filtered counts.
const COUNT_TABLES: &[(&str, Option<&str>)] = &[
    ("chats", Some("SELECT COUNT(*) FROM chats WHERE chat_id=?1")),
    (
        "persons",
        Some("SELECT COUNT(DISTINCT person_id) FROM person_aliases WHERE chat_id=?1"),
    ),
    (
        "person_aliases",
        Some("SELECT COUNT(*) FROM person_aliases WHERE chat_id=?1"),
    ),
    (
        "imports",
        Some("SELECT COUNT(*) FROM imports WHERE chat_id=?1"),
    ),
    (
        "messages",
        Some("SELECT COUNT(*) FROM messages WHERE chat_id=?1"),
    ),
    (
        "message_mentions",
        Some(
            "SELECT COUNT(*) FROM message_mentions e JOIN messages m ON m.message_id=e.message_id WHERE m.chat_id=?1",
        ),
    ),
    (
        "topics",
        Some("SELECT COUNT(*) FROM topics WHERE chat_id=?1"),
    ),
    (
        "topic_messages",
        Some(
            "SELECT COUNT(*) FROM topic_messages e JOIN topics t ON t.topic_id=e.topic_id WHERE t.chat_id=?1",
        ),
    ),
    (
        "insights",
        Some("SELECT COUNT(*) FROM insights WHERE chat_id=?1"),
    ),
    (
        "evidence",
        Some(
            "SELECT COUNT(*) FROM evidence e JOIN insights i ON i.insight_id=e.insight_id WHERE i.chat_id=?1",
        ),
    ),
    (
        "feedback",
        Some(
            "SELECT COUNT(*) FROM feedback e JOIN insights i ON i.insight_id=e.insight_id WHERE i.chat_id=?1",
        ),
    ),
    ("runs", Some("SELECT COUNT(*) FROM runs WHERE chat_id=?1")),
    (
        "run_checkpoints",
        Some(
            "SELECT COUNT(*) FROM run_checkpoints e JOIN runs r ON r.run_id=e.run_id WHERE r.chat_id=?1",
        ),
    ),
    (
        "decisions",
        Some(
            "SELECT COUNT(*) FROM decisions e JOIN runs r ON r.run_id=e.run_id WHERE r.chat_id=?1",
        ),
    ),
    (
        "jev_answers",
        Some(
            "SELECT COUNT(*) FROM jev_answers e JOIN runs r ON r.run_id=e.run_id WHERE r.chat_id=?1",
        ),
    ),
    (
        "usage",
        Some("SELECT COUNT(*) FROM usage e JOIN runs r ON r.run_id=e.run_id WHERE r.chat_id=?1"),
    ),
    ("model_cache", None),
    ("preference_weights", None),
];

#[cfg(test)]
mod tests;
