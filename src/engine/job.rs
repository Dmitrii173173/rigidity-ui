//! What the engine can be asked to do, and what it says back.
//!
//! Requests carry an identifier and the engine only finishes the one the
//! application still wants. Dragging the voxel slider queues a request per
//! frame; the engine abandons every stale one at the next stage boundary
//! and runs the last. The same mechanism answers Esc, because a cancelled
//! request and a superseded one are the same fact to the worker — nobody
//! is waiting for the answer.

use std::path::PathBuf;
use std::sync::Arc;

use rigidity_core::PointCloud;
use rigidity_core::icp::IterationReport;
use rigidity_core::lie::Se3;
use rigidity_core::observability::Analysis;
use rigidity_pipeline::{PipelineError, PrepareParams, Progress, RegisterParams};

/// How far along a piece of work is, in the viewer's own terms.
///
/// The pipeline's `Progress` names the stages the pipeline has; a distance
/// computation is none of them, and adding a variant upstream for
/// something the pipeline does not do would be the wrong way round. One
/// label and a fraction covers both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step {
    /// What is happening.
    pub(crate) label: &'static str,
    /// How much of it is finished.
    pub(crate) done: usize,
    /// How much there is.
    pub(crate) total: usize,
}

impl From<Progress> for Step {
    fn from(progress: Progress) -> Self {
        Self {
            label: progress.stage.label(),
            done: progress.done,
            total: progress.total,
        }
    }
}

use super::session::{Registration, Surface};

/// Which built-in scene to generate.
///
/// Three, not the seven `rigidity-scenes` offers: one that is degenerate
/// by construction, one that is not, and one that is curved. A menu of
/// seven would be a menu; three is an explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Demo {
    /// A corridor: translation along it is unobservable.
    Corridor,
    /// A trihedral corner: every degree of freedom is determined.
    Corner,
    /// A sphere: rotation about its centre is unobservable.
    Sphere,
}

impl Demo {
    /// The name shown on the button.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Corridor => "corridor",
            Self::Corner => "corner",
            Self::Sphere => "sphere",
        }
    }
}

/// A cloud and a number that changes when the cloud does.
///
/// The engine caches prepared surfaces, and a million points are expensive
/// to compare; the counter is how it knows whether the cache still holds.
#[derive(Clone)]
pub(crate) struct Held {
    /// The points.
    pub(crate) cloud: Arc<PointCloud>,
    /// Bumped whenever this becomes a different cloud.
    pub(crate) generation: u64,
}

/// Which queue a request belongs to.
///
/// Requests supersede each other *within* a lane and not across them. One
/// counter for everything meant that colouring a cloud by its distance to
/// another abandoned the registration that was still running — two
/// different questions, and answering the second is no reason to stop
/// answering the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    /// Anything that ends in a conditioning report.
    Report,
    /// Anything that ends in a scalar field.
    Measure,
    /// Anything that ends in a new cloud in the list.
    Scene,
}

impl Lane {
    /// Its slot in the engine's table of wanted requests.
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Report => 0,
            Self::Measure => 1,
            Self::Scene => 2,
        }
    }

    /// How many there are.
    pub(crate) const COUNT: usize = 3;
}

/// How a new cloud is made from an old one.
///
/// Never *to* an old one: every operation in this application produces a
/// new entry and leaves its input alone, which is what makes undo free and
/// a save-before-quit dialog unnecessary.
#[derive(Debug, Clone)]
pub(crate) enum Derivation {
    /// Keep the selected points.
    Keep(Arc<Vec<u32>>),
    /// Keep everything else.
    Drop(Arc<Vec<u32>>),
    /// One point per voxel, deterministically.
    Subsample(f64),
}

impl Derivation {
    /// What the result is called, given what it came from.
    pub(crate) fn name(&self, from: &str) -> String {
        let stem = from.strip_suffix(".ply").unwrap_or(from);
        match self {
            Self::Keep(_) => format!("{stem} (kept).ply"),
            Self::Drop(_) => format!("{stem} (rest).ply"),
            Self::Subsample(voxel) => format!("{stem} ({voxel:.3} m).ply"),
        }
    }
}

/// Work for the engine.
pub(crate) enum Job {
    /// Read a point cloud from disk.
    Load(PathBuf),
    /// Generate a scene with an analytically known null space, and a copy
    /// of it displaced by a known amount.
    ///
    /// The application explains itself in one click to someone who has no
    /// dataset to hand, which is most people the first time.
    Scene(Demo),
    /// Prepare a surface and report what its geometry would determine.
    Analyse {
        /// Which request this is.
        id: u64,
        /// What to analyse.
        target: Held,
        /// How to prepare it first.
        prepare: PrepareParams,
    },
    /// Measure how far each point of one cloud is from another.
    ///
    /// At full density, both sides: this is the number people quote about
    /// two scans, and quoting it about a downsampled copy would understate
    /// it by however coarse the voxel was.
    Distance {
        /// Which request this is.
        id: u64,
        /// The cloud being measured.
        from: Held,
        /// What it is measured against.
        to: Held,
        /// Where `from` sits when the question is asked.
        ///
        /// After a registration that is the pose the solver found, because
        /// the distance people want is the one that is left *after*
        /// aligning — and because the cloud on screen has moved, and
        /// colours painted at its old position would be describing
        /// somewhere it no longer is.
        pose: Se3,
    },
    /// Find which points of a cloud fall inside a shape drawn on screen.
    Select {
        /// Which request this is.
        id: u64,
        /// The cloud being selected from.
        from: Held,
        /// World to clip, column-major, with the cloud's model folded in.
        matrix: [f32; 16],
        /// The viewport, in the pixels the shape is measured in.
        viewport: [f32; 2],
        /// The shape, closed implicitly.
        polygon: Vec<[f32; 2]>,
        /// The cross-section in force, so what is selected is what was
        /// visible.
        slab: ([f32; 3], f32, f32),
    },
    /// Find the point nearest a place on the screen.
    Pick {
        /// Which request this is.
        id: u64,
        /// The cloud being picked from.
        from: Held,
        /// World to clip, column-major, with the cloud's model folded in.
        matrix: [f32; 16],
        /// The viewport, in the pixels `at` is measured in.
        viewport: [f32; 2],
        /// Where the pointer was.
        at: [f32; 2],
        /// The cross-section in force.
        slab: ([f32; 3], f32, f32),
    },
    /// Write a cloud to disk, in whatever format the extension names.
    Save {
        /// What to write.
        from: Held,
        /// Where, and — by its extension — how.
        path: PathBuf,
    },
    /// Make a new cloud from an existing one.
    ///
    /// No request identifier: it is one pass over a cloud, it lands in the
    /// scene list like any other load, and there is nothing to supersede
    /// it with.
    Derive {
        /// What it comes from.
        from: Held,
        /// What is done to it.
        how: Derivation,
        /// What the result is called.
        name: String,
    },
    /// Register one surface onto another and report what the answer is worth.
    Register {
        /// Which request this is.
        id: u64,
        /// What moves.
        source: Held,
        /// What it moves onto.
        target: Held,
        /// How to prepare the source.
        source_prepare: PrepareParams,
        /// How to prepare the target.
        ///
        /// Separately, because a scan taken up against a wall and one taken
        /// across a hall are not the same density, and making the pair
        /// agree lets the coarser of the two decide for both.
        target_prepare: PrepareParams,
        /// How to register them.
        params: RegisterParams,
        /// Where to start from.
        start: Start,
    },
}

/// Where a registration begins.
///
/// Not an `Option<Se3>`: the identity is a perfectly good pose to start
/// from and means "begin here", while searching means "I do not know". An
/// option would spell both of those the same way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Start {
    /// Begin at this pose. What every registration did before the search
    /// existed, and still the right thing whenever anything is known: the
    /// previous leg of a survey, a manual alignment, the identity.
    At(Se3),
    /// Do not assume one — lay out starts and keep the candidate whose
    /// median residual is smallest.
    Search,
}

/// What the engine reports back.
pub(crate) enum Event {
    /// A file was opened and is being read.
    Started(PathBuf),
    /// A cloud arrived.
    Loaded {
        /// Where it came from.
        path: PathBuf,
        /// The points, shared rather than copied — the renderer takes a
        /// handle to the same allocation.
        cloud: Arc<PointCloud>,
        /// Whether the cloud was generated rather than read.
        ///
        /// The command line the viewer can hand back has to name files,
        /// and a generated scene has none — so it names the `rigidity
        /// scene` invocations that would produce them first.
        generated: bool,
        /// A coarse copy for drawing while the camera moves, when the
        /// cloud is big enough to need one.
        ///
        /// Built here rather than on demand because it is a whole-cloud
        /// pass and this thread is the one that is allowed to take a
        /// second. See `engine::coarse` and PLAN.md §8, S1.
        coarse: Option<Arc<PointCloud>>,
        /// The bounding box, in absolute coordinates.
        ///
        /// Absolute rather than local because two clouds have two origins,
        /// and which frame they are drawn in is the application's choice,
        /// not the reader's.
        bounds: Option<([f64; 3], [f64; 3])>,
        /// How long the read took.
        seconds: f64,
    },
    /// A stage of a request is under way.
    Working {
        /// Which request.
        id: u64,
        /// How far it has got.
        step: Step,
    },
    /// A picked point arrived, or nothing was near enough.
    Picked {
        /// Which request.
        id: u64,
        /// Where it is, in absolute coordinates.
        at: Option<[f64; 3]>,
    },
    /// A selection arrived.
    Selected {
        /// Which request.
        id: u64,
        /// Which cloud the indices are into.
        entry: u64,
        /// The points inside the shape.
        indices: Arc<Vec<u32>>,
    },
    /// A distance field arrived.
    Measured {
        /// Which request.
        id: u64,
        /// Which cloud the values belong to.
        entry: u64,
        /// One distance per point of it, metres.
        values: Vec<f32>,
    },
    /// An accepted ICP iteration.
    ///
    /// Sent as it happens, not collected at the end: this is what the
    /// viewer draws while the solver is still working, and what the
    /// timeline is made of afterwards.
    Iteration {
        /// Which request.
        id: u64,
        /// The pose, residual and correspondence count after it.
        report: IterationReport,
    },
    /// An analysis finished.
    Analysed {
        /// Which request.
        id: u64,
        /// The conditioning of the prepared surface.
        analysis: Box<Analysis>,
        /// The surface it is about.
        surface: Box<Surface>,
        /// How long the whole request took.
        seconds: f64,
    },
    /// A registration finished.
    Registered {
        /// Which request.
        id: u64,
        /// Everything it produced.
        outcome: Box<Registration>,
        /// How long the whole request took.
        seconds: f64,
    },
    /// A request was superseded or cancelled before it finished.
    Abandoned {
        /// Which request.
        id: u64,
    },
    /// A cloud reached the disk.
    Saved {
        /// Where it went.
        path: PathBuf,
        /// How long it took.
        seconds: f64,
    },
    /// Something went wrong.
    Failed(PipelineError),
}
