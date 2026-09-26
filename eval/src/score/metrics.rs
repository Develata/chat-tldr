use std::collections::{HashMap, HashSet};

use chat_tldr_core::{Insight, InsightKind, TemporalConstraint, TemporalRelation};
use serde::Serialize;

use crate::annotation::{GoldDeadline, GoldItem};

#[derive(Debug, Serialize)]
pub struct Metric {
    pub name: String,
    pub value: Option<f64>,
    pub numerator: Option<u64>,
    pub denominator: Option<u64>,
    pub status: String,
}

impl Metric {
    fn ratio(name: String, numerator: u64, denominator: u64) -> Self {
        let value = (denominator != 0).then(|| numerator as f64 / denominator as f64);
        Self {
            name,
            value,
            numerator: Some(numerator),
            denominator: Some(denominator),
            status: status(value).into(),
        }
    }
}

fn status(value: Option<f64>) -> &'static str {
    if value.is_some() { "ok" } else { "undefined" }
}

/// Score already validated items in their recorded inbox order. Filtering
/// rejected predictions, and deciding whether this snapshot is complete, belong
/// to the caller. Matching is shared by extraction, deadlines, and ranking.
pub fn calculate(predictions: &[Insight], gold: &[GoldItem]) -> Vec<Metric> {
    let matched = match_items(predictions, gold);
    let mut metrics = Vec::with_capacity(16);
    for (kind, name) in [(0, "todo"), (1, "announcement"), (2, "decision")] {
        let predicted = predictions
            .iter()
            .filter(|item| prediction_kind(item.kind) == Some(kind))
            .count() as u64;
        let expected = gold
            .iter()
            .filter(|item| gold_kind(&item.kind) == Some(kind))
            .count() as u64;
        let correct = predictions
            .iter()
            .zip(&matched)
            .filter(|(item, target)| prediction_kind(item.kind) == Some(kind) && target.is_some())
            .count() as u64;
        precision_recall_f1(&mut metrics, name, correct, predicted, expected);
    }

    let predicted = predictions
        .iter()
        .filter(|item| item.kind == InsightKind::Todo && item.deadline.is_some())
        .count() as u64;
    let expected = gold
        .iter()
        .filter(|item| item.kind == "todo" && item.deadline.is_some())
        .count() as u64;
    let correct = predictions
        .iter()
        .zip(&matched)
        .filter(|(item, target)| {
            item.kind == InsightKind::Todo
                && item
                    .deadline
                    .as_ref()
                    .zip(target.and_then(|index| gold[index].deadline.as_ref()))
                    .is_some_and(|(predicted, expected)| deadline_matches(predicted, expected))
        })
        .count() as u64;
    precision_recall_f1(&mut metrics, "deadline", correct, predicted, expected);

    // Only four relevance levels exist, so IDCG needs a fixed-size histogram,
    // not an allocation and sort of the full gold set. Missing gold items must
    // still contribute to this ideal ranking and the P0 recall denominator.
    let mut relevance_counts = [0_usize; 4];
    for item in gold {
        relevance_counts[relevance(item)] += 1;
    }
    for cutoff in [5, 10] {
        let dcg: f64 = matched
            .iter()
            .take(cutoff)
            .enumerate()
            .map(|(index, target)| {
                let relevance = target.map_or(0, |target| relevance(&gold[target]));
                discounted_gain(relevance, index)
            })
            .sum();
        let ideal = ideal_dcg(&relevance_counts, cutoff);
        let value = (ideal > 0.0).then(|| dcg / ideal);
        metrics.push(Metric {
            name: format!("ndcg_at_{cutoff}"),
            value,
            numerator: None,
            denominator: None,
            status: status(value).into(),
        });
        let found = matched
            .iter()
            .take(cutoff)
            .filter(|target| target.is_some_and(|index| gold[index].importance == "P0"))
            .count() as u64;
        metrics.push(Metric::ratio(
            format!("p0_recall_at_{cutoff}"),
            found,
            relevance_counts[3] as u64,
        ));
    }
    metrics
}

fn precision_recall_f1(
    metrics: &mut Vec<Metric>,
    name: &str,
    correct: u64,
    predicted: u64,
    expected: u64,
) {
    metrics.push(Metric::ratio(
        format!("{name}_precision"),
        correct,
        predicted,
    ));
    metrics.push(Metric::ratio(format!("{name}_recall"), correct, expected));
    metrics.push(Metric::ratio(
        format!("{name}_f1"),
        2 * correct,
        predicted + expected,
    ));
}

fn prediction_kind(kind: InsightKind) -> Option<u8> {
    match kind {
        InsightKind::Todo => Some(0),
        InsightKind::Announcement => Some(1),
        InsightKind::Decision => Some(2),
        _ => None,
    }
}

fn gold_kind(kind: &str) -> Option<u8> {
    match kind {
        "todo" => Some(0),
        "announcement" => Some(1),
        "decision" => Some(2),
        _ => None,
    }
}

struct Candidate {
    prediction: usize,
    gold: usize,
    overlap: usize,
}

fn match_items(predictions: &[Insight], gold: &[GoldItem]) -> Vec<Option<usize>> {
    let mut by_anchor: HashMap<(u8, &str), Vec<usize>> = HashMap::new();
    for (index, item) in gold.iter().enumerate() {
        let Some(kind) = gold_kind(&item.kind) else {
            continue;
        };
        let mut seen = HashSet::new();
        for anchor in &item.anchors {
            if seen.insert(anchor.as_str()) {
                by_anchor
                    .entry((kind, anchor.as_str()))
                    .or_default()
                    .push(index);
            }
        }
    }

    // Only same-kind pairs sharing an anchor become candidates. Cost is linear
    // in anchors/evidence and their postings, then O(C log C) for C candidates;
    // disjoint items never incur a full predictions x gold scan. Each message
    // contributes once even if a prediction quotes it multiple times.
    let mut candidates = Vec::new();
    let mut overlaps: HashMap<usize, usize> = HashMap::new();
    let mut evidence = HashSet::new();
    for (prediction, item) in predictions.iter().enumerate() {
        let Some(kind) = prediction_kind(item.kind) else {
            continue;
        };
        overlaps.clear();
        evidence.clear();
        for quote in &item.evidence {
            let id = quote.message_id.as_ref();
            if evidence.insert(id)
                && let Some(targets) = by_anchor.get(&(kind, id))
            {
                for &target in targets {
                    *overlaps.entry(target).or_default() += 1;
                }
            }
        }
        candidates.extend(overlaps.iter().map(|(&target, &overlap)| Candidate {
            prediction,
            gold: target,
            overlap,
        }));
    }
    candidates.sort_unstable_by(|left, right| {
        right
            .overlap
            .cmp(&left.overlap)
            .then_with(|| left.prediction.cmp(&right.prediction))
            .then_with(|| gold[left.gold].item_id.cmp(&gold[right.gold].item_id))
            .then_with(|| left.gold.cmp(&right.gold))
    });
    let mut matched = vec![None; predictions.len()];
    let mut used = vec![false; gold.len()];
    for candidate in candidates {
        if matched[candidate.prediction].is_none() && !used[candidate.gold] {
            matched[candidate.prediction] = Some(candidate.gold);
            used[candidate.gold] = true;
        }
    }
    matched
}

fn deadline_matches(predicted: &TemporalConstraint, expected: &GoldDeadline) -> bool {
    match (&expected.relation, &expected.bound_date) {
        (Some(relation), Some(date)) => {
            let relation_matches = matches!(
                (predicted.relation, relation.as_str()),
                (TemporalRelation::Before, "before")
                    | (TemporalRelation::At, "at")
                    | (TemporalRelation::After, "after")
            );
            relation_matches
                && predicted.bound_date.is_some_and(|predicted| {
                    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") == Ok(predicted)
                })
        }
        (None, None) => predicted.raw == expected.raw,
        _ => false,
    }
}

fn relevance(item: &GoldItem) -> usize {
    match item.importance.as_str() {
        "P0" => 3,
        "P1" => 2,
        "P2" => 1,
        _ => 0,
    }
}

fn discounted_gain(relevance: usize, index: usize) -> f64 {
    ((1_u32 << relevance) - 1) as f64 / ((index + 2) as f64).log2()
}

fn ideal_dcg(relevance_counts: &[usize; 4], cutoff: usize) -> f64 {
    let mut index = 0;
    let mut result = 0.0;
    for relevance in (1..=3).rev() {
        for _ in 0..relevance_counts[relevance].min(cutoff - index) {
            result += discounted_gain(relevance, index);
            index += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chat_tldr_core::{
        Assignee, Evidence, Granularity, Lifecycle, NormalizedBy, Priority, VerificationStatus,
    };

    fn prediction(kind: InsightKind, anchors: &[&str]) -> Insight {
        Insight {
            id: "i_test".into(),
            chat_id: "qq:group:synthetic".into(),
            kind,
            title: "synthetic".into(),
            summary: String::new(),
            priority: Priority::P0,
            rank_score: 1.0,
            confidence: None,
            assignee: Assignee::Unknown,
            deadline: None,
            evidence: anchors
                .iter()
                .map(|anchor| Evidence {
                    message_id: (*anchor).into(),
                    quote: "synthetic".into(),
                    render_profile: Default::default(),
                })
                .collect(),
            topic_id: None,
            verification_status: VerificationStatus::Verified,
            lifecycle: Lifecycle::Open,
            created_in_run: "r_test".into(),
            updated_at: "2026-09-26T00:00:00-03:00".parse().unwrap(),
        }
    }

    fn gold(id: &str, kind: &str, anchors: &[&str], importance: &str) -> GoldItem {
        GoldItem {
            item_id: id.into(),
            kind: kind.into(),
            assignee: "unknown".into(),
            anchors: anchors.iter().map(|anchor| (*anchor).into()).collect(),
            deadline: None,
            importance: importance.into(),
        }
    }

    fn metric<'a>(metrics: &'a [Metric], name: &str) -> &'a Metric {
        metrics.iter().find(|metric| metric.name == name).unwrap()
    }

    fn value(metrics: &[Metric], name: &str, expected: f64) {
        let metric = metric(metrics, name);
        assert_eq!(metric.status, "ok");
        assert!((metric.value.unwrap() - expected).abs() < 1e-12);
    }

    #[test]
    fn nonperfect_extraction_counts_each_kind_and_preserves_rank_positions() {
        let predictions = [
            prediction(InsightKind::TopicSummary, &["a"]),
            prediction(InsightKind::Todo, &["a"]),
            prediction(InsightKind::Todo, &["missing"]),
            prediction(InsightKind::Announcement, &["b"]),
            prediction(InsightKind::Decision, &["c"]),
        ];
        let gold = [
            gold("g1", "todo", &["a"], "P0"),
            gold("g2", "todo", &["d"], "P1"),
            gold("g3", "todo", &["e"], "P0"),
            gold("g4", "announcement", &["b"], "P1"),
            gold("g5", "decision", &["c"], "P2"),
        ];
        let metrics = calculate(&predictions, &gold);
        value(&metrics, "todo_precision", 0.5);
        value(&metrics, "todo_recall", 1.0 / 3.0);
        value(&metrics, "todo_f1", 0.4);
        value(&metrics, "announcement_f1", 1.0);
        value(&metrics, "decision_f1", 1.0);
        value(&metrics, "p0_recall_at_5", 0.5);
        // The unmatched P0 and P1 are included in IDCG. The summary consumes
        // rank 1 despite having zero relevance, so the matched P0 is at rank 2.
        let dcg = 7.0 / 3_f64.log2() + 3.0 / 5_f64.log2() + 1.0 / 6_f64.log2();
        let ideal =
            7.0 + 7.0 / 3_f64.log2() + 3.0 / 4_f64.log2() + 3.0 / 5_f64.log2() + 1.0 / 6_f64.log2();
        value(&metrics, "ndcg_at_5", dcg / ideal);
    }

    #[test]
    fn intersection_size_takes_precedence_over_prediction_order() {
        let predictions = [
            prediction(InsightKind::Todo, &["a"]),
            prediction(InsightKind::Todo, &["a", "b"]),
        ];
        let gold = [gold("g1", "todo", &["a", "b"], "P0")];
        assert_eq!(match_items(&predictions, &gold), [None, Some(0)]);
        let metrics = calculate(&predictions, &gold);
        value(&metrics, "todo_precision", 0.5);
        value(&metrics, "todo_recall", 1.0);
        value(&metrics, "ndcg_at_5", 1.0 / 3_f64.log2());
    }

    #[test]
    fn ties_use_prediction_order_then_gold_id_and_do_not_reuse_matches() {
        let predictions = [
            prediction(InsightKind::Todo, &["a"]),
            prediction(InsightKind::Todo, &["a"]),
            prediction(InsightKind::Todo, &["a"]),
        ];
        let gold = [
            gold("z", "todo", &["a"], "P1"),
            gold("a", "todo", &["a"], "P0"),
        ];
        assert_eq!(match_items(&predictions, &gold), [Some(1), Some(0), None]);
        let metrics = calculate(&predictions, &gold);
        value(&metrics, "todo_f1", 0.8);
        value(&metrics, "ndcg_at_5", 1.0);
        value(&metrics, "p0_recall_at_5", 1.0);
    }

    #[test]
    fn duplicate_evidence_is_set_intersection_and_kind_must_match() {
        let predictions = [
            prediction(InsightKind::Todo, &["a", "a", "a"]),
            prediction(InsightKind::Todo, &["a", "b"]),
            prediction(InsightKind::Decision, &["c"]),
            prediction(InsightKind::MentionMe, &["c"]),
        ];
        let gold = [
            gold("g1", "todo", &["a", "b", "b"], "P0"),
            gold("g2", "announcement", &["c"], "P1"),
        ];
        assert_eq!(
            match_items(&predictions, &gold),
            [None, Some(0), None, None]
        );
        let metrics = calculate(&predictions, &gold);
        value(&metrics, "decision_precision", 0.0);
        value(&metrics, "announcement_recall", 0.0);
    }

    fn deadline(raw: &str, relation: TemporalRelation, date: Option<&str>) -> TemporalConstraint {
        TemporalConstraint {
            raw: raw.into(),
            relation,
            bound_date: date.map(|date| date.parse().unwrap()),
            bound_time: None,
            granularity: Granularity::Day,
            anchor: "2026-09-26T00:00:00-03:00".parse().unwrap(),
            normalized_by: NormalizedBy::Rule,
            confidence: 1.0,
        }
    }

    #[test]
    fn deadlines_require_matching_todo_date_and_relation_or_unnormalized_raw() {
        let mut predictions: Vec<_> = ["a", "b", "c", "d", "e", "extra"]
            .map(|anchor| prediction(InsightKind::Todo, &[anchor]))
            .into();
        for item in &mut predictions {
            item.deadline = Some(deadline(
                "Friday",
                TemporalRelation::Before,
                Some("2026-09-25"),
            ));
        }
        predictions[1].deadline.as_mut().unwrap().bound_date = Some("2026-10-02".parse().unwrap());
        predictions[2].deadline.as_mut().unwrap().relation = TemporalRelation::At;
        predictions[3].deadline = Some(deadline("soon", TemporalRelation::Unknown, None));
        predictions[4].deadline = None;
        let mut gold: Vec<_> = ["a", "b", "c", "d", "e", "missing"]
            .map(|anchor| gold(anchor, "todo", &[anchor], "P0"))
            .into();
        for item in &mut gold {
            item.deadline = Some(GoldDeadline {
                raw: "Friday".into(),
                relation: Some("before".into()),
                bound_date: Some("2026-09-25".into()),
            });
        }
        gold[3].deadline = Some(GoldDeadline {
            raw: "soon".into(),
            relation: None,
            bound_date: None,
        });
        let metrics = calculate(&predictions, &gold);
        value(&metrics, "todo_f1", 5.0 / 6.0);
        value(&metrics, "deadline_precision", 2.0 / 5.0);
        value(&metrics, "deadline_recall", 2.0 / 6.0);
        value(&metrics, "deadline_f1", 4.0 / 11.0);
        assert_eq!(metric(&metrics, "deadline_f1").numerator, Some(4));
        assert_eq!(metric(&metrics, "deadline_f1").denominator, Some(11));
        predictions[3].deadline.as_mut().unwrap().raw = "later".into();
        value(
            &calculate(&predictions, &gold),
            "deadline_recall",
            1.0 / 6.0,
        );
    }

    #[test]
    fn deadline_score_excludes_decisions_and_requires_an_item_match() {
        let mut predicted = prediction(InsightKind::Decision, &["a"]);
        predicted.deadline = Some(deadline(
            "Friday",
            TemporalRelation::Before,
            Some("2026-09-25"),
        ));
        let mut expected = gold("g1", "decision", &["a"], "P0");
        expected.deadline = Some(GoldDeadline {
            raw: "Friday".into(),
            relation: Some("before".into()),
            bound_date: Some("2026-09-25".into()),
        });
        let metrics = calculate(&[predicted.clone()], &[expected]);
        assert_eq!(metric(&metrics, "deadline_f1").value, None);
        predicted.kind = InsightKind::Todo;
        let mut expected = gold("g2", "todo", &["other"], "P0");
        expected.deadline = Some(GoldDeadline {
            raw: "Friday".into(),
            relation: None,
            bound_date: None,
        });
        value(&calculate(&[predicted], &[expected]), "deadline_f1", 0.0);
    }

    #[test]
    fn cutoffs_keep_all_predictions_in_recorded_order() {
        let mut predictions = vec![prediction(InsightKind::MentionMe, &["a"]); 5];
        predictions.push(prediction(InsightKind::Todo, &["a"]));
        let gold = [gold("g1", "todo", &["a"], "P0")];
        let metrics = calculate(&predictions, &gold);
        value(&metrics, "p0_recall_at_5", 0.0);
        value(&metrics, "p0_recall_at_10", 1.0);
        value(&metrics, "ndcg_at_5", 0.0);
        value(&metrics, "ndcg_at_10", 1.0 / 7_f64.log2());
    }

    #[test]
    fn zero_denominators_are_undefined_not_perfect_scores() {
        let metrics = calculate(&[], &[]);
        assert_eq!(metrics.len(), 16);
        assert!(
            metrics
                .iter()
                .all(|metric| metric.value.is_none() && metric.status == "undefined")
        );
        let gold = [gold("g1", "todo", &["a"], "P3")];
        let metrics = calculate(&[], &gold);
        assert_eq!(metric(&metrics, "todo_precision").value, None);
        value(&metrics, "todo_recall", 0.0);
        value(&metrics, "todo_f1", 0.0);
        assert_eq!(metric(&metrics, "ndcg_at_5").value, None);
        assert_eq!(metric(&metrics, "p0_recall_at_5").value, None);
        let metrics = calculate(&[prediction(InsightKind::Todo, &["a"])], &[]);
        value(&metrics, "todo_precision", 0.0);
        assert_eq!(metric(&metrics, "todo_recall").value, None);
        value(&metrics, "todo_f1", 0.0);
    }
}
