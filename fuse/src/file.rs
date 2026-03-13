use fuser::FileType;
use std::{num::NonZeroUsize, time::SystemTime};

use fuser::FileAttr;

use crate::channel::ChannelMode;

/// Return the current user's UID and GID.
///
/// Centralises the two `unsafe` libc calls so the rest of the crate never
/// needs to touch them directly.
pub(crate) fn current_uid_gid() -> (u32, u32) {
    // SAFETY: getuid/getgid are always safe to call and cannot fail.
    unsafe { (libc::getuid(), libc::getgid()) }
}

/// Build a `FileAttr` populated with sensible defaults for a Nexus virtual
/// file or directory.
///
/// Callers override whichever fields differ from the defaults (e.g. `ino`,
/// `kind`, `perm`, `size`, `blocks`).
pub(crate) fn default_attr(
    ino: u64,
    kind: FileType,
    perm: u16,
    size: u64,
    blocks: u64,
) -> FileAttr {
    let now = SystemTime::now();
    let (uid, gid) = current_uid_gid();
    FileAttr {
        ino,
        size,
        blocks,
        atime: now,
        mtime: now,
        ctime: now,
        crtime: now,
        kind,
        perm,
        nlink: 2,
        uid,
        gid,
        rdev: 0,
        flags: 0,
        blksize: 512,
    }
}

/// Struct containing file system metadata and all queued messages
/// for a single virtual file corresponding to the view of one process on a
/// given channel.
#[derive(Debug)]
pub(crate) struct NexusFile {
    pub mode: ChannelMode,
    pub attr: FileAttr,
    #[allow(unused)]
    pub max_msg_size: NonZeroUsize,
    pub unread_msg: Option<(usize, Vec<u8>)>,
}

impl NexusFile {
    pub(crate) fn new(max_msg_size: NonZeroUsize, mode: ChannelMode, ino: u64) -> Self {
        Self {
            mode,
            attr: default_attr(ino, FileType::RegularFile, 0o644, u16::MAX as u64, 1),
            max_msg_size,
            unread_msg: None,
        }
    }
}
