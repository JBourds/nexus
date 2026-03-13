use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::App;
use egui::Context;

use config::ast::DistanceUnit;

use crate::config_editor;
use crate::panels::{breakpoints, grid, inspector, messages, sequence, timeline, toolbar};
use crate::render::grid::GridView;
use crate::sim::bridge::GuiEvent;
use crate::state::*;
use trace::format::TraceEvent;

pub struct NexusApp {
    pub mode: AppMode,
}

impl NexusApp {
    pub fn new_with_config(p: PathBuf) -> Result<Self> {
        let state = ConfigEditorState::new(p)?;
        Ok(Self {
            mode: AppMode::ConfigEditor(Box::new(state)),
        })
    }
}

impl Default for NexusApp {
    fn default() -> Self {
        Self {
            mode: AppMode::Home,
        }
    }
}

impl App for NexusApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Toolbar
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            let sim_finished = match &self.mode {
                AppMode::LiveSimulation(state) => state.controller.is_finished(),
                _ => false,
            };
            let panels = match &self.mode {
                AppMode::LiveSimulation(state) => Some(&state.panels),
                AppMode::Replay(state) => Some(&state.panels),
                _ => None,
            };
            let view_mode = match &self.mode {
                AppMode::LiveSimulation(state) => Some(state.view_mode),
                AppMode::Replay(state) => Some(state.view_mode),
                _ => None,
            };
            let action = toolbar::show_toolbar(ui, &self.mode, sim_finished, panels, view_mode);
            match action {
                toolbar::ToolbarAction::GoHome => {
                    self.mode = AppMode::Home;
                }
                toolbar::ToolbarAction::OpenConfig => {
                    self.open_config();
                }
                toolbar::ToolbarAction::NewConfig => {
                    self.new_config();
                }
                toolbar::ToolbarAction::OpenTrace => {
                    self.open_trace();
                }
                toolbar::ToolbarAction::RunSimulation => {
                    self.run_simulation();
                }
                toolbar::ToolbarAction::StopSimulation => {
                    if let AppMode::LiveSimulation(state) = &self.mode {
                        state.controller.stop();
                    }
                }
                toolbar::ToolbarAction::RerunSimulation => {
                    self.rerun_simulation();
                }
                toolbar::ToolbarAction::ToggleInspector => match &mut self.mode {
                    AppMode::LiveSimulation(state) => {
                        state.panels.inspector = !state.panels.inspector;
                    }
                    AppMode::Replay(state) => {
                        state.panels.inspector = !state.panels.inspector;
                    }
                    _ => {}
                },
                toolbar::ToolbarAction::ToggleMessages => match &mut self.mode {
                    AppMode::LiveSimulation(state) => {
                        state.panels.messages = !state.panels.messages;
                    }
                    AppMode::Replay(state) => {
                        state.panels.messages = !state.panels.messages;
                    }
                    _ => {}
                },
                toolbar::ToolbarAction::ToggleViewMode => match &mut self.mode {
                    AppMode::LiveSimulation(state) => {
                        state.view_mode = match state.view_mode {
                            ViewMode::Grid => ViewMode::Sequence,
                            ViewMode::Sequence => ViewMode::Grid,
                        };
                    }
                    AppMode::Replay(state) => {
                        state.view_mode = match state.view_mode {
                            ViewMode::Grid => ViewMode::Sequence,
                            ViewMode::Sequence => ViewMode::Grid,
                        };
                    }
                    _ => {}
                },
                toolbar::ToolbarAction::None => {}
            }
        });

        match &mut self.mode {
            AppMode::Home => self.show_home(ctx),
            AppMode::ConfigEditor(_) => self.show_config_editor_mode(ctx),
            AppMode::LiveSimulation(_) => self.show_live_sim_mode(ctx),
            AppMode::Replay(_) => self.show_replay_mode(ctx),
        }
    }
}

impl NexusApp {
    fn show_home(&mut self, ctx: &Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading("Nexus Network Simulator");
                ui.add_space(20.0);
                ui.label("Discrete-event network simulator for testing production protocol code");
                ui.add_space(40.0);

                if ui.button("Open Configuration File").clicked() {
                    self.open_config();
                }
                ui.add_space(10.0);
                if ui.button("New Configuration").clicked() {
                    self.new_config();
                }
                ui.add_space(10.0);
                if ui.button("Open Trace File").clicked() {
                    self.open_trace();
                }
            });
        });
    }

    fn show_config_editor_mode(&mut self, ctx: &Context) {
        let AppMode::ConfigEditor(state) = &mut self.mode else {
            return;
        };

        // Right panel: breakpoints (pre-simulation)
        egui::SidePanel::right("config_breakpoints")
            .default_width(180.0)
            .resizable(true)
            .show(ctx, |ui| {
                let mut node_names: Vec<_> = state.sim.nodes.keys().cloned().collect();
                node_names.sort();
                let mut channel_names: Vec<_> = state.sim.channels.keys().cloned().collect();
                channel_names.sort();
                let bp_action = breakpoints::show_breakpoints(
                    ui,
                    &mut state.breakpoints,
                    None,
                    0,
                    &node_names,
                    &channel_names,
                    &mut state.bp_input,
                );
                if let breakpoints::BreakpointsAction::Add(bp) = bp_action {
                    state.breakpoints.push(bp);
                }
            });

        // Central panel with grid for node placement
        egui::CentralPanel::default().show(ctx, |ui| {
            // Left: config editor panel
            config_editor::show_config_editor(ui, state);

            // Remaining: grid view with nodes
            let nodes = nodes_from_sim(&state.sim);
            if state.needs_fit {
                state.grid.fit_to_nodes(&nodes, ui.available_size());
                state.needs_fit = false;
            }
            let dist_unit = sim_distance_unit(&state.sim);
            let no_highlights = HashMap::new();
            let (clicked, _hovered) = grid::show_grid_panel(
                ui,
                &mut state.grid,
                &nodes,
                &state.selected_node,
                &[],
                dist_unit,
                &no_highlights,
            );
            if let Some(clicked) = clicked {
                state.selected_node = Some(clicked);
            }
        });
    }

    fn show_live_sim_mode(&mut self, ctx: &Context) {
        let AppMode::LiveSimulation(state) = &mut self.mode else {
            return;
        };

        // Process events from simulation (only when not paused)
        let egui_time = ctx.input(|i| i.time);
        if !state.paused {
            // Expire finished arrow animations (unless frozen)
            if !state.arrows_frozen {
                state
                    .active_arrows
                    .retain(|a| (egui_time - a.start_time) < a.duration as f64);
            } else {
                // Unfreeze when simulation resumes
                state.arrows_frozen = false;
            }

            while let Ok(event) = state.controller.rx.try_recv() {
                let mut should_pause = false;
                match &event {
                    GuiEvent::Trace(record) => {
                        state.all_records.push(record.clone());
                        // Check event-based breakpoints
                        if breakpoints::check_breakpoints(
                            &state.breakpoints,
                            record.timestep,
                            &record.event,
                            &state.sim,
                        ) {
                            should_pause = true;
                        }
                        // Check run-until (one-shot)
                        if let Some(ref kind) = state.run_until {
                            let run_bp = Breakpoint {
                                kind: kind.clone(),
                                enabled: true,
                            };
                            if breakpoints::check_breakpoints(
                                &[run_bp],
                                record.timestep,
                                &record.event,
                                &state.sim,
                            ) {
                                should_pause = true;
                                state.arrows_frozen = true;
                                state.run_until = None;
                            }
                        }
                    }
                    GuiEvent::TimestepAdvanced(ts) => {
                        // Check timestep breakpoints in the range
                        // (current_timestep, reported_ts] since updates are downsampled
                        let prev = state.current_timestep;
                        for bp in &state.breakpoints {
                            if !bp.enabled {
                                continue;
                            }
                            if let BreakpointKind::Timestep(bp_ts) = &bp.kind {
                                if *bp_ts > prev && *bp_ts <= *ts {
                                    should_pause = true;
                                    break;
                                }
                            }
                        }
                        // Check run-until timestep (one-shot)
                        if let Some(BreakpointKind::Timestep(target)) = &state.run_until {
                            if *target > prev && *target <= *ts {
                                should_pause = true;
                                state.arrows_frozen = true;
                                state.run_until = None;
                            }
                        }
                    }
                    _ => {}
                }
                // Process the event, then stop if breakpoint hit
                process_gui_event(
                    event,
                    &mut state.current_timestep,
                    &mut state.messages,
                    &mut state.node_states,
                    &state.sim,
                    &mut state.active_arrows,
                    &state.channel_subscribers,
                    &mut state.last_sender,
                    egui_time,
                );
                if should_pause {
                    state.paused = true;
                    state.controller.set_paused(true);
                    break;
                }
            }
        }

        // Left panel: inspector + messages (collapsible sections)
        if state.panels.inspector {
            egui::SidePanel::left("inspector")
                .default_width(220.0)
                .resizable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::CollapsingHeader::new("Inspector")
                            .default_open(true)
                            .show(ui, |ui| {
                                let insp_action = inspector::show_inspector(
                                    ui,
                                    &state.sim,
                                    &state.node_states,
                                    &state.selected_node,
                                    &mut state.expanded_nodes,
                                    &state.messages,
                                    state.event_cursor,
                                );
                                if let inspector::InspectorAction::JumpToEvent(idx) = insp_action {
                                    state.event_cursor = Some(idx);
                                    state.event_stepping = true;
                                }
                            });

                        if state.panels.messages {
                            ui.separator();

                            egui::CollapsingHeader::new("Messages")
                                .default_open(true)
                                .show(ui, |ui| {
                                    let msg_action = messages::show_messages(
                                        ui,
                                        &state.messages,
                                        200,
                                        state.event_cursor,
                                        &mut state.expanded_messages,
                                    );
                                    match msg_action {
                                        messages::MessagesAction::SelectNode(name) => {
                                            state.expanded_nodes.clear();
                                            state.expanded_nodes.insert(name.clone());
                                            state.selected_node = Some(name);
                                        }
                                        messages::MessagesAction::JumpToEvent(idx) => {
                                            state.event_cursor = Some(idx);
                                            state.event_stepping = true;
                                        }
                                        messages::MessagesAction::None => {}
                                    }
                                });
                        }
                    });
                });
        }

        // Right panel: debugger (breakpoints + run-until)
        egui::SidePanel::right("debugger")
            .default_width(180.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut node_names: Vec<_> = state.sim.nodes.keys().cloned().collect();
                    node_names.sort();
                    let mut channel_names: Vec<_> = state.sim.channels.keys().cloned().collect();
                    channel_names.sort();
                    let bp_action = breakpoints::show_breakpoints(
                        ui,
                        &mut state.breakpoints,
                        Some(&mut state.run_until),
                        state.current_timestep,
                        &node_names,
                        &channel_names,
                        &mut state.bp_input,
                    );
                    match bp_action {
                        breakpoints::BreakpointsAction::Add(bp) => {
                            state.breakpoints.push(bp);
                        }
                        breakpoints::BreakpointsAction::RunUntil(kind) => {
                            state.run_until = Some(kind);
                        }
                        breakpoints::BreakpointsAction::None => {}
                    }
                });
            });

        // Timeline at bottom
        let total = state.sim.params.timestep.count.get();
        let finished = state.controller.is_finished();
        // Sync pause state: when the simulation finishes, mark as paused so
        // the play/pause button shows the correct state. Also snap the
        // displayed timestep to the total count to avoid an off-by-one.
        if finished && !state.paused {
            state.paused = true;
            state.current_timestep = total;
        }
        let mut view_replay = false;
        egui::TopBottomPanel::bottom("timeline").show(ctx, |ui| {
            let mut playing = !state.paused;
            // Speed control reads/writes the kernel's time_dilation atomic directly.
            let mut speed = f64::from_bits(
                state
                    .time_dilation
                    .load(std::sync::atomic::Ordering::Relaxed),
            ) as f32;
            let total_records = state.all_records.len();
            let action = timeline::show_timeline(
                ui,
                &mut state.current_timestep,
                total,
                &mut playing,
                &mut speed,
                &mut state.event_stepping,
                state.event_cursor,
                total_records,
                &state.breakpoints,
            );
            // Push speed changes back to the kernel's time_dilation atomic.
            state
                .time_dilation
                .store((speed as f64).to_bits(), std::sync::atomic::Ordering::Relaxed);
            if action.toggle_play {
                state.paused = !state.paused;
                state.controller.set_paused(state.paused);
            }
            if finished {
                ui.horizontal(|ui| {
                    ui.label("Simulation complete.");
                    if ui.button("View Replay").clicked() {
                        view_replay = true;
                    }
                });
            }
        });

        // Central panel: Grid or Sequence diagram
        egui::CentralPanel::default().show(ctx, |ui| {
            match state.view_mode {
                ViewMode::Sequence => {
                    let node_names: Vec<String> =
                        state.node_states.iter().map(|n| n.name.clone()).collect();
                    let seq_action = sequence::show_sequence_diagram(
                        ui,
                        &state.messages,
                        &node_names,
                        state.current_timestep,
                        state.event_cursor,
                        &mut state.seq_zoom,
                    );
                    if let sequence::SequenceAction::JumpToEvent {
                        record_index,
                        node,
                    } = seq_action
                    {
                        state.event_cursor = Some(record_index);
                        state.event_stepping = true;
                        state.expanded_nodes.clear();
                        state.expanded_nodes.insert(node.clone());
                        state.selected_node = Some(node);
                    }
                }
                ViewMode::Grid => {
                    if state.needs_fit {
                        state
                            .grid
                            .fit_to_nodes(&state.node_states, ui.available_size());
                        state.needs_fit = false;
                    }
                    let dist_unit = sim_distance_unit(&state.sim);
                    let highlights =
                        build_receiver_highlights(&state.messages, &state.expanded_messages);
                    let (clicked, hovered) = grid::show_grid_panel(
                        ui,
                        &mut state.grid,
                        &state.node_states,
                        &state.selected_node,
                        &state.active_arrows,
                        dist_unit,
                        &highlights,
                    );
                    if let Some(clicked) = clicked {
                        let already_selected = state.selected_node.as_ref() == Some(&clicked);
                        state.expanded_nodes.clear();
                        if already_selected {
                            state.selected_node = None;
                        } else {
                            state.expanded_nodes.insert(clicked.clone());
                            state.selected_node = Some(clicked);
                        }
                    }
                    state.hovered_node = hovered;
                    if let Some(ref name) = state.hovered_node
                        && let Some(n) = state.node_states.iter().find(|n| &n.name == name)
                    {
                        egui::containers::popup::show_tooltip_at_pointer(
                            ui.ctx(),
                            egui::LayerId::new(
                                egui::Order::Tooltip,
                                ui.id().with("node_tip"),
                            ),
                            ui.id().with("node_tip"),
                            |ui| {
                                ui.label(format!(
                                    "{} ({:.1}, {:.1}, {:.1})",
                                    n.name, n.x, n.y, n.z
                                ));
                                if n.motion_spec != "none" {
                                    ui.label(format!("Motion: {}", n.motion_spec));
                                }
                                if let Some(r) = n.charge_ratio {
                                    ui.label(format!("Charge: {:.0}%", r * 100.0));
                                }
                            },
                        );
                    }
                }
            }
        });

        // Keep requesting repaints during live sim, during active animations,
        // and one extra frame after completion for remaining buffered events.
        if !finished || state.controller.has_pending_events() || !state.active_arrows.is_empty() {
            ctx.request_repaint();
        }

        if view_replay {
            self.try_transition_to_replay();
        }
    }

    fn try_transition_to_replay(&mut self) {
        let AppMode::LiveSimulation(state) = &self.mode else {
            return;
        };
        let trace_path = state.sim_dir.join("trace.nxs");
        if let Ok(controller) = crate::sim::replay::ReplayController::open(&trace_path) {
            let sim = state.sim.clone();
            let initial_states = nodes_from_sim(&sim);
            let total_timesteps = controller.total_timesteps;
            let channel_subscribers = controller.build_channel_subscribers();
            let num_channels = controller.num_channels();
            self.mode = AppMode::Replay(Box::new(ReplayState {
                sim,
                controller,
                grid: GridView::default(),
                selected_node: None,
                current_timestep: 0,
                total_timesteps,
                playing: false,
                playback_speed: 1.0,
                messages: Vec::new(),
                node_states: initial_states.clone(),
                initial_states,
                needs_fit: true,
                expanded_nodes: HashSet::new(),
                hovered_node: None,
                panels: PanelVisibility::default(),
                active_arrows: Vec::new(),
                channel_subscribers,
                last_sender: vec![None; num_channels],
                time_accumulator: 0.0,
                arrows_frozen: false,
                run_until: None,
                event_cursor: None,
                event_stepping: false,
                breakpoints: Vec::new(),
                expanded_messages: HashSet::new(),
                view_mode: ViewMode::default(),
                bp_input: BreakpointInput::default(),
                seq_zoom: 1.0,
            }));
        }
    }

    fn show_replay_mode(&mut self, ctx: &Context) {
        let AppMode::Replay(state) = &mut self.mode else {
            return;
        };

        // Expire finished arrow animations (unless frozen by run-until trigger)
        let egui_time = ctx.input(|i| i.time);
        if !state.arrows_frozen {
            state
                .active_arrows
                .retain(|a| (egui_time - a.start_time) < a.duration as f64);
        }

        // Unfreeze arrows when playback resumes
        if state.playing && state.arrows_frozen {
            state.arrows_frozen = false;
        }

        // Advance timesteps proportional to real time and playback speed
        if state.playing && state.current_timestep < state.total_timesteps.saturating_sub(1) {
            let dt = ctx.input(|i| i.stable_dt) as f64;
            use config::ast::TimeUnit;
            let unit_seconds = match state.sim.params.timestep.unit {
                TimeUnit::Hours => 3600.0,
                TimeUnit::Minutes => 60.0,
                TimeUnit::Seconds => 1.0,
                TimeUnit::Milliseconds => 1e-3,
                TimeUnit::Microseconds => 1e-6,
                TimeUnit::Nanoseconds => 1e-9,
            };
            let ts_duration = state.sim.params.timestep.length.get() as f64 * unit_seconds;
            state.time_accumulator += dt * state.playback_speed as f64;
            let steps = (state.time_accumulator / ts_duration).floor() as u64;
            state.time_accumulator -= steps as f64 * ts_duration;

            let max_ts = state.total_timesteps.saturating_sub(1);
            let target_ts = (state.current_timestep + steps).min(max_ts);
            if target_ts > state.current_timestep {
                // Gather messages and arrows for each stepped timestep
                let msg_start = state.messages.len();
                let mut actual_target = target_ts;
                for ts in (state.current_timestep + 1)..=target_ts {
                    // Check timestep breakpoints (fire even without events at this ts)
                    let mut hit_breakpoint =
                        breakpoints::check_timestep_breakpoints(&state.breakpoints, ts);

                    // Check run-until timestep (one-shot, no event needed)
                    if !hit_breakpoint {
                        if let Some(BreakpointKind::Timestep(target)) = &state.run_until {
                            if ts >= *target {
                                hit_breakpoint = true;
                                state.arrows_frozen = true;
                                state.run_until = None;
                            }
                        }
                    }

                    // Check event-based breakpoints against records at this timestep
                    if !hit_breakpoint {
                        for record in state.controller.records_at(ts) {
                            if breakpoints::check_breakpoints(
                                &state.breakpoints,
                                ts,
                                &record.event,
                                &state.sim,
                            ) {
                                hit_breakpoint = true;
                                break;
                            }
                            // Check run-until event conditions (one-shot)
                            if let Some(ref kind) = state.run_until {
                                let run_bp = Breakpoint {
                                    kind: kind.clone(),
                                    enabled: true,
                                };
                                if breakpoints::check_breakpoints(
                                    &[run_bp],
                                    ts,
                                    &record.event,
                                    &state.sim,
                                ) {
                                    hit_breakpoint = true;
                                    state.arrows_frozen = true;
                                    state.run_until = None;
                                    break;
                                }
                            }
                        }
                    }
                    gather_messages_at(&state.controller, ts, &state.sim, &mut state.messages);
                    gather_arrows_at(
                        &state.controller,
                        ts,
                        &mut state.active_arrows,
                        &state.channel_subscribers,
                        &mut state.last_sender,
                        egui_time,
                    );
                    if hit_breakpoint {
                        actual_target = ts;
                        state.playing = false;
                        break;
                    }
                }
                // Correlate TX receivers for newly added messages
                correlate_all_tx_receivers(
                    &state.controller,
                    &state.sim,
                    &mut state.messages[msg_start..],
                );
                state.current_timestep = actual_target;
                state.node_states = state
                    .controller
                    .reconstruct_states(state.current_timestep, &state.initial_states);
            }
        }

        // Left panel: inspector + messages (collapsible sections)
        if state.panels.inspector {
            egui::SidePanel::left("inspector")
                .default_width(220.0)
                .resizable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::CollapsingHeader::new("Inspector")
                            .default_open(true)
                            .show(ui, |ui| {
                                let insp_action = inspector::show_inspector(
                                    ui,
                                    &state.sim,
                                    &state.node_states,
                                    &state.selected_node,
                                    &mut state.expanded_nodes,
                                    &state.messages,
                                    state.event_cursor,
                                );
                                if let inspector::InspectorAction::JumpToEvent(idx) = insp_action {
                                    state.event_cursor = Some(idx);
                                    state.event_stepping = true;
                                }
                            });

                        if state.panels.messages {
                            ui.separator();

                            egui::CollapsingHeader::new("Messages")
                                .default_open(true)
                                .show(ui, |ui| {
                                    let msg_action = messages::show_messages(
                                        ui,
                                        &state.messages,
                                        200,
                                        state.event_cursor,
                                        &mut state.expanded_messages,
                                    );
                                    match msg_action {
                                        messages::MessagesAction::SelectNode(name) => {
                                            state.expanded_nodes.clear();
                                            state.expanded_nodes.insert(name.clone());
                                            state.selected_node = Some(name);
                                        }
                                        messages::MessagesAction::JumpToEvent(idx) => {
                                            state.event_cursor = Some(idx);
                                            state.event_stepping = true;
                                        }
                                        messages::MessagesAction::None => {}
                                    }
                                });
                        }
                    });
                });
        }

        // Right panel: debugger (breakpoints + run-until)
        egui::SidePanel::right("debugger")
            .default_width(180.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut node_names: Vec<_> = state.sim.nodes.keys().cloned().collect();
                    node_names.sort();
                    let mut channel_names: Vec<_> = state.sim.channels.keys().cloned().collect();
                    channel_names.sort();
                    let bp_action = breakpoints::show_breakpoints(
                        ui,
                        &mut state.breakpoints,
                        Some(&mut state.run_until),
                        state.current_timestep,
                        &node_names,
                        &channel_names,
                        &mut state.bp_input,
                    );
                    match bp_action {
                        breakpoints::BreakpointsAction::Add(bp) => {
                            state.breakpoints.push(bp);
                        }
                        breakpoints::BreakpointsAction::RunUntil(kind) => {
                            state.run_until = Some(kind);
                        }
                        breakpoints::BreakpointsAction::None => {}
                    }
                });
            });

        // Timeline
        let total_records = state.controller.total_records();
        egui::TopBottomPanel::bottom("timeline").show(ctx, |ui| {
            let action = timeline::show_timeline(
                ui,
                &mut state.current_timestep,
                state.total_timesteps,
                &mut state.playing,
                &mut state.playback_speed,
                &mut state.event_stepping,
                state.event_cursor,
                total_records,
                &state.breakpoints,
            );

            if action.toggle_play {
                state.playing = !state.playing;
            }

            // Helper: seek to a timestep (shared by seek_to and event stepping)
            let seek_to_ts = |state: &mut ReplayState, ts: u64, egui_time: f64| {
                state.current_timestep = ts;
                state.playing = false;
                state.node_states = state
                    .controller
                    .reconstruct_states(ts, &state.initial_states);
                state.messages.clear();
                gather_messages_through(&state.controller, ts, &state.sim, &mut state.messages);
                correlate_all_tx_receivers(&state.controller, &state.sim, &mut state.messages);
                state.active_arrows.clear();
                state.last_sender.fill(None);
                gather_arrows_at(
                    &state.controller,
                    ts,
                    &mut state.active_arrows,
                    &state.channel_subscribers,
                    &mut state.last_sender,
                    egui_time,
                );
            };

            if let Some(ts) = action.seek_to {
                seek_to_ts(state, ts, egui_time);
                // Snap event cursor to first record at this timestep
                if state.event_stepping {
                    state.event_cursor = state.controller.first_record_index_at(ts);
                }
            }
            if action.step_forward {
                state.playing = false;
                if state.event_stepping {
                    // Event-level step forward
                    let next = state.event_cursor.map(|c| c + 1).unwrap_or(0);
                    if next < total_records {
                        state.event_cursor = Some(next);
                        if let Some(ts) = state.controller.timestep_for_record(next) {
                            if ts != state.current_timestep {
                                seek_to_ts(state, ts, egui_time);
                            }
                            state.current_timestep = ts;
                        }
                    }
                } else if state.current_timestep < state.total_timesteps.saturating_sub(1) {
                    state.current_timestep += 1;
                    state.node_states = state
                        .controller
                        .reconstruct_states(state.current_timestep, &state.initial_states);
                    let msg_start = state.messages.len();
                    gather_messages_at(
                        &state.controller,
                        state.current_timestep,
                        &state.sim,
                        &mut state.messages,
                    );
                    correlate_all_tx_receivers(
                        &state.controller,
                        &state.sim,
                        &mut state.messages[msg_start..],
                    );
                    gather_arrows_at(
                        &state.controller,
                        state.current_timestep,
                        &mut state.active_arrows,
                        &state.channel_subscribers,
                        &mut state.last_sender,
                        egui_time,
                    );
                }
            }
            if action.step_backward {
                state.playing = false;
                if state.event_stepping {
                    // Event-level step backward
                    if let Some(cursor) = state.event_cursor {
                        let prev = cursor.saturating_sub(1);
                        state.event_cursor = Some(prev);
                        if let Some(ts) = state.controller.timestep_for_record(prev) {
                            seek_to_ts(state, ts, egui_time);
                        }
                    }
                } else {
                    state.current_timestep = state.current_timestep.saturating_sub(1);
                    seek_to_ts(state, state.current_timestep, egui_time);
                }
            }
        });

        // Central panel: Grid or Sequence diagram
        egui::CentralPanel::default().show(ctx, |ui| {
            match state.view_mode {
                ViewMode::Sequence => {
                    let node_names: Vec<String> =
                        state.node_states.iter().map(|n| n.name.clone()).collect();
                    let seq_action = sequence::show_sequence_diagram(
                        ui,
                        &state.messages,
                        &node_names,
                        state.current_timestep,
                        state.event_cursor,
                        &mut state.seq_zoom,
                    );
                    if let sequence::SequenceAction::JumpToEvent {
                        record_index,
                        node,
                    } = seq_action
                    {
                        state.event_cursor = Some(record_index);
                        state.event_stepping = true;
                        state.expanded_nodes.clear();
                        state.expanded_nodes.insert(node.clone());
                        state.selected_node = Some(node);
                    }
                }
                ViewMode::Grid => {
                    if state.needs_fit {
                        state
                            .grid
                            .fit_to_nodes(&state.node_states, ui.available_size());
                        state.needs_fit = false;
                    }
                    let dist_unit = sim_distance_unit(&state.sim);
                    let highlights =
                        build_receiver_highlights(&state.messages, &state.expanded_messages);
                    let (clicked, hovered) = grid::show_grid_panel(
                        ui,
                        &mut state.grid,
                        &state.node_states,
                        &state.selected_node,
                        &state.active_arrows,
                        dist_unit,
                        &highlights,
                    );
                    if let Some(clicked) = clicked {
                        let already_selected = state.selected_node.as_ref() == Some(&clicked);
                        state.expanded_nodes.clear();
                        if already_selected {
                            state.selected_node = None;
                        } else {
                            state.expanded_nodes.insert(clicked.clone());
                            state.selected_node = Some(clicked);
                        }
                    }
                    state.hovered_node = hovered;
                    if let Some(ref name) = state.hovered_node
                        && let Some(n) = state.node_states.iter().find(|n| &n.name == name)
                    {
                        egui::containers::popup::show_tooltip_at_pointer(
                            ui.ctx(),
                            egui::LayerId::new(
                                egui::Order::Tooltip,
                                ui.id().with("node_tip"),
                            ),
                            ui.id().with("node_tip"),
                            |ui| {
                                ui.label(format!(
                                    "{} ({:.1}, {:.1}, {:.1})",
                                    n.name, n.x, n.y, n.z
                                ));
                                if n.motion_spec != "none" {
                                    ui.label(format!("Motion: {}", n.motion_spec));
                                }
                                if let Some(r) = n.charge_ratio {
                                    ui.label(format!("Charge: {:.0}%", r * 100.0));
                                }
                            },
                        );
                    }
                }
            }
        });

        if state.playing || !state.active_arrows.is_empty() {
            ctx.request_repaint();
        }
    }

    fn run_simulation(&mut self) {
        let AppMode::ConfigEditor(state) = &mut self.mode else {
            return;
        };

        match crate::sim::launch::launch_simulation(state.sim.clone(), None) {
            Ok((controller, sim_dir, td)) => {
                let node_states = nodes_from_sim(&state.sim);
                let channel_subscribers = build_channel_subscribers(&state.sim);
                let num_channels = state.sim.channels.len();
                let pre_breakpoints = std::mem::take(&mut state.breakpoints);
                self.mode = AppMode::LiveSimulation(Box::new(LiveSimState {
                    sim: state.sim.clone(),
                    controller,
                    grid: GridView::default(),
                    selected_node: None,
                    current_timestep: 0,
                    messages: Vec::new(),
                    node_states,
                    sim_dir,
                    paused: false,
                    needs_fit: true,
                    expanded_nodes: HashSet::new(),
                    hovered_node: None,
                    panels: PanelVisibility::default(),
                    active_arrows: Vec::new(),
                    channel_subscribers,
                    last_sender: vec![None; num_channels],
                    time_dilation: td,
                    arrows_frozen: false,
                    run_until: None,
                    event_cursor: None,
                    event_stepping: false,
                    breakpoints: pre_breakpoints,
                    expanded_messages: HashSet::new(),
                    all_records: Vec::new(),
                    view_mode: ViewMode::default(),
                    bp_input: BreakpointInput::default(),
                    seq_zoom: 1.0,
                }));
            }
            Err(e) => {
                state.validation_error = Some(format!("Launch failed: {e:#}"));
            }
        }
    }

    fn rerun_simulation(&mut self) {
        let AppMode::LiveSimulation(state) = &mut self.mode else {
            return;
        };
        let sim = state.sim.clone();

        match crate::sim::launch::launch_simulation(sim.clone(), None) {
            Ok((controller, sim_dir, td)) => {
                let node_states = nodes_from_sim(&sim);
                let channel_subscribers = build_channel_subscribers(&sim);
                let num_channels = sim.channels.len();
                self.mode = AppMode::LiveSimulation(Box::new(LiveSimState {
                    sim,
                    controller,
                    grid: GridView::default(),
                    selected_node: None,
                    current_timestep: 0,
                    messages: Vec::new(),
                    node_states,
                    sim_dir,
                    paused: false,
                    needs_fit: true,
                    expanded_nodes: HashSet::new(),
                    hovered_node: None,
                    panels: PanelVisibility::default(),
                    active_arrows: Vec::new(),
                    channel_subscribers,
                    last_sender: vec![None; num_channels],
                    time_dilation: td,
                    arrows_frozen: false,
                    run_until: None,
                    event_cursor: None,
                    event_stepping: false,
                    breakpoints: Vec::new(),
                    expanded_messages: HashSet::new(),
                    all_records: Vec::new(),
                    view_mode: ViewMode::default(),
                    bp_input: BreakpointInput::default(),
                    seq_zoom: 1.0,
                }));
            }
            Err(e) => {
                eprintln!("Rerun failed: {e:#}");
            }
        }
    }

    fn open_config(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("TOML", &["toml"])
            .pick_file()
        {
            match ConfigEditorState::new(path) {
                Ok(mode) => {
                    self.mode = AppMode::ConfigEditor(Box::new(mode));
                }
                Err(e) => {
                    eprintln!("Failed to parse config: {e:#}");
                }
            }
        }
    }

    fn new_config(&mut self) {
        use config::ast::*;
        use std::collections::HashMap;
        use std::num::NonZeroU64;
        use std::time::SystemTime;

        let sim = Simulation {
            params: Params {
                timestep: TimestepConfig {
                    length: NonZeroU64::new(1).unwrap(),
                    unit: TimeUnit::Milliseconds,
                    count: NonZeroU64::new(100).unwrap(),
                    start: SystemTime::now(),
                },
                seed: 42,
                root: std::env::temp_dir().join("nexus"),
                time_dilation: 1.0,
            },
            channels: HashMap::new(),
            nodes: HashMap::new(),
        };
        self.mode = AppMode::ConfigEditor(Box::new(ConfigEditorState {
            sim,
            file_path: None,
            grid: GridView::default(),
            selected_node: None,
            selected_channel: None,
            validation_error: None,
            dirty: true,
            add_item_buf: String::new(),
            needs_fit: true,
            modules: crate::state::ModuleState::default(),
            breakpoints: Vec::new(),
            bp_input: BreakpointInput::default(),
        }));
    }

    fn open_trace(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Nexus Trace", &["nxs"])
            .pick_file()
        {
            match crate::sim::replay::ReplayController::open(&path) {
                Ok(controller) => {
                    // Try to load config from same directory
                    let config_path = path.parent().unwrap().join("nexus.toml");
                    let sim = if config_path.exists() {
                        config::deserialize_config(&config_path).ok()
                    } else {
                        None
                    };

                    let sim = sim.unwrap_or_else(|| {
                        // Create minimal sim from trace header
                        create_sim_from_trace_header(&controller)
                    });

                    let initial_states = nodes_from_sim(&sim);
                    let total_timesteps = controller.total_timesteps;
                    let channel_subscribers = controller.build_channel_subscribers();
                    let num_channels = controller.num_channels();

                    self.mode = AppMode::Replay(Box::new(ReplayState {
                        sim,
                        controller,
                        grid: GridView::default(),
                        selected_node: None,
                        current_timestep: 0,
                        total_timesteps,
                        playing: false,
                        playback_speed: 1.0,
                        messages: Vec::new(),
                        node_states: initial_states.clone(),
                        initial_states,
                        needs_fit: true,
                        expanded_nodes: HashSet::new(),
                        hovered_node: None,
                        panels: PanelVisibility::default(),
                        active_arrows: Vec::new(),
                        channel_subscribers,
                        last_sender: vec![None; num_channels],
                        time_accumulator: 0.0,
                        arrows_frozen: false,
                        run_until: None,
                        event_cursor: None,
                        event_stepping: false,
                        breakpoints: Vec::new(),
                        expanded_messages: HashSet::new(),
                        view_mode: ViewMode::default(),
                        bp_input: BreakpointInput::default(),
                        seq_zoom: 1.0,
                    }));
                }
                Err(e) => {
                    eprintln!("Failed to open trace: {e}");
                }
            }
        }
    }
}

/// Build receiver highlight map from expanded TX messages.
/// Returns node_name -> color (green for received, red for dropped).
fn build_receiver_highlights(
    messages: &[MessageEntry],
    expanded_messages: &HashSet<usize>,
) -> HashMap<String, egui::Color32> {
    let mut highlights = HashMap::new();
    for &idx in expanded_messages {
        if let Some(msg) = messages.get(idx) {
            for recv in &msg.receivers {
                let color = match &recv.outcome {
                    ReceiverOutcome::Received => egui::Color32::from_rgb(100, 200, 100),
                    ReceiverOutcome::Dropped(_) => egui::Color32::from_rgb(255, 100, 100),
                };
                highlights.insert(recv.node.clone(), color);
            }
        }
    }
    highlights
}

/// Build NodeState vec from the simulation AST.
pub fn nodes_from_sim(sim: &config::ast::Simulation) -> Vec<NodeState> {
    let mut nodes: Vec<_> = sim
        .nodes
        .iter()
        .map(|(name, node)| {
            let max_nj = node.energy.charge.as_ref().map(|c| c.unit.to_nj(c.max));
            NodeState {
                name: name.clone(),
                x: node.position.point.x,
                y: node.position.point.y,
                z: node.position.point.z,
                charge_ratio: node.energy.charge.as_ref().map(|c| {
                    if c.max == 0 {
                        1.0
                    } else {
                        c.quantity as f32 / c.max as f32
                    }
                }),
                max_nj,
                is_dead: false,
                prev_x: node.position.point.x,
                prev_y: node.position.point.y,
                prev_z: node.position.point.z,
                last_move_ts: 0,
                motion_spec: "none".to_string(),
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
}

/// Get the distance unit from the first node, falling back to Kilometers.
fn sim_distance_unit(sim: &config::ast::Simulation) -> DistanceUnit {
    sim.nodes
        .values()
        .next()
        .map(|n| n.position.unit)
        .unwrap_or(DistanceUnit::Kilometers)
}

/// Build a channel_index -> Vec<subscriber node_index> lookup from the simulation config.
fn build_channel_subscribers(sim: &config::ast::Simulation) -> Vec<Vec<usize>> {
    let mut ch_names: Vec<_> = sim.channels.keys().cloned().collect();
    ch_names.sort();
    let mut node_names: Vec<_> = sim.nodes.keys().cloned().collect();
    node_names.sort();

    let mut subs = vec![Vec::new(); ch_names.len()];
    for (node_idx, node_name) in node_names.iter().enumerate() {
        if let Some(node) = sim.nodes.get(node_name) {
            for proto in node.protocols.values() {
                for ch in &proto.subscribers {
                    if let Some(ch_idx) = ch_names.iter().position(|c| c == ch)
                        && !subs[ch_idx].contains(&node_idx)
                    {
                        subs[ch_idx].push(node_idx);
                    }
                }
            }
        }
    }
    subs
}

#[allow(clippy::too_many_arguments)]
fn process_gui_event(
    event: GuiEvent,
    current_timestep: &mut u64,
    message_list: &mut Vec<MessageEntry>,
    node_states: &mut [NodeState],
    sim: &config::ast::Simulation,
    active_arrows: &mut Vec<ArrowAnimation>,
    channel_subscribers: &[Vec<usize>],
    last_sender: &mut [Option<usize>],
    egui_time: f64,
) {
    match event {
        GuiEvent::Trace(record) => {
            *current_timestep = record.timestep;
            match &record.event {
                TraceEvent::MessageSent {
                    src_node,
                    channel,
                    data,
                } => {
                    let src_idx = *src_node as usize;
                    let ch_idx = *channel as usize;
                    let src_name = node_name_by_index(sim, src_idx);
                    let ch_name = channel_name_by_index(sim, ch_idx);
                    message_list.push(MessageEntry {
                        timestep: record.timestep,
                        kind: MessageKind::Sent,
                        src_node: src_name,
                        dst_node: None,
                        channel: ch_name,
                        data_preview: format_data_preview(data),
                        data_raw: data.clone(),
                        receivers: Vec::new(),
                        record_index: None,
                    });
                    // Track last sender for this channel
                    if let Some(slot) = last_sender.get_mut(ch_idx) {
                        *slot = Some(src_idx);
                    }
                    // Create TX arrows to all subscribers
                    if let Some(subs) = channel_subscribers.get(ch_idx) {
                        for &dst_idx in subs {
                            if dst_idx != src_idx {
                                active_arrows.push(ArrowAnimation {
                                    src_node: src_idx,
                                    dst_node: dst_idx,
                                    kind: ArrowKind::Sent,
                                    start_time: egui_time,
                                    duration: 0.25,
                                });
                            }
                        }
                    }
                }
                TraceEvent::MessageRecv {
                    dst_node,
                    channel,
                    data,
                    bit_errors,
                } => {
                    let dst_idx = *dst_node as usize;
                    let ch_idx = *channel as usize;
                    let dst_name = node_name_by_index(sim, dst_idx);
                    let ch_name = channel_name_by_index(sim, ch_idx);
                    // Look up who sent on this channel
                    let sender_name = last_sender
                        .get(ch_idx)
                        .and_then(|s| s.as_ref())
                        .map(|&idx| node_name_by_index(sim, idx));
                    let dst_name_clone = dst_name.clone();
                    let ch_name_clone = ch_name.clone();
                    message_list.push(MessageEntry {
                        timestep: record.timestep,
                        kind: MessageKind::Received,
                        src_node: dst_name,
                        dst_node: sender_name,
                        channel: ch_name,
                        data_preview: format_data_preview(data),
                        data_raw: data.clone(),
                        receivers: Vec::new(),
                        record_index: None,
                    });
                    // Correlate: attach this RX to matching TX entry
                    if let Some(tx_entry) = message_list.iter_mut().rev().find(|m| {
                        m.kind == MessageKind::Sent
                            && m.timestep == record.timestep
                            && m.channel == ch_name_clone
                    }) {
                        tx_entry.receivers.push(ReceiverInfo {
                            node: dst_name_clone,
                            outcome: ReceiverOutcome::Received,
                            has_bit_errors: *bit_errors,
                        });
                    }
                    // Create RX arrow from last known sender
                    if let Some(Some(src_idx)) = last_sender.get(ch_idx) {
                        active_arrows.push(ArrowAnimation {
                            src_node: *src_idx,
                            dst_node: dst_idx,
                            kind: ArrowKind::Received,
                            start_time: egui_time,
                            duration: 0.25,
                        });
                    }
                }
                TraceEvent::MessageDropped {
                    src_node,
                    channel,
                    reason,
                } => {
                    let src_idx = *src_node as usize;
                    let ch_idx = *channel as usize;
                    let src_name = node_name_by_index(sim, src_idx);
                    let ch_name = channel_name_by_index(sim, ch_idx);
                    let sender_name = last_sender
                        .get(ch_idx)
                        .and_then(|s| s.as_ref())
                        .map(|&idx| node_name_by_index(sim, idx));
                    let src_name_clone = src_name.clone();
                    let ch_name_clone = ch_name.clone();
                    let reason_str = format!("{reason:?}");
                    message_list.push(MessageEntry {
                        timestep: record.timestep,
                        kind: MessageKind::Dropped(reason_str.clone()),
                        src_node: src_name,
                        dst_node: sender_name,
                        channel: ch_name,
                        data_preview: String::new(),
                        data_raw: Vec::new(),
                        receivers: Vec::new(),
                        record_index: None,
                    });
                    // Correlate: attach this Drop to matching TX entry
                    if let Some(tx_entry) = message_list.iter_mut().rev().find(|m| {
                        m.kind == MessageKind::Sent
                            && m.timestep == record.timestep
                            && m.channel == ch_name_clone
                    }) {
                        tx_entry.receivers.push(ReceiverInfo {
                            node: src_name_clone,
                            outcome: ReceiverOutcome::Dropped(reason_str),
                            has_bit_errors: false,
                        });
                    }
                    // Create drop arrows to all subscribers
                    if let Some(subs) = channel_subscribers.get(ch_idx) {
                        for &dst_idx in subs {
                            if dst_idx != src_idx {
                                active_arrows.push(ArrowAnimation {
                                    src_node: src_idx,
                                    dst_node: dst_idx,
                                    kind: ArrowKind::Dropped,
                                    start_time: egui_time,
                                    duration: 0.25,
                                });
                            }
                        }
                    }
                }
                TraceEvent::PositionUpdate { node, x, y, z } => {
                    if let Some(state) = node_states.get_mut(*node as usize) {
                        state.prev_x = state.x;
                        state.prev_y = state.y;
                        state.prev_z = state.z;
                        state.x = *x;
                        state.y = *y;
                        state.z = *z;
                        state.last_move_ts = *current_timestep;
                    }
                }
                TraceEvent::EnergyUpdate { node, energy_nj } => {
                    if let Some(state) = node_states.get_mut(*node as usize)
                        && let Some(max) = state.max_nj
                    {
                        let ratio = if max == 0 {
                            1.0
                        } else {
                            *energy_nj as f32 / max as f32
                        };
                        state.charge_ratio = Some(ratio.clamp(0.0, 1.0));
                        state.is_dead = *energy_nj == 0 && max > 0;
                    }
                }
                TraceEvent::MotionUpdate { node, spec } => {
                    if let Some(state) = node_states.get_mut(*node as usize) {
                        state.motion_spec = spec.clone();
                    }
                }
            }
        }
        GuiEvent::TimestepAdvanced(ts) => {
            *current_timestep = ts;
        }
        GuiEvent::SimulationComplete | GuiEvent::SimulationError(_) => {}
    }
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

fn format_data_preview(data: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(data)
        && s.chars().all(|c| !c.is_control() || c == '\n')
    {
        return if s.len() <= 64 {
            s.to_string()
        } else {
            let boundary = s.floor_char_boundary(64);
            format!("{}... ({} bytes)", &s[..boundary], data.len())
        };
    }
    // Fallback to hex
    if data.len() <= 32 {
        hex::encode(data)
    } else {
        format!("{}... ({} bytes)", hex::encode(&data[..32]), data.len())
    }
}

fn create_sim_from_trace_header(
    controller: &crate::sim::replay::ReplayController,
) -> config::ast::Simulation {
    use config::ast::*;
    use std::collections::HashMap;
    use std::num::NonZeroU64;
    use std::time::SystemTime;

    let mut nodes = HashMap::new();
    for (i, name) in controller.node_names().iter().enumerate() {
        nodes.insert(
            name.clone(),
            Node {
                position: Position {
                    point: Point {
                        x: (i as f64) * 10.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    ..Default::default()
                },
                energy: config::ast::EnergyConfig {
                    charge: controller.node_max_nj().get(i).and_then(|opt| {
                        opt.map(|max_nj| Charge {
                            max: max_nj,
                            quantity: max_nj,
                            unit: EnergyUnit::NanoJoule,
                        })
                    }),
                    ..Default::default()
                },
                protocols: HashMap::new(),
                internal_names: Vec::new(),
                resources: Resources::default(),
                start: SystemTime::now(),
            },
        );
    }

    Simulation {
        params: Params {
            timestep: TimestepConfig {
                length: NonZeroU64::new(1).unwrap(),
                unit: TimeUnit::Milliseconds,
                count: NonZeroU64::new(controller.total_timesteps.max(1)).unwrap(),
                start: SystemTime::now(),
            },
            seed: 0,
            root: std::env::temp_dir(),
            time_dilation: 1.0,
        },
        channels: HashMap::new(),
        nodes,
    }
}

fn gather_messages_at(
    controller: &crate::sim::replay::ReplayController,
    ts: u64,
    sim: &config::ast::Simulation,
    messages: &mut Vec<MessageEntry>,
) {
    // Find the starting flat index for this timestep
    let base_idx = controller.first_record_index_at(ts);
    let records = controller.records_at(ts);
    for (i, record) in records.iter().enumerate() {
        let flat_idx = base_idx.map(|b| b + i);
        if let Some(entry) = trace_record_to_message(record, sim, flat_idx) {
            messages.push(entry);
        }
    }
}

fn gather_messages_through(
    controller: &crate::sim::replay::ReplayController,
    ts: u64,
    sim: &config::ast::Simulation,
    messages: &mut Vec<MessageEntry>,
) {
    for (i, record) in controller.records_through(ts).iter().enumerate() {
        if let Some(entry) = trace_record_to_message(record, sim, Some(i)) {
            messages.push(entry);
        }
    }
}

/// Correlate TX messages with their corresponding RX/Drop events in replay mode.
/// For each Sent message, scans the records at that timestep for matching Recv/Dropped events.
/// Also populates `dst_node` on RX/Drop entries with the sender's name from the matching TX.
fn correlate_all_tx_receivers(
    controller: &crate::sim::replay::ReplayController,
    sim: &config::ast::Simulation,
    messages: &mut [MessageEntry],
) {
    // First pass: build a map of (timestep, channel) -> sender name from TX entries
    let mut tx_senders: Vec<(u64, String, String)> = Vec::new(); // (ts, channel, sender)
    for msg in messages.iter() {
        if msg.kind == MessageKind::Sent {
            tx_senders.push((msg.timestep, msg.channel.clone(), msg.src_node.clone()));
        }
    }

    for msg in messages.iter_mut() {
        match &msg.kind {
            MessageKind::Sent => {
                // Populate receivers on TX entries
                let ts = msg.timestep;
                let ch = &msg.channel;
                let mut receivers = Vec::new();
                for record in controller.records_at(ts) {
                    match &record.event {
                        TraceEvent::MessageRecv {
                            dst_node, channel, bit_errors, ..
                        } => {
                            let ch_name = channel_name_by_index(sim, *channel as usize);
                            if &ch_name == ch {
                                let node_name =
                                    node_name_by_index(sim, *dst_node as usize);
                                receivers.push(ReceiverInfo {
                                    node: node_name,
                                    outcome: ReceiverOutcome::Received,
                                    has_bit_errors: *bit_errors,
                                });
                            }
                        }
                        TraceEvent::MessageDropped {
                            src_node, channel, reason,
                        } => {
                            let ch_name = channel_name_by_index(sim, *channel as usize);
                            if &ch_name == ch {
                                let drop_node = node_name_by_index(sim, *src_node as usize);
                                receivers.push(ReceiverInfo {
                                    node: drop_node,
                                    outcome: ReceiverOutcome::Dropped(format!("{reason:?}")),
                                    has_bit_errors: false,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                msg.receivers = receivers;
            }
            MessageKind::Received | MessageKind::Dropped(_) => {
                // Populate dst_node (sender) on RX/Drop entries from matching TX
                if msg.dst_node.is_none() {
                    for (ts, ch, sender) in &tx_senders {
                        if *ts == msg.timestep && ch == &msg.channel {
                            msg.dst_node = Some(sender.clone());
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Create arrow animations from trace records at the given timestep.
fn gather_arrows_at(
    controller: &crate::sim::replay::ReplayController,
    ts: u64,
    active_arrows: &mut Vec<ArrowAnimation>,
    channel_subscribers: &[Vec<usize>],
    last_sender: &mut [Option<usize>],
    egui_time: f64,
) {
    for record in controller.records_at(ts) {
        match &record.event {
            TraceEvent::MessageSent {
                src_node, channel, ..
            } => {
                let src_idx = *src_node as usize;
                let ch_idx = *channel as usize;
                if let Some(slot) = last_sender.get_mut(ch_idx) {
                    *slot = Some(src_idx);
                }
                if let Some(subs) = channel_subscribers.get(ch_idx) {
                    for &dst_idx in subs {
                        if dst_idx != src_idx {
                            active_arrows.push(ArrowAnimation {
                                src_node: src_idx,
                                dst_node: dst_idx,
                                kind: ArrowKind::Sent,
                                start_time: egui_time,
                                duration: 0.25,
                            });
                        }
                    }
                }
            }
            TraceEvent::MessageRecv {
                dst_node, channel, ..
            } => {
                let dst_idx = *dst_node as usize;
                let ch_idx = *channel as usize;
                if let Some(Some(src_idx)) = last_sender.get(ch_idx) {
                    active_arrows.push(ArrowAnimation {
                        src_node: *src_idx,
                        dst_node: dst_idx,
                        kind: ArrowKind::Received,
                        start_time: egui_time,
                        duration: 0.25,
                    });
                }
            }
            TraceEvent::MessageDropped {
                src_node, channel, ..
            } => {
                let src_idx = *src_node as usize;
                let ch_idx = *channel as usize;
                if let Some(subs) = channel_subscribers.get(ch_idx) {
                    for &dst_idx in subs {
                        if dst_idx != src_idx {
                            active_arrows.push(ArrowAnimation {
                                src_node: src_idx,
                                dst_node: dst_idx,
                                kind: ArrowKind::Dropped,
                                start_time: egui_time,
                                duration: 0.25,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn trace_record_to_message(
    record: &trace::format::TraceRecord,
    sim: &config::ast::Simulation,
    record_index: Option<usize>,
) -> Option<MessageEntry> {
    match &record.event {
        TraceEvent::MessageSent {
            src_node,
            channel,
            data,
        } => Some(MessageEntry {
            timestep: record.timestep,
            kind: MessageKind::Sent,
            src_node: node_name_by_index(sim, *src_node as usize),
            dst_node: None,
            channel: channel_name_by_index(sim, *channel as usize),
            data_preview: format_data_preview(data),
            data_raw: data.clone(),
            receivers: Vec::new(),
            record_index,
        }),
        TraceEvent::MessageRecv {
            dst_node,
            channel,
            data,
            ..
        } => Some(MessageEntry {
            timestep: record.timestep,
            kind: MessageKind::Received,
            src_node: node_name_by_index(sim, *dst_node as usize),
            dst_node: None,
            channel: channel_name_by_index(sim, *channel as usize),
            data_preview: format_data_preview(data),
            data_raw: data.clone(),
            receivers: Vec::new(),
            record_index,
        }),
        TraceEvent::MessageDropped {
            src_node,
            channel,
            reason,
        } => Some(MessageEntry {
            timestep: record.timestep,
            kind: MessageKind::Dropped(format!("{reason:?}")),
            src_node: node_name_by_index(sim, *src_node as usize),
            dst_node: None,
            channel: channel_name_by_index(sim, *channel as usize),
            data_preview: String::new(),
            data_raw: Vec::new(),
            receivers: Vec::new(),
            record_index,
        }),
        _ => None,
    }
}
