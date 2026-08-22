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
use rigidity_core::nalgebra as na;
use rigidity_pipeline::{PipelineError, PrepareParams, Progress, RegisterParams};
use rigidity_scenes::{Scene, SceneKind, SceneParams};

pub(crate) mod job;
pub(crate) mod session;

pub(crate) use job::{Demo, Derivation, Event, Held, Job, Lane, Step};
use session::Session;

/// A handle to the worker thread.
pub(crate) struct Engine {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    /// The request the application still wants an answer to, per lane.
    ///
    /// Shared with the worker, which abandons anything else. One counter
    /// per lane serves both cancellation and supersession, because to the
    /// worker they are the same fact: nobody is waiting.
    wanted: Arc<[AtomicU64; Lane::COUNT]>,
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
        let wanted = Arc::new([const { AtomicU64::new(0) }; Lane::COUNT]);
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

    /// Queues a built-in scene and its displaced copy.
    pub(crate) fn scene(&self, demo: Demo) {
        self.send(Job::Scene(demo));
    }

    /// Queues an analysis, superseding any earlier request.
    ///
    /// Returns the identifier: events carry it, and the application ignores
    /// answers to questions it has stopped asking.
    pub(crate) fn analyse(&mut self, target: Held, prepare: PrepareParams) -> u64 {
        let id = self.issue(Lane::Report);
        self.send(Job::Analyse {
            id,
            target,
            prepare,
        });
        id
    }

    /// Queues a selection.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn select(
        &mut self,
        from: Held,
        matrix: [f32; 16],
        viewport: [f32; 2],
        polygon: Vec<[f32; 2]>,
        slab: ([f32; 3], f32, f32),
    ) -> u64 {
        let id = self.issue(Lane::Scene);
        self.send(Job::Select {
            id,
            from,
            matrix,
            viewport,
            polygon,
            slab,
        });
        id
    }

    /// Queues a pick.
    pub(crate) fn pick(
        &mut self,
        from: Held,
        matrix: [f32; 16],
        viewport: [f32; 2],
        at: [f32; 2],
        slab: ([f32; 3], f32, f32),
    ) -> u64 {
        let id = self.issue(Lane::Scene);
        self.send(Job::Pick {
            id,
            from,
            matrix,
            viewport,
            at,
            slab,
        });
        id
    }

    /// Queues a new cloud made from an old one.
    pub(crate) fn derive(&self, from: Held, how: Derivation, name: String) {
        self.send(Job::Derive { from, how, name });
    }

    /// Queues a distance field, superseding any earlier request.
    pub(crate) fn measure(&mut self, from: Held, to: Held, pose: Se3) -> u64 {
        let id = self.issue(Lane::Measure);
        self.send(Job::Distance { id, from, to, pose });
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
        let id = self.issue(Lane::Report);
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

    fn issue(&mut self, lane: Lane) -> u64 {
        self.issued += 1;
        self.wanted[lane.index()].store(self.issued, Ordering::Relaxed);
        self.issued
    }

    /// Abandons everything in flight, in every lane.
    pub(crate) fn cancel(&mut self) {
        self.issue(Lane::Report);
        self.issue(Lane::Measure);
        self.issue(Lane::Scene);
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
    wanted: &[AtomicU64; Lane::COUNT],
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
                    generated: false,
                    bounds: bounds_of(&cloud),
                    cloud: Arc::new(cloud),
                    seconds: start.elapsed().as_secs_f64(),
                },
                Err(source) => Event::Failed(PipelineError::Read { path, source }),
            };
            emit(events, ctx, event);
        }

        Job::Distance { id, from, to, pose } => {
            let stale = || wanted[Lane::Measure.index()].load(Ordering::Relaxed) != id;
            let entry = from.generation;
            let mut progress = |done, total| {
                emit(
                    events,
                    ctx,
                    Event::Working {
                        id,
                        step: Step {
                            label: "measuring",
                            done,
                            total,
                        },
                    },
                );
            };
            let event = match session.distances(&from, &to, &pose, &stale, &mut progress) {
                Ok(Some(values)) => Event::Measured { id, entry, values },
                Ok(None) => Event::Abandoned { id },
                Err(error) => Event::Failed(error),
            };
            emit(events, ctx, event);
        }

        Job::Select {
            id,
            from,
            matrix,
            viewport,
            polygon,
            slab,
        } => {
            let stale = || wanted[Lane::Scene.index()].load(Ordering::Relaxed) != id;
            let entry = from.generation;
            let event =
                match session::select(&from.cloud, &matrix, viewport, &polygon, &slab, &stale) {
                    Some(indices) => Event::Selected {
                        id,
                        entry,
                        indices: Arc::new(indices),
                    },
                    None => Event::Abandoned { id },
                };
            emit(events, ctx, event);
        }

        Job::Pick {
            id,
            from,
            matrix,
            viewport,
            at,
            slab,
        } => {
            // Ten points of slack: a click is not a pixel, and a cloud
            // dense enough to look solid still has gaps a click can fall
            // into.
            let found = session::nearest(&from.cloud, &matrix, viewport, at, 10.0, &slab);
            emit(
                events,
                ctx,
                Event::Picked {
                    id,
                    at: found.map(|point| [point.x, point.y, point.z]),
                },
            );
        }

        Job::Derive { from, how, name } => {
            let start = Instant::now();
            let event = match session::derive(&from.cloud, &how) {
                Ok(cloud) => Event::Loaded {
                    path: PathBuf::from(name),
                    generated: true,
                    bounds: bounds_of(&cloud),
                    cloud: Arc::new(cloud),
                    seconds: start.elapsed().as_secs_f64(),
                },
                Err(error) => Event::Failed(error),
            };
            emit(events, ctx, event);
        }

        Job::Scene(demo) => {
            let start = Instant::now();
            for (name, cloud) in demo_pair(demo) {
                let bounds = bounds_of(&cloud);
                emit(
                    events,
                    ctx,
                    Event::Loaded {
                        path: PathBuf::from(name),
                        generated: true,
                        cloud: Arc::new(cloud),
                        bounds,
                        seconds: start.elapsed().as_secs_f64(),
                    },
                );
            }
        }

        Job::Analyse {
            id,
            target,
            prepare,
        } => {
            let start = Instant::now();
            let stale = || wanted[Lane::Report.index()].load(Ordering::Relaxed) != id;
            let mut progress = |progress: Progress| {
                emit(
                    events,
                    ctx,
                    Event::Working {
                        id,
                        step: progress.into(),
                    },
                );
            };

            let event = match session.analyse(&target, &prepare, &stale, &mut progress) {
                Ok(Some((analysis, surface))) => Event::Analysed {
                    id,
                    analysis: Box::new(analysis),
                    surface: Box::new(surface),
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
            let stale = || wanted[Lane::Report.index()].load(Ordering::Relaxed) != id;
            let mut progress = |progress: Progress| {
                emit(
                    events,
                    ctx,
                    Event::Working {
                        id,
                        step: progress.into(),
                    },
                );
            };
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

/// A demo scene and a copy of it displaced by a known amount.
///
/// Target first, then source, which is the order the application fills its
/// two roles in. The displacement is the one the CLI's own demo uses, so
/// what appears on screen can be checked against the README.
fn demo_pair(demo: Demo) -> [(String, PointCloud); 2] {
    let (kind, name) = match demo {
        Demo::Corridor => (SceneKind::Corridor, "corridor"),
        Demo::Corner => (SceneKind::Corner, "corner"),
        Demo::Sphere => (SceneKind::Sphere, "sphere"),
    };
    let scene = Scene::generate(
        kind,
        SceneParams {
            points_per_face: 60_000,
            // Two millimetres, about what a good terrestrial scanner
            // leaves at these ranges. A noiseless demo would flatter the
            // report.
            noise_sigma: 0.002,
            ..SceneParams::default()
        },
    );
    let motion = Se3::exp(&na::Vector6::new(0.03, 0.02, 0.01, 0.0, 0.0, 0.0));
    let moved = rigidity_pipeline::transform_cloud(&scene.cloud, &motion);
    [
        (format!("{name}-target.ply"), scene.cloud),
        (format!("{name}-source.ply"), moved),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every demo produces two clouds a registration can actually be run
    /// on, displaced by the amount the copied command line claims.
    ///
    /// The button is the only caller, and a button cannot be pressed from
    /// a test — so the work behind it is a function, and this is that
    /// function's test.
    #[test]
    fn every_demo_makes_a_registrable_pair() {
        for demo in [Demo::Corridor, Demo::Corner, Demo::Sphere] {
            let [(target_name, target), (source_name, source)] = demo_pair(demo);
            assert!(target_name.ends_with("-target.ply"), "{target_name}");
            assert!(source_name.ends_with("-source.ply"), "{source_name}");
            assert!(target.len() > 10_000, "{demo:?}: {} points", target.len());
            assert_eq!(target.len(), source.len());

            let offset = source.point(0) - target.point(0);
            let expected = na::Vector3::new(0.03, 0.02, 0.01);
            assert!(
                (offset - expected).norm() < 1e-6,
                "{demo:?}: displaced by {offset:?}"
            );
        }
    }
}
