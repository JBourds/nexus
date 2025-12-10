use std::sync::mpsc::{RecvError, SendError};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("Error sending message: {0:#?}")]
    SendError(SendError<fuse::KernelMessage>),
    #[error("Error receiving message: {0:#?}")]
    RecvError(RecvError),
    #[error("Failed to deliver queued messages.")]
    RouteError,
    #[error("Impossible error encountered during `step` function!")]
    StepError,
    #[error("Failed to create simulator publisher.")]
    SimulatorCreation,
    #[error("Failed to create replay publisher.")]
    ReplayCreation,
}
