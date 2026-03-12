use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use config::ast;
use config::parse::NodeProfile;

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

/// A profile resolved from an imported module.
pub struct ResolvedProfile {
    pub source_module: String,
    pub profile: NodeProfile,
}

/// What a module provides (for the browser display).
pub struct ModuleProvides {
    pub links: Vec<String>,
    pub channels: Vec<String>,
    pub profiles: Vec<String>,
}

/// A stdlib module entry for the browser.
pub struct StdlibEntry {
    pub spec: String,
    pub description: String,
    pub provides: ModuleProvides,
}

/// Module-related state tracked alongside the AST.
pub struct ModuleState {
    /// The `use = [...]` list from the original config.
    pub use_list: Vec<String>,
    /// Per-node profile assignments: node_name -> Vec<profile_name>.
    pub node_profiles: HashMap<String, Vec<String>>,
    /// Profiles available from imported modules, keyed by profile name.
    pub available_profiles: HashMap<String, ResolvedProfile>,
    /// Cached catalog of stdlib modules for the browser.
    pub stdlib_catalog: Vec<StdlibEntry>,
    /// Whether the module browser window is open.
    pub browser_open: bool,
    /// Currently selected module in the browser.
    pub browser_selected: Option<String>,
    /// Search query for fuzzy-filtering in the browser.
    pub browser_search: String,
    /// Set of expanded categories in the browser tree.
    pub browser_expanded: HashSet<String>,
}

impl Default for ModuleState {
    fn default() -> Self {
        Self {
            use_list: Vec::new(),
            node_profiles: HashMap::new(),
            available_profiles: HashMap::new(),
            stdlib_catalog: build_stdlib_catalog(),
            browser_open: false,
            browser_selected: None,
            browser_search: String::new(),
            browser_expanded: HashSet::new(),
        }
    }
}

impl ModuleState {
    /// Resolve all modules in `use_list` and populate `available_profiles`.
    pub fn resolve_profiles(&mut self, config_dir: Option<&Path>) {
        self.available_profiles.clear();
        for spec in &self.use_list {
            let path = match config::module::resolve_module_path(spec, config_dir) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let module = match config::parse_module_file(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if let Some(profiles) = module.profiles {
                for (name, profile) in profiles {
                    self.available_profiles
                        .entry(name.to_ascii_lowercase())
                        .or_insert(ResolvedProfile {
                            source_module: spec.clone(),
                            profile,
                        });
                }
            }
        }
    }
}

/// Build the stdlib catalog by walking the stdlib directory.
fn build_stdlib_catalog() -> Vec<StdlibEntry> {
    let stdlib = config::module::stdlib_path();
    if !stdlib.is_dir() {
        return Vec::new();
    }
    let mut entries = Vec::new();
    walk_stdlib(stdlib, "", &mut entries);
    entries.sort_by(|a, b| a.spec.cmp(&b.spec));
    entries
}

fn walk_stdlib(dir: &Path, prefix: &str, entries: &mut Vec<StdlibEntry>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dir_entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    dir_entries.sort_by_key(|e| e.file_name());

    for entry in dir_entries {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if ft.is_dir() {
            let sub_prefix = if prefix.is_empty() {
                name_str.to_string()
            } else {
                format!("{prefix}/{name_str}")
            };
            walk_stdlib(&entry.path(), &sub_prefix, entries);
        } else if ft.is_file() && name_str.ends_with(".toml") {
            let stem = name_str.trim_end_matches(".toml");
            let spec = if prefix.is_empty() {
                stem.to_string()
            } else {
                format!("{prefix}/{stem}")
            };

            let (description, provides) = match std::fs::read_to_string(entry.path()) {
                Ok(text) => {
                    let desc = text
                        .lines()
                        .find(|l| l.starts_with("# "))
                        .map(|l| l.trim_start_matches("# ").to_string())
                        .unwrap_or_default();
                    let provides = match toml::from_str::<config::parse::ModuleFile>(&text) {
                        Ok(m) => ModuleProvides {
                            links: m.links.keys().cloned().collect(),
                            channels: m.channels.keys().cloned().collect(),
                            profiles: m
                                .profiles
                                .as_ref()
                                .map(|p| p.keys().cloned().collect())
                                .unwrap_or_default(),
                        },
                        Err(_) => ModuleProvides {
                            links: Vec::new(),
                            channels: Vec::new(),
                            profiles: Vec::new(),
                        },
                    };
                    (desc, provides)
                }
                Err(_) => (
                    String::new(),
                    ModuleProvides {
                        links: Vec::new(),
                        channels: Vec::new(),
                        profiles: Vec::new(),
                    },
                ),
            };

            entries.push(StdlibEntry {
                spec,
                description,
                provides,
            });
        }
    }
}

/// State for the configuration editor mode.
pub struct ConfigEditorState {
    pub sim: ast::Simulation,
    pub file_path: Option<std::path::PathBuf>,
    pub grid: GridView,
    pub selected_node: Option<String>,
    #[allow(dead_code)]
    pub selected_channel: Option<String>,
    pub validation_error: Option<String>,
    pub dirty: bool,
    /// Shared buffer for inline "add item" text inputs (only one active at a time).
    pub add_item_buf: String,
    /// When true, auto-fit the grid viewport on next frame.
    pub needs_fit: bool,
    /// Module system state (use list, profiles, browser).
    pub modules: ModuleState,
}

impl ConfigEditorState {
    pub fn new(path: PathBuf) -> Result<Self> {
        // Extract module info from raw TOML before config::parse() consumes it.
        let raw_text = std::fs::read_to_string(&path).unwrap_or_default();
        let (use_list, node_profiles) = config::extract_module_info(&raw_text);

        let sim = config::parse(path.clone())
            .or_else(|_| config::deserialize_config(&path))
            .with_context(|| format!("Failed to parse config at path: {path:#?}"))?;

        let config_dir = path.parent().map(Path::to_path_buf);
        let mut modules = ModuleState {
            use_list,
            node_profiles,
            ..Default::default()
        };
        modules.resolve_profiles(config_dir.as_deref());

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
            modules,
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
    /// When true, arrow animations are frozen (not expired) due to a run-until trigger.
    pub arrows_frozen: bool,
    /// One-shot run-until condition. Cleared after triggering.
    pub run_until: Option<BreakpointKind>,
    /// Event-level stepping: index into accumulated events. None = timestep mode.
    pub event_cursor: Option<usize>,
    /// Whether event-stepping mode is active.
    pub event_stepping: bool,
    /// Breakpoints that pause when matched.
    pub breakpoints: Vec<Breakpoint>,
    /// Set of expanded TX message indices for receiver expansion.
    pub expanded_messages: HashSet<usize>,
    /// All trace records accumulated during live simulation (for event stepping).
    pub all_records: Vec<trace::format::TraceRecord>,
    /// View mode: Grid or Sequence diagram.
    pub view_mode: ViewMode,
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
    /// channel_index -> Vec<node_index> for drawing arrows to subscribers.
    pub channel_subscribers: Vec<Vec<usize>>,
    /// channel_index -> last sender node_index (for linking RX arrows back to TX).
    pub last_sender: Vec<Option<usize>>,
    /// Fractional timestep accumulator for real-time replay.
    pub time_accumulator: f64,
    /// When true, arrow animations are frozen (not expired) due to a run-until trigger.
    pub arrows_frozen: bool,
    /// One-shot run-until condition. Cleared after triggering.
    pub run_until: Option<BreakpointKind>,
    /// Event-level stepping: index into the flat record array. None = timestep mode.
    pub event_cursor: Option<usize>,
    /// Whether event-stepping mode is active (vs timestep mode).
    pub event_stepping: bool,
    /// Breakpoints that pause playback when matched.
    pub breakpoints: Vec<Breakpoint>,
    /// Set of expanded TX message indices (for receiver expansion in messages panel).
    pub expanded_messages: HashSet<usize>,
    /// View mode: Grid or Sequence diagram.
    pub view_mode: ViewMode,
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
    /// For TX messages: correlated receivers (populated by correlate_tx_receivers).
    pub receivers: Vec<ReceiverInfo>,
    /// Index of this entry's corresponding record in the flat record array (for event cursor sync).
    pub record_index: Option<usize>,
}

/// Information about a receiver of a TX message.
#[derive(Clone, Debug)]
pub struct ReceiverInfo {
    pub node: String,
    pub outcome: ReceiverOutcome,
}

/// Whether a node received or dropped a message.
#[derive(Clone, Debug)]
pub enum ReceiverOutcome {
    Received,
    Dropped(String),
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

/// A breakpoint that pauses playback when its condition is met.
#[derive(Clone, Debug)]
pub struct Breakpoint {
    pub kind: BreakpointKind,
    pub enabled: bool,
}

/// What triggers a breakpoint.
#[derive(Clone, Debug, PartialEq)]
pub enum BreakpointKind {
    /// Stop at a specific timestep.
    Timestep(u64),
    /// Stop on any event involving this node.
    NodeEvent(String),
    /// Stop on any TX/RX on this channel.
    ChannelActivity(String),
}

/// Which main view is shown in the central panel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    Grid,
    Sequence,
}
