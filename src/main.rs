//! Syncora - a friendly file and folder synchronization app.
//!
//! Release builds are GUI-only (no console window) and ship as one executable.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod engine;
mod i18n;
mod model;
mod platform;
mod store;
mod sync;
mod theme;
mod tray;
mod util;

fn main() -> eframe::Result<()> {
    util::rotate_log_if_needed();
    platform::prepare_data_dir();
    // Always leave one line behind so "Open log file" is usable from the very
    // first run, even when nothing has gone wrong yet.
    util::log(&format!("Syncora {} starting", config::APP_VERSION));

    let config = config::Config::load();

    // One tray icon per user is enough; a second launch just tells the user
    // where the first one is.
    if platform::already_running() {
        let lang = i18n::Lang::detect(config.settings.language);
        platform::show_already_running_message(i18n::Key::AlreadyRunning.tr(lang));
        return Ok(());
    }

    let start_minimized =
        config.settings.start_minimized || std::env::args().any(|arg| arg == "--minimized");

    let icon = std::sync::Arc::new(platform::egui_icon(128));
    let viewport = egui::ViewportBuilder::default()
        .with_title("Syncora")
        .with_inner_size([1120.0, 760.0])
        .with_min_inner_size([980.0, 640.0])
        .with_icon(icon)
        .with_visible(!start_minimized);

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Syncora",
        options,
        Box::new(move |cc| Ok(Box::new(app::SyncoraApp::new(cc, start_minimized)))),
    )
}
