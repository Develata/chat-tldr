//! Offline, model-separated calibration. Unknown labels are never negative examples.
mod metrics;
mod plot;

use crate::annotation::GoldMessage;
use chat_tldr_core::{EventBody, JevAnswerPayload, RunStatus, SubjectKind};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChoiceGold {
    request_key: String,
    question_id: String,
    choice: String,
}

fn read<T: DeserializeOwned>(path: &Path, kind: &str) -> Result<Vec<T>, String> {
    let file = File::open(path).map_err(|_| format!("cannot open {kind}"))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let line = line.map_err(|_| format!("{kind}: cannot read UTF-8 line {}", index + 1))?;
            serde_json::from_str(&line)
                .map_err(|_| format!("{kind}: invalid JSON or fields on line {}", index + 1))
        })
        .collect()
}

pub fn run(
    gold: &Path,
    source: &Path,
    choices: Option<&Path>,
    out: &Path,
) -> Result<Value, String> {
    let mut labels = BTreeMap::new();
    for label in read::<GoldMessage>(&gold.join("messages.jsonl"), "gold messages")? {
        label.validate()?;
        if labels.insert(label.message_id.clone(), label).is_some() {
            return Err("duplicate gold message ID".into());
        }
    }
    let mut choice_labels = BTreeMap::new();
    if let Some(path) = choices {
        for label in read::<ChoiceGold>(path, "choice gold")? {
            if label.request_key.is_empty()
                || label.question_id.is_empty()
                || label.choice.is_empty()
                || choice_labels
                    .insert((label.request_key, label.question_id), label.choice)
                    .is_some()
            {
                return Err("choice gold has empty fields or duplicate keys".into());
            }
        }
    }
    let mut seen = BTreeMap::<(String, String, String), JevAnswerPayload>::new();
    let mut groups = BTreeMap::<(String, String), metrics::Accumulator>::new();
    let mut excluded = BTreeMap::<String, u64>::new();
    let mut total = 0_u64;
    let mut duplicates = 0_u64;
    let mut header = false;
    let (_, result) = crate::stream::visit(source, None, |body| {
        let answer = match body {
            EventBody::Ack(ack) if ack.command == "jev-log" && !ack.changed && !header => {
                header = true;
                return Ok(());
            }
            EventBody::JevAnswer(answer) if header => answer,
            EventBody::Warning(_) | EventBody::Unknown { .. } => return Ok(()),
            EventBody::Done(done)
                if header && done.exit_code == 0 && done.status == RunStatus::Complete =>
            {
                return Ok(());
            }
            _ => return Err("expected a complete successful jev-log stream".into()),
        };
        total += 1;
        if answer.model.trim().is_empty()
            || answer.model.len() > 200
            || answer.request_key.is_empty()
            || answer.question_id.is_empty()
        {
            return Err("missing or oversized model/question identity".into());
        }
        let key = (
            answer.model.clone(),
            answer.request_key.clone(),
            answer.question_id.clone(),
        );
        if let Some(previous) = seen.get(&key) {
            let controller_replay = previous.subject.kind == SubjectKind::Controller
                && answer.subject.kind == SubjectKind::Controller
                && previous.answer == answer.answer
                && previous.confidence == answer.confidence
                && previous.qtype == answer.qtype
                && previous.subject.candidates == answer.subject.candidates;
            if previous != &answer && !controller_replay {
                return Err("conflicting answers for the same model/request/question".into());
            }
            duplicates += 1;
            return Ok(());
        }
        let sample = sample(&answer, &labels, &choice_labels)?;
        match sample {
            Ok((task, p, actual)) => groups
                .entry((answer.model.clone(), task))
                .or_default()
                .add(p, actual),
            Err(reason) => *excluded.entry(reason.into()).or_default() += 1,
        }
        seen.insert(key, answer);
        Ok(())
    });
    // Stream parse errors may embed source values; keep aggregate diagnostics private-text-free.
    result.map_err(|_| "invalid, conflicting or incomplete jev-log stream".to_owned())?;
    let groups: Vec<_> = groups
        .into_iter()
        .map(|((model, task), values)| values.finish(model, task))
        .collect();
    let included: u64 = groups.iter().map(|group| group.count).sum();
    let result = json!({"calibration_version":1,"status":if included==0 {"unavailable_no_labels"} else {"ok"},
        "sample_unit":"unique model/request_key/question_id", "bins":10, "total_answers":total,
        "included":included, "duplicate_answers":duplicates,"excluded":excluded,"groups":groups});
    crate::output::directory(out, |directory| {
        let file = File::create(directory.join("calibration.json"))
            .map_err(|_| "cannot create calibration JSON")?;
        serde_json::to_writer_pretty(&file, &result)
            .map_err(|_| "cannot write calibration JSON")?;
        file.sync_all()
            .map_err(|_| "cannot flush calibration JSON")?;
        plot::draw(&groups, &directory.join("reliability.svg"))
    })?;
    Ok(
        json!({"command":"calibrate","valid":true,"included":included,"groups":groups.len(),
        "total_answers":total,"duplicate_answers":duplicates,"excluded":excluded,"status":result["status"],"error":null}),
    )
}

type LabelledSample = Result<(String, f64, bool), &'static str>;

fn sample(
    answer: &JevAnswerPayload,
    labels: &BTreeMap<String, GoldMessage>,
    choices: &BTreeMap<(String, String), String>,
) -> Result<LabelledSample, String> {
    match answer.qtype.as_str() {
        "noul" => {
            if answer.answer["type"] != "noul" {
                return Err("wrong noul answer type".into());
            }
            let p = probability(&answer.answer["p_yes"])?;
            if answer.subject.kind != SubjectKind::Message {
                return Ok(Err("unlabelled_subject"));
            }
            let Some(label) = labels.get(&answer.subject.id) else {
                return Ok(Err("missing_message_gold"));
            };
            if answer.question_id.ends_with("_todo") {
                Ok(Ok(("todo".into(), p, label.todo)))
            } else if answer.question_id.ends_with("_announcement") {
                Ok(Ok(("announcement".into(), p, label.announcement)))
            } else {
                Ok(Err("unlabelled_question"))
            }
        }
        "choice" => {
            if answer.answer["type"] != "choice" {
                return Err("wrong choice answer type".into());
            }
            let probabilities = answer.answer["probabilities"]
                .as_object()
                .ok_or("missing choice distribution")?;
            let values: BTreeMap<_, _> = probabilities
                .iter()
                .map(|(key, value)| probability(value).map(|p| (key.as_str(), p)))
                .collect::<Result<_, _>>()?;
            if values.is_empty() || (values.values().sum::<f64>() - 1.0).abs() > 0.001 {
                return Err("invalid choice distribution".into());
            }
            let choice = answer.answer["choice"]
                .as_str()
                .ok_or("missing chosen option")?;
            let p = *values
                .get(choice)
                .ok_or("chosen option absent from distribution")?;
            if values.values().any(|value| *value > p + 1e-5) {
                return Err("chosen option is not a maximum".into());
            }
            let Some(correct) =
                choices.get(&(answer.request_key.clone(), answer.question_id.clone()))
            else {
                return Ok(Err("missing_choice_gold"));
            };
            if !values.contains_key(correct.as_str()) {
                return Err("choice gold option absent from candidates".into());
            }
            // Top-label probability, NOT Jev's normalized confidence and NOT multiclass Brier.
            Ok(Ok(("choice_top_label".into(), p, choice == correct)))
        }
        _ => Ok(Err("unlabelled_question_type")),
    }
}

fn probability(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
        .ok_or("invalid probability".into())
}

#[cfg(test)]
mod tests;
