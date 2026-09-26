//! Partition metrics over identical, non-recalled message universes.
use std::collections::BTreeMap;

use super::{metrics::Metric, ratio, unavailable};
use crate::annotation::GoldMessage;
use chat_tldr_core::MessagePayload;

pub(super) fn calculate(messages: &[MessagePayload], labels: &[GoldMessage]) -> Vec<Metric> {
    let assigned = messages.iter().filter(|row| row.topic_id.is_some()).count();
    let mut rows = vec![ratio(
        "thread_assignment_coverage",
        assigned as u64,
        messages.len() as u64,
    )];
    if messages.is_empty() || assigned != messages.len() {
        for name in ["thread_one_to_one", "thread_exact_f1", "ari", "nmi"] {
            rows.push(unavailable(
                name,
                if messages.is_empty() {
                    "undefined"
                } else {
                    "unavailable_missing_assignments"
                },
            ));
        }
        return rows;
    }
    let gold: BTreeMap<_, _> = labels
        .iter()
        .map(|row| (row.message_id.as_str(), row.thread.as_str()))
        .collect();
    let mut predicted = BTreeMap::new();
    let mut truth = BTreeMap::new();
    let pairs: Vec<_> = messages
        .iter()
        .map(|message| {
            let next = predicted.len();
            let p = *predicted
                .entry(message.topic_id.as_ref().expect("checked"))
                .or_insert(next);
            let next = truth.len();
            let g = *truth
                .entry(gold[message.message_id.as_ref()])
                .or_insert(next);
            (p, g)
        })
        .collect();
    let mut cells = BTreeMap::<(usize, usize), u64>::new();
    let mut totals_p = vec![0_u64; predicted.len()];
    let mut totals_g = vec![0_u64; truth.len()];
    for (p, g) in pairs {
        *cells.entry((p, g)).or_default() += 1;
        totals_p[p] += 1;
        totals_g[g] += 1;
    }
    let n = messages.len() as f64;
    let exact = cells
        .iter()
        .filter(|((p, g), count)| **count == totals_p[*p] && **count == totals_g[*g])
        .count() as u64;
    rows.push(ratio(
        "thread_exact_f1",
        2 * exact,
        (predicted.len() + truth.len()) as u64,
    ));
    if predicted.len().max(truth.len()) <= 512 {
        let transpose = predicted.len() > truth.len();
        let (a, b) = if transpose {
            (truth.len(), predicted.len())
        } else {
            (predicted.len(), truth.len())
        };
        let mut weights = vec![vec![0; b]; a];
        for (&(p, g), &count) in &cells {
            if transpose {
                weights[g][p] = count;
            } else {
                weights[p][g] = count;
            }
        }
        rows.push(ratio(
            "thread_one_to_one",
            maximum_overlap(&weights),
            messages.len() as u64,
        ));
    } else {
        rows.push(unavailable(
            "thread_one_to_one",
            "unavailable_topic_limit_512",
        ));
    }
    let choose = |count: u64| count as f64 * count.saturating_sub(1) as f64 / 2.0;
    let intersections: f64 = cells.values().map(|&count| choose(count)).sum();
    let rp: f64 = totals_p.iter().map(|&count| choose(count)).sum();
    let rg: f64 = totals_g.iter().map(|&count| choose(count)).sum();
    let expected = if n < 2.0 {
        0.0
    } else {
        rp * rg / (n * (n - 1.0) / 2.0)
    };
    let denominator = (rp + rg) / 2.0 - expected;
    let ari = if denominator.abs() < 1e-12 {
        1.0
    } else {
        (intersections - expected) / denominator
    };
    let entropy = |totals: &[u64]| {
        totals
            .iter()
            .map(|&count| {
                let p = count as f64 / n;
                -p * p.ln()
            })
            .sum::<f64>()
    };
    let information: f64 = cells
        .iter()
        .map(|(&(p, g), &count)| {
            count as f64 / n * ((count as f64 * n) / (totals_p[p] as f64 * totals_g[g] as f64)).ln()
        })
        .sum();
    let entropy_sum = entropy(&totals_p) + entropy(&totals_g);
    let nmi = if entropy_sum < 1e-12 {
        1.0
    } else {
        2.0 * information / entropy_sum
    };
    for (name, value) in [("ari", ari.clamp(-1.0, 1.0)), ("nmi", nmi.clamp(0.0, 1.0))] {
        rows.push(Metric {
            name: name.into(),
            value: Some(value),
            numerator: None,
            denominator: None,
            status: "ok".into(),
        });
    }
    rows
}

/// Rectangular Hungarian assignment, n <= m: O(n^2 m) time and O(nm) storage.
/// An optimal global assignment matters: greedy overlap can give a lower score.
fn maximum_overlap(weights: &[Vec<u64>]) -> u64 {
    if weights.is_empty() {
        return 0;
    }
    let (n, m) = (weights.len(), weights[0].len());
    let (mut u, mut v, mut p, mut way) = (
        vec![0_i64; n + 1],
        vec![0_i64; m + 1],
        vec![0_usize; m + 1],
        vec![0_usize; m + 1],
    );
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut minima = vec![i64::MAX; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let (mut delta, mut j1) = (i64::MAX, 0);
            for j in 1..=m {
                if !used[j] {
                    let cost = -(weights[i0 - 1][j - 1] as i64) - u[i0] - v[j];
                    if cost < minima[j] {
                        minima[j] = cost;
                        way[j] = j0;
                    }
                    if minima[j] < delta {
                        delta = minima[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minima[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    (1..=m)
        .filter(|&j| p[j] > 0)
        .map(|j| weights[p[j] - 1][j - 1])
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn partition(predicted: &[Option<&str>], gold: &[&str]) -> BTreeMap<String, Metric> {
        let messages: Vec<MessagePayload> = predicted.iter().enumerate().map(|(n,topic)| serde_json::from_value(serde_json::json!({
            "message_id":format!("m_{n:016x}"),"sender":"qq:synthetic","sender_display":"synthetic","sent_at":"2026-09-26T00:00:00Z",
            "display_text":"synthetic","recalled":false,"system":false,"reply_to":null,"mentions_me":false,"topic_id":topic,"burst_id":null,"cursor":format!("1:{}",n+1)
        })).unwrap()).collect();
        let labels: Vec<_> = gold
            .iter()
            .enumerate()
            .map(|(n, thread)| GoldMessage {
                message_id: format!("m_{n:016x}"),
                thread: (*thread).into(),
                todo: false,
                announcement: false,
            })
            .collect();
        calculate(&messages, &labels)
            .into_iter()
            .map(|m| (m.name.clone(), m))
            .collect()
    }
    #[test]
    fn partition_labels_are_invariant_and_discordance_can_have_negative_ari() {
        let exact = partition(
            &[Some("b"), Some("b"), Some("a"), Some("a")],
            &["x", "x", "y", "y"],
        );
        for key in ["thread_one_to_one", "thread_exact_f1", "ari", "nmi"] {
            assert_eq!(exact[key].value, Some(1.0));
        }
        let crossed = partition(
            &[Some("b"), Some("a"), Some("b"), Some("a")],
            &["x", "x", "y", "y"],
        );
        assert!((crossed["ari"].value.unwrap() + 0.5).abs() < 1e-12);
        assert_eq!(crossed["thread_one_to_one"].value, Some(0.5));
        assert_eq!(crossed["thread_exact_f1"].value, Some(0.0));
        assert!(crossed["nmi"].value.unwrap().abs() < 1e-12);
        let missing = partition(&[Some("b"), None], &["x", "x"]);
        assert_eq!(missing["thread_assignment_coverage"].value, Some(0.5));
        assert!(missing["ari"].value.is_none());
        assert!(partition(&[], &[])["nmi"].value.is_none());
    }
    #[test]
    fn optimal_overlap_is_not_greedy() {
        assert_eq!(maximum_overlap(&[vec![4, 3], vec![3, 0]]), 6);
        assert_eq!(maximum_overlap(&[vec![0, 2, 0], vec![3, 0, 1]]), 5);
        assert_eq!(maximum_overlap(&[vec![0, 0], vec![0, 0]]), 0);
    }

    #[test]
    fn hungarian_matches_exhaustive_assignment_for_small_matrices() {
        fn brute(w: &[Vec<u64>], row: usize, used: u32) -> u64 {
            if row == w.len() {
                return 0;
            }
            (0..w[0].len())
                .filter(|j| used & (1 << j) == 0)
                .map(|j| w[row][j] + brute(w, row + 1, used | (1 << j)))
                .max()
                .unwrap()
        }
        for seed in 0..100_u64 {
            let w: Vec<_> = (0..3)
                .map(|i| {
                    (0..4)
                        .map(|j| (seed * (i + 3) * (j + 7) + i * j + 3) % 11)
                        .collect()
                })
                .collect();
            assert_eq!(maximum_overlap(&w), brute(&w, 0, 0));
        }
    }
}
