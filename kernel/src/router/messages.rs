#[derive(Debug)]
pub enum KernelMessage {
    Shutdown,
    /// Wake the router; advance simulated time per the shared `current_ts`
    /// atomic. Replaces the old `Poll(u64)` variant whose embedded timestep
    /// required the kernel main thread to wait for a synchronous reply on
    /// every spin iteration.
    Tick,
    /// Remap PIDs in handles and fuse_mapping after a process respawn.
    RemapPids(Vec<(u32, u32)>),
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
