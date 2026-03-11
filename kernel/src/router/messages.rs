#[derive(Debug)]
pub enum KernelMessage {
    Shutdown,
    Poll(u64),
    /// Remap PIDs in handles and fuse_mapping after a process respawn.
    RemapPids(Vec<(u32, u32)>),
}

#[derive(Debug)]
pub enum RouterMessage {
    /// Nodes that newly depleted or recovered their charge this timestep.
    EnergyEvents {
        /// Node names that just ran out of charge.
        depleted: Vec<String>,
        /// Node names that recovered above their restart threshold.
        recovered: Vec<String>,
    },
    /// Acknowledgement that PID remapping is complete.
    PidsRemapped,
}
