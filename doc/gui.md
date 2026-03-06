# GUI Crate

## Table of Contents

1. [Overview](#overview)
2. [Module Structure](#module-structure)
3. [Application State Machine](#application-state-machine)
   - [AppMode](#appmode)
   - [Mode Transitions](#mode-transitions)
4. [State Structs](#state-structs)
   - [ConfigEditorState](#configeditorstate)
   - [LiveSimState](#livesimstate)
   - [ReplayState](#replaystate)
   - [NodeState](#nodestate)
   - [MessageEntry and MessageKind](#messageentry-and-messagekind)
   - [PanelVisibility](#panelvisibility)
5. [Rendering Pipeline](#rendering-pipeline)
   - [Frame Ordering](#frame-ordering)
   - [Toolbar Actions](#toolbar-actions)
6. [Grid and Node Rendering](#grid-and-node-rendering)
   - [GridView](#gridview)
   - [draw\_node](#draw_node)
   - [show\_grid\_panel and Hit Testing](#show_grid_panel-and-hit-testing)
7. [Inspector Panel](#inspector-panel)
8. [Panel Visibility](#panel-visibility)
9. [Config Editor](#config-editor)
10. [Simulation Control](#simulation-control)
    - [SimController](#simcontroller)
    - [launch\_simulation](#launch_simulation)
    - [Pause and Stop](#pause-and-stop)
11. [Replay System](#replay-system)
    - [ReplayController](#replaycontroller)
    - [Timestep Index](#timestep-index)
    - [State Reconstruction](#state-reconstruction)
    - [Message Gathering](#message-gathering)
12. [Trace Bridge](#trace-bridge)
    - [SimSinks](#simsinks)
    - [ReloadableSimLayer](#reloadablesimlayer)
    - [GuiEvent](#guievent)
    - [Event Flow](#event-flow)
13. [Messages Panel](#messages-panel)
14. [Timeline Panel](#timeline-panel)
15. [Key Design Decisions](#key-design-decisions)

---

## Overview

The `gui` crate is the graphical frontend for Nexus. It provides a native
desktop application (built with [eframe](https://github.com/emilk/egui/tree/master/crates/eframe)
and [egui](https://github.com/emilk/egui)) that gives users three entry points
into the simulator:

1. **Config Editor** — a form-based editor for the simulation TOML config, with
   a live node grid showing node positions.
2. **Live Simulation** — launches the kernel on a background thread, streams
   trace events in real time, and displays node state and messages as they occur.
3. **Replay** — opens a `.nxs` trace file and allows scrubbing, stepping, and
   playback of a recorded simulation.

The GUI never modifies the kernel's internal state. Everything the GUI knows
about a running simulation comes through a single `crossbeam_channel` of
`GuiEvent` values sent by the trace bridge.

---

## Module Structure

```
gui/src/
├── main.rs                   Entry point; constructs NexusApp and hands it to eframe
├── app.rs                    NexusApp: App impl, per-mode rendering, event processing
├── state.rs                  AppMode enum and all state structs
├── config_editor/
│   ├── mod.rs                show_config_editor: the side panel with all config sections
│   ├── channels.rs           Channel type editor (Shared / Exclusive, link inline)
│   ├── links.rs              Link editor (medium, delays, error models)
│   ├── nodes.rs              Node editor (position, charge, resources, protocols)
│   ├── params.rs             Simulation params editor (timestep, seed, dilation, root)
│   └── widgets.rs            Shared widgets: add_item_ui, enum_combo, remove_button, …
├── panels/
│   ├── mod.rs                Re-exports
│   ├── grid.rs               show_grid_panel: canvas with hit testing
│   ├── inspector.rs          show_inspector: collapsible node list
│   ├── messages.rs           show_messages: message event log
│   ├── timeline.rs           show_timeline: scrubber and playback controls
│   └── toolbar.rs            show_toolbar: mode-aware top bar
└── render/
    ├── mod.rs                Re-exports
    ├── grid.rs               GridView: pan/zoom/coordinate transform/grid drawing
    ├── node.rs               draw_node: circle + label + selection ring
    └── message.rs            draw_message_arc: animated message arrows (TX/RX/Drop)
```

---

## Application State Machine

### AppMode

`AppMode` is the single top-level discriminant. `NexusApp` holds exactly one
`AppMode` at a time. The enum variants carry their entire mode-specific state
as inline data:

```rust
pub enum AppMode {
    Home,
    ConfigEditor(Box<ConfigEditorState>),
    LiveSimulation(Box<LiveSimState>),
    Replay(Box<ReplayState>),
}
```

The large state structs are boxed to satisfy clippy's `large_enum_variant` lint
and keep the `AppMode` enum itself small.

`Home` is the default. It renders a centered splash screen with three action
buttons.

### Mode Transitions

All transitions are driven by toolbar actions or in-frame UI events. The
triggering condition and resulting state are described below.

```
Home  ──────────────────────────────────────────────────────────────►  ConfigEditor
         "Open Configuration File" → parse TOML via config::parse()
         "New Configuration"       → build default Simulation AST
         "Open Trace File"         → (bypasses ConfigEditor, see below)

Home  ──────────────────────────────────────────────────────────────►  Replay
         "Open Trace File" → open .nxs + optional adjacent nexus.toml

ConfigEditor  ──────────────────────────────────────────────────────►  LiveSimulation
         "▶ Run" toolbar button → launch_simulation()

LiveSimulation  ────────────────────────────────────────────────────►  Replay
         "View Replay" button (appears after sim finishes)
         → ReplayController::open(sim_dir/trace.nxs)

LiveSimulation  ────────────────────────────────────────────────────►  LiveSimulation
         "▶ Rerun" toolbar button → launch_simulation() with same sim AST

Any mode  ──────────────────────────────────────────────────────────►  Home
         "Home" toolbar button

Any mode  ──────────────────────────────────────────────────────────►  ConfigEditor
         "Open Config" or "New Config" toolbar button
```

Opening a config while a simulation is running drops `LiveSimState`, which
drops `SimController`, which sets the abort flag and joins the sim thread (see
[SimController](#simcontroller)).

---

## State Structs

### ConfigEditorState

Defined in `gui/src/state.rs`. Holds the in-memory simulation AST being edited.

| Field | Type | Purpose |
|---|---|---|
| `sim` | `ast::Simulation` | Live AST; all edits mutate this directly |
| `file_path` | `Option<PathBuf>` | The file the config was loaded from; `None` for new configs |
| `grid` | `GridView` | Pan/zoom state for the node placement preview |
| `selected_node` | `Option<String>` | Currently selected node name |
| `selected_channel` | `Option<String>` | Currently selected channel name |
| `validation_error` | `Option<String>` | Last validation or save error to display in red |
| `dirty` | `bool` | `true` if there are unsaved changes |
| `add_item_buf` | `String` | Shared text buffer for all inline "add item" rows |
| `needs_fit` | `bool` | When `true`, `fit_to_nodes` runs on the next frame and is reset |

`add_item_buf` is shared across all add-item rows in the config editor. Because
only one text input can be focused at a time, this is safe and avoids allocating
a separate buffer per section.

### LiveSimState

Holds the state of a running or just-finished simulation.

| Field | Type | Purpose |
|---|---|---|
| `sim` | `ast::Simulation` | Snapshot of the config used to launch this run |
| `controller` | `SimController` | Handle to the background sim thread |
| `grid` | `GridView` | Pan/zoom state for the node grid |
| `selected_node` | `Option<String>` | Currently selected node name |
| `current_timestep` | `u64` | Last timestep received from the kernel |
| `messages` | `Vec<MessageEntry>` | Accumulated message events (TX/RX/Drop) |
| `node_states` | `Vec<NodeState>` | Current per-node position and charge, updated by events |
| `sim_dir` | `PathBuf` | Directory where `trace.nxs` is written; used by "View Replay" |
| `paused` | `bool` | When `true`, events are not consumed from the channel |
| `needs_fit` | `bool` | Triggers auto-fit on next frame |
| `expanded_nodes` | `HashSet<String>` | Sole source of truth for inspector expand/collapse state |
| `hovered_node` | `Option<String>` | Node name under the pointer, set each frame |
| `panels` | `PanelVisibility` | Which side panels are currently visible |
| `active_arrows` | `Vec<ArrowAnimation>` | In-flight message arrow animations on the grid |
| `channel_subscribers` | `Vec<Vec<usize>>` | channel_index → subscriber node indices for arrow routing |
| `last_sender` | `Vec<Option<usize>>` | channel_index → last TX node index (for linking RX arrows) |
| `time_dilation` | `Arc<AtomicU64>` | Shared f64 (as bits) for live kernel time dilation adjustment |

### ReplayState

Holds the state of the replay mode. Shares the same visual fields as
`LiveSimState` but drives them from a trace file rather than a live channel.

| Field | Type | Purpose |
|---|---|---|
| `sim` | `ast::Simulation` | Config snapshot (loaded from adjacent `nexus.toml` if present, otherwise synthesized from the trace header) |
| `controller` | `ReplayController` | Loaded trace file with index and cache |
| `grid` | `GridView` | Pan/zoom state |
| `selected_node` | `Option<String>` | Currently selected node |
| `current_timestep` | `u64` | The timestep currently displayed |
| `total_timesteps` | `u64` | Total timestep count from the trace header |
| `playing` | `bool` | Whether auto-advance is active |
| `playback_speed` | `f32` | Speed multiplier (0.1–10.0); governs real-time advance rate via `time_accumulator` |
| `messages` | `Vec<MessageEntry>` | Accumulated message events up to `current_timestep` |
| `node_states` | `Vec<NodeState>` | Per-node state reconstructed at `current_timestep` |
| `initial_states` | `Vec<NodeState>` | Starting node states from the AST; kept to seed reconstruction without re-reading the AST |
| `needs_fit` | `bool` | Triggers auto-fit on next frame |
| `expanded_nodes` | `HashSet<String>` | Inspector expand/collapse state |
| `hovered_node` | `Option<String>` | Node under pointer |
| `panels` | `PanelVisibility` | Which side panels are visible |
| `active_arrows` | `Vec<ArrowAnimation>` | In-flight message arrow animations on the grid |
| `channel_subscribers` | `Vec<Vec<usize>>` | channel_index → subscriber node indices for arrow routing |
| `last_sender` | `Vec<Option<usize>>` | channel_index → last TX node index (for linking RX arrows) |
| `time_accumulator` | `f64` | Fractional timestep accumulator for real-time replay advance |

### NodeState

A lightweight, cloneable summary of a node's runtime properties used by all
rendering and inspection code.

```rust
pub struct NodeState {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub charge_ratio: Option<f32>,
    pub max_nj: Option<u64>,
    pub is_dead: bool,
    pub prev_x: f64,
    pub prev_y: f64,
    pub prev_z: f64,
    pub last_move_ts: u64,
}
```

`charge_ratio` is `None` for nodes without a `charge` block in their config.
When present, it is a value in `[0.0, 1.0]` where `1.0` means fully charged.
This ratio controls the color of the node circle on the grid:

- Blue — no charge tracking
- Green — high charge (ratio near 1.0)
- Yellow — medium charge (ratio near 0.5)
- Red — low charge (ratio near 0.0)
- Grey (semi-transparent) — dead (charge == 0)

`prev_x/y/z` and `last_move_ts` track the previous position for velocity
computation in the inspector panel. The velocity delta vector and speed
magnitude are displayed when `last_move_ts > 0`.

`nodes_from_sim` in `app.rs` builds the initial `Vec<NodeState>` from the AST.
Nodes are sorted alphabetically by name so that index-based node name lookups
from trace events (which refer to nodes by sorted index) are consistent.

### MessageEntry and MessageKind

`MessageEntry` is one row in the message log.

```rust
pub struct MessageEntry {
    pub timestep: u64,
    pub kind: MessageKind,
    pub src_node: String,
    pub dst_node: Option<String>,
    pub channel: String,
    pub data_preview: String,
    pub data_raw: Vec<u8>,
}

pub enum MessageKind {
    Sent,
    Received,
    Dropped(String),  // String is the drop reason
}
```

`data_preview` is produced by `format_data_preview` in `app.rs`:

1. Attempt to interpret the bytes as UTF-8.
2. If valid, check that all characters are printable or newline. If so, use the
   string directly, truncating at 64 characters with a byte-count suffix.
3. Otherwise fall back to lowercase hex, truncating at 32 bytes (64 hex chars)
   with a byte-count suffix.

`data_raw` stores the original bytes for clipboard copy, regardless of whether
the preview uses text or hex.

### PanelVisibility

```rust
pub struct PanelVisibility {
    pub inspector: bool,
    pub messages: bool,
}
```

Both fields default to `true`. Toggling via toolbar buttons flips the boolean
directly. A panel with its flag set to `false` is simply not rendered — no
collapsed strip is shown and no `egui::SidePanel` is created for it.

### ArrowAnimation and ArrowKind

```rust
pub struct ArrowAnimation {
    pub src_node: usize,
    pub dst_node: usize,
    pub kind: ArrowKind,
    pub start_time: f64,
    pub duration: f32,
}

pub enum ArrowKind {
    Sent,      // green (#64C864)
    Received,  // blue (#6496FF)
    Dropped,   // red (#FF6464) with X at midpoint
}
```

`ArrowAnimation` represents an in-flight message arrow on the grid. Arrows are
created by `process_gui_event` when message events arrive:

- **MessageSent**: green arrow from sender to each subscriber of the channel.
- **MessageRecv**: blue arrow from the last known sender to the receiver.
- **MessageDropped**: red arrow from sender to each subscriber with an X overlay.

Arrows animate for 0.25 seconds. Each frame, expired arrows are removed before
new events are processed. The `channel_subscribers` lookup (built at mode init
from the simulation config) maps channel indices to subscriber node indices.
`last_sender` tracks the most recent TX source per channel so that RX arrows
can be linked back to the sender.

---

## Rendering Pipeline

### Frame Ordering

egui renders panels in the order they are added within a single `update` call.
The order matters because each panel claims a portion of the available space and
the central panel fills whatever is left. The GUI uses the following order:

```
1. TopBottomPanel::top("toolbar")       — always rendered first; claims top strip
2. SidePanel::left("inspector")         — only when visible; claims left strip
3. SidePanel::right("messages")         — only when visible; claims right strip
4. TopBottomPanel::bottom("timeline")   — only in LiveSim and Replay modes
5. CentralPanel::default()              — fills remaining space; contains the grid
```

This order is required by egui. `TopBottomPanel` and `SidePanel` must be added
before `CentralPanel` because `CentralPanel` is a "greedy" widget that takes
all remaining space. Reversing the order would cause a panic or layout failure.

All panel rendering happens inside `NexusApp::update`. The toolbar is rendered
unconditionally first; then the body delegates to a mode-specific `show_*`
method which renders the rest.

### Toolbar Actions

`show_toolbar` returns a `ToolbarAction` enum value each frame rather than
mutating state directly. This keeps the toolbar function pure (it only reads
state) and lets `app.rs` handle all transitions in one place. Actions not
applicable to the current mode are never produced:

| Action | Condition |
|---|---|
| `GoHome` | Always available |
| `OpenConfig`, `NewConfig`, `OpenTrace` | Always available |
| `RunSimulation` | Only in `ConfigEditor` mode |
| `StopSimulation` | Only in `LiveSimulation` and simulation not yet finished |
| `RerunSimulation` | Only in `LiveSimulation` and simulation finished |
| `ToggleInspector`, `ToggleMessages` | Only in `LiveSimulation` and `Replay` |

---

## Grid and Node Rendering

### GridView

`GridView` (in `render/grid.rs`) manages the world-space to screen-space
coordinate transform. It holds two fields:

```rust
pub struct GridView {
    pub offset: Vec2,   // pixel offset from canvas center
    pub zoom: f32,      // world units per pixel inverse (higher = more zoomed in)
}
```

The coordinate system has the X axis pointing right and the Y axis pointing up
(screen Y is inverted). The canvas center maps to world origin `(0, 0)`.

**Coordinate transforms:**

```
screen_x = canvas_center_x + (world_x * zoom) + offset_x
screen_y = canvas_center_y - (world_y * zoom) + offset_y
```

**Pan input:** primary (left) mouse drag on empty space, middle-mouse drag, or
shift+left drag. Primary drag is suppressed when it starts on a node (tracked
via egui temp data) to avoid panning when clicking nodes.

**Zoom input:** scroll wheel on the canvas. Zoom is clamped to `[0.01, 1000.0]`
and applied multiplicatively (`factor = 1 + scroll * 0.002`).

**Auto-fit:** `fit_to_nodes` computes the bounding box of all node world
positions, then sets zoom and offset so the bounding box fits the canvas with
a 20% margin on each side. It is called once (guarded by `needs_fit`) when a
mode is first entered, and never again unless explicitly reset.

**Grid lines:** `GridView::draw` computes a "nice" world-space grid spacing that
targets approximately 80 pixels between lines, snapping to 1/2/5/10 multiples
of the nearest power of ten. The X and Y axes are drawn with a brighter stroke.
Grid labels appear at axis crossings.

### draw\_node

`render::node::draw_node` is a pure rendering function. It takes a `NodeState`,
looks up the screen position via `GridView::world_to_screen`, and uses the
egui painter to draw:

1. A filled circle colored by `charge_ratio` (blue/green/yellow/red).
2. A white selection ring (2px stroke, radius + 3px) if `selected` is `true`.
3. A text label 4px above the circle, centered horizontally.

The node radius is computed as:

```rust
pub fn node_radius(zoom: f32) -> f32 {
    NODE_RADIUS * zoom.sqrt().clamp(0.3, 3.0)
}
```

where `NODE_RADIUS = 4.0`. The square-root scaling softens radius changes as
the user zooms, keeping nodes visible at low zoom and not filling the screen at
high zoom.

Nodes whose screen position falls outside `canvas_rect` are skipped entirely.

### show\_grid\_panel and Hit Testing

`panels::grid::show_grid_panel` allocates the full remaining UI area as a
single egui widget with `Sense::click_and_drag()`, then:

1. Tracks whether the drag started on a node (persisted in egui temp data).
2. Calls `grid.handle_input(&response, drag_started_on_node)` to process
   pan/zoom (primary drag pans only when not started on a node).
3. Calls `grid.draw(ui, canvas_rect)` to paint grid lines.
4. Calls `draw_node` for each node.
5. Draws active message arrows (`ArrowAnimation` list) with animated progress.
6. Performs hit testing to determine which node (if any) was clicked or is
   currently hovered.

**Click detection** uses `response.clicked()` combined with
`response.interact_pointer_pos()`. The canvas widget with
`Sense::click_and_drag()` consumes all pointer events within its area, so
`response.clicked()` is the reliable signal — `ui.ctx().input(|i|
i.pointer.any_click())` would fire even for clicks outside the canvas.

**Hover detection** uses the raw pointer position:

```rust
let hovered_node = ui
    .ctx()
    .input(|i| i.pointer.hover_pos())
    .and_then(|pos| hit_test_node(pos, canvas_rect, grid, nodes));
```

Hover uses the raw pointer rather than `response.hovered()` because
`response.hovered()` is `true` only when the pointer is directly over the
canvas response area, not when it is over a tooltip or child widget. Using
`hover_pos()` directly gives accurate continuous hover tracking.

`hit_test_node` iterates all nodes and returns the name of the first node
whose circular hit area contains `pos`. The hit area is a square bounding box
of side `2 * node_radius(zoom)` centered at the node's screen position.

---

## Inspector Panel

`panels::inspector::show_inspector` renders a vertically scrollable list of
nodes. Each node entry has a header row (triangle button + name label) and,
when expanded, an indented detail block.

**Expand/collapse** is driven entirely by `expanded_nodes: &mut HashSet<String>`:

- If `name` is in the set, the node is expanded.
- Clicking the triangle button or the name label toggles membership.

egui's built-in `CollapsingState` is not used for the top-level inspector rows
because `CollapsingState` stores its open/closed boolean in egui's internal
memory keyed by widget ID. This creates a conflict: the grid panel can select a
node in the same frame that the inspector renders, but the inspector reads its
state at the beginning of the frame before the grid click is processed. The
result is a one-frame lag where the newly selected node appears selected but not
expanded. Using `HashSet` means the caller (`app.rs`) can insert the node name
into `expanded_nodes` at click time, and the inspector will read the updated set
at the start of the next frame — or, more precisely, in the same frame since
clicks are processed before the inspector panel is rendered (toolbar → inspector
→ grid in the rendering order).

> **Note:** Protocols inside a node's detail block do use `ui.collapsing`, which
> internally uses `CollapsingState`. This is safe because those inner sections
> are purely informational and do not interact with cross-panel selection state.

**Selected node highlighting:** when `selected_node` matches the node name, the
name is rendered with `ui.strong()` rather than `ui.label()`. There is no
selection-driven auto-scroll; the user must scroll to the selected node manually.

**Node details** show:

- Current position (`x`, `y`, `z`) from `NodeState`
- Velocity vector and speed magnitude (computed from `prev_x/y/z` delta) when
  the node has moved at least once (`last_move_ts > 0`)
- Charge percentage, if `charge_ratio` is `Some`
- Protocol list from the AST (`ast::Simulation`), each with its root path,
  publishers, and subscribers as collapsing sections

---

## Panel Visibility

When a panel's `PanelVisibility` flag is `false`, its `egui::SidePanel::show`
call is simply not made. There is no collapsed strip, no thin handle — the
panel is completely absent from the layout.

The alternative of collapsing the panel to a minimal width was rejected because
egui stores each `SidePanel`'s width in its memory. If a panel is rendered at
width 0 (or near zero) one frame and then at `default_width` the next, egui
interpolates and the panel animates in from nothing. This looked fine but
introduced a problem: if the panel was never rendered (its ID never appeared in
the layout), egui had no stored width and would use `default_width` without any
animation — producing an inconsistent experience depending on whether the user
had previously shown the panel. Omitting the panel entirely avoids this state
entirely and gives immediate, crisp show/hide behavior.

A side effect is that the central panel expands to fill the full width
immediately when a side panel is hidden, without any animation. This is
intentional.

---

## Config Editor

The config editor occupies the left portion of the `CentralPanel` via
`egui::SidePanel::left("config_sections")` rendered with `show_inside(ui, …)`.
The remaining area to the right of that panel shows the node placement grid.

The editor is organized into collapsing sections:

- **Parameters** — timestep length/unit/count, start time (RFC-3339 text
  input), seed, time dilation, root directory
- **Nodes** — add/remove nodes; per-node: position (x/y/z/az/el/roll),
  distance unit, optional charge (max/quantity/unit), resources
  (CPU cores/rate, memory), protocols, internal channels, sinks, sources,
  per-channel TX/RX energy costs
- **Channels** — add/remove channels; per-channel: type (Shared/Exclusive)
  with type-specific fields, then an inline link editor
- **Sinks** — add/remove global sink definitions with `PowerRate`
- **Sources** — add/remove global source definitions with `PowerRate`

At the bottom are **Validate**, **Save**, and **Save As...** buttons.
`Validate` round-trips the AST through `config::serialize_config` and
`config::deserialize_config`. Any error is shown in red below the buttons.

**Config loading** tries `config::parse` (the TOML source format) first, then
falls back to `config::deserialize_config` (the snapshot/round-trip format).
This lets users open either the original TOML or a `nexus.toml` saved by a
previous simulation run.

**Shared add buffer:** `ConfigEditorState.add_item_buf` is a single `String`
reused by all "add item" rows in `params`, `nodes`, `channels`, `sinks`, and
`sources`. Because egui focus is exclusive, only one text input is active at a
time, so sharing is safe.

**Protocol-level add buffers** are stored in egui's transient data store
(`ui.data(|d| d.get_temp(id))`) rather than in the AST state, because the
number of protocols per node is not known at `ConfigEditorState` construction
time and allocating per-entry state in the AST would be intrusive.

---

## Simulation Control

### SimController

`SimController` (in `sim/controller.rs`) is the GUI-side handle to the
background simulation thread.

```rust
pub struct SimController {
    pub rx: Receiver<GuiEvent>,
    abort: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
```

| Method | Description |
|---|---|
| `poll_events() -> Vec<GuiEvent>` | Non-blocking drain of all pending events via `try_iter()` |
| `stop()` | Sets `abort = true` with `Relaxed` ordering; the kernel checks this flag each step |
| `set_paused(bool)` | Sets `pause`; the kernel spin-waits while this flag is true |
| `is_finished() -> bool` | Returns `true` when the sim thread's `JoinHandle::is_finished()` is true |

`SimController` implements `Drop`: when the controller is dropped (e.g., when
the user opens a new config), it sets `abort = true` and joins the thread. This
ensures the sim thread never outlives the controller.

### launch\_simulation

`sim::launch::launch_simulation` prepares and starts a simulation run:

1. Creates a timestamped subdirectory under `sim.params.root` (format:
   `YYYY-MM-DD_HH:MM:SS`).
2. Serializes the current `ast::Simulation` to `nexus.toml` inside that
   directory, so the config is preserved alongside the trace.
3. Creates a `crossbeam_channel::unbounded` pair for `GuiEvent` delivery.
4. Initializes `AtomicBool` flags for abort and pause.
5. Calls `ensure_global_subscriber()` to install the tracing subscriber (once
   per process lifetime; subsequent calls return the existing `SimSinks` clone).
6. Spawns a `"nexus-sim"` thread that calls `run_simulation`, then sends
   `GuiEvent::SimulationComplete` or `GuiEvent::SimulationError` on exit, then
   calls `sinks.clear()` to flush the trace writer.
7. Returns `(SimController, sim_dir, Arc<AtomicU64>)` where the atomic holds
   the shared time dilation value (f64 bits) for live GUI adjustment.

Inside the thread, `run_simulation` creates a `TraceWriter`, installs it into
`SimSinks`, then calls `run_inner` which builds the FUSE filesystem, spawns
protocol processes, constructs the kernel, and runs it to completion.

### Pause and Stop

**Pause** sets `pause = true` via `SimController::set_paused`. The GUI side
stops consuming from `rx` while `state.paused` is true. Events accumulate in
the unbounded channel and are processed when playback resumes. The kernel reads
the same `AtomicBool` and spin-waits in its `step` loop while the flag is set,
so the simulation is held at a consistent timestep boundary.

**Stop** sets `abort = true`. The kernel checks this flag at the top of each
step and returns early if set.

**Rerun** drops the current `LiveSimState` (triggering `SimController::drop`
which aborts and joins the old thread) and calls `launch_simulation` again with
the same `ast::Simulation`. The new state starts fresh with empty messages and
reset node positions from the AST.

---

## Replay System

### ReplayController

`sim::replay::ReplayController` (in `sim/replay.rs`) loads an entire `.nxs`
trace file into memory on `open`:

```rust
pub struct ReplayController {
    reader: TraceReader,
    all_records: Vec<TraceRecord>,
    ts_ranges: Vec<(u64, Range<usize>)>,
    pub total_timesteps: u64,
    last_reconstructed: Option<(u64, Vec<NodeState>)>,
}
```

All records are eagerly loaded so that seeking is O(log n) without any I/O.

### Timestep Index

`build_ts_index` scans `all_records` once and builds `ts_ranges`: a sorted
`Vec<(timestep, range)>` where `range` is the slice of `all_records` belonging
to that timestep. All records within a timestep are assumed to appear
consecutively (they are written in-order by the kernel).

**`records_at(ts)`** does a binary search on `ts_ranges` and returns the
corresponding slice, or `&[]` if no records exist for that timestep.

**`records_through(ts)`** finds the end index of timestep `ts` in `ts_ranges`
and returns `&all_records[..end_idx]`. This covers all records from timestep 0
through `ts` inclusive.

### State Reconstruction

`reconstruct_states(ts, initial_states)` computes `Vec<NodeState>` for
timestep `ts` by applying all `PositionUpdate` and `EnergyUpdate` trace events
up to and including `ts` to a copy of `initial_states`.

The method uses incremental caching via `last_reconstructed: Option<(u64,
Vec<NodeState>)>`:

- **Cache hit (same timestep):** returns the cached states directly.
- **Forward seek (ts > cached_ts):** applies only the records between
  `cached_ts` and `ts` to the cached states, avoiding a full replay.
- **Backward seek (ts < cached_ts):** falls back to a full replay from
  `initial_states` through `ts`.

In normal playback (advancing one timestep per frame), the incremental path is
taken almost every frame, making reconstruction O(events in one timestep).
Seeking backward (slider scrub to an earlier time) triggers a full replay,
which is O(total events through ts).

### Message Gathering

The replay mode separates message accumulation from state reconstruction:

- **`gather_messages_at(controller, ts, sim, messages)`** appends only the
  message events (TX, RX, Drop) from exactly timestep `ts`.
- **`gather_messages_through(controller, ts, sim, messages)`** appends all
  message events from timestep 0 through `ts`.

`gather_messages_at` is used during forward playback and step-forward: events
are added incrementally. `gather_messages_through` is used after any seek
(slider, step-backward, jump-to-start/end): the message list is cleared first
and then rebuilt from scratch. This keeps the displayed message log consistent
with the current timeline position.

---

## Trace Bridge

### SimSinks

`sim::bridge::SimSinks` is a pair of swappable, lock-protected sinks:

```rust
pub struct SimSinks {
    pub gui_tx: Arc<Mutex<Option<Sender<GuiEvent>>>>,
    pub trace_writer: Arc<Mutex<Option<TraceWriter>>>,
}
```

`SimSinks` is `Clone`. All clones share the same underlying `Arc<Mutex<…>>`
values. A single global `OnceLock<SimSinks>` in `launch.rs` ensures only one
`SimSinks` instance is ever created per process.

`install(gui_tx, writer)` fills both options before a simulation run.
`clear()` replaces both with `None` after the run ends, which drops the
`TraceWriter` and flushes any buffered output to disk.

### ReloadableSimLayer

`sim::bridge::ReloadableSimLayer` is a `tracing_subscriber::Layer` that
intercepts events emitted by the kernel's routing code. It handles three
tracing targets:

| Target | Emitted by | Converted to |
|---|---|---|
| `"tx"` | Kernel on message send | `TraceEvent::MessageSent` |
| `"rx"` | Kernel on message receive | `TraceEvent::MessageRecv` |
| `"drop"` | Kernel on message drop | `TraceEvent::MessageDropped` |
| `"battery"` | Kernel on energy update | `TraceEvent::EnergyUpdate` |
| `"movement"` | Kernel on position change | `TraceEvent::PositionUpdate` |

Events on any other target pass through to the `fmt` layer (stderr) filtered
by `RUST_LOG`.

`BridgeVisitor` implements `tracing::field::Visit` and extracts named fields
from the tracing event:

| Field | Type | Purpose |
|---|---|---|
| `timestep` | `u64` | Simulation timestep |
| `channel` | `u64` → `u32` | Channel index (sorted alphabetically) |
| `node` | `u64` → `u32` | Node index (sorted alphabetically) |
| `tx` | `bool` | `true` for TX, `false` for RX (on `"tx"`/`"rx"` targets) |
| `data` | `&[u8]` | Message payload bytes |
| `reason` | `&str` | Drop reason string (on `"drop"` target) |

The layer is installed exactly once by `ensure_global_subscriber`. Subsequent
simulation runs share the same layer; only the sinks inside `SimSinks` change.

### GuiEvent

```rust
pub enum GuiEvent {
    Trace(TraceRecord),
    TimestepAdvanced(u64),
    SimulationComplete,
    SimulationError(String),
}
```

`Trace` carries a `TraceRecord` (timestep + `TraceEvent`) for every TX, RX, or
drop event. `TimestepAdvanced` updates the current timestep display even during
timesteps with no message events. `SimulationComplete` and `SimulationError`
signal the end of the run; both cause `is_finished()` to become true shortly
after.

### Event Flow

```
Kernel routing code
  │  tracing::event!(target: "tx" | "rx" | "drop" | "battery" | "movement", ...)
  ▼
ReloadableSimLayer::on_event()
  │  extracts fields via BridgeVisitor
  │
  ├──► SimSinks::gui_tx  ──crossbeam──►  SimController::rx
  │                                           │
  │                                           │  (polled each frame by NexusApp)
  │                                           ▼
  │                                      process_gui_event()
  │                                           │
  │                                           ├── updates current_timestep
  │                                           ├── updates node_states (position/charge)
  │                                           ├── appends to messages
  │                                           └── creates ArrowAnimations (TX/RX/Drop)
  │
  └──► SimSinks::trace_writer  ──►  trace.nxs  (for post-sim replay)
```

---

## Messages Panel

`panels::messages::show_messages` renders up to `max_display` (currently 200)
of the most recent entries in a vertical scroll area with `stick_to_bottom(true)`
so new messages auto-scroll into view while allowing manual scroll-up.

Each entry occupies two rows:

1. A horizontal row with: colored type tag (`[TX]`/`[RX]`/`[XX]`), timestep,
   source node, optional destination node, channel name, and a copy button (✘)
   if `data_raw` is non-empty.
2. An indented monospace small-text row with `data_preview` (omitted if empty).

Colors: TX = green `(100, 200, 100)`, RX = blue `(100, 150, 255)`, Drop =
red `(255, 100, 100)`.

The copy button calls `ui.ctx().copy_text(msg.data_preview.clone())`, placing
the preview text (UTF-8 string or hex string) on the system clipboard.

The panel is stateless as does not own the message list. The list lives in
`LiveSimState.messages` or `ReplayState.messages` and grows without bound
(in the current implementation, there is no eviction). The 200-entry display
cap is enforced only at render time by slicing from `messages.len() -
max_display`.

---

## Timeline Panel

`panels::timeline::show_timeline` renders a horizontal row of controls inside
a `TopBottomPanel::bottom`:

```
|<   <   >/>||   >   >|   ─── scrubber ───   Speed: 1.0x   t=42 / 1000
```

The function returns a `TimelineAction` struct rather than mutating state
directly:

```rust
pub struct TimelineAction {
    pub seek_to: Option<u64>,
    pub toggle_play: bool,
    pub step_forward: bool,
    pub step_backward: bool,
}
```

`seek_to` is set by both the slider (continuous drag) and the jump-to-start /
jump-to-end buttons. The caller in `app.rs` checks `seek_to` first; if set, it
clears the message list and calls `gather_messages_through` to rebuild from
scratch. Step operations append only the single-timestep delta.

In `LiveSimulation` mode, `toggle_play` calls `state.controller.set_paused`.
In `Replay` mode, `toggle_play` flips `state.playing`.

The speed `DragValue` modifies `playback_speed` in place. In replay mode, the
advance logic uses real-time playback: each frame, `dt * playback_speed` is
accumulated in `time_accumulator` and converted to simulation timesteps based
on the configured timestep duration. This produces correct real-time playback
at 1.0x speed and proportional fast/slow at other speeds.

In live simulation mode, a **time dilation slider** (0.1x–10.0x, logarithmic)
adjusts the kernel's CPU bandwidth allocation in real time via a shared
`Arc<AtomicU64>`. The kernel reads this value each step.

---

## Key Design Decisions

### Manual expand/collapse instead of egui CollapsingState

egui's `CollapsingState` stores its open/closed boolean in egui's per-frame
memory, keyed by widget ID. This works well when the collapsing widget is the
only code that needs to know whether a section is open. The inspector panel
breaks this assumption: clicking a node on the grid should expand that node in
the inspector in the same interaction. The grid click is processed at the end
of the `CentralPanel` rendering, which comes after the inspector
`SidePanel` has already been rendered for the current frame. Writing to
`CollapsingState` memory at that point would take effect only on the next
frame, causing a one-frame lag.

Using `HashSet<String>` as the expansion state moves the truth out of egui's
memory and into application state. `app.rs` can insert a node name into
`expanded_nodes` when a grid click is processed, and since `expanded_nodes` is
in the app state (not egui's internal memory), the inspector reads the updated
value the moment the HashSet is modified — even across the frame boundary.

### Response.clicked() for canvas click detection

The canvas is allocated with `Sense::click_and_drag()`, which means egui
routes all pointer events inside the canvas rect to this widget. Using the raw
`ctx.input(|i| i.pointer.any_click())` signal is unreliable here because it
fires for any click anywhere in the window and cannot be scoped to the canvas
rect without re-implementing egui's pointer routing logic. `response.clicked()`
is already correctly scoped and deduplicates with drag detection (a drag
gesture does not produce a click).

### Panels are not rendered rather than collapsed to thin strips

Collapsing a `SidePanel` to a thin strip requires the panel to remain present
in the layout — otherwise egui forgets its stored width. Keeping a zero-width
or near-zero-width panel present means the panel ID is in the layout tree, and
egui will try to animate its width back to `default_width` when it is shown
again. The animation looks reasonable but the stored-width state can become
stale across sessions (egui does not persist it). Omitting the panel entirely
avoids all stored-width interactions and gives a clean, immediate show/hide
with no animation artifact.
