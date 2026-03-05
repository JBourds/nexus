use egui::Ui;

use crate::state::AppMode;

#[derive(Clone, Copy, PartialEq)]
pub enum ToolbarAction {
    None,
    GoHome,
    OpenConfig,
    NewConfig,
    OpenTrace,
}

pub fn show_toolbar(ui: &mut Ui, mode: &AppMode) -> ToolbarAction {
    let mut action = ToolbarAction::None;

    ui.horizontal(|ui| {
        let home_label = match mode {
            AppMode::Home => "[ Home ]",
            _ => "  Home  ",
        };
        if ui.button(home_label).clicked() {
            action = ToolbarAction::GoHome;
        }

        ui.separator();

        if ui.button("Open Config").clicked() {
            action = ToolbarAction::OpenConfig;
        }
        if ui.button("New Config").clicked() {
            action = ToolbarAction::NewConfig;
        }
        if ui.button("Open Trace").clicked() {
            action = ToolbarAction::OpenTrace;
        }

        // Show mode indicator on the right
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mode_text = match mode {
                AppMode::Home => "Home",
                AppMode::ConfigEditor(_) => "Config Editor",
                AppMode::LiveSimulation(_) => "Live Simulation",
                AppMode::Replay(_) => "Replay",
            };
            ui.label(mode_text);
        });
    });

    action
}
