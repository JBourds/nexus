# Energy Framework

## Table of Contents

1. [Overview](#overview)
2. [Configuration](#configuration)
3. [Internal Representation](#internal-representation)
4. [Per-Timestep Energy Accounting](#per-timestep-energy-accounting)
5. [TX/RX Energy Costs](#txrx-energy-costs)
6. [Control Files](#control-files)
7. [Node Death and Restart](#node-death-and-restart)
8. [Battery Logging](#battery-logging)
9. [Energy Lifecycle Diagram](#energy-lifecycle-diagram)
10. [Test Coverage](#test-coverage)

---

## Overview

Real wireless sensor nodes run on batteries. A protocol that looks correct in
an infinite-energy environment can behave entirely differently when nodes run
out of power mid-operation. Messages go unanswered, routing tables become
stale, and the rest of the network must adapt. The energy framework lets
simulations model these conditions directly.

Each node can be configured with a battery capacity and a set of named power
states (e.g., "sleep", "active", "transmit"). The kernel deducts energy each
simulated timestep according to the active power state and applies named
`power_sources` and `power_sinks`- always-on flows that run regardless of
whether the node is alive (modeling a solar panel, a quiescent MCU draw, etc.).
When a node's charge reaches zero its protocol process is killed and the node
is marked dead. If a `restart_threshold` is configured, the node automatically
respawns once the battery recovers to that level; otherwise death is permanent
for the simulation run.

Individual channel operations can also carry energy costs. Configuring TX and
RX costs per channel lets you model the real power draw of radio hardware
during transmission and reception independently of background consumption.

This makes it possible to test how protocols handle:

- Nodes going offline mid-flight (messages in transit, pending ACKs, etc.)
- Nodes coming back online after a dead period
- Trade-offs between listen duty cycles and battery life
- The interaction between routing strategy and energy depletion order

---

## Configuration

Energy is opt-in per node. A node without a `charge` block has no battery and
is unaffected by the energy accounting described here.

### Battery Capacity

The `charge` block lives inside a deployment entry:

```toml
[nodes.sensor]
deployments = [{ charge = { max = 3000, quantity = 2400, unit = "mwh" } }]
```

| Field | Type | Description |
|-------|------|-------------|
| `max` | integer | Maximum battery capacity in the given unit |
| `quantity` | integer | Initial charge level (must be <= `max`) |
| `unit` | string | Energy unit (see table below) |

If `quantity` is zero the node starts dead; it will only be able to run once
a power source charges it past `restart_threshold` (if one is set).

#### Energy Units

| String | Aliases | Value |
|--------|---------|-------|
| `"nanojoule"` / `"nanojoules"` | `"nj"` | 1 nJ |
| `"microjoule"` / `"microjoules"` | `"uj"` | 10³ nJ |
| `"millijoule"` / `"millijoules"` | `"mj"` | 10⁶ nJ |
| `"joule"` / `"joules"` | `"j"` | 10⁹ nJ |
| `"kilojoule"` / `"kilojoules"` | `"kj"` | 10¹² nJ |
| `"microwatthour"` / `"microwatthours"` | `"uwh"` | 3,600 nJ |
| `"milliwatthour"` / `"milliwatthours"` | `"mwh"` | 3,600,000 nJ |
| `"watthour"` / `"watthours"` | `"wh"` | 3.6 × 10⁹ nJ |
| `"kilowatthour"` / `"kilowatthours"` | `"kwh"` | 3.6 × 10¹² nJ |

### Power States

Named power states describe the background drain rate while the node is alive.
Each state is a power rate — an amount of energy consumed per unit of time:

```toml
[nodes.sensor.power_states]
sleep    = { rate = 10,  unit = "uw", time = "s" }
transmit = { rate = 100, unit = "mw", time = "s" }
```

A node with no `current_state` (or one that starts with `initial_state`
pointing to a valid key) drains at zero if no state is active. Power state
drain is only applied while the node is alive.

#### Power Units

| String | Aliases | Value |
|--------|---------|-------|
| `"Nanowatt"` / `"nanowatt"` | `"nw"`, `"Nw"` | 1 nW |
| `"Microwatt"` / `"microwatt"` | `"uw"`, `"Uw"` | 10³ nW |
| `"Milliwatt"` / `"milliwatt"` | `"mw"` | 10⁶ nW |
| `"Watt"` / `"watt"` | `"w"` | 10⁹ nW |
| `"Kilowatt"` / `"kilowatt"` | `"kw"`, `"Kw"` | 10¹² nW |
| `"Megawatt"` / `"megawatt"` | `"Mw"` | 10¹⁵ nW |
| `"Gigawatt"` / `"gigawatt"` | `"gw"`, `"Gw"` | 10¹⁸ nW |

The `time` field uses the standard time unit strings (`"s"`, `"ms"`, `"us"`,
`"ns"`, `"m"`, `"h"`). Together, `rate`, `unit`, and `time` express a rate
like "80 mW per second" — meaning 80 mJ of energy consumed every simulated
second.

### Power Sources and Power Sinks

`power_sources` and `power_sinks` are named always-on energy flows applied
every timestep regardless of whether the node is alive or dead. Sources add
charge; sinks remove it (using `saturating_sub`). They replace the old
`ambient_rate` field.

#### Constant flows

A constant flow uses the same `{ rate, unit, time }` shape as a power state:

```toml
[nodes.sensor.power_sinks]
mcu = { rate = 80, unit = "mw", time = "s" }

[nodes.sensor.power_sources]
battery_charger = { rate = 400, unit = "mw", time = "s" }
```

#### Piecewise linear flows

A piecewise linear flow varies over simulated time according to a `schedule`
of `{ at, rate }` breakpoints. Between breakpoints the rate is linearly
interpolated. The `repeat` field makes the schedule loop; without it the last
breakpoint rate holds forever.

```toml
[nodes.sensor.power_sources.solar]
unit     = "mw"
time     = "s"
schedule = [
  { at = "0h",  rate = 0   },
  { at = "6h",  rate = 0   },
  { at = "12h", rate = 500 },
  { at = "18h", rate = 0   },
  { at = "24h", rate = 0   },
]
repeat = "24h"
```

Duration strings in `at` and `repeat` accept: `h`, `m`, `s`, `ms`, `us`.

A dead node with a source will slowly recharge. Combined with
`restart_threshold`, this models a solar-powered node that recovers
automatically after going dark.

> **Note:** Flows added at runtime via `ctl.power_flows` are always Constant;
> piecewise linear flows can only be defined in the simulation config file.

### Initial State and Restart Threshold

Both fields are optional and live inside the deployment entry alongside
`charge`:

```toml
deployments = [{
    charge             = { max = 3000, quantity = 2400, unit = "mwh" },
    initial_state      = "sleep",
    restart_threshold  = 0.05
}]
```

| Field | Type | Description |
|-------|------|-------------|
| `initial_state` | string | Which power state the node starts in. Must be a key in `power_states`. |
| `restart_threshold` | float (0–1) | Fraction of `max` charge at which a dead node restarts. Omit to make death permanent. |

`restart_threshold = 0.05` on a node with `max = 3000 mWh` means the node
restarts when its charge recovers to 150 mWh (5% of capacity).

Validation rejects `initial_state` values that do not appear in `power_states`
and `restart_threshold` values outside `[0.0, 1.0]`.

### Per-Channel Energy Costs

Channel operations can carry a one-time energy cost configured per node:

```toml
[nodes.sensor.channel_energy.lora]
tx = { quantity = 150, unit = "uj" }
rx = { quantity = 50,  unit = "uj" }
```

`tx` and `rx` are both optional. Either can be omitted if there is no cost for
that direction. Each uses the same energy unit table as `charge`. Channel
energy references are validated: the channel name must appear in either
`publishers` or `subscribers` for one of the node's protocols.

### Complete Example

```toml
# Sensor node: 3000 mWh battery, starts 80% charged, sleeps by default,
# restarts at 5%, with a solar source, a quiescent MCU sink, and explicit
# radio TX/RX costs.

[params]
timestep.length = 1
timestep.unit   = "ms"
timestep.count  = 10000
seed            = 0
root            = "~/simulations"

[channels.lora]

[nodes.sensor]
deployments = [{
    charge            = { max = 3000, quantity = 2400, unit = "mwh" },
    initial_state     = "sleep",
    restart_threshold = 0.05
}]

[nodes.sensor.power_states]
sleep    = { rate = 10,  unit = "uw", time = "s" }
transmit = { rate = 100, unit = "mw", time = "s" }

[nodes.sensor.power_sinks]
mcu = { rate = 80, unit = "mw", time = "s" }

[nodes.sensor.power_sources.solar]
unit     = "mw"
time     = "s"
schedule = [
  { at = "0h",  rate = 0   },
  { at = "6h",  rate = 0   },
  { at = "12h", rate = 500 },
  { at = "18h", rate = 0   },
  { at = "24h", rate = 0   },
]
repeat = "24h"

[nodes.sensor.channel_energy.lora]
tx = { quantity = 150, unit = "uj" }
rx = { quantity = 50,  unit = "uj" }

[[nodes.sensor.protocols]]
name        = "firmware"
runner      = "python3"
runner_args = ["sensor.py"]
publishers  = ["lora"]
subscribers = ["lora"]
```

```toml
# Gateway node: no battery (infinite power), no energy accounting.

[nodes.gateway]
deployments = [{}]

[[nodes.gateway.protocols]]
name        = "firmware"
runner      = "python3"
runner_args = ["gateway.py"]
publishers  = ["lora"]
subscribers = ["lora"]
```

---

## Internal Representation

At kernel initialization, `EnergyState::from_node` converts all config values
into a single kernel-internal struct stored alongside each node in
`ResolvedChannels`. The conversion happens once; the hot event loop works
entirely in nanojoules with integer arithmetic.

```rust
pub struct EnergyState {
    /// Current charge in nanojoules. Saturates at 0 (node dead when == 0).
    pub charge_nj: u64,
    /// Maximum capacity in nanojoules.
    pub max_nj: u64,
    /// Named always-on generation flows (applied even when dead).
    pub power_sources: Vec<(String, PowerFlowState)>,
    /// Named always-on drain flows (applied even when dead, saturating_sub).
    pub power_sinks: Vec<(String, PowerFlowState)>,
    /// Per-timestep drain in nJ for each named power state.
    pub power_states_nj: HashMap<String, u64>,
    /// Currently active power state name.
    pub current_state: Option<String>,
    /// Charge level in nJ at which a dead node is restarted.
    pub restart_threshold_nj: Option<u64>,
    /// Whether this node is currently dead.
    pub is_dead: bool,
}

pub enum PowerFlowState {
    /// Fixed nJ added/removed every timestep.
    Constant { nj_per_ts: u64 },
    /// Linearly interpolated schedule. `breakpoints` is a sorted list of
    /// `(timestamp_ns, rate_nw)` pairs. `repeat_us` is the period in
    /// microseconds (None = no repeat).
    PiecewiseLinear {
        breakpoints: Vec<(u64, u64)>,
        repeat_us: Option<u64>,
    },
}
```

`charge_nj` is an unsigned integer. All drain uses `saturating_sub` so charge
never goes below zero. Death fires when charge hits exactly 0.

### The `nj_per_timestep` Formula

Power state rates and constant power flows are stored as `PowerRate` values in
config. The conversion to nanojoules-per-timestep is:

```
nj_per_ts = rate_nw × timestep_ns / time_ns
```

Where:

- `rate_nw` = configured rate multiplied by the unit's nanowatt factor
- `timestep_ns` = timestep length in nanoseconds (from `TimestepConfig`)
- `time_ns` = the denominator time unit in nanoseconds (e.g., 10⁹ for "per second")

For example, a 100 mW drain with a 1 ms timestep:

```
rate_nw      = 100 × 1_000_000 = 100_000_000 nW
timestep_ns  = 1 × 1_000_000   = 1_000_000 ns
time_ns      = 1_000_000_000   (seconds in ns)

nj_per_ts    = 100_000_000 × 1_000_000 / 1_000_000_000 = 100_000 nJ
```

This conversion runs only once at startup for Constant flows. The per-timestep
loop never touches floating-point arithmetic or performs unit conversion.

For PiecewiseLinear flows, the interpolated rate in nW is computed at each
timestep from the breakpoint table, then converted to nJ/ts using the same
formula above.

`restart_threshold_nj` is derived as a floating-point multiplication at init
time (`threshold × max_nj`) then truncated to `u64`. This is the only
floating-point operation in the energy path (excluding piecewise interpolation).

---

## Per-Timestep Energy Accounting

Once per timestep, inside `RoutingServer::step()`, the kernel iterates over
all nodes and updates their energy state. The operations happen in a fixed
order:

```
for each node with an EnergyState:

  1. Apply all power_sources (always, even if the node is dead)
         for each (name, flow) in power_sources:
             charge_nj += flow.nj_for_current_ts(current_time)

  2. Apply all power_sinks (always, even if the node is dead)
         for each (name, flow) in power_sinks:
             charge_nj = saturating_sub(charge_nj, flow.nj_for_current_ts(current_time))

  3. Apply power state drain (only if the node is alive)
         drain = power_states_nj[current_state]  (0 if no state is active)
         charge_nj = saturating_sub(charge_nj, drain)

  4. Cap charge at maximum capacity
         charge_nj = min(charge_nj, max_nj)

  5. Detect death transition (alive → dead)
         if !was_dead && charge_nj == 0:  (saturating_sub ensures no underflow)
             is_dead = true
             push node index to newly_depleted

  6. Detect restart transition (dead → alive)
         if was_dead && restart_threshold set && charge_nj >= threshold:
             is_dead = false
             push node index to newly_recovered

  7. Emit battery snapshot to tracing log
```

Sources are applied before sinks so that a source and a sink with exactly
cancelling rates leave the charge unchanged rather than risking a
`saturating_sub` underflow that clips to zero. Both are applied before the
death check, so the check sees the fully-adjusted value. Sources and sinks are
applied before capping so a node at `max_nj` does not permanently lose
generation energy on the same step a sink drains a small amount.

After `step()`, the main event loop drains `newly_depleted` and
`newly_recovered` and calls into the `StatusServer` to freeze or unfreeze the
corresponding cgroups:

```rust
let RouterMessage::EnergyEvents { depleted, recovered } = routing_server.poll(timestep)? else {
    continue;
};
for name in depleted {
    status_server.freeze_node(name)?;
}
for name in recovered {
    if let StatusMessage::Respawned { pid_changes, .. } = status_server.respawn_node(name)? {
        if !pid_changes.is_empty() {
            routing_server.remap_pids(pid_changes)?;
        }
    }
}
```

The `RouterMessage::EnergyEvents` response is returned from every poll, even
when both lists are empty. This keeps the per-timestep communication interface
uniform.

---

## TX/RX Energy Costs

Channel energy costs are deducted at two different points in the message
lifecycle.

### TX Cost

The TX cost is deducted from the sender immediately when a protocol writes to a
channel file, before the message is queued for delivery:

```rust
// In write_channel_file():
let tx_cost_nj: u64 = node.channel_energy
    .get(&channel_handle)
    .and_then(|ce| ce.tx.as_ref())
    .map(|e| e.unit.to_nj(e.quantity))
    .unwrap_or(0);
if tx_cost_nj > 0 {
    if let Some(energy) = &mut node.energy {
        energy.charge_nj = energy.charge_nj.saturating_sub(tx_cost_nj);
    }
}
```

The cost is deducted regardless of whether the message is ultimately delivered
(e.g., if it is later dropped due to packet loss or a full mailbox). This
matches physical reality: the radio hardware draws power to transmit whether or
not the receiver hears the frame.

If a node has no `EnergyState` (no battery configured), TX costs are silently
ignored.

### RX Cost

The RX cost is deducted from the receiving node at the moment a message is
placed into that node's mailbox during delivery:

```rust
// In step(), during message delivery:
let rx_cost_nj: u64 = dst_node.channel_energy
    .get(&channel_handle)
    .and_then(|ce| ce.rx.as_ref())
    .map(|e| e.unit.to_nj(e.quantity))
    .unwrap_or(0);
if rx_cost_nj > 0 {
    if let Some(energy) = &mut node.energy {
        energy.charge_nj = energy.charge_nj.saturating_sub(rx_cost_nj);
    }
}
```

RX cost is applied after the ambient/drain accounting for the same timestep.
A node can therefore go dead due to RX cost on the same step it would have
survived on background drain alone — the death detection in the next step will
catch this.

---

## Control Files

Three control files expose energy state to running protocol processes.

### `ctl.energy_left`

**Mode:** Read-only

Returns the node's current charge as an ASCII decimal integer in nanojoules.
If the node has no battery configured, reading this file returns `0`.

```python
with open("ctl.energy_left") as f:
    charge_nj = int(f.read())
```

Because drain uses `saturating_sub`, the value will be exactly `0` when the
node is dead.

### `ctl.energy_state`

**Mode:** Read/Write

**Read:** Returns the name of the currently active power state, or an empty
string if no state is active.

```python
with open("ctl.energy_state") as f:
    state = f.read()   # e.g., "sleep" or "transmit"
```

**Write:** Requests a power state transition. The written value must exactly
match one of the names defined in `power_states` for this node. If the name is
not recognised the write is silently ignored and the current state is unchanged.

```python
with open("ctl.energy_state", "w") as f:
    f.write("transmit")   # switch to high-power transmit mode
# ... transmit on channel ...
with open("ctl.energy_state", "w") as f:
    f.write("sleep")      # return to low-power sleep
```

State transitions take effect at the start of the following timestep. Writing
an unknown state name is a no-op, not an error. This is a deliberate
robustness choice: protocol code that writes a state that was removed from the
config will not crash the simulation.

### `ctl.power_flows`

**Mode:** Read/Write

**Read:** Returns all currently active power flows, one per line. Each line has
the form `<kind> <name> <value> nj/ts`:

```
source solar 350 nj/ts
sink mcu 80 nj/ts
```

**Write:** Adds, updates, or removes a Constant flow. The write format is one
command per write:

```
source battery_charger 400 mw/s   # add/update a source
sink radio 120 mw/s               # add/update a sink
remove mcu                        # remove a flow by name (source or sink)
```

The unit string in writes uses the same `<power_unit>/<time_unit>` form as the
config file (e.g., `mw/s`, `uw/s`). Flows added dynamically are always
Constant. Piecewise linear flows defined in config appear in the read output
with their current instantaneous rate but cannot be updated through this file.

---

## Node Death and Restart

### Death

When a node's `charge_nj` reaches zero, `is_dead` is set to `true`
and the node's index is added to `newly_depleted`. At the end of the poll,
`newly_depleted` is drained and translated to node names, which are forwarded
to the `StatusServer`. The status server calls `CgroupController::freeze_node`,
which writes `"1"` to the node's `cgroup.freeze` file:

```
/sys/fs/cgroup/<nexus-root>/nodes_unlimited/<node-name>/cgroup.freeze
```

The Linux kernel then freezes all processes in that cgroup — the protocol
processes stop executing at their next scheduling point. From the protocol's
perspective, time simply stops: no reads return, no code runs, no timers fire.

### Restart (Kill + Respawn)

If `restart_threshold_nj` is set, the kernel checks on each timestep whether a
dead node's charge has recovered to or above the threshold. When it has,
`is_dead` is set to `false`, the node index goes into `newly_recovered`, and
the main loop initiates a **kill and respawn** sequence:

1. The status server unfreezes the node's cgroup and kills all its protocol
   processes, then spawns fresh instances. Each respawn produces a
   `(old_pid, new_pid)` pair.
2. The router receives the PID pairs via `RemapPids`, updates its handle
   table and `fuse_mapping`, and clears the mailboxes for affected handles
   (a real device losing power loses buffered radio frames).
3. The PID pairs are pushed to a shared queue that the FUSE filesystem drains
   lazily, migrating buffer entries from old PIDs to new PIDs.

The respawned process starts from scratch — all RAM/register state is lost,
just like a real embedded device losing power. This is more realistic than
the previous freeze/unfreeze approach, which preserved process state and
could mask bugs that would appear on real hardware.

### Permanent Death

Omitting `restart_threshold` means the node stays frozen for the remainder of
the simulation. Power sources still apply — `charge_nj` can recover and even
reach `max_nj` — but without a threshold the recovery check never fires and
`is_dead` never clears.

### State Summary

| Condition | `is_dead` | Process state | Sources/sinks applied | Power state drain |
|-----------|-----------|---------------|-----------------------|-------------------|
| Normal operation | `false` | Running | Yes | Yes |
| Charge == 0, no threshold | `true` | Frozen | Yes | No |
| Charge == 0, threshold not yet reached | `true` | Frozen | Yes | No |
| Charge >= restart_threshold | `false` | Killed + respawned | Yes | Yes |

---

## Battery Logging

Each timestep, after energy accounting, the kernel emits one `LogRecord::Battery`
entry per node that has an energy state:

```rust
pub enum LogRecord {
    Message {
        timestep: u64,
        is_publisher: bool,
        node: NodeHandle,
        channel: ChannelHandle,
        data: Vec<u8>,
    },
    Battery {
        timestep: u64,
        node: NodeHandle,
        charge_nj: u64,
    },
}
```

Battery records are written to the same binary log file as `Message` records
using `bincode` encoding via the `BinaryLogLayer` tracing subscriber. They are
interleaved with message records in timestep order.

During replay (`nexus replay`), `Battery` records are skipped entirely — the
replay source only reissues `Message` (TX) records. Battery records are purely
informational and exist for post-simulation analysis of charge curves.

The log file is located at `<root>/<run-id>/tx` alongside the RX log.

---

## Energy Lifecycle Diagram

The following diagram shows where energy is modified within a single simulated
timestep. Events on the left side happen during `step()`; events on the right
happen when a protocol process makes a filesystem call.

```
  ┌──────────────────────────────────────────────────────────┐
  │  RoutingServer::step()                                   │
  │                                                          │
  │  for each node with EnergyState:                         │
  │    charge_nj += power_sources        ← solar, charger …  │
  │    charge_nj -= power_sinks (sat.)   ← quiescent MCU …   │
  │    if alive:                                             │
  │      charge_nj -= power_states_nj    ← background drain  │
  │    charge_nj = min(charge_nj, max)   ← cap at full       │
  │    if just reached 0: mark dead      ─────────────────┐  │
  │    if dead + threshold met: mark alive ───────────────┤  │
  │    emit Battery log record                            │  │
  │                                                       │  │
  │  deliver queued messages:                             │  │
  │    for each message due this timestep:                │  │
  │      push to subscriber mailbox                       │  │
  │      charge_nj = saturating_sub(rx)   ← RX cost       │  │
  └───────────────────────────────────────────────────────┼──┘
                                                          │
  ┌───────────────────────────────────────────────────────┼──┐
  │  Kernel::run() main loop                              │  │
  │    routing_server.poll() →                            │  │
  │      EnergyEvents { depleted, recovered }  ◄──────────┘  │
  │    for each depleted node:                               │
  │      status_server.freeze_node()    ← cgroup.freeze=1    │
  │    for each recovered node:                              │
  │      status_server.respawn_node()   ← kill + respawn     │
  │      routing_server.remap_pids()    ← update PID maps    │
  └──────────────────────────────────────────────────────────┘

  ┌──────────────────────────────────────────────────────────┐
  │  Protocol process writes to channel file                 │
  │    → FUSE → RoutingServer::write_channel_file()          │
  │        charge_nj -= tx_cost_nj       ← TX cost           │
  │        queue_message(...)                                │
  └──────────────────────────────────────────────────────────┘
```

Key observations from the diagram:

- Sources and sinks run before power-state drain and before message delivery.
  A node that goes dead this timestep does not also receive a message in the
  same step.
- TX cost is deducted at write time (synchronous with the protocol's write
  syscall), not at delivery time.
- RX cost is deducted at delivery time within `step()`, not when the protocol
  reads the file. A message arriving in a dead node's mailbox still costs
  energy; the next death check will catch any resulting depletion.
- The freeze/kill cgroup writes happen in the main loop, one level above
  the routing server. There is a one-timestep lag between the depletion event
  and the actual process freeze.

---

## Test Coverage

37 tests in `kernel/src/router/energy_tests.rs` and 4 tests in `fuse/src/fs.rs`
cover the full energy accounting and PID remapping paths. Router tests operate
by constructing a `RoutingServer` directly (no FUSE filesystem, no real
processes) and calling `step()` or `write_channel_file()` directly. FUSE tests
construct a `NexusFs` and exercise buffer migration via the shared remap queue.

The tests are grouped by concern:

**Core per-timestep accounting (13):**

- `test_constant_source_generation` — constant source adds to charge each step
- `test_constant_sink_drain` — constant sink removes charge each step (saturating)
- `test_piecewise_linear_source` — piecewise source interpolates between breakpoints
- `test_piecewise_linear_repeat` — piecewise source wraps at `repeat` boundary
- `test_source_applied_when_dead` — source still adds charge when node is dead
- `test_sink_applied_when_dead` — sink still removes charge when node is dead
- `test_power_state_drain` — active state drains the correct amount
- `test_source_plus_drain` — source and power-state drain combined in the correct order
- `test_charge_capped_at_max` — overflow past max is clipped
- `test_multi_step_accumulation` — 100 steps accumulate correctly
- `test_no_current_state_no_drain` — node with no active state has zero drain
- `test_no_battery_node` — node with `energy: None` does not panic
- `test_two_nodes_independent` — two nodes with different energy states do not interfere

**Death and restart lifecycle (6):**

- `test_node_death` — charge == 0 sets `is_dead` and populates `newly_depleted`
- `test_newly_depleted_populated` — `newly_depleted` is not auto-drained by `step()`
- `test_dead_node_sources_only` — dead node receives sources/sinks but no power-state drain
- `test_node_restart_at_threshold` — node revives when charge crosses threshold
- `test_permanent_death_without_threshold` — no threshold means no restart
- `test_full_lifecycle` — alive → dead → solar charging → restart → alive again

**TX/RX cost deduction (2):**

- `test_tx_energy_deduction` — 100 µJ TX cost deducted (= 100,000 nJ); saturates to 0 if cost exceeds charge
- `test_rx_energy_deduction` — 50 µJ RX cost deducted on message delivery

**Control file state transitions (3):**

- `test_energy_state_transition` — writing `"active"` to `ctl.energy_state` changes drain rate
- `test_unknown_energy_state_ignored` — writing an unknown name leaves state unchanged
- `test_power_flows_ctl_add_and_remove` — writing to `ctl.power_flows` adds a constant flow and `remove` deletes it

**PID remapping (6):**

- `test_pid_remap_updates_handles` — `apply_pid_remaps` rewrites PID in handle table
- `test_pid_remap_rebuilds_fuse_mapping` — `fuse_mapping` keys updated to new PIDs
- `test_pid_remap_clears_mailboxes` — mailboxes for remapped handles are cleared (buffered frames lost on power loss)
- `test_pid_remap_pushes_to_shared_queue` — remap pairs pushed to the shared `pending_remaps` queue for FUSE
- `test_pid_remap_no_match` — remap with no matching PID is a no-op
- `test_pid_remap_multiple_handles` — multiple handles for the same node all get remapped

**Config conversion (`EnergyState::from_node`) (3):**

- `test_from_node_basic` — µJ capacity, mW drain, constant source and sink, 0.5 restart threshold
- `test_from_node_no_charge_returns_none` — node without `charge` block produces `None`
- `test_from_node_zero_charge_is_dead` — initial charge of 0 sets `is_dead = true`

**Unit conversion (`nj_per_timestep`) (4):**

- `test_nj_per_timestep_milliwatt_per_second` — 100 mW at 1 ms timestep = 100,000 nJ/ts
- `test_nj_per_timestep_watt_per_millisecond` — 1 W at 1 ms / ms rate = 10⁹ nJ/ts
- `test_piecewise_interpolation_midpoint` — rate interpolated correctly at the midpoint of two breakpoints
- `test_piecewise_interpolation_at_repeat_boundary` — value at exact repeat boundary wraps to schedule start

**FUSE buffer migration (4, in `fuse/src/fs.rs`):**

- `test_apply_pending_remaps_migrates_buffers` — buffer entries migrate from old PID to new PID
- `test_apply_pending_remaps_empty_queue` — empty remap queue is a no-op
- `test_apply_pending_remaps_no_matching_pid` — remap for non-existent PID leaves buffers untouched
- `test_apply_pending_remaps_multiple_channels` — all channels for a PID are migrated together
