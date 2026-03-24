# Configuration Reference

Nexus simulations are defined in TOML. The top-level sections are:

- `use` — module imports (optional)
- `[params]` — simulation-wide settings
- `[links]` — reusable link definitions
- `[channels]` — named communication channels
- `[nodes]` — node class definitions

**Source files:** `config/src/ast.rs` (type definitions), `config/src/parse.rs`
(deserialization), `config/src/validate.rs` (validation).

---

## `use` — Module Imports

```toml
use = [
    "lora/sx1276_915mhz",            # standard library module
    "boards/esp32_devkit",            # board profile
    "batteries/cr2032",              # battery profile
    "./my_modules/custom_link",       # user-defined, relative path
]
```

Modules contribute links, channels, and node profiles to the simulation.
See [modules.md](modules.md) for full documentation.

---

## `[params]`

```toml
[params]
timestep.length = 1          # integer, quantity per step (default: 10)
timestep.unit   = "ms"       # time unit string (see Units section)
timestep.count  = 100000     # total number of timesteps to simulate

seed            = 42         # random seed for reproducibility
root            = "~/simulations"  # directory where simulation output is written
time_dilation   = 1.0        # speed scaling factor (default: 1.0)
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `timestep.length` | integer | 10 | Duration per timestep in the configured unit |
| `timestep.unit` | string | — | Time unit (see Units section) |
| `timestep.count` | integer | — | Total number of timesteps to simulate |
| `seed` | integer | — | Random seed for reproducibility |
| `root` | string | — | Output directory (each run creates a new subdirectory) |
| `time_dilation` | float | 1.0 | Speed scaling factor applied to CPU throttling. Values > 1.0 make simulated time pass faster relative to wall clock. |

The timestep unit should be chosen to match the finest time granularity that
matters for the protocols under test. For a 1 ms MAC timer, `unit = "ms"` and
`length = 1` is a natural choice. Coarser timesteps run faster but reduce
timing resolution.

---

## `[links]`

Links define the physical properties of a communication medium. Channels
reference links by name. The implicit `"ideal"` link exists without being
declared: zero delay, zero error, infinite range. This is the default link
which other definitions inherit properties from. An explicit ancestor can be
specified as well.

```toml
[links.my_link]
inherit = "parent_link"      # optional: inherit properties from another link

# Signal model — controls range and RSSI computation.
# Wireless (Friis free-space path loss):
[links.my_link.medium]
type           = "wireless"
shape          = "omni"          # "omni" (default), "cone", or "direct"
wavelength_m   = 0.346           # wavelength in meters (speed_of_light / frequency)
gain_dbi       = 2.15            # antenna gain in dBi
rx_min_dbm     = -120.0          # minimum receivable signal in dBm
tx_min_dbm     = -10.0           # minimum transmit power in dBm
tx_max_dbm     = 20.0            # maximum transmit power in dBm

# Wired (RLGC transmission line model):
[links.my_link.medium]
type     = "wired"
r        = 0.02     # resistance per meter (Ω/m)
l        = 250e-9   # inductance per meter (H/m)
c        = 100e-12  # capacitance per meter (F/m)
g        = 0.0      # conductance per meter (S/m)
f        = 1e6      # frequency (Hz)
rx_min_dbm = -30.0
tx_min_dbm = -10.0
tx_max_dbm = 10.0

# Bit error rate — probability of a bit being flipped.
# Rate is an expression that can reference the variable `rssi`.
[links.my_link.bit_error]
rate     = "0.01"       # constant: 1% BER
# or:
rate     = "max(0, -rssi / 1000)"  # RSSI-dependent expression

# Packet loss — probability of the entire packet being dropped.
[links.my_link.packet_loss]
rate = "0.0"            # constant 0% loss

# Delays
[links.my_link.delays.transmission]
# Transmission delay: time to put all bits on the wire.
rate = 250              # e.g., 250 Kb/s
data = "Kb"
time = "s"

[links.my_link.delays.processing]
# Processing delay at the receiver.
rate = 0                # 0 means no processing delay
data = "b"
time = "s"

[links.my_link.delays.propagation]
# Propagation delay — time for signal to travel through the medium.
rate     = 3e8          # speed of light in m/s for wireless
distance = "m"
time     = "s"
```

### Link Inheritance

Links can inherit from other links using the `inherit` field. The child link
only needs to specify the fields it wants to override; all other fields are
taken from the parent.

```toml
[links.base_lora]
# ... full LoRa link definition ...

[links.noisy_lora]
inherit = "base_lora"
bit_error.rate = "0.05"    # override just the bit error rate
```

### Signal Shapes

| Shape | Description |
|-------|-------------|
| `"omni"` | Omnidirectional broadcast in all directions (default) |
| `"cone"` | Directional cone — signal attenuated outside beam angle |
| `"direct"` | Point-to-point only |

### Error Rate Expressions

`bit_error.rate` and `packet_loss.rate` are string expressions evaluated at
message delivery time. Available variables:

| Variable | Description |
|----------|-------------|
| `rssi` | Received signal strength in dBm (computed from distance + medium model) |

A rate of `"0"` is zero; `"1"` is 100%. Values are clamped within this range.
This supports all string expressions implemented by the Rust [meval](https://docs.rs/meval/latest/meval/)
crate (including `max`, `min`, `exp`, `log`, `sin`, `cos`, etc.).

---

## `[channels]`

```toml
[channels.my_channel]
link = "my_link"       # link definition to use (default: "ideal")

# Channel type (optional — defaults to exclusive):
type = { type = "exclusive" }
# or with options:
type = { type = "exclusive", ttl = 500, unit = "ms", read_own_writes = false, max_size = 255 }
# or shared:
type = { type = "shared", ttl = 100, unit = "ms", read_own_writes = true }
```

### Channel Types

**`exclusive`** (default): Each subscriber gets its own independent FIFO
queue. Messages are never dropped due to collision. Models point-to-point
links, TCP connections, or any channel where each recipient gets its own copy.

**`shared`**: Models a true broadcast medium. When multiple publishers
transmit within the same timestep, all transmissions collide and are dropped
(OR-collision semantics). Protocols sharing this channel must implement MAC
(e.g., TDMA, CSMA). Models LoRa, 802.11, or any shared-medium radio.

| Option | Default | Description |
|--------|---------|-------------|
| `ttl` | none (no expiry) | Message lifetime; messages older than this are dropped |
| `unit` | — | Time unit for `ttl` |
| `read_own_writes` | `false` | Whether a node receives its own transmissions |
| `max_size` | none | Maximum message size in bytes (writes exceeding this are truncated) |
| `nbuffered` | none | For exclusive channels: max queue depth per subscriber (none = unlimited) |

### Channel Filesystem Interface

Each channel appears as a directory in the FUSE filesystem with three
sub-files: `channel` (read/write data), `rssi` (read signal strength), and
`snr` (read signal-to-noise ratio). See
[simulation-files.md](simulation-files.md) for details.

---

## `[nodes]`

Node sections define *classes* of nodes. Multiple instances of a class can
be deployed using the `deployments` array.

```toml
[nodes.my_node]
profile = "esp32"                  # optional: apply a module profile (or list of profiles)
deployments = [
    { position = { point = [0, 0, 0], unit = "m" } },
    { position = { point = [1000, 0, 0], unit = "m" }, run_args = ["--id=2"] },
]
internal_names = ["internal_bus"]  # channel names visible only within this node
start = "2024-01-01T00:00:00Z"    # optional: real-world start time (ISO 8601)
```

### `profile`

References one or more node profiles imported via the module system. Profiles
provide reusable hardware characteristics (resources, power states, sinks,
sources).

```toml
profile = "esp32"                    # single profile
profile = ["esp32", "solar_small"]   # multiple profiles, applied in order
```

See [modules.md](modules.md) for profile documentation.

### `deployments`

Each entry in `deployments` creates one running instance of this node class.
Fields per deployment:

| Field | Description |
|-------|-------------|
| `position` | 3D position (see Position section) |
| `run_args` | Extra command-line arguments passed to each protocol process |
| `charge` | Battery configuration (see Energy section) |
| `initial_state` | Starting power state name |
| `restart_threshold` | Fraction of max charge at which a dead node restarts (0.0–1.0) |

If `deployments` is omitted or `[{}]`, one instance is created with default
position (origin).

### Position

```toml
# Simple array form:
position = { point = [x, y, z], unit = "m" }

# With orientation:
position = { point = [x, y, z], orientation = [az, el, roll], unit = "m" }

# Object form:
position = { point = { x = 0, y = 0, z = 0 }, unit = "meters" }
```

Position defaults to the origin with all angles zero. Units apply to the
`point` coordinates. Orientation angles are always in degrees.

### `internal_names`

A list of channel names that are local to this node. Internal channels behave
like `exclusive` channels but are invisible to other nodes. Use them for
inter-process communication within a multi-protocol node.

```toml
internal_names = ["ipc_bus", "sensor_data"]
```

### `[nodes.X.resources]`

```toml
[nodes.my_node.resources]
clock_rate  = 16        # simulated CPU clock rate
clock_units = "Mhz"     # "Hz", "Khz", "Mhz", "Ghz"
# memory limits: defined in config types but not yet enforced (see known-gaps.md)
```

When `clock_rate` is set, Nexus calculates the ratio of simulated clock speed
to the host CPU's measured frequency and applies it as a cgroup v2 `cpu.max`
bandwidth limit. The `time_dilation` parameter further scales this ratio.

### `[nodes.X.power_states]`

Named power consumption states. See [energy-framework.md](energy-framework.md).

```toml
[nodes.sensor.power_states]
sleep    = { rate = 10,  unit = "uw", time = "s" }
active   = { rate = 80,  unit = "mw", time = "s" }
transmit = { rate = 100, unit = "mw", time = "s" }
```

### `[nodes.X.power_sources]` / `[nodes.X.power_sinks]`

Always-on energy flows. Sources add charge, sinks remove it. Applied every
timestep regardless of whether the node is alive.

```toml
# Constant flow
[nodes.sensor.power_sinks]
mcu = { rate = 80, unit = "mw", time = "s" }

# Piecewise linear flow (varies over time)
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

### `[nodes.X.channel_energy]`

Per-channel TX/RX energy costs. See [energy-framework.md](energy-framework.md).

```toml
[nodes.sensor.channel_energy.lora]
tx = { quantity = 150, unit = "uj" }
rx = { quantity = 50,  unit = "uj" }
```

### `[[nodes.X.protocols]]`

Each node can have one or more protocol entries. Each protocol becomes one
OS process.

```toml
[[nodes.my_node.protocols]]
name        = "main"            # protocol name (used for logging)
root        = "."               # working directory for build + run (default: ".")

# Build step (optional):
build       = "make"            # build command
build_args  = ["build"]         # arguments to build command

# Run step (required):
runner      = "python3"         # executable or interpreter
runner_args = ["main.py"]       # arguments to runner

# Channel subscriptions:
publishers  = ["channel_a"]     # channels this protocol can write to
subscribers = ["channel_b"]     # channels this protocol receives from
```

A protocol can both publish and subscribe to the same channel:

```toml
publishers  = ["radio"]
subscribers = ["radio"]
```

If `build` is present, Nexus runs the build step before starting the
simulation and will not proceed if the build fails.

---

## Units Reference

### Time Units

| String | Aliases | Value |
|--------|---------|-------|
| `"hours"` | `"h"` | 3600 s |
| `"minutes"` | `"m"` | 60 s |
| `"seconds"` | `"s"` | 1 s |
| `"milliseconds"` | `"ms"` | 10⁻³ s |
| `"microseconds"` | `"us"` | 10⁻⁶ s |
| `"nanoseconds"` | `"ns"` | 10⁻⁹ s |

### Data Units

| String | Aliases | Value |
|--------|---------|-------|
| `"bit"` | `"b"` | 1 bit |
| `"kilobit"` | `"Kb"` | 1000 bits |
| `"megabit"` | `"Mb"` | 10⁶ bits |
| `"gigabit"` | `"Gb"` | 10⁹ bits |
| `"byte"` | `"B"` | 8 bits |
| `"kilobyte"` | `"KB"` | 8000 bits |
| `"megabyte"` | `"MB"` | 8×10⁶ bits |
| `"gigabyte"` | `"GB"` | 8×10⁹ bits |

### Distance Units

| String | Aliases |
|--------|---------|
| `"meters"` | `"m"` |
| `"kilometers"` | `"km"` |
| `"feet"` | — |
| `"yards"` | — |
| `"miles"` | `"mi"` |

### Clock Rate Units

| String |
|--------|
| `"Hz"` |
| `"Khz"` |
| `"Mhz"` |
| `"Ghz"` |

### Energy Units

| String | Aliases | Value |
|--------|---------|-------|
| `"nanojoule"` | `"nj"` | 1 nJ |
| `"microjoule"` | `"uj"` | 10³ nJ |
| `"millijoule"` | `"mj"` | 10⁶ nJ |
| `"joule"` | `"j"` | 10⁹ nJ |
| `"kilojoule"` | `"kj"` | 10¹² nJ |
| `"microwatthour"` | `"uwh"` | 3,600 nJ |
| `"milliwatthour"` | `"mwh"` | 3,600,000 nJ |
| `"watthour"` | `"wh"` | 3.6 × 10⁹ nJ |
| `"kilowatthour"` | `"kwh"` | 3.6 × 10¹² nJ |

### Power Units

| String | Aliases | Value |
|--------|---------|-------|
| `"nanowatt"` | `"nw"` | 1 nW |
| `"microwatt"` | `"uw"` | 10³ nW |
| `"milliwatt"` | `"mw"` | 10⁶ nW |
| `"watt"` | `"w"` | 10⁹ nW |
| `"kilowatt"` | `"kw"` | 10¹² nW |

---

## Complete Example

```toml
# Three-node chain: client → proxy → server with an unreliable wireless link.

use = ["lora/sx1276_915mhz", "boards/esp32_devkit", "batteries/cr2032"]

[params]
timestep.length = 10
timestep.unit   = "ms"
timestep.count  = 5000
seed            = 42
root            = "~/simulations"
time_dilation   = 1.0

[channels]

[channels.uplink]
link = "lora_915"
type = { type = "exclusive", ttl = 500, unit = "ms", max_size = 255 }

[channels.downlink]
link = "lora_915"
type = { type = "exclusive", ttl = 500, unit = "ms", max_size = 255 }

[nodes]

[nodes.sensor]
profile = ["esp32", "cr2032"]
deployments = [{
    position          = { point = [0, 0, 0], unit = "m" },
    charge            = { max = 675, quantity = 675, unit = "mwh" },
    initial_state     = "sleep",
    restart_threshold = 0.05,
}]

[nodes.sensor.power_states]
sleep    = { rate = 10,  unit = "uw", time = "s" }
transmit = { rate = 100, unit = "mw", time = "s" }

[nodes.sensor.channel_energy.uplink]
tx = { quantity = 150, unit = "uj" }

[[nodes.sensor.protocols]]
name        = "app"
runner      = "python3"
runner_args = ["sensor.py"]
publishers  = ["uplink"]
subscribers = ["downlink"]

[nodes.gateway]
profile = "esp32"
deployments = [{ position = { point = [500, 0, 0], unit = "m" } }]

[[nodes.gateway.protocols]]
name        = "gw"
runner      = "python3"
runner_args = ["gateway.py"]
publishers  = ["downlink"]
subscribers = ["uplink"]
```
