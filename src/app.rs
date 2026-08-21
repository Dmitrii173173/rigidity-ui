//! The shell: what is on screen, where, and what the mouse does to it.
//!
//! The panels say what they have rather than showing controls that do
//! nothing. A disabled button for a feature that has not been written is a
//! worse lie than an empty panel.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, Key, Layout, Modifiers, PointerButton, Rect, RichText, Sense, Vec2,
};
use rigidity_core::PointCloud;
use rigidity_core::nalgebra as na;
use rigidity_pipeline::PipelineError;

use crate::bench::Bench;
use crate::engine::{Engine, Event, Job};
use crate::render::camera::Camera;
use crate::render::{self, ViewportCallback};
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

/// What the status strip is saying.
enum Status {
    Nothing,
    Reading(PathBuf),
    Failed(PipelineError),
}

/// The application.
pub(crate) struct App {
    mode: Mode,
    bench: Option<Bench>,
    engine: Engine,
    gpu: bool,
    scene: Option<Scene>,
    status: Status,
    camera: Camera,
    point_size: f32,
    edl_strength: f32,
    generation: u64,
}

impl App {
    /// Builds the app, the GPU pipelines and the worker thread.
    ///
    /// A path on the command line is loaded straight away — which is what
    /// makes the window openable from a shell, from a file manager, and
    /// from the measurement mode.
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let mode = Mode::Dark;
        theme::apply(&cc.egui_ctx, mode);
        let engine = Engine::spawn(cc.egui_ctx.clone());
        if let Some(path) = open {
            engine.send(Job::Load(path));
        }
        Self {
            mode,
            bench: Bench::from_environment(),
            engine,
            gpu: render::install(cc.wgpu_render_state.as_ref()),
            scene: None,
            status: Status::Nothing,
            camera: Camera::default(),
            // Three points across, not one: at one physical pixel a
            // splat covers less than the average spacing of a real scan
            // and the surface comes out as noise. Three closes the gaps
            // without turning the cloud into paste.
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
                Event::Started(path) => self.status = Status::Reading(path),
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
                    self.status = Status::Nothing;
                }
                Event::Failed(error) => self.status = Status::Failed(error),
            }
        }
    }

    /// Asks for a file and queues it.
    fn open(&self) {
        // The dialog blocks, and on every platform this application runs on
        // it is modal anyway: there is nothing behind it to interact with.
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("point cloud", &["ply"])
            .pick_file()
        {
            self.engine.send(Job::Load(path));
        }
    }

    fn shortcuts(&mut self, ui: &egui::Ui) {
        let (open, fit, dropped) = ui.input(|input| {
            (
                input.modifiers.matches_logically(Modifiers::COMMAND) && input.key_pressed(Key::O),
                input.key_pressed(Key::F),
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
        // Dropping is how a file actually gets opened; the dialog is the
        // fallback for people who do not know that yet.
        for path in dropped {
            self.engine.send(Job::Load(path));
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
                    let (dot, text) = match (&self.status, &self.scene) {
                        (Status::Failed(error), _) => (palette.low, error.to_string()),
                        (Status::Reading(path), _) => {
                            (palette.medium, format!("reading {}…", file_name(path)))
                        }
                        (Status::Nothing, Some(scene)) => (
                            palette.high,
                            format!(
                                "{} points · {:.1} m across · read in {:.2} s",
                                thousands(scene.cloud.len()),
                                scene.extent(),
                                scene.seconds
                            ),
                        ),
                        (Status::Nothing, None) => {
                            (palette.faint, "drop a PLY file here".to_owned())
                        }
                    };
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

    /// The left panel: what is loaded, and how it is drawn.
    fn inspector(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        egui::Panel::left("inspector")
            .default_size(264.0)
            .size_range(220.0..=380.0)
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

                ui.add_space(space::GROUP);
                heading(ui, palette, "display");
                slider(
                    ui,
                    palette,
                    "point size",
                    &mut self.point_size,
                    1.0..=8.0,
                    "px",
                );
                slider(
                    ui,
                    palette,
                    "shading",
                    &mut self.edl_strength,
                    0.0..=800.0,
                    "",
                );

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.label(
                        RichText::new("M1 · registration arrives with M3")
                            .color(palette.faint)
                            .size(11.0),
                    );
                });
            });
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
            // The handle is taken and the borrow released before the
            // camera is touched: everything below wants `&mut self`.
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
                        point_colour: normalised(palette.point),
                    },
                ));
        });
    }

    /// Mouse and wheel over the viewport.
    ///
    /// The camera is the only thing in the application that repaints
    /// continuously — and only while it is being moved. A still camera over
    /// a still cloud costs nothing, which is the whole reason M0's
    /// unconditional `request_repaint` had to go.
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

fn normalised(colour: Color32) -> [f32; 4] {
    colour.to_normalized_gamma_f32()
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
fn slider(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    unit: &str,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(palette.muted).size(11.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{value:.0}{unit}"))
                    .color(palette.faint)
                    .size(11.0)
                    .monospace(),
            );
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    ui.add(egui::Slider::new(value, range).show_value(false));
    ui.add_space(space::ROW);
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
