//! The survey: registrations kept as edges, and the solve over them.
//!
//! [`rigidity_graph`] holds the mathematics. This module is the two things
//! the mathematics cannot know about: which clouds on screen a node refers
//! to, and which frame the numbers should be expressed in.
//!
//! # The frame, and why it is not the obvious one
//!
//! A conditioning report describes directions relative to the *coordinate
//! origin*, and a scan's pose is in absolute coordinates too, so the two
//! already agree and a graph could be built from them directly. It would
//! also be unusable on any survey that is georeferenced. Referred to
//! absolute zero, a milliradian of rotation about a scene four million
//! metres north is four kilometres of translation: the residual and the
//! information matrix both stay *correct*, and they span twelve orders of
//! magnitude, which the Cholesky of the normal equations does not survive.
//!
//! So everything is conjugated into the frame the viewport already draws
//! in, whose origin sits on the survey. `T' = S·T·S⁻¹` for a pose and
//! `Λ' = Adj(S)⁻ᵀ·Λ·Adj(S)⁻¹` for an information matrix, with
//! `S = (I, −origin)`. Both are exact — this is a change of coordinates and
//! not an approximation — and `the_cost_does_not_depend_on_the_frame` is
//! what says so.

use rigidity_core::lie::Se3;
use rigidity_core::nalgebra as na;
use rigidity_core::observability::{Conditioning, ObservabilityCriteria};
use rigidity_graph::{Edge, GraphError, OptimiseParams, PoseGraph, Report, weighted_information};

/// A registration kept as an edge of the survey.
#[derive(Debug, Clone)]
pub(crate) struct Link {
    /// The entry the measurement is expressed in — the target.
    pub(crate) from: u64,
    /// The entry it measures — the source.
    pub(crate) to: u64,
    /// `Z`: the source's coordinates in the target's frame, absolute.
    pub(crate) measurement: Se3,
    /// The weight, in absolute coordinates. Built once, when the edge is
    /// made, from the conditioning of the registration that made it.
    pub(crate) information: na::Matrix6<f64>,
    /// How many of the six directions that registration determined.
    ///
    /// Kept for the panel rather than recomputed: the conditioning it came
    /// from belongs to a run that has since been replaced, and a number
    /// that silently started describing a different run would be worse than
    /// no number.
    pub(crate) determined: usize,
    /// The residual it was made at, metres.
    pub(crate) rmse: f64,
}

impl Link {
    /// Builds an edge from a finished registration.
    pub(crate) fn new(
        from: u64,
        to: u64,
        measurement: Se3,
        rmse: f64,
        conditioning: &Conditioning,
        criteria: &ObservabilityCriteria,
    ) -> Self {
        use rigidity_core::observability::Observability;
        Self {
            from,
            to,
            measurement,
            information: weighted_information(conditioning, criteria),
            determined: conditioning
                .classify(criteria)
                .iter()
                .filter(|state| **state == Observability::High)
                .count(),
            rmse,
        }
    }
}

/// The shift carrying absolute coordinates into the frame with this origin.
fn shift(origin: &na::Vector3<f64>) -> Se3 {
    Se3::from_parts(rigidity_core::lie::So3::identity(), -origin)
}

/// What one solve did, and to which entries.
pub(crate) struct Solved {
    /// The new pose of each entry, in the order they were given.
    pub(crate) poses: Vec<Se3>,
    /// What the optimiser reports about itself.
    pub(crate) report: Report,
}

/// Solves the survey, and gives back a pose per entry.
///
/// `nodes` is the entries in scene order with their current poses; `anchor`
/// is the index of the one held still. Links naming an entry that is not in
/// `nodes` are skipped — a survey outlives the clouds that happen to be
/// loaded, and refusing to solve because one scan was cleared would be the
/// wrong answer to a thing the person did on purpose.
///
/// Returns `None` when there is nothing to solve: no links, or none whose
/// two ends are both present.
pub(crate) fn solve(
    nodes: &[(u64, Se3)],
    links: &[Link],
    anchor: usize,
    origin: &na::Vector3<f64>,
    params: &OptimiseParams,
) -> Option<Result<Solved, GraphError>> {
    let shift = shift(origin);
    let unshift = shift.inverse();
    let inverse_adjoint = unshift.adjoint();

    let index_of = |id: u64| nodes.iter().position(|(other, _)| *other == id);

    let mut graph = PoseGraph::new(
        nodes
            .iter()
            .map(|(_, pose)| shift * *pose * unshift)
            .collect(),
    );
    let mut used = 0;
    for link in links {
        let (Some(from), Some(to)) = (index_of(link.from), index_of(link.to)) else {
            continue;
        };
        if from == to {
            continue;
        }
        // The quadratic form has to come out the same in either frame, so
        // the information transforms by the inverse adjoint on both sides.
        let information = inverse_adjoint.transpose() * link.information * inverse_adjoint;
        if let Err(error) = graph.push(Edge {
            from,
            to,
            measurement: shift * link.measurement * unshift,
            information,
        }) {
            return Some(Err(error));
        }
        used += 1;
    }
    if used == 0 {
        return None;
    }

    let mut params = *params;
    params.anchor = anchor;
    match graph.optimise(&params) {
        Ok(report) => Some(Ok(Solved {
            poses: graph
                .poses()
                .iter()
                .map(|pose| unshift * *pose * shift)
                .collect(),
            report,
        })),
        Err(error) => Some(Err(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rigidity_core::lie::So3;
    use rigidity_graph::Edge;

    fn pose(x: f64, y: f64, yaw: f64) -> Se3 {
        Se3::from_parts(
            So3::exp(&na::Vector3::new(0.0, 0.0, yaw)),
            na::Vector3::new(x, y, 0.0),
        )
    }

    /// The cost of one deliberately-wrong measurement, computed in the
    /// frame with this origin, for a scene centred at `centre`.
    fn cost_in(origin: na::Vector3<f64>, centre: na::Vector3<f64>) -> f64 {
        let nodes = [
            pose(centre.x, centre.y, 0.0),
            pose(centre.x + 20.0, centre.y + 1.0, 0.3),
            pose(centre.x + 19.0, centre.y + 21.0, 1.2),
        ];
        let truth = nodes[0].inverse() * nodes[1];
        // Slightly wrong, so there is a residual to weigh.
        let measurement = Se3::exp(&na::Vector6::new(0.02, -0.01, 0.005, 0.0, 0.0, 0.001)) * truth;

        let s = shift(&origin);
        let inverse = s.inverse();
        let adjoint = inverse.adjoint();
        let mut graph = PoseGraph::new(nodes.iter().map(|pose| s * *pose * inverse).collect());
        graph
            .push(Edge {
                from: 0,
                to: 1,
                measurement: s * measurement * inverse,
                information: adjoint.transpose() * lopsided() * adjoint,
            })
            .expect("the edge names real nodes");
        graph.cost()
    }

    /// An information matrix with different weights in different directions,
    /// so that a frame change that got the adjoint wrong could not hide
    /// behind a multiple of the identity.
    fn lopsided() -> na::Matrix6<f64> {
        let mut out = na::Matrix6::zeros();
        for axis in 0..6 {
            out[(axis, axis)] = 10f64.powi(axis as i32 - 2);
        }
        // Off-diagonal terms too: a diagonal matrix is invariant under more
        // transformations than the right one.
        out[(0, 4)] = 0.3;
        out[(4, 0)] = 0.3;
        out
    }

    /// The change of frame is a change of coordinates, so the cost is the
    /// same number on both sides of it.
    ///
    /// The scene sits at the origin here and only the frame moves, which is
    /// what isolates the algebra. Building the scene at a UTM coordinate
    /// instead would lose eight digits in `nodes[0].inverse() * nodes[1]`
    /// before any of this code ran, and the comparison would be measuring
    /// that subtraction rather than the adjoint.
    #[test]
    fn the_cost_does_not_depend_on_the_frame() {
        let at_zero = cost_in(na::Vector3::zeros(), na::Vector3::zeros());
        let moved = cost_in(na::Vector3::new(10.0, -7.0, 3.0), na::Vector3::zeros());
        assert!(at_zero > 0.0, "the fixture has no residual to weigh");
        let relative = (at_zero - moved).abs() / at_zero;
        assert!(
            relative < 1e-12,
            "the cost changed by {relative} relative when the frame moved"
        );
    }

    /// And the absolute frame is the one that cannot hold the answer.
    ///
    /// This is the whole argument for shifting, made as a measurement
    /// rather than as a claim in a doc comment. The same cost computed at
    /// absolute zero disagrees with the local one by around 1e-8 relative
    /// — about eight digits, which is what `f64` loses when a two-centimetre
    /// residual is built out of coordinates of four million metres.
    ///
    /// The lower bound is the assertion that matters. If this ever stops
    /// disagreeing, either the fixture stopped being georeferenced or the
    /// shift stopped happening, and both are worth being told about.
    #[test]
    fn the_absolute_frame_is_the_one_that_loses_digits() {
        let centre = na::Vector3::new(512_345.678_9, 4_123_456.789_1, 231.5);
        let local = cost_in(centre, centre);
        let absolute = cost_in(na::Vector3::zeros(), centre);
        let relative = (absolute - local).abs() / local;
        assert!(
            relative > 1e-11,
            "absolute coordinates lost only {relative} relative — is the fixture still far from zero?"
        );
        assert!(
            relative < 1e-5,
            "absolute coordinates lost {relative} relative, which is more than rounding"
        );
    }

    /// And the solve gives back poses in absolute coordinates, not shifted
    /// ones.
    ///
    /// The failure this guards against is silent and enormous: poses handed
    /// back still shifted would put every scan four million metres from
    /// where it belongs, and the viewport — which draws relative to the same
    /// origin — would show them exactly where they were.
    #[test]
    fn the_answer_comes_back_in_the_frame_it_was_given_in() {
        let origin = na::Vector3::new(512_345.678_9, 4_123_456.789_1, 231.5);
        let nodes = vec![
            (1u64, pose(origin.x, origin.y, 0.0)),
            (2u64, pose(origin.x + 20.0, origin.y, 0.0)),
        ];
        let truth = nodes[0].1.inverse() * nodes[1].1;

        let link = Link {
            from: 1,
            to: 2,
            measurement: truth,
            information: na::Matrix6::identity(),
            determined: 6,
            rmse: 0.0,
        };
        let solved = solve(&nodes, &[link], 0, &origin, &OptimiseParams::default())
            .expect("there is an edge to solve")
            .expect("and it solves");

        // The measurement is exactly right, so nothing should move at all.
        for (index, (before, after)) in nodes.iter().zip(&solved.poses).enumerate() {
            let moved = (before.1.translation() - after.translation()).norm();
            assert!(moved < 1e-6, "node {index} moved {moved} m for no reason");
        }
    }

    /// A link naming a cloud that is no longer loaded is skipped, not fatal.
    #[test]
    fn a_link_to_a_missing_scan_is_skipped() {
        let nodes = vec![(1u64, Se3::identity()), (2u64, Se3::identity())];
        let dangling = Link {
            from: 1,
            to: 99,
            measurement: Se3::identity(),
            information: na::Matrix6::identity(),
            determined: 6,
            rmse: 0.0,
        };
        assert!(
            solve(
                &nodes,
                std::slice::from_ref(&dangling),
                0,
                &na::Vector3::zeros(),
                &OptimiseParams::default()
            )
            .is_none(),
            "a survey of only dangling links has nothing to solve"
        );

        let good = Link {
            from: 1,
            to: 2,
            ..dangling.clone()
        };
        assert!(
            solve(
                &nodes,
                &[dangling, good],
                0,
                &na::Vector3::zeros(),
                &OptimiseParams::default()
            )
            .is_some(),
            "one good link should still solve"
        );
    }
}
