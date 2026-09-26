//! Offline metrics over validated CLI artifacts. No engine or database access.
mod input;
mod metrics;

use std::path::Path;

use chat_tldr_core::VerificationStatus;
use serde_json::{Value, json};

use self::metrics::Metric;

pub fn run(gold: &Path, run: &Path, out: &Path) -> Result<Value, String> {
    let input = input::load(gold, run)?;
    let total = input.insights.len() as u64;
    let predictions: Vec<_> = input
        .insights
        .into_iter()
        .filter(|item| item.verification_status != VerificationStatus::Rejected)
        .collect();
    let retained = predictions.len() as u64;
    let mut rows = metrics::calculate(&predictions, &input.gold);
    rows.push(ratio("snapshot_rejected_rate", total - retained, total));
    rows.push(ratio("snapshot_rejected_rate_after_filter", 0, retained));
    rows.push(count("gold_messages", input.messages.len() as u64));
    rows.push(count("gold_items", input.gold.len() as u64));
    rows.push(count("snapshot_items", total));
    rows.push(count("scored_items", retained));

    // Saved inbox items may have been replaced by later extraction. They cannot
    // reconstruct the denominator of all raw LLM proposals across a run.
    rows.push(unavailable(
        "raw_unsupported_rate",
        "unavailable_raw_proposals",
    ));
    for name in [
        "thread_one_to_one",
        "thread_exact_f1",
        "ari",
        "nmi",
        "burst_purity",
        "boundary_f1",
        "mention_me_accuracy",
        "manual_unsupported_rate",
        "ece",
        "brier",
    ] {
        rows.push(unavailable(name, "not_implemented"));
    }

    let stats_available = input.stats.is_some();
    if let Some(stats) = input.stats {
        let sum = |extract: fn(&chat_tldr_core::UsageStats) -> u64| {
            stats.usage.iter().try_fold(0_u64, |sum, usage| {
                sum.checked_add(extract(usage))
                    .ok_or("usage counter overflow")
            })
        };
        rows.push(count("run_input_tokens", sum(|usage| usage.input_tokens)?));
        rows.push(count(
            "run_output_tokens",
            sum(|usage| usage.output_tokens)?,
        ));
        rows.push(count("run_calls", sum(|usage| usage.calls)?));
        rows.push(count("run_cache_hits", sum(|usage| usage.cache_hits)?));
        rows.push(count("run_elapsed_ms", stats.elapsed_ms));
        rows.push(Metric {
            name: "run_estimated_cost_usd".into(),
            value: Some(stats.cost_usd),
            numerator: None,
            denominator: None,
            status: "ok".into(),
        });
    } else {
        for name in [
            "run_input_tokens",
            "run_output_tokens",
            "run_calls",
            "run_cache_hits",
            "run_elapsed_ms",
            "run_estimated_cost_usd",
        ] {
            rows.push(unavailable(name, "unavailable_run_stats"));
        }
    }

    crate::output::file(out, |file| {
        let mut writer = csv::Writer::from_writer(file);
        writer
            .write_record([
                "score_version",
                "system",
                "metric",
                "value",
                "numerator",
                "denominator",
                "status",
            ])
            .map_err(|error| format!("cannot write score header: {error}"))?;
        for row in &rows {
            writer
                .write_record([
                    "1".to_owned(),
                    "ours".to_owned(),
                    row.name.clone(),
                    row.value.map(|value| value.to_string()).unwrap_or_default(),
                    row.numerator
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    row.denominator
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                    row.status.clone(),
                ])
                .map_err(|error| format!("cannot write score row: {error}"))?;
        }
        writer
            .flush()
            .map_err(|error| format!("cannot flush scores: {error}"))
    })?;
    Ok(json!({
        "command":"score", "valid":true, "score_version":1, "system":"ours",
        "gold_messages":input.messages.len(), "gold_items":input.gold.len(),
        "snapshot_items":total, "scored_items":retained, "metrics":rows.len(),
        "run_stats_available":stats_available, "exit_code_source":"stream_only",
        "warning_codes":input.warnings, "error":null
    }))
}

fn count(name: &str, value: u64) -> Metric {
    Metric {
        name: name.into(),
        value: Some(value as f64),
        numerator: Some(value),
        denominator: None,
        status: "ok".into(),
    }
}

fn ratio(name: &str, numerator: u64, denominator: u64) -> Metric {
    Metric {
        name: name.into(),
        value: (denominator != 0).then(|| numerator as f64 / denominator as f64),
        numerator: Some(numerator),
        denominator: Some(denominator),
        status: if denominator == 0 { "undefined" } else { "ok" }.into(),
    }
}

fn unavailable(name: &str, status: &str) -> Metric {
    Metric {
        name: name.into(),
        value: None,
        numerator: None,
        denominator: None,
        status: status.into(),
    }
}
