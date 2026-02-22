use std::{
    io,
    sync::mpsc::{RecvError, SendError},
};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("Attempt to write to unknown file: {0}")]
    UnknownFile(String),
    #[error("Invalid string from bytes: {0:#?}")]
    InvalidString(Vec<u8>),
    #[error("Invalid integer format: {0}")]
    InvalidInteger(String),
    #[error("Error sending fuse message: {0:#?}")]
    FuseSendError(SendError<fuse::KernelMessage>),
    #[error("Error sending kernel message: {0:#?}")]
    KernelSendError(SendError<crate::router::KernelMessage>),
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
    #[error("Error creating thread: {0}")]
    ThreadCreation(io::Error),
}
