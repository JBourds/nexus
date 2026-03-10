use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use config::ast;

use crate::render::grid::GridView;
use crate::sim::controller::SimController;
use crate::sim::replay::ReplayController;

/// Top-level application mode.
#[derive(Default)]
pub enum AppMode {
    #[default]
    Home,
    ConfigEditor(Box<ConfigEditorState>),
    LiveSimulation(Box<LiveSimState>),
    Replay(Box<ReplayState>),
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

impl ConfigEditorState {
    pub fn new(path: PathBuf) -> Result<Self> {
        let sim = config::parse(path.clone())
            .or_else(|_| config::deserialize_config(&path))
            .with_context(|| format!("Failed to parse config at path: {path:#?}"))?;
        Ok(Self {
            sim,
            file_path: Some(path),
            grid: GridView::default(),
            selected_node: None,
            selected_channel: None,
            validation_error: None,
            dirty: false,
            add_item_buf: String::new(),
            needs_fit: true,
        })
    }
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
    /// Active arrow animations on the grid.
    pub active_arrows: Vec<ArrowAnimation>,
    /// channel_index → Vec<node_index> for drawing arrows to subscribers.
    pub channel_subscribers: Vec<Vec<usize>>,
    /// channel_index → last sender node_index (for linking RX arrows back to TX).
    pub last_sender: Vec<Option<usize>>,
    /// Shared time dilation value (f64 bits in AtomicU64) for live kernel adjustment.
    pub time_dilation: Arc<AtomicU64>,
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
    /// Active arrow animations on the grid.
    pub active_arrows: Vec<ArrowAnimation>,
    /// channel_index → Vec<node_index> for drawing arrows to subscribers.
    pub channel_subscribers: Vec<Vec<usize>>,
    /// channel_index → last sender node_index (for linking RX arrows back to TX).
    pub last_sender: Vec<Option<usize>>,
    /// Fractional timestep accumulator for real-time replay.
    pub time_accumulator: f64,
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
    /// Previous position for velocity computation.
    pub prev_x: f64,
    pub prev_y: f64,
    pub prev_z: f64,
    /// Timestep of last position update (0 = never moved).
    pub last_move_ts: u64,
    /// Current motion pattern spec string (e.g. "none", "velocity 0.1 0 0").
    pub motion_spec: String,
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

/// An in-flight message arrow animation on the grid.
#[derive(Clone, Debug)]
pub struct ArrowAnimation {
    pub src_node: usize,
    pub dst_node: usize,
    pub kind: ArrowKind,
    pub start_time: f64,
    pub duration: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArrowKind {
    Sent,
    Received,
    Dropped,
}
