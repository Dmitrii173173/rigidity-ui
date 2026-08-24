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
    PipelineError, PrepareParams, Prepared, Progress, RegisterParams, SearchParams, Stage,
    analyse_cloud, analyse_registration, median_absolute_residual, prepare_cloud_observed,
    register_globally_observed, register_pair_observed,
};

use rigidity_spatial::KdTree;

use super::job::{Derivation, Held, Start};

/// How many points are measured between two progress reports.
const DISTANCE_CHUNK: usize = 8_192;

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
    /// A full-density index over whatever was last measured against.
    ///
    /// Separate from the prepared surfaces because it indexes every point,
    /// not the downsampled ones: a distance quoted about a voxel grid
    /// understates the real one by however coarse the grid was.
    reference: Option<(u64, KdTree)>,
}

/// The surface a report is about, as the solver saw it.
///
/// Shipped to the viewer so that anything computed per correspondence can
/// be drawn on the points it was computed for. Colouring the full-density
/// cloud instead would mean a nearest-neighbour query per raw point — a
/// second of work to say the same thing, and an implication that the
/// number came from there.
pub(crate) struct Surface {
    /// The downsampled cloud.
    pub(crate) cloud: Arc<PointCloud>,
    /// The normal each correspondence was taken against.
    ///
    /// For a registration these are the *target's* normals at the matched
    /// points, because that is what the Jacobian row is built from. Zero
    /// where nothing was near enough to match.
    pub(crate) normals: Arc<Vec<[f32; 3]>>,
    /// The absolute point-to-plane residual, when there has been a
    /// registration to have one.
    ///
    /// Where its ramp should start and end is not decided here: that is a
    /// question about how to look at the numbers, and it belongs with the
    /// histogram that shows them.
    pub(crate) residuals: Option<Arc<Vec<f32>>>,
}

/// Everything a finished registration produces.
pub(crate) struct Registration {
    /// The pose, residual, iteration count and information matrix.
    pub(crate) result: IcpResult,
    /// What the geometry determined, at the pose found.
    pub(crate) analysis: Analysis,
    /// The moved source, and what it was matched against.
    pub(crate) surface: Surface,
    /// How many points each side kept after downsampling.
    pub(crate) points: [usize; 2],
    /// The median absolute residual at the pose found, metres.
    ///
    /// Whether this is the *right* minimum, which the analysis beside it
    /// cannot say. `None` when nothing matched at all.
    ///
    /// Taken from `rigidity_pipeline` rather than from the residuals this
    /// struct already carries, at the cost of one more pass: the numbers
    /// the viewer warns on and the numbers the command line warns on have
    /// to be the same numbers, and two implementations of one median are
    /// two chances for them not to be.
    pub(crate) median_residual: Option<f64>,
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
    ) -> Result<Option<(Analysis, Surface)>, PipelineError> {
        if stale() {
            return Ok(None);
        }
        prepare(&mut self.target, target, params, progress)?;
        if stale() {
            return Ok(None);
        }
        let prepared = &self.target.as_ref().expect("just prepared").prepared;
        let analysis = analyse_cloud(prepared)?;
        // A single cloud is its own correspondence at zero residual, so
        // the normals are its own.
        let normals = prepared
            .normals
            .iter()
            .map(|n| [n.x as f32, n.y as f32, n.z as f32])
            .collect();
        Ok(Some((
            analysis,
            Surface {
                cloud: Arc::new(prepared.cloud.clone()),
                normals: Arc::new(normals),
                residuals: None,
            },
        )))
    }

    /// Registers one surface onto another and reports what the answer is
    /// worth.
    ///
    /// Here the cancellation is per iteration rather than per stage:
    /// `register_observed`'s observer returns a `ControlFlow`, so Esc is
    /// answered after the next accepted step instead of after the whole
    /// solve.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register(
        &mut self,
        source: &Held,
        target: &Held,
        source_prepare: &PrepareParams,
        target_prepare: &PrepareParams,
        params: &RegisterParams,
        start: Start,
        stale: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(Progress),
        iteration: &mut dyn FnMut(&IterationReport),
    ) -> Result<Option<Registration>, PipelineError> {
        if stale() {
            return Ok(None);
        }
        // One set of parameters each. A survey is not made of scans at one
        // density: a station taken close to a wall and one taken across a
        // hall want different voxels, and forcing the pair to agree means
        // the coarser of the two decides for both.
        prepare(&mut self.target, target, target_prepare, progress)?;
        if stale() {
            return Ok(None);
        }
        prepare(&mut self.source, source, source_prepare, progress)?;
        if stale() {
            return Ok(None);
        }

        let moving = &self.source.as_ref().expect("just prepared").prepared;
        let fixed = &self.target.as_ref().expect("just prepared").prepared;

        // Where to begin. A search is a separate pass over the pair, and a
        // deliberately coarse one; what it hands back is a starting pose,
        // which then goes through the very same registration as any other.
        // Nothing downstream — the timeline, the report, the warning — can
        // tell how the starting pose was arrived at, and none of them
        // should.
        let initial = match start {
            Start::At(pose) => pose,
            Start::Search => {
                // Said once, before the search rather than during it. The
                // screening runs on every core at once, and its observer is
                // therefore called from all of them, while the channel this
                // progress goes down belongs to this thread. A second of
                // work does not justify making the whole progress path
                // thread-safe; abandoning the search does, and that is what
                // the observer is used for.
                progress(Progress {
                    stage: Stage::Searching,
                    done: 0,
                    total: 1,
                });
                let searched = register_globally_observed(
                    moving,
                    fixed,
                    params,
                    &SearchParams::default(),
                    |_, _| {
                        if stale() {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    },
                );
                if stale() {
                    return Ok(None);
                }
                match searched {
                    Some(found) => found.result.pose,
                    None => return Err(PipelineError::NoCorrespondences),
                }
            }
        };

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
        let (residuals, normals) = matched(moving, fixed, &result.pose, params);

        let median_residual = median_absolute_residual(moving, fixed, &result.pose, params);

        Ok(Some(Registration {
            result,
            analysis,
            median_residual,
            surface: Surface {
                cloud: Arc::new(moving.cloud.clone()),
                normals: Arc::new(normals),
                residuals: Some(Arc::new(residuals)),
            },
            points: [moving.len(), fixed.len()],
        }))
    }
}

impl Session {
    /// The distance from every point of one cloud to the nearest point of
    /// another.
    ///
    /// Cancellation here is per chunk rather than per stage: the loop is
    /// ours, so there is nothing to wait for.
    pub(crate) fn distances(
        &mut self,
        from: &Held,
        to: &Held,
        pose: &Se3,
        stale: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Option<Vec<f32>>, PipelineError> {
        if stale() {
            return Ok(None);
        }
        if !self
            .reference
            .as_ref()
            .is_some_and(|(id, _)| *id == to.generation)
        {
            self.reference = Some((to.generation, KdTree::build(&to.cloud)?));
        }
        let (_, tree) = self.reference.as_ref().expect("just built");

        let count = from.cloud.len();
        let mut values = Vec::with_capacity(count);
        let mut found = Vec::with_capacity(1);
        for index in 0..count {
            if index.is_multiple_of(DISTANCE_CHUNK) {
                if stale() {
                    return Ok(None);
                }
                progress(index, count);
            }
            tree.knn_into(
                &pose.transform_point(&from.cloud.point(index)),
                1,
                &mut found,
            );
            values.push(match found.first() {
                Some(nearest) => nearest.distance_squared.sqrt() as f32,
                None => f32::NAN,
            });
        }
        progress(count, count);
        Ok(Some(values))
    }
}

/// Which points of a cloud fall inside a shape drawn on the screen.
///
/// The test is done where the shape was drawn — in viewport pixels, after
/// projection — because that is where the person drew it. Doing it in
/// three dimensions would need a frustum and would still answer a
/// different question from the one they asked.
///
/// Points the cross-section hid are not selected. What is on screen is
/// what a lasso round it means.
pub(crate) fn select(
    from: &PointCloud,
    matrix: &[f32; 16],
    viewport: [f32; 2],
    polygon: &[[f32; 2]],
    slab: &([f32; 3], f32, f32),
    stale: &dyn Fn() -> bool,
) -> Option<Vec<u32>> {
    if polygon.len() < 3 {
        return Some(Vec::new());
    }
    let (normal, near, far) = *slab;
    let sectioned = far > near;

    let (x, y, z) = from.columns();
    let mut inside = Vec::new();
    for index in 0..from.len() {
        if index.is_multiple_of(SELECT_CHUNK) && stale() {
            return None;
        }
        let (px, py, pz) = (x[index], y[index], z[index]);
        if sectioned {
            let along = px * normal[0] + py * normal[1] + pz * normal[2];
            if along < near || along > far {
                continue;
            }
        }

        // Column-major, as the shader reads it.
        let w = matrix[3] * px + matrix[7] * py + matrix[11] * pz + matrix[15];
        if w <= 0.0 {
            // Behind the eye. Projecting it would put it back on screen,
            // mirrored, and select things nobody could see.
            continue;
        }
        let clip_x = matrix[0] * px + matrix[4] * py + matrix[8] * pz + matrix[12];
        let clip_y = matrix[1] * px + matrix[5] * py + matrix[9] * pz + matrix[13];
        let at = [
            (clip_x / w * 0.5 + 0.5) * viewport[0],
            (0.5 - clip_y / w * 0.5) * viewport[1],
        ];
        if encloses(polygon, at) {
            inside.push(index as u32);
        }
    }
    Some(inside)
}

/// How many points are tested between two cancellation checks.
const SELECT_CHUNK: usize = 16_384;

/// Whether a closed polygon encloses a point, by crossing number.
///
/// A self-crossing lasso is not an error and is not rejected: the
/// even-odd rule gives it a meaning, and a freehand shape that touches
/// itself once is far more common than one drawn on purpose.
fn encloses(polygon: &[[f32; 2]], at: [f32; 2]) -> bool {
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for corner in polygon {
        if (corner[1] > at[1]) != (previous[1] > at[1]) {
            let span = previous[1] - corner[1];
            if span != 0.0 {
                let crossing = (previous[0] - corner[0]) * (at[1] - corner[1]) / span + corner[0];
                if at[0] < crossing {
                    inside = !inside;
                }
            }
        }
        previous = *corner;
    }
    inside
}

/// The point nearest a place on the screen, if anything is near enough.
///
/// The same projection the lasso uses, asking a smaller question. Nearest
/// *on screen* rather than nearest along the ray: the person clicked at a
/// place in a picture, and the point they meant is the one that looks
/// closest to where they clicked. Ties in the picture are broken by depth,
/// so the front surface wins over the back of the same wall.
pub(crate) fn nearest(
    from: &PointCloud,
    matrix: &[f32; 16],
    viewport: [f32; 2],
    at: [f32; 2],
    radius: f32,
    slab: &([f32; 3], f32, f32),
) -> Option<na::Vector3<f64>> {
    let (normal, near, far) = *slab;
    let sectioned = far > near;
    let (x, y, z) = from.columns();

    let mut best: Option<(f32, f32, usize)> = None;
    for index in 0..from.len() {
        let (px, py, pz) = (x[index], y[index], z[index]);
        if sectioned {
            let along = px * normal[0] + py * normal[1] + pz * normal[2];
            if along < near || along > far {
                continue;
            }
        }
        let w = matrix[3] * px + matrix[7] * py + matrix[11] * pz + matrix[15];
        if w <= 0.0 {
            continue;
        }
        let screen = [
            ((matrix[0] * px + matrix[4] * py + matrix[8] * pz + matrix[12]) / w * 0.5 + 0.5)
                * viewport[0],
            (0.5 - (matrix[1] * px + matrix[5] * py + matrix[9] * pz + matrix[13]) / w * 0.5)
                * viewport[1],
        ];
        let away = (screen[0] - at[0]).hypot(screen[1] - at[1]);
        if away > radius {
            continue;
        }
        // Reverse-Z: a larger clip depth is nearer the eye.
        let depth = (matrix[2] * px + matrix[6] * py + matrix[10] * pz + matrix[14]) / w;
        if best.is_none_or(|(_, front, _)| depth > front) {
            best = Some((away, depth, index));
        }
    }
    best.map(|(_, _, index)| from.point(index))
}

/// A new cloud made from an old one.
///
/// Attributes are not carried over, for the same reason downsampling drops
/// them: whether a column survives depends on what it means, and the cloud
/// does not know.
pub(crate) fn derive(from: &PointCloud, how: &Derivation) -> Result<PointCloud, PipelineError> {
    let selected = |indices: &[u32], keep: bool| {
        let mut wanted = vec![!keep; from.len()];
        for index in indices {
            if let Some(slot) = wanted.get_mut(*index as usize) {
                *slot = keep;
            }
        }
        let mut out = PointCloud::with_capacity(from.len());
        out.rebase(from.origin());
        for (index, take) in wanted.iter().enumerate() {
            if *take {
                out.push(from.point(index));
            }
        }
        out
    };
    Ok(match how {
        Derivation::Keep(indices) => selected(indices, true),
        Derivation::Drop(indices) => selected(indices, false),
        Derivation::Subsample(voxel) => rigidity_core::voxel::voxel_downsample(from, *voxel)?,
    })
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

/// What each point of the moved source was matched against: its residual
/// and the normal the Jacobian row was built from.
///
/// The two come from one pass because they come from one query. The
/// normals are what makes the contribution of a point to a singular
/// direction computable later, on the viewer's side, without another
/// journey through the index.
///
fn matched(
    moving: &Prepared,
    fixed: &Prepared,
    pose: &Se3,
    params: &RegisterParams,
) -> (Vec<f32>, Vec<[f32; 3]>) {
    let limit = params.max_distance * params.max_distance;
    let mut found = Vec::with_capacity(1);
    let mut residuals = Vec::with_capacity(moving.cloud.len());
    let mut normals = Vec::with_capacity(moving.cloud.len());
    for index in 0..moving.cloud.len() {
        let point = pose.transform_point(&moving.cloud.point(index));
        fixed.tree.knn_into(&point, 1, &mut found);
        match found.first() {
            Some(nearest) if nearest.distance_squared <= limit => {
                let matched = nearest.index as usize;
                let normal = fixed.normals[matched];
                let offset: na::Vector3<f64> = point - fixed.cloud.point(matched);
                residuals.push(normal.dot(&offset).abs() as f32);
                normals.push([normal.x as f32, normal.y as f32, normal.z as f32]);
            }
            // A point with nothing near enough took no part in the answer.
            // Zero residual says "close", which would be a lie, but the
            // ramp's cold end is the least misleading place to put it; a
            // zero normal is not a lie at all — it contributes nothing to
            // every direction, which is exactly what happened.
            _ => {
                residuals.push(0.0);
                normals.push([0.0; 3]);
            }
        }
    }

    (residuals, normals)
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
    ///
    /// The tag keeps two tests from writing the same file: they run in
    /// parallel, and a fixture half-written by one and read by another
    /// fails as a truncated PLY, which is a confusing way to be told about
    /// a race.
    fn corridor_pair(tag: &str) -> (PathBuf, PathBuf, Scene) {
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

        let target = std::env::temp_dir().join(format!("rigidity-ui-{tag}-target.ply"));
        let source = std::env::temp_dir().join(format!("rigidity-ui-{tag}-source.ply"));
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
        let (_, target, _) = corridor_pair("analysis");
        let prepare = PrepareParams::default();
        let report = ReportParams::default();

        let mut session = Session::default();
        let held = Held {
            cloud: Arc::new(rigidity_io::read_ply(&target).expect("the fixture would not read")),
            generation: 1,
        };
        let (analysis, _surface) = session
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
        let (source, target, _) = corridor_pair("registration");
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
                // The same values on both sides: that is what the command
                // line does, and parity with it is what these tests are
                // for. A pair prepared two ways is a thing only the viewer
                // can express, and it is not this test's subject.
                &prepare,
                &prepare,
                &params,
                Start::At(Se3::identity()),
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
        let (_, target, _) = corridor_pair("cancelled");
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

    /// Cloud-to-cloud distance measures the gap between two surfaces.
    ///
    /// A plane, and the same plane lifted along its own normal: every
    /// point's nearest neighbour on the other one is then that lift away,
    /// whatever the sampling did — which makes this the one arrangement
    /// where the answer is known exactly rather than approximately.
    ///
    /// The measured median sits a little *over* the lift, and should: the
    /// nearest neighbour is a sample, not a foot of the perpendicular, so
    /// it is `√(lift² + spacing²)` away. At a five-centimetre lift and a
    /// centimetre of spacing that is two percent, which is why the
    /// tolerance is five and not one.
    #[test]
    fn distance_measures_the_gap_between_two_surfaces() {
        const LIFT: f64 = 0.05;

        let scene = Scene::generate(
            SceneKind::Plane,
            SceneParams {
                points_per_face: 40_000,
                noise_sigma: 0.0,
                outlier_ratio: 0.0,
                ..SceneParams::default()
            },
        );
        let lifted = transform_cloud(
            &scene.cloud,
            &Se3::exp(&na::Vector6::new(0.0, 0.0, LIFT, 0.0, 0.0, 0.0)),
        );

        let mut session = Session::default();
        let values = session
            .distances(
                &Held {
                    cloud: Arc::new(lifted),
                    generation: 1,
                },
                &Held {
                    cloud: Arc::new(scene.cloud),
                    generation: 2,
                },
                &Se3::identity(),
                &|| false,
                &mut |_, _| {},
            )
            .expect("the measurement failed")
            .expect("the measurement was abandoned with nothing to abandon it");

        let mut sorted = values.clone();
        sorted.sort_by(f32::total_cmp);
        let median = f64::from(sorted[sorted.len() / 2]);
        assert!(
            (median - LIFT).abs() < LIFT * 0.05,
            "a plane lifted by {LIFT} m measured {median} m away"
        );
        assert!(
            median >= LIFT,
            "the nearest sample cannot be closer than the surface it is on"
        );
    }

    /// A cancelled measurement returns nothing rather than a short vector.
    #[test]
    fn a_cancelled_measurement_produces_no_result() {
        let (_, target, _) = corridor_pair("cancelled-measurement");
        let cloud = Arc::new(rigidity_io::read_ply(&target).expect("the fixture would not read"));
        let held = Held {
            cloud,
            generation: 1,
        };
        let mut session = Session::default();
        let outcome = session
            .distances(
                &held.clone(),
                &held,
                &Se3::identity(),
                &|| true,
                &mut |_, _| {},
            )
            .expect("a cancellation is not a failure");
        assert!(outcome.is_none());
    }

    /// A rectangular lasso selects exactly the points inside it.
    ///
    /// Counted twice, the second time without going near the projection:
    /// with an identity matrix a point at `x` lands at
    /// `(x·0.5 + 0.5)·width`, so the same rectangle is a range in `x` and
    /// `y` that can be tested directly. The two counts must agree exactly
    /// — this is a question with a right answer, not a tolerance.
    #[test]
    fn a_rectangular_lasso_selects_what_is_inside_it() {
        const SIDE: usize = 200;
        let mut cloud = PointCloud::new();
        for row in 0..SIDE {
            for column in 0..SIDE {
                let x = -1.0 + 2.0 * (column as f64 + 0.5) / SIDE as f64;
                let y = -1.0 + 2.0 * (row as f64 + 0.5) / SIDE as f64;
                cloud.push(na::Vector3::new(x, y, 0.0));
            }
        }

        let identity: [f32; 16] = *na::Matrix4::identity().as_slice().first_chunk().unwrap();
        let viewport = [100.0, 100.0];
        // The upper-right quadrant of the screen, which is x > 0 and
        // y > 0 in the cloud — pixel y counts downwards.
        let polygon = [[50.0, 0.0], [100.0, 0.0], [100.0, 50.0], [50.0, 50.0]];

        let inside = select(
            &cloud,
            &identity,
            viewport,
            &polygon,
            &([0.0; 3], 0.0, 0.0),
            &|| false,
        )
        .expect("the selection was abandoned with nothing to abandon it");

        let expected = (0..cloud.len())
            .filter(|index| {
                let point = cloud.point(*index);
                point.x > 0.0 && point.y > 0.0
            })
            .count();
        assert_eq!(inside.len(), expected);
        assert_eq!(
            expected,
            SIDE * SIDE / 4,
            "the fixture is not what it looks like"
        );
    }

    /// Keeping and deleting partition the cloud, and neither touches it.
    ///
    /// The whole of stage two rests on this: every operation makes a new
    /// entry and leaves its input alone, which is what buys the
    /// application its missing undo stack.
    #[test]
    fn deriving_leaves_the_original_alone() {
        let scene = Scene::generate(
            SceneKind::Plane,
            SceneParams {
                points_per_face: 5_000,
                ..SceneParams::default()
            },
        );
        let original = scene.cloud;
        let before = original.len();
        let indices: Arc<Vec<u32>> = Arc::new((0..before as u32).step_by(3).collect());

        let kept =
            derive(&original, &Derivation::Keep(Arc::clone(&indices))).expect("keeping failed");
        let rest =
            derive(&original, &Derivation::Drop(Arc::clone(&indices))).expect("deleting failed");

        assert_eq!(original.len(), before, "the input was modified");
        assert_eq!(kept.len(), indices.len());
        assert_eq!(
            kept.len() + rest.len(),
            before,
            "the two halves do not add up to the whole"
        );
        // The kept points are the ones asked for, in order.
        for (position, index) in indices.iter().enumerate() {
            assert_eq!(kept.point(position), original.point(*index as usize));
        }
    }

    /// Three picked pairs rescue a registration ICP cannot find alone.
    ///
    /// Both halves matter. A corridor turned thirty degrees is far outside
    /// the basin ICP can reach from the identity, and the test requires it
    /// to *fail* from there — a tool that rescues something never in
    /// danger has not been shown to do anything.
    ///
    /// The picked points are deliberately imprecise, a couple of
    /// centimetres off on each side, because a person clicking a corner in
    /// two scans is imprecise. The closed form then lands near the answer
    /// rather than on it, and ICP does the rest, which is the division of
    /// labour the tool exists for.
    #[test]
    fn three_pairs_rescue_a_registration_from_the_wrong_basin() {
        let scene = Scene::generate(
            SceneKind::Corridor,
            SceneParams {
                points_per_face: 20_000,
                noise_sigma: 0.002,
                ..SceneParams::default()
            },
        );
        let turn = Se3::exp(&na::Vector6::new(
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            30f64.to_radians(),
        ));
        let turned = transform_cloud(&scene.cloud, &turn);

        let source = Held {
            cloud: Arc::new(turned),
            generation: 1,
        };
        let target = Held {
            cloud: Arc::new(scene.cloud),
            generation: 2,
        };
        let prepare = PrepareParams::default();
        let params = RegisterParams::default();
        let mut session = Session::default();

        // How far a found pose is from the one that undoes the turn.
        let error = |pose: &Se3| (*pose * turn).log().norm();

        let blind = session
            .register(
                &source,
                &target,
                // The same values on both sides: that is what the command
                // line does, and parity with it is what these tests are
                // for. A pair prepared two ways is a thing only the viewer
                // can express, and it is not this test's subject.
                &prepare,
                &prepare,
                &params,
                Start::At(Se3::identity()),
                &|| false,
                &mut |_| {},
                &mut |_| {},
            )
            .expect("the registration failed")
            .expect("the registration was abandoned");
        assert!(
            error(&blind.result.pose) > 0.1,
            "thirty degrees was supposed to be out of reach, and ICP found it \
             from the identity anyway — the test proves nothing as written"
        );

        // Three points spread along the corridor, clicked by a shaky hand
        // in both scans.
        let wobble = [
            na::Vector3::new(0.02, -0.01, 0.015),
            na::Vector3::new(-0.015, 0.02, -0.01),
            na::Vector3::new(0.01, 0.012, 0.02),
        ];
        let mut from = Vec::new();
        let mut to = Vec::new();
        for (offset, index) in wobble.iter().zip([0, 25_000, 55_000]) {
            let on_target = target.cloud.point(index);
            to.push(on_target + offset);
            from.push(turn.transform_point(&on_target) - offset);
        }
        let start = rigidity_core::lie::absolute_orientation(&from, &to)
            .expect("three spread points determine a motion");

        let guided = session
            .register(
                &source,
                &target,
                &prepare,
                &prepare,
                &params,
                Start::At(start),
                &|| false,
                &mut |_| {},
                &mut |_| {},
            )
            .expect("the registration failed")
            .expect("the registration was abandoned");
        let found = error(&guided.result.pose);
        assert!(
            found < 0.02,
            "three pairs put it {found} away from the truth, which is not a rescue"
        );
        // And the clicks were not already the answer: the solver improved
        // on them, which is the division of labour this tool assumes.
        let clicked = error(&start);
        assert!(
            clicked > found,
            "the picked pose was {clicked} away and the registration {found} — \
             if the clicks were already right, the test is measuring nothing"
        );
    }

    /// The correction multiplies the noise and nothing else.
    ///
    /// It is a statement about how many measurements are really
    /// independent, not about how accurate the application needs to be, and
    /// a viewer that applied it to the tolerance instead would classify
    /// every scene wrongly while looking entirely plausible.
    /// Each side is prepared with its own voxel, and both reach the solver.
    ///
    /// The whole of the per-scan parameter change is that two numbers
    /// Asking the session to search must not quietly start from the
    /// identity.
    ///
    /// The two ways of starting go down the same call and end in the same
    /// `Registration`, and nothing in the result records which was used —
    /// deliberately, so that the report cannot come to depend on it. That
    /// leaves nobody to notice if the search were dropped on the way, so it
    /// is noticed here: a welded tee turned fifty degrees. From the
    /// identity ICP comes back thirty-two degrees out; the search comes
    /// back on it.
    #[test]
    fn searching_reaches_a_pose_the_identity_does_not() {
        let scene = Scene::generate(
            SceneKind::TeeJoint,
            SceneParams {
                points_per_face: 20_000,
                noise_sigma: 0.002,
                ..SceneParams::default()
            },
        );
        let turn = Se3::exp(&na::Vector6::new(
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            50f64.to_radians(),
        ));
        let moved = transform_cloud(&scene.cloud, &turn.inverse());

        let held = |cloud: PointCloud, generation: u64| Held {
            cloud: Arc::new(cloud),
            generation,
        };
        let source = held(moved, 1);
        let target = held(scene.cloud.clone(), 2);
        let prepare = PrepareParams::default();
        let params = RegisterParams::default();

        let run = |start: Start| {
            Session::default()
                .register(
                    &source,
                    &target,
                    &prepare,
                    &prepare,
                    &params,
                    start,
                    &|| false,
                    &mut |_| {},
                    &mut |_| {},
                )
                .expect("the registration failed")
                .expect("the registration was abandoned")
        };
        let off = |found: &Registration| {
            (found.result.pose.rotation().log() - turn.rotation().log()).norm()
        };

        let cold = run(Start::At(Se3::identity()));
        assert!(
            off(&cold) > 0.10,
            "the identity already recovered the turn, to {} rad — this scene no longer \
             tests anything",
            off(&cold)
        );

        let searched = run(Start::Search);
        assert!(
            off(&searched) < 0.02,
            "the search came back {} rad from the turn, which is where the identity \
             would have left it",
            off(&searched)
        );
    }

    /// travel where one did. Nothing about the pose or the residual would
    /// notice if the second were dropped on the way and the first used for
    /// both — the run would converge and look right — so what is asserted
    /// is the count each side kept, which is the one number that can only
    /// come from that side's own voxel.
    #[test]
    fn a_pair_is_prepared_one_voxel_each() {
        let (source, target, _) = corridor_pair("per-scan-voxel");
        let held = |path: &PathBuf, generation: u64| Held {
            cloud: Arc::new(rigidity_io::read_ply(path).expect("the fixture would not read")),
            generation,
        };
        let coarse = PrepareParams {
            voxel: 0.20,
            neighbours: 16,
        };
        let fine = PrepareParams {
            voxel: 0.05,
            neighbours: 16,
        };

        let run = |source_prepare: PrepareParams, target_prepare: PrepareParams| {
            Session::default()
                .register(
                    &held(&source, 1),
                    &held(&target, 2),
                    &source_prepare,
                    &target_prepare,
                    &RegisterParams::default(),
                    Start::At(Se3::identity()),
                    &|| false,
                    &mut |_| {},
                    &mut |_| {},
                )
                .expect("the registration failed")
                .expect("the registration was abandoned")
                .points
        };

        // What each voxel gives when both sides use it, so the mixed run
        // below has something to be equal to rather than merely different
        // from.
        let [both_coarse, _] = run(coarse, coarse);
        let [_, both_fine] = run(fine, fine);
        assert!(
            both_coarse < both_fine,
            "the coarse voxel kept {both_coarse} and the fine one {both_fine}"
        );

        let [moving, fixed] = run(coarse, fine);
        assert_eq!(moving, both_coarse, "the source did not use its own voxel");
        assert_eq!(fixed, both_fine, "the target did not use its own voxel");
    }

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
