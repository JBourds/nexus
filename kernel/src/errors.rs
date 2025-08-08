use config::ast;
use fuse::errors::SocketError;

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
