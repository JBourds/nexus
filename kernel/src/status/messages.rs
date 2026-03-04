pub enum KernelMessage {
    HealthCheck,
    UpdateResources,
    Freeze,
    Unfreeze,
    /// Freeze a single node's cgroup (called when that node runs out of charge).
    FreezeNode(String),
    /// Unfreeze a single node's cgroup (called when charge recovers above threshold).
    UnfreezeNode(String),
    Shutdown,
}

pub enum StatusMessage {
    Ok,
    PrematureExit,
}
