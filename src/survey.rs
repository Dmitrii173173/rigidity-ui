//! The survey: registrations kept as edges, and the solve over them.
//!
//! [`rigidity_graph`] holds the mathematics. This module is the two things
//! the mathematics cannot know about: which clouds on screen a node refers
//! to, and which frame the numbers should be expressed in.
//!
//! # The frame, and the measurement that decided it
//!
//! A pose graph could be built straight out of absolute coordinates: a
//! conditioning report describes directions relative to the coordinate
//! origin, a scan's pose is in absolute coordinates too, and the two agree.
//! On anything georeferenced it is unusable, and not for the reason it first
//! looks like.
//!
//! The poses are not the problem. Every quantity the solve touches is a
//! relative one, and `T_i⁻¹·T_j` between two stations four million metres
//! from zero is still good to about a nanometre — measured, in
//! `the_world_alone_would_not_have_needed_moving`.
//!
//! The *information matrix* is the problem. `calibrated_information` reports
//! its directions about the coordinate origin, so a rotation of one
//! milliradian carries four kilometres of translation with it, and the
//! matrix comes back with a condition number of 1e18 where the same room at
//! the origin gives 1e2. Sixteen orders of avoidable ill-conditioning, and
//! the Cholesky of the normal equations has nothing left afterwards.
//!
//! So the graph is *conjugated* onto the survey — `T' = S·T·S⁻¹`,
//! `Z' = S·Z·S⁻¹`, `Λ' = Adj(S)⁻ᵀ·Λ·Adj(S)⁻¹`, with `S = (I, −origin)`. That
//! moves the point every direction is referred to from absolute zero onto
//! the survey, which is the whole of the fix.
//!
//! It also moves the point each station's marginal uncertainty is *about*,
//! and that would quietly turn "how well do we know where this station is"
//! into "how well do we know where the survey's origin is, as seen from
//! this station". [`diagnose`] carries the covariances back by the same
//! adjoint before anybody reads them.

use rigidity_core::lie::Se3;
use rigidity_core::nalgebra as na;
use rigidity_core::observability::{Conditioning, ObservabilityCriteria};
use rigidity_graph::{
    Diagnosis, Edge, GraphError, OptimiseParams, PoseGraph, Report, Shape, calibrated_information,
};

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
    /// The median absolute residual it was made at, metres.
    ///
    /// Whether that registration was in the right place at all — the one
    /// question the conditioning beside it cannot answer. `None` for an
    /// edge read from a project written before this was recorded, and the
    /// panel says "not recorded" rather than "fine", because an edge that
    /// was never checked is not an edge that passed.
    pub(crate) median_residual: Option<f64>,
}

impl Link {
    /// Builds an edge from a finished registration.
    pub(crate) fn new(
        from: u64,
        to: u64,
        measurement: Se3,
        rmse: f64,
        median_residual: Option<f64>,
        conditioning: &Conditioning,
        criteria: &ObservabilityCriteria,
    ) -> Self {
        use rigidity_core::observability::Observability;
        Self {
            from,
            to,
            measurement,
            // The tolerance is no longer part of this: `calibrated_information`
            // stopped dropping directions on a threshold when S6 measured
            // that doing so never once helped a real survey. What the edge
            // carries now is `JᵀWJ` with the project's calibration in it.
            // `determined` below is unchanged, because *saying* which
            // directions are weak is the part that held up.
            information: calibrated_information(conditioning, criteria.noise_sigma),
            determined: conditioning
                .classify(criteria)
                .iter()
                .filter(|state| **state == Observability::High)
                .count(),
            rmse,
            median_residual,
        }
    }
}

/// The shift carrying absolute world coordinates onto the survey.
fn shift(origin: &na::Vector3<f64>) -> Se3 {
    Se3::from_parts(rigidity_core::lie::So3::identity(), -origin)
}

/// What the edges add up to, for the panel to say out loud.
///
/// Built from identity poses: the shape of a graph is a fact about which
/// nodes its edges name, and none of the arithmetic that needs a frame
/// happens here.
pub(crate) fn shape(nodes: &[(u64, Se3)], links: &[Link], anchor: usize) -> Shape {
    let mut graph = PoseGraph::new(vec![Se3::identity(); nodes.len()]);
    let index_of = |id: u64| nodes.iter().position(|(other, _)| *other == id);
    for link in links {
        let (Some(from), Some(to)) = (index_of(link.from), index_of(link.to)) else {
            continue;
        };
        if from == to {
            continue;
        }
        let _ = graph.push(Edge {
            from,
            to,
            measurement: Se3::identity(),
            information: na::Matrix6::identity(),
        });
    }
    graph.shape(anchor)
}

/// What one solve did, and to which entries.
pub(crate) struct Solved {
    /// The new pose of each entry, in the order they were given.
    pub(crate) poses: Vec<Se3>,
    /// What the optimiser reports about itself.
    pub(crate) report: Report,
    /// The survey seen whole, with every marginal carried back into the
    /// frame its station stands in.
    pub(crate) diagnosis: Option<Diagnosis>,
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
    let carry = unshift.adjoint();
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
        if let Err(error) = graph.push(Edge {
            from,
            to,
            measurement: shift * link.measurement * unshift,
            // The quadratic form has to come out the same, so the
            // information transforms by the inverse adjoint on both sides.
            information: carry.transpose() * link.information * carry,
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
            diagnosis: graph.diagnose(anchor).ok().map(|mut diagnosis| {
                // Back into the frame each station actually stands in. The
                // conjugation put every marginal about the survey's origin;
                // without this the position block would answer a question
                // nobody asked.
                let back = shift.adjoint();
                for node in &mut diagnosis.nodes {
                    node.covariance = back.transpose() * node.covariance * back;
                }
                diagnosis
            }),
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

    /// An information matrix with different weights in different directions,
    /// so that a frame change could not hide behind a multiple of the
    /// identity.
    fn lopsided() -> na::Matrix6<f64> {
        let mut out = na::Matrix6::zeros();
        for axis in 0..6 {
            out[(axis, axis)] = 10f64.powi(axis as i32 - 2);
        }
        out[(0, 4)] = 0.3;
        out[(4, 0)] = 0.3;
        out
    }

    /// The cost of one deliberately-wrong measurement, with the world's
    /// origin put here, for a scene centred at `centre`.
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
        let mut graph = PoseGraph::new(nodes.iter().map(|pose| s * *pose).collect());
        graph
            .push(Edge {
                from: 0,
                to: 1,
                measurement,
                information: lopsided(),
            })
            .expect("the edge names real nodes");
        graph.cost()
    }

    /// Moving the world's origin changes nothing, because every quantity the
    /// solve touches is a relative one.
    ///
    /// The scene sits at the origin here and only the frame moves, which is
    /// what isolates the claim. Building the scene at a UTM coordinate
    /// instead loses eight digits in `nodes[0].inverse() * nodes[1]` before
    /// any of this code runs, and the comparison would be measuring that
    /// subtraction — which is the next test.
    #[test]
    fn the_cost_does_not_depend_on_the_frame() {
        let at_zero = cost_in(na::Vector3::zeros(), na::Vector3::zeros());
        let moved = cost_in(na::Vector3::new(10.0, -7.0, 3.0), na::Vector3::zeros());
        assert!(at_zero > 0.0, "the fixture has no residual to weigh");
        let relative = (at_zero - moved).abs() / at_zero;
        assert!(
            relative < 1e-12,
            "the cost changed by {relative} relative when the world moved"
        );
    }

    /// And the absolute frame is the one that cannot hold the answer.
    ///
    /// This is the whole argument for shifting, made as a measurement rather
    /// than as a claim in a doc comment: the same cost computed with the
    /// world's origin at absolute zero disagrees by around 1e-8 relative,
    /// which is the eight digits `f64` loses when a two-centimetre residual
    /// is built out of coordinates of four million metres.
    ///
    /// The lower bound is the assertion that matters. If this ever stops
    /// disagreeing, either the fixture stopped being georeferenced or the
    /// shift stopped happening, and both are worth being told about.
    /// Eight stations round a ring, joined consecutively with a bias.
    fn ring(
        centre: na::Vector3<f64>,
        information: na::Matrix6<f64>,
    ) -> (Vec<(u64, Se3)>, Vec<Link>) {
        let nodes: Vec<(u64, Se3)> = (0..8)
            .map(|index| {
                let angle = index as f64 / 8.0 * std::f64::consts::TAU;
                (
                    index as u64,
                    pose(
                        centre.x + 20.0 * angle.cos(),
                        centre.y + 20.0 * angle.sin(),
                        angle,
                    ),
                )
            })
            .collect();
        let bias = Se3::exp(&na::Vector6::new(0.01, 0.004, 0.0, 0.0, 0.0, 0.0015));
        let links = (0..8)
            .map(|from| {
                let to = (from + 1) % 8;
                Link {
                    from: from as u64,
                    to: to as u64,
                    measurement: bias * (nodes[from].1.inverse() * nodes[to].1),
                    information,
                    determined: 6,
                    median_residual: None,
                    rmse: 0.0,
                }
            })
            .collect();
        (nodes, links)
    }

    /// What not moving the frame costs the answer.
    ///
    /// The weight here is the shape a real one has: well-conditioned about
    /// the room it was measured in, and *expressed* about the coordinate
    /// origin, which is where `calibrated_information` puts it. At a UTM
    /// easting that is a matrix conditioning at 1e18, and solving with it as
    /// given rather than conjugated onto the survey moves the answer by
    /// centimetres.
    ///
    /// The assertion is a lower bound, deliberately. If this ever stops
    /// disagreeing, the conjugation has stopped being load-bearing and
    /// should go — but it should go on a measurement, and this is the one.
    #[test]
    fn not_moving_the_frame_costs_the_answer() {
        let centre = na::Vector3::new(512_345.678_9, 4_123_456.789_1, 231.5);
        let carry = shift(&centre).inverse().adjoint();
        let realistic = carry.transpose() * (na::Matrix6::identity() * 1e6) * carry;
        let (nodes, links) = ring(centre, realistic);

        let run = |origin: na::Vector3<f64>| {
            solve(&nodes, &links, 0, &origin, &OptimiseParams::default())
                .expect("there are edges")
                .expect("and it solves")
                .poses
        };
        let worst = run(centre)
            .iter()
            .zip(&run(na::Vector3::zeros()))
            .map(|(a, b)| (a.translation() - b.translation()).norm())
            .fold(0.0f64, f64::max);
        println!("leaving the frame where it is costs {worst:e} m");
        assert!(
            worst > 1e-3,
            "the two frames agree to {worst} m, so the conjugation is no \
             longer earning its place"
        );
    }

    /// And the information matrix is what did.
    ///
    /// A weight referred to the coordinate origin carries four kilometres of
    /// translation for every milliradian of rotation once the survey is four
    /// million metres out. Conjugating moves the point it is referred to
    /// onto the survey, and this measures what that is worth: the condition
    /// number of the matrix the solve is actually handed.
    #[test]
    fn conjugating_is_what_saves_the_information_matrix() {
        let centre = na::Vector3::new(512_345.678_9, 4_123_456.789_1, 231.5);
        // A weight of the shape a real registration produces: strong in
        // translation, far stronger in rotation, referred to the origin.
        let mut absolute = na::Matrix6::zeros();
        for axis in 0..3 {
            absolute[(axis, axis)] = 2.5e8;
            absolute[(axis + 3, axis + 3)] = 2.5e8;
        }
        let s = shift(&centre);
        let carry = s.inverse().adjoint();
        let conjugated = carry.transpose() * absolute * carry;

        let condition = |m: &na::Matrix6<f64>| {
            let eigen = m.symmetric_eigen().eigenvalues;
            let hi = eigen.iter().fold(0.0f64, |b, v| b.max(v.abs()));
            let lo = eigen.iter().fold(f64::INFINITY, |b, v| b.min(v.abs()));
            hi / lo
        };
        let before = condition(&conjugated);
        let after = condition(&absolute);
        assert!(
            before > 1e10 * after,
            "referred to the origin the weight conditions at {before:e} and \
             referred to the survey at {after:e} — if those are close, the \
             conjugation is no longer earning its place"
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
            median_residual: None,
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
            median_residual: None,
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
