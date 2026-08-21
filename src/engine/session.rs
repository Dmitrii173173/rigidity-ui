//! The state the worker owns, and the work it does with it.
//!
//! Everything here calls `rigidity-pipeline` and nothing else: the same
//! functions, in the same order, with the same defaults as the command
//! line. That is not tidiness, it is the only reason the two agree — and
//! the test at the bottom of this file is what keeps them agreeing.

use std::sync::Arc;

use rigidity_core::PointCloud;
use rigidity_core::observability::Analysis;
use rigidity_pipeline::{PipelineError, PrepareParams, Prepared, Progress, analyse_cloud};

/// What the worker keeps between requests.
#[derive(Default)]
pub(crate) struct Session {
    /// The last surface prepared, kept because M3 registers against it.
    prepared: Option<Prepared>,
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
        cloud: &Arc<PointCloud>,
        params: &PrepareParams,
        stale: &dyn Fn() -> bool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Option<(Analysis, usize)>, PipelineError> {
        if stale() {
            return Ok(None);
        }

        let prepared = rigidity_pipeline::prepare_cloud_observed(cloud, params, |report| {
            progress(report);
        })?;
        if stale() {
            return Ok(None);
        }

        let analysis = analyse_cloud(&prepared)?;
        let points = prepared.len();
        self.prepared = Some(prepared);
        Ok(Some((analysis, points)))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use rigidity_core::observability::ObservabilityCriteria;
    use rigidity_pipeline::ReportParams;
    use rigidity_scenes::{Scene, SceneKind, SceneParams};

    use super::*;

    /// Where `../rigidity` lives, relative to this crate.
    fn rigidity() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rigidity")
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
    ///
    /// The comparison is against the CLI's own printed report, byte for
    /// byte, because that is the artefact people quote at each other.
    #[test]
    fn the_report_matches_the_command_line() {
        let scene = Scene::generate(
            SceneKind::Corridor,
            SceneParams {
                points_per_face: 20_000,
                noise_sigma: 0.002,
                ..SceneParams::default()
            },
        );
        let path = std::env::temp_dir().join("rigidity-ui-parity-corridor.ply");
        rigidity_io::write_ply(&scene.cloud, &path).expect("the fixture could not be written");

        // Defaults on both sides, deliberately: the parameters the two
        // front ends disagree about first are the ones nobody typed.
        let prepare = PrepareParams::default();
        let report = ReportParams::default();

        let mut session = Session::default();
        let cloud = Arc::new(scene.cloud);
        let (analysis, _points) = session
            .analyse(&cloud, &prepare, &|| false, &mut |_| {})
            .expect("the viewer's pipeline failed")
            .expect("the viewer's pipeline was abandoned with nothing to abandon it");
        let ours = analysis.describe(&report.criteria());

        let build = Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "rigidity-cli", "--manifest-path"])
            .arg(rigidity().join("Cargo.toml"))
            .status()
            .expect("cargo could not be run");
        assert!(build.success(), "the command line would not build");

        let output = Command::new(rigidity().join("target/debug/rigidity"))
            .arg("analyse")
            .arg(&path)
            .args(["--voxel", &prepare.voxel.to_string()])
            .args(["--neighbours", &prepare.neighbours.to_string()])
            .args(["--noise", &report.noise.to_string()])
            .args(["--tolerance", &report.tolerance.to_string()])
            .output()
            .expect("the command line could not be run");
        let theirs = String::from_utf8(output.stdout).expect("the report was not text");

        assert!(
            theirs.contains(&ours),
            "the viewer and the command line disagree.\n\
             --- viewer ---\n{ours}\n--- command line ---\n{theirs}"
        );
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
