# Trace Format (.nxs)

Nexus records simulation events in a binary `.nxs` trace file. This file is
produced by the kernel's `BinaryLogLayer` tracing subscriber and is consumed
by the `nexus parse` command, the GUI replay system, and post-simulation
analysis tools.

**Source files:** `kernel/src/logging.rs` (trace writer), `trace/src/lib.rs`
(trace reader and types), `trace/src/parse.rs` (parse command implementation).

---

## File Structure

A `.nxs` file consists of a header followed by a sequence of binary-encoded
events.

### Header

The trace header contains metadata needed to interpret events:

| Field | Type | Description |
|-------|------|-------------|
| Node names | `Vec<String>` | Sorted list of all node names in the simulation |
| Channel names | `Vec<String>` | Sorted list of all channel names |
| Timestep count | `u64` | Total number of timesteps configured |
| Max energy per node | `Vec<u64>` | Maximum battery capacity in nJ for each node (0 if no battery) |

The header is encoded with `bincode` and written at the start of the file.

### Events

After the header, events are written sequentially as `bincode`-encoded
records. Each event carries a discriminant tag identifying its type.

---

## Event Types

### MessageSent (TX)

Emitted when a protocol writes to a channel file and the message is queued
for delivery.

| Field | Description |
|-------|-------------|
| timestep | Simulation timestep when the message was sent |
| message_id | Unique identifier for correlating TX/RX/Drop events |
| source_node | Index into the header's node name list |
| channel | Index into the header's channel name list |
| payload | Raw message bytes |

### MessageRecv (RX)

Emitted when a message is delivered to a subscriber's mailbox.

| Field | Description |
|-------|-------------|
| timestep | Simulation timestep when the message was delivered |
| message_id | Matches the TX event's message_id |
| dest_node | Index into the header's node name list |
| channel | Index into the header's channel name list |
| rssi_dbm | Received signal strength in dBm |
| snr_db | Signal-to-noise ratio in dB |
| payload | Raw message bytes (may differ from TX if bit errors applied) |

### MessageDropped (Drop)

Emitted when a message is lost during link simulation.

| Field | Description |
|-------|-------------|
| timestep | Simulation timestep |
| message_id | Matches the TX event's message_id |
| dest_node | Intended recipient |
| channel | Channel index |
| reason | Drop reason (see below) |

**Drop reasons:**

| Reason | Description |
|--------|-------------|
| `BelowSensitivity` | RSSI below receiver's `rx_min_dbm` threshold |
| `PacketLoss` | Dropped by the packet loss probability expression |
| `TtlExpired` | Message exceeded its time-to-live |
| `BufferFull` | Subscriber's message buffer was full |

### PositionUpdate

Emitted each timestep for nodes with active (non-Static) motion patterns,
and when a protocol writes to a `ctl.pos/*` control file.

| Field | Description |
|-------|-------------|
| timestep | Simulation timestep |
| node | Node index |
| x, y, z | Position coordinates (f64) |
| az, el, roll | Orientation in degrees (f64) |

### EnergyUpdate

Emitted each timestep for nodes with a battery configured.

| Field | Description |
|-------|-------------|
| timestep | Simulation timestep |
| node | Node index |
| charge_nj | Current battery charge in nanojoules (u64) |

### MotionUpdate

Emitted when a node's motion pattern changes (via `ctl.pos/motion` write or
implicit reset from absolute position write).

| Field | Description |
|-------|-------------|
| timestep | Simulation timestep |
| node | Node index |
| pattern | Motion pattern spec string |

---

## Using `nexus parse`

The `parse` command reads a `.nxs` file and outputs events in a human-readable
or machine-readable format. See [cli-reference.md](cli-reference.md) for
full flag documentation.

### Quick Examples

```bash
# Inspect trace metadata only
nexus parse trace.nxs --header_only

# Show all TX and RX events
nexus parse trace.nxs --events tx,rx

# Filter by node and timestep range, output as JSON Lines
nexus parse trace.nxs --nodes sensor --from 0 --to 1000 --output jsonlines

# Decode payloads with a custom adapter
nexus parse trace.nxs --adapter "python3 my_decoder.py"
```

### External Adapters

The `--adapter` flag specifies a command that receives each message payload
on stdin and should output a decoded representation on stdout. This is useful
for protocol-specific payload interpretation without modifying Nexus.

---

## Replay

The `nexus replay` command reads the TX events from a trace and re-issues
them to the kernel without running protocol processes. This reproduces the
exact message delivery sequence for analysis or visualization.

During replay:
- Only TX (MessageSent) events are replayed
- Position, energy, and motion events are skipped
- The replayed simulation writes a new trace file for comparison

---

## GUI Integration

The GUI uses trace files in two ways:

1. **Live simulation**: The kernel's trace layer sends `GuiEvent` values
   through a `crossbeam_channel` to the GUI thread in real time. The GUI
   updates node state, message logs, and the grid visualization as events
   arrive.

2. **Replay mode**: The GUI reads a `.nxs` file and builds an index of
   timestep boundaries for O(1) scrubbing. State is reconstructed by
   replaying all events up to the selected timestep.
