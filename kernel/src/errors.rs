use config::ast;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Failed to initialize kernel due to `{0}`")]
    KernelInit(ConversionError),
}

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("Failed to convert link `{0}` to handle")]
    LinkHandleConversion(ast::LinkHandle),
    #[error("Failed to convert node `{0}` to handle")]
    NodeHandleConversion(ast::NodeHandle),
}
