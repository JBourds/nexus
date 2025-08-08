use std::io;
use std::path::PathBuf;

use super::PID;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LinkError {
    #[error("Error creating UNIX datagram socket.")]
    DatagramCreation,
    #[error("Duplicate link mapping.")]
    DuplicateLink,
    #[error("Invalid link mode `{0}`.")]
    InvalidMode(i32),
}

#[derive(Error, Debug)]
pub enum FsError {
    #[error("Failed to mount at \"`{0}`\"")]
    MountError(PathBuf),
    #[error("Failed to create directory at \"`{0}`\"")]
    CreateDirError(PathBuf),
}

#[derive(Error, Debug)]
pub enum SocketError {
    #[error("Failed to write socket \"`{link_name}`\" for pid `{pid}`.\nError: `{ioerr}`")]
    SocketWriteError {
        ioerr: io::Error,
        pid: PID,
        link_name: String,
    },
    #[error("Failed to read socket \"`{link_name}`\" for pid `{pid}`.\nError: `{ioerr}`")]
    SocketReadError {
        ioerr: io::Error,
        pid: PID,
        link_name: String,
    },
    #[error("Expected to write `{expected}` bytes but wrote `{actual}`")]
    WriteSizeMismatch { expected: usize, actual: usize },
    #[error("Expected to read `{expected}` bytes but read `{actual}`")]
    ReadSizeMismatch { expected: usize, actual: usize },
    #[error("Nothing to read")]
    NothingToRead,
}
