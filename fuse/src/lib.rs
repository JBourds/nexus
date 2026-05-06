pub mod channel;
pub mod errors;
pub mod file;
pub mod fs;

use config::ast::{self, TimeUnit};
use fuser::{ReplyData, ReplyWrite};

pub type Mode = i32;
pub type PID = u32;
pub type Inode = u64;
pub type ChannelId = (PID, ast::ChannelHandle);

pub mod ctrl_files;

/// Read request from FUSE to the kernel/router. The router calls
/// `reply.data(buf)` (or `reply.error(e)`) directly when it has the answer,
/// so the FUSE worker thread never blocks waiting for a reply. This is the
/// "top-half / bottom-half" split: the FUSE worker just dispatches a
/// `FsMessage::Read` and returns immediately; the router does the work and
/// calls the reply token from its own thread.
#[derive(Debug)]
pub struct ReadRequest {
    pub id: ChannelId,
    /// Bytes the syscall asked for. The router slices its message to this
    /// size and stashes any remainder in per-handle `unread_msg` state for
    /// the next read.
    pub size: u32,
    pub reply: ReplyData,
}

#[derive(Debug)]
pub enum FsMessage {
    Write(Message),
    Read(ReadRequest),
    Sleep(SleepEvent),
}

// `KernelMessage` (Exclusive/Shared/Empty) used to carry replies back from
// the kernel to the FUSE filesystem. It is gone now: the router invokes
// the per-request `ReplyData` token directly, so there is no separate
// reply channel and no reply enum.

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
