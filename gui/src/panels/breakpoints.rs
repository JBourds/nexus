use egui::Ui;

use crate::state::{Breakpoint, BreakpointKind};

/// Action from the breakpoints panel.
pub enum BreakpointsAction {
    None,
    /// Add a new breakpoint.
    Add(Breakpoint),
}

/// Show the breakpoints panel/section.
pub fn show_breakpoints(ui: &mut Ui, breakpoints: &mut Vec<Breakpoint>) -> BreakpointsAction {
    let mut action = BreakpointsAction::None;
    let mut to_remove = Vec::new();

    ui.heading("Breakpoints");
    ui.separator();

    if breakpoints.is_empty() {
        ui.label("No breakpoints set");
    } else {
        for (i, bp) in breakpoints.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                // Enable/disable toggle
                ui.checkbox(&mut bp.enabled, "");

                // Description
                let desc = match &bp.kind {
                    BreakpointKind::Timestep(ts) => format!("t={ts}"),
                    BreakpointKind::NodeEvent(name) => format!("node: {name}"),
                    BreakpointKind::ChannelActivity(ch) => format!("channel: {ch}"),
                };
                let color = if bp.enabled {
                    egui::Color32::from_rgb(255, 80, 80)
                } else {
                    egui::Color32::from_gray(120)
                };
                ui.colored_label(color, desc);

                // Remove button
                if ui.small_button("\u{2717}").on_hover_text("Remove").clicked() {
                    to_remove.push(i);
                }
            });
        }
    }

    // Remove in reverse order to preserve indices
    for i in to_remove.into_iter().rev() {
        breakpoints.remove(i);
    }

    ui.separator();

    // Quick-add section
    ui.horizontal(|ui| {
        ui.label("Add:");
        if ui.button("Timestep...").clicked() {
            // Add a timestep breakpoint at current timestep (caller can set the value)
            action = BreakpointsAction::Add(Breakpoint {
                kind: BreakpointKind::Timestep(0),
                enabled: true,
            });
        }
    });

    action
}

/// Check if any enabled breakpoint matches the given event context.
/// Returns true if playback should pause.
pub fn check_breakpoints(
    breakpoints: &[Breakpoint],
    timestep: u64,
    event: &trace::format::TraceEvent,
    sim: &config::ast::Simulation,
) -> bool {
    use trace::format::TraceEvent;

    for bp in breakpoints {
        if !bp.enabled {
            continue;
        }
        let matches = match &bp.kind {
            BreakpointKind::Timestep(ts) => timestep == *ts,
            BreakpointKind::NodeEvent(name) => {
                let node_idx = match event {
                    TraceEvent::MessageSent { src_node, .. } => Some(*src_node),
                    TraceEvent::MessageRecv { dst_node, .. } => Some(*dst_node),
                    TraceEvent::MessageDropped { src_node, .. } => Some(*src_node),
                    TraceEvent::PositionUpdate { node, .. } => Some(*node),
                    TraceEvent::EnergyUpdate { node, .. } => Some(*node),
                    TraceEvent::MotionUpdate { node, .. } => Some(*node),
                };
                if let Some(idx) = node_idx {
                    let node_name = node_name_by_index(sim, idx as usize);
                    &node_name == name
                } else {
                    false
                }
            }
            BreakpointKind::ChannelActivity(ch_name) => {
                let ch_idx = match event {
                    TraceEvent::MessageSent { channel, .. } => Some(*channel),
                    TraceEvent::MessageRecv { channel, .. } => Some(*channel),
                    TraceEvent::MessageDropped { channel, .. } => Some(*channel),
                    _ => None,
                };
                if let Some(idx) = ch_idx {
                    let name = channel_name_by_index(sim, idx as usize);
                    &name == ch_name
                } else {
                    false
                }
            }
        };
        if matches {
            return true;
        }
    }
    false
}

fn node_name_by_index(sim: &config::ast::Simulation, index: usize) -> String {
    let mut names: Vec<_> = sim.nodes.keys().cloned().collect();
    names.sort();
    names
        .get(index)
        .cloned()
        .unwrap_or_else(|| format!("node_{index}"))
}

fn channel_name_by_index(sim: &config::ast::Simulation, index: usize) -> String {
    let mut names: Vec<_> = sim.channels.keys().cloned().collect();
    names.sort();
    names
        .get(index)
        .cloned()
        .unwrap_or_else(|| format!("ch_{index}"))
}
