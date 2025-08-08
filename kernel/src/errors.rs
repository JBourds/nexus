use config::ast;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Failed to initialize kernel due to `{0}`")]
    KernelInit(ConversionError),
    #[error("Error encountered with socket file: `{0}`")]
    FileError(SocketError),
}

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("Failed to convert link `{0}` to handle")]
    LinkHandleConversion(ast::LinkHandle),
    #[error("Failed to convert node `{0}` to handle")]
    NodeHandleConversion(ast::NodeHandle),
}

#[derive(Error, Debug)]
pub enum SocketError {
    #[error("Failed to write socket \"`{link_name}`\" for pid `{pid}`")]
    SocketWriteError { pid: fuse::PID, link_name: String },
    #[error("Expected to write `{expected}` bytes but wrote `{actual}`")]
    WriteSizeMismatch { expected: usize, actual: usize },
    #[error("Expected to read `{expected}` bytes but read `{actual}`")]
    ReadSizeMismatch { expected: usize, actual: usize },
    #[error("Failed to read socket \"`{link_name}`\" for pid `{pid}`")]
    SocketReadError { pid: fuse::PID, link_name: String },
}
