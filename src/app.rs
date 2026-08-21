//! The shell: what is on screen, where, and what the mouse does to it.
//!
//! The application has no modes. One cloud loaded is a question about a
//! surface — *if anything were registered against this, what would it
//! determine?* — and the answer appears without being asked for. A second
//! cloud will make it a registration (M3). There is nothing a mode switch
//! could tell the application that the scene does not already say.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, Key, Layout, Modifiers, PointerButton, Rect, RichText, Sense, Vec2,
};
use rigidity_core::PointCloud;
use rigidity_core::nalgebra as na;
use rigidity_core::observability::Analysis;
use rigidity_pipeline::{PipelineError, PrepareParams, Progress, ReportParams};

use crate::bench::Bench;
use crate::engine::{Engine, Event};
use crate::render::camera::Camera;
use crate::render::{self, ViewportCallback};
use crate::spectrum;
use crate::theme::{self, Mode, Palette, space};

/// A cloud, ready to look at.
struct Scene {
    path: PathBuf,
    cloud: Arc<PointCloud>,
    /// Bumped on every load, so the renderer can tell one cloud from the
    /// next without comparing a million points.
    generation: u64,
    min: na::Point3<f32>,
    max: na::Point3<f32>,
    seconds: f64,
}

impl Scene {
    /// The longest side of the bounding box, for the status strip.
    fn extent(&self) -> f32 {
        (self.max - self.min).amax()
    }

    fn name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cloud")
    }
}

/// What the analysis is doing.
enum Work {
    /// Nothing has been asked for.
    Idle,
    /// A request is in flight.
    Running { id: u64, progress: Option<Progress> },
    /// It finished.
    Done {
        analysis: Box<Analysis>,
        points: usize,
        seconds: f64,
    },
    /// It was cancelled, and the user should know it was not merely slow.
    Cancelled,
}

/// The application.
pub(crate) struct App {
    mode: Mode,
    bench: Option<Bench>,
    engine: Engine,
    gpu: bool,
    scene: Option<Scene>,
    reading: Option<PathBuf>,
    failure: Option<PipelineError>,
    work: Work,
    prepare: PrepareParams,
    report: ReportParams,
    camera: Camera,
    point_size: f32,
    edl_strength: f32,
    generation: u64,
}

impl App {
    /// Builds the app, the GPU pipelines and the worker thread.
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let mode = Mode::Dark;
        theme::apply(&cc.egui_ctx, mode);
        let engine = Engine::spawn(cc.egui_ctx.clone());
        if let Some(path) = open {
            engine.load(path);
        }
        Self {
            mode,
            bench: Bench::from_environment(),
            engine,
            gpu: render::install(cc.wgpu_render_state.as_ref()),
            scene: None,
            reading: None,
            failure: None,
            work: Work::Idle,
            // The command line's defaults, deliberately. The parameters two
            // front ends disagree about first are the ones nobody typed.
            prepare: PrepareParams::default(),
            report: ReportParams::default(),
            camera: Camera::default(),
            // Three points across, not one: at one physical pixel a splat
            // covers less than the average spacing of a real scan and the
            // surface comes out as noise. Three closes the gaps without
            // turning the cloud into paste.
            point_size: 3.0,
            edl_strength: 300.0,
            generation: 0,
        }
    }

    fn palette(&self) -> Palette {
        Palette::of(self.mode)
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
                    self.camera.fit(scene.min, scene.max);
                    self.scene = Some(scene);
                    self.reading = None;
                    self.failure = None;
                    self.request_analysis();
                }

                // Answers to questions that are no longer being asked are
                // dropped here rather than checked for everywhere below.
                Event::Working { id, progress } => {
                    if let Work::Running { id: current, .. } = &self.work
                        && *current == id
                    {
                        self.work = Work::Running {
                            id,
                            progress: Some(progress),
                        };
                    }
                }
                Event::Analysed {
                    id,
                    analysis,
                    points,
                    seconds,
                } => {
                    if matches!(&self.work, Work::Running { id: current, .. } if *current == id) {
                        self.work = Work::Done {
                            analysis,
                            points,
                            seconds,
                        };
                    }
                }
                Event::Abandoned { id } => {
                    if matches!(&self.work, Work::Running { id: current, .. } if *current == id) {
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

    /// Asks the engine what the loaded surface would determine.
    fn request_analysis(&mut self) {
        let Some(cloud) = self.scene.as_ref().map(|scene| Arc::clone(&scene.cloud)) else {
            return;
        };
        let id = self.engine.analyse(cloud, self.prepare);
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
        if fit && let Some(scene) = &self.scene {
            self.camera.fit(scene.min, scene.max);
        }
        if cancel && matches!(self.work, Work::Running { .. }) {
            self.engine.cancel();
            self.work = Work::Cancelled;
        }
        if run && !matches!(self.work, Work::Running { .. }) {
            self.request_analysis();
        }
        // Dropping is how a file actually gets opened; the dialog is the
        // fallback for people who do not know that yet.
        for path in dropped {
            self.engine.load(path);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.drain_events();
        self.shortcuts(ui);
        if let Some(bench) = &mut self.bench {
            bench.step(ui.ctx(), &mut self.camera, self.scene.is_some());
        }
        let palette = self.palette();

        // Order decides nesting: the status strip is added first so that it
        // spans the full width, and the inspector sits above it.
        self.status_strip(ui, &palette, frame);
        self.inspector(ui, &palette);
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
        match (&self.work, &self.scene) {
            (Work::Running { progress, .. }, _) => {
                let text = match progress {
                    Some(Progress { stage, done, total }) if *total > 0 => format!(
                        "{} · {:.0}%   esc to stop",
                        stage.label(),
                        100.0 * *done as f32 / *total as f32
                    ),
                    _ => "preparing…   esc to stop".to_owned(),
                };
                (palette.medium, text)
            }
            // A stop between stages is the honest granularity: the core
            // reports progress from inside a pass over the points but does
            // not take an answer back, so a pass finishes before anyone is
            // asked anything.
            (Work::Cancelled, _) => (palette.faint, "stopped after the current stage".to_owned()),
            (
                Work::Done {
                    points, seconds, ..
                },
                Some(scene),
            ) => (
                palette.high,
                format!(
                    "{} points, {} after downsampling · {:.1} m across · analysed in {:.2} s",
                    thousands(scene.cloud.len()),
                    thousands(*points),
                    scene.extent(),
                    seconds
                ),
            ),
            (_, Some(scene)) => (
                palette.high,
                format!(
                    "{} points · {:.1} m across · read in {:.2} s",
                    thousands(scene.cloud.len()),
                    scene.extent(),
                    scene.seconds
                ),
            ),
            (_, None) => (palette.faint, "drop a PLY file here".to_owned()),
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
                self.cloud_section(ui, palette);
                ui.add_space(space::GROUP);
                self.spectrum_section(ui, palette);
                ui.add_space(space::GROUP);
                self.parameters_section(ui, palette);
            });
    }

    fn cloud_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        heading(ui, palette, "cloud");
        match &self.scene {
            Some(scene) => {
                ui.label(RichText::new(scene.name()).color(palette.text));
                ui.label(
                    RichText::new(format!("{} points", thousands(scene.cloud.len())))
                        .color(palette.muted)
                        .size(11.0),
                );
            }
            None => {
                ui.label(RichText::new("—").color(palette.faint));
            }
        }
        ui.add_space(space::ROW);
        if quiet_button(ui, palette, "open…", "⌘O") {
            self.open();
        }
    }

    fn spectrum_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        heading(ui, palette, "spectrum");
        let criteria = self.report.criteria();
        let Work::Done { analysis, .. } = &self.work else {
            match self.work {
                Work::Running { .. } => {
                    ui.label(RichText::new("working…").color(palette.faint).size(11.0));
                }
                Work::Cancelled => {
                    ui.label(RichText::new("stopped").color(palette.faint).size(11.0));
                    if quiet_button(ui, palette, "run again", "space") {
                        self.request_analysis();
                    }
                }
                _ => {
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
        let hovered = spectrum::show(ui, palette, conditioning, &criteria);

        ui.add_space(space::ROW);
        let states = conditioning.classify(&criteria);
        ui.label(
            RichText::new(spectrum::verdict(&states))
                .color(palette.text)
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
    }

    fn parameters_section(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        heading(ui, palette, "report");
        // These three cost nothing to change: `Conditioning` holds the
        // spectrum and both `uncertainty` and `classify` are pure functions
        // of it, so the lines move and the bars recolour within the frame.
        // The readings are formatted before the sliders take the values by
        // reference: a slider both reads and writes, and the borrow checker
        // is right to object to doing them at once.
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
        // These two do cost something: the cloud is downsampled, indexed
        // and re-normalled. The engine abandons superseded requests at the
        // next stage boundary, so dragging is safe.
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
        if changed {
            self.prepare = prepare;
            self.request_analysis();
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
            // The handle is taken and the borrow released before the camera
            // is touched: everything below wants `&mut self`.
            let Some((cloud, generation)) = self
                .scene
                .as_ref()
                .map(|scene| (Arc::clone(&scene.cloud), scene.generation))
            else {
                centred_note(ui, palette, rect, "drop a PLY file here, or ⌘O");
                return;
            };

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
                        cloud,
                        generation,
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
                        point_colour: palette.point.to_normalized_gamma_f32(),
                    },
                ));
        });
    }

    /// Mouse and wheel over the viewport.
    ///
    /// The camera is the only thing in the application that repaints
    /// continuously — and only while it is being moved. A still camera over
    /// a still cloud costs nothing.
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

/// The row that most deserves an explanation: the least determined one.
fn worst(
    conditioning: &rigidity_core::observability::Conditioning,
    criteria: &rigidity_core::observability::ObservabilityCriteria,
) -> usize {
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
