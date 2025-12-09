use std::num::NonZeroUsize;

use config::ast;
use libc::{O_RDONLY, O_RDWR, O_WRONLY};

use crate::{PID, errors::ChannelError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChannelMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
    ReplayWrites,
    FuzzWrites,
}

/// Necessary handles to identify each channel.
#[derive(Debug)]
pub struct NexusChannel {
    /// Node's name
    pub node: ast::NodeHandle,
    /// Process ID of the protocol
    pub pid: PID,
    /// Channel name (corresponds to file name shown)
    pub channel: ast::ChannelHandle,
    /// Available link operations
    pub mode: ChannelMode,
    /// Maximum size of a message along this channel
    pub max_msg_size: NonZeroUsize,
}

impl TryFrom<i32> for ChannelMode {
    type Error = ChannelError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            O_RDONLY => Ok(Self::ReadOnly),
            O_WRONLY => Ok(Self::WriteOnly),
            O_RDWR => Ok(Self::ReadWrite),
            _ => Err(Self::Error::InvalidMode(value)),
        }
    }
}
