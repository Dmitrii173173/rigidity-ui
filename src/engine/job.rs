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

use super::session::Registration;

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

/// Work for the engine.
pub(crate) enum Job {
    /// Read a point cloud from disk.
    Load(PathBuf),
    /// Prepare a surface and report what its geometry would determine.
    Analyse {
        /// Which request this is.
        id: u64,
        /// What to analyse.
        target: Held,
        /// How to prepare it first.
        prepare: PrepareParams,
    },
    /// Register one surface onto another and report what the answer is worth.
    Register {
        /// Which request this is.
        id: u64,
        /// What moves.
        source: Held,
        /// What it moves onto.
        target: Held,
        /// How to prepare both.
        prepare: PrepareParams,
        /// How to register them.
        params: RegisterParams,
        /// Where to start from.
        initial: Se3,
    },
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
        progress: Progress,
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
        /// How many points survived downsampling.
        points: usize,
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
    /// Something went wrong.
    Failed(PipelineError),
}
