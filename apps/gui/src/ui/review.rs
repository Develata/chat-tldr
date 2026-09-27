//! Display coverage for one inbox snapshot, including clipped and paged rows.
use std::collections::HashMap;

use eframe::egui::{Rect, Vec2};

#[derive(Default)]
pub(super) struct ReviewCoverage {
    seen: Vec<bool>,
    remaining: usize,
    partial: HashMap<usize, PartialRow>,
}

struct PartialRow {
    size: Vec2,
    spans: Vec<(f32, f32)>,
}

impl ReviewCoverage {
    pub(super) fn reset(&mut self, rows: usize) {
        self.seen.clear();
        self.seen.resize(rows, false);
        self.remaining = rows;
        self.partial.clear();
    }

    pub(super) fn complete(&self) -> bool {
        self.remaining == 0
    }

    pub(super) fn remaining(&self) -> usize {
        self.remaining
    }

    pub(super) fn seen(&self, row: usize) -> bool {
        self.seen.get(row).copied().unwrap_or(false)
    }

    pub(super) fn observe(&mut self, row: usize, rect: Rect, viewport: Rect) {
        if self.seen.get(row).copied().unwrap_or(true) || !rect.is_positive() {
            return;
        }
        let visible = rect.intersect(viewport);
        // Horizontal clipping cannot count as displaying this row either.
        if !visible.is_positive() || visible.width() + 1.0 < rect.width() {
            return;
        }
        let partial = self.partial.entry(row).or_insert_with(|| PartialRow {
            size: rect.size(),
            spans: Vec::new(),
        });
        if (partial.size - rect.size()).length_sq() > 1.0 {
            partial.size = rect.size();
            partial.spans.clear();
        }
        partial
            .spans
            .push((visible.top() - rect.top(), visible.bottom() - rect.top()));
        partial.spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32)> = Vec::with_capacity(partial.spans.len());
        for &(start, end) in &partial.spans {
            if let Some(previous) = merged.last_mut()
                && start <= previous.1 + 1.0
            {
                previous.1 = previous.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        if merged.len() == 1 && merged[0].0 <= 1.0 && merged[0].1 + 1.0 >= rect.height() {
            self.seen[row] = true;
            self.remaining -= 1;
            self.partial.remove(&row);
        } else {
            partial.spans = merged;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    #[test]
    fn clipped_rows_require_full_coverage_without_skipped_middle() {
        let mut review = ReviewCoverage::default();
        review.reset(1);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 500.0));
        for (top, bottom) in [(0.0, 100.0), (400.0, 500.0)] {
            review.observe(
                0,
                rect,
                Rect::from_min_max(pos2(0.0, top), pos2(100.0, bottom)),
            );
            assert!(!review.complete());
        }
        review.observe(
            0,
            rect,
            Rect::from_min_max(pos2(0.0, 100.0), pos2(100.0, 400.0)),
        );
        assert!(review.complete());
        review.reset(1);
        assert!(!review.complete());
    }

    #[test]
    fn layout_changes_reset_partial_coverage_and_horizontal_clipping_is_not_seen() {
        let mut review = ReviewCoverage::default();
        review.reset(1);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 500.0));
        review.observe(
            0,
            rect,
            Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 250.0)),
        );
        let wider = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 500.0));
        review.observe(
            0,
            wider,
            Rect::from_min_size(pos2(0.0, 250.0), vec2(200.0, 250.0)),
        );
        assert!(!review.complete());
        review.observe(0, wider, rect);
        assert!(!review.complete());
        review.observe(0, wider, wider);
        assert!(review.complete());
    }
}
