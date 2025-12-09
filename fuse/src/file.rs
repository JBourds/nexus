use fuser::FileType;
use std::{collections::VecDeque, num::NonZeroUsize, time::SystemTime};

use fuser::FileAttr;

use crate::KernelMessage;
use crate::channel::ChannelMode;

/// Struct containing file system metadata and all queued messages
/// for a single virtual file corresponding to the view of one process on a
/// given channel.
#[derive(Debug)]
pub(crate) struct NexusFile {
    pub mode: ChannelMode,
    pub attr: FileAttr,
    pub max_msg_size: NonZeroUsize,
    pub unread_msg: Option<(usize, Vec<u8>)>,
}

impl NexusFile {
    pub(crate) fn new(max_msg_size: NonZeroUsize, mode: ChannelMode, ino: u64) -> Self {
        let now = SystemTime::now();
        Self {
            mode,
            attr: FileAttr {
                ino,
                size: u16::MAX as u64,
                blocks: 1,
                atime: now,
                mtime: now,
                ctime: now,
                crtime: now,
                kind: FileType::RegularFile,
                perm: 0o644,
                nlink: 2,
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                rdev: 0,
                flags: 0,
                blksize: 512,
            },
            max_msg_size,
            unread_msg: None,
        }
    }
}
