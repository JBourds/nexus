pub mod channels;
pub mod links;
pub mod nodes;
pub mod params;
pub mod widgets;

use config::ast;
use egui::Ui;

use crate::state::ConfigEditorState;
use widgets::{add_item_ui, power_rate_editor, remove_button};

/// Show the full configuration editor UI.
pub fn show_config_editor(ui: &mut Ui, state: &mut ConfigEditorState) {
    egui::SidePanel::left("config_sections")
        .default_width(300.0)
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Configuration");
                ui.separator();

                ui.collapsing("Parameters", |ui| {
                    params::show_params(ui, &mut state.sim.params);
                });

                ui.collapsing("Nodes", |ui| {
                    nodes::show_nodes(ui, &mut state.sim, &mut state.add_item_buf);
                });

                ui.collapsing("Channels", |ui| {
                    channels::show_channels(ui, &mut state.sim, &mut state.add_item_buf);
                });

                ui.collapsing("Sinks", |ui| {
                    show_power_rate_map(ui, "sink", &mut state.sim.sinks, &mut state.add_item_buf);
                });

                ui.collapsing("Sources", |ui| {
                    show_power_rate_map(
                        ui,
                        "source",
                        &mut state.sim.sources,
                        &mut state.add_item_buf,
                    );
                });

                ui.separator();

                // Save / validate buttons
                ui.horizontal(|ui| {
                    if ui.button("Validate").clicked() {
                        state.validation_error = validate_config(&state.sim);
                    }
                    if ui.button("Save").clicked()
                        && let Some(ref path) = state.file_path {
                            if let Err(e) = config::serialize_config(&state.sim, path) {
                                state.validation_error = Some(format!("Save error: {e}"));
                            } else {
                                state.dirty = false;
                                state.validation_error = None;
                            }
                        }
                    if ui.button("Save As...").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("TOML", &["toml"])
                            .save_file()
                        {
                            if let Err(e) = config::serialize_config(&state.sim, &path) {
                                state.validation_error = Some(format!("Save error: {e}"));
                            } else {
                                state.file_path = Some(path);
                                state.dirty = false;
                                state.validation_error = None;
                            }
                        }
                });

                if let Some(ref err) = state.validation_error {
                    ui.colored_label(egui::Color32::RED, err);
                }
            });
        });
}

/// Reusable CRUD for a `HashMap<String, PowerRate>` (sinks or sources).
fn show_power_rate_map(
    ui: &mut Ui,
    kind: &str,
    map: &mut std::collections::HashMap<String, ast::PowerRate>,
    buf: &mut String,
) {
    if let Some(name) = add_item_ui(ui, &format!("+ {kind}:"), buf) {
        map.entry(name).or_insert_with(|| ast::PowerRate {
                    rate: 0,
                    unit: ast::PowerUnit::default(),
                    time: ast::TimeUnit::default(),
                });
    }

    let mut to_remove = Vec::new();
    let names: Vec<String> = {
        let mut n: Vec<_> = map.keys().cloned().collect();
        n.sort();
        n
    };

    for name in &names {
        if let Some(rate) = map.get_mut(name) {
            ui.horizontal(|ui| {
                ui.label(format!("{name}:"));
                power_rate_editor(ui, &format!("{kind}_{name}"), rate);
                if remove_button(ui) {
                    to_remove.push(name.clone());
                }
            });
        }
    }

    for name in to_remove {
        map.remove(&name);
    }
}

fn validate_config(sim: &ast::Simulation) -> Option<String> {
    // Round-trip through snapshot serialization to validate the AST
    let tmp = std::env::temp_dir().join("nexus_validate.toml");
    if let Err(e) = config::serialize_config(sim, &tmp) {
        return Some(format!("Serialization error: {e:#}"));
    }
    match config::deserialize_config(&tmp) {
        Ok(_) => None,
        Err(e) => Some(format!("{e:#}")),
    }
}
