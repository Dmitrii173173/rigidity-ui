//! Window bootstrap. Everything that draws lives in [`app`].

mod app;
mod bench;
mod commands;
mod engine;
mod render;
mod spectrum;
mod theme;
mod timeline;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([880.0, 560.0]);
    // The content runs to the top of the window and the traffic lights
    // float over it, which is what a macOS application built in the last
    // decade looks like. The inspector leaves room for them; see
    // `app::CHROME_INSET`.
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);

    let options = eframe::NativeOptions {
        viewport,
        // The defaults are what we want: no multisampling (the viewport
        // does its own antialiasing, and points do not benefit from MSAA)
        // and no depth buffer on egui's own pass, which our callback
        // therefore must not expect either.
        ..Default::default()
    };
    // Every file named on the command line, in order. The first becomes
    // the target and the second the source; the rest are context until
    // someone gives them a role.
    let open: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    eframe::run_native(
        "rigidity",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, open)))),
    )
}
