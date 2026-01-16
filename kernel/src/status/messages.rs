pub enum KernelMessage {
    HealthCheck,
    Freeze,
    Unfreeze,
    Shutdown,
}

pub enum StatusMessage {
    Ok,
    PrematureExit,
}
