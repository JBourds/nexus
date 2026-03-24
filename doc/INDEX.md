# Nexus Codebase Index

> **Purpose:** This document is designed for LLM agents working on the Nexus
> codebase. It maps features to source files so agents can load the right
> context for any task. Read this file first when starting work on Nexus.

---

## Quick Reference: What To Read For Common Tasks

| Task | Files to load |
|------|---------------|
| Add a new CLI subcommand | `runner/src/cli.rs`, `cli/src/main.rs` |
| Add a new control file | `fuse/src/fs.rs` (CONTROL_FILES/subfile arrays), `kernel/src/router/mod.rs` (handler) |
| Add a new channel sub-file | `fuse/src/fs.rs` (CHANNEL_SUBFILES), `kernel/src/router/delivery.rs` |
| Modify link simulation | `kernel/src/router/link_simulation.rs`, `config/src/signal.rs` |
| Add a config field | `config/src/ast.rs` (types), `config/src/parse.rs` (deser), `config/src/validate.rs` (validation) |
| Add a new module to stdlib | `modules/` directory, `config/src/module.rs` |
| Change energy accounting | `kernel/src/router/mod.rs` (step fn), `kernel/src/energy.rs`, `kernel/src/types.rs` (EnergyState) |
| Change position/motion | `kernel/src/types.rs` (MotionPattern), `kernel/src/router/mod.rs` (apply_all_motions_and_log) |
| Add a trace event type | `kernel/src/logging.rs`, `trace/src/lib.rs` |
| Modify the GUI | `gui/src/app.rs` (main), `gui/src/state.rs` (state machine) |
| Add a config editor field | `gui/src/config_editor/` (params.rs, links.rs, channels.rs, nodes.rs) |
| Fix message delivery | `kernel/src/router/delivery.rs`, `kernel/src/router/table.rs` |
| Fix cgroup/resource control | `runner/src/cgroups.rs`, `runner/src/assignment.rs`, `kernel/src/status/mod.rs` |
| Write a test | `kernel/src/router/energy_tests.rs` (energy), `fuse/src/fs.rs` (FUSE), `config/src/validate.rs` (config) |

---

## Project Overview

Nexus is a discrete-event network simulator. Protocols run as real OS
processes; the simulator exposes channels as FUSE filesystem files and
controls CPU timing via Linux cgroups v2.

- **Language:** Rust (2024 edition)
- **Build:** Cargo workspace with 8 member crates
- **Platform:** Linux only (requires cgroup v2 + FUSE3)
- **Test command:** `cargo test`
- **Build command:** `cargo build --release`

---

## Workspace Crates

### `cli/` — Binary entry point
- `cli/src/main.rs` — Subcommand dispatch: simulate, replay, logs, modules, parse
- `cli/src/output.rs` — CSV output formatting for protocol summaries

### `config/` — TOML parsing, validation, modules
- `config/src/ast.rs` — **All type definitions**: Simulation, Node, Channel, Link, Medium, Charge, PowerRate, Position, Resources, NodeProfile, ChannelType, DeploymentConfig
- `config/src/parse.rs` — TOML deserialization into AST
- `config/src/validate.rs` — Cross-field validation and AST finalization (called `validate::validate()`)
- `config/src/module.rs` — Module resolution: `use` directive, stdlib path, NEXUS_MODULE_PATH
- `config/src/profile.rs` — Node profile merging and multi-profile layering
- `config/src/serialize.rs` — Config snapshot with CRC32 for replay
- `config/src/channel.rs` — ChannelType helpers (TTL, buffer size)
- `config/src/signal.rs` — Friis and RLGC signal model implementations
- `config/src/medium.rs` — `rssi_wireless()`, `rssi_wired()` computations
- `config/src/time.rs` — Timestep config, delay calculations, time unit conversions
- `config/src/resources.rs` — CPU/memory resource parsing
- `config/src/position.rs` — Position type, unit conversions, distance calc
- `config/src/units.rs` — Shared unit conversion utilities
- `config/src/namespace.rs` — Name validation, handle uniqueness

### `kernel/` — Discrete-event engine
- `kernel/src/lib.rs` — `KernelBuilder`, `Kernel` struct, main event loop (`run()`)
- `kernel/src/router/mod.rs` — `RoutingServer`: `serve()` thread, `poll()` per timestep, `step()` with energy + motion + message delivery
- `kernel/src/router/delivery.rs` — Message delivery: TTL, exclusive/shared fanout, RSSI recomputation from live positions
- `kernel/src/router/link_simulation.rs` — Bit error injection, packet loss, RSSI probability evaluation
- `kernel/src/router/table.rs` — `RoutingTable`: publisher→subscriber graph, dynamic distance
- `kernel/src/router/timectl.rs` — `ctl.time/*` and `ctl.elapsed/*` read/write handlers
- `kernel/src/router/messages.rs` — `RouterMessage` enum
- `kernel/src/router/energy_tests.rs` — 37 energy accounting tests
- `kernel/src/energy.rs` — `EnergyManager`, `EnergyState::from_node()` conversion
- `kernel/src/types.rs` — `NodeIdx`, `ChannelIdx`, `MotionPattern` (Static/Velocity/Linear/Circle), `PowerFlowState`, `EnergyState`, `SignalInfo`
- `kernel/src/status/mod.rs` — `StatusServer`: health checks, CPU freq refresh, cgroup bandwidth, freeze/respawn
- `kernel/src/status/health.rs` — PID existence checks
- `kernel/src/status/messages.rs` — `StatusMessage` enum
- `kernel/src/logging.rs` — Trace event emission via `tracing` crate
- `kernel/src/resolver.rs` — String→usize handle resolution at startup

### `fuse/` — FUSE filesystem
- `fuse/src/lib.rs` — Public types: `KernelMessage`, `FsMessage`, `NexusFs`, `NexusChannel`
- `fuse/src/fs.rs` — **FUSE ops** + control file arrays: `CONTROL_FILES` (3 flat files), `TIME_SUBFILES` (4), `ELAPSED_SUBFILES` (4), `POS_SUBFILES` (10), `CHANNEL_SUBFILES` (3: channel, rssi, snr)
- `fuse/src/file.rs` — `NexusFile`: per-PID message buffer
- `fuse/src/channel.rs` — `ChannelMode` enum: ReadOnly, WriteOnly, ReadWrite, ReplayWrites, FuzzWrites

### `runner/` — Process execution, cgroups
- `runner/src/lib.rs` — `build()`, `run()`, `RunController`
- `runner/src/cli.rs` — `Cli` struct (clap), `RunCmd` enum with all subcommands/flags
- `runner/src/cgroups.rs` — `CgroupController`: cgroup hierarchy creation, cpu.max, cpu.weight, cgroup.freeze
- `runner/src/assignment.rs` — `Affinity`, `Bandwidth` (with time_dilation), `Relative` computation

### `cpuutils/` — CPU frequency/affinity
- `cpuutils/src/cpufreq.rs` — Read CPU frequencies from sysfs
- `cpuutils/src/cpuset.rs` — `sched_setaffinity`, CPU topology

### `trace/` — Trace file parsing
- `trace/src/lib.rs` — `TraceHeader`, `TraceEvent` types, trace file reading
- `trace/src/parse.rs` — `run_parse()`: filtering, formatting, adapter support

### `gui/` — Desktop GUI (egui/eframe)
- `gui/src/main.rs` — Entry point
- `gui/src/app.rs` — `NexusApp`: per-mode rendering, event processing
- `gui/src/state.rs` — `AppMode` enum (Home, ConfigEditor, LiveSimulation, Replay), all state structs
- `gui/src/config_editor/` — Form editors: params.rs, links.rs, channels.rs, nodes.rs, widgets.rs
- `gui/src/panels/` — grid.rs, inspector.rs, messages.rs, timeline.rs, toolbar.rs
- `gui/src/render/` — grid.rs (GridView), node.rs (draw_node), message.rs (message arcs)
- `gui/src/sim/` — SimController, launch_simulation, kernel thread management

---

## FUSE Filesystem Structure

Protocol processes see this directory layout:

```
<node>/
├── <channel>/channel          # message read/write
├── <channel>/rssi             # last RX signal strength (dBm)
├── <channel>/snr              # last RX signal-to-noise (dB)
├── ctl.time/{s,ms,us,ns}     # simulated time (RW)
├── ctl.elapsed/{s,ms,us,ns}  # elapsed time (RO)
├── ctl.pos/{x,y,z,az,el,roll,dx,dy,dz,motion}  # position (RW/WO)
├── ctl.energy_left            # battery charge in nJ (RO)
├── ctl.energy_state           # power state name (RW)
└── ctl.power_flows            # power sources/sinks (RW)
```

Defined in: `fuse/src/fs.rs` lines 27–68.

---

## Configuration Structure

```toml
use = ["module/name"]           # Module imports

[params]                        # Simulation-wide: timestep, seed, root, time_dilation
[links.X]                       # Link definitions: medium, errors, delays, inherit
[channels.X]                    # Channel definitions: link, type, ttl, max_size
[nodes.X]                       # Node classes: profile, deployments, protocols, energy
```

Types defined in: `config/src/ast.rs`.
Parsing in: `config/src/parse.rs`.
Validation in: `config/src/validate.rs`.

---

## Standard Library Modules (24 files)

Located in `modules/`:

| Category | Modules | Provides |
|----------|---------|----------|
| `batteries/` | 18650, cr2032, lipo_1s_500mah | Profiles (charge configs) |
| `boards/` | arduino_mega, arduino_uno, esp32_devkit, esp32_s3, rpi_pico, rpi_zero_w, stm32f4 | Profiles (resources, power states) |
| `energy/` | energy_harvester, solar_medium, solar_small | Profiles (power sources) |
| `lora/` | ra01_433mhz, sx1262_915mhz, sx1276_868mhz, sx1276_915mhz | Links + channels |
| `wifi/` | esp32_wifi, wifi_2_4ghz, wifi_5ghz | Links + channels + profiles |
| `wired/` | ethernet_cat5e, ethernet_cat6, serial_uart, usb_2_0 | Links + channels |

---

## Examples (18 simulations)

Located in `examples/`:

| Example | What it demonstrates |
|---------|---------------------|
| `loopback` | Single node, read_own_writes |
| `single_client_single_server` | Basic point-to-point |
| `multi_client_single_server` | Multiple publishers |
| `multihop_bad_link` | 3-node chain with lossy link |
| `multihop_same_channel` | 3-node chain, shared channel |
| `multihop_separate_channels` | 3-node chain, separate channels |
| `count` | Timestep counting (C) |
| `time` | Time control files (C) |
| `elapsed` | Elapsed time queries (C) |
| `arduino` | Cross-compiled Arduino binary |
| `tdma` | TDMA on shared channel |
| `mobile_nodes` | Moving nodes (velocity, circle, linear) |
| `energy_framework` | Battery, power states, death/restart |
| `modules_lora_mesh` | 3-node LoRa mesh via modules |
| `modules_mixed_network` | Mixed wired/wireless via modules |
| `modules_solar_sensor` | Solar energy harvester via modules |

---

## Key Architectural Patterns

1. **String handles in config → usize indices in kernel** (`kernel/src/resolver.rs`)
2. **FUSE↔kernel IPC via mpsc channels** (`FsMessage` and `KernelMessage`)
3. **Per-PID buffering** — each process has independent message queues
4. **Dynamic RSSI** — distance computed from live positions, not precomputed
5. **Energy as integer nanojoules** — all arithmetic uses `u64` with `saturating_sub`
6. **Motion patterns evaluated per-timestep** — `current_point(timestep, us_per_step)`
7. **Trace events via `tracing` crate** — kernel emits structured events, captured by either binary log layer or GUI bridge

---

## Documentation Map

| Document | When to read it |
|----------|----------------|
| [architecture.md](architecture.md) | Understanding the system design and data flow |
| [config-reference.md](config-reference.md) | Adding or modifying configuration options |
| [simulation-files.md](simulation-files.md) | Working on the FUSE filesystem or control files |
| [cli-reference.md](cli-reference.md) | Adding CLI subcommands or flags |
| [trace-format.md](trace-format.md) | Working on trace logging or the parse command |
| [energy-framework.md](energy-framework.md) | Modifying energy accounting or battery lifecycle |
| [position-control.md](position-control.md) | Working on mobile nodes or motion patterns |
| [modules.md](modules.md) | Adding modules or changing the module system |
| [gui.md](gui.md) | Working on the GUI application |
| [crates.md](crates.md) | Understanding crate responsibilities and dependencies |
| [known-gaps.md](known-gaps.md) | Finding unimplemented features or known limitations |
| [defaults.toml](defaults.toml) | Default values for all configuration parameters |
