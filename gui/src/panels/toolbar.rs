use egui::Ui;

use crate::state::{AppMode, PanelVisibility, ViewMode};

#[derive(Clone, Copy, PartialEq)]
pub enum ToolbarAction {
    None,
    GoHome,
    OpenConfig,
    NewConfig,
    OpenTrace,
    RunSimulation,
    StopSimulation,
    RerunSimulation,
    ToggleInspector,
    ToggleMessages,
    ToggleViewMode,
}

pub fn show_toolbar(
    ui: &mut Ui,
    mode: &AppMode,
    sim_finished: bool,
    panels: Option<&PanelVisibility>,
    view_mode: Option<ViewMode>,
) -> ToolbarAction {
    let mut action = ToolbarAction::None;

    egui::Frame::NONE.inner_margin(6.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            let home_label = match mode {
                AppMode::Home => "[ Home ]",
                _ => "  Home  ",
            };
            if ui
                .button(home_label)
                .on_hover_text("Go to home screen")
                .clicked()
            {
                action = ToolbarAction::GoHome;
            }

            ui.separator();

            if ui
                .button("Open Config")
                .on_hover_text("Open config file")
                .clicked()
            {
                action = ToolbarAction::OpenConfig;
            }
            if ui
                .button("New Config")
                .on_hover_text("Create new config")
                .clicked()
            {
                action = ToolbarAction::NewConfig;
            }
            if ui
                .button("Open Trace")
                .on_hover_text("Open trace file")
                .clicked()
            {
                action = ToolbarAction::OpenTrace;
            }

            if matches!(mode, AppMode::ConfigEditor(_)) {
                ui.separator();
                if ui
                    .button("\u{25b6} Run")
                    .on_hover_text("Run simulation")
                    .clicked()
                {
                    action = ToolbarAction::RunSimulation;
                }
            }

            if matches!(mode, AppMode::LiveSimulation(_)) {
                ui.separator();
                if sim_finished {
                    if ui
                        .button("\u{25b6} Rerun")
                        .on_hover_text("Rerun simulation")
                        .clicked()
                    {
                        action = ToolbarAction::RerunSimulation;
                    }
                } else if ui
                    .button("\u{23f9} Stop")
                    .on_hover_text("Stop simulation")
                    .clicked()
                {
                    action = ToolbarAction::StopSimulation;
                }
            }

            // Panel toggles (for modes that have panels)
            if let Some(panels) = panels {
                ui.separator();
                let insp_label = if panels.inspector {
                    "Inspector \u{2713}"
                } else {
                    "Inspector"
                };
                if ui
                    .button(insp_label)
                    .on_hover_text("Toggle inspector panel")
                    .clicked()
                {
                    action = ToolbarAction::ToggleInspector;
                }
                let msg_label = if panels.messages {
                    "Messages \u{2713}"
                } else {
                    "Messages"
                };
                if ui
                    .button(msg_label)
                    .on_hover_text("Toggle messages panel")
                    .clicked()
                {
                    action = ToolbarAction::ToggleMessages;
                }
            }

            // View mode toggle (Grid vs Sequence)
            if let Some(vm) = view_mode {
                ui.separator();
                let view_label = match vm {
                    ViewMode::Grid => "Grid",
                    ViewMode::Sequence => "Sequence",
                };
                if ui
                    .button(view_label)
                    .on_hover_text("Toggle between Grid and Sequence diagram view")
                    .clicked()
                {
                    action = ToolbarAction::ToggleViewMode;
                }
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
    }); // Frame

    action
}
