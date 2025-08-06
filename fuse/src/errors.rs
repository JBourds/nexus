use std::{fmt::Display, os};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum LinkError {
    #[error("Error creating UNIX datagram socket.")]
    DatagramCreation,
    #[error("Duplicate link mapping.")]
    DuplicateLink,
}
