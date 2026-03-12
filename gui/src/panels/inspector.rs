use std::collections::HashSet;

use config::ast;
use egui::Ui;

use crate::state::{MessageEntry, MessageKind, NodeState};

/// Action from the inspector panel.
pub enum InspectorAction {
    None,
    /// Jump the event cursor to a specific record index.
    JumpToEvent(usize),
}

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
    messages: &[MessageEntry],
    current_event: Option<usize>,
) -> InspectorAction {
    let mut action = InspectorAction::None;

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
                        ui.separator();
                        let node_action =
                            show_node_events(ui, name, messages, current_event);
                        if let InspectorAction::JumpToEvent(_) = &node_action {
                            action = node_action;
                        }
                    });
                }

                ui.add_space(2.0);
            }
        });
    }); // Frame

    action
}

fn show_node_details(ui: &mut Ui, sim: &ast::Simulation, node_states: &[NodeState], name: &str) {
    let runtime = node_states.iter().find(|n| n.name == name);

    // Position
    if let Some(rt) = runtime {
        ui.label(format!("x: {:.2}  y: {:.2}  z: {:.2}", rt.x, rt.y, rt.z));

        // Velocity (computed from delta between last two position updates)
        if rt.last_move_ts > 0 {
            let dx = rt.x - rt.prev_x;
            let dy = rt.y - rt.prev_y;
            let dz = rt.z - rt.prev_z;
            let speed = (dx * dx + dy * dy + dz * dz).sqrt();
            if speed > 1e-9 {
                ui.label(format!(
                    "v: ({:.2}, {:.2}, {:.2})  |v|={:.2}",
                    dx, dy, dz, speed
                ));
            }
        }
    }

    // Motion pattern
    if let Some(rt) = runtime {
        ui.horizontal(|ui| {
            ui.label("Motion:");
            if rt.motion_spec == "none" {
                ui.label("Static");
            } else {
                ui.colored_label(egui::Color32::from_rgb(180, 180, 255), &rt.motion_spec);
            }
        });
    }

    // Charge
    if let Some(rt) = runtime {
        if rt.is_dead {
            ui.colored_label(egui::Color32::RED, "DEAD (battery depleted)");
        }
        if let Some(ratio) = rt.charge_ratio {
            ui.label(format!("Charge: {:.1}%", ratio * 100.0));
        }
    }

    // Protocols (from AST if available)
    if let Some(ast_node) = sim.nodes.get(name)
        && !ast_node.protocols.is_empty()
    {
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

/// Show the per-node event log: filtered list of this node's TX/RX/Drop events,
/// with prev/next buttons to jump the event cursor.
fn show_node_events(
    ui: &mut Ui,
    node_name: &str,
    messages: &[MessageEntry],
    current_event: Option<usize>,
) -> InspectorAction {
    let mut action = InspectorAction::None;

    // Filter messages for this node
    let node_msgs: Vec<(usize, &MessageEntry)> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.src_node == node_name
                || m.dst_node.as_ref().is_some_and(|d| d == node_name)
        })
        .collect();

    let events_id = ui.id().with(("node_events", node_name));
    egui::CollapsingHeader::new(format!("Events ({})", node_msgs.len()))
        .id_salt(events_id)
        .default_open(true)
        .show(ui, |ui| {
            if node_msgs.is_empty() {
                ui.label("No events for this node");
                return;
            }

            // Prev/Next buttons for jumping within this node's events
            ui.horizontal(|ui| {
                if ui.small_button("< Prev").on_hover_text("Previous event for this node").clicked() {
                    // Find the previous event before current_event
                    if let Some(cur) = current_event {
                        for (_, msg) in node_msgs.iter().rev() {
                            if let Some(ri) = msg.record_index {
                                if ri < cur {
                                    action = InspectorAction::JumpToEvent(ri);
                                    break;
                                }
                            }
                        }
                    } else if let Some((_, msg)) = node_msgs.last() {
                        if let Some(ri) = msg.record_index {
                            action = InspectorAction::JumpToEvent(ri);
                        }
                    }
                }
                if ui.small_button("Next >").on_hover_text("Next event for this node").clicked() {
                    if let Some(cur) = current_event {
                        for (_, msg) in &node_msgs {
                            if let Some(ri) = msg.record_index {
                                if ri > cur {
                                    action = InspectorAction::JumpToEvent(ri);
                                    break;
                                }
                            }
                        }
                    } else if let Some((_, msg)) = node_msgs.first() {
                        if let Some(ri) = msg.record_index {
                            action = InspectorAction::JumpToEvent(ri);
                        }
                    }
                }
            });

            // Scrollable event list
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .id_salt(ui.id().with(("node_events_scroll", node_name)))
                .show(ui, |ui| {
                    for (_, msg) in &node_msgs {
                        let is_current = current_event.is_some()
                            && msg.record_index.is_some()
                            && msg.record_index == current_event;

                        let (icon, color) = match &msg.kind {
                            MessageKind::Sent => (
                                "TX",
                                egui::Color32::from_rgb(100, 200, 100),
                            ),
                            MessageKind::Received => (
                                "RX",
                                egui::Color32::from_rgb(100, 150, 255),
                            ),
                            MessageKind::Dropped(_) => (
                                "XX",
                                egui::Color32::from_rgb(255, 100, 100),
                            ),
                        };

                        let frame = if is_current {
                            egui::Frame::NONE
                                .fill(egui::Color32::from_rgba_premultiplied(255, 255, 100, 30))
                                .inner_margin(1.0)
                                .corner_radius(2.0)
                        } else {
                            egui::Frame::NONE
                        };

                        frame.show(ui, |ui| {
                            let resp = ui.horizontal(|ui| {
                                ui.colored_label(color, format!("[{}]", icon));
                                ui.label(
                                    egui::RichText::new(format!("t={}", msg.timestep)).small(),
                                );
                                ui.label(
                                    egui::RichText::new(&msg.channel).small(),
                                );
                            });
                            // Click to jump to this event
                            if resp.response.clicked() {
                                if let Some(ri) = msg.record_index {
                                    action = InspectorAction::JumpToEvent(ri);
                                }
                            }
                        });
                    }
                });
        });

    action
}
