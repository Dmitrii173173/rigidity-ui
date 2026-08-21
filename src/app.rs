//! The shell: what is on screen, where, and what the mouse does to it.
//!
//! The application has no modes. One cloud loaded is a question about a
//! surface — *if anything were registered against this, what would it
//! determine?* — and the answer appears without being asked for. A second
//! cloud makes it a registration. There is nothing a mode switch could tell
//! the application that the scene does not already say.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, Key, Layout, Modifiers, PointerButton, Rect, RichText, Sense, Vec2,
};
use rigidity_core::PointCloud;
use rigidity_core::icp::IterationReport;
use rigidity_core::lie::Se3;
use rigidity_core::nalgebra as na;
use rigidity_core::observability::{Analysis, Conditioning, ObservabilityCriteria};
use rigidity_pipeline::{PipelineError, PrepareParams, Progress, RegisterParams, ReportParams};

use crate::bench::Bench;
use crate::engine::session::Registration;
use crate::engine::{Engine, Event, Held};
use crate::render::camera::Camera;
use crate::render::{self, CloudDraw, ViewportCallback};
use crate::theme::{self, Mode, Palette, space};
use crate::{spectrum, timeline};

/// A cloud, ready to look at.
struct Scene {
    path: PathBuf,
    cloud: Arc<PointCloud>,
    /// Bumped on every load, so the renderer and the engine's cache can
    /// tell one cloud from the next without comparing a million points.
    generation: u64,
    min: na::Vector3<f64>,
    max: na::Vector3<f64>,
    seconds: f64,
}

impl Scene {
    fn name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cloud")
    }

    fn held(&self) -> Held {
        Held {
            cloud: Arc::clone(&self.cloud),
            generation: self.generation,
        }
    }
}

/// What the engine is doing for us.
enum Work {
    Idle,
    Running { id: u64, progress: Option<Progress> },
    Cancelled,
}

/// What it has produced.
enum Outcome {
    Nothing,
    Analysis {
        analysis: Box<Analysis>,
        points: usize,
        seconds: f64,
    },
    Registration {
        outcome: Box<Registration>,
        seconds: f64,
    },
}

/// The application.
pub(crate) struct App {
    mode: Mode,
    bench: Option<Bench>,
    engine: Engine,
    gpu: bool,
    target: Option<Scene>,
    source: Option<Scene>,
    reading: Option<PathBuf>,
    failure: Option<PipelineError>,
    work: Work,
    outcome: Outcome,
    /// Every accepted iteration of the last run, in order.
    iterations: Vec<IterationReport>,
    /// Which of them the viewport is showing.
    scrub: usize,
    residual_colours: bool,
    prepare: PrepareParams,
    report: ReportParams,
    registration: RegisterParams,
    camera: Camera,
    point_size: f32,
    edl_strength: f32,
    generation: u64,
}

impl App {
    /// Builds the app, the GPU pipelines and the worker thread.
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, open: Vec<PathBuf>) -> Self {
        let mode = Mode::Dark;
        theme::apply(&cc.egui_ctx, mode);
        let engine = Engine::spawn(cc.egui_ctx.clone());
        for path in open {
            engine.load(path);
        }
        Self {
            mode,
            bench: Bench::from_environment(),
            engine,
            gpu: render::install(cc.wgpu_render_state.as_ref()),
            target: None,
            source: None,
            reading: None,
            failure: None,
            work: Work::Idle,
            outcome: Outcome::Nothing,
            iterations: Vec::new(),
            scrub: 0,
            residual_colours: false,
            // The command line's defaults, deliberately. The parameters two
            // front ends disagree about first are the ones nobody typed.
            prepare: PrepareParams::default(),
            report: ReportParams::default(),
            registration: RegisterParams::default(),
            camera: Camera::default(),
            // Three points across, not one: at one physical pixel a splat
            // covers less than the average spacing of a real scan and the
            // surface comes out as noise.
            point_size: 3.0,
            edl_strength: 300.0,
            generation: 0,
        }
    }

    fn palette(&self) -> Palette {
        Palette::of(self.mode)
    }

    /// The frame everything is drawn in: the target's origin, or the
    /// source's while there is no target.
    ///
    /// Some origin has to be chosen, and it has to be one of theirs. A
    /// georeferenced pair sits half a million metres from zero, where `f32`
    /// steps in centimetres.
    fn origin(&self) -> na::Vector3<f64> {
        self.target
            .as_ref()
            .or(self.source.as_ref())
            .map(|scene| scene.cloud.origin())
            .unwrap_or_else(na::Vector3::zeros)
    }

    /// Everything loaded, boxed together, in that frame.
    fn bounds(&self) -> Option<(na::Point3<f32>, na::Point3<f32>)> {
        let origin = self.origin();
        let mut span: Option<(na::Vector3<f64>, na::Vector3<f64>)> = None;
        for scene in [self.target.as_ref(), self.source.as_ref()]
            .into_iter()
            .flatten()
        {
            span = Some(match span {
                None => (scene.min, scene.max),
                Some((min, max)) => (min.inf(&scene.min), max.sup(&scene.max)),
            });
        }
        let (min, max) = span?;
        let local = |v: na::Vector3<f64>| {
            let v = v - origin;
            na::Point3::new(v.x as f32, v.y as f32, v.z as f32)
        };
        Some((local(min), local(max)))
    }

    /// The pose the source is being shown at.
    fn pose(&self) -> Se3 {
        self.iterations
            .get(self.scrub)
            .map(|report| report.pose)
            .unwrap_or_else(Se3::identity)
    }

    /// Takes everything the worker has reported since the last frame.
    fn drain_events(&mut self) {
        for event in self.engine.poll().collect::<Vec<_>>() {
            match event {
                Event::Started(path) => self.reading = Some(path),

                Event::Loaded {
                    path,
                    cloud,
                    bounds,
                    seconds,
                } => {
                    self.generation += 1;
                    let (min, max) = bounds.unwrap_or(([0.0; 3], [0.0; 3]));
                    let scene = Scene {
                        path,
                        cloud,
                        generation: self.generation,
                        min: min.into(),
                        max: max.into(),
                        seconds,
                    };
                    // The first cloud is the one to register *against*,
                    // which is also the one a single-cloud analysis asks
                    // about. The second becomes what moves.
                    if self.target.is_none() {
                        self.target = Some(scene);
                    } else {
                        self.source = Some(scene);
                    }
                    self.iterations.clear();
                    self.reading = None;
                    self.failure = None;
                    if let Some((min, max)) = self.bounds() {
                        self.camera.fit(min, max);
                    }
                    self.request();
                }

                // Answers to questions that are no longer being asked are
                // dropped here rather than checked for everywhere below.
                Event::Working { id, progress } => {
                    if self.current() == Some(id) {
                        self.work = Work::Running {
                            id,
                            progress: Some(progress),
                        };
                    }
                }
                Event::Iteration { id, report } => {
                    if self.current() == Some(id) {
                        self.iterations.push(report);
                        // Follow the solver unless the pointer has taken
                        // the timeline somewhere else.
                        self.scrub = self.iterations.len() - 1;
                    }
                }
                Event::Analysed {
                    id,
                    analysis,
                    points,
                    seconds,
                } => {
                    if self.current() == Some(id) {
                        self.outcome = Outcome::Analysis {
                            analysis,
                            points,
                            seconds,
                        };
                        self.work = Work::Idle;
                    }
                }
                Event::Registered {
                    id,
                    outcome,
                    seconds,
                } => {
                    if self.current() == Some(id) {
                        self.outcome = Outcome::Registration { outcome, seconds };
                        self.work = Work::Idle;
                    }
                }
                Event::Abandoned { id } => {
                    if self.current() == Some(id) {
                        self.work = Work::Cancelled;
                    }
                }

                Event::Failed(error) => {
                    self.failure = Some(error);
                    self.reading = None;
                    self.work = Work::Idle;
                }
            }
        }
    }

    fn current(&self) -> Option<u64> {
        match self.work {
            Work::Running { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Asks the engine whatever the scene makes it sensible to ask.
    ///
    /// One cloud is a question about a surface; two are a registration.
    /// The initial pose is whatever the timeline is showing, so scrubbing
    /// back to a plausible iteration and running again from there needs no
    /// separate control.
    fn request(&mut self) {
        let id = match (&self.source, &self.target) {
            (Some(source), Some(target)) => {
                let (source, target) = (source.held(), target.held());
                let initial = self.pose();
                self.iterations.clear();
                self.scrub = 0;
                self.engine
                    .register(source, target, self.prepare, self.registration, initial)
            }
            (None, Some(target)) => {
                let target = target.held();
                self.engine.analyse(target, self.prepare)
            }
            _ => return,
        };
        self.work = Work::Running { id, progress: None };
    }

    /// Asks for a file and queues it.
    fn open(&self) {
        // The dialog blocks, and on every platform this application runs on
        // it is modal anyway: there is nothing behind it to interact with.
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("point cloud", &["ply"])
            .pick_file()
        {
            self.engine.load(path);
        }
    }

    fn shortcuts(&mut self, ui: &egui::Ui) {
        let (open, fit, cancel, run, dropped) = ui.input(|input| {
            (
                input.modifiers.matches_logically(Modifiers::COMMAND) && input.key_pressed(Key::O),
                input.key_pressed(Key::F),
                input.key_pressed(Key::Escape),
                input.key_pressed(Key::Space),
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect::<Vec<_>>(),
            )
        });
        if open {
            self.open();
        }
        if fit && let Some((min, max)) = self.bounds() {
            self.camera.fit(min, max);
        }
        if cancel && matches!(self.work, Work::Running { .. }) {
            self.engine.cancel();
            self.work = Work::Cancelled;
        }
        if run && !matches!(self.work, Work::Running { .. }) {
            self.request();
        }
        // Dropping is how a file actually gets opened; the dialog is the
        // fallback for people who do not know that yet.
        for path in dropped {
            self.engine.load(path);
        }
    }

    /// The registration's residual, if there is one to have.
    fn rmse(&self) -> Option<f64> {
        match &self.outcome {
            Outcome::Registration { outcome, .. } => Some(outcome.result.rmse),
            _ => None,
        }
    }

    /// Whether the report may be believed.
    ///
    /// Conditioning describes the local shape of the cost function around
    /// wherever the solver stopped. Inside a wrong minimum the surfaces
    /// agree just as tightly and the spectrum looks just as confident — on
    /// real data, eleven of thirty pairs converged to the wrong basin with
    /// conditioning no worse than the successful ones. The residual is the
    /// only thing that separates them, so it decides whether the spectrum
    /// is drawn as an answer or as a suspicion.
    fn trusted(&self) -> bool {
        self.rmse()
            .is_none_or(|rmse| rmse <= WRONG_BASIN_FACTOR * self.report.noise)
    }
}

/// How far the residual may exceed the stated sensor noise before the
/// report stops being believable. A converged registration leaves
/// residuals of about the noise; three times that is generous.
const WRONG_BASIN_FACTOR: f64 = 3.0;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.drain_events();
        self.shortcuts(ui);
        if let Some(bench) = &mut self.bench {
            bench.step(ui.ctx(), &mut self.camera, self.target.is_some());
        }
        let palette = self.palette();

        // Order decides nesting: the status strip is added first so that it
        // spans the full width, then the timeline above it, then the
        // inspector beside both.
        self.status_strip(ui, &palette, frame);
        self.inspector(ui, &palette);
        // After the inspector, so it belongs to the viewport rather than
        // running underneath the panel.
        self.timeline_strip(ui, &palette);
        self.viewport(ui, &palette);
    }
}

impl App {
    /// The bottom strip: what the application is doing, and on what hardware.
    fn status_strip(&self, ui: &mut egui::Ui, palette: &Palette, frame: &eframe::Frame) {
        let backend = frame
            .wgpu_render_state
            .as_ref()
            .map(|state| {
                let info = state.adapter.get_info();
                format!("{} · {:?}", info.name, info.backend)
            })
            .unwrap_or_else(|| "no gpu".to_owned());

        egui::Panel::bottom("status")
            .exact_size(26.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(space::PANEL, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let (dot, text) = self.status_line(palette);
                    bullet(ui, dot);
                    ui.add_space(space::TIGHT + 2.0);
                    ui.label(RichText::new(text).color(palette.muted).size(11.0));

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let fps = ui.ctx().input(|input| input.stable_dt).recip();
                        ui.label(
                            RichText::new(format!("{fps:.0} fps"))
                                .color(palette.faint)
                                .size(11.0)
                                .monospace(),
                        );
                        ui.add_space(space::GROUP);
                        ui.label(RichText::new(backend).color(palette.faint).size(11.0));
                    });
                });
            });
    }

    fn status_line(&self, palette: &Palette) -> (Color32, String) {
        if let Some(error) = &self.failure {
            return (palette.low, error.to_string());
        }
        if let Some(path) = &self.reading {
            return (palette.medium, format!("reading {}…", file_name(path)));
        }
        match &self.work {
            Work::Running { progress, .. } => {
                let text = match progress {
                    _ if !self.iterations.is_empty() => format!(
                        "iteration {} · rmse {:.2e} m   esc to stop",
                        self.iterations.len(),
                        self.iterations.last().map_or(f64::NAN, |it| it.rmse)
                    ),
                    Some(Progress { stage, done, total }) if *total > 0 => format!(
                        "{} · {:.0}%   esc to stop",
                        stage.label(),
                        100.0 * *done as f32 / *total as f32
                    ),
                    _ => "preparing…   esc to stop".to_owned(),
                };
                return (palette.medium, text);
            }
            // A stop between stages is the honest granularity while
            // preparing; inside the solver it is per accepted iteration.
            Work::Cancelled => return (palette.faint, "stopped".to_owned()),
            Work::Idle => {}
        }
        match &self.outcome {
            Outcome::Registration { outcome, seconds } => (
                if self.trusted() {
                    palette.high
                } else {
                    palette.medium
                },
                format!(
                    "rmse {:.2e} m · {} correspondences · {} iterations · {} · {:.2} s",
                    outcome.result.rmse,
                    thousands(outcome.result.correspondences),
                    outcome.result.iterations,
                    if outcome.result.converged {
                        "converged"
                    } else {
                        "stopped at the iteration cap"
                    },
                    seconds
                ),
            ),
            Outcome::Analysis {
                points, seconds, ..
            } => (
                palette.high,
                format!(
                    "{} points, {} after downsampling · analysed in {:.2} s",
                    self.target
                        .as_ref()
                        .map_or(0, |scene| scene.cloud.len())
                        .pipe(thousands),
                    thousands(*points),
                    seconds
                ),
            ),
            Outcome::Nothing => (palette.faint, "drop a PLY file here".to_owned()),
        }
    }

    /// The iteration strip, shown only once there are iterations.
    fn timeline_strip(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        if self.iterations.is_empty() {
            return;
        }
        let reports = std::mem::take(&mut self.iterations);
        let chosen = egui::Panel::bottom("timeline")
            .exact_size(timeline::HEIGHT)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(space::PANEL, 0)),
            )
            .show(ui, |ui| timeline::show(ui, palette, &reports, self.scrub))
            .inner;
        self.iterations = reports;
        if let Some(index) = chosen {
            self.scrub = index;
        }
    }

    /// The left panel: what is loaded, what it determines, and on what terms.
    fn inspector(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        egui::Panel::left("inspector")
            .default_size(300.0)
            .size_range(260.0..=420.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::same(space::PANEL)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("rigidity").color(palette.text).size(15.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if quiet_button(ui, palette, self.mode.label(), "switch theme") {
                            self.mode = self.mode.flipped();
                            theme::apply(ui.ctx(), self.mode);
                        }
                    });
                });
                ui.add_space(space::GROUP);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.clouds_section(ui, palette);
                        ui.add_space(space::GROUP);
                        self.spectrum_section(ui, palette);
                        ui.add_space(space::GROUP);
                        self.parameters_section(ui, palette);
                    });
            });
    }

    fn clouds_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        // How many points each side kept, once there has been a
        // registration to keep them for. Downsampling is the largest thing
        // that happens to a cloud between the file and the answer, and
        // leaving it unsaid invites the reader to assume the report is
        // about every point they loaded.
        let kept = match &self.outcome {
            Outcome::Registration { outcome, .. } => Some(outcome.points),
            _ => None,
        };
        for (index, (label, scene, colour)) in [
            ("target — fixed", &self.target, palette.point),
            ("source — moves", &self.source, palette.point_moving),
        ]
        .into_iter()
        .enumerate()
        {
            heading(ui, palette, label);
            match scene {
                Some(scene) => {
                    ui.horizontal(|ui| {
                        bullet(ui, colour);
                        ui.add_space(space::TIGHT);
                        ui.label(RichText::new(scene.name()).color(palette.text));
                    });
                    // `points` is ordered source, target — the order
                    // the solver takes them in — while the panel reads
                    // target first, so the index flips here.
                    let used = kept.map(|points| points[1 - index]);
                    ui.label(
                        RichText::new(match used {
                            Some(used) => format!(
                                "{} points, {} used",
                                thousands(scene.cloud.len()),
                                thousands(used)
                            ),
                            None => format!(
                                "{} points · read in {:.2} s",
                                thousands(scene.cloud.len()),
                                scene.seconds
                            ),
                        })
                        .color(palette.muted)
                        .size(11.0),
                    );
                }
                None => {
                    ui.label(RichText::new("—").color(palette.faint));
                }
            }
            ui.add_space(space::ROW);
        }

        ui.horizontal(|ui| {
            if quiet_button(ui, palette, "open…", "⌘O") {
                self.open();
            }
            if self.source.is_some() && quiet_button(ui, palette, "swap", "exchange the roles") {
                std::mem::swap(&mut self.source, &mut self.target);
                self.iterations.clear();
                self.request();
            }
        });
    }

    fn spectrum_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        heading(ui, palette, "spectrum");
        let criteria = self.report.criteria();
        let trusted = self.trusted();
        let analysis = match &self.outcome {
            Outcome::Analysis { analysis, .. } => Some(&**analysis),
            Outcome::Registration { outcome, .. } => Some(&outcome.analysis),
            Outcome::Nothing => None,
        };
        let Some(analysis) = analysis else {
            match self.work {
                Work::Running { .. } => {
                    ui.label(RichText::new("working…").color(palette.faint).size(11.0));
                }
                Work::Cancelled => {
                    ui.label(RichText::new("stopped").color(palette.faint).size(11.0));
                    if quiet_button(ui, palette, "run again", "space") {
                        self.request();
                    }
                }
                Work::Idle => {
                    ui.label(
                        RichText::new("load a cloud to see what its geometry would determine")
                            .color(palette.faint)
                            .size(11.0),
                    );
                }
            }
            return;
        };

        let conditioning = &analysis.conditioning;
        let hovered = spectrum::show(ui, palette, conditioning, &criteria, trusted);
        ui.add_space(space::ROW);

        if !trusted {
            let ratio = self.rmse().unwrap_or_default() / self.report.noise.max(f64::MIN_POSITIVE);
            ui.label(
                RichText::new(format!(
                    "the residual is {ratio:.0}× the stated sensor noise. this may be a \
                     wrong minimum, and the spectrum cannot tell you"
                ))
                .color(palette.low)
                .size(11.0),
            );
            ui.add_space(space::ROW);
        }

        let states = conditioning.classify(&criteria);
        ui.label(
            RichText::new(spectrum::verdict(&states))
                .color(if trusted { palette.text } else { palette.faint })
                .size(11.0),
        );

        // A detail line that is always populated: the hovered row, or the
        // worst one. An empty area where an explanation belongs teaches
        // people to ignore that part of the screen.
        let index = hovered.unwrap_or_else(|| worst(conditioning, &criteria));
        let direction = conditioning.direction_in_world(index);
        ui.add_space(space::TIGHT);
        ui.label(
            RichText::new(format!(
                "σ{}  {}",
                index + 1,
                states[index].label().to_lowercase()
            ))
            .color(palette.muted)
            .size(11.0),
        );
        for (name, offset) in [("ρ", 0), ("φ", 3)] {
            ui.label(
                RichText::new(format!(
                    "{name} {:+.2} {:+.2} {:+.2}",
                    direction[offset],
                    direction[offset + 1],
                    direction[offset + 2]
                ))
                .color(palette.faint)
                .size(10.0)
                .monospace(),
            );
        }
        ui.add_space(space::TIGHT);
        ui.label(
            RichText::new(format!(
                "κ {:.2e} · {} correspondences",
                conditioning.condition_number(),
                thousands(conditioning.used())
            ))
            .color(palette.faint)
            .size(10.0)
            .monospace(),
        );

        if let Outcome::Registration { outcome, .. } = &self.outcome {
            ui.add_space(space::GROUP);
            heading(ui, palette, "pose found");
            let translation = outcome.result.pose.translation();
            let rotation = outcome.result.pose.rotation().log();
            for line in [
                format!(
                    "x {:+8.4}  y {:+8.4}  z {:+8.4}  m",
                    translation.x, translation.y, translation.z
                ),
                format!(
                    "r {:+8.3}  p {:+8.3}  y {:+8.3}  °",
                    rotation.x.to_degrees(),
                    rotation.y.to_degrees(),
                    rotation.z.to_degrees()
                ),
            ] {
                ui.label(
                    RichText::new(line)
                        .color(palette.muted)
                        .size(10.0)
                        .monospace(),
                );
            }
            ui.add_space(space::ROW);
            let mut residuals = self.residual_colours;
            if ui
                .checkbox(
                    &mut residuals,
                    RichText::new("colour by residual").size(11.0),
                )
                .on_hover_text("on the downsampled source, which is what the solver used")
                .changed()
            {
                self.residual_colours = residuals;
            }
        }
    }

    fn parameters_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        heading(ui, palette, "report");
        // These three cost nothing to change: `Conditioning` holds the
        // spectrum and both `uncertainty` and `classify` are pure functions
        // of it, so the lines move and the bars recolour within the frame.
        let mut report = self.report;
        let (noise, tolerance, correction) = (
            format!("{:.4} m", report.noise),
            format!("{:.4} m", report.tolerance),
            format!("×{:.0}", report.calibration),
        );
        slider(
            ui,
            palette,
            "sensor noise",
            &mut report.noise,
            1e-4..=1.0,
            true,
            &noise,
        );
        slider(
            ui,
            palette,
            "tolerance",
            &mut report.tolerance,
            1e-5..=1.0,
            true,
            &tolerance,
        );
        slider(
            ui,
            palette,
            "correction",
            &mut report.calibration,
            1.0..=50.0,
            false,
            &correction,
        );
        self.report = report;

        ui.add_space(space::ROW);
        heading(ui, palette, "prepare");
        // These do cost something: the cloud is downsampled, indexed and
        // re-normalled, and then registered again. The engine abandons
        // superseded requests at the next stage boundary, so dragging is
        // safe if not free.
        let mut prepare = self.prepare;
        let (voxel, neighbours) = (
            format!("{:.3} m", prepare.voxel),
            prepare.neighbours.to_string(),
        );
        let mut changed = slider(
            ui,
            palette,
            "voxel",
            &mut prepare.voxel,
            1e-3..=1.0,
            true,
            &voxel,
        );
        changed |= slider(
            ui,
            palette,
            "neighbours",
            &mut prepare.neighbours,
            4..=64,
            false,
            &neighbours,
        );

        let mut registration = self.registration;
        if self.source.is_some() {
            ui.add_space(space::ROW);
            heading(ui, palette, "register");
            let (distance, huber) = (
                format!("{:.2} m", registration.max_distance),
                format!("{:.3} m", registration.huber),
            );
            changed |= slider(
                ui,
                palette,
                "max correspondence",
                &mut registration.max_distance,
                1e-2..=10.0,
                true,
                &distance,
            );
            changed |= slider(
                ui,
                palette,
                "huber threshold",
                &mut registration.huber,
                1e-3..=1.0,
                true,
                &huber,
            );
        }

        if changed {
            self.prepare = prepare;
            self.registration = registration;
            self.request();
        }
    }

    /// The viewport: everything that is not a panel.
    fn viewport(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            ui.painter().rect_filled(rect, 0.0, palette.background);
            if rect.width() < 1.0 || rect.height() < 1.0 {
                return;
            }
            if !self.gpu {
                centred_note(ui, palette, rect, "no gpu adapter — nothing to draw on");
                return;
            }
            let draws = self.draws(palette);
            if draws.is_empty() {
                centred_note(ui, palette, rect, "drop a PLY file here, or ⌘O");
                return;
            }

            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            self.navigate(ui, &response, rect);

            let pixels_per_point = ui.ctx().pixels_per_point();
            let size = viewport_pixels(rect, pixels_per_point);
            let aspect = size[0] as f32 / size[1].max(1) as f32;
            let mut view_projection = [0.0f32; 16];
            view_projection.copy_from_slice(self.camera.view_projection(aspect).as_slice());

            ui.painter()
                .add(eframe::egui_wgpu::Callback::new_paint_callback(
                    rect,
                    ViewportCallback {
                        draws,
                        view_projection,
                        size,
                        point_size: self.point_size * pixels_per_point,
                        edl_strength: self.edl_strength,
                        // Two points of screen distance. A one-pixel
                        // neighbourhood only sees the depth step at a
                        // silhouette; two picks up the curvature across a
                        // crease, which is most of what there is to see in
                        // a building.
                        edl_radius: (2.0 * pixels_per_point).round().max(1.0),
                    },
                ));
        });
    }

    /// What the renderer should draw this frame.
    fn draws(&self, palette: &Palette) -> Vec<CloudDraw> {
        let origin = self.origin();
        let mut draws = Vec::with_capacity(2);

        if let Some(target) = &self.target {
            draws.push(CloudDraw {
                cloud: Arc::clone(&target.cloud),
                generation: target.generation,
                model: model(&Se3::identity(), target.cloud.origin(), origin),
                colour: palette.point.to_normalized_gamma_f32(),
                hot: palette.point.to_normalized_gamma_f32(),
                scalar: None,
                scalar_generation: 0,
                range: [0.0, 1.0],
            });
        }

        if let Some(source) = &self.source {
            let pose = self.pose();
            // With residual colouring on, the cloud shown is the one the
            // solver actually used. Colouring the full-density source would
            // mean a nearest-neighbour query per raw point to say the same
            // thing, and would quietly imply the residual was computed
            // there.
            let sampled = match (&self.outcome, self.residual_colours) {
                (Outcome::Registration { outcome, .. }, true) => Some(outcome),
                _ => None,
            };
            let (cloud, generation, scalar, scalar_generation, range) = match sampled {
                Some(outcome) => (
                    Arc::clone(&outcome.sampled),
                    source.generation ^ (1 << 63),
                    Some(Arc::clone(&outcome.residuals)),
                    source.generation,
                    outcome.span,
                ),
                None => (
                    Arc::clone(&source.cloud),
                    source.generation,
                    None,
                    0,
                    [0.0, 1.0],
                ),
            };
            draws.push(CloudDraw {
                cloud: cloud.clone(),
                generation,
                model: model(&pose, cloud.origin(), origin),
                colour: palette.point_moving.to_normalized_gamma_f32(),
                hot: palette.low.to_normalized_gamma_f32(),
                scalar,
                scalar_generation,
                range,
            });
        }

        draws
    }

    /// Mouse and wheel over the viewport.
    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect) {
        let delta = response.drag_delta();
        if response.dragged_by(PointerButton::Primary) {
            self.camera.orbit([delta.x, delta.y]);
        } else if response.dragged_by(PointerButton::Secondary)
            || response.dragged_by(PointerButton::Middle)
        {
            self.camera.pan([delta.x, delta.y], rect.height());
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.dolly(scroll * 0.05);
                ui.ctx().request_repaint();
            }
        }
    }
}

/// A cloud's placement: its own origin, a pose, and the frame everything is
/// drawn in.
///
/// `p_frame = pose · (origin_cloud + local) − origin_frame`, which is one
/// rotation and one translation once the constants are folded. The
/// arithmetic is `f64` and only the result narrows, so a scene half a
/// million metres from zero keeps its millimetres.
fn model(pose: &Se3, cloud_origin: na::Vector3<f64>, frame_origin: na::Vector3<f64>) -> [f32; 16] {
    let matrix = pose.matrix();
    let rotation = matrix.fixed_view::<3, 3>(0, 0);
    let translation: na::Vector3<f64> = matrix.fixed_view::<3, 1>(0, 3).into();
    let offset = rotation * cloud_origin + translation - frame_origin;

    let mut out = [0.0f32; 16];
    for column in 0..3 {
        for row in 0..3 {
            out[column * 4 + row] = rotation[(row, column)] as f32;
        }
    }
    out[12] = offset.x as f32;
    out[13] = offset.y as f32;
    out[14] = offset.z as f32;
    out[15] = 1.0;
    out
}

/// The row that most deserves an explanation: the least determined one.
fn worst(conditioning: &Conditioning, criteria: &ObservabilityCriteria) -> usize {
    let spreads = conditioning.uncertainty(criteria.noise_sigma);
    (0..6).fold(0, |worst, index| {
        if spreads[index] > spreads[worst] {
            index
        } else {
            worst
        }
    })
}

/// The physical-pixel rectangle egui will set as the viewport.
///
/// Rounded exactly as `epaint::ViewportInPixels` rounds it: the offscreen
/// textures have to be the size of the region the composite covers, or the
/// image is resampled by a fraction of a pixel and every splat softens.
fn viewport_pixels(rect: Rect, pixels_per_point: f32) -> [u32; 2] {
    let left = (rect.min.x * pixels_per_point).round();
    let right = (rect.max.x * pixels_per_point).round();
    let top = (rect.min.y * pixels_per_point).round();
    let bottom = (rect.max.y * pixels_per_point).round();
    [
        (right - left).max(1.0) as u32,
        (bottom - top).max(1.0) as u32,
    ]
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_owned()
}

/// `1234567` as `1 234 567`. A point count is read, not calculated with.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(digit);
    }
    out
}

/// Lets a value be passed to a function at the end of a chain.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

/// A section heading: small, quiet, and never underlined.
fn heading(ui: &mut egui::Ui, palette: &Palette, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .color(palette.muted)
            .size(10.0),
    );
    ui.add_space(space::TIGHT);
}

/// A button with no frame until it is wanted.
fn quiet_button(ui: &mut egui::Ui, palette: &Palette, text: &str, hint: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.0).color(palette.muted))
            .fill(Color32::TRANSPARENT),
    )
    .on_hover_text(hint)
    .clicked()
}

/// A labelled slider, laid out as label above rail rather than beside it:
/// the panel is narrow and a side label steals the half of the width that
/// makes the rail worth dragging.
fn slider<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    logarithmic: bool,
    reading: &str,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(palette.muted).size(11.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(reading)
                    .color(palette.faint)
                    .size(11.0)
                    .monospace(),
            );
        });
    });
    // The handle is a circle centred on the end of the rail, so a rail that
    // spans the full width has half a handle outside the panel.
    let handle = ui.spacing().slider_rail_height.max(10.0);
    ui.spacing_mut().slider_width = ui.available_width() - handle;
    let changed = ui
        .horizontal(|ui| {
            ui.add_space(handle * 0.5);
            ui.add(
                egui::Slider::new(value, range)
                    .logarithmic(logarithmic)
                    .show_value(false),
            )
            .changed()
        })
        .inner;
    ui.add_space(space::ROW);
    changed
}

/// The status dot.
fn bullet(ui: &mut egui::Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, colour);
}

/// A single line in the middle of an empty viewport.
fn centred_note(ui: &egui::Ui, palette: &Palette, rect: Rect, text: &str) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        palette.faint,
    );
}
