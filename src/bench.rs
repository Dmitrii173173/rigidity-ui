//! Frame-time measurement, so the M1 gate is a number rather than an
//! impression.
//!
//! `RIGIDITY_UI_BENCH=<seconds>` turns the camera by itself, records how
//! long each frame took, prints the distribution and closes the window.
//! `RIGIDITY_UI_SHOT=<path>` writes what the window looked like on the way
//! out. Nothing here runs unless one of those is set, and the application
//! has no other timing code — a permanent frame graph is a thing you look
//! at instead of the cloud.
//!
//! The screenshot is not a convenience. Frame times prove the renderer is
//! fast; they cannot tell a correct image from an empty one, and a viewport
//! that draws nothing is very fast indeed. Later milestones are gated on
//! what the image shows, so the ability to capture it is part of the
//! harness rather than an extra.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{Context, Event, UserData, ViewportCommand};

use crate::render::camera::Camera;

/// Frames discarded before measurement begins.
///
/// The first frames after a cloud arrives carry its upload, and the very
/// first carries pipeline compilation. Both are real costs and neither is
/// the steady-state frame time this gate is about.
const WARMUP: usize = 30;

/// The measurement in progress.
pub(crate) struct Bench {
    seconds: f32,
    started: Option<Instant>,
    frames: Vec<f32>,
    shot: Option<PathBuf>,
    finished: bool,
}

impl Bench {
    /// Reads `RIGIDITY_UI_BENCH`, in seconds.
    pub(crate) fn from_environment() -> Option<Self> {
        let seconds: f32 = std::env::var("RIGIDITY_UI_BENCH").ok()?.parse().ok()?;
        Some(Self {
            seconds,
            started: None,
            frames: Vec::new(),
            shot: std::env::var("RIGIDITY_UI_SHOT").ok().map(PathBuf::from),
            finished: false,
        })
    }

    /// Turns the camera one step and records the frame.
    pub(crate) fn step(&mut self, ctx: &Context, camera: &mut Camera, loaded: bool) {
        if self.finished {
            self.collect_shot(ctx);
            return;
        }
        if !loaded {
            return;
        }
        // Orbiting is what the gate asks about: a still camera measures the
        // compositor, not the renderer.
        camera.orbit([6.0, 0.0]);
        ctx.request_repaint();

        let elapsed = ctx.input(|input| input.unstable_dt);
        self.frames.push(elapsed);
        if self.frames.len() < WARMUP {
            return;
        }
        let started = *self.started.get_or_insert_with(Instant::now);
        if started.elapsed().as_secs_f32() < self.seconds {
            return;
        }

        self.finished = true;
        let mut measured: Vec<f32> = self.frames.drain(WARMUP..).collect();
        measured.sort_by(f32::total_cmp);
        if measured.is_empty() {
            println!("frames: none measured — the run was shorter than the warm-up");
            self.after_measuring(ctx);
            return;
        }
        let at = |fraction: f32| {
            measured
                .get(((measured.len() - 1) as f32 * fraction) as usize)
                .copied()
                .unwrap_or(f32::NAN)
                * 1000.0
        };
        println!(
            "frames {}  median {:.2} ms ({:.0} fps)  p95 {:.2} ms ({:.0} fps)  worst {:.2} ms",
            measured.len(),
            at(0.5),
            1000.0 / at(0.5),
            at(0.95),
            1000.0 / at(0.95),
            at(1.0),
        );
        self.after_measuring(ctx);
    }

    fn after_measuring(&self, ctx: &Context) {
        match &self.shot {
            // The window has to survive one more frame for the reply to
            // arrive, so the close is deferred to `collect_shot`.
            Some(_) => ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default())),
            None => ctx.send_viewport_cmd(ViewportCommand::Close),
        }
    }

    /// Writes the screenshot as soon as it comes back, then quits.
    ///
    /// The file is width, height and raw RGBA — a header of two integers and
    /// then the pixels. Encoding a PNG here would mean a compression
    /// dependency in a released binary for the sake of a diagnostic.
    fn collect_shot(&mut self, ctx: &Context) {
        let Some(path) = self.shot.take() else {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        };
        let image = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else {
            // Not yet: ask again next frame.
            self.shot = Some(path);
            ctx.request_repaint();
            return;
        };

        let mut bytes = Vec::with_capacity(8 + image.pixels.len() * 4);
        bytes.extend_from_slice(&(image.size[0] as u32).to_le_bytes());
        bytes.extend_from_slice(&(image.size[1] as u32).to_le_bytes());
        for pixel in &image.pixels {
            bytes.extend_from_slice(&pixel.to_array());
        }
        match std::fs::write(&path, bytes) {
            Ok(()) => println!(
                "screenshot {} × {} written: {}",
                image.size[0],
                image.size[1],
                path.display()
            ),
            Err(error) => eprintln!("screenshot failed: {error}"),
        }
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}
