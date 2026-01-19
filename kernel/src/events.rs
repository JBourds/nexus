//! events.rs
//! Self-queued events by the kernel for some point in the future.

#[derive(Debug)]
pub enum Event {
    UpdateResources,
}
