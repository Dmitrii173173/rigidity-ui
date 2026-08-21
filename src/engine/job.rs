//! What the engine can be asked to do, and what it says back.
//!
//! Requests carry an identifier and the engine only finishes the one the
//! application still wants. Dragging the voxel slider queues a request per
//! frame; the engine abandons every stale one at the next stage boundary
//! and runs the last. The same mechanism answers Esc, because a cancelled
//! request and a superseded one are the same thing to the worker — nobody
//! is waiting for the answer.

use std::path::PathBuf;
use std::sync::Arc;

use rigidity_core::PointCloud;
use rigidity_core::observability::Analysis;
use rigidity_pipeline::{PipelineError, PrepareParams, Progress};

/// Work for the engine.
pub(crate) enum Job {
    /// Read a point cloud from disk.
    Load(PathBuf),
    /// Downsample, index, estimate normals, and report what the geometry
    /// would determine.
    Analyse {
        /// Which request this is.
        id: u64,
        /// What to analyse.
        cloud: Arc<PointCloud>,
        /// How to prepare it first.
        params: PrepareParams,
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
        /// The bounding box, in the local coordinates the GPU sees.
        bounds: Option<([f32; 3], [f32; 3])>,
        /// How long the read took.
        seconds: f64,
    },
    /// A stage of an analysis is under way.
    Working {
        /// Which request.
        id: u64,
        /// How far it has got.
        progress: Progress,
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
    /// A request was superseded or cancelled before it finished.
    Abandoned {
        /// Which request.
        id: u64,
    },
    /// Something went wrong.
    Failed(PipelineError),
}
