//! The six σ rows: the screen this application exists for.
//!
//! Each row is one degree of freedom, and its bar is the spread the
//! geometry leaves in that direction — `σ_noise / σ'ᵢ`, in metres, on a
//! logarithmic axis because in a real scene the six values span orders of
//! magnitude.
//!
//! **The tolerance is a line, not a label.** The core classifies a
//! direction by comparing its spread against the required accuracy and
//! again against ten times it; drawing both thresholds as vertical lines
//! makes the classification something you read off the picture rather than
//! something the application asserts. A bar that crosses the first line is
//! the finding. Colour repeats what position already says, which is the
//! point — nothing here depends on being able to tell teal from amber.
//!
//! Dragging the noise or tolerance slider moves the lines and recolours the
//! bars at once, with no work behind it: `Conditioning` holds the spectrum,
//! and both `uncertainty` and `classify` are pure functions of six stored
//! numbers. That is what makes "is five millimetres acceptable here?" a
//! question you answer by looking instead of by re-running anything.

use eframe::egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, Vec2, pos2};
use rigidity_core::observability::{Conditioning, Observability, ObservabilityCriteria};

use crate::theme::Palette;

const NAMES: [&str; 6] = ["σ₁", "σ₂", "σ₃", "σ₄", "σ₅", "σ₆"];
/// Height of one row.
const ROW: f32 = 20.0;
/// Width reserved for the σ name.
const LABEL: f32 = 20.0;
/// Width reserved for the printed spread.
const VALUE: f32 = 54.0;
/// Height of the strip that carries the threshold captions.
const HEADER: f32 = 13.0;
/// Height of the strip that carries the decade labels.
const AXIS: f32 = 13.0;

/// Draws the block and returns the row under the cursor.
pub(crate) fn show(
    ui: &mut Ui,
    palette: &Palette,
    conditioning: &Conditioning,
    criteria: &ObservabilityCriteria,
    trusted: bool,
) -> Option<usize> {
    let spreads = conditioning.uncertainty(criteria.noise_sigma);
    let states = conditioning.classify(criteria);

    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, HEADER + ROW * 6.0 + AXIS), Sense::hover());
    let painter = ui.painter();

    let bars = Rect::from_min_max(
        pos2(rect.left() + LABEL, rect.top() + HEADER),
        pos2(rect.right() - VALUE, rect.top() + HEADER + ROW * 6.0),
    );

    // The axis holds every spread and both thresholds, with a decade of air
    // at each end so that a bar never starts or ends exactly on the frame.
    let (low, high) = extent(&spreads, criteria.tolerance);
    let (log_low, log_high) = (low.log10(), high.log10());
    let position = |value: f64| {
        let clamped = value.clamp(low, high).log10();
        bars.left() + bars.width() * ((clamped - log_low) / (log_high - log_low)) as f32
    };

    // Decades, thinned out until their labels stop colliding.
    let first = log_low.ceil() as i32;
    let last = log_high.floor() as i32;
    let step = (((last - first + 1) as f32 / 5.0).ceil() as i32).max(1);
    let mut decade = first;
    while decade <= last {
        let x = position(10f64.powi(decade));
        painter.line_segment(
            [pos2(x, bars.top()), pos2(x, bars.bottom())],
            Stroke::new(1.0, palette.line),
        );
        painter.text(
            pos2(x, bars.bottom() + 2.0),
            Align2::CENTER_TOP,
            format!("1e{decade}"),
            FontId::monospace(9.0),
            palette.faint,
        );
        decade += step;
    }

    // The two thresholds the core actually classifies against.
    threshold(
        painter,
        &bars,
        position(criteria.tolerance),
        palette.accent,
        "tolerance",
        palette,
    );
    threshold(
        painter,
        &bars,
        position(criteria.tolerance * 10.0),
        palette.faint,
        "×10",
        palette,
    );

    let hovered = response.hover_pos().and_then(|at| {
        let row = ((at.y - bars.top()) / ROW).floor();
        (bars.y_range().contains(at.y) && (0.0..6.0).contains(&row)).then_some(row as usize)
    });

    for index in 0..6 {
        let middle = bars.top() + ROW * (index as f32 + 0.5);
        let colour = colour_of(palette, states[index]);
        // A spectrum computed at a pose that may be the wrong minimum
        // is drawn faint. Conditioning describes the shape of the cost
        // function around wherever the solver stopped; it has nothing to
        // say about whether that was the right place, and a confident
        // picture would be a lie of exactly the kind this project exists
        // to prevent.
        let emphasis = match (trusted, hovered == Some(index)) {
            (false, _) => 0.35,
            (true, true) => 1.0,
            (true, false) => 0.85,
        };

        painter.text(
            pos2(rect.left(), middle),
            Align2::LEFT_CENTER,
            NAMES[index],
            FontId::proportional(11.0),
            if hovered == Some(index) {
                palette.text
            } else {
                palette.muted
            },
        );
        let end = position(spreads[index]).max(bars.left() + 2.0);
        painter.rect_filled(
            Rect::from_min_max(pos2(bars.left(), middle - 3.0), pos2(end, middle + 3.0)),
            CornerRadius::same(1),
            colour.gamma_multiply(emphasis),
        );
        painter.text(
            pos2(rect.right(), middle),
            Align2::RIGHT_CENTER,
            metres(spreads[index]),
            FontId::monospace(10.0),
            colour.gamma_multiply(emphasis),
        );
    }

    hovered
}

/// One line down the block, captioned above it.
fn threshold(
    painter: &eframe::egui::Painter,
    bars: &Rect,
    x: f32,
    colour: Color32,
    caption: &str,
    palette: &Palette,
) {
    painter.line_segment(
        [pos2(x, bars.top()), pos2(x, bars.bottom())],
        Stroke::new(1.0, colour),
    );
    painter.text(
        pos2(x, bars.top() - 2.0),
        Align2::CENTER_BOTTOM,
        caption,
        FontId::proportional(9.0),
        if caption == "×10" {
            palette.faint
        } else {
            colour
        },
    );
}

/// The colour of a classification.
fn colour_of(palette: &Palette, state: Observability) -> Color32 {
    match state {
        Observability::High => palette.high,
        Observability::Medium => palette.medium,
        Observability::Low => palette.low,
    }
}

/// The axis range: every finite spread and both thresholds, plus a decade
/// of air at each end.
///
/// A direction the geometry does not constrain at all has an infinite
/// spread. It is not an error and not a special case in the mathematics —
/// it is the answer — so it simply runs off the end of the axis.
fn extent(spreads: &[f64; 6], tolerance: f64) -> (f64, f64) {
    let finite = spreads
        .iter()
        .copied()
        .filter(|s| s.is_finite() && *s > 0.0);
    let mut low = tolerance;
    let mut high = tolerance * 10.0;
    for spread in finite {
        low = low.min(spread);
        high = high.max(spread);
    }
    // Half a decade of air, not a whole one: `high` already stands off
    // from the data because the ×10 threshold is in the running, and a
    // second full decade leaves the bars huddled against the left edge.
    let low = (low / 3.0).max(1e-12);
    let high = (high * 3.0).max(low * 10.0);
    (low, high)
}

/// A spread, short enough for a narrow column.
pub(crate) fn metres(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.1e}")
    } else {
        "∞".to_owned()
    }
}

/// How many directions the geometry fails to pin down well enough, said in
/// words. Position on the axis says it too; a count says it without
/// counting bars.
pub(crate) fn verdict(states: &[Observability; 6]) -> String {
    let lost = states.iter().filter(|s| **s == Observability::Low).count();
    let weak = states
        .iter()
        .filter(|s| **s == Observability::Medium)
        .count();
    match (lost, weak) {
        (0, 0) => "all six directions determined".to_owned(),
        (0, weak) => format!("{weak} of 6 short of the tolerance"),
        (lost, 0) => format!("{lost} of 6 not determined"),
        (lost, weak) => format!("{lost} of 6 not determined, {weak} short"),
    }
}
