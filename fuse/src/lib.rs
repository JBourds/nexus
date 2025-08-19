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
pub type KernelChannels = HashMap<
    (PID, ast::ChannelHandle),
    (
        ast::NodeHandle,
        Sender<ControlSignal>,
        Receiver<()>,
        UnixDatagram,
    ),
>;
