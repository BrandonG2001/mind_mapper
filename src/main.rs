// In release builds on Windows, don't open a console window next to the GUI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use anyhow::anyhow;
use eframe::egui;
use mind_mapper::app::MindMapApp;

/// Start the GUI. An optional first command-line argument is a map file to open.
fn main() -> anyhow::Result<()> {
    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mind Mapper")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Mind Mapper",
        options,
        // `move` hands ownership of `initial_path` to the closure, which eframe calls once.
        Box::new(move |cc| Ok(Box::new(MindMapApp::new(cc, initial_path.as_deref())))),
    )
    .map_err(|err| anyhow!("failed to start the window: {err}"))
}
