pub mod channel;
pub mod errors;
pub mod file;
pub mod fs;
use std::sync::mpsc;

use config::ast;

pub type Mode = i32;
pub type PID = u32;
pub type Inode = u64;
pub type ChannelId = (PID, ast::ChannelHandle);
pub type KernelChannels = (mpsc::Sender<KernelMessage>, mpsc::Receiver<FsMessage>);
pub type FsChannels = (mpsc::Sender<FsMessage>, mpsc::Receiver<KernelMessage>);

#[derive(Clone, Debug)]
pub enum KernelMessage {
    Exclusive(Message),
    Shared(Message),
    Empty(Message),
}

impl KernelMessage {
    pub(crate) fn data(&self) -> &[u8] {
        match self {
            KernelMessage::Exclusive(message) => &message.data,
            KernelMessage::Shared(message) => &message.data,
            KernelMessage::Empty(message) => &message.data,
        }
    }
}

#[derive(Clone, Debug)]
pub enum FsMessage {
    Write(Message),
    Read(Message),
    Time(Message),
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: ChannelId,
    pub data: Vec<u8>,
}
