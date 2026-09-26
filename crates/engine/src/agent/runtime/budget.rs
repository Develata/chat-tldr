//! Synchronous requests reserve an upper estimate, then settle reported usage.
use crate::{EngineError, Result, llm::Usage};

#[derive(Default)]
pub(in crate::agent) struct Budget {
    spent: f64,
    pending: f64,
}

impl Budget {
    pub(super) fn reserve(&mut self, estimate: f64, limit: f64) -> Result<()> {
        // An unknown outcome retains its estimate; a later request cannot spend it twice.
        if self.spent + self.pending + estimate > limit {
            return Err(EngineError::BudgetExceeded);
        }
        self.spent += self.pending;
        self.pending = estimate;
        Ok(())
    }

    pub(super) fn settle(&mut self, usage: &Usage) {
        if usage.input_tokens > 0 || usage.output_tokens > 0 || usage.cost_usd > 0.0 {
            self.spent += usage.cost_usd;
            self.pending = 0.0;
        }
        // Missing usage is not evidence of a free request (including transport errors).
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_usage_releases_each_completed_requests_estimate() {
        let mut budget = Budget::default();
        for _ in 0..20 {
            budget.reserve(0.10, 0.50).unwrap();
            budget.settle(&Usage {
                input_tokens: 100,
                output_tokens: 10,
                cost_usd: 0.001,
            });
        }
        assert!((budget.spent - 0.02).abs() < 1e-12);
        assert_eq!(budget.pending, 0.0);
    }

    #[test]
    fn unknown_usage_keeps_reservation_and_refused_request_changes_nothing() {
        let mut budget = Budget::default();
        budget.reserve(0.3, 0.5).unwrap();
        budget.settle(&Usage::default());
        assert!(budget.reserve(0.3, 0.5).is_err());
        budget.reserve(0.1, 0.5).unwrap();
        budget.settle(&Usage {
            input_tokens: 20,
            output_tokens: 0,
            cost_usd: 0.01,
        });
        assert!((budget.spent - 0.31).abs() < 1e-12);
        assert!(budget.reserve(0.2, 0.5).is_err());
    }

    #[test]
    fn reported_cost_above_estimate_blocks_next_call_and_known_free_usage_settles() {
        let mut budget = Budget::default();
        budget.reserve(0.1, 0.5).unwrap();
        budget.settle(&Usage {
            input_tokens: 1,
            output_tokens: 0,
            cost_usd: 0.6,
        });
        assert!(budget.reserve(0.0, 0.5).is_err());
        let mut free = Budget::default();
        free.reserve(0.5, 0.5).unwrap();
        free.settle(&Usage {
            input_tokens: 1,
            output_tokens: 0,
            cost_usd: 0.0,
        });
        free.reserve(0.5, 0.5).unwrap();
    }
}
