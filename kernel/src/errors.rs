use bincode::error::DecodeError;
use std::path::PathBuf;
use std::{io, process::Output};

use config::ast;
use fuse::{PID, errors::SocketError};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Encountered error with checking elapsed time at timestep: {0}")]
    TimestepError(u64),
    #[error("Failed to initialize kernel due to `{0:#?}`")]
    KernelInit(ConversionError),
    #[error("Protocol prematurely exited: `{self:#?}`")]
    ProcessExit {
        node: String,
        protocol: String,
        pid: PID,
        output: Output,
    },
    #[error("Error during message routing {0:#?}.")]
    RouterError(RouterError),
    #[error("Error creating message source {0:#?}")]
    SourceError(SourceError),
    #[error("Error encountered when creating file poll.")]
    PollCreation,
    #[error("Error encountered when registering file to poll.")]
    PollRegistration,
    #[error("Error encountered when polling file.")]
    PollError,
}

#[derive(Error, Debug)]
pub enum SourceError {
    #[error("Failed to create source for simulated events.\n{0:#?}")]
    SimulatedEvents(io::Error),
    #[error("Failed to register file descriptor with poll.\n{0:#?}")]
    PollRegistration(io::Error),
    #[error("Error polling event sources: \n{0:#?}")]
    PollError(io::Error),
    #[error("Error while sending to router.")]
    RouterError(RouterError),
    #[error("Error found decoding replay log file: `{0:#?}`")]
    ReplayLogRead(DecodeError),
    #[error("Expected the `tx` logs for replay but found `rx` logs.")]
    InvalidLogType,
    #[error("Error found opening replay log file: `{0:#?}`")]
    ReplayLogOpen(io::Error),
    #[error("No replay log found at `{0:#?}`")]
    NonexistentReplayLog(PathBuf),
    #[error("No replay log to simulate writes from.")]
    NoReplayLog,
}

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("Failed to convert channel `{0}` to handle")]
    ChannelHandleConversion(ast::ChannelHandle),
    #[error("Failed to convert node `{0}` to handle")]
    NodeHandleConversion(ast::NodeHandle),
}

#[derive(Error, Debug)]
pub enum RouterError {
    #[error(
        "Failed to route message from node `{node_name}`, PID `{sender}`, to channel `{channel_name}` at timestep `{timestep}`"
    )]
    SendError {
        sender: PID,
        node_name: String,
        channel_name: String,
        timestep: u64,
        base: Box<Self>,
    },
    #[error("Failed to deliver queued messages.")]
    RouteError,
    #[error("Resource temporarily blocked.")]
    Busy,
    #[error("Error encountered with socket file: `{0:#?}`")]
    FileError(SocketError),
    #[error("Impossible error encountered during `step` function!")]
    StepError,
    #[error("Failed to create simulator publisher.")]
    SimulatorCreation,
    #[error("Failed to create replay publisher.")]
    ReplayCreation,
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
