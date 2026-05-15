#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod autostart;
mod daemon;
mod log_stream;
mod tray;
mod views;

use std::sync::Arc;

use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // The tray must live for the duration of the program. We wrap it in an
    // Arc and clone it into the eframe app. `try_new` returns None on
    // unsupported environments (typically headless Linux) — the app stays
    // functional without it.
    let tray = tray::TrayHandle::try_new().map(Arc::new);
    if tray.is_none() {
        tracing::warn!("tray icon unavailable on this platform; continuing without it");
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 500.0])
            .with_title("Speedy"),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "Speedy GUI",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::SpeedyApp::new(cc, tray.clone())))),
    )
    .map_err(|e| anyhow::anyhow!("eframe init failed: {e}"))?;
    Ok(())
}
