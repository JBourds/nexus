use std::io;
use std::sync::mpsc::{RecvError, SendError};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StatusError {
    #[error("Error sending kernel message: {0:#?}")]
    KernelSendError(SendError<crate::status::KernelMessage>),
    #[error("Error sending status message: {0:#?}")]
    StatusSendError(SendError<crate::status::StatusMessage>),
    #[error("Error receiving message: {0:#?}")]
    RecvError(RecvError),
    #[error("Error creating thread: {0}")]
    ThreadCreation(io::Error),
}
