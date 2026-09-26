use serde::Serialize;

#[derive(Clone, Copy, Default)]
struct Bin {
    count: u64,
    probability: f64,
    positives: u64,
}

#[derive(Default)]
pub(super) struct Accumulator {
    bins: [Bin; 10],
    squared_error: f64,
}

#[derive(Serialize)]
pub(super) struct Group {
    pub model: String,
    pub task: String,
    pub count: u64,
    pub ece: f64,
    pub binary_brier: f64,
    pub bins: Vec<Bucket>,
}

#[derive(Serialize)]
pub(super) struct Bucket {
    pub lower: f64,
    pub upper: f64,
    pub count: u64,
    pub mean_probability: Option<f64>,
    pub observed_rate: Option<f64>,
}

impl Accumulator {
    pub(super) fn add(&mut self, p: f64, positive: bool) {
        let bin = &mut self.bins[((p * 10.0).floor() as usize).min(9)];
        bin.count += 1;
        bin.probability += p;
        bin.positives += u64::from(positive);
        self.squared_error += (p - f64::from(positive)).powi(2);
    }
    pub(super) fn finish(self, model: String, task: String) -> Group {
        let count = self.bins.iter().map(|bin| bin.count).sum::<u64>();
        let ece = self
            .bins
            .iter()
            .map(|bin| (bin.probability - bin.positives as f64).abs())
            .sum::<f64>()
            / count as f64;
        let bins = self
            .bins
            .into_iter()
            .enumerate()
            .map(|(i, bin)| Bucket {
                lower: i as f64 / 10.0,
                upper: (i + 1) as f64 / 10.0,
                count: bin.count,
                mean_probability: (bin.count > 0).then(|| bin.probability / bin.count as f64),
                observed_rate: (bin.count > 0).then(|| bin.positives as f64 / bin.count as f64),
            })
            .collect();
        Group {
            model,
            task,
            count,
            ece,
            binary_brier: self.squared_error / count as f64,
            bins,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hand_computed_binary_scores_and_endpoint_bins() {
        let mut values = Accumulator::default();
        for (p, y) in [(0.0, false), (0.25, false), (0.75, true), (1.0, true)] {
            values.add(p, y);
        }
        let group = values.finish("synthetic".into(), "todo".into());
        assert_eq!(group.count, 4);
        assert!((group.ece - 0.125).abs() < 1e-12);
        assert!((group.binary_brier - 0.03125).abs() < 1e-12);
        assert_eq!(group.bins[0].count, 1);
        assert_eq!(group.bins[9].count, 1);
        assert_eq!(group.bins[5].observed_rate, None);
    }
    #[test]
    fn ece_uses_empirical_frequency_within_bins() {
        let mut values = Accumulator::default();
        values.add(0.51, true);
        values.add(0.59, false);
        let group = values.finish("synthetic".into(), "todo".into());
        assert!((group.ece - 0.05).abs() < 1e-12);
        assert_eq!(group.bins[5].observed_rate, Some(0.5));
        assert!((group.binary_brier - 0.2941).abs() < 1e-12);
    }
}
