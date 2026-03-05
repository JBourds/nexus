use std::collections::HashSet;
use std::time::SystemTime;

use config::ast::{self, Charge, Cmd, NodeProtocol};
use egui::Ui;

use super::widgets::{
    CLOCK_UNIT_PAIRS, DATA_UNIT_PAIRS, DISTANCE_UNIT_PAIRS, POWER_UNIT_PAIRS, add_item_ui,
    channel_multi_select, cmd_editor, enum_combo, optional_nonzero_u64, remove_button,
};

pub fn show_nodes(ui: &mut Ui, sim: &mut ast::Simulation, buf: &mut String) {
    if let Some(name) = add_item_ui(ui, "+ Node:", buf) {
        sim.nodes.entry(name).or_insert_with(|| ast::Node {
                    position: ast::Position::default(),
                    charge: None,
                    protocols: Default::default(),
                    internal_names: Vec::new(),
                    resources: ast::Resources::default(),
                    sinks: HashSet::new(),
                    sources: HashSet::new(),
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
    let available_sinks: Vec<String> = {
        let mut s: Vec<_> = sim.sinks.keys().cloned().collect();
        s.sort();
        s
    };
    let available_sources: Vec<String> = {
        let mut s: Vec<_> = sim.sources.keys().cloned().collect();
        s.sort();
        s
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
                    show_node(
                        ui,
                        name,
                        node,
                        &available_channels,
                        &available_sinks,
                        &available_sources,
                    );
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
    available_sinks: &[String],
    available_sources: &[String],
) {
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
                POWER_UNIT_PAIRS,
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

    // --- Sinks ---
    if !available_sinks.is_empty() {
        ui.separator();
        ui.label("Sinks:");
        channel_multi_select(
            ui,
            &format!("node_sinks_{name}"),
            &mut node.sinks,
            available_sinks,
        );
    }

    // --- Sources ---
    if !available_sources.is_empty() {
        ui.separator();
        ui.label("Sources:");
        channel_multi_select(
            ui,
            &format!("node_sources_{name}"),
            &mut node.sources,
            available_sources,
        );
    }
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
