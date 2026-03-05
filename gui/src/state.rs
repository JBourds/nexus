use config::ast;

use crate::render::grid::GridView;
use crate::sim::controller::SimController;
use crate::sim::replay::ReplayController;

/// Top-level application mode.
#[derive(Default)]
pub enum AppMode {
    #[default]
    Home,
    ConfigEditor(ConfigEditorState),
    LiveSimulation(LiveSimState),
    Replay(ReplayState),
}

/// State for the configuration editor mode.
pub struct ConfigEditorState {
    pub sim: ast::Simulation,
    pub file_path: Option<std::path::PathBuf>,
    pub grid: GridView,
    pub selected_node: Option<String>,
    pub selected_channel: Option<String>,
    pub validation_error: Option<String>,
    pub dirty: bool,
    /// Shared buffer for inline "add item" text inputs (only one active at a time).
    pub add_item_buf: String,
}

/// State for a live simulation.
pub struct LiveSimState {
    pub sim: ast::Simulation,
    pub controller: SimController,
    pub grid: GridView,
    pub selected_node: Option<String>,
    pub current_timestep: u64,
    pub messages: Vec<MessageEntry>,
    pub node_states: Vec<NodeState>,
    /// Directory where trace.nxs lives, for post-sim replay.
    pub sim_dir: std::path::PathBuf,
}

/// State for replay mode.
pub struct ReplayState {
    pub sim: ast::Simulation,
    pub controller: ReplayController,
    pub grid: GridView,
    pub selected_node: Option<String>,
    pub current_timestep: u64,
    pub total_timesteps: u64,
    pub playing: bool,
    pub playback_speed: f32,
    pub messages: Vec<MessageEntry>,
    pub node_states: Vec<NodeState>,
}

/// Per-node runtime state for visualization.
#[derive(Clone, Debug)]
pub struct NodeState {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub charge_ratio: Option<f32>,
}

/// A message event for display in the message panel.
#[derive(Clone, Debug)]
pub struct MessageEntry {
    pub timestep: u64,
    pub kind: MessageKind,
    pub src_node: String,
    pub dst_node: Option<String>,
    pub channel: String,
    pub data_preview: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MessageKind {
    Sent,
    Received,
    Dropped(String),
}
