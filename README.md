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

## Documentation

| Document | Description |
|----------|-------------|
| [doc/architecture.md](doc/architecture.md) | Conceptual model, components, and data flow |
| [doc/config-reference.md](doc/config-reference.md) | Complete TOML configuration reference |
| [doc/simulation-files.md](doc/simulation-files.md) | FUSE filesystem interface for protocol code |
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

See the [examples/](examples/) directory for runnable examples.

## Requirements

- Linux with cgroup v2 mounted
- FUSE3 (`libfuse3`)
- Rust toolchain (for building Nexus itself)
