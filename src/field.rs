//! A scalar per point, and the ramp that shows it.
//!
//! Every number this application computes about a cloud ends up here:
//! height, a column the file carried, the residual after a registration,
//! the distance to another cloud. One representation for all of them means
//! one histogram, one pair of clamps and one shader path, rather than a
//! special case per quantity.
//!
//! **The clamps move the ramp and never the data.** A histogram handle
//! decides which value is the cold end of the colour scale and which is the
//! hot end; it does not filter, rescale or discard anything. That is worth
//! stating because the opposite is common and quietly destroys the number
//! you were trying to read.

use std::sync::Arc;

use rigidity_core::PointCloud;

/// How many bars the histogram has.
///
/// Enough to show a shape, few enough that each is a readable width in a
/// three-hundred-point panel.
pub(crate) const BINS: usize = 48;

/// Where a scalar came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// The z coordinate.
    Height,
    /// A column the file carried — intensity, classification, whatever the
    /// scanner wrote.
    Attribute(String),
    /// Distance from each point to the nearest point of the target.
    Distance,
    /// Point-to-plane residual after a registration.
    ///
    /// Computed on the downsampled cloud the solver used, so a cloud
    /// showing it is drawn at that density.
    Residual,
}

impl Source {
    /// What the picker calls it.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Height => "height".to_owned(),
            Self::Attribute(name) => name.clone(),
            Self::Distance => "distance to target".to_owned(),
            Self::Residual => "residual".to_owned(),
        }
    }

    /// Whether the values belong to the solver's downsampled cloud rather
    /// than to the entry's own points.
    pub(crate) fn on_sampled(&self) -> bool {
        matches!(self, Self::Residual)
    }

    /// The unit the values are in, for the axis.
    pub(crate) fn unit(&self) -> &'static str {
        match self {
            Self::Height | Self::Distance | Self::Residual => "m",
            Self::Attribute(_) => "",
        }
    }
}

/// One scalar field, ready to colour with.
pub(crate) struct Field {
    /// Where it came from.
    pub(crate) source: Source,
    /// One value per point. Never modified after it is built.
    pub(crate) values: Arc<Vec<f32>>,
    /// The whole range of the data.
    pub(crate) full: [f32; 2],
    /// The part of it the ramp spans. This is what the handles move.
    pub(crate) clamp: [f32; 2],
    /// How many values fall in each bin of `full`.
    pub(crate) bins: [u32; BINS],
}

/// Where the ramp's hot end starts out, as a percentile of the data.
///
/// Not the maximum. A residual field is almost all noise with a handful of
/// points on an edge fifty times worse, and a ramp stretched to that
/// maximum paints every real difference the same cold colour. The handle
/// starts pulled in, the histogram shows the tail sticking out past it,
/// and dragging it back out is one gesture.
const HOT_PERCENTILE: usize = 98;

impl Field {
    /// Builds a field from values.
    pub(crate) fn new(source: Source, values: Vec<f32>) -> Self {
        let finite = || values.iter().copied().filter(|v| v.is_finite());
        let low = finite().fold(f32::INFINITY, f32::min);
        let high = finite().fold(f32::NEG_INFINITY, f32::max);
        // A constant field is not an error — a flat floor has one height —
        // and it needs a range wide enough to divide by.
        let (low, high) = if low.is_finite() && high > low {
            (low, high)
        } else if low.is_finite() {
            (low, low + 1.0)
        } else {
            (0.0, 1.0)
        };

        let mut bins = [0u32; BINS];
        let span = high - low;
        for value in finite() {
            let at = ((value - low) / span * BINS as f32) as usize;
            bins[at.min(BINS - 1)] += 1;
        }

        let mut sorted: Vec<f32> = finite().collect();
        sorted.sort_by(f32::total_cmp);
        let hot = sorted
            .get(sorted.len().saturating_sub(1) * HOT_PERCENTILE / 100)
            .copied()
            .filter(|hot| *hot > low)
            .unwrap_or(high);

        Self {
            source,
            values: Arc::new(values),
            full: [low, high],
            clamp: [low, hot],
            bins,
        }
    }

    /// Moves a clamp, keeping the two in order and apart.
    ///
    /// Nothing else changes: `values` is behind an `Arc` that is never
    /// unwrapped, and the histogram is of the data, not of the ramp.
    pub(crate) fn set_clamp(&mut self, low: f32, high: f32) {
        let span = (self.full[1] - self.full[0]).max(f32::MIN_POSITIVE);
        let smallest = span / BINS as f32;
        let low = low.clamp(self.full[0], self.full[1]);
        let high = high.clamp(self.full[0], self.full[1]);
        self.clamp = if high - low < smallest {
            [low, (low + smallest).min(self.full[1])]
        } else {
            [low, high]
        };
    }

    /// Where a value sits in the full range, from zero to one.
    pub(crate) fn position(&self, value: f32) -> f32 {
        let span = (self.full[1] - self.full[0]).max(f32::MIN_POSITIVE);
        ((value - self.full[0]) / span).clamp(0.0, 1.0)
    }

    /// The value at a position in the full range.
    pub(crate) fn value_at(&self, position: f32) -> f32 {
        self.full[0] + (self.full[1] - self.full[0]) * position.clamp(0.0, 1.0)
    }
}

/// The z coordinate of every point, in the cloud's own local frame.
///
/// Local rather than absolute: on a georeferenced cloud the absolute
/// height is half a million metres of offset plus the metre of relief
/// anybody wants to look at, and `f32` cannot hold both.
pub(crate) fn height(cloud: &PointCloud) -> Vec<f32> {
    let (_, _, z) = cloud.columns();
    z.to_vec()
}

/// A column the file carried, as `f32`.
pub(crate) fn attribute(cloud: &PointCloud, name: &str) -> Option<Vec<f32>> {
    use rigidity_core::AttributeData;
    let attribute = cloud.attribute(name)?;
    Some(match &attribute.data {
        AttributeData::F32(values) => values.clone(),
        AttributeData::F64(values) => values.iter().map(|v| *v as f32).collect(),
        AttributeData::U8(values) => values.iter().map(|v| f32::from(*v)).collect(),
        AttributeData::U16(values) => values.iter().map(|v| f32::from(*v)).collect(),
        AttributeData::U32(values) => values.iter().map(|v| *v as f32).collect(),
        AttributeData::I32(values) => values.iter().map(|v| *v as f32).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clamp_moves_the_ramp_and_not_the_data() {
        let mut field = Field::new(Source::Height, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        let before = Arc::clone(&field.values);
        let histogram = field.bins;

        field.set_clamp(1.0, 2.0);

        assert_eq!(field.clamp, [1.0, 2.0]);
        assert_eq!(field.full, [0.0, 4.0], "the data's range is not the ramp's");
        assert!(
            Arc::ptr_eq(&before, &field.values),
            "the values were rebuilt, which means something rewrote them"
        );
        assert_eq!(*field.values, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(field.bins, histogram, "the histogram is of the data");
    }

    #[test]
    fn the_clamps_stay_in_order_and_apart() {
        let mut field = Field::new(Source::Height, vec![0.0, 10.0]);
        field.set_clamp(7.0, 7.0);
        assert!(field.clamp[1] > field.clamp[0], "{:?}", field.clamp);
        field.set_clamp(-5.0, 100.0);
        assert_eq!(field.clamp, [0.0, 10.0], "a clamp cannot leave the data");
    }

    /// A field of one value is a flat floor, not a failure.
    #[test]
    fn a_constant_field_still_has_a_range() {
        let field = Field::new(Source::Height, vec![2.5; 16]);
        assert!(field.full[1] > field.full[0]);
        assert_eq!(field.bins.iter().sum::<u32>(), 16);
    }
}
