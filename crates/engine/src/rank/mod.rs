//! Hard priority rules and bounded ranking within each priority layer.

use std::cmp::Ordering;

use chat_tldr_core::{Assignee, Insight, InsightKind, Priority, TemporalConstraint};
use chrono::{DateTime, FixedOffset};

fn probability(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
}

pub fn priority(
    kind: InsightKind,
    assignee: Assignee,
    has_deadline: bool,
    needs_action: Option<f32>,
    is_chitchat: Option<f32>,
) -> Priority {
    if has_deadline && kind != InsightKind::TopicSummary {
        return Priority::P0;
    }
    match kind {
        InsightKind::MentionMe => {
            if probability(needs_action).is_some_and(|value| value >= 0.5) {
                Priority::P0
            } else {
                Priority::P1
            }
        }
        InsightKind::Todo => match assignee {
            Assignee::Me | Assignee::All => Priority::P0,
            Assignee::Unknown => Priority::P1,
            Assignee::Other => Priority::P2,
        },
        InsightKind::Announcement | InsightKind::Decision => Priority::P1,
        InsightKind::TopicSummary => {
            if probability(is_chitchat).is_some_and(|value| value >= 0.5) {
                Priority::P3
            } else {
                Priority::P2
            }
        }
        InsightKind::Unknown => Priority::Unknown,
    }
}

/// The urgency argument is Jev's raw score in [0, 2], before normalization.
/// Invalid probabilities are treated as absent. Date-only deadlines use calendar
/// days in the evidence anchor's offset, without inventing a midnight deadline.
pub fn rank_prior(
    deadline: Option<&TemporalConstraint>,
    confidence: Option<f32>,
    urgency: Option<f32>,
    last_evidence_at: DateTime<FixedOffset>,
    now: DateTime<FixedOffset>,
) -> f32 {
    let deadline_proximity = deadline.map_or(0.0, |deadline| {
        let Some(date) = deadline.bound_date else {
            return 0.0;
        };
        let days_remaining = if let Some(time) = deadline.bound_time {
            let Some(bound) = date
                .and_time(time)
                .and_local_timezone(*deadline.anchor.offset())
                .single()
            else {
                return 0.0;
            };
            (bound - now).num_milliseconds() as f64 / 86_400_000.0
        } else {
            let local_today = now.with_timezone(deadline.anchor.offset()).date_naive();
            (date - local_today).num_days() as f64
        };
        if days_remaining < 0.0 {
            0.0
        } else {
            1.0 / (1.0 + days_remaining)
        }
    });
    let confidence = f64::from(probability(confidence).unwrap_or(0.5));
    let urgency = urgency
        .filter(|value| value.is_finite() && (0.0..=2.0).contains(value))
        .map_or(0.0, |value| f64::from(value) / 2.0);
    let elapsed_days = ((now - last_evidence_at).num_milliseconds() as f64 / 86_400_000.0).max(0.0);
    let recency = (-elapsed_days).exp();
    (deadline_proximity + 0.8 * confidence + 0.5 * urgency + 0.3 * recency) as f32
}

/// Feedback only changes a score. It cannot receive or change a Priority.
/// Non-finite weights are neutral; finite weights are clamped to the documented
/// per-feature limits before averaging. Non-finite priors use a neutral zero.
pub fn apply_feedback(prior: f32, weights: &[f32]) -> f32 {
    let prior = if prior.is_finite() { prior } else { 0.0 };
    if weights.is_empty() {
        return prior;
    }
    let sum: f64 = weights
        .iter()
        .map(|weight| {
            if weight.is_finite() {
                f64::from(weight.clamp(-0.5, 0.5))
            } else {
                0.0
            }
        })
        .sum();
    let adjustment = (sum / weights.len() as f64).clamp(-0.5, 0.5) as f32;
    prior + adjustment
}

/// For `slice::sort_by`: priority ascending, finite score descending, ID ascending.
/// Invalid scores sort last in their layer instead of poisoning comparisons.
pub fn compare_insights(left: &Insight, right: &Insight) -> Ordering {
    left.priority
        .cmp(&right.priority)
        .then_with(|| {
            let left = if left.rank_score.is_finite() {
                left.rank_score
            } else {
                f32::NEG_INFINITY
            };
            let right = if right.rank_score.is_finite() {
                right.rank_score
            } else {
                f32::NEG_INFINITY
            };
            right.partial_cmp(&left).unwrap_or(Ordering::Equal)
        })
        .then_with(|| left.id.cmp(&right.id))
}

#[cfg(test)]
mod tests {
    use chat_tldr_core::{Granularity, NormalizedBy, TemporalRelation};
    use chrono::{Duration, NaiveTime};

    use super::*;

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-09-26T21:00:00+08:00").unwrap()
    }

    fn deadline(days: i64, time: Option<NaiveTime>) -> TemporalConstraint {
        TemporalConstraint {
            raw: "合成截止日期".into(),
            relation: TemporalRelation::Before,
            bound_date: Some((now() + Duration::days(days)).date_naive()),
            bound_time: time,
            granularity: Granularity::Day,
            anchor: now(),
            normalized_by: NormalizedBy::Rule,
            confidence: 1.0,
        }
    }

    #[test]
    fn hard_layers_cover_assignees_mentions_deadlines_and_summary_exception() {
        for kind in [
            InsightKind::MentionMe,
            InsightKind::Todo,
            InsightKind::Announcement,
            InsightKind::Decision,
        ] {
            assert_eq!(
                priority(kind, Assignee::Other, true, None, None),
                Priority::P0
            );
        }
        assert_eq!(
            priority(InsightKind::Todo, Assignee::Me, false, None, None),
            Priority::P0
        );
        assert_eq!(
            priority(InsightKind::Todo, Assignee::All, false, None, None),
            Priority::P0
        );
        assert_eq!(
            priority(InsightKind::Todo, Assignee::Other, false, None, None),
            Priority::P2
        );
        assert_eq!(
            priority(InsightKind::Todo, Assignee::Unknown, false, None, None),
            Priority::P1
        );
        assert_eq!(
            priority(InsightKind::MentionMe, Assignee::Me, false, Some(0.5), None),
            Priority::P0
        );
        assert_eq!(
            priority(
                InsightKind::MentionMe,
                Assignee::Me,
                false,
                Some(0.49),
                None
            ),
            Priority::P1
        );
        assert_eq!(
            priority(
                InsightKind::Announcement,
                Assignee::Other,
                false,
                None,
                None
            ),
            Priority::P1
        );
        assert_eq!(
            priority(InsightKind::Decision, Assignee::Other, false, None, None),
            Priority::P1
        );
        assert_eq!(
            priority(
                InsightKind::TopicSummary,
                Assignee::Me,
                true,
                Some(1.0),
                Some(0.49)
            ),
            Priority::P2
        );
        assert_eq!(
            priority(
                InsightKind::TopicSummary,
                Assignee::Me,
                true,
                Some(1.0),
                Some(0.5)
            ),
            Priority::P3
        );
    }

    #[test]
    fn invalid_model_numbers_do_not_trigger_priority_or_infinite_scores() {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
            assert_eq!(
                priority(
                    InsightKind::MentionMe,
                    Assignee::Me,
                    false,
                    Some(invalid),
                    None
                ),
                Priority::P1
            );
            assert_eq!(
                priority(
                    InsightKind::TopicSummary,
                    Assignee::Me,
                    false,
                    None,
                    Some(invalid)
                ),
                Priority::P2
            );
            let score = rank_prior(None, Some(invalid), Some(f32::NAN), now(), now());
            assert!((score - 0.7).abs() < 1e-6);
        }
        assert_eq!(
            priority(InsightKind::Unknown, Assignee::Unknown, false, None, None),
            Priority::Unknown
        );
    }

    #[test]
    fn rank_formula_handles_exact_times_date_only_and_expiry_in_anchor_offset() {
        let tomorrow = deadline(1, None);
        let today = deadline(0, None);
        let yesterday = deadline(-1, None);
        assert!(
            (rank_prior(Some(&tomorrow), Some(1.0), Some(2.0), now(), now()) - 2.1).abs() < 1e-6
        );
        assert!((rank_prior(Some(&today), Some(1.0), Some(2.0), now(), now()) - 2.6).abs() < 1e-6);
        assert!((rank_prior(Some(&yesterday), None, None, now(), now()) - 0.7).abs() < 1e-6);
        let expired_hour = deadline(0, NaiveTime::from_hms_opt(20, 59, 59));
        assert!((rank_prior(Some(&expired_hour), None, None, now(), now()) - 0.7).abs() < 1e-6);
        let exactly_now = deadline(0, Some(now().time()));
        assert!((rank_prior(Some(&exactly_now), None, None, now(), now()) - 1.7).abs() < 1e-6);
        // UTC is still the preceding date, but this deadline is already expired in +08.
        let tomorrow_early = DateTime::parse_from_rfc3339("2026-09-27T00:01:00+08:00").unwrap();
        assert!(
            (rank_prior(Some(&today), None, None, tomorrow_early, tomorrow_early) - 0.7).abs()
                < 1e-6
        );
        // Future evidence timestamps are not rewarded beyond maximum recency.
        assert!(
            (rank_prior(None, None, None, now() + Duration::days(1), now()) - 0.7).abs() < 1e-6
        );
        let old_score = rank_prior(None, None, None, now() - Duration::days(1), now());
        assert!((old_score - (0.4 + 0.3 / std::f32::consts::E)).abs() < 1e-6);
    }

    #[test]
    fn feedback_remains_finite_and_bounded_without_changing_hard_layer() {
        for prior in [-1.0, 0.0, 2.6] {
            for weights in [
                vec![],
                vec![10.0; 8],
                vec![-10.0; 8],
                vec![f32::NAN, f32::INFINITY, 0.5],
                vec![0.5, -0.5],
            ] {
                let score = apply_feedback(prior, &weights);
                assert!(score.is_finite());
                assert!((score - prior).abs() <= 0.500_001);
            }
        }
        assert_eq!(apply_feedback(2.0, &[0.5, -0.5]), 2.0);
        assert_eq!(apply_feedback(2.0, &[0.5, 0.0]), 2.25);
        assert!(apply_feedback(f32::NAN, &[f32::NAN]).is_finite());
        assert_eq!(
            priority(InsightKind::Todo, Assignee::Other, true, None, None),
            Priority::P0
        );
    }

    #[test]
    fn sorting_keeps_priority_above_scores_and_breaks_ties_by_id() {
        use chat_tldr_core::{CliEvent, EventBody};

        let mut first = include_str!("../../../../fixtures/jsonl/inbox.jsonl")
            .lines()
            .find_map(|line| {
                let event: CliEvent = serde_json::from_str(line).unwrap();
                if let EventBody::Insight(payload) = event.body {
                    Some(payload.insight)
                } else {
                    None
                }
            })
            .unwrap();
        first.id = "i_a".into();
        first.priority = Priority::P0;
        first.rank_score = -100.0;
        let mut second = first.clone();
        second.id = "i_b".into();
        second.priority = Priority::P1;
        second.rank_score = 100.0;
        assert_eq!(compare_insights(&first, &second), Ordering::Less);
        second.priority = Priority::P0;
        assert_eq!(compare_insights(&first, &second), Ordering::Greater);
        second.rank_score = f32::NAN;
        assert_eq!(compare_insights(&first, &second), Ordering::Less);
        first.rank_score = 0.0;
        second.rank_score = -0.0;
        assert_eq!(compare_insights(&first, &second), Ordering::Less);
        assert_eq!(compare_insights(&second, &first), Ordering::Greater);
    }
}
