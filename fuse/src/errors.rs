use std::path::PathBuf;

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
