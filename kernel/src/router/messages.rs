pub enum KernelMessage {
    Shutdown,
    Poll(u64),
}

pub enum RouterMessage {}
