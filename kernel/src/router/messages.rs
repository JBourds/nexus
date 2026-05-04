/// Single input stream into the router thread. FUSE filesystem events and
/// kernel-side control signals are merged into one mpsc so the router
/// blocks on exactly one `recv()` and responds to whichever arrives first.
/// Without this merge, the router would either need a `select`-style
/// primitive (not in std) or have to poll, both of which reintroduce
/// latency or busy-wait the very change this enum exists to remove.
#[derive(Debug)]
pub enum RouterInput {
    Shutdown,
    /// Wake the router; advance simulated time per the shared `current_ts`
    /// atomic. Replaces the old `Poll(u64)` variant whose embedded timestep
    /// required the kernel main thread to wait for a synchronous reply on
    /// every spin iteration.
    Tick,
    /// Remap PIDs in handles and fuse_mapping after a process respawn.
    RemapPids(Vec<(u32, u32)>),
    /// FUSE filesystem event from a protocol process. The FUSE filesystem
    /// is generic over a `From<FsMessage>` sender type and constructs this
    /// variant directly via `Into`, so events reach the router in one mpsc
    /// hop with no forwarder thread.
    Fs(fuse::FsMessage),
}

impl From<fuse::FsMessage> for RouterInput {
    fn from(msg: fuse::FsMessage) -> Self {
        Self::Fs(msg)
    }
}

#[derive(Debug)]
pub enum RouterMessage {
    /// Acknowledgement that PID remapping is complete.
    PidsRemapped,
}

/// Energy depletion and recovery events from a `step()` call. Pushed
/// asynchronously by the router on a dedicated channel; the kernel main
/// thread drains the receiver each tick. Decoupling these from the Tick
/// reply means Tick is fire-and-forget.
#[derive(Debug)]
pub struct EnergyEvents {
    /// Node names that ran out of charge this step.
    pub depleted: Vec<String>,
    /// Node names that recovered above their restart threshold this step.
    pub recovered: Vec<String>,
}
