use config::ast;
use egui::Ui;

use crate::state::NodeState;

/// Show the inspector panel with details about the selected node.
pub fn show_inspector(
    ui: &mut Ui,
    sim: &ast::Simulation,
    node_states: &[NodeState],
    selected_node: &Option<String>,
) {
    ui.heading("Inspector");
    ui.separator();

    let Some(name) = selected_node else {
        ui.label("No node selected");
        return;
    };

    let Some(ast_node) = sim.nodes.get(name) else {
        ui.label(format!("Node '{name}' not found in config"));
        return;
    };

    let runtime = node_states.iter().find(|n| n.name == *name);

    ui.strong(name);
    ui.separator();

    // Position
    ui.label("Position:");
    if let Some(rt) = runtime {
        ui.label(format!("  x: {:.2}", rt.x));
        ui.label(format!("  y: {:.2}", rt.y));
        ui.label(format!("  z: {:.2}", rt.z));
    } else {
        ui.label(format!("  x: {:.2}", ast_node.position.point.x));
        ui.label(format!("  y: {:.2}", ast_node.position.point.y));
        ui.label(format!("  z: {:.2}", ast_node.position.point.z));
    }

    // Charge
    if let Some(charge) = &ast_node.charge {
        ui.separator();
        ui.label("Charge:");
        ui.label(format!("  max: {}", charge.max));
        ui.label(format!("  current: {}", charge.quantity));
        if let Some(rt) = runtime
            && let Some(ratio) = rt.charge_ratio {
                ui.label(format!("  ratio: {:.1}%", ratio * 100.0));
            }
    }

    // Protocols
    ui.separator();
    ui.label("Protocols:");
    for (proto_name, proto) in &ast_node.protocols {
        ui.collapsing(proto_name, |ui| {
            ui.label(format!("Root: {}", proto.root.display()));
            if !proto.publishers.is_empty() {
                ui.label(format!("Publishers: {:?}", proto.publishers));
            }
            if !proto.subscribers.is_empty() {
                ui.label(format!("Subscribers: {:?}", proto.subscribers));
            }
        });
    }
}
