# Control and Channel Files

When Nexus mounts a simulation, each protocol process sees a directory
containing channel directories and control files. These are the primary
interface between protocol code and the simulator.

## Filesystem Layout

Each protocol's working directory (its `root`) is the mount point. The exact
files present depend on which channels the protocol subscribes/publishes to,
but the layout looks like:

```
<node-name>/
├── <channel>/                # directory per channel the protocol uses
│   ├── channel               # read/write: message data (TX/RX)
│   ├── rssi                  # read-only: RSSI in dBm from last received message
│   └── snr                   # read-only: SNR in dB from last received message
├── ctl.time/                 # directory: simulated time control
│   ├── s                     # read/write: time in seconds
│   ├── ms                    # read/write: time in milliseconds
│   ├── us                    # read/write: time in microseconds
│   └── ns                    # read/write: time in nanoseconds
├── ctl.elapsed/              # directory: elapsed simulated time (read-only)
│   ├── s                     # read-only: elapsed seconds
│   ├── ms                    # read-only: elapsed milliseconds
│   ├── us                    # read-only: elapsed microseconds
│   └── ns                    # read-only: elapsed nanoseconds
├── ctl.pos/                  # directory: node position control
│   ├── x                     # read/write: X coordinate
│   ├── y                     # read/write: Y coordinate
│   ├── z                     # read/write: Z coordinate
│   ├── az                    # read/write: azimuth (yaw) in degrees
│   ├── el                    # read/write: elevation (pitch) in degrees
│   ├── roll                  # read/write: roll in degrees
│   ├── dx                    # write-only: relative X delta
│   ├── dy                    # write-only: relative Y delta
│   ├── dz                    # write-only: relative Z delta
│   └── motion                # read/write: motion pattern spec
├── ctl.energy_left           # read-only: remaining charge in nanojoules
├── ctl.energy_state          # read/write: current power state name
└── ctl.power_flows           # read/write: dynamic power sources and sinks
```

**Source files:** `fuse/src/fs.rs` (CONTROL_FILES, TIME_SUBFILES,
ELAPSED_SUBFILES, POS_SUBFILES, CHANNEL_SUBFILES arrays),
`fuse/src/file.rs` (NexusFile buffer), `fuse/src/channel.rs` (ChannelMode).

---

## Channel Directories

Each channel the protocol publishes to or subscribes from appears as a
directory containing three sub-files.

### `<channel>/channel`

**Mode:** Read/Write (actual mode depends on publisher/subscriber declaration)

**Writing (transmit):** Write a message to this file. The simulator picks it
up, applies link simulation (delays, bit errors, packet loss based on
distance), and delivers it to subscribers.

**Reading (receive):** Read from this file. If a message is queued, it is
returned immediately. If no message is queued, the read blocks until the
simulator delivers one. This is how protocols can naturally sleep until data
arrives without polling.

Each protocol only sees channel directories for channels it is declared as a
publisher or subscriber to in the config. A protocol declared as both
publisher and subscriber to the same channel has a single directory for both
operations.

```python
# Transmit
with open("radio/channel", "w") as f:
    f.write("hello world")

# Receive (blocks until message arrives)
with open("radio/channel") as f:
    data = f.read()
```

### `<channel>/rssi`

**Mode:** Read-only

Returns the RSSI (Received Signal Strength Indicator) in dBm from the last
message received on this channel, as an ASCII decimal float.

```python
with open("radio/rssi") as f:
    rssi_dbm = float(f.read())
```

### `<channel>/snr`

**Mode:** Read-only

Returns the SNR (Signal-to-Noise Ratio) in dB from the last message received
on this channel, as an ASCII decimal float.

```python
with open("radio/snr") as f:
    snr_db = float(f.read())
```

---

## Time Control Files

These files allow protocol code to query and control simulated time. They live
inside the `ctl.time/` directory.

### `ctl.time/{s,ms,us,ns}`

**Mode:** Read/Write

**Read:** Returns the current simulated time as an ASCII decimal integer in
the specified unit. This is relative to the Unix epoch.

```python
with open("ctl.time/ms") as f:
    now_ms = int(f.read())
```

**Write:** Sets the simulated time using a Unix epoch timestamp. The protocol
blocks until the simulation reaches the written time.

```python
# Set simulation time to Thursday, January 1, 2026 at 07:00:00 GMT-05:00
with open("ctl.time/ms", "w") as f:
    f.write("1767268800000")
```

---

## Elapsed Time Files

These files live inside the `ctl.elapsed/` directory and report total elapsed
simulated time since the simulation started.

### `ctl.elapsed/{s,ms,us,ns}`

**Mode:** Read-only

Returns the total elapsed simulated time since the simulation started, as an
ASCII decimal integer.

```python
with open("ctl.elapsed/ms") as f:
    elapsed_ms = int(f.read())
```

This differs from `ctl.time/*` in that it always increases monotonically and
cannot be written to. Useful for computing durations without needing to store
a start time.

---

## Energy Control Files

These files are fully wired. See [energy-framework.md](energy-framework.md)
for detailed documentation.

### `ctl.energy_left`

**Mode:** Read-only

Returns the node's current charge as an ASCII decimal integer in nanojoules.
Returns `0` if the node has no battery configured. Can be negative when the
node is dead (charge depleted past zero).

```python
with open("ctl.energy_left") as f:
    charge_nj = int(f.read())
```

### `ctl.energy_state`

**Mode:** Read/Write

**Read:** Returns the name of the currently active power state (e.g.,
`"sleep"`, `"active"`), or an empty string if no state is active.

**Write:** Switches to a named power state. The name must exactly match a
key in the node's `power_states` config. Unknown names are silently ignored.

```python
with open("ctl.energy_state", "w") as f:
    f.write("transmit")
```

### `ctl.power_flows`

**Mode:** Read/Write

**Read:** Returns all currently active power flows, one per line:

```
source solar 350 nj/ts
sink mcu 80 nj/ts
```

**Write:** Adds, updates, or removes a Constant flow:

```
source battery_charger 400 mw/s   # add/update a source
sink radio 120 mw/s               # add/update a sink
remove mcu                        # remove a flow by name
```

Supported power units: `nw`, `uw`, `mw`, `w`, `kw`.
Supported time units: `h`, `m`, `s`, `ms`, `us`, `ns`.

Flows added dynamically are always Constant. Piecewise linear flows defined
in config appear in the read output with their current instantaneous rate but
cannot be updated through this file.

---

## Position Control Files

These files live inside the `ctl.pos/` directory. See
[position-control.md](position-control.md) for complete documentation of
motion patterns and interaction rules.

### `ctl.pos/{x,y,z}` — Absolute Position

**Mode:** Read/Write

Read or write the node's X, Y, or Z coordinate in the node's configured
distance unit. Writing any absolute coordinate clears the active motion
pattern.

```python
with open("ctl.pos/x", "w") as f:
    f.write("100.0")
```

### `ctl.pos/{az,el,roll}` — Orientation

**Mode:** Read/Write

Read or write orientation angles in degrees. Writing clears the motion pattern.

### `ctl.pos/{dx,dy,dz}` — Relative Deltas

**Mode:** Write-only

Add a delta to the corresponding coordinate. Snapshots the current position
first (including any in-progress motion), then resets to Static.

```python
with open("ctl.pos/dx", "w") as f:
    f.write("10.0")
```

### `ctl.pos/motion` — Motion Pattern

**Mode:** Read/Write

**Read:** Returns the active motion pattern as a spec string (e.g.,
`"velocity 0.001 0.0 0.0"` or `"none"`).

**Write:** Sets a new motion pattern. Formats:

```
none                                           # stop moving
velocity <vx> <vy> <vz>                       # constant velocity (units/µs)
linear <tx> <ty> <tz> <duration_us>            # interpolate to target
circle <cx> <cy> <cz> <radius> <deg_per_us>   # circular orbit
```

---

## Protocol Integration Patterns

### Checking elapsed time

```python
def elapsed_ms():
    with open("ctl.elapsed/ms") as f:
        return int(f.read())

start = elapsed_ms()
# ... do work ...
duration = elapsed_ms() - start
print(f"took {duration} ms simulated time")
```

### Energy-aware transmission

```python
with open("ctl.energy_state", "w") as f:
    f.write("transmit")

with open("radio/channel", "w") as f:
    f.write(payload)

with open("ctl.energy_state", "w") as f:
    f.write("sleep")
```

### Reading signal quality after reception

```python
with open("radio/channel") as f:
    data = f.read()

with open("radio/rssi") as f:
    rssi = float(f.read())

with open("radio/snr") as f:
    snr = float(f.read())

print(f"Received {len(data)} bytes, RSSI={rssi} dBm, SNR={snr} dB")
```

### Moving to a waypoint

```python
# Move to (500, 500, 0) over 10 seconds
with open("ctl.pos/motion", "w") as f:
    f.write("linear 500.0 500.0 0.0 10000000")
```
