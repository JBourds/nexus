use egui::Ui;

use crate::state::{Breakpoint, BreakpointInput, BreakpointKind};

/// Action from the breakpoints panel.
pub enum BreakpointsAction {
    None,
    /// Add a new breakpoint.
    Add(Breakpoint),
    /// Set a one-shot "run until" condition.
    RunUntil(BreakpointKind),
}

/// Show the breakpoints panel/section.
///
/// `run_until` is `None` when in config editor mode (no run-until available).
/// `node_names` and `channel_names` are sorted lists for the searchable lists.
/// `input` holds persistent text buffer state across frames.
pub fn show_breakpoints(
    ui: &mut Ui,
    breakpoints: &mut Vec<Breakpoint>,
    run_until: Option<&mut Option<BreakpointKind>>,
    current_timestep: u64,
    node_names: &[String],
    channel_names: &[String],
    input: &mut BreakpointInput,
) -> BreakpointsAction {
    let mut action = BreakpointsAction::None;
    let mut to_remove = Vec::new();

    // --- Existing breakpoints list ---
    if breakpoints.is_empty() {
        ui.label("No breakpoints set");
    } else {
        for (i, bp) in breakpoints.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut bp.enabled, "");

                let desc = describe_kind(&bp.kind);
                let color = if bp.enabled {
                    egui::Color32::from_rgb(255, 80, 80)
                } else {
                    egui::Color32::from_gray(120)
                };
                ui.colored_label(color, desc);

                if ui
                    .small_button("\u{2717}")
                    .on_hover_text("Remove")
                    .clicked()
                {
                    to_remove.push(i);
                }
            });
        }
    }

    for i in to_remove.into_iter().rev() {
        breakpoints.remove(i);
    }

    ui.separator();

    // --- Add breakpoint section ---
    ui.label("Add breakpoint:");

    // Timestep input
    ui.horizontal(|ui| {
        ui.label("Timestep:");
        let te = egui::TextEdit::singleline(&mut input.timestep_buf)
            .desired_width(60.0)
            .hint_text(current_timestep.to_string());
        ui.add(te);
        if ui
            .button("+")
            .on_hover_text("Add timestep breakpoint")
            .clicked()
        {
            let ts = input
                .timestep_buf
                .trim()
                .parse::<u64>()
                .unwrap_or(current_timestep);
            action = BreakpointsAction::Add(Breakpoint {
                kind: BreakpointKind::Timestep(ts),
                enabled: true,
            });
            input.timestep_buf.clear();
        }
    });

    // Node event breakpoint (collapsible + searchable)
    if !node_names.is_empty() {
        egui::CollapsingHeader::new("Node breakpoints")
            .id_salt("bp_nodes")
            .show(ui, |ui| {
                show_searchable_list(
                    ui,
                    &mut input.node_search,
                    node_names,
                    "Filter nodes...",
                    "bp_node_search",
                    |name| {
                        action = BreakpointsAction::Add(Breakpoint {
                            kind: BreakpointKind::NodeEvent(name),
                            enabled: true,
                        });
                    },
                );
            });
    }

    // Channel activity breakpoint (collapsible + searchable)
    if !channel_names.is_empty() {
        egui::CollapsingHeader::new("Channel breakpoints")
            .id_salt("bp_channels")
            .show(ui, |ui| {
                show_searchable_list(
                    ui,
                    &mut input.channel_search,
                    channel_names,
                    "Filter channels...",
                    "bp_ch_search",
                    |name| {
                        action = BreakpointsAction::Add(Breakpoint {
                            kind: BreakpointKind::ChannelActivity(name),
                            enabled: true,
                        });
                    },
                );
            });
    }

    // --- Run-Until section (only in live sim / replay) ---
    if let Some(run_until) = run_until {
        ui.separator();
        ui.label("Run Until");

        let has_run_until = run_until.is_some();
        if has_run_until {
            let desc = describe_kind(run_until.as_ref().unwrap());
            let mut clear = false;
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 200, 80), desc);
                if ui.small_button("Clear").clicked() {
                    clear = true;
                }
            });
            if clear {
                *run_until = None;
            }
        } else {
            // Next event
            if ui
                .button("Next event")
                .on_hover_text("Run until the next trace event occurs")
                .clicked()
            {
                action = BreakpointsAction::RunUntil(BreakpointKind::NextEvent);
            }

            // Run until timestep
            ui.horizontal(|ui| {
                ui.label("Until t=");
                let te = egui::TextEdit::singleline(&mut input.timestep_buf)
                    .desired_width(60.0)
                    .hint_text((current_timestep + 10).to_string());
                ui.add(te);
                if ui
                    .button("Go")
                    .on_hover_text("Run until this timestep")
                    .clicked()
                {
                    let ts = input
                        .timestep_buf
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(current_timestep + 10);
                    action = BreakpointsAction::RunUntil(BreakpointKind::Timestep(ts));
                    input.timestep_buf.clear();
                }
            });

            // Run until node event (collapsible + searchable)
            if !node_names.is_empty() {
                egui::CollapsingHeader::new("Until node event")
                    .id_salt("ru_nodes")
                    .show(ui, |ui| {
                        show_searchable_list(
                            ui,
                            &mut input.node_search,
                            node_names,
                            "Filter nodes...",
                            "ru_node_search",
                            |name| {
                                action = BreakpointsAction::RunUntil(
                                    BreakpointKind::NodeEvent(name),
                                );
                            },
                        );
                    });
            }

            // Run until channel activity (collapsible + searchable)
            if !channel_names.is_empty() {
                egui::CollapsingHeader::new("Until channel activity")
                    .id_salt("ru_channels")
                    .show(ui, |ui| {
                        show_searchable_list(
                            ui,
                            &mut input.channel_search,
                            channel_names,
                            "Filter channels...",
                            "ru_ch_search",
                            |name| {
                                action = BreakpointsAction::RunUntil(
                                    BreakpointKind::ChannelActivity(name),
                                );
                            },
                        );
                    });
            }
        }
    }

    action
}

/// Render a searchable list of names as clickable buttons.
fn show_searchable_list(
    ui: &mut Ui,
    search_buf: &mut String,
    names: &[String],
    hint: &str,
    id_salt: &str,
    mut on_click: impl FnMut(String),
) {
    ui.horizontal(|ui| {
        ui.label("Search:");
        ui.add(
            egui::TextEdit::singleline(search_buf)
                .desired_width(100.0)
                .hint_text(hint)
                .id_salt(id_salt),
        );
    });

    let query = search_buf.trim().to_lowercase();
    let filtered: Vec<_> = if query.is_empty() {
        names.to_vec()
    } else {
        names
            .iter()
            .filter(|n| n.to_lowercase().contains(&query))
            .cloned()
            .collect()
    };

    if filtered.is_empty() {
        ui.label("(no matches)");
    } else {
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .id_salt(format!("{id_salt}_scroll"))
            .show(ui, |ui| {
                for name in &filtered {
                    if ui.small_button(name).clicked() {
                        on_click(name.clone());
                    }
                }
            });
    }
}

fn describe_kind(kind: &BreakpointKind) -> String {
    match kind {
        BreakpointKind::NextEvent => "next event".to_string(),
        BreakpointKind::Timestep(ts) => format!("t={ts}"),
        BreakpointKind::NodeEvent(name) => format!("node: {name}"),
        BreakpointKind::ChannelActivity(ch) => format!("channel: {ch}"),
    }
}

/// Check if any enabled timestep breakpoint matches (no event required).
/// Returns true if playback should pause at this timestep.
pub fn check_timestep_breakpoints(breakpoints: &[Breakpoint], timestep: u64) -> bool {
    for bp in breakpoints {
        if !bp.enabled {
            continue;
        }
        if let BreakpointKind::Timestep(ts) = &bp.kind {
            if timestep == *ts {
                return true;
            }
        }
    }
    false
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
            BreakpointKind::NextEvent => true,
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
