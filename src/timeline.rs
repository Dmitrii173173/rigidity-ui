//! The iteration strip.
//!
//! It appears only after a registration has run, and it does two jobs at
//! once: it says how the residual came down, and it lets any pose along
//! the way be looked at again. The second is what makes the first useful —
//! a curve that flattens early and a curve that never settles look
//! different, and being able to scrub back to where it still looked right
//! is most of the value of a manual alignment tool for none of its cost.
//!
//! `register_pair_observed` already takes an initial pose, so the scrubbed
//! iteration can also start the next run.

use eframe::egui::{Align2, FontId, Rect, Sense, Stroke, Ui, Vec2, pos2};
use rigidity_core::icp::IterationReport;

use crate::theme::Palette;

/// Height of the strip.
pub(crate) const HEIGHT: f32 = 38.0;

/// Draws the strip and returns the iteration the pointer chose.
pub(crate) fn show(
    ui: &mut Ui,
    palette: &Palette,
    reports: &[IterationReport],
    selected: usize,
) -> Option<usize> {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), HEIGHT),
        Sense::click_and_drag(),
    );
    if reports.is_empty() {
        return None;
    }
    let painter = ui.painter();

    // Text at both ends, plot between them.
    let plot = Rect::from_min_max(
        pos2(rect.left() + 96.0, rect.top() + 6.0),
        pos2(rect.right() - 108.0, rect.bottom() - 6.0),
    );
    let last = reports.len() - 1;
    let selected = selected.min(last);

    painter.text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        // The solver's own counter includes the steps it rejected,
        // while the strip holds only the accepted ones — printing both
        // numbers together read as "iteration 29 / 11".
        format!("step {} of {}", selected + 1, reports.len()),
        FontId::proportional(11.0),
        palette.muted,
    );
    painter.text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        format!("rmse {:.2e} m", reports[selected].rmse),
        FontId::monospace(10.0),
        palette.muted,
    );

    // The residual over a logarithmic axis: on a converging run it falls by
    // orders of magnitude, and a linear axis would show one step and then a
    // flat line along the bottom.
    let finite = |value: f64| value.is_finite() && value > 0.0;
    let logs: Vec<f32> = reports
        .iter()
        .map(|report| {
            if finite(report.rmse) {
                report.rmse.log10() as f32
            } else {
                f32::NAN
            }
        })
        .collect();
    let high = logs
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    let low = logs
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::INFINITY, f32::min);
    let span = (high - low).max(1e-3);

    let x_of = |index: usize| {
        plot.left()
            + plot.width()
                * if last == 0 {
                    0.5
                } else {
                    index as f32 / last as f32
                }
    };
    let y_of = |log: f32| plot.bottom() - plot.height() * ((log - low) / span);

    let curve: Vec<_> = logs
        .iter()
        .enumerate()
        .filter(|(_, log)| log.is_finite())
        .map(|(index, log)| pos2(x_of(index), y_of(*log)))
        .collect();
    if curve.len() > 1 {
        painter.add(eframe::egui::Shape::line(
            curve.clone(),
            Stroke::new(1.0, palette.muted),
        ));
    }

    // The handle: a full-height line rather than a dot, so it can be aimed
    // at anywhere in the strip.
    let x = x_of(selected);
    painter.line_segment(
        [pos2(x, rect.top() + 4.0), pos2(x, rect.bottom() - 4.0)],
        Stroke::new(1.0, palette.accent),
    );
    if let Some(point) = curve.get(selected) {
        painter.circle_filled(*point, 2.5, palette.accent);
    }

    let pointer = response
        .interact_pointer_pos()
        .filter(|_| response.clicked() || response.dragged())?;
    let fraction = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    Some((fraction * last as f32).round() as usize)
}
