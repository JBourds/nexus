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
└── cpuutils/           CPU frequency / affinity utilities
```

## Crate Reference

### `cli` — Entry Point

**Path:** `cli/src/`

The `cli` crate is the binary entry point. It dispatches three subcommands:

| Subcommand | Description |
|------------|-------------|
| `simulate` | Run a simulation from a TOML config file |
| `replay` | Replay a completed simulation from binary trace logs |
| `logs` | Inspect or convert binary log files to CSV |

**Key files:**
- `main.rs` — argument parsing, subcommand dispatch, top-level orchestration
- `output.rs` — formats per-node protocol summaries as CSV (exit codes, stdout, stderr paths)

The `simulate` path performs the full startup sequence: config parse →
runner build → runner run → FUSE mount → kernel run → collect results.

---

### `config` — TOML Parsing and Validation

**Path:** `config/src/`

Parses a TOML simulation file into a typed `Simulation` AST. All user-visible
types (nodes, channels, links, positions, resources) are defined here.

**Key files:**

| File | Responsibility |
|------|---------------|
| `ast.rs` | All type definitions: `Simulation`, `Node`, `Channel`, `Link`, `Medium`, `Charge`, `PowerRate`, `Position`, `Resources` |
| `parse.rs` | TOML → AST deserialization |
| `validate.rs` | Cross-field validation (e.g., channel references valid links, node protocols reference valid channels) |
| `channel.rs` | `ChannelType` helpers: TTL calculation, max buffer size |
| `medium.rs` | Signal model implementations: `rssi_wireless()` (Friis), `rssi_wired()` (RLGC) |
| `time.rs` | Timestep and delay calculations; time unit conversions |
| `signal_model.rs` | Friis free-space path loss and RLGC models |
| `resources.rs` | CPU/memory resource parsing and unit handling |
| `position.rs` | `Position` type: parsing, unit conversions, distance calculation |
| `units.rs` | Shared unit conversion utilities |
| `namespace.rs` | Name validation; checks handle uniqueness and naming rules |

**Key types:**

```rust
Simulation {
    params: Params,
    channels: HashMap<ChannelHandle, Channel>,
    nodes: HashMap<NodeHandle, Node>,
    sinks: HashMap<SinkHandle, PowerRate>,    // power sinks (future)
    sources: HashMap<SourceHandle, PowerRate>, // power sources (future)
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

The core of Nexus. Owns the main simulation loop, message routing, health
monitoring, and binary log writing.

#### `kernel/src/lib.rs` — `Kernel`

The `Kernel` struct owns two server threads and drives the event loop.

```rust
Kernel::new(config, fuse_handles) -> Kernel
Kernel::run() -> Vec<ProtocolSummary>
```

#### `kernel/src/router/` — `RoutingServer`

Runs in a dedicated thread. On each timestep it:

1. Drains all `FsMessage::Write` events (protocol transmissions) from FUSE.
2. For each write, computes delivery time and applies link simulation (bit
   error injection, packet loss via RSSI expression evaluation).
3. Drains all `FsMessage::Read` events (protocols waiting for data) from FUSE.
4. Delivers messages whose simulated delivery time has elapsed by sending
   `KernelMessage::Exclusive` / `Shared` / `Empty` back to FUSE.

Key files:

| File | Responsibility |
|------|---------------|
| `mod.rs` | `RoutingServer` struct; `serve()` thread loop; `poll()` per timestep |
| `delivery.rs` | Message delivery logic: TTL expiry, exclusive vs. shared fanout |
| `link_simulation.rs` | Bit error injection, packet loss evaluation, RSSI → probability |
| `table.rs` | `RoutingTable`: pre-computed at startup; maps (publisher, channel) → subscribers with RSSI |
| `timectl.rs` | Handles `ctl.time.*` reads (return current sim time) and writes (block until time) |
| `messages.rs` | `RouterMessage` enum |

The message queue is a `BinaryHeap<(Reverse<Timestep>, seq, Message)>` to
maintain causal order with deterministic tie-breaking via sequence numbers.

#### `kernel/src/status/` — `StatusServer`

Runs in a dedicated thread. Responsibilities:

- **Health check**: Polls all child process PIDs each timestep to detect
  premature exits (e.g., a protocol crashed before the simulation ended).
- **Resource update**: Reads host CPU frequency from `/sys/`, recomputes the
  throttle ratio for each protocol, and writes updated `cpu.max` values to
  the cgroup hierarchy.

Key files:

| File | Responsibility |
|------|---------------|
| `mod.rs` | `StatusServer`; CPU freq refresh; periodic tick |
| `health.rs` | PID existence checks; premature exit detection |
| `messages.rs` | `KernelMessage` / `StatusMessage` enums |

#### `kernel/src/log.rs` — Binary Logging

Writes binary TX and RX event logs using `bincode`. Used by the `replay`
command to reconstruct the exact sequence of message deliveries.

#### `kernel/src/resolver.rs` — Handle Resolution

Converts string-keyed config types to usize-indexed kernel types at startup,
enabling O(1) lookup in the hot routing path.

```rust
ResolvedChannels {
    nodes: Vec<Node>,
    node_names: Vec<String>,
    channels: Vec<Channel>,
    channel_names: Vec<String>,
    handles: Vec<(PID, NodeHandle, ChannelHandle)>,
}
```

---

### `fuse` — FUSE Filesystem

**Path:** `fuse/src/`

Implements the FUSE filesystem that protocol processes interact with.
Exposes channel files (one per subscribed/published channel) and control
files (`ctl.*`) under each protocol's root directory.

**Key files:**

| File | Responsibility |
|------|---------------|
| `lib.rs` | Public types: `KernelMessage`, `FsMessage`, `NexusFs`, `NexusChannel` |
| `fs.rs` | FUSE ops: `lookup`, `getattr`, `open`, `read`, `write`, `release`; `CONTROL_FILES` array |
| `file.rs` | `NexusFile`: per-PID message buffer; manages queued messages for each subscriber |
| `channel.rs` | `ChannelMode` enum: distinguishes exclusive vs. shared channel behavior |
| `errors.rs` | `FsError` enum |

**IPC with kernel:**

The FUSE thread and kernel communicate via two `mpsc` channels:
- `FsMessage` (FUSE → kernel): `Write(Message)` when a process writes to a
  channel file; `Read(Message)` when a process reads and needs a queued
  message.
- `KernelMessage` (kernel → FUSE): `Exclusive(Message)`, `Shared(Message)`,
  or `Empty(Message)` to deliver a message or signal no-data.

**CONTROL_FILES** is a static array in `fs.rs` listing all control file
names and their access modes. The FUSE read/write handlers dispatch on
filename to the appropriate handler (`timectl.rs` for time files, etc.).

---

### `runner` — Process Execution

**Path:** `runner/src/`

Compiles protocol code and launches protocol processes. Sets up the
cgroup v2 hierarchy for resource control.

**Key files:**

| File | Responsibility |
|------|---------------|
| `lib.rs` | `build()` — compile protocols; `run()` — spawn processes, return `RunController` |
| `cli.rs` | `RunCmd` enum: build vs. run command definitions |
| `cgroups.rs` | `CgroupController`: create cgroup hierarchy, write `cgroup.procs`, `cpu.weight`, `cpu.max`, `cpu.uclamp.*`, `cgroup.freeze` |
| `assignment.rs` | `Affinity` (CPU pinning), `Bandwidth` (cpu.max ratio), `Relative` (cpu.weight) computation |
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
 └── fuse

kernel
 ├── config
 ├── fuse      (message types)
 └── runner    (RunController for resource updates)

runner
 ├── config
 └── cpuutils

fuse
 └── config    (channel type info)
```
