//! The shell: what is on screen, and where.
//!
//! M0 establishes the layout and nothing else. The inspector says what it
//! does not have yet rather than showing controls that do nothing — a
//! disabled button that will never be pressed is a worse lie than an
//! empty panel.

use std::time::Instant;

use eframe::egui::{self, Align, Color32, Layout, Rect, RichText, Vec2};

use crate::render::{self, ViewportCallback};
use crate::theme::{self, Mode, Palette, space};

/// The application.
pub(crate) struct App {
    mode: Mode,
    /// Where the wall clock started, for the M0 rotation.
    started: Instant,
    /// Whether the wgpu pipeline was built. False means no viewport.
    gpu: bool,
}

impl App {
    /// Builds the app and everything the GPU needs.
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mode = Mode::Dark;
        theme::apply(&cc.egui_ctx, mode);
        Self {
            mode,
            started: Instant::now(),
            gpu: render::install(cc.wgpu_render_state.as_ref()),
        }
    }

    fn palette(&self) -> Palette {
        Palette::of(self.mode)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let palette = self.palette();

        // Order decides nesting: the status strip is added first so it
        // spans the full width, and the inspector sits above it.
        status_strip(ui, &palette, frame, self.gpu);
        inspector(ui, &palette, &mut self.mode);
        viewport(ui, &palette, self.started, self.gpu);
    }
}

/// The bottom strip: what the application is doing, and on what hardware.
fn status_strip(ui: &mut egui::Ui, palette: &Palette, frame: &eframe::Frame, gpu: bool) {
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
                let (dot, text) = if gpu {
                    (palette.high, "M0 · scaffold")
                } else {
                    (palette.low, "M0 · no gpu")
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

/// The left panel: sources, parameters, and — later — the spectrum.
fn inspector(ui: &mut egui::Ui, palette: &Palette, mode: &mut Mode) {
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
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(mode.label()).size(11.0).color(palette.muted),
                            )
                            .fill(Color32::TRANSPARENT),
                        )
                        .on_hover_text("switch theme")
                        .clicked()
                    {
                        *mode = mode.flipped();
                        theme::apply(ui.ctx(), *mode);
                    }
                });
            });

            ui.add_space(space::GROUP);
            heading(ui, palette, "source");
            ui.label(RichText::new("—").color(palette.faint));
            ui.add_space(space::GROUP);
            heading(ui, palette, "target");
            ui.label(RichText::new("—").color(palette.faint));

            ui.add_space(space::GROUP);
            heading(ui, palette, "spectrum");
            ui.label(
                RichText::new(
                    "Register two clouds to see which degrees of freedom the geometry determined.",
                )
                .color(palette.faint)
                .size(11.0),
            );

            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.label(
                    RichText::new("M0 · loading arrives with M1")
                        .color(palette.faint)
                        .size(11.0),
                );
            });
        });
}

/// The viewport: everything that is not a panel.
fn viewport(ui: &mut egui::Ui, palette: &Palette, started: Instant, gpu: bool) {
    egui::CentralPanel::no_frame().show(ui, |ui| {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, palette.background);

        if !gpu {
            centred_note(ui, palette, rect, "no gpu adapter — nothing to draw on");
            return;
        }
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }

        ui.painter()
            .add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                ViewportCallback {
                    // A quarter turn every three seconds: enough to prove
                    // the frame loop is alive, slow enough not to nag.
                    angle: started.elapsed().as_secs_f32() * 0.5,
                    aspect: rect.aspect_ratio(),
                },
            ));

        // M0 animates, so it repaints continuously. From M1 the viewport
        // repaints on demand — a still camera over a still cloud should
        // cost nothing.
        ui.ctx().request_repaint();
    });
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

/// The status dot.
fn bullet(ui: &mut egui::Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(6.0), egui::Sense::hover());
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
