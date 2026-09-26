//! Conservative, deterministic normalization of the documented Chinese date phrases.
//! A whole expression must match; unsupported prose never becomes a guessed date.

mod calendar;
mod clock;
mod number;

use chat_tldr_core::{Granularity, NormalizedBy, TemporalConstraint, TemporalRelation};
use chrono::{DateTime, FixedOffset, TimeDelta};

/// Resolve a date expression relative to the evidence message's local timestamp.
/// Unknown or invalid expressions retain `raw` and have no normalized bounds.
pub fn normalize(raw: &str, anchor: DateTime<FixedOffset>) -> TemporalConstraint {
    let mut result = TemporalConstraint {
        raw: raw.to_owned(),
        relation: TemporalRelation::At,
        bound_date: None,
        bound_time: None,
        granularity: Granularity::Unknown,
        anchor,
        normalized_by: NormalizedBy::None,
        confidence: 0.0,
    };
    let compact: String = raw
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    let Some(expression) = expression(&compact) else {
        return result;
    };
    result.relation = expression.relation;
    // Unlike naive_local(), checked arithmetic also tolerates chrono boundary
    // instants whose fixed-offset local date lies outside NaiveDate's range.
    let Some(local) = anchor
        .naive_utc()
        .checked_add_signed(TimeDelta::seconds(i64::from(
            anchor.offset().local_minus_utc(),
        )))
    else {
        return result;
    };
    let Ok((date, remaining)) = calendar::parse(expression.text, local.date(), expression.suffix)
    else {
        return result;
    };
    let hint = date.as_ref().and_then(|value| value.period);
    if remaining.is_empty() {
        if let Some(date) = date {
            result.bound_date = Some(date.value);
            result.granularity = if date.period.is_some() {
                Granularity::HalfDay
            } else {
                Granularity::Day
            };
            result.confidence = date.confidence;
            result.normalized_by = NormalizedBy::Rule;
        }
        return result;
    }
    let Some(time) = clock::parse(remaining, hint, date.is_some()) else {
        return result;
    };
    result.bound_date = Some(date.as_ref().map_or(local.date(), |value| value.value));
    result.bound_time = time.value;
    result.granularity = time.granularity;
    result.confidence = date
        .map_or(0.9_f32, |value| value.confidence)
        .min(time.confidence);
    result.normalized_by = NormalizedBy::Rule;
    result
}

struct Expression<'a> {
    text: &'a str,
    relation: TemporalRelation,
    suffix: Option<&'a str>,
}

fn expression(raw: &str) -> Option<Expression<'_>> {
    let mut text = raw;
    let mut relation = TemporalRelation::At;
    for prefix in ["截止", "截至", "ddl"] {
        if let Some(tail) = text.strip_prefix(prefix) {
            relation = TemporalRelation::Before;
            text = tail.strip_prefix([':', '：']).unwrap_or(tail);
            break;
        }
    }
    let mut suffix = None;
    // Match relationship suffixes, not the character 后 inside 后天/大后天.
    for marker in ["之前", "以前", "以内", "之后", "以后", "前", "内", "后"] {
        if let Some(head) = text.strip_suffix(marker) {
            let marked = if matches!(marker, "之后" | "以后" | "后") {
                TemporalRelation::After
            } else {
                TemporalRelation::Before
            };
            // Conflicting explicit relationships are not resolved by guessing.
            if relation == TemporalRelation::Before && marked == TemporalRelation::After {
                return None;
            }
            relation = marked;
            text = head;
            suffix = Some(marker);
            break;
        }
    }
    if text.is_empty() {
        return None;
    }
    Some(Expression {
        text,
        relation,
        suffix,
    })
}

#[cfg(test)]
mod tests;
