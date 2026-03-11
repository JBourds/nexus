#[derive(Debug)]
pub enum KernelMessage {
    HealthCheck,
    UpdateResources,
    Freeze,
    Unfreeze,
    /// Freeze a single node's cgroup (called when that node runs out of charge).
    FreezeNode(String),
    /// Unfreeze a single node's cgroup (called when charge recovers above threshold).
    UnfreezeNode(String),
    /// Kill a frozen node's processes and respawn them (realistic restart).
    RespawnNode(String),
    Shutdown,
}

#[derive(Debug)]
pub enum StatusMessage {
    Ok,
    PrematureExit,
    /// Response to RespawnNode with the PID changes.
    Respawned {
        node: String,
        pid_changes: Vec<(u32, u32)>,
    },
}
