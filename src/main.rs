//! Window bootstrap. Everything that draws lives in [`app`].

mod app;
mod bench;
mod engine;
mod render;
mod theme;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([880.0, 560.0]),
        // The defaults are what we want: no multisampling (the viewport
        // does its own antialiasing, and points do not benefit from MSAA)
        // and no depth buffer on egui's own pass, which our callback
        // therefore must not expect either.
        ..Default::default()
    };
    // One optional argument: a file to open. Anything more belongs to the
    // CLI, which is a different program with a different job.
    let open = std::env::args().nth(1).map(PathBuf::from);
    eframe::run_native(
        "rigidity",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, open)))),
    )
}
