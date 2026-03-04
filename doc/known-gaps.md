# Known Gaps and Planned Features

This document tracks implementation gaps, architectural weaknesses, and
planned features. It is intended to guide future development work and to
help contributors understand where the project stands relative to its goals.

Items are ordered roughly by development priority (see also the full
implementation plan in the project memory).

---

## 1. Energy Framework

**Priority: High** — Required for the thesis energy case studies (battery-aware
clusterhead election in Ring Routing).

### What exists

- `config::ast`: `Charge { max, quantity, unit }`, `PowerRate { rate, unit, time }`,
  `Simulation.sources/sinks` as `HashMap<*Handle, PowerRate>`,
  `Node.sinks/sources` as `HashSet<*Handle>`.
- `kernel::types::Node` carries `charge: Option<Charge>`.
- FUSE: `ctl.energy_left` (ReadOnly) and `ctl.energy_state` (ReadWrite) are
  listed in `CONTROL_FILES` but are not wired to any handler — reads return
  nothing, writes are silently ignored.

### What is missing

Three categories of power flows need to be tracked per-timestep:

1. **Fixed sources/sinks** — constant power in/out regardless of behavior
   (e.g., solar panel, standby draw). Deduct/add proportional to elapsed
   simulated time per tick.
2. **CPU operation sinks** — read `cpu.stat` (cgroup v2 exposes
   `usage_usec` cumulatively) each tick, diff from previous value, multiply
   by a configurable watts-per-Hz factor.
3. **Link tx/rx sinks** — deduct `bytes * energy_per_byte` from sender on
   write, from each receiver on delivery. Wired into `kernel/router/`.

Energy state machine. Depleted nodes (energy ≤ 0) transition to `"dead"` and
their cgroups are frozen. When/if the node exceeds its low water mark power
threshold, restart the process.

### Key design challenges

- **Thread safety**: Energy state is mutated by both the routing server
  thread (on message events) and the status server thread (on periodic
  ticks). Recommended: `Arc<RwLock<Vec<NodeDynamicState>>>` shared between
  both threads.
- **Unit normalization**: All energy must be converted to a single internal
  unit (nanojoules recommended) at config parse time.
- **Control file reads from FUSE thread**: FUSE reads happen on the FUSE
  thread; needs a query mechanism to the kernel (e.g., a new
  `FsMessage::ReadControl` variant or shared atomic state).
- **CPU accounting delta**: `usage_usec` is cumulative; must diff from the
  previous tick's value.

### Config changes needed

- Add `power` sub-table to `Link` with `tx_power_per_byte` and
  `rx_power_per_byte` fields.
- Add `cpu_power` to `Resources` (watts per Hz).
- Add `charge` to `Node` with `max`, `quantity`, and `unit` fields
  (partially done in AST but not validated or used at runtime).

---

## 2. Mobile Nodes

**Priority: High** — Required for position-dependent link quality demos.

### What exists

- `ast::Position` stores 3D coordinates + orientation, set at config parse time.
- `kernel::types::Node` holds `position: ast::Position` but never updates it.
- `fuse::CONTROL_FILES` includes `ctl.position` as ReadWrite, but no handler
  is wired up — writes are silently ignored.
- Routing table (`kernel/router/table.rs`) pre-computes all node-pair RSSI
  values at startup; this computation becomes stale as nodes move.

### What is missing

- FUSE write handler for `ctl.position`: parse the position format, emit a
  `FsMessage::SetPosition` message to the routing server.
- Routing server: maintain a `positions: Vec<Position>` array, update it on
  `SetPosition` events, and compute RSSI on-the-fly at message enqueue time
  rather than from the pre-computed table.
- FUSE read handler for `ctl.position`: return current position from the
  routing server via a query message.

### Key design challenges

- **Routing table redesign**: The current table pre-computes the full RSSI
  matrix. With mobile nodes, this must be split into: (a) a static
  publisher/subscriber graph (unchanged), and (b) per-message RSSI computed
  dynamically from current positions.
- **Coordinate system consistency**: Positions are 3D + orientation. Simpler
  protocols may only use 2D. A 2D shorthand deserves consideration.
- **Concurrent writes**: If multiple protocols on the same node write
  `ctl.position` concurrently, last-write wins is simplest; document this
  constraint.

---

## 3. Memory Limits Not Enforced

**Priority: Medium**

`Resources` in config has memory limit fields (defined in `config/src/resources.rs`)
but `runner/src/cgroups.rs` never writes `memory.max` or `memory.high` to
the cgroup. Protocol processes can use unbounded memory regardless of the
configured limit.

**Fix**: After creating each protocol's cgroup directory, write the configured
memory limit to `memory.max` if set. Requires testing that the cgroup memory
controller is enabled on the host.

---

## 4. Trace File Format Is Unstable

**Priority: Medium** — Blocks GUI development and long-term replay
compatibility.

### Current state

- `kernel/log.rs` writes TX and RX events as separate binary files using
  `bincode` with no version header.
- No index file, so seeking by timestamp requires a full scan.
- TX and RX logs are separate files.

### Needed

A single, versioned trace format with:

- Magic bytes + version header.
- A `TraceHeader` containing config hash, start time, timestep size, node/channel name lists.
- `TraceRecord { timestep, event }` where `event` covers:
  `MessageSent`, `MessageRecv`, `MessageDropped`, `NodeDied`,
  `PositionUpdate`, `EnergyUpdate`.
- A separate `.idx` file with byte offsets per timestep boundary, enabling
  O(1) seek for GUI scrubbing.

Until this is stabilized, the `replay` command and any future GUI tooling
cannot be built against a stable contract.

---

## 5. No Automated Testing

**Priority: Medium** — The codebase has no tests; regressions are caught
only by running examples manually.

### Needed (in order of impact)

1. **Config unit tests** — Round-trip parse/serialize, signal model
   invariants (RSSI decreases with distance), delay calculation edge cases,
   power unit normalization.
2. **Router mock tests** — Drive the routing server with raw `mpsc` channel
   pairs (no FUSE mount). Test: message timing, TTL expiry, 100% packet
   loss, shared-channel collision, exclusive buffer limits, replay matching
   live run.
3. **Cgroup mockability** — Inject `root: PathBuf` into `CgroupController`
   and use `tempfile::tempdir()` in tests to avoid requiring root.
4. **Energy accounting tests** — Once the energy module exists: linear drain,
   source replenishment cap, freeze-on-death, CPU accounting delta.
5. **End-to-end `#[ignore]` tests** — Require cgroup v2 + FUSE; run a
   0-timestep simulation, verify premature exit detection. Gated for CI.

### CI

No `.github/workflows/` exists. Should add:

- `cargo test` (unprivileged, `#[ignore]` skipped)
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- Separate privileged job for integration tests.

---

## 6. Fuzz Mode Is a Skeleton

**Priority: Low** — The `fuzz` concept is mentioned in design notes but
has no implementation beyond a placeholder.

The original design intended a fuzz mode where the simulator would inject
adversarial timing and message reordering to find protocol bugs. This is
not yet designed in detail or implemented.

---

## 7. No Precanned Link Presets

**Priority: Low**

Every simulation must define link parameters from scratch. Common wireless
standards (LoRa SF7–SF12, 802.11, Bluetooth LE) require looking up and
transcribing physical layer parameters.

**Proposed**: A `preset = "lora_sf7"` field on `[channels.X]` that expands
to a full link definition. Individual fields can still be overridden.
Presets would be Rust constants or a built-in TOML file compiled via
`include_str!()`. Versioning presets is necessary for config reproducibility.

---

## 8. No Web GUI

**Priority: Low** — Useful for demos and thesis figures.

No visualization exists. The proposed design (in implementation-plan.md) is
a Leptos-based web app with:

- Live mode: tail a running simulation's trace file via WebSocket.
- Replay mode: load a complete trace, scrub through time.
- 2D canvas with nodes, animated message arcs, energy bars.
- Config editor (drag nodes → generate TOML).

This requires the trace file format to be finalized first (item 4).

---

## 9. Environment Simulation Not Planned

**Priority: Low**

Signal attenuation currently uses only distance (Friis/RLGC). There is no
support for terrain, material obstructions, or non-line-of-sight effects.

**Proposed**: An optional heightmap + material layer. For each sender→receiver
pair with a link-of-sight question, ray-trace through the heightmap and
accumulate dB attenuation from material types (concrete, foliage, water,
metal). This additional loss term is passed into the RSSI calculation.

This is a large feature with N² ray trace costs per timestep. Spatial
caching and bounding-box culling would be required for performance.

---

## 10. Routing Table Must Be Reworked for Mobile Nodes

**Priority: Depends on item 2**

The current `kernel/router/table.rs` pre-computes a full RSSI matrix at
startup. This design assumption pervades the routing server. Once mobile
nodes are supported, RSSI must be computed at message-enqueue time using
current positions. The table should be split into:

- **Static**: subscriber/publisher relationships (unchanged across the run).
- **Dynamic**: per-message RSSI from current position snapshot.

This rework also enables energy-based link quality modulation in the future
(e.g., adjusting tx power and recomputing RSSI).

---

## 11. Case Studies Incomplete

The two thesis case studies are partially implemented.

### Ring Routing (Arduino/ATMega2560 + LoRa)

Code at `/home/jordan/repos/ciroh/UVM-NRT-RoS/embedded_projects/aura/projects/simulated_network`.

| Phase | Status |
|-------|--------|
| TDMA link layer | Done |
| Physical layer stub (channel file I/O) | Done |
| Clusterhead Announcement | Done |
| Neighbor Discovery (heartbeat) | Unclear |
| Clusterhead Joining (slot assignment ACK) | Unclear |
| Follower/Clusterhead data phases | Unclear |

End-to-end test in Nexus with 5+ nodes needed; hardware validation
(ATMega2560 flash) needed for thesis.

### LoRaMesher (ESP32 + FreeRTOS)

- FreeRTOS POSIX port needed (available upstream).
- RadioLib send/receive must be replaced with channel file reads/writes.
- Multi-hop packet delivery verification in Nexus needed.
- Hardware validation (ESP32 flash) as stretch goal.

FreeRTOS task priorities may interact adversely with cgroup CPU limits;
may require tuning time dilation or disabling DVFS.

---

## Summary Table

| Gap | Priority | Blocks |
|-----|----------|--------|
| Energy framework | High | Ring Routing case study, thesis |
| Mobile nodes | High | Thesis position demos |
| Memory limits enforcement | Medium | Resource accuracy |
| Stable trace format | Medium | GUI, replay long-term |
| Automated tests + CI | Medium | Developer confidence |
| Fuzz mode | Low | — |
| Precanned link presets | Low | Ease of use |
| Web GUI | Low | Requires stable trace format |
| Environment simulation | Low | — |
| Routing table rework | Depends on mobile nodes | Mobile nodes |
| Ring Routing completion | High | Thesis |
| LoRaMesher port | Medium | Thesis (secondary) |
