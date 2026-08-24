//! The distribution of a scalar field, and the two handles that clamp its
//! ramp.
//!
//! It is here so that a colour scale can be *read* rather than guessed at:
//! a distance field whose values are almost all under a millimetre with
//! one point at half a metre looks, on an unclamped ramp, like a uniformly
//! cold cloud with a single hot speck — and the shape of the histogram is
//! what tells you to pull the handle in.
//!
//! The handles move the ramp. They never touch the values, and the bars
//! never move: the histogram is of the data.
//!
//! A caller may also ask for reference lines through the bars — see
//! [`Mark`]. On a residual field those are the sensor's noise and the
//! median, which turns the rule the whole application rests on into
//! something you look at rather than something you are told.

use eframe::egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, Vec2, pos2};

use crate::field::Field;
use crate::theme::Palette;

/// Height of the bars.
const BARS: f32 = 34.0;
/// Height of the strip of numbers under them.
const AXIS: f32 = 12.0;
/// Height of the strip of mark labels above them.
const MARKS: f32 = 11.0;

/// A reference line through the bars, with a name.
///
/// Not a handle: it moves nothing and is not draggable. It is a value the
/// distribution should be read *against* — where the sensor's noise falls,
/// where the median fell — so that a verdict stated in words above has a
/// picture under it.
///
/// A value outside the field's range pins to the near edge, which reads
/// correctly on its own: a σ pinned right means every residual is inside
/// the noise.
pub(crate) struct Mark {
    /// Where, in the field's own units.
    pub(crate) at: f32,
    /// Two or three characters. There is room for no more.
    pub(crate) label: &'static str,
    /// Marks carry verdicts, so they carry their own colour.
    pub(crate) colour: Color32,
}

/// Draws the histogram and returns a new clamp if a handle was dragged.
pub(crate) fn show(
    ui: &mut Ui,
    palette: &Palette,
    field: &Field,
    cold: Color32,
    hot: Color32,
    marks: &[Mark],
) -> Option<[f32; 2]> {
    let above = if marks.is_empty() { 0.0 } else { MARKS };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), above + BARS + AXIS),
        Sense::click_and_drag(),
    );
    let painter = ui.painter();
    let plot = Rect::from_min_max(
        pos2(rect.left(), rect.top() + above),
        pos2(rect.right(), rect.top() + above + BARS),
    );

    // Counts go up as their square root. One bin usually holds most of a
    // real field — the floor of a room, the near-zero of a good
    // registration — and on a linear axis every other bin is then a line
    // one pixel high.
    let tallest = field.bins.iter().copied().max().unwrap_or(1).max(1) as f32;
    let width = plot.width() / field.bins.len() as f32;
    let low = field.position(field.clamp[0]);
    let high = field.position(field.clamp[1]);

    for (index, count) in field.bins.iter().enumerate() {
        let at = (index as f32 + 0.5) / field.bins.len() as f32;
        let height = (*count as f32 / tallest).sqrt() * plot.height();
        let left = plot.left() + index as f32 * width;
        let bar = Rect::from_min_max(
            pos2(left, plot.bottom() - height),
            pos2(left + width - 1.0, plot.bottom()),
        );
        // A bar takes the colour the ramp would give it, so the histogram
        // and the cloud say the same thing in the same hues. Outside the
        // clamps the ramp is flat, and so is this.
        let along = ((at - low) / (high - low).max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
        painter.rect_filled(bar, 0.0, mix(cold, hot, along));
    }

    for (position, value) in [(low, field.clamp[0]), (high, field.clamp[1])] {
        let x = plot.left() + plot.width() * position;
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, palette.text),
        );
        painter.text(
            pos2(x, plot.bottom() + 1.0),
            Align2::CENTER_TOP,
            format!("{value:.3}"),
            FontId::monospace(9.0),
            palette.muted,
        );
    }

    // After the handles, so that where the two coincide the reference line
    // is the one left visible: a handle can be dragged back out, and a σ
    // cannot be recovered by looking.
    for mark in marks {
        let x = plot.left() + plot.width() * field.position(mark.at);
        // Dashed, so that it reads as an annotation rather than as data and
        // cannot be mistaken for a bar.
        let mut y = plot.top();
        while y < plot.bottom() {
            let to = (y + 3.0).min(plot.bottom());
            painter.line_segment([pos2(x, y), pos2(x, to)], Stroke::new(1.0, mark.colour));
            y += 6.0;
        }
        painter.text(
            pos2(x, rect.top()),
            Align2::CENTER_TOP,
            mark.label,
            FontId::monospace(9.0),
            mark.colour,
        );
    }

    let pointer = response
        .interact_pointer_pos()
        .filter(|_| response.dragged() || response.clicked())?;
    let at = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    // Whichever handle is nearer, so a drag does what it looked like it
    // would.
    let value = field.value_at(at);
    Some(if (at - low).abs() <= (at - high).abs() {
        [value, field.clamp[1]]
    } else {
        [field.clamp[0], value]
    })
}

fn mix(cold: Color32, hot: Color32, along: f32) -> Color32 {
    let blend = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * along) as u8;
    Color32::from_rgb(
        blend(cold.r(), hot.r()),
        blend(cold.g(), hot.g()),
        blend(cold.b(), hot.b()),
    )
}
