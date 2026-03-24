# Codebase Structure

Nexus is a Cargo workspace. Each crate has a single, focused responsibility.

## Workspace Layout

```
nexus/
├── Cargo.toml          workspace manifest
├── cli/                entry point binary
├── config/             TOML parsing and validation
├── kernel/             discrete-event engine and message routing
├── fuse/               FUSE filesystem implementation
├── runner/             process execution and cgroup management
├── cpuutils/           CPU frequency / affinity utilities
├── trace/              binary trace format parsing and analysis
├── gui/                native desktop GUI (egui/eframe)
├── modules/            standard library of reusable config modules
├── examples/           18 runnable example simulations
└── doc/                documentation
```

## Crate Reference

### `cli` — Entry Point

**Path:** `cli/src/`

The `cli` crate is the binary entry point. It dispatches six subcommands:

| Subcommand | Description |
|------------|-------------|
| `simulate` | Run a simulation from a TOML config file |
| `replay` | Replay a completed simulation from binary trace logs |
| `logs` | Inspect or convert binary log files to CSV |
| `modules` | List, show, or verify reusable module files |
| `parse` | Parse and filter `.nxs` binary trace files |
| `fuzz` | (placeholder) Fuzz testing mode |

**Key files:**
- `main.rs` — argument parsing, subcommand dispatch, top-level orchestration
- `output.rs` — formats per-node protocol summaries as CSV (exit codes, stdout, stderr paths)

The `simulate` path performs the full startup sequence: config parse →
runner build → runner run → FUSE mount → kernel run → collect results.

See [cli-reference.md](cli-reference.md) for complete subcommand documentation.

---

### `config` — TOML Parsing and Validation

**Path:** `config/src/`

Parses a TOML simulation file into a typed `Simulation` AST. All user-visible
types (nodes, channels, links, positions, resources) are defined here. Also
handles module resolution and profile merging.

**Key files:**

| File | Responsibility |
|------|---------------|
| `ast.rs` | All type definitions: `Simulation`, `Node`, `Channel`, `Link`, `Medium`, `Charge`, `PowerRate`, `Position`, `Resources`, `NodeProfile` |
| `parse.rs` | TOML → AST deserialization |
| `validate.rs` | Cross-field validation (channel references, node protocols, energy config) |
| `channel.rs` | `ChannelType` helpers: TTL calculation, max buffer size |
| `medium.rs` | Signal model implementations: `rssi_wireless()` (Friis), `rssi_wired()` (RLGC) |
| `signal.rs` | Friis free-space path loss and RLGC transmission line models |
| `time.rs` | Timestep and delay calculations; time unit conversions |
| `resources.rs` | CPU/memory resource parsing and unit handling |
| `position.rs` | `Position` type: parsing, unit conversions, distance calculation |
| `units.rs` | Shared unit conversion utilities |
| `namespace.rs` | Name validation; checks handle uniqueness and naming rules |
| `module.rs` | Module resolution: `use` directive, stdlib path, NEXUS_MODULE_PATH search |
| `profile.rs` | Node profile merging: multi-profile layering, user-wins semantics |
| `serialize.rs` | Config snapshot serialization with CRC32 checksums for replay |

**Key types:**

```rust
Simulation {
    params: Params,                              // timestep, seed, root, time_dilation
    channels: HashMap<ChannelHandle, Channel>,
    nodes: HashMap<NodeHandle, Node>,
    sinks: HashMap<SinkHandle, PowerRate>,
    sources: HashMap<SourceHandle, PowerRate>,
}

Node {
    position: Position,
    charge: Option<Charge>,
    protocols: HashMap<ProtocolHandle, NodeProtocol>,
    internal_names: Vec<String>,
    resources: Resources,
    sinks: HashSet<SinkHandle>,
    sources: HashSet<SourceHandle>,
    start: Option<SystemTime>,
}

Channel { link: Link, channel_type: ChannelType }

Link {
    medium: Medium,
    bit_error: RssiProbExpr,
    packet_loss: RssiProbExpr,
    delays: DelayCalculator,
}
```

---

### `kernel` — Discrete-Event Engine

**Path:** `kernel/src/`

The core of Nexus. Owns the main simulation loop, message routing, energy
accounting, position tracking, health monitoring, and trace log writing.

#### `kernel/src/lib.rs` — `Kernel`

The `Kernel` struct owns two server threads and drives the event loop.

```rust
KernelBuilder::new(config, fuse_handles) -> KernelBuilder
    .time_dilation(arc)     // optional: shared speed control
    .build() -> Kernel
Kernel::run() -> Vec<ProtocolSummary>
```

#### `kernel/src/router/` — `RoutingServer`

Runs in a dedicated thread. On each timestep it:

1. Advances all node positions via active motion patterns.
2. Performs per-node energy accounting (sources, sinks, power state drain).
3. Drains all `FsMessage::Write` events (protocol transmissions) from FUSE.
4. For each write, computes delivery time and applies link simulation (bit
   error injection, packet loss via RSSI expression evaluation).
5. Drains all `FsMessage::Read` events (protocols waiting for data) from FUSE.
6. Delivers messages whose simulated delivery time has elapsed.
7. Reports energy events (depleted/recovered nodes) to the main loop.

Key files:

| File | Responsibility |
|------|---------------|
| `mod.rs` | `RoutingServer` struct; `serve()` thread loop; `poll()` per timestep |
| `delivery.rs` | Message delivery: TTL expiry, exclusive vs. shared fanout, RSSI recomputation |
| `link_simulation.rs` | Bit error injection, packet loss evaluation, RSSI → probability |
| `table.rs` | `RoutingTable`: publisher→subscriber graph with dynamic distance computation |
| `timectl.rs` | Handles `ctl.time/*` reads/writes and `ctl.elapsed/*` reads |
| `messages.rs` | `RouterMessage` enum |
| `energy_tests.rs` | 37 tests for energy accounting, death/restart, TX/RX costs, PID remapping |

The message queue is a `BinaryHeap<(Reverse<Timestep>, seq, Message)>` to
maintain causal order with deterministic tie-breaking via sequence numbers.

#### `kernel/src/energy.rs` — `EnergyManager`

Per-node battery tracking with power states, sources, sinks, and death/restart
lifecycle. See [energy-framework.md](energy-framework.md).

#### `kernel/src/types.rs` — Core Types

Defines `NodeIdx`, `ChannelIdx`, `MotionPattern` (Static/Velocity/Linear/Circle),
`PowerFlowState` (Constant/PiecewiseLinear), `EnergyState`, and `SignalInfo`.

#### `kernel/src/status/` — `StatusServer`

Runs in a dedicated thread. Responsibilities:

- **Health check**: Polls all child process PIDs each timestep to detect
  premature exits.
- **Resource update**: Reads host CPU frequency from `/sys/`, recomputes the
  throttle ratio for each protocol, and writes updated `cpu.max` values to
  the cgroup hierarchy.
- **Node freeze/unfreeze**: Writes to `cgroup.freeze` when nodes deplete
  or recover energy.
- **Process respawn**: Kills and respawns protocol processes on energy recovery,
  producing PID remap pairs for the FUSE filesystem.

Key files:

| File | Responsibility |
|------|---------------|
| `mod.rs` | `StatusServer`; CPU freq refresh; periodic tick; time dilation support |
| `health.rs` | PID existence checks; premature exit detection |
| `messages.rs` | `KernelMessage` / `StatusMessage` enums |

#### `kernel/src/logging.rs` — Trace Logging

Emits structured trace events via the `tracing` crate. Events include:
MessageSent, MessageRecv, MessageDropped, PositionUpdate, EnergyUpdate,
MotionUpdate. These are captured by either the binary log layer (for `.nxs`
files) or the GUI bridge layer (for live visualization).

#### `kernel/src/resolver.rs` — Handle Resolution

Converts string-keyed config types to usize-indexed kernel types at startup,
enabling O(1) lookup in the hot routing path.

---

### `fuse` — FUSE Filesystem

**Path:** `fuse/src/`

Implements the FUSE filesystem that protocol processes interact with.
Exposes channel directories (with `channel`, `rssi`, `snr` sub-files) and
control file directories (`ctl.time/`, `ctl.elapsed/`, `ctl.pos/`) plus
flat control files (`ctl.energy_left`, `ctl.energy_state`, `ctl.power_flows`)
under each protocol's root directory.

**Key files:**

| File | Responsibility |
|------|---------------|
| `lib.rs` | Public types: `KernelMessage`, `FsMessage`, `NexusFs`, `NexusChannel` |
| `fs.rs` | FUSE ops: `lookup`, `getattr`, `open`, `read`, `write`, `release`; `CONTROL_FILES`, `TIME_SUBFILES`, `ELAPSED_SUBFILES`, `POS_SUBFILES`, `CHANNEL_SUBFILES` arrays |
| `file.rs` | `NexusFile`: per-PID message buffer; manages queued messages for each subscriber |
| `channel.rs` | `ChannelMode` enum: ReadOnly, WriteOnly, ReadWrite, ReplayWrites, FuzzWrites |
| `errors.rs` | `FsError` enum |

**IPC with kernel:**

The FUSE thread and kernel communicate via two `mpsc` channels:
- `FsMessage` (FUSE → kernel): `Write(Message)` when a process writes to a
  channel file; `Read(Message)` when a process reads and needs a queued
  message.
- `KernelMessage` (kernel → FUSE): `Exclusive(Message)`, `Shared(Message)`,
  or `Empty(Message)` to deliver a message or signal no-data.

---

### `runner` — Process Execution

**Path:** `runner/src/`

Compiles protocol code and launches protocol processes. Sets up the
cgroup v2 hierarchy for resource control.

**Key files:**

| File | Responsibility |
|------|---------------|
| `lib.rs` | `build()` — compile protocols; `run()` — spawn processes, return `RunController` |
| `cli.rs` | `Cli` struct (clap), `RunCmd` enum with all subcommands and flags |
| `cgroups.rs` | `CgroupController`: create cgroup hierarchy, write `cgroup.procs`, `cpu.weight`, `cpu.max`, `cpu.uclamp.*`, `cgroup.freeze` |
| `assignment.rs` | `Affinity` (CPU pinning), `Bandwidth` (cpu.max ratio with time dilation), `Relative` (cpu.weight) computation |
| `errors.rs` | `RunnerError` enum |

**cgroup v2 hierarchy:**

```
/sys/fs/cgroup/nexus/
├── nodes_limited/          ← CPU-throttled protocols
│   └── <node>_<protocol>/
└── nodes_unlimited/        ← unrestricted protocols (no clock_rate set)
    └── <node>_<protocol>/
```

Control files written per-cgroup: `cgroup.procs`, `cgroup.freeze`,
`cpu.weight`, `cpu.max`, `cpu.uclamp.min`, `cpu.uclamp.max`.

**`RunController`** holds references to all cgroup controllers, affinity
settings, and process handles. The kernel's status server holds a
`RunController` to update bandwidth on each tick.

---

### `trace` — Trace File Parsing

**Path:** `trace/src/`

Reads and analyzes `.nxs` binary trace files produced by the kernel's logging
layer. Provides the implementation for the `nexus parse` CLI command.

**Key files:**

| File | Responsibility |
|------|---------------|
| `lib.rs` | `TraceHeader`, `TraceEvent` types; trace file reading |
| `parse.rs` | `run_parse()`: filtering, formatting, and adapter support for the `parse` command |

**Capabilities:**
- Parse trace headers (node names, channel names, timestep count, max energy)
- Filter events by type (tx, rx, drop, position, energy, motion)
- Filter by node name, channel name, and timestep range
- Output as text, JSON, or JSON Lines
- Pipe payloads through external adapter commands for decoding

See [trace-format.md](trace-format.md) for format details.

---

### `gui` — Desktop GUI

**Path:** `gui/src/`

Native desktop application built with [egui](https://github.com/emilk/egui)
and [eframe](https://github.com/emilk/egui/tree/master/crates/eframe).

**Key files:**

| File | Responsibility |
|------|---------------|
| `main.rs` | Entry point; constructs `NexusApp` and hands it to eframe |
| `app.rs` | `NexusApp`: App impl, per-mode rendering, event processing |
| `state.rs` | `AppMode` enum (Home, ConfigEditor, LiveSimulation, Replay) and all state structs |
| `config_editor/` | Form-based TOML editor (params, links, channels, nodes) with module browser |
| `panels/` | Grid, inspector, messages, timeline, and toolbar panels |
| `render/` | GridView (pan/zoom), node circles, message arcs |
| `sim/` | `SimController`, `launch_simulation`, kernel thread management |

**Application modes:**
- **Home**: Splash screen with action buttons
- **Config Editor**: Form-based config editing with live grid preview and module browser
- **Live Simulation**: Real-time visualization with pause/resume and speed control
- **Replay**: Scrubber interface for recorded `.nxs` trace files

See [gui.md](gui.md) for detailed documentation.

---

### `cpuutils` — CPU Frequency and Affinity

**Path:** `cpuutils/src/`

Low-level helpers for reading host CPU state and setting process affinity.

| File | Responsibility |
|------|---------------|
| `cpufreq.rs` | `CpuInfo`, `CoreInfo`: reads `/sys/devices/system/cpu/cpu*/cpufreq/` for current/min/max frequency |
| `cpuset.rs` | `sched_setaffinity` wrapper; CPU topology detection (physical vs. logical cores) |
| `errors.rs` | `CpuUtilsError` enum |

Used by `runner` to determine CPU topology for affinity assignment and by
the kernel's status server to refresh per-core frequency measurements each
tick (since frequency can change dynamically under DVFS).

## Inter-Crate Dependencies

```
cli
 ├── config
 ├── runner
 ├── kernel
 ├── fuse
 └── trace

kernel
 ├── config
 ├── fuse      (message types)
 └── runner    (RunController for resource updates)

runner
 ├── config
 └── cpuutils

gui
 ├── config
 ├── kernel
 ├── fuse
 └── trace

fuse
 └── config    (channel type info)

trace
 └── (standalone, no internal deps)
```
