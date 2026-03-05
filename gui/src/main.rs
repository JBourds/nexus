#![allow(dead_code)]

mod app;
mod config_editor;
mod panels;
mod render;
mod sim;
mod state;

use app::NexusApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Nexus Simulator",
        options,
        Box::new(|_cc| Ok(Box::new(NexusApp::default()))),
    )
}
