# Configuration Reference

Nexus simulations are defined in TOML. The top-level sections are:

- `[params]` — simulation-wide settings
- `[links]` — reusable link definitions
- `[channels]` — named communication channels
- `[nodes]` — node class definitions

## `[params]`

```toml
[params]
timestep.length = 1          # integer, quantity per step (default: 10)
timestep.unit   = "ms"       # time unit string (see Units section)
timestep.count  = 100000     # total number of timesteps to simulate

seed = 42                    # random seed for reproducibility
root = "~/simulations"       # directory where simulation output is written
                             # (each run creates a new subdirectory)
```

The timestep unit should be chosen to match the finest time granularity that
matters for the protocols under test. For a 1 ms MAC timer, `unit = "ms"` and
`length = 1` is a natural choice. Coarser timesteps run faster but reduce
timing resolution.

## `[links]`

Links define the physical properties of a communication medium. Channels
reference links by name. The implicit `"ideal"` link exists without being
declared: zero delay, zero error, infinite range. This is the default link
which other definitions inherit properties from. An explicit ancestor can be
specified as well.

```toml
[links.my_link]

# Signal model — controls range and RSSI computation.
# Wireless (Friis free-space path loss):
[links.my_link.medium]
type           = "wireless"
shape          = "omni"          # "omni" (default) — broadcast in all directions
                                 # "direct" — point-to-point only
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
# Expressed as a data rate (bits per unit time).
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
# Expressed as a rate (distance per unit time = speed of signal).
rate     = 3e8          # speed of light in m/s for wireless
distance = "m"
time     = "s"
```

### Error Rate Expressions

`bit_error.rate` and `packet_loss.rate` are string expressions evaluated at
message delivery time. Available variables:

| Variable | Description |
|----------|-------------|
| `rssi` | Received signal strength in dBm (computed from distance + medium model) |

A rate of `"0"` is zero; `"1"` is 100%. Values are clamped within this range.
This supports all string expressions implemented by the Rust [meval](https://docs.rs/meval/latest/meval/)
crate.

## `[channels]`

```toml
[channels.my_channel]
link = "my_link"       # link definition to use (default: "ideal")

# Channel type (optional — defaults to exclusive):
type = { type = "exclusive" }
# or:
type = { type = "exclusive", ttl = 500, unit = "ms", read_own_writes = false }
# or:
type = { type = "shared",    ttl = 100, unit = "ms", read_own_writes = true  }
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

## `[nodes]`

Node sections define *classes* of nodes. Multiple instances of a class can
be deployed using the `deployments` array.

```toml
[nodes.my_node]
deployments = [
    { position = { point = [0, 0, 0], unit = "m" } },
    { position = { point = [1000, 0, 0], unit = "m" }, run_args = ["--id=2"] },
]
internal_names = ["internal_bus"]   # channel names visible only within this node
start = "2024-01-01T00:00:00Z"      # optional: real-world start time (ISO 8601)
```

### `deployments`

Each entry in `deployments` creates one running instance of this node class.
Fields per deployment:

| Field | Description |
|-------|-------------|
| `position` | 3D position (see Position section) |
| `run_args` | Extra command-line arguments passed to each protocol process |

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
bandwidth limit. This causes the protocol to experience the same number of
CPU cycles per simulated second as it would on the target hardware.

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

# Optional: extra args passed only at runtime (not build)
# run_args from deployment are appended here

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

## Complete Example

```toml
# Three-node chain: client → proxy → server with an unreliable wireless link.

[params]
timestep.length = 10
timestep.unit   = "ms"
timestep.count  = 5000
seed            = 42
root            = "~/simulations"

[links]

[links.lora_sf7]

[links.lora_sf7.medium]
type          = "wireless"
shape         = "omni"
wavelength_m  = 0.346
gain_dbi      = 2.15
rx_min_dbm    = -137.0
tx_min_dbm    = 2.0
tx_max_dbm    = 20.0

[links.lora_sf7.bit_error]
rate = "0.0"

[links.lora_sf7.packet_loss]
rate = "max(0.0, (rssi + 137.0) / 137.0)"

[links.lora_sf7.delays.transmission]
rate = 5.47       # 5.47 kb/s at SF7 250 kHz BW
data = "Kb"
time = "s"

[links.lora_sf7.delays.propagation]
rate     = 3e8
distance = "m"
time     = "s"

[channels]

[channels.uplink]
link = "lora_sf7"
type = { type = "exclusive", ttl = 500, unit = "ms" }

[channels.downlink]
link = "lora_sf7"
type = { type = "exclusive", ttl = 500, unit = "ms" }

[nodes]

[nodes.sensor]
deployments = [{ position = { point = [0, 0, 0], unit = "m" } }]

[nodes.sensor.resources]
clock_rate  = 8
clock_units = "Mhz"

[[nodes.sensor.protocols]]
name        = "app"
runner      = "python3"
runner_args = ["sensor.py"]
publishers  = ["uplink"]
subscribers = ["downlink"]

[nodes.gateway]
deployments = [{ position = { point = [500, 0, 0], unit = "m" } }]

[[nodes.gateway.protocols]]
name        = "gw"
runner      = "python3"
runner_args = ["gateway.py"]
publishers  = ["downlink"]
subscribers = ["uplink"]
```
