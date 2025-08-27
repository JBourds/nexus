use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("Error creating UNIX datagram socket.")]
    DatagramCreation,
    #[error("Duplicate channel mapping.")]
    DuplicateChannel,
    #[error("Invalid channel mode `{0}`.")]
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
    #[error("Failed to write to channel \"`{channel_name}`\".\nError: `{ioerr}`")]
    SocketWriteError {
        ioerr: io::Error,
        channel_name: String,
    },
    #[error("Failed to read from channel \"`{channel_name}`\".\nError: `{ioerr}`")]
    SocketReadError {
        ioerr: io::Error,
        channel_name: String,
    },
    #[error("Expected to write `{expected}` bytes but wrote `{actual}`")]
    WriteSizeMismatch { expected: usize, actual: usize },
    #[error("Expected to read `{expected}` bytes but read `{actual}`")]
    ReadSizeMismatch { expected: usize, actual: usize },
    #[error("Nothing to read")]
    NothingToRead,
}
