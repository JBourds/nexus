# Architecture

## What Nexus Is

Nexus is a discrete-event network simulator designed for testing production
protocol code against simulated network conditions. Unlike packet-level
simulators (ns-3, OMNET++) where the protocol logic lives inside the simulator,
Nexus runs protocol code as real OS processes. The simulator's job is to:

1. Present each process with a filesystem interface that looks like a network
   (files to read from / write to in place of sockets or radio calls).
2. Inject realistic channel behavior: delays, bit errors, packet loss, signal
   attenuation based on distance.
3. Control how fast each process perceives time by throttling CPU using cgroups,
   so an ATmega2560 protocol compiled for Linux runs at 16 MHz equivalent speed.

This means the exact same source code that runs on embedded hardware (with thin
HAL shims replacing radio calls with file I/O) can be tested in simulation.

## Conceptual Model

### Simulation Config

A simulation is described in a TOML file. The three main entities are:

- **Nodes:** logical hosts in the network. A node can represent a physical
  device, a sensor, a gateway, etc. Each node has a position in 3D space,
  optional resource limits (CPU clock rate, memory), and one or more protocols.

- **Protocols:** OS processes that run inside a node. A node can run multiple
  protocols (e.g., a MAC layer and an application layer process). Each protocol
  specifies which channels it publishes to and subscribes from.

- **Channels:** named communication mediums. Channels have a type (exclusive
  or shared) and reference a link definition that describes the physical
  properties of the medium.

- **Links:** reusable link definitions specifying signal model, error rates,
  and delays. A channel references a link by name.

### Channel Types

**Exclusive** channels are FIFO buffers per subscriber pair. When a publisher
writes, each subscriber that is part of that channel gets an independent copy
of the message in its own queue. Messages are delivered in order with no
collisions. This models a point-to-point or star-topology channel.

**Shared** channels model a true broadcast medium (e.g., LoRa, 802.11 air).
When multiple nodes transmit simultaneously, an OR-collision occurs : all
concurrent transmissions are dropped. Protocols must implement their own
medium access control (e.g., TDMA, CSMA) to avoid collisions.

### Time Model

Simulation advances in discrete timesteps. Each timestep the routing server
processes all pending writes and delivers messages whose simulated delivery
time has elapsed. The timestep length is configurable (e.g., 1 ms, 20 ms).

Protocols can query and block on simulated time via the `ctl.time.*` control
files. This allows a protocol to sleep until a specific simulated time without
busy-waiting on wall-clock time.

### Resource Emulation

CPU clock-rate emulation is implemented via cgroup v2 `cpu.max`. Given a
configured clock rate (e.g., 16 MHz) and the host CPU's current frequency,
Nexus computes a throttle ratio and applies it to each protocol's cgroup. The
protocol process therefore gets exactly the CPU time budget proportional to
the ratio of simulated-to-real clock speed.

## Components

```
┌────────────────────────────────────────────────────┐
│  CLI (cli/)                                        │
│  simulate / replay / logs subcommands              │
└──────────┬─────────────────────────────────────────┘
           │ config parse + validate
           ▼
┌────────────────────────────────────────────────────┐
│  Config (config/)                                  │
│  TOML → Simulation AST                             │
│  Signal models, delay calculators, unit handling   │
└──────────┬─────────────────────────────────────────┘
           │ AST
           ▼
┌────────────────────────────────────────────────────┐
│  Runner (runner/)                                  │
│  Build protocols (make/cargo/etc.)                 │
│  Spawn protocol processes                          │
│  Set up cgroup v2 hierarchy                        │
│  Set CPU affinity, bandwidth, weights              │
└──────────┬─────────────────────────────────────────┘
           │ process handles + cgroup controllers
           ▼
┌─────────────────────────────────┐  ┌──────────────────────────────────┐
│  FUSE Filesystem (fuse/)        │  │  Kernel (kernel/)                │
│  Channel files (read/write)     ◄──►  RoutingServer: event loop       │
│  Control files (ctl.*)          │  │  StatusServer: health + resources│
│  Per-PID buffering              │  │  Binary TX/RX log writing        │
└─────────────────────────────────┘  └──────────────────────────────────┘
           │
           │ channel files mounted at each node's root path
           ▼
┌────────────────────────────────────────────────────┐
│  Protocol Processes                                │
│  (the code under test)                             │
│  Read from channel files → receive messages        │
│  Write to channel files → transmit messages        │
│  Read/write ctl.* files → query/control sim state  │
└────────────────────────────────────────────────────┘
```

## Data Flow

### Startup Sequence

```
1. CLI reads TOML → config::parse() → Simulation AST
2. runner::build()   : compile each protocol (optional)
3. runner::run()     : spawn processes, create cgroup hierarchy,
                       apply CPU affinity + bandwidth throttling
4. NexusFs::new()    : create FUSE filesystem with channel + control files
5. Kernel::new()     : resolve string channel names → usize handles,
                       pre-compute routing table (subscriber/publisher graph,
                       RSSI between all node pairs)
6. FUSE mounted at each node's root path
7. Kernel::run()     : main event loop begins
```

### Per-Timestep Loop

```
for each timestep:
  StatusServer::check_health()           // detect premature process exits
  RoutingServer::poll(timestep)
      ← FsMessage::Write  (protocol wrote to a channel file)
      ← FsMessage::Read   (protocol is blocking on a channel file)
      → KernelMessage::Exclusive / Shared / Empty  (deliver message to FUSE)
  StatusServer::update_resources()       // refresh CPU frequencies,
                                         // recompute + apply cgroup bandwidth
```

### Message Lifecycle

```
Protocol A writes to channel file
  → FUSE captures write → FsMessage::Write sent to RoutingServer
  → RoutingServer looks up publishers/subscribers for that channel
  → Applies link simulation:
      - Computes RSSI from sender/receiver positions
      - Evaluates bit_error and packet_loss expressions against RSSI
      - If not lost: schedules delivery at (now + propagation + processing
        + transmission delay)
  → At delivery timestep: KernelMessage::Exclusive (or Shared) sent to FUSE
  → FUSE buffers message in per-PID queue for subscriber
  → Protocol B reads channel file → FUSE dequeues + returns message
```

## Key Design Decisions

**FUSE as IPC layer**: Using a filesystem means protocol code only needs to
replace socket/radio calls with `open/read/write`. No special libraries or
language bindings required.

**Discrete-event over real-time**: The simulator does not run in real time.
Simulated time advances as fast as the host allows. This enables
deterministic replay and makes long-duration protocol tests feasible.

**cgroup v2 for resource control**: Rather than simulating CPU time
in software, Nexus throttles the actual OS scheduler. The protocol process
genuinely cannot run faster than its configured clock rate allows.

**String handles in config, usize in kernel**: Config uses human-readable
names throughout. At kernel startup, a resolver converts all string handles to
array indices for O(1) lookup in the hot event loop path.

**Separate TX and RX logs**: Binary logs record every write (TX) and every
delivery (RX) with timestamps. Used for the `replay` command and for
post-simulation analysis.
