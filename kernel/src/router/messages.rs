pub enum KernelMessage {
    Shutdown,
    Poll(u64),
}

pub enum RouterMessage {
    /// Nodes that newly depleted or recovered their charge this timestep.
    EnergyEvents {
        /// Node names that just ran out of charge.
        depleted: Vec<String>,
        /// Node names that recovered above their restart threshold.
        recovered: Vec<String>,
    },
}
