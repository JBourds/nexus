pub mod channel;
pub mod errors;
pub mod file;
pub mod fs;
use std::sync::mpsc;

use config::ast::{self, TimeUnit};
use fuser::ReplyWrite;

pub type Mode = i32;
pub type PID = u32;
pub type Inode = u64;
pub type ChannelId = (PID, ast::ChannelHandle);
pub type KernelChannels = (mpsc::Sender<KernelMessage>, mpsc::Receiver<FsMessage>);
pub type FsChannels = (mpsc::Sender<FsMessage>, mpsc::Receiver<KernelMessage>);

pub mod ctrl_files;

#[derive(Clone, Debug)]
pub enum KernelMessage {
    Exclusive(Message),
    Shared(Message),
    Empty(Message),
}

#[derive(Debug)]
pub enum FsMessage {
    Write(Message),
    Read(Message),
    Sleep(SleepEvent),
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: ChannelId,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct SleepEvent {
    /// PID of the node requesting a sleep
    pub pid: PID,
    /// Integer value written
    pub val: u64,
    /// Time unit to use
    pub unit: TimeUnit,
    /// Whether the sleep value is an offset from node's current time or absolute
    pub is_relative: bool,
    /// Length of the original write payload, returned to the caller via
    /// `reply.written()` so glibc's write loop sees the full request as
    /// consumed and doesn't reissue the syscall.
    pub bytes_consumed: u32,
    /// FUSE object for marking write as concluded
    pub reply: ReplyWrite,
}
