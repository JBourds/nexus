use std::path::PathBuf;

use clap::Parser;

mod app;
mod config_editor;
mod panels;
mod render;
mod sim;
mod state;

use app::NexusApp;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Optional config file to open GUI with
    pub config: Option<PathBuf>,
}

fn main() -> eframe::Result<()> {
    let args = Cli::parse();
    let app = args
        .config
        .map(|p| match NexusApp::new_with_config(p) {
            Ok(app) => app,
            Err(e) => {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        })
        .unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Nexus Simulator",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    )
}
