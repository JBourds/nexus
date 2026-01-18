pub enum KernelMessage {
    HealthCheck,
    UpdateResources,
    Freeze,
    Unfreeze,
    Shutdown,
}

pub enum StatusMessage {
    Ok,
    PrematureExit,
}
