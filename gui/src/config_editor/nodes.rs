use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use config::ast::{self, ChannelEnergy, Charge, Cmd, Energy, NodeProtocol, PowerFlow, PowerRate};
use egui::Ui;

use super::modules::show_profile_preview;
use super::widgets::{
    CLOCK_UNIT_PAIRS, DATA_UNIT_PAIRS, DISTANCE_UNIT_PAIRS, ENERGY_UNIT_PAIRS, add_item_ui,
    channel_multi_select, cmd_editor, enum_combo, optional_nonzero_u64, power_flow_editor,
    power_rate_editor, remove_button,
};
use crate::state::ModuleState;

pub fn show_nodes(
    ui: &mut Ui,
    sim: &mut ast::Simulation,
    buf: &mut String,
    modules: &mut ModuleState,
) {
    if let Some(name) = add_item_ui(ui, "+ Node:", buf) {
        sim.nodes.entry(name).or_insert_with(|| ast::Node {
            position: ast::Position::default(),
            charge: None,
            protocols: Default::default(),
            internal_names: Vec::new(),
            resources: ast::Resources::default(),
            power_states: HashMap::new(),
            power_sources: HashMap::new(),
            power_sinks: HashMap::new(),
            channel_energy: HashMap::new(),
            initial_state: None,
            restart_threshold: None,
            start: SystemTime::now(),
        });
    }

    let mut to_remove = Vec::new();
    let node_names: Vec<String> = {
        let mut names: Vec<_> = sim.nodes.keys().cloned().collect();
        names.sort();
        names
    };

    // Collect available channel names for protocol pub/sub selectors
    let available_channels: Vec<String> = {
        let mut ch: Vec<_> = sim.channels.keys().cloned().collect();
        ch.sort();
        ch
    };
    for name in &node_names {
        let id = ui.make_persistent_id(format!("node_{name}"));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.label(name);
                if remove_button(ui) {
                    to_remove.push(name.clone());
                }
            })
            .body(|ui| {
                if let Some(node) = sim.nodes.get_mut(name) {
                    show_node(ui, name, node, &available_channels, modules);
                }
            });
    }

    for name in to_remove {
        sim.nodes.remove(&name);
    }
}

fn show_node(
    ui: &mut Ui,
    name: &str,
    node: &mut ast::Node,
    available_channels: &[String],
    modules: &mut ModuleState,
) {
    // --- Profiles ---
    if !modules.available_profiles.is_empty() {
        ui.label("Profiles:");

        // Show current assignments with remove buttons
        let current = modules.node_profiles.get(name).cloned().unwrap_or_default();
        let mut profile_removed = None;
        for (i, pname) in current.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("[{pname}]"));
                if remove_button(ui) {
                    profile_removed = Some(i);
                }
            });

            // Expandable profile preview
            let key = pname.to_ascii_lowercase();
            if let Some(resolved) = modules.available_profiles.get(&key) {
                let pid = ui.make_persistent_id(format!("profile_preview_{name}_{pname}"));
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    pid,
                    false,
                )
                .show_header(ui, |ui| {
                    ui.weak(format!("{pname} ({})", resolved.source_module));
                })
                .body(|ui| {
                    show_profile_preview(ui, &resolved.profile);
                });
            }
        }
        if let Some(i) = profile_removed
            && let Some(profiles) = modules.node_profiles.get_mut(name) {
                profiles.remove(i);
                if profiles.is_empty() {
                    modules.node_profiles.remove(name);
                }
            }

        // Add profile dropdown
        let assigned: HashSet<String> = current.iter().map(|s| s.to_ascii_lowercase()).collect();
        let mut unassigned: Vec<_> = modules
            .available_profiles
            .keys()
            .filter(|k| !assigned.contains(*k))
            .cloned()
            .collect();
        unassigned.sort();

        if !unassigned.is_empty() {
            let add_id = egui::Id::new(format!("profile_add_{name}"));
            let mut selected: String = ui.data(|d| d.get_temp(add_id)).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label("+");
                egui::ComboBox::from_id_salt(format!("profile_sel_{name}"))
                    .selected_text(if selected.is_empty() {
                        "Add profile..."
                    } else {
                        &selected
                    })
                    .show_ui(ui, |ui| {
                        for p in &unassigned {
                            ui.selectable_value(&mut selected, p.clone(), p);
                        }
                    });
                if ui.button("Add").clicked() && !selected.is_empty() {
                    modules
                        .node_profiles
                        .entry(name.to_string())
                        .or_default()
                        .push(selected.clone());
                    selected.clear();
                }
            });
            ui.data_mut(|d| d.insert_temp(add_id, selected));
        }

        ui.separator();
    }

    // --- Position ---
    ui.label("Position:");
    ui.horizontal(|ui| {
        ui.label("x:");
        ui.add(egui::DragValue::new(&mut node.position.point.x).speed(0.1));
        ui.label("y:");
        ui.add(egui::DragValue::new(&mut node.position.point.y).speed(0.1));
        ui.label("z:");
        ui.add(egui::DragValue::new(&mut node.position.point.z).speed(0.1));
    });
    ui.horizontal(|ui| {
        ui.label("az:");
        ui.add(egui::DragValue::new(&mut node.position.orientation.az).speed(0.1));
        ui.label("el:");
        ui.add(egui::DragValue::new(&mut node.position.orientation.el).speed(0.1));
        ui.label("roll:");
        ui.add(egui::DragValue::new(&mut node.position.orientation.roll).speed(0.1));
    });
    ui.horizontal(|ui| {
        ui.label("Distance unit:");
        enum_combo(
            ui,
            &format!("node_dunit_{name}"),
            &mut node.position.unit,
            DISTANCE_UNIT_PAIRS,
        );
    });

    // --- Charge ---
    ui.separator();
    let mut has_charge = node.charge.is_some();
    if ui.checkbox(&mut has_charge, "Charge").changed() {
        node.charge = if has_charge {
            Some(Charge::default())
        } else {
            None
        };
    }
    if let Some(charge) = &mut node.charge {
        ui.horizontal(|ui| {
            ui.label("Max:");
            ui.add(egui::DragValue::new(&mut charge.max));
            ui.label("Qty:");
            ui.add(egui::DragValue::new(&mut charge.quantity));
            ui.label("Unit:");
            enum_combo(
                ui,
                &format!("node_punit_{name}"),
                &mut charge.unit,
                ENERGY_UNIT_PAIRS,
            );
        });
    }

    // --- Resources ---
    ui.separator();
    ui.label("Resources:");
    ui.indent(format!("node_res_{name}"), |ui| {
        optional_nonzero_u64(ui, "CPU cores:", &mut node.resources.cpu.cores);
        optional_nonzero_u64(ui, "CPU rate:", &mut node.resources.cpu.hertz);
        if node.resources.cpu.hertz.is_some() {
            ui.horizontal(|ui| {
                ui.label("  Clock unit:");
                enum_combo(
                    ui,
                    &format!("node_clku_{name}"),
                    &mut node.resources.cpu.unit,
                    CLOCK_UNIT_PAIRS,
                );
            });
        }
        optional_nonzero_u64(ui, "Memory:", &mut node.resources.mem.amount);
        if node.resources.mem.amount.is_some() {
            ui.horizontal(|ui| {
                ui.label("  Memory unit:");
                enum_combo(
                    ui,
                    &format!("node_memu_{name}"),
                    &mut node.resources.mem.unit,
                    DATA_UNIT_PAIRS,
                );
            });
        }
    });

    // --- Protocols ---
    ui.separator();
    show_protocols(ui, name, &mut node.protocols, available_channels);

    // --- Internal Channels ---
    ui.separator();
    show_internal_channels(ui, name, &mut node.internal_names);

    // --- Power States ---
    ui.separator();
    show_power_rate_map(ui, name, "power_state", &mut node.power_states);

    // --- Power Sources ---
    ui.separator();
    show_power_flow_map(ui, name, "power_source", &mut node.power_sources);

    // --- Power Sinks ---
    ui.separator();
    show_power_flow_map(ui, name, "power_sink", &mut node.power_sinks);

    // --- Channel Energy ---
    ui.separator();
    show_channel_energy(ui, name, &mut node.channel_energy, available_channels);

    // --- Initial State ---
    if !node.power_states.is_empty() {
        ui.separator();
        let state_names: Vec<String> = node.power_states.keys().cloned().collect();
        ui.horizontal(|ui| {
            ui.label("Initial State:");
            let current = node.initial_state.clone().unwrap_or_default();
            egui::ComboBox::from_id_salt(format!("node_init_state_{name}"))
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(node.initial_state.is_none(), "(none)")
                        .clicked()
                    {
                        node.initial_state = None;
                    }
                    for s in &state_names {
                        if ui
                            .selectable_label(node.initial_state.as_ref() == Some(s), s)
                            .clicked()
                        {
                            node.initial_state = Some(s.clone());
                        }
                    }
                });
        });
    }

    // --- Restart Threshold ---
    ui.separator();
    let mut has_threshold = node.restart_threshold.is_some();
    if ui
        .checkbox(&mut has_threshold, "Restart Threshold")
        .changed()
    {
        node.restart_threshold = if has_threshold { Some(0.1) } else { None };
    }
    if let Some(threshold) = &mut node.restart_threshold {
        ui.horizontal(|ui| {
            ui.label("Threshold:");
            ui.add(egui::Slider::new(threshold, 0.0..=1.0));
        });
    }
}

/// CRUD editor for a `HashMap<String, PowerRate>` on a node.
fn show_power_rate_map(
    ui: &mut Ui,
    node_name: &str,
    kind: &str,
    map: &mut HashMap<String, PowerRate>,
) {
    ui.label(format!("{}s:", kind.replace('_', " ")));
    let add_buf_id = egui::Id::new(format!("{kind}_add_buf_{node_name}"));
    let mut add_buf: String = ui.data(|d| d.get_temp(add_buf_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label("+");
        ui.text_edit_singleline(&mut add_buf);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add").clicked() || enter) && !add_buf.is_empty() {
            map.entry(add_buf.clone()).or_insert_with(|| PowerRate {
                rate: 0,
                unit: Default::default(),
                time: Default::default(),
            });
            add_buf.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(add_buf_id, add_buf));

    let mut to_remove = Vec::new();
    let names: Vec<String> = {
        let mut n: Vec<_> = map.keys().cloned().collect();
        n.sort();
        n
    };
    for n in &names {
        if let Some(rate) = map.get_mut(n) {
            ui.horizontal(|ui| {
                ui.label(format!("{n}:"));
                power_rate_editor(ui, &format!("{kind}_{node_name}_{n}"), rate);
                if remove_button(ui) {
                    to_remove.push(n.clone());
                }
            });
        }
    }
    for n in to_remove {
        map.remove(&n);
    }
}

/// CRUD editor for a `HashMap<String, PowerFlow>` on a node.
fn show_power_flow_map(
    ui: &mut Ui,
    node_name: &str,
    kind: &str,
    map: &mut HashMap<String, PowerFlow>,
) {
    ui.label(format!("{}s:", kind.replace('_', " ")));
    let add_buf_id = egui::Id::new(format!("{kind}_add_buf_{node_name}"));
    let mut add_buf: String = ui.data(|d| d.get_temp(add_buf_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label("+");
        ui.text_edit_singleline(&mut add_buf);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add").clicked() || enter) && !add_buf.is_empty() {
            map.entry(add_buf.clone()).or_insert_with(|| {
                PowerFlow::Constant(PowerRate {
                    rate: 0,
                    unit: Default::default(),
                    time: Default::default(),
                })
            });
            add_buf.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(add_buf_id, add_buf));

    let mut to_remove = Vec::new();
    let names: Vec<String> = {
        let mut n: Vec<_> = map.keys().cloned().collect();
        n.sort();
        n
    };
    for n in &names {
        if let Some(flow) = map.get_mut(n) {
            ui.horizontal(|ui| {
                ui.label(format!("{n}:"));
                if remove_button(ui) {
                    to_remove.push(n.clone());
                }
            });
            power_flow_editor(ui, &format!("{kind}_{node_name}_{n}"), flow);
        }
    }
    for n in to_remove {
        map.remove(&n);
    }
}

fn show_channel_energy(
    ui: &mut Ui,
    node_name: &str,
    channel_energy: &mut HashMap<String, ChannelEnergy>,
    available_channels: &[String],
) {
    ui.label("Channel Energy:");

    // Add channel energy entry — show channels that don't already have an entry
    let unset: Vec<_> = available_channels
        .iter()
        .filter(|ch| !channel_energy.contains_key(*ch))
        .cloned()
        .collect();
    if !unset.is_empty() {
        ui.horizontal(|ui| {
            ui.label("+");
            let add_id = format!("chenergy_add_{node_name}");
            let selected_id = egui::Id::new(&add_id);
            let mut selected: String = ui.data(|d| d.get_temp(selected_id)).unwrap_or_default();
            egui::ComboBox::from_id_salt(&add_id)
                .selected_text(if selected.is_empty() {
                    "Select channel"
                } else {
                    &selected
                })
                .show_ui(ui, |ui| {
                    for ch in &unset {
                        ui.selectable_value(&mut selected, ch.clone(), ch);
                    }
                });
            if ui.button("Add").clicked() && !selected.is_empty() {
                channel_energy.insert(selected.clone(), ChannelEnergy { tx: None, rx: None });
                selected.clear();
            }
            ui.data_mut(|d| d.insert_temp(selected_id, selected));
        });
    }

    let mut to_remove = Vec::new();
    let names: Vec<String> = {
        let mut n: Vec<_> = channel_energy.keys().cloned().collect();
        n.sort();
        n
    };
    for ch_name in &names {
        if let Some(ce) = channel_energy.get_mut(ch_name) {
            let id = ui.make_persistent_id(format!("chenergy_{node_name}_{ch_name}"));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    ui.label(ch_name);
                    if remove_button(ui) {
                        to_remove.push(ch_name.clone());
                    }
                })
                .body(|ui| {
                    energy_cost_editor(
                        ui,
                        &format!("chenergy_tx_{node_name}_{ch_name}"),
                        "TX cost",
                        &mut ce.tx,
                    );
                    energy_cost_editor(
                        ui,
                        &format!("chenergy_rx_{node_name}_{ch_name}"),
                        "RX cost",
                        &mut ce.rx,
                    );
                });
        }
    }
    for n in to_remove {
        channel_energy.remove(&n);
    }
}

fn energy_cost_editor(ui: &mut Ui, id: &str, label: &str, energy: &mut Option<Energy>) {
    ui.horizontal(|ui| {
        let mut has = energy.is_some();
        if ui.checkbox(&mut has, label).changed() {
            *energy = if has {
                Some(Energy {
                    quantity: 0,
                    unit: Default::default(),
                })
            } else {
                None
            };
        }
        if let Some(e) = energy {
            ui.add(egui::DragValue::new(&mut e.quantity));
            enum_combo(ui, id, &mut e.unit, ENERGY_UNIT_PAIRS);
        }
    });
}

fn show_protocols(
    ui: &mut Ui,
    node_name: &str,
    protocols: &mut std::collections::HashMap<String, NodeProtocol>,
    available_channels: &[String],
) {
    ui.label("Protocols:");

    // Add protocol — use a local buffer via egui's data store
    let add_buf_id = egui::Id::new(format!("proto_add_buf_{node_name}"));
    let mut add_buf: String = ui.data(|d| d.get_temp(add_buf_id)).unwrap_or_default();
    let mut added = false;
    ui.horizontal(|ui| {
        ui.label("+ Protocol:");
        ui.text_edit_singleline(&mut add_buf);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add").clicked() || enter) && !add_buf.is_empty() {
            if !protocols.contains_key(&add_buf) {
                protocols.insert(
                    add_buf.clone(),
                    NodeProtocol {
                        root: std::path::PathBuf::from("."),
                        build: Cmd {
                            cmd: String::new(),
                            args: Vec::new(),
                        },
                        runner: Cmd {
                            cmd: String::new(),
                            args: Vec::new(),
                        },
                        publishers: HashSet::new(),
                        subscribers: HashSet::new(),
                    },
                );
            }
            add_buf.clear();
            added = true;
        }
    });
    ui.data_mut(|d| d.insert_temp(add_buf_id, add_buf));
    let _ = added;

    let mut proto_to_remove = Vec::new();
    let proto_names: Vec<String> = {
        let mut names: Vec<_> = protocols.keys().cloned().collect();
        names.sort();
        names
    };

    for pname in &proto_names {
        let pid = ui.make_persistent_id(format!("proto_{node_name}_{pname}"));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), pid, false)
            .show_header(ui, |ui| {
                ui.label(pname);
                if remove_button(ui) {
                    proto_to_remove.push(pname.clone());
                }
            })
            .body(|ui| {
                if let Some(proto) = protocols.get_mut(pname) {
                    show_protocol(ui, node_name, pname, proto, available_channels);
                }
            });
    }

    for pname in proto_to_remove {
        protocols.remove(&pname);
    }
}

fn show_protocol(
    ui: &mut Ui,
    node_name: &str,
    proto_name: &str,
    proto: &mut NodeProtocol,
    available_channels: &[String],
) {
    ui.horizontal(|ui| {
        ui.label("Root:");
        let mut root_str = proto.root.to_string_lossy().to_string();
        if ui.text_edit_singleline(&mut root_str).changed() {
            proto.root = root_str.into();
        }
    });

    ui.label("Build:");
    cmd_editor(
        ui,
        &format!("proto_build_{node_name}_{proto_name}"),
        &mut proto.build,
    );

    ui.label("Run:");
    cmd_editor(
        ui,
        &format!("proto_run_{node_name}_{proto_name}"),
        &mut proto.runner,
    );

    if !available_channels.is_empty() {
        ui.label("Publishers:");
        channel_multi_select(
            ui,
            &format!("proto_pub_{node_name}_{proto_name}"),
            &mut proto.publishers,
            available_channels,
        );
        ui.label("Subscribers:");
        channel_multi_select(
            ui,
            &format!("proto_sub_{node_name}_{proto_name}"),
            &mut proto.subscribers,
            available_channels,
        );
    }
}

fn show_internal_channels(ui: &mut Ui, node_name: &str, internal: &mut Vec<String>) {
    ui.label("Internal Channels:");
    // Add
    let add_buf_id = egui::Id::new(format!("intch_add_buf_{node_name}"));
    let mut add_buf: String = ui.data(|d| d.get_temp(add_buf_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label("+");
        ui.text_edit_singleline(&mut add_buf);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add").clicked() || enter) && !add_buf.is_empty() {
            if !internal.contains(&add_buf) {
                internal.push(add_buf.clone());
            }
            add_buf.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(add_buf_id, add_buf));

    // List with remove buttons
    let mut to_remove = Vec::new();
    for (i, ch) in internal.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(ch);
            if remove_button(ui) {
                to_remove.push(i);
            }
        });
    }
    for i in to_remove.into_iter().rev() {
        internal.remove(i);
    }
}
