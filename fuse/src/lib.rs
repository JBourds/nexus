pub mod errors;
pub mod fs;
pub mod socket;
use std::{
    collections::HashMap,
    os::unix::net::UnixDatagram,
    sync::mpsc::{Receiver, Sender},
};

use config::ast;

use crate::fs::ControlSignal;

pub type Mode = i32;
pub type PID = u32;
pub type Inode = u64;
pub type ChannelId = (PID, ast::ChannelHandle);

/// Synchronization through the kernel and the FS occurs through several pairs
/// of channels which requests and responses get sent. This ensures that IO
/// happens cleanly on a timestep boundary since the threads spawned by the FS
/// to service reads/writes will wait for responses.
#[derive(Debug)]
pub struct KernelControlFile {
    pub request: Receiver<()>,
    pub ack: Sender<ControlSignal>,
}

impl KernelControlFile {
    fn new(request: Receiver<()>, ack: Sender<ControlSignal>) -> Self {
        Self { request, ack }
    }
}

/// Handle for a specific node attached to a channel.
#[derive(Debug)]
pub struct KernelChannelHandle {
    /// Name of the node participating on the channel.
    pub node: ast::NodeHandle,
    /// Control file for receiving and responsing to read requests issued by the
    /// FS worker thread.
    pub read: KernelControlFile,
    /// Control file for receiving and responsing to write requests issued by
    /// the FS worker thread.
    pub write: KernelControlFile,
    /// Unix datagram socket for actually sending/transmitting data over.
    pub file: UnixDatagram,
}

pub type KernelChannels = HashMap<(PID, ast::ChannelHandle), KernelChannelHandle>;
