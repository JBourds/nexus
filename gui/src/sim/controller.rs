use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use crossbeam_channel::Receiver;

use crate::sim::bridge::GuiEvent;

/// Controls a running simulation, allowing the GUI to receive events
/// and request early termination.
pub struct SimController {
    pub rx: Receiver<GuiEvent>,
    abort: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SimController {
    pub fn new(rx: Receiver<GuiEvent>, abort: Arc<AtomicBool>, handle: JoinHandle<()>) -> Self {
        Self {
            rx,
            abort,
            handle: Some(handle),
        }
    }

    /// Non-blocking drain of all pending events.
    pub fn poll_events(&self) -> Vec<GuiEvent> {
        self.rx.try_iter().collect()
    }

    /// Signal the simulation to stop early.
    pub fn stop(&self) {
        self.abort.store(true, Ordering::Relaxed);
    }

    /// Check if the simulation thread has finished.
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| h.is_finished())
    }
}

impl Drop for SimController {
    fn drop(&mut self) {
        self.abort.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
