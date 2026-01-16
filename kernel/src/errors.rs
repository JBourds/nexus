use bincode::error::DecodeError;
use std::path::PathBuf;
use std::{io, process::Output};

use config::ast;
use fuse::PID;

use thiserror::Error;

use crate::router::RouterError;
use crate::status::errors::StatusError;

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
    #[error("Error from router server {0:#?}.")]
    RouterError(RouterError),
    #[error("Error from status server {0:#?}.")]
    StatusError(StatusError),
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
    #[error("Error from routing server: {0}")]
    RouterError(RouterError),
    #[error("Error from status server: {0}")]
    StatusError(StatusError),
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
