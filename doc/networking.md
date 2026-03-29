# Linux Networking Interface Integration — Research and Implementation Plan

## Table of Contents

1. [Motivation](#motivation)
2. [Research Findings](#research-findings)
   - [Current Protocol Execution Model](#current-protocol-execution-model)
   - [Linux Networking Primitives](#linux-networking-primitives)
   - [Compatibility with Nexus Goals](#compatibility-with-nexus-goals)
3. [Design](#design)
   - [Architecture Overview](#architecture-overview)
   - [Network Mode: TAP Bridge](#network-mode-tap-bridge)
   - [Configuration Schema](#configuration-schema)
4. [Implementation Plan](#implementation-plan)
5. [Injection Points](#injection-points)
6. [Permissions and Requirements](#permissions-and-requirements)
7. [Test Plan](#test-plan)

---

## Motivation

Nexus currently uses a FUSE filesystem as the sole IPC mechanism between
protocol processes and the simulator. Protocols read and write files to
send and receive messages. This works well for custom embedded protocols,
but it cannot simulate protocols that rely on the **real Linux networking
stack** — TCP/IP, UDP, DNS, mDNS, MQTT over sockets, HTTP, etc.

Adding real networking interface support would let Nexus:

- Test unmodified socket-based applications (e.g., an MQTT broker, a CoAP
  endpoint, a gRPC service) in simulated network conditions.
- Simulate packet loss, delay, and bit errors on real IP traffic.
- Model multi-hop IP routing with realistic latency.
- Bridge the gap between embedded-protocol testing (FUSE) and
  application-layer testing (sockets).

---

## Research Findings

### Current Protocol Execution Model

**Process spawning** (`runner/src/lib.rs:46-63`):

```
Command::new("bash")
    .arg("-c")
    .arg("echo $$ > cgroup.procs && unbuffer {runner} {args}")
    .current_dir(protocol.root)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .stdin(Stdio::null())
    .spawn()
```

Key observations:
- Processes run in the **host network namespace** — they see all host interfaces.
- No network isolation exists; processes can bind to any port.
- The bash wrapper self-registers the PID into a cgroup via `echo $$ > cgroup.procs`.
- There is a **freeze/unfreeze** mechanism: processes start frozen and are
  unfrozen after FUSE mount completes.

**Integration points for networking**:

1. **Pre-spawn** (`run_protocol()` in `runner/src/lib.rs`): Create a network
   namespace and TAP interface before spawning the process.
2. **Bash wrapper**: Extend the script to enter the namespace via
   `ip netns exec` or `nsenter`.
3. **Post-spawn**: Configure the TAP interface (IP addresses, routes).
4. **Respawn** (`ProtocolHandle::respawn()` in `runner/src/cgroups.rs`):
   Recreate the namespace and TAP for the new process.
5. **Cleanup** (`CgroupController::drop()` in `runner/src/cgroups.rs`):
   Delete namespaces and TAP interfaces.

### Linux Networking Primitives

#### TAP Interfaces

A TAP (network TAP) interface is a virtual Layer 2 (Ethernet) device. The
kernel presents one end as a normal network interface; the other end is a
file descriptor that userspace can read/write raw Ethernet frames from.

**How it works for Nexus:**
- Create a TAP device per node (e.g., `nexus_node0`).
- The protocol process uses the TAP as its only network interface.
- Nexus reads frames from the TAP fd, applies link simulation (delay, loss,
  corruption), and writes them to the destination node's TAP fd.
- From the protocol's perspective, it has a normal network interface.

**Creation from Rust:**
```rust
// Using the nix crate + ioctl
let fd = open("/dev/net/tun", O_RDWR)?;
let mut ifr: ifreq = zeroed();
ifr.ifr_name = b"nexus_node0\0";
ifr.ifr_flags = IFF_TAP | IFF_NO_PI;
ioctl(fd, TUNSETIFF, &ifr)?;
```

Or via the `tun-tap` crate which wraps this.

**Permissions:** Requires `CAP_NET_ADMIN` or root. The existing cgroup
delegation already runs under `systemd-run --user --scope -p Delegate=true`,
which can be extended to include `AmbientCapabilities=CAP_NET_ADMIN`.

#### Network Namespaces

A network namespace is a Linux kernel feature that gives a process its own
isolated network stack (interfaces, routing table, iptables rules, etc.).

**How it works for Nexus:**
- Create a namespace per node: `ip netns add nexus_node0`.
- Move the TAP interface into the namespace: `ip link set nexus_node0 netns nexus_node0`.
- The protocol process is spawned inside the namespace.
- Inside the namespace, the protocol sees only its TAP interface — no host
  interfaces, no other nodes' interfaces.

**Rust APIs:**
- `nix::sched::unshare(CloneFlags::CLONE_NEWNET)` — creates a new namespace
  for the current process.
- `nix::sched::setns(fd, CloneFlags::CLONE_NEWNET)` — enters an existing
  namespace.
- `std::process::Command::new("ip").args(["netns", "exec", name, ...])` —
  spawns a process inside a namespace.

#### veth Pairs

A veth (virtual Ethernet) pair creates two linked network interfaces. Packets
sent to one end appear on the other. They're typically used to connect a
network namespace to the host.

**How it works for Nexus:**
- Create a veth pair: one end in the host, one end in the node's namespace.
- The host end connects to the Nexus routing server.
- The namespace end is the node's network interface.

**Trade-off vs. TAP:**
- TAP is simpler: single interface, single fd, direct frame access.
- veth requires bridge setup or manual routing.
- **Recommendation: Use TAP** for simplicity. Nexus reads/writes frames
  directly, applying its own link simulation.

#### Packet Manipulation

Once Nexus intercepts packets from a TAP interface, it can apply the same
link simulation as FUSE messages:

- **Delay**: Hold the packet in a queue and deliver it after the computed
  propagation + transmission delay.
- **Packet loss**: Drop the packet based on the RSSI-dependent probability.
- **Bit errors**: Flip bits in the frame based on the BER.
- **Terrain attenuation**: Apply the terrain loss computed from the obstacle
  map (already implemented).

This is functionally identical to the FUSE message path, but operating on
raw Ethernet frames instead of application-level byte buffers.

### Compatibility with Nexus Goals

| Nexus Goal | FUSE Channels | TAP Networking | Compatibility |
|-----------|---------------|----------------|---------------|
| Protocol-agnostic | File I/O only | Any socket-based app | Extends reach |
| Physics-accurate links | Via RoutingServer | Same RSSI/delay models | Full |
| Deterministic | Discrete-event, blocking reads | Non-deterministic (real sockets) | Partial — see below |
| Resource emulation | cgroup CPU throttling | Same cgroups | Full |
| Energy model | TX/RX costs on file ops | TX/RX costs on frame send/recv | Full |

**Determinism challenge:** Real networking is inherently non-deterministic.
TCP retransmissions, kernel scheduling, and buffer sizes introduce timing
variations. This is acceptable for a new `network_mode = "tap"` channel type
because:

1. The primary use case is testing real applications, not bit-exact replay.
2. FUSE channels remain available for deterministic protocol testing.
3. The link simulation (delay, loss) is still controlled by Nexus.

---

## Design

### Architecture Overview

```
┌─────────────────────┐     ┌─────────────────────┐
│  Node A (namespace)  │     │  Node B (namespace)  │
│  ┌────────────────┐  │     │  ┌────────────────┐  │
│  │ Protocol (app)  │  │     │  │ Protocol (app)  │  │
│  │ binds to socket │  │     │  │ binds to socket │  │
│  └───────┬────────┘  │     │  └───────┬────────┘  │
│          │           │     │          │           │
│     ┌────▼────┐      │     │     ┌────▼────┐      │
│     │ TAP dev │      │     │     │ TAP dev │      │
│     └────┬────┘      │     │     └────┬────┘      │
└──────────┼───────────┘     └──────────┼───────────┘
           │ raw frames                  │ raw frames
      ┌────▼────────────────────────────▼────┐
      │         Nexus TAP Router             │
      │  (reads frames from all TAPs)        │
      │  - Applies RSSI model                │
      │  - Applies delay/loss/BER            │
      │  - Applies terrain attenuation       │
      │  - Queues for delivery               │
      │  - Writes to destination TAP         │
      └─────────────────────────────────────┘
```

### Network Mode: TAP Bridge

A new channel type `network` (distinct from `exclusive` and `shared`) that:

1. Creates a TAP interface per node (not per protocol — all protocols on a
   node share the same network namespace).
2. Creates a network namespace per node.
3. Spawns protocol processes inside their node's namespace.
4. Runs a **TAP Router** thread that:
   - Polls all TAP fds using `mio` or `epoll`.
   - For each frame, determines the destination by MAC address or IP.
   - Applies link simulation (same RSSI/delay/loss models as FUSE channels).
   - Writes the frame to the destination node's TAP fd.

### Configuration Schema

```toml
[channels.ip_network]
link = "wifi_link"
type = { type = "network" }

# Network settings per node (inside deployment)
[nodes.sensor]
deployments = [{
    position = { point = [0, 0, 0], unit = "m" },
    network = {
        address = "10.0.0.1/24",
        gateway = "10.0.0.254",
    }
}]

[nodes.gateway]
deployments = [{
    position = { point = [100, 0, 0], unit = "m" },
    network = {
        address = "10.0.0.254/24",
    }
}]
```

When a `network` channel exists, all nodes subscribing to it get:
- A network namespace (`nexus_{sim_id}_{node_name}`)
- A TAP interface inside the namespace (configured with the given address)
- Their protocol processes spawned inside the namespace

---

## Implementation Plan

### Phase 1: TAP Interface Core (new `netdev` crate)

Create a new workspace member `netdev/` with:

1. **`netdev/src/tap.rs`** — TAP interface creation and I/O
   - `TapDevice::create(name: &str) -> Result<Self>`
   - `TapDevice::read_frame(&self) -> Result<Vec<u8>>`
   - `TapDevice::write_frame(&self, frame: &[u8]) -> Result<()>`
   - `TapDevice::set_up(&self) -> Result<()>`
   - Uses `nix` crate for ioctl

2. **`netdev/src/namespace.rs`** — Network namespace management
   - `Namespace::create(name: &str) -> Result<Self>`
   - `Namespace::move_interface(&self, ifname: &str) -> Result<()>`
   - `Namespace::configure_address(&self, ifname: &str, addr: &str) -> Result<()>`
   - `Namespace::delete(name: &str) -> Result<()>`

3. **`netdev/src/router.rs`** — TAP frame router
   - `TapRouter::new(taps: Vec<TapDevice>, channels: ...) -> Self`
   - `TapRouter::poll(&mut self) -> Result<Vec<Frame>>`
   - `TapRouter::deliver(&mut self, frame: Frame, delay: Duration) -> Result<()>`
   - Integrates with the existing `send_through_channel` for link simulation

### Phase 2: Kernel Integration

4. **`config/src/ast.rs`** — Add `Network` channel kind and per-node network config
5. **`config/src/parse.rs`** — Deserialize network config
6. **`runner/src/lib.rs`** — Spawn processes in namespaces when network channels exist
7. **`runner/src/cgroups.rs`** — Extend cleanup to delete namespaces
8. **`kernel/src/lib.rs`** — Start TapRouter thread alongside RoutingServer

### Phase 3: GUI Integration

9. **`gui/src/state.rs`** — Display network interface status in inspector
10. **`gui/src/panels/inspector.rs`** — Show IP address, packet counts per node

---

## Injection Points

| File | What Changes | Why |
|------|-------------|-----|
| `runner/src/lib.rs:46-63` | Wrap spawn in `ip netns exec` when network mode | Process isolation |
| `runner/src/cgroups.rs:426-432` | Delete namespaces on Drop | Cleanup |
| `runner/src/cgroups.rs:105-118` | Recreate namespace on respawn | Energy recovery |
| `kernel/src/lib.rs:202-205` | Start TapRouter alongside RoutingServer | Frame routing |
| `kernel/src/router/link_simulation.rs` | Reuse `send_through_channel` for frames | Link sim |
| `config/src/ast.rs` | Add `ChannelKind::Network` | Config model |
| `config/src/parse.rs` | Add network deployment config | Parsing |
| `fuse/src/fs.rs` | No changes — FUSE and TAP coexist | Compatibility |

---

## Permissions and Requirements

| Requirement | Current | With Networking |
|-------------|---------|-----------------|
| Linux | Required | Required |
| cgroup v2 | Required | Required |
| FUSE3 | Required | Required |
| CAP_NET_ADMIN | Not needed | **Required** for TAP/namespace creation |
| Root or sudo | Not needed | May be needed depending on capability setup |

**Mitigation:** The `justfile` already uses `systemd-run --user --scope -p Delegate=true`.
This can be extended:
```
systemd-run --user --scope -p Delegate=true -p AmbientCapabilities=CAP_NET_ADMIN
```

Alternatively, the `nexus` binary can be given the capability via:
```
sudo setcap cap_net_admin+ep target/release/nexus
```

---

## Test Plan

### Unit tests (netdev crate)

1. `test_tap_create_and_destroy` — create TAP, verify it exists in `ip link`, destroy
2. `test_tap_read_write_loopback` — write frame to TAP, read it back
3. `test_namespace_create_delete` — create netns, verify isolation, delete
4. `test_namespace_move_interface` — move TAP into namespace, verify visibility
5. `test_namespace_configure_address` — set IP, verify with `ip addr`

### Integration tests (kernel)

6. `test_network_channel_creates_tap` — config with network channel creates TAP interfaces
7. `test_network_channel_frame_delivery` — frame sent from node A arrives at node B
8. `test_network_channel_applies_delay` — frame delivery respects propagation delay
9. `test_network_channel_applies_loss` — packet loss expression drops frames
10. `test_network_channel_coexists_with_fuse` — FUSE channels work alongside network channels

### System tests (require CAP_NET_ADMIN)

11. `test_tcp_echo_through_simulator` — TCP echo server/client through simulated network
12. `test_udp_broadcast_shared_channel` — UDP broadcast with collision detection
13. `test_network_survives_respawn` — energy depletion → respawn preserves network config
