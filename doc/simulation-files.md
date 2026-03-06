# Control Files

When Nexus mounts a simulation, each protocol process sees a directory
containing channel files and control files. Control files let protocol code
query and interact with the simulator's state.

## Filesystem Layout

Each protocol's working directory (its `root`) is the mount point. The exact
files present depend on which channels the protocol subscribes/publishes to,
but the layout looks like:

```
<node-name>/
├── <channel-name>        # channel file: read to receive, write to transmit
├── <channel-name>        # (one file per channel the protocol is subscribed/published to)
├── ctl.time.us           # read/write: simulated time in microseconds
├── ctl.time.ms           # read/write: simulated time in milliseconds
├── ctl.time.s            # read/write: simulated time in seconds
├── ctl.elapsed.us        # read-only: elapsed simulated time in microseconds
├── ctl.elapsed.ms        # read-only: elapsed simulated time in milliseconds
├── ctl.elapsed.s         # read-only: elapsed simulated time in seconds
├── ctl.energy_left       # read-only: remaining energy in nanojoules
├── ctl.energy_state      # read/write: current power state name
└── ctl.position          # read/write: node position (NOT YET IMPLEMENTED)
```

## Channel Files

Reading and writing channel files is the primary way protocol code sends and
receives data in the simulation.

**Writing (transmit):** Write a message to a channel file. The simulator picks
it up, applies link simulation (delays, bit errors, packet loss based on
distance), and delivers it to subscribers.

**Reading (receive):** Read from a channel file. If a message is queued, it is
returned immediately. If no message is queued, the read blocks until the
simulator delivers one. This is how protocols can naturally sleep until data
arrives without polling.

Each protocol only sees channel files for channels it is declared as a
publisher or subscriber to in the config. A protocol declared as both
publisher and subscriber to the same channel has a single file for both
operations.

## Time Control Files

These files allow protocol code to query and control simulated time.

### `ctl.time.us` / `ctl.time.ms` / `ctl.time.s`

**Mode:** Read/Write

**Read:** Returns the current simulated time as an ASCII decimal integer in
the specified unit. This is relative to the Unix epoch.

```python
with open("ctl.time.ms") as f:
    now_ms = int(f.read())
```

**Write:** Sets the simulated time using a Unix epoch timestamp.

```python
# Set simulation time to Thursday, January 1, 2026 at 07:00:00 GMT-05:00
with open("ctl.time.ms", "w") as f:
    f.write("1767268800000")
```

### `ctl.elapsed.us` / `ctl.elapsed.ms` / `ctl.elapsed.s`

**Mode:** Read-only

**Read:** Returns the total elapsed simulated time since the simulation
started, as an ASCII decimal integer.

```python
with open("ctl.elapsed.ms") as f:
    elapsed_ms = int(f.read())
```

This differs from `ctl.time.*` in that it always increases monotonically and
cannot be written to. Useful for computing durations without needing to store
a start time.

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

## Position File

> **Status: Not yet implemented.** The file is defined but not wired to
> kernel logic. See [known-gaps.md](known-gaps.md#mobile-nodes).

### `ctl.position`

**Mode:** Read/Write (planned)

**Read:** Returns the node's current 3D position and orientation.

**Write:** Updates the node's position. The simulator recalculates RSSI
between this node and all others on the next message delivery.

Format:

```
x,y,z
x,y,z,unit
x,y,z,az,el,roll
x,y,z,az,el,roll,unit
```

Examples:

```
1.5,2.0,0.0
1500,0,0,m
0,0,0,0,45,0,m
```

Default unit is whatever the node's config specifies.

## Protocol Integration Patterns

### Checking elapsed time

```python
def elapsed_ms():
    with open("ctl.elapsed.ms") as f:
        return int(f.read())

start = elapsed_ms()
# ... do work ...
duration = elapsed_ms() - start
print(f"took {duration} ms simulated time")
```
