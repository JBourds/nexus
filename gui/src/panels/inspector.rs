use std::collections::HashSet;

use config::ast;
use egui::Ui;

use crate::state::NodeState;

/// Show the inspector panel with all nodes as collapsible sections.
///
/// Uses manual expand/collapse logic with `expanded_nodes` as the sole
/// source of truth, avoiding egui's internal CollapsingState which can
/// conflict with cross-panel frame ordering.
pub fn show_inspector(
    ui: &mut Ui,
    sim: &ast::Simulation,
    node_states: &[NodeState],
    selected_node: &Option<String>,
    expanded_nodes: &mut HashSet<String>,
) {
    egui::Frame::NONE.inner_margin(6.0).show(ui, |ui| {
        ui.heading("Inspector");
        ui.separator();

        if node_states.is_empty() {
            ui.label("No nodes");
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut sorted_names: Vec<_> = node_states.iter().map(|n| &n.name).collect();
            sorted_names.sort();

            for name in sorted_names {
                let is_expanded = expanded_nodes.contains(name);
                let is_selected = selected_node.as_ref().is_some_and(|s| s == name);

                // Node header row with toggle arrow + name
                let header_resp = ui.horizontal(|ui| {
                    let arrow = if is_expanded { "\u{25bc}" } else { "\u{25b6}" };
                    let toggle = ui.small_button(arrow);
                    let label = if is_selected {
                        ui.strong(name)
                    } else {
                        ui.label(name)
                    };
                    toggle.clicked() || label.clicked()
                });

                if header_resp.inner {
                    if is_expanded {
                        expanded_nodes.remove(name);
                    } else {
                        expanded_nodes.insert(name.clone());
                    }
                }

                // Show body if expanded
                if is_expanded {
                    ui.indent(name, |ui| {
                        show_node_details(ui, sim, node_states, name);
                    });
                }

                ui.add_space(2.0);
            }
        });
    }); // Frame
}

fn show_node_details(
    ui: &mut Ui,
    sim: &ast::Simulation,
    node_states: &[NodeState],
    name: &str,
) {
    let runtime = node_states.iter().find(|n| n.name == name);

    // Position
    if let Some(rt) = runtime {
        ui.label(format!("x: {:.2}  y: {:.2}  z: {:.2}", rt.x, rt.y, rt.z));
    }

    // Charge
    if let Some(rt) = runtime {
        if let Some(ratio) = rt.charge_ratio {
            ui.label(format!("Charge: {:.1}%", ratio * 100.0));
        }
    }

    // Protocols (from AST if available)
    if let Some(ast_node) = sim.nodes.get(name) {
        if !ast_node.protocols.is_empty() {
            ui.separator();
            ui.label("Protocols:");
            for (proto_name, proto) in &ast_node.protocols {
                ui.collapsing(proto_name, |ui| {
                    ui.label(format!("Root: {}", proto.root.display()));
                    if !proto.publishers.is_empty() {
                        ui.label(format!("Pub: {:?}", proto.publishers));
                    }
                    if !proto.subscribers.is_empty() {
                        ui.label(format!("Sub: {:?}", proto.subscribers));
                    }
                });
            }
        }
    }
}
