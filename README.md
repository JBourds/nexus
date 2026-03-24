# Nexus

A discrete-event network simulator for testing real protocol code. Protocols
run as ordinary OS processes; the simulator exposes channels as filesystem
files (via FUSE) and controls timing using Linux cgroups v2.

## Key Properties

- **Protocol-agnostic**: Any executable that reads/writes files works. C, Rust,
  Python, Arduino binaries cross-compiled for Linux — all work without
  modification beyond replacing radio calls with file I/O.
- **Physics-accurate links**: Configurable Friis/RLGC signal models, bit error
  rates, packet loss, propagation/processing/transmission delays.
- **Deterministic**: Fixed random seed, discrete event loop, reproducible
  results with a replay mode.
- **Resource emulation**: CPU clock-rate emulation via cgroup v2 bandwidth
  throttling. Protocols see simulated CPU speed, not host CPU speed.
- **Energy-aware**: Per-node battery tracking with power states, solar/charger
  sources, death/restart lifecycle, and per-channel TX/RX costs.
- **Mobile nodes**: Runtime position control with velocity, linear interpolation,
  and circular orbit motion patterns. RSSI recomputed dynamically from live
  positions.
- **Reusable modules**: Standard library of 24 hardware/protocol modules (LoRa,
  Wi-Fi, Ethernet, batteries, boards) importable via a `use` directive.
- **GUI debugger**: Native desktop application for config editing, live
  simulation monitoring, and trace replay with scrubbing.

## Documentation

| Document | Description |
|----------|-------------|
| [doc/INDEX.md](doc/INDEX.md) | LLM-indexable codebase map — start here for automated agents |
| [doc/architecture.md](doc/architecture.md) | Conceptual model, components, and data flow |
| [doc/config-reference.md](doc/config-reference.md) | Complete TOML configuration reference |
| [doc/simulation-files.md](doc/simulation-files.md) | FUSE filesystem interface for protocol code |
| [doc/cli-reference.md](doc/cli-reference.md) | CLI subcommands and flags |
| [doc/trace-format.md](doc/trace-format.md) | Binary trace format (.nxs) and the `parse` command |
| [doc/energy-framework.md](doc/energy-framework.md) | Per-node battery tracking and power state lifecycle |
| [doc/position-control.md](doc/position-control.md) | Mobile nodes, motion patterns, and position control files |
| [doc/modules.md](doc/modules.md) | Reusable config components and standard library |
| [doc/gui.md](doc/gui.md) | GUI debugger: config editor, live simulation, replay |
| [doc/crates.md](doc/crates.md) | Codebase structure and crate responsibilities |
| [doc/known-gaps.md](doc/known-gaps.md) | Known limitations and planned features |

## Quick Start

```toml
# nexus.toml — minimal two-node loopback example
[params]
timestep.length = 20
timestep.unit = "ms"
timestep.count = 100
seed = 42
root = "~/simulations"

[channels]
[channels.radio]
type = { type = "exclusive", read_own_writes = true }

[nodes]
[nodes.main]
deployments = [{}]

[[nodes.main.protocols]]
name = "main"
runner = "python3"
runner_args = ["loopback.py"]
publishers = ["radio"]
subscribers = ["radio"]
```

```
nexus simulate nexus.toml
```

See the [examples/](examples/) directory for 18 runnable examples covering
loopback, multihop, TDMA, mobile nodes, energy frameworks, and module-based
configurations.

## Requirements

- Linux with cgroup v2 mounted
- FUSE3 (`libfuse3`)
- Rust toolchain (for building Nexus itself)
