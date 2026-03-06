# Position Control

## Table of Contents

1. [Overview](#overview)
2. [Control Files](#control-files)
3. [Motion Patterns](#motion-patterns)
   - [Static](#static)
   - [Velocity](#velocity)
   - [Linear](#linear)
   - [Circle](#circle)
4. [Motion Pattern Spec Format](#motion-pattern-spec-format)
5. [Interaction Rules](#interaction-rules)
6. [Integration with Routing](#integration-with-routing)
7. [Logging](#logging)
8. [Examples](#examples)

---

## Overview

Nexus supports mobile nodes: nodes whose positions change continuously during a
simulation. A node's position affects the RSSI between it and every other node,
which in turn drives link simulation. Bit error rates, packet loss, and
propagation delay are all computed from inter-node distance at the moment a
message is sent or received.

Protocol code controls its node's position through a set of `ctl.pos.*` files
exposed in its FUSE working directory. Writes to these files take effect
immediately: the routing server computes distances from the updated live
positions at the next message queue or delivery event. No simulation restart is
required.

Position is represented as a 3D point `(x, y, z)` in the node's configured
distance unit (e.g., meters, kilometers) plus an orientation described by
azimuth, elevation, and roll in degrees. Orientation is carried through
the API and logged, but only the XYZ coordinates factor into distance
computations.

---

## Control Files

Each protocol process sees the following `ctl.pos.*` files in its working
directory. All files are per-PID: each process reads and writes its own node's
position.

| File | Mode | Description |
|------|------|-------------|
| `ctl.pos.x` | Read/Write | X coordinate in the node's distance unit. |
| `ctl.pos.y` | Read/Write | Y coordinate in the node's distance unit. |
| `ctl.pos.z` | Read/Write | Z coordinate in the node's distance unit. |
| `ctl.pos.az` | Read/Write | Azimuth (yaw) in degrees. |
| `ctl.pos.el` | Read/Write | Elevation (pitch) in degrees. |
| `ctl.pos.roll` | Read/Write | Roll in degrees. |
| `ctl.pos.dx` | Write-only | Add a delta to X; snapshots current position first. |
| `ctl.pos.dy` | Write-only | Add a delta to Y; snapshots current position first. |
| `ctl.pos.dz` | Write-only | Add a delta to Z; snapshots current position first. |
| `ctl.pos.motion` | Read/Write | Active motion pattern; see spec format below. |

**Read format.** All readable files return an ASCII decimal representation of
their value followed by no trailing newline. Float values use Rust's default
`f64` `to_string()` rendering (e.g., `"3.14"`, `"-0.5"`, `"1"`).

**Write format.** All writable files accept a decimal float string, optionally
surrounded by whitespace, which is trimmed before parsing.

---

## Motion Patterns

The `MotionPattern` type in `kernel/src/types.rs` describes how a node's
`(x, y, z)` position evolves over time. The active pattern is stored per-node
and evaluated at every simulation step.

All coordinate values are in the node's configured distance unit. Velocities
and angular rates are expressed **per microsecond**.

`current_point(timestep, us_per_step)` converts the raw step counter to elapsed
microseconds before applying velocities and durations. The `us_per_step` factor
is derived from the simulation's timestep config (e.g., 10ms steps = 10,000
µs/step). This ensures that motion speeds specified in distance-unit/µs produce
correct displacement regardless of the configured timestep duration.

### Static

The node does not move. `position.point` remains at whatever coordinates it
was last set to.

`current_point()` returns `None` for `Static`, meaning the routing server
skips the position update entirely — no work is done and no log event is
emitted for nodes that are not moving.

### Velocity

Constant-velocity rectilinear motion from an initial point.

**Fields:**

- `initial` — position at the moment the pattern was activated
- `velocity` — per-axis velocity in distance-unit per microsecond
- `start_ts` — simulator timestep when the pattern was activated

**Formula** at timestep `t` with `us_per_step` microseconds per step:

```
dt_us = max(t - start_ts, 0) * us_per_step   # elapsed microseconds

x(t) = initial.x + velocity.x * dt_us
y(t) = initial.y + velocity.y * dt_us
z(t) = initial.z + velocity.z * dt_us
```

The node moves indefinitely; there is no stopping condition. To halt a
velocity-driven node, write `none` to `ctl.pos.motion` or write an absolute
position to any `ctl.pos.{x,y,z}` file.

### Linear

Linear interpolation between a start point and an end point over a fixed
duration. The node stops at the end point once the duration elapses.

**Fields:**

- `start` — position snapshotted when the pattern was activated
- `end` — target position (`tx`, `ty`, `tz` from the spec string)
- `start_ts` — simulator timestep when the pattern was activated
- `duration_us` — total travel time in microseconds

**Formula** at timestep `t` with `us_per_step` microseconds per step:

```
dt_us = max(t - start_ts, 0) * us_per_step
frac = min(dt_us / duration_us, 1.0)   # clamped to [0, 1]

x(t) = start.x + (end.x - start.x) * frac
y(t) = start.y + (end.y - start.y) * frac
z(t) = start.z + (end.z - start.z) * frac
```

Once the elapsed microseconds exceed `duration_us`, `frac` is clamped at `1.0`
and the node sits at `end` until the pattern changes.

### Circle

Circular orbit in the XY plane. The Z coordinate is held fixed at
`center.z`. The orbit direction is determined by the sign of
`angular_vel_deg_per_us`: positive rotates counter-clockwise.

**Fields:**

- `center` — center of the orbit
- `radius` — orbit radius in the node's distance unit
- `start_angle_deg` — initial angle in degrees, derived automatically from
  the node's current position relative to the center (see
  [Interaction Rules](#interaction-rules))
- `angular_vel_deg_per_us` — rate of rotation in degrees per microsecond
- `start_ts` — simulator timestep when the pattern was activated

**Formula** at timestep `t` with `us_per_step` microseconds per step:

```
dt_us = max(t - start_ts, 0) * us_per_step
angle_deg = start_angle_deg + angular_vel_deg_per_us * dt_us
angle_rad = angle_deg * pi / 180

x(t) = center.x + radius * cos(angle_rad)
y(t) = center.y + radius * sin(angle_rad)
z(t) = center.z
```

The start angle is computed automatically from the node's current position
when the pattern is written, so the node begins orbiting from exactly where
it currently is without an instantaneous jump:

```
start_angle_deg = atan2(current.y - center.y, current.x - center.x)
```

---

## Motion Pattern Spec Format

`ctl.pos.motion` is read and written as a plain-text spec string. Whitespace
between tokens is collapsed; leading and trailing whitespace is stripped.

### Read

Returns the current pattern as a spec string. For example, a node orbiting at
0.5 degrees per microsecond reads back:

```
circle 0 0 0 500 0.5
```

> **Note:** The spec string returned on read omits internal state fields
> (`start_ts`, `start_angle_deg`, `initial`, `start`) that were captured at
> activation time. Those fields are re-derived from the current position and
> current timestep when the spec is written back, so a read-modify-write cycle
> produces a pattern equivalent to the original from the reader's perspective.

### Write: `none`

Clears the active motion pattern. The node remains at its current position.

```
none
```

### Write: `velocity <vx> <vy> <vz>`

Sets constant-velocity motion. The three arguments are velocity components in
distance-unit per microsecond.

```
velocity 0.001 0.0 0.0
```

Sets the node drifting in the positive X direction at 0.001 units/µs
(1000 units/second).

### Write: `linear <tx> <ty> <tz> <dur_us>`

Sets a linear interpolation from the current position to `(tx, ty, tz)` over
`dur_us` microseconds.

```
linear 100.0 200.0 0.0 5000000
```

Moves the node to `(100, 200, 0)` over 5 seconds (5,000,000 µs), starting
from wherever the node currently is.

### Write: `circle <cx> <cy> <cz> <radius> <angular_vel_deg_per_us>`

Sets circular orbit around `(cx, cy, cz)` with the given radius and angular
velocity.

```
circle 0.0 0.0 0.0 500.0 0.000001
```

Orbits the origin at radius 500 units, rotating counter-clockwise at
0.000001 degrees/µs (one full revolution per 360,000,000 µs = 360 seconds).

**Negative angular velocity** rotates clockwise.

---

## Interaction Rules

These rules govern what happens to the active motion pattern when a position
control file is written. They ensure that transitions between positioning modes
are continuous. The node never jumps.

### Writing an absolute coordinate (`ctl.pos.{x,y,z,az,el,roll}`)

1. If a motion pattern is active, `current_point()` is evaluated at the
   current timestep to snapshot the node's in-progress position into
   `position.point`.
2. The written coordinate is set on `position.point` (or orientation).
3. The active motion pattern is reset to `Static`.

The snapshot in step 1 preserves the other coordinates that were not written.
For example, writing only `ctl.pos.x` while a velocity pattern is active will
fix the node at the Y and Z coordinates it had reached at that timestep, then
set X to the written value.

### Writing a relative delta (`ctl.pos.{dx,dy,dz}`)

1. If a motion pattern is active, `current_point()` is evaluated to snapshot
   the in-progress position into `position.point`.
2. The delta is added to the snapshotted coordinate.
3. The active motion pattern is reset to `Static`.

### Writing a motion pattern (`ctl.pos.motion`)

1. If a motion pattern is active, `current_point()` is evaluated to snapshot
   the in-progress position into `position.point`.
2. The new pattern is parsed. The current `position.point` is used as the
   pattern's `initial`/`start` point and the current timestep becomes
   `start_ts`.
3. The new pattern replaces the old one.

This means transitioning from, say, `Velocity` to `Circle` starts the orbit
from exactly where the velocity-driven node had reached, with no jump.

### Effect of all writes on the movement log

Every write to a `ctl.pos.*` file — absolute, delta, or motion pattern — emits
a `movement` tracing event with the resulting position. These events are
captured by the binary log layer and written as `LogRecord::Movement` records.
See [Logging](#logging).

---

## Integration with Routing

### Distance computed dynamically

The routing table (`kernel/src/router/table.rs`) stores only a `handle_ptr`
for each route: a pointer into the handles array for the destination endpoint.
There is no precomputed distance stored in the route. Instead, distance is
computed from live node positions at two points in the message lifecycle:

1. **Queue time** (`queue_message` in `delivery.rs`): When a protocol writes to
   a channel file, the routing server computes the current distance between the
   source node and each destination node. This distance drives propagation
   delay and, for exclusive channels, the packet-loss / bit-error decision.

2. **Delivery time** (`deliver_shared_msg` in `delivery.rs`): For shared
   (broadcast) channels, the RSSI and bit-error model are re-evaluated when
   the message is actually delivered to the subscriber. This is because shared
   channels defer the link simulation until read time to handle collision
   detection correctly.

Because distance is recomputed from `position.point` at these moments, moving
a node between the time a message is queued and the time it is delivered can
change the channel quality the message experiences.

### Motion applied every step

At the start of `RoutingServer::step()`, before any messages are delivered,
`apply_all_motions_and_log()` advances the position of every node that has an
active (non-`Static`) motion pattern:

```
us_per_step = ts_config.length * ts_config.unit.to_ns_factor() / 1000
for each node:
    if node.motion != Static:
        node.position.point = node.motion.current_point(timestep, us_per_step)
        emit "movement" tracing event
```

This means that at any given timestep, `position.point` reflects the
mathematically correct position for that timestep's motion formula before
any routing decisions are made.

---

## Logging

### `MovementRecord`

Every position update — whether triggered by a direct control-file write or by
the per-step `apply_all_motions_and_log()` — emits a `movement` tracing event.
The binary log layer (`BinaryLogLayer` in `kernel/src/log.rs`) captures this
event and writes a `LogRecord::Movement` record to the log file:

```rust
pub struct MovementRecord {
    pub timestep: u64,
    pub node: NodeHandle,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub az: f64,
    pub el: f64,
    pub roll: f64,
}
```

Movement records are interleaved with `LogRecord::Message` records in the same
binary log stream, encoded with `bincode` using a `u32` variant tag.

### Replay behavior

During replay (`Source::Replay` in `kernel/src/sources.rs`), the log reader
skips all `LogRecord::Movement` records entirely. Positions are not replayed
from the log; instead, position state starts from the node's initial
configuration value, and any protocol writes that happened during the original
simulation that changed position are not re-issued. Only TX `MessageRecord`s
(records where `tx == true`) are replayed.

This is intentional: the replay's job is to reproduce the message traffic, not
to reconstruct the exact spatial state. For spatial reconstruction, the
movement records in the log can be post-processed separately.

---

## Examples

The following pseudocode uses Python-style file I/O, matching Nexus's existing
example conventions.

### Read the current position

```python
def get_position():
    with open("ctl.pos.x") as f:
        x = float(f.read())
    with open("ctl.pos.y") as f:
        y = float(f.read())
    with open("ctl.pos.z") as f:
        z = float(f.read())
    return x, y, z

x, y, z = get_position()
print(f"Current position: ({x}, {y}, {z})")
```

### Teleport to an absolute position

Writing any absolute coordinate file clears the motion pattern and fixes the
node at the specified location. Write all three axes to fully reposition:

```python
def set_position(x, y, z):
    with open("ctl.pos.x", "w") as f:
        f.write(str(x))
    with open("ctl.pos.y", "w") as f:
        f.write(str(y))
    with open("ctl.pos.z", "w") as f:
        f.write(str(z))

set_position(100.0, 200.0, 0.0)
```

> **Note:** Each write to a `ctl.pos.*` absolute file independently clears the
> motion pattern. Writing all three axes in sequence is safe; the intermediate
> states after the first and second writes are coherent (Static pattern, partial
> position update).

### Apply a relative displacement

```python
# Move 10 units in the X direction from the current position,
# stopping any active motion pattern.
with open("ctl.pos.dx", "w") as f:
    f.write("10.0")
```

### Start constant-velocity motion

```python
# Drift north (positive Y) at 0.002 units/µs (2000 units/second).
with open("ctl.pos.motion", "w") as f:
    f.write("velocity 0.0 0.002 0.0")
```

### Move to a waypoint over 10 seconds

```python
# Linear interpolation to (500, 500, 0) over 10,000,000 µs (10 seconds).
with open("ctl.pos.motion", "w") as f:
    f.write("linear 500.0 500.0 0.0 10000000")
```

### Orbit a point

```python
# Orbit (0, 0, 0) at radius 300 units, counter-clockwise, one full
# revolution per 60 seconds (360 degrees / 60,000,000 µs).
angular_vel = 360.0 / 60_000_000   # ~6e-6 deg/µs
with open("ctl.pos.motion", "w") as f:
    f.write(f"circle 0.0 0.0 0.0 300.0 {angular_vel}")
```

### Read the active motion pattern

```python
with open("ctl.pos.motion") as f:
    spec = f.read()
print(f"Active pattern: {spec}")
# e.g.: "circle 0.0 0.0 0.0 300.0 6e-6"
```

### Stop all motion

```python
with open("ctl.pos.motion", "w") as f:
    f.write("none")
```

### Reorient without changing position

Orientation (azimuth, elevation, roll) is independent of the XYZ coordinates.
Writing an orientation file clears the motion pattern but does not change the
position coordinates.

```python
# Point the node 45 degrees azimuth, 10 degrees elevation.
with open("ctl.pos.az", "w") as f:
    f.write("45.0")
with open("ctl.pos.el", "w") as f:
    f.write("10.0")
```
