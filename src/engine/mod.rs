//! The worker thread.
//!
//! Everything that calls into `rigidity` happens here, off the frame loop.
//! Principle §1 is not a preference: preparing a million points takes long
//! enough that doing it inline would drop every frame of the wait, and the
//! one thing a diagnostic must never do is stop responding while it thinks.
//!
//! Nothing in this module may import `egui` for anything but the repaint
//! signal, and nothing may import `wgpu` at all.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Instant;

use rigidity_core::PointCloud;
use rigidity_core::icp::IterationReport;
use rigidity_core::lie::Se3;
use rigidity_pipeline::{PipelineError, PrepareParams, RegisterParams};

pub(crate) mod job;
pub(crate) mod session;

pub(crate) use job::{Event, Held, Job};
use session::Session;

/// A handle to the worker thread.
pub(crate) struct Engine {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    /// The request the application still wants an answer to.
    ///
    /// Shared with the worker, which abandons anything else. One counter
    /// serves both cancellation and supersession, because to the worker
    /// they are the same fact: nobody is waiting.
    wanted: Arc<AtomicU64>,
    /// The last identifier handed out.
    issued: u64,
}

impl Engine {
    /// Starts the thread.
    ///
    /// The context is cloned in so that the worker can ask for a repaint
    /// when it has something to show. Without it the application would sit
    /// still until the user moved the mouse.
    pub(crate) fn spawn(ctx: eframe::egui::Context) -> Self {
        let (job_sender, job_receiver) = channel::<Job>();
        let (event_sender, event_receiver) = channel::<Event>();
        let wanted = Arc::new(AtomicU64::new(0));
        let worker_wanted = Arc::clone(&wanted);

        thread::Builder::new()
            .name("rigidity-engine".to_owned())
            .spawn(move || {
                let mut session = Session::default();
                // Ends when the sender is dropped, which happens when the
                // application does.
                for job in job_receiver {
                    run(job, &mut session, &worker_wanted, &event_sender, &ctx);
                }
            })
            .expect("the engine thread could not be started");

        Self {
            jobs: job_sender,
            events: event_receiver,
            wanted,
            issued: 0,
        }
    }

    /// Queues a file to read.
    pub(crate) fn load(&self, path: PathBuf) {
        self.send(Job::Load(path));
    }

    /// Queues an analysis, superseding any earlier request.
    ///
    /// Returns the identifier: events carry it, and the application ignores
    /// answers to questions it has stopped asking.
    pub(crate) fn analyse(&mut self, target: Held, prepare: PrepareParams) -> u64 {
        let id = self.issue();
        self.send(Job::Analyse {
            id,
            target,
            prepare,
        });
        id
    }

    /// Queues a registration, superseding any earlier request.
    pub(crate) fn register(
        &mut self,
        source: Held,
        target: Held,
        prepare: PrepareParams,
        params: RegisterParams,
        initial: Se3,
    ) -> u64 {
        let id = self.issue();
        self.send(Job::Register {
            id,
            source,
            target,
            prepare,
            params,
            initial,
        });
        id
    }

    fn issue(&mut self) -> u64 {
        self.issued += 1;
        self.wanted.store(self.issued, Ordering::Relaxed);
        self.issued
    }

    /// Abandons whatever is in flight.
    pub(crate) fn cancel(&mut self) {
        self.issue();
    }

    /// Everything reported since the last frame.
    pub(crate) fn poll(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }

    /// Dropping the result is deliberate: a dead engine means the
    /// application is closing, and there is nobody left to tell.
    fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

fn run(
    job: Job,
    session: &mut Session,
    wanted: &AtomicU64,
    events: &Sender<Event>,
    ctx: &eframe::egui::Context,
) {
    match job {
        Job::Load(path) => {
            emit(events, ctx, Event::Started(path.clone()));
            let start = Instant::now();
            let event = match rigidity_io::read_ply(&path) {
                Ok(cloud) => Event::Loaded {
                    path,
                    bounds: bounds_of(&cloud),
                    cloud: Arc::new(cloud),
                    seconds: start.elapsed().as_secs_f64(),
                },
                Err(source) => Event::Failed(PipelineError::Read { path, source }),
            };
            emit(events, ctx, event);
        }

        Job::Analyse {
            id,
            target,
            prepare,
        } => {
            let start = Instant::now();
            let stale = || wanted.load(Ordering::Relaxed) != id;
            let mut progress = |progress| emit(events, ctx, Event::Working { id, progress });

            let event = match session.analyse(&target, &prepare, &stale, &mut progress) {
                Ok(Some((analysis, points))) => Event::Analysed {
                    id,
                    analysis: Box::new(analysis),
                    points,
                    seconds: start.elapsed().as_secs_f64(),
                },
                Ok(None) => Event::Abandoned { id },
                Err(error) => Event::Failed(error),
            };
            emit(events, ctx, event);
        }

        Job::Register {
            id,
            source,
            target,
            prepare,
            params,
            initial,
        } => {
            let start = Instant::now();
            let stale = || wanted.load(Ordering::Relaxed) != id;
            let mut progress = |progress| emit(events, ctx, Event::Working { id, progress });
            // Every accepted iteration crosses the channel as it happens.
            // The viewer draws the pose it carries, so the source moves
            // while the solver is still working rather than jumping once
            // at the end.
            let mut iteration = |report: &IterationReport| {
                emit(
                    events,
                    ctx,
                    Event::Iteration {
                        id,
                        report: *report,
                    },
                );
            };

            let event = match session.register(
                &source,
                &target,
                &prepare,
                &params,
                initial,
                &stale,
                &mut progress,
                &mut iteration,
            ) {
                Ok(Some(outcome)) => Event::Registered {
                    id,
                    outcome: Box::new(outcome),
                    seconds: start.elapsed().as_secs_f64(),
                },
                Ok(None) => Event::Abandoned { id },
                Err(error) => Event::Failed(error),
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

/// The bounding box, as an array rather than as `nalgebra` vectors.
///
/// It is a pass over every point, which is exactly the kind of work the
/// frame loop cannot afford, so it happens here.
fn bounds_of(cloud: &PointCloud) -> Option<([f64; 3], [f64; 3])> {
    let (min, max) = cloud.bounds()?;
    Some(([min.x, min.y, min.z], [max.x, max.y, max.z]))
}
