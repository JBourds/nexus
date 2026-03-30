# Distributed Simulation Design

## Status: Draft / RFC

## Motivation

Nexus currently runs its routing engine on a single thread. For simulations with
thousands of nodes and many channels, this becomes a bottleneck — `step()` must
process all energy ticks, position updates, message expiry, and delivery
sequentially. By partitioning work across CPU cores, we can scale to larger
simulations while preserving determinism.

The design draws inspiration from ns-3's distributed simulation (MPI-based
partitioning across network links) but adapted to Nexus's architecture where
protocol processes are real OS processes communicating via FUSE.

## Current Architecture Summary

```
Kernel (main thread)
  │
  ├─ RoutingServer (dedicated thread)
  │    ├─ Owns: RoutingTable, MessageQueue (global BinaryHeap), Mailboxes[]
  │    ├─ step() per timestep:
  │    │    1. energy_mgr.tick()           — update all node energy
  │    │    2. apply_all_motions_and_log() — update all node positions
  │    │    3. expire_messages()           — scan all mailboxes
  │    │    4. deliver_queued_messages()   — drain global priority queue
  │    └─ Handles FUSE writes/reads between steps
  │
  ├─ StatusServer (dedicated thread)
  └─ NexusFs / FUSE (dedicated thread)
```

### Cross-Channel Coupling Points

Channels are **mostly independent**: separate routes, separate mailboxes,
separate buffer limits. But three pieces of shared state couple them:

| Shared State | Why It Couples | Access Pattern |
|---|---|---|
| **Node energy** (`nodes[i].energy`) | TX/RX costs deducted from a single pool across all channels; node death affects all channels | Write on TX (queue time), write on RX (delivery time), read/write in `energy_mgr.tick()` |
| **Node position** (`nodes[i].position`) | Distance calculations for link simulation use the same position regardless of channel | Write once per step in `apply_all_motions_and_log()`, read on queue (exclusive) and delivery (shared) |
| **Global message queue** (`queued: BinaryHeap`) | All channels share one priority queue ordered by activation timestep | Write on queue, read/drain on delivery |

## Proposed Design

### Core Idea: Partition by Channel, Synchronize at Step Boundaries

Each **worker** owns a disjoint subset of channels and is responsible for:
- Queuing messages written to its channels
- Delivering messages from its local priority queue
- Expiring messages in its mailboxes
- Running link simulation for its channels

Workers run in parallel within each timestep. Shared node state (energy,
position) is synchronized at step boundaries via a coordinator.

### Partitioning Strategy

#### Phase 1: Channel-Level Partitioning

Assign each channel to exactly one worker. A worker owns:
- The subset of `RoutingTable::entries` for its channels
- Local `MessageQueue` (priority queue) for its channels
- Local `Mailbox`es for handles belonging to its channels
- Read-only snapshot of node positions/energy for link simulation

```
             ┌──────────────┐
             │  Coordinator  │
             │  (main thread)│
             └──┬───┬───┬───┘
        sync    │   │   │   sync
       ┌────────┘   │   └────────┐
       ▼            ▼            ▼
  ┌─────────┐ ┌─────────┐ ┌─────────┐
  │Worker 0 │ │Worker 1 │ │Worker 2 │
  │Ch: A, B │ │Ch: C, D │ │Ch: E    │
  │Nodes:   │ │Nodes:   │ │Nodes:   │
  │ 0,1,2   │ │ 2,3,4   │ │ 0,3,4,5│
  └─────────┘ └─────────┘ └─────────┘
       Note: Node 2 appears in Workers 0 and 1
```

**Channel assignment heuristic** (computed at startup):
1. Build a weighted channel graph: weight = number of subscribers x publishers
   (proxy for message volume)
2. Greedily assign channels to workers to balance total weight
3. Prefer co-locating channels that share many nodes (reduces sync overhead)

#### Phase 2: Intra-Channel Partitioning (Large Channels)

When a single channel has e.g. 1000 nodes, one worker becomes a bottleneck.
Split the channel's subscriber set across workers:

- For **exclusive channels**: partition subscribers into shards. Each shard
  independently runs link simulation and manages its own mailboxes. No
  cross-shard dependency (exclusive channels have no collision model).

- For **shared channels**: more complex due to collision semantics. Options:
  1. **Spatial partitioning**: divide nodes by geographic region. Collisions
     only happen between nodes close enough for signal overlap. Nodes in
     different spatial cells are independent.
  2. **Two-phase delivery**: all workers queue messages independently, then a
     merge step combines colliding messages per mailbox before delivery.

### Synchronization Protocol

Each timestep proceeds in phases with barriers between them:

```
 ┌─────────────────── Timestep T ───────────────────────┐
 │                                                       │
 │  Phase 0: Coordinator updates positions & energy      │
 │           Broadcasts snapshots to all workers         │
 │           ┌─ barrier ─┐                               │
 │                                                       │
 │  Phase 1: Workers process FUSE writes (queue msgs)    │
 │           Workers run link sim for exclusive channels  │
 │           (parallel, no sync needed)                   │
 │           ┌─ barrier ─┐                               │
 │                                                       │
 │  Phase 2: Workers expire messages                     │
 │           Workers deliver from local queues            │
 │           Workers report energy deltas back            │
 │           (parallel, no sync needed)                   │
 │           ┌─ barrier ─┐                               │
 │                                                       │
 │  Phase 3: Coordinator collects energy deltas           │
 │           Applies them to canonical node state          │
 │           Detects node death/recovery                  │
 │                                                       │
 └───────────────────────────────────────────────────────┘
```

### Data Flow Detail

#### FUSE Message Routing

Currently, the FUSE thread sends all `FsMessage`s to the single router via one
`mpsc` channel. For distributed simulation:

**Option A — Demux in FUSE thread**: FUSE looks up which worker owns the
destination channel and sends directly to that worker's inbox. Requires FUSE to
know the channel-to-worker mapping.

**Option B — Central demux**: Keep single FUSE→Router channel. Coordinator
(or a lightweight dispatcher) reads messages and fans out to workers. Simpler
but adds a serialization point.

**Recommendation**: Option A. The FUSE mapping already resolves
`(PID, channel_name) → handle_ptr`, and we add `handle_ptr → worker_id`.
The current `mpsc::Sender` (which is already `Clone`-able since it's the
std `mpsc` multi-producer sender) becomes one sender per worker. FUSE holds
a `Vec<Sender<FsMessage>>` indexed by worker ID.

#### Node State Snapshots

Workers need read-only access to node positions and energy for link simulation.
Two approaches:

**Option A — Shared read-only snapshots**: Coordinator produces an
`Arc<NodeSnapshot>` each step. Workers hold `Arc` references. Zero-copy but
requires allocation per step.

**Option B — Per-worker copies**: Each worker maintains its own `Vec<Node>`
copy. Coordinator broadcasts deltas (position changes, energy changes) via
channels. Workers apply deltas locally. Avoids allocation but adds message
volume.

**Recommendation**: Option A for positions (read-only, small, changes every
step for mobile nodes). Energy deltas use a lightweight message since only
TX/RX events change energy and those are sparse.

#### Energy Accounting

Energy is the trickiest shared state because TX drains happen during write
processing (Phase 1) and RX drains happen during delivery (Phase 2), both
in parallel across workers.

**Design**: Each worker tracks energy deltas locally:
```rust
struct EnergyDelta {
    node_idx: usize,
    delta_nj: i64,  // negative for drain, positive for source
}
```

After Phase 2, workers send their deltas to the coordinator. The coordinator:
1. Applies all deltas to the canonical `nodes[i].energy`
2. Runs `energy_mgr.tick()` (sources/sinks) on the canonical state
3. Detects death/recovery transitions
4. Broadcasts updated energy state for next step

**Determinism**: Deltas are applied in a fixed order (worker 0, then 1, then 2,
etc.) to ensure identical results regardless of timing. Death detection happens
after ALL deltas are applied, matching the current behavior where all TX/RX
in a step complete before the next `energy_mgr.tick()`.

#### Read Requests (Protocol Reads from FUSE)

When a protocol reads from a channel file, it blocks until a message is
available. Currently:
1. FUSE sends `FsMessage::Read` to router
2. Router checks mailbox, sends `KernelMessage::{Exclusive,Shared,Empty}` back
3. FUSE unblocks the protocol

For distributed simulation: the read goes to the worker owning that channel.
The worker checks its local mailbox and responds directly to FUSE. This is
fully local — no cross-worker communication needed.

**Implementation**: Each worker has its own `Sender<KernelMessage>` back to
FUSE. Workers share a single FUSE `Sender` (it's already multi-producer). Or
each worker gets its own response channel and FUSE multiplexes.

### Concrete Implementation Plan

#### Step 1: Introduce `Worker` Struct

```rust
struct Worker {
    id: usize,
    channels: Vec<ChannelIdx>,              // Channels owned by this worker
    handles: Vec<usize>,                    // Handle indices for owned channels
    routing_entries: Vec<(ChannelIdx, ChannelRoutes)>,
    queued: BinaryHeap<(Reverse<Timestep>, usize, AddressedMsg)>,
    mailboxes: Vec<Option<VecDeque<QueuedMessage>>>,  // Sparse: only owned handles
    node_snapshot: Arc<Vec<Node>>,          // Read-only positions/energy
    energy_deltas: Vec<EnergyDelta>,        // Accumulated this step

    // Communication
    fuse_rx: crossbeam::channel::Receiver<FsMessage>,
    fuse_tx: mpsc::Sender<fuse::KernelMessage>,
    coord_rx: crossbeam::channel::Receiver<CoordMessage>,
    coord_tx: crossbeam::channel::Sender<WorkerMessage>,
}
```

#### Step 2: Introduce `Coordinator`

```rust
struct Coordinator {
    workers: Vec<WorkerHandle>,
    nodes: Vec<Node>,                       // Canonical node state
    energy_mgr: EnergyManager,
    handle_to_worker: Vec<usize>,           // handle_ptr → worker_id
    barrier: Arc<Barrier>,                  // Phase synchronization
}

struct WorkerHandle {
    thread: JoinHandle<()>,
    tx: crossbeam::channel::Sender<CoordMessage>,
    rx: crossbeam::channel::Receiver<WorkerMessage>,
}

enum CoordMessage {
    StepBegin { timestep: u64, node_snapshot: Arc<Vec<Node>> },
    Shutdown,
}

enum WorkerMessage {
    StepComplete { energy_deltas: Vec<EnergyDelta> },
}
```

#### Step 3: Replace Global State with Worker-Local State

Refactor `Router` internals:
- Extract `queue_message()`, `deliver_queued_messages()`, `expire_messages()`,
  `deliver_msg()`, `deliver_shared_msg()` into methods on `Worker`
- Keep link simulation functions as pure functions (they already take node
  positions as parameters)
- `Rc<[u8]>` in shared channel messages must become `Arc<[u8]>` for
  cross-thread safety

#### Step 4: FUSE Demultiplexing

Modify `NexusFs` to hold a routing table from `handle_ptr → worker_id`:
```rust
struct NexusFs {
    worker_txs: Vec<crossbeam::channel::Sender<FsMessage>>,
    handle_to_worker: Vec<usize>,
    // ...existing fields...
}
```

On write/read: look up worker, send to that worker's channel.

#### Step 5: Barrier-Based Step Synchronization

Use `std::sync::Barrier` (or crossbeam's `WaitGroup`) for phase
synchronization:

```rust
// In Coordinator::step()
fn step(&mut self, timestep: u64) {
    // Phase 0: update positions, energy tick, broadcast snapshot
    self.apply_all_motions();
    self.energy_mgr.tick(&mut self.nodes, timestep, self.timestep_ns);
    let snapshot = Arc::new(self.nodes.clone());
    for w in &self.workers {
        w.tx.send(CoordMessage::StepBegin { timestep, node_snapshot: snapshot.clone() });
    }

    // Workers execute Phases 1-2 autonomously

    // Phase 3: collect results
    for w in &self.workers {
        let WorkerMessage::StepComplete { energy_deltas } = w.rx.recv().unwrap();
        for delta in energy_deltas {
            self.apply_energy_delta(delta);
        }
    }
}
```

#### Step 6: Partitioner

```rust
struct Partitioner;

impl Partitioner {
    /// Assigns channels to workers, balancing load and minimizing cross-worker
    /// node sharing.
    fn partition(
        channels: &[Channel],
        nodes: &[Node],
        routing_table: &RoutingTable,
        num_workers: usize,
    ) -> Vec<Vec<ChannelIdx>> {
        // 1. Compute channel weights (subscribers * publishers * avg_message_rate)
        // 2. Build affinity graph (edge weight = shared nodes between channels)
        // 3. Greedy balanced partitioning with affinity preference
        //    (or use METIS-style graph partitioning for better results)
        todo!()
    }
}
```

### Migration Path

The implementation can be done incrementally without breaking existing behavior:

1. **Refactor Router internals** into composable pieces (pure functions, worker
   methods). No behavioral change. Ship and test.

2. **Add Worker struct** that wraps the extracted methods. Single-worker mode
   behaves identically to current code. Ship and test.

3. **Add Coordinator** and multi-worker support behind a feature flag
   (`--workers N` CLI flag, default 1). Ship and test.

4. **Add channel partitioner**. Ship and test with various topologies.

5. **Add intra-channel splitting** (Phase 2) for large single-channel
   simulations.

### Determinism Guarantees

Determinism is critical for Nexus (replay, debugging). The distributed design
preserves it:

- **Fixed partitioning**: channel-to-worker assignment is deterministic
  (computed from config, not runtime state)
- **Ordered energy application**: deltas applied in worker-id order
- **Per-worker sequence counters**: within a worker, message ordering matches
  single-threaded behavior (same priority queue logic)
- **Cross-worker message ordering**: messages between channels on different
  workers are ordered by (timestep, channel_idx, sequence) — the timestep
  provides a synchronization point
- **Barrier synchronization**: all workers see the same node snapshot at each
  step start — no race conditions on position/energy reads

### Performance Considerations

**Expected speedup**: For N channels distributed across W workers, the per-step
work (link simulation, delivery, expiry) divides roughly by W. Synchronization
overhead is O(nodes) per step for snapshot creation and O(energy_events) for
delta collection.

**When it helps most**:
- Many independent channels (e.g., 50 frequency bands, each with ~20 nodes)
- Expensive link simulation (complex path loss models, bit error computation)
- Large exclusive channels (embarrassingly parallel delivery)

**When it helps least**:
- Single shared channel with all nodes (cannot partition without Phase 2)
- Very fast steps with little per-step work (sync overhead dominates)
- Few channels (limited parallelism)

**Memory overhead**: Each worker holds a read-only `Arc<Vec<Node>>` snapshot.
For 10,000 nodes at ~200 bytes each, that's ~2MB per snapshot — negligible.

### Key Dependency Changes

| Dependency | Current | Proposed |
|---|---|---|
| `std::sync::mpsc` | FUSE↔Router, Kernel↔Router, Kernel↔Status | Keep for Kernel↔Status. Replace FUSE↔Router with per-worker crossbeam channels |
| `Rc<[u8]>` (shared channel bufs) | Used in `QueuedMessage` | Must become `Arc<[u8]>` for `Send` across threads |
| `crossbeam` | Not currently used | Add for bounded MPMC channels (better perf than std mpsc for fan-out) |
| `std::sync::Barrier` | Not currently used | Phase synchronization between workers |

### Open Questions

1. **Trace logging**: Currently the router emits trace events sequentially.
   With multiple workers, events from different workers may interleave. Should
   we buffer per-worker and merge, or use a concurrent trace sink?

2. **PID remapping**: When a node respawns (new PID), the coordinator must
   update the FUSE demux table and the owning worker's handle mapping. This is
   rare but must be handled atomically relative to message routing.

3. **Dynamic rebalancing**: If message patterns shift during simulation (e.g.,
   one channel becomes very active), should we support re-partitioning? Initial
   answer: no, keep static partitioning for simplicity and determinism.

4. **Control file reads** (`ctl.time`, `ctl.pos`, etc.): These are currently
   handled by the router but don't route through channels. They should be
   handled by the coordinator or a dedicated lightweight handler, not workers.

5. **Replay/trace sources**: `ReplayTrace` mode reads from a file and injects
   messages. This needs to be demuxed to workers the same way FUSE messages are.
