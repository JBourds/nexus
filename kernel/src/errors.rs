use config::ast;
use fuse::{PID, errors::SocketError};

use thiserror::Error;

use crate::{router::Message, types::LinkHandle};

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Failed to initialize kernel due to `{0}`")]
    KernelInit(ConversionError),
    #[error("Error encountered when creating file poll.")]
    PollCreation,
    #[error("Error encountered when registering file to poll.")]
    PollRegistration,
    #[error("Error encountered when polling file.")]
    PollError,
    #[error("Error during message routing.")]
    RouterError(RouterError),
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
    },
    #[error("Failed to deliver queued messages.")]
    RouteError,
    #[error("Error encountered with socket file: `{0}`")]
    FileError(SocketError),
}
