//! The state the worker owns, and the work it does with it.
//!
//! Everything here calls `rigidity-pipeline` and nothing else: the same
//! functions, in the same order, with the same defaults as the command
//! line. That is not tidiness, it is the only reason the two agree — and
//! the tests at the bottom of this file are what keep them agreeing.

use std::ops::ControlFlow;
use std::sync::Arc;

use rigidity_core::PointCloud;
use rigidity_core::icp::{IcpResult, IterationReport};
use rigidity_core::lie::Se3;
use rigidity_core::observability::Analysis;
use rigidity_core::{NeighborSearch, nalgebra as na};
use rigidity_pipeline::{
    PipelineError, PrepareParams, Prepared, Progress, RegisterParams, analyse_cloud,
    analyse_registration, prepare_cloud_observed, register_pair_observed,
};

use super::job::Held;

/// A prepared surface, and what it was prepared from.
struct Slot {
    generation: u64,
    params: PrepareParams,
    prepared: Prepared,
}

/// What the worker keeps between requests.
///
/// Preparing a million points takes about a second, and a registration
/// re-run after a parameter change would otherwise pay it twice for
/// nothing. The cache is keyed by the cloud's generation and the
/// parameters that produced it, which are the only two things that can
/// make it wrong.
#[derive(Default)]
pub(crate) struct Session {
    source: Option<Slot>,
    target: Option<Slot>,
}

/// Everything a finished registration produces.
pub(crate) struct Registration {
    /// The pose, residual, iteration count and information matrix.
    pub(crate) result: IcpResult,
    /// What the geometry determined, at the pose found.
    pub(crate) analysis: Analysis,
    /// The downsampled source, as the solver saw it.
    ///
    /// Shipped to the viewer so that the residual can be drawn on the
    /// points it was actually computed for. Colouring the full-density
    /// cloud would mean a nearest-neighbour query per raw point — a second
    /// of work to say the same thing.
    pub(crate) sampled: Arc<PointCloud>,
    /// The absolute point-to-plane residual at each of those points.
    pub(crate) residuals: Arc<Vec<f32>>,
    /// Where the residual ramp should start and end.
    pub(crate) span: [f32; 2],
    /// How many points each side kept after downsampling.
    pub(crate) points: [usize; 2],
}

impl Session {
    /// Prepares a cloud and reports what its geometry would determine.
    ///
    /// `stale` is consulted at every stage boundary. It cannot interrupt a
    /// stage — the core's observed functions report progress but do not
    /// take an answer back — so a cancellation is honoured after the
    /// current pass over the points, not during it. The status strip says
    /// as much rather than pretending the stop was instant.
    pub(crate) fn analyse(
        &mut self,
        target: &Held,
        params: &PrepareParams,
        stale: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Option<(Analysis, usize)>, PipelineError> {
        if stale() {
            return Ok(None);
        }
        prepare(&mut self.target, target, params, progress)?;
        if stale() {
            return Ok(None);
        }
        let prepared = self.target.as_ref().expect("just prepared");
        let analysis = analyse_cloud(&prepared.prepared)?;
        Ok(Some((analysis, prepared.prepared.len())))
    }

    /// Registers one surface onto another and reports what the answer is
    /// worth.
    ///
    /// Here the cancellation is per iteration rather than per stage:
    /// `register_observed`'s observer returns a `ControlFlow`, so Esc is
    /// answered after the next accepted step instead of after the whole
    /// solve.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register(
        &mut self,
        source: &Held,
        target: &Held,
        prepare_params: &PrepareParams,
        params: &RegisterParams,
        initial: Se3,
        stale: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(Progress),
        iteration: &mut dyn FnMut(&IterationReport),
    ) -> Result<Option<Registration>, PipelineError> {
        if stale() {
            return Ok(None);
        }
        prepare(&mut self.target, target, prepare_params, progress)?;
        if stale() {
            return Ok(None);
        }
        prepare(&mut self.source, source, prepare_params, progress)?;
        if stale() {
            return Ok(None);
        }

        let moving = &self.source.as_ref().expect("just prepared").prepared;
        let fixed = &self.target.as_ref().expect("just prepared").prepared;

        let result = register_pair_observed(moving, fixed, initial, params, |report| {
            iteration(report);
            if stale() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if stale() {
            return Ok(None);
        }

        let analysis = analyse_registration(moving, fixed, &result.pose, params)?;
        let (residuals, span) = residuals(moving, fixed, &result.pose, params);

        Ok(Some(Registration {
            result,
            analysis,
            sampled: Arc::new(moving.cloud.clone()),
            residuals: Arc::new(residuals),
            span,
            points: [moving.len(), fixed.len()],
        }))
    }
}

/// Prepares a cloud, unless the cache already holds exactly that.
fn prepare(
    slot: &mut Option<Slot>,
    held: &Held,
    params: &PrepareParams,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), PipelineError> {
    if slot
        .as_ref()
        .is_some_and(|slot| slot.generation == held.generation && slot.params == *params)
    {
        return Ok(());
    }
    let prepared = prepare_cloud_observed(&held.cloud, params, &mut *progress)?;
    *slot = Some(Slot {
        generation: held.generation,
        params: *params,
        prepared,
    });
    Ok(())
}

/// The absolute point-to-plane residual at every point of the moved source.
///
/// The ramp ends at the 95th percentile rather than the maximum: one
/// outlier at fifty times the median would otherwise flatten every real
/// difference into the first pixel of the ramp.
fn residuals(
    moving: &Prepared,
    fixed: &Prepared,
    pose: &Se3,
    params: &RegisterParams,
) -> (Vec<f32>, [f32; 2]) {
    let limit = params.max_distance * params.max_distance;
    let mut found = Vec::with_capacity(1);
    let mut values = Vec::with_capacity(moving.cloud.len());
    for index in 0..moving.cloud.len() {
        let point = pose.transform_point(&moving.cloud.point(index));
        fixed.tree.knn_into(&point, 1, &mut found);
        let residual = match found.first() {
            Some(nearest) if nearest.distance_squared <= limit => {
                let matched = nearest.index as usize;
                let offset: na::Vector3<f64> = point - fixed.cloud.point(matched);
                fixed.normals[matched].dot(&offset).abs()
            }
            // A point with nothing near enough took no part in the answer.
            // Zero says "not a large residual", which would be a lie; the
            // ramp's cold end is the least misleading place to put it,
            // and the correspondence count next to the report says how
            // many such points there were.
            _ => 0.0,
        };
        values.push(residual as f32);
    }

    let mut sorted = values.clone();
    sorted.sort_by(f32::total_cmp);
    let percentile = sorted
        .get((sorted.len().saturating_sub(1)) * 95 / 100)
        .copied()
        .unwrap_or(1.0)
        .max(1e-6);
    (values, [0.0, percentile])
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use rigidity_core::observability::ObservabilityCriteria;
    use rigidity_pipeline::{ReportParams, transform_cloud};
    use rigidity_scenes::{Scene, SceneKind, SceneParams};

    use super::*;

    /// Where `../rigidity` lives, relative to this crate.
    fn rigidity() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rigidity")
    }

    /// The command line, built once and run on a fixture.
    fn run_cli(args: &[&str], paths: &[&Path]) -> String {
        let build = Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "rigidity-cli", "--manifest-path"])
            .arg(rigidity().join("Cargo.toml"))
            .status()
            .expect("cargo could not be run");
        assert!(build.success(), "the command line would not build");

        let output = Command::new(rigidity().join("target/debug/rigidity"))
            .arg(args[0])
            .args(paths)
            .args(&args[1..])
            .output()
            .expect("the command line could not be run");
        String::from_utf8(output.stdout).expect("the report was not text")
    }

    /// A corridor, and the same corridor moved by a known amount.
    fn corridor_pair() -> (PathBuf, PathBuf, Scene) {
        let scene = Scene::generate(
            SceneKind::Corridor,
            SceneParams {
                points_per_face: 20_000,
                noise_sigma: 0.002,
                ..SceneParams::default()
            },
        );
        let motion = Se3::exp(&na::Vector6::new(0.03, 0.02, 0.01, 0.0, 0.0, 0.0));
        let moved = transform_cloud(&scene.cloud, &motion);

        let target = std::env::temp_dir().join("rigidity-ui-parity-target.ply");
        let source = std::env::temp_dir().join("rigidity-ui-parity-source.ply");
        rigidity_io::write_ply(&scene.cloud, &target).expect("the target could not be written");
        rigidity_io::write_ply(&moved, &source).expect("the source could not be written");
        (source, target, scene)
    }

    /// The viewer and the command line must report the same thing.
    ///
    /// Not a tautology, though both end up inside `rigidity-pipeline`: what
    /// is under test is everything on the way in. Two front ends each map
    /// their own controls onto `PrepareParams` and `ObservabilityCriteria`,
    /// and a viewer whose defaults have quietly drifted from the CLI's
    /// prints numbers that cannot be compared with anyone else's — which
    /// would make the whole application dishonest in a way no user could
    /// detect.
    #[test]
    fn the_analysis_matches_the_command_line() {
        let (_, target, _) = corridor_pair();
        let prepare = PrepareParams::default();
        let report = ReportParams::default();

        let mut session = Session::default();
        let held = Held {
            cloud: Arc::new(rigidity_io::read_ply(&target).expect("the fixture would not read")),
            generation: 1,
        };
        let (analysis, _) = session
            .analyse(&held, &prepare, &|| false, &mut |_| {})
            .expect("the viewer's pipeline failed")
            .expect("the viewer's pipeline was abandoned with nothing to abandon it");

        let theirs = run_cli(
            &[
                "analyse",
                "--voxel",
                &prepare.voxel.to_string(),
                "--neighbours",
                &prepare.neighbours.to_string(),
                "--noise",
                &report.noise.to_string(),
                "--tolerance",
                &report.tolerance.to_string(),
            ],
            &[&target],
        );
        let ours = analysis.describe(&report.criteria());
        assert!(
            theirs.contains(&ours),
            "the viewer and the command line disagree.\n\
             --- viewer ---\n{ours}\n--- command line ---\n{theirs}"
        );
    }

    /// The same, for a registration: pose, residual, correspondences,
    /// iterations, convergence and the conditioning report.
    ///
    /// The numbers are compared as the command line prints them, because
    /// that is the form people quote at each other — and because a
    /// difference too small to change the printed digits is a difference
    /// nobody could act on.
    #[test]
    fn the_registration_matches_the_command_line() {
        let (source, target, _) = corridor_pair();
        let prepare = PrepareParams::default();
        let params = RegisterParams::default();
        let report = ReportParams::default();

        let mut session = Session::default();
        let held = |path: &Path, generation| Held {
            cloud: Arc::new(rigidity_io::read_ply(path).expect("the fixture would not read")),
            generation,
        };
        let outcome = session
            .register(
                &held(&source, 1),
                &held(&target, 2),
                &prepare,
                &params,
                Se3::identity(),
                &|| false,
                &mut |_| {},
                &mut |_| {},
            )
            .expect("the viewer's pipeline failed")
            .expect("the viewer's pipeline was abandoned with nothing to abandon it");

        let theirs = run_cli(
            &[
                "register",
                "--voxel",
                &prepare.voxel.to_string(),
                "--neighbours",
                &prepare.neighbours.to_string(),
                "--noise",
                &report.noise.to_string(),
                "--tolerance",
                &report.tolerance.to_string(),
                "--max-distance",
                &params.max_distance.to_string(),
                "--huber",
                &params.huber.to_string(),
            ],
            &[&source, &target],
        );

        let translation = outcome.result.pose.translation();
        let lines = [
            format!(
                "translation: x = {:+8.4} m  y = {:+8.4} m  z = {:+8.4} m",
                translation.x, translation.y, translation.z
            ),
            format!(
                "RMSE: {:.5} m   correspondences: {}   iterations: {}   converged: {}",
                outcome.result.rmse,
                outcome.result.correspondences,
                outcome.result.iterations,
                outcome.result.converged
            ),
            outcome.analysis.describe(&report.criteria()),
        ];
        for line in lines {
            assert!(
                theirs.contains(&line),
                "the viewer and the command line disagree.\n\
                 --- viewer ---\n{line}\n--- command line ---\n{theirs}"
            );
        }
    }

    /// A cancellation returns nothing rather than a half-finished answer.
    #[test]
    fn a_cancelled_request_produces_no_result() {
        let (_, target, _) = corridor_pair();
        let mut session = Session::default();
        let held = Held {
            cloud: Arc::new(rigidity_io::read_ply(&target).expect("the fixture would not read")),
            generation: 1,
        };
        let outcome = session
            .analyse(&held, &PrepareParams::default(), &|| true, &mut |_| {})
            .expect("a cancellation is not a failure");
        assert!(outcome.is_none());
    }

    /// The correction multiplies the noise and nothing else.
    ///
    /// It is a statement about how many measurements are really
    /// independent, not about how accurate the application needs to be, and
    /// a viewer that applied it to the tolerance instead would classify
    /// every scene wrongly while looking entirely plausible.
    #[test]
    fn the_calibration_only_scales_the_noise() {
        let plain = ReportParams::default();
        let corrected = ReportParams {
            calibration: 17.0,
            ..plain
        };
        let ObservabilityCriteria {
            noise_sigma,
            tolerance,
        } = corrected.criteria();
        assert_eq!(noise_sigma, plain.noise * 17.0);
        assert_eq!(tolerance, plain.tolerance);
    }
}
