//! The worker thread.
//!
//! M1 asks one thing of it: reading a file must not happen on the frame
//! loop. A million-point PLY takes long enough that a synchronous read
//! would drop frames on the first file anyone opens, which principle §1
//! forbids. M2 grows this into the full job queue — prepare, register,
//! analyse, cancel — and the shape here is chosen to be grown rather than
//! replaced.
//!
//! Nothing in this module may import `egui` for anything but the repaint
//! signal, and nothing may import `wgpu` at all.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Instant;

use rigidity_core::PointCloud;
use rigidity_pipeline::PipelineError;

/// Work for the engine.
pub(crate) enum Job {
    /// Read a point cloud from disk.
    Load(PathBuf),
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
        ///
        /// Computed here rather than by the caller because it is a pass
        /// over every point, and the frame loop is the one place that
        /// cannot afford one.
        bounds: Option<([f32; 3], [f32; 3])>,
        /// How long the read took.
        seconds: f64,
    },
    /// It did not arrive.
    Failed(PipelineError),
}

/// A handle to the worker thread.
pub(crate) struct Engine {
    jobs: Sender<Job>,
    events: Receiver<Event>,
}

impl Engine {
    /// Starts the thread.
    ///
    /// The context is cloned in so that the worker can ask for a repaint
    /// when it has something to show. Without it the application would
    /// sit still until the user moved the mouse.
    pub(crate) fn spawn(ctx: eframe::egui::Context) -> Self {
        let (job_sender, job_receiver) = channel::<Job>();
        let (event_sender, event_receiver) = channel::<Event>();

        thread::Builder::new()
            .name("rigidity-engine".to_owned())
            .spawn(move || {
                // Ends when the sender is dropped, which happens when the
                // application does.
                for job in job_receiver {
                    run(job, &event_sender, &ctx);
                }
            })
            .expect("the engine thread could not be started");

        Self {
            jobs: job_sender,
            events: event_receiver,
        }
    }

    /// Queues work. Dropping the result is deliberate: a dead engine means
    /// the application is closing, and there is nobody left to tell.
    pub(crate) fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    /// Everything reported since the last frame.
    pub(crate) fn poll(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
}

fn run(job: Job, events: &Sender<Event>, ctx: &eframe::egui::Context) {
    match job {
        Job::Load(path) => {
            emit(events, ctx, Event::Started(path.clone()));
            let start = Instant::now();
            let event = match rigidity_io::read_ply(&path) {
                Ok(cloud) => Event::Loaded {
                    path,
                    bounds: local_bounds(&cloud),
                    cloud: Arc::new(cloud),
                    seconds: start.elapsed().as_secs_f64(),
                },
                Err(source) => Event::Failed(PipelineError::Read { path, source }),
            };
            emit(events, ctx, event);
        }
    }
}

fn emit(events: &Sender<Event>, ctx: &eframe::egui::Context, event: Event) {
    if events.send(event).is_ok() {
        ctx.request_repaint();
    }
}

/// The bounding box relative to the cloud's origin.
///
/// `PointCloud::bounds` answers in absolute coordinates; the renderer and
/// the camera both work in the local frame, so the origin comes off here
/// once instead of at every use.
fn local_bounds(cloud: &PointCloud) -> Option<([f32; 3], [f32; 3])> {
    let (min, max) = cloud.bounds()?;
    let origin = cloud.origin();
    let local = |v: rigidity_core::nalgebra::Vector3<f64>| {
        let v = v - origin;
        [v.x as f32, v.y as f32, v.z as f32]
    };
    Some((local(min), local(max)))
}
