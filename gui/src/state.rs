use std::collections::HashSet;

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
    /// When true, auto-fit the grid viewport on next frame.
    pub needs_fit: bool,
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
    /// Whether the live display is paused (events still buffer, just not processed).
    pub paused: bool,
    /// When true, auto-fit the grid viewport on next frame.
    pub needs_fit: bool,
    /// Set of expanded node names in the inspector.
    pub expanded_nodes: HashSet<String>,
    /// Currently hovered node name (from grid panel).
    pub hovered_node: Option<String>,
    /// Panel visibility.
    pub panels: PanelVisibility,
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
    /// Cached initial node states (from sim AST) to avoid recomputing each frame.
    pub initial_states: Vec<NodeState>,
    /// When true, auto-fit the grid viewport on next frame.
    pub needs_fit: bool,
    /// Set of expanded node names in the inspector.
    pub expanded_nodes: HashSet<String>,
    /// Currently hovered node name (from grid panel).
    pub hovered_node: Option<String>,
    /// Panel visibility.
    pub panels: PanelVisibility,
}

/// Per-node runtime state for visualization.
#[derive(Clone, Debug)]
pub struct NodeState {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub charge_ratio: Option<f32>,
    pub max_nj: Option<u64>,
    pub is_dead: bool,
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
    /// Raw message bytes for clipboard copy.
    pub data_raw: Vec<u8>,
}

/// Which panels are visible (for collapsible panes).
pub struct PanelVisibility {
    pub inspector: bool,
    pub messages: bool,
}

impl Default for PanelVisibility {
    fn default() -> Self {
        Self {
            inspector: true,
            messages: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MessageKind {
    Sent,
    Received,
    Dropped(String),
}
