use std::collections::HashSet;

use eframe::App;
use egui::Context;

use crate::config_editor;
use crate::panels::{grid, inspector, messages, timeline, toolbar};
use crate::render::grid::GridView;
use crate::sim::bridge::GuiEvent;
use crate::state::*;
use trace::format::TraceEvent;

pub struct NexusApp {
    pub mode: AppMode,
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
            let action = toolbar::show_toolbar(ui, &self.mode, sim_finished, panels);
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
            let (clicked, _hovered) =
                grid::show_grid_panel(ui, &mut state.grid, &nodes, &state.selected_node);
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
        if !state.paused {
            for event in state.controller.poll_events() {
                process_gui_event(
                    event,
                    &mut state.current_timestep,
                    &mut state.messages,
                    &mut state.node_states,
                    &state.sim,
                );
            }
        }

        // Inspector panel (only rendered when visible — no panel at all when hidden)
        if state.panels.inspector {
            egui::SidePanel::left("inspector")
                .default_width(200.0)
                .resizable(true)
                .show(ctx, |ui| {
                    inspector::show_inspector(
                        ui,
                        &state.sim,
                        &state.node_states,
                        &state.selected_node,
                        &mut state.expanded_nodes,
                    );
                });
        }

        // Messages panel (only rendered when visible)
        if state.panels.messages {
            egui::SidePanel::right("messages")
                .default_width(250.0)
                .resizable(true)
                .show(ctx, |ui| {
                    messages::show_messages(ui, &state.messages, 200);
                });
        }

        // Timeline at bottom
        let total = state.sim.params.timestep.count.get();
        let finished = state.controller.is_finished();
        let mut view_replay = false;
        egui::TopBottomPanel::bottom("timeline").show(ctx, |ui| {
            let mut playing = !state.paused;
            let mut speed = 1.0;
            let action = timeline::show_timeline(
                ui,
                &mut state.current_timestep,
                total,
                &mut playing,
                &mut speed,
            );
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

        // Central grid
        egui::CentralPanel::default().show(ctx, |ui| {
            if state.needs_fit {
                state.grid.fit_to_nodes(&state.node_states, ui.available_size());
                state.needs_fit = false;
            }
            let (clicked, hovered) = grid::show_grid_panel(
                ui,
                &mut state.grid,
                &state.node_states,
                &state.selected_node,
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
            if let Some(ref name) = state.hovered_node {
                if let Some(n) = state.node_states.iter().find(|n| &n.name == name) {
                    egui::containers::popup::show_tooltip_at_pointer(
                        ui.ctx(),
                        egui::LayerId::new(egui::Order::Tooltip, ui.id().with("node_tip")),
                        ui.id().with("node_tip"),
                        |ui| {
                            ui.label(format!("{} ({:.1}, {:.1}, {:.1})", n.name, n.x, n.y, n.z));
                            if let Some(r) = n.charge_ratio {
                                ui.label(format!("Charge: {:.0}%", r * 100.0));
                            }
                        },
                    );
                }
            }
        });

        // Keep requesting repaints during live sim
        if !finished {
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
            self.mode = AppMode::Replay(ReplayState {
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
            });
        }
    }

    fn show_replay_mode(&mut self, ctx: &Context) {
        let AppMode::Replay(state) = &mut self.mode else {
            return;
        };

        // Advance timestep if playing — append messages (don't replace)
        if state.playing && state.current_timestep < state.total_timesteps.saturating_sub(1) {
            state.current_timestep += 1;
            state.node_states = state
                .controller
                .reconstruct_states(state.current_timestep, &state.initial_states);
            gather_messages_at(
                &state.controller,
                state.current_timestep,
                &state.sim,
                &mut state.messages,
            );
        }

        // Inspector panel (only rendered when visible)
        if state.panels.inspector {
            egui::SidePanel::left("inspector")
                .default_width(200.0)
                .resizable(true)
                .show(ctx, |ui| {
                    inspector::show_inspector(
                        ui,
                        &state.sim,
                        &state.node_states,
                        &state.selected_node,
                        &mut state.expanded_nodes,
                    );
                });
        }

        // Messages panel (only rendered when visible)
        if state.panels.messages {
            egui::SidePanel::right("messages")
                .default_width(250.0)
                .resizable(true)
                .show(ctx, |ui| {
                    messages::show_messages(ui, &state.messages, 200);
                });
        }

        // Timeline
        egui::TopBottomPanel::bottom("timeline").show(ctx, |ui| {
            let action = timeline::show_timeline(
                ui,
                &mut state.current_timestep,
                state.total_timesteps,
                &mut state.playing,
                &mut state.playback_speed,
            );

            if action.toggle_play {
                state.playing = !state.playing;
            }
            if let Some(ts) = action.seek_to {
                state.current_timestep = ts;
                state.playing = false;
                state.node_states = state.controller.reconstruct_states(ts, &state.initial_states);
                state.messages.clear();
                gather_messages_through(&state.controller, ts, &state.sim, &mut state.messages);
            }
            if action.step_forward {
                state.playing = false;
                if state.current_timestep < state.total_timesteps.saturating_sub(1) {
                    state.current_timestep += 1;
                    state.node_states = state
                        .controller
                        .reconstruct_states(state.current_timestep, &state.initial_states);
                    gather_messages_at(
                        &state.controller,
                        state.current_timestep,
                        &state.sim,
                        &mut state.messages,
                    );
                }
            }
            if action.step_backward {
                state.playing = false;
                state.current_timestep = state.current_timestep.saturating_sub(1);
                state.node_states = state
                    .controller
                    .reconstruct_states(state.current_timestep, &state.initial_states);
                state.messages.clear();
                gather_messages_through(
                    &state.controller,
                    state.current_timestep,
                    &state.sim,
                    &mut state.messages,
                );
            }
        });

        // Central grid
        egui::CentralPanel::default().show(ctx, |ui| {
            if state.needs_fit {
                state.grid.fit_to_nodes(&state.node_states, ui.available_size());
                state.needs_fit = false;
            }
            let (clicked, hovered) = grid::show_grid_panel(
                ui,
                &mut state.grid,
                &state.node_states,
                &state.selected_node,
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
            if let Some(ref name) = state.hovered_node {
                if let Some(n) = state.node_states.iter().find(|n| &n.name == name) {
                    egui::containers::popup::show_tooltip_at_pointer(
                        ui.ctx(),
                        egui::LayerId::new(egui::Order::Tooltip, ui.id().with("node_tip")),
                        ui.id().with("node_tip"),
                        |ui| {
                            ui.label(format!("{} ({:.1}, {:.1}, {:.1})", n.name, n.x, n.y, n.z));
                            if let Some(r) = n.charge_ratio {
                                ui.label(format!("Charge: {:.0}%", r * 100.0));
                            }
                        },
                    );
                }
            }
        });

        if state.playing {
            ctx.request_repaint();
        }
    }

    fn run_simulation(&mut self) {
        let AppMode::ConfigEditor(state) = &mut self.mode else {
            return;
        };

        match crate::sim::launch::launch_simulation(state.sim.clone(), None) {
            Ok((controller, sim_dir)) => {
                let node_states = nodes_from_sim(&state.sim);
                self.mode = AppMode::LiveSimulation(LiveSimState {
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
                });
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
            Ok((controller, sim_dir)) => {
                let node_states = nodes_from_sim(&sim);
                self.mode = AppMode::LiveSimulation(LiveSimState {
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
                });
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
            // Try raw config first, then snapshot format
            let sim = config::parse(path.clone()).or_else(|_| config::deserialize_config(&path));
            match sim {
                Ok(sim) => {
                    self.mode = AppMode::ConfigEditor(ConfigEditorState {
                        sim,
                        file_path: Some(path),
                        grid: GridView::default(),
                        selected_node: None,
                        selected_channel: None,
                        validation_error: None,
                        dirty: false,
                        add_item_buf: String::new(),
                        needs_fit: true,
                    });
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
        self.mode = AppMode::ConfigEditor(ConfigEditorState {
            sim,
            file_path: None,
            grid: GridView::default(),
            selected_node: None,
            selected_channel: None,
            validation_error: None,
            dirty: true,
            add_item_buf: String::new(),
            needs_fit: true,
        });
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

                    self.mode = AppMode::Replay(ReplayState {
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
                    });
                }
                Err(e) => {
                    eprintln!("Failed to open trace: {e}");
                }
            }
        }
    }
}

/// Build NodeState vec from the simulation AST.
pub fn nodes_from_sim(sim: &config::ast::Simulation) -> Vec<NodeState> {
    let mut nodes: Vec<_> = sim
        .nodes
        .iter()
        .map(|(name, node)| {
            let max_nj = node.charge.as_ref().map(|c| c.unit.to_nj(c.max));
            NodeState {
                name: name.clone(),
                x: node.position.point.x,
                y: node.position.point.y,
                z: node.position.point.z,
                charge_ratio: node.charge.as_ref().map(|c| {
                    if c.max == 0 {
                        1.0
                    } else {
                        c.quantity as f32 / c.max as f32
                    }
                }),
                max_nj,
                is_dead: false,
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    nodes
}

fn process_gui_event(
    event: GuiEvent,
    current_timestep: &mut u64,
    message_list: &mut Vec<MessageEntry>,
    node_states: &mut [NodeState],
    sim: &config::ast::Simulation,
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
                    let src_name = node_name_by_index(sim, *src_node as usize);
                    let ch_name = channel_name_by_index(sim, *channel as usize);
                    message_list.push(MessageEntry {
                        timestep: record.timestep,
                        kind: MessageKind::Sent,
                        src_node: src_name,
                        dst_node: None,
                        channel: ch_name,
                        data_preview: format_data_preview(data),
                        data_raw: data.clone(),
                    });
                }
                TraceEvent::MessageRecv {
                    dst_node,
                    channel,
                    data,
                } => {
                    let dst_name = node_name_by_index(sim, *dst_node as usize);
                    let ch_name = channel_name_by_index(sim, *channel as usize);
                    message_list.push(MessageEntry {
                        timestep: record.timestep,
                        kind: MessageKind::Received,
                        src_node: dst_name,
                        dst_node: None,
                        channel: ch_name,
                        data_preview: format_data_preview(data),
                        data_raw: data.clone(),
                    });
                }
                TraceEvent::MessageDropped {
                    src_node,
                    channel,
                    reason,
                } => {
                    let src_name = node_name_by_index(sim, *src_node as usize);
                    let ch_name = channel_name_by_index(sim, *channel as usize);
                    message_list.push(MessageEntry {
                        timestep: record.timestep,
                        kind: MessageKind::Dropped(format!("{reason:?}")),
                        src_node: src_name,
                        dst_node: None,
                        channel: ch_name,
                        data_preview: String::new(),
                        data_raw: Vec::new(),
                    });
                }
                TraceEvent::PositionUpdate { node, x, y, z } => {
                    if let Some(state) = node_states.get_mut(*node as usize) {
                        state.x = *x;
                        state.y = *y;
                        state.z = *z;
                    }
                }
                TraceEvent::EnergyUpdate { node, energy_nj } => {
                    if let Some(state) = node_states.get_mut(*node as usize) {
                        if let Some(max) = state.max_nj {
                            let ratio = if max == 0 { 1.0 } else { *energy_nj as f32 / max as f32 };
                            state.charge_ratio = Some(ratio.clamp(0.0, 1.0));
                            state.is_dead = *energy_nj == 0 && max > 0;
                        }
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
    if let Ok(s) = std::str::from_utf8(data) {
        if s.chars().all(|c| !c.is_control() || c == '\n') {
            return if s.len() <= 64 {
                s.to_string()
            } else {
                format!("{}... ({} bytes)", &s[..64], data.len())
            };
        }
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
                charge: controller.node_max_nj().get(i).and_then(|opt| {
                    opt.map(|max_nj| Charge {
                        max: max_nj,
                        quantity: max_nj,
                        unit: EnergyUnit::NanoJoule,
                    })
                }),
                protocols: HashMap::new(),
                internal_names: Vec::new(),
                resources: Resources::default(),
                power_states: HashMap::new(),
                power_sources: HashMap::new(),
                power_sinks: HashMap::new(),
                channel_energy: HashMap::new(),
                initial_state: None,
                restart_threshold: None,
                start: SystemTime::now(),
            },
        );
    }

    Simulation {
        params: Params {
            timestep: TimestepConfig {
                length: NonZeroU64::new(1).unwrap(),
                unit: TimeUnit::Milliseconds,
                count: NonZeroU64::new(controller.total_timesteps).unwrap(),
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
    for record in controller.records_at(ts) {
        if let Some(entry) = trace_record_to_message(record, sim) {
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
    for record in controller.records_through(ts) {
        if let Some(entry) = trace_record_to_message(record, sim) {
            messages.push(entry);
        }
    }
}

fn trace_record_to_message(
    record: &trace::format::TraceRecord,
    sim: &config::ast::Simulation,
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
        }),
        TraceEvent::MessageRecv {
            dst_node,
            channel,
            data,
        } => Some(MessageEntry {
            timestep: record.timestep,
            kind: MessageKind::Received,
            src_node: node_name_by_index(sim, *dst_node as usize),
            dst_node: None,
            channel: channel_name_by_index(sim, *channel as usize),
            data_preview: format_data_preview(data),
            data_raw: data.clone(),
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
        }),
        _ => None,
    }
}
