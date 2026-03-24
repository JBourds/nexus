# Known Gaps and Planned Features

This document tracks implementation gaps, architectural weaknesses, and
planned features. It is intended to guide future development work and to
help contributors understand where the project stands relative to its goals.

Items are ordered roughly by development priority.

---

## 1. Energy Framework

**Status: Implemented.** See [energy-framework.md](energy-framework.md).

Per-node battery tracking with named power states, power sources/sinks
(constant and piecewise-linear), per-channel TX/RX costs, death via cgroup
freezer, kill-and-respawn restart, and configurable restart threshold. All
accounting in integer nanojoules. 37 kernel tests + 4 FUSE buffer migration
tests.

### Remaining gaps

- **CPU-proportional drain** — `cpu.stat` usage_usec tracking not
  implemented. Energy drain is currently config-driven (power states), not
  measured from actual CPU usage.
- **Per-byte TX/RX costs** — channel energy is a flat per-message cost, not
  proportional to message size.

---

## 2. Mobile Nodes

**Status: Implemented.** See [position-control.md](position-control.md).

Full position control with 10 control files under `ctl.pos/` (x, y, z, az,
el, roll, dx, dy, dz, motion). Four motion patterns (Static, Velocity, Linear,
Circle). Dynamic RSSI computation from live positions at message queue and
delivery time. Position and motion events logged to `.nxs` trace files.
33 kernel tests for motion patterns and position control.

### Remaining gaps

- **2D shorthand** — no convenience for 2D-only simulations; Z must always
  be specified (defaults to 0).
- **Terrain/obstacles** — position affects only free-space distance; no
  support for line-of-sight obstructions.

---

## 3. Trace Format

**Status: Implemented.** See [trace-format.md](trace-format.md).

The `.nxs` binary trace format includes a header (node names, channel names,
timestep count, max energy per node) and six event types (MessageSent,
MessageRecv, MessageDropped, PositionUpdate, EnergyUpdate, MotionUpdate).
The `nexus parse` command supports filtering by event type, node, channel,
and timestep range, with text/JSON/JSON Lines output and external adapter
support.

### Remaining gaps

- **No version header** — the format has no magic bytes or version field,
  making forward/backward compatibility fragile.
- **No index file** — seeking by timestamp requires a full scan; an `.idx`
  file with byte offsets per timestep boundary would enable O(1) GUI scrubbing.

---

## 4. Module System

**Status: Implemented.** See [modules.md](modules.md).

24 standard library modules covering batteries, boards, energy harvesters,
LoRa, Wi-Fi, and wired connections. `use` directive for imports, node profiles
for reusable hardware templates, multi-profile layering with merge semantics.
Three CLI subcommands (`modules list`, `modules show`, `modules verify`).

---

## 5. GUI

**Status: Implemented.** See [gui.md](gui.md).

Native desktop application (egui/eframe) with four modes: Home, Config Editor,
Live Simulation, and Replay. Config editor with form-based TOML editing and
module browser. Live simulation with real-time event streaming, grid
visualization, and speed control. Replay with timestep scrubbing and state
reconstruction from `.nxs` trace files.

### Remaining gaps

- **Web GUI** — a browser-based version would be useful for demos and remote
  access. Would require the trace format to be fully stabilized (version
  header + index).

---

## 6. Memory Limits Not Enforced

**Priority: Medium**

`Resources` in config has memory limit fields (defined in `config/src/resources.rs`)
but `runner/src/cgroups.rs` never writes `memory.max` or `memory.high` to
the cgroup. Protocol processes can use unbounded memory regardless of the
configured limit.

**Fix**: After creating each protocol's cgroup directory, write the configured
memory limit to `memory.max` if set. Requires testing that the cgroup memory
controller is enabled on the host.

---

## 7. Test Coverage Incomplete

**Priority: Medium** — Tests exist for config parsing/validation (21 tests),
energy accounting (37 tests), position control (33 tests), and FUSE buffer
migration (4 tests). Gaps remain.

### What exists

1. **Config unit tests** — Accept/reject fixture tests for TOML parsing and
   validation, including energy and position config.
2. **Energy accounting tests** — 37 tests driving `RoutingServer` directly
   via `mpsc` channels (no FUSE). Covers drain, ambient, death/restart,
   TX/RX costs, control files, unit conversion, PID remapping.
3. **Position control tests** — 33 tests for motion patterns, position
   control file parsing, and log round-trips.
4. **FUSE tests** — 4 tests for buffer migration during PID remapping.

### Still needed

1. **Router mock tests** — Message timing, TTL expiry, 100% packet loss,
   shared-channel collision, exclusive buffer limits, replay matching live run.
2. **Cgroup mockability** — Inject `root: PathBuf` into `CgroupController`
   and use `tempfile::tempdir()` in tests to avoid requiring root.
3. **End-to-end `#[ignore]` tests** — Require cgroup v2 + FUSE; run a
   0-timestep simulation, verify premature exit detection. Gated for CI.

### CI

No `.github/workflows/` exists. Should add:

- `cargo test` (unprivileged, `#[ignore]` skipped)
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- Separate privileged job for integration tests.

---

## 8. Fuzz Mode Is a Skeleton

**Priority: Low** — The `fuzz` subcommand exists in the CLI but has no
implementation beyond a `todo!()` placeholder.

The original design intended a fuzz mode where the simulator would inject
adversarial timing and message reordering to find protocol bugs. This is
not yet designed in detail or implemented.

---

## 9. Environment Simulation Not Planned

**Priority: Low**

Signal attenuation currently uses only distance (Friis/RLGC). There is no
support for terrain, material obstructions, or non-line-of-sight effects.

**Proposed**: An optional heightmap + material layer. For each sender→receiver
pair with a line-of-sight question, ray-trace through the heightmap and
accumulate dB attenuation from material types (concrete, foliage, water,
metal). This additional loss term is passed into the RSSI calculation.

---

## Summary Table

| Gap | Priority | Status |
|-----|----------|--------|
| ~~Energy framework~~ | ~~High~~ | Implemented |
| ~~Mobile nodes~~ | ~~High~~ | Implemented |
| ~~Trace format~~ | ~~Medium~~ | Implemented (no version header or index) |
| ~~Module system~~ | ~~High~~ | Implemented |
| ~~GUI~~ | ~~Medium~~ | Implemented (desktop; no web) |
| CPU-proportional energy drain | Medium | Not implemented |
| Per-byte TX/RX costs | Medium | Not implemented |
| Memory limits enforcement | Medium | Not implemented |
| Test coverage gaps + CI | Medium | Partial |
| Fuzz mode | Low | Skeleton only |
| Environment simulation | Low | Not planned |
| Trace version header + index | Medium | Not implemented |
| Web GUI | Low | Not planned |
