use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("Duplicate channel mapping.")]
    DuplicateChannel,
    #[error("Invalid channel mode `{0}`.")]
    InvalidMode(i32),
}

#[derive(Error, Debug)]
pub enum FsError {
    #[error("Failed to mount at \"`{root}`.\nError: {err}\"")]
    MountError { root: PathBuf, err: io::Error },
    #[error("Failed to create directory at \"`{dir}`\"\n{err:#?}")]
    CreateDirError { dir: PathBuf, err: io::Error },
    #[error("Kernel shutdown. Error on read request: {0:#?}")]
    KernelShutdown(Box<dyn std::error::Error>),
}
