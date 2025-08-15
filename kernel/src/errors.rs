use std::{io, process::Output};

use config::ast;
use fuse::{PID, errors::SocketError};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Failed to initialize kernel due to `{0:#?}`")]
    KernelInit(ConversionError),
    #[error("Protocol prematurely exited: `{self:#?}`")]
    ProcessExit {
        node: String,
        node_id: usize,
        protocol: String,
        pid: PID,
        output: Output,
    },
    #[error("Error during message routing `{0:#?}.")]
    RouterError(RouterError),
    #[error("Error encountered when creating file poll.")]
    PollCreation,
    #[error("Error encountered when registering file to poll.")]
    PollRegistration,
    #[error("Error encountered when polling file.")]
    PollError,
}

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("Failed to convert link `{0}` to handle")]
    LinkHandleConversion(ast::LinkHandle),
    #[error("Failed to convert node `{0}` to handle")]
    NodeHandleConversion(ast::NodeHandle),
}

#[derive(Error, Debug)]
pub enum RouterError {
    #[error(
        "Failed to route message from node `{node_name}`, PID `{sender}`, to link `{link_name}` at timestep `{timestep}`"
    )]
    SendError {
        sender: PID,
        node_name: String,
        link_name: String,
        timestep: u64,
        base: Box<Self>,
    },
    #[error("Failed to deliver queued messages.")]
    RouteError,
    #[error("Resource temporarily blocked.")]
    Busy,
    #[error("Error encountered with socket file: `{0:#?}`")]
    FileError(SocketError),
}

impl RouterError {
    pub fn recoverable(&self) -> bool {
        match self {
            Self::Busy => true,
            Self::SendError { base, .. } => base.recoverable(),
            Self::FileError(inner) => match inner {
                SocketError::NothingToRead => true,
                SocketError::SocketReadError { ioerr, .. }
                    if ioerr.kind() == io::ErrorKind::WouldBlock =>
                {
                    true
                }
                SocketError::SocketWriteError { ioerr, .. }
                    if ioerr.kind() == io::ErrorKind::WouldBlock =>
                {
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }
}
