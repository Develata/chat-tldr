use super::metrics::Group;
use plotters::prelude::*;
use std::path::Path;

pub(super) fn draw(groups: &[Group], path: &Path) -> Result<(), String> {
    let root = SVGBackend::new(path, (1000, 700)).into_drawing_area();
    let fail = |_| "cannot render calibration plot".to_owned();
    root.fill(&WHITE).map_err(fail)?;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            if groups.is_empty() {
                "Reliability: unavailable (no labelled answers)"
            } else {
                "Reliability by model and question type"
            },
            ("sans-serif", 24),
        )
        .margin(30)
        .x_label_area_size(45)
        .y_label_area_size(55)
        .build_cartesian_2d(0.0_f64..1.0, 0.0_f64..1.0)
        .map_err(fail)?;
    chart
        .configure_mesh()
        .x_desc("Mean predicted probability")
        .y_desc("Observed positive / correct rate")
        .x_labels(11)
        .y_labels(11)
        .draw()
        .map_err(fail)?;
    chart
        .draw_series(LineSeries::new([(0.0, 0.0), (1.0, 1.0)], &BLACK.mix(0.35)))
        .map_err(fail)?;
    for (index, group) in groups.iter().enumerate() {
        let color = Palette99::pick(index).to_rgba();
        let points: Vec<_> = group
            .bins
            .iter()
            .filter_map(|bin| bin.mean_probability.zip(bin.observed_rate))
            .collect();
        chart
            .draw_series(LineSeries::new(
                points.iter().copied(),
                color.stroke_width(2),
            ))
            .map_err(fail)?
            .label(format!(
                "{} / {} (n={}, ECE={:.3})",
                group.model, group.task, group.count, group.ece
            ))
            .legend(move |(x, y)| PathElement::new([(x, y), (x + 20, y)], color.stroke_width(2)));
        chart
            .draw_series(
                points
                    .into_iter()
                    .map(|point| Circle::new(point, 4, color.filled())),
            )
            .map_err(fail)?;
    }
    chart
        .configure_series_labels()
        .background_style(WHITE.mix(0.85))
        .border_style(BLACK)
        .draw()
        .map_err(fail)?;
    root.present().map_err(fail)
}
