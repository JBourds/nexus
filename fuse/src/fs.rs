use crate::channel::{ChannelMode, NexusChannel};
use crate::errors::{ChannelError, FsError};
use crate::file::{NexusFile, default_attr};
use crate::{ChannelId, FsChannels, FsMessage, KernelChannels, KernelMessage};
use config::ast::{self};
use fuser::ReplyWrite;
use std::num::NonZeroUsize;
use std::sync::mpsc;
use tracing::instrument;

use crate::Message;
use fuser::{
    BackgroundSession, FUSE_ROOT_ID, FileAttr, FileType, Filesystem, MountOption, ReplyAttr,
    ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, Request, consts::FOPEN_DIRECT_IO,
};
use libc::{EACCES, EISDIR, EMSGSIZE, ENOENT, O_APPEND};
use libc::{O_ACCMODE, O_RDONLY, O_WRONLY};
use std::cmp::min;
use std::ffi::OsStr;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use std::{collections::HashMap, path::PathBuf};

static INODE_GEN: AtomicU64 = AtomicU64::new(FUSE_ROOT_ID + 1);
const TTL: Duration = Duration::from_secs(1);

pub const CONTROL_FILES: [(&str, ChannelMode); 19] = [
    // Time control: read/write current virtual time per node
    ("ctl.time.us", ChannelMode::ReadWrite),
    ("ctl.time.ms", ChannelMode::ReadWrite),
    ("ctl.time.s", ChannelMode::ReadWrite),
    // Elapsed time since simulation start (read-only)
    ("ctl.elapsed.us", ChannelMode::ReadOnly),
    ("ctl.elapsed.ms", ChannelMode::ReadOnly),
    ("ctl.elapsed.s", ChannelMode::ReadOnly),
    // Energy (stubs, not yet implemented)
    ("ctl.energy_left", ChannelMode::ReadOnly),
    ("ctl.energy_state", ChannelMode::ReadWrite),
    // Absolute position and orientation (read/write in node's distance unit / degrees)
    ("ctl.pos.x", ChannelMode::ReadWrite),
    ("ctl.pos.y", ChannelMode::ReadWrite),
    ("ctl.pos.z", ChannelMode::ReadWrite),
    ("ctl.pos.az", ChannelMode::ReadWrite),
    ("ctl.pos.el", ChannelMode::ReadWrite),
    ("ctl.pos.roll", ChannelMode::ReadWrite),
    // Relative position delta (write-only; adds to current position and clears motion pattern)
    ("ctl.pos.dx", ChannelMode::WriteOnly),
    ("ctl.pos.dy", ChannelMode::WriteOnly),
    ("ctl.pos.dz", ChannelMode::WriteOnly),
    // Motion pattern (read/write; formats: none | velocity | linear | circle)
    ("ctl.pos.motion", ChannelMode::ReadWrite),
    // Power flows (read/write; runtime manipulation of power sources/sinks)
    ("ctl.power_flows", ChannelMode::ReadWrite),
];

pub fn control_files() -> Vec<String> {
    CONTROL_FILES
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect()
}

#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    attr: FileAttr,
    files: Vec<ast::ChannelHandle>,
    buffers: HashMap<ChannelId, NexusFile>,
    fs_side: FsChannels,
    kernel_side: Option<KernelChannels>,
    /// Shared queue of (old_pid, new_pid) pairs pushed by the router.
    pending_remaps: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl NexusFs {
    pub fn new(root: PathBuf, pending_remaps: Arc<Mutex<Vec<(u32, u32)>>>) -> Self {
        Self {
            root,
            attr: Self::root_attr(),
            pending_remaps,
            ..Default::default()
        }
    }

    /// Drain the shared remap queue and migrate FUSE buffer entries from
    /// old PIDs to new PIDs.
    fn apply_pending_remaps(&mut self) {
        let pairs: Vec<(u32, u32)> = {
            let Ok(mut queue) = self.pending_remaps.lock() else {
                return;
            };
            queue.drain(..).collect()
        };
        for (old_pid, new_pid) in pairs {
            let keys_to_migrate: Vec<String> = self
                .buffers
                .keys()
                .filter(|(pid, _)| *pid == old_pid)
                .map(|(_, channel)| channel.clone())
                .collect();
            for channel in keys_to_migrate {
                if let Some(file) = self.buffers.remove(&(old_pid, channel.clone())) {
                    self.buffers.insert((new_pid, channel), file);
                }
            }
        }
    }

    fn root_attr() -> FileAttr {
        default_attr(FUSE_ROOT_ID, FileType::Directory, 0o755, 0, 0)
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    fn get_or_make_inode(&mut self, name: String) -> u64 {
        if let Some(index) = self.files.iter().position(|file| **file == name) {
            index_to_inode(index)
        } else {
            self.files.push(name);
            next_inode()
        }
    }

    pub fn add_processes(mut self, pids: &[u32]) -> Self {
        for (file, mode) in CONTROL_FILES.iter() {
            let inode = self.get_or_make_inode(file.to_string());
            for &pid in pids {
                self.buffers.insert(
                    (pid, file.to_string()),
                    NexusFile::new(NonZeroUsize::new(1000).unwrap(), *mode, inode),
                );
            }
        }
        self
    }

    pub fn add_channels(mut self, channels: Vec<NexusChannel>) -> Result<Self, ChannelError> {
        for NexusChannel {
            pid,
            node: _,
            channel,
            mode,
            max_msg_size,
        } in channels
        {
            let key = (pid, channel.clone());
            let inode = self.get_or_make_inode(channel);
            if self
                .buffers
                .insert(key.clone(), NexusFile::new(max_msg_size, mode, inode))
                .is_some()
            {
                return Err(ChannelError::DuplicateChannel);
            }
        }
        Ok(self)
    }

    /// Request a message from the kernel for the channel identified by `id`.
    /// Performs blocking I/O on the channel to send a request and receive the
    /// response from the kernel.
    fn pull_message(fs_side: &mut FsChannels, id: ChannelId) -> Result<KernelMessage, FsError> {
        fs_side
            .0
            .send(FsMessage::Read(Message {
                id,
                data: Vec::new(),
            }))
            .map_err(|e| FsError::KernelShutdown(Box::new(e)))?;
        fs_side
            .1
            .recv()
            .map_err(|e| FsError::KernelShutdown(Box::new(e)))
    }

    /// Mount the filesystem without blocking, yield the background session it
    /// is mounted in, and return the kernel's end of
    pub fn mount(mut self) -> Result<(BackgroundSession, KernelChannels), FsError> {
        let options = vec![
            MountOption::FSName("nexus".to_string()),
            MountOption::AllowOther,
        ];
        let root = self.root.clone();
        if !root.exists() {
            fs::create_dir_all(&root).map_err(|err| FsError::CreateDirError {
                dir: root.clone(),
                err,
            })?;
        }
        let kernel_side =
            core::mem::take(&mut self.kernel_side).expect("must have created kernel channels");
        let sess =
            fuser::spawn_mount2(self, &root, &options).map_err(|err| FsError::MountError {
                root: root.clone(),
                err,
            })?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !root.exists() {
            if std::time::Instant::now() > deadline {
                return Err(FsError::MountTimeout { root });
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok((sess, kernel_side))
    }

    fn read_message(&mut self, reply: ReplyData, size: usize, key: ChannelId) {
        // Get the file containing all message buffers
        let Some(file) = self.buffers.get_mut(&key) else {
            reply.error(EACCES);
            return;
        };

        // Serve unread parts of previous message first
        if let Some((read_ptr, buf)) = &mut file.unread_msg {
            // EOF
            if *read_ptr == buf.len() {
                file.unread_msg = None;
                reply.data(&[]);
                return;
            }
            let remaining = buf.len() - *read_ptr;
            let read_size = min(remaining, size);
            let end = *read_ptr + read_size;
            reply.data(&buf.as_slice()[*read_ptr..end]);
            file.unread_msg = Some((end, std::mem::take(buf)));
            return;
        }

        // See if there is anything for client to read
        if let Ok(msg) = Self::pull_message(&mut self.fs_side, key) {
            let (allow_incremental_reads, msg) = match msg {
                KernelMessage::Exclusive(msg) => (true, msg.data),
                KernelMessage::Shared(msg) => (false, msg.data),
                KernelMessage::Empty(_) => {
                    reply.data(&[]);
                    return;
                }
            };
            let read_size = min(msg.len(), size);
            reply.data(&msg[..read_size as usize]);
            if allow_incremental_reads && read_size < msg.len() {
                // need to buffer remaining parts of the message
                file.unread_msg = Some((read_size, msg));
            }
            // else {
            //     // TODO: Python requires this because of the read
            //     // implementation- does this cause issues on other platforms?
            //
            //     // serve explicit EOF condition for next read
            //     file.unread_msg = Some((0, Self::EMPTY));
            // }
        } else {
            reply.data(&[]);
        }
    }
}

impl Default for NexusFs {
    fn default() -> Self {
        let root = expand_home(&PathBuf::from("~/nexus"));
        let (fs_tx, kernel_rx) = mpsc::channel();
        let (kernel_tx, fs_rx) = mpsc::channel();
        Self {
            root,
            attr: Self::root_attr(),
            files: Vec::default(),
            buffers: HashMap::default(),
            fs_side: (fs_tx, fs_rx),
            kernel_side: Some((kernel_tx, kernel_rx)),
            pending_remaps: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Filesystem for NexusFs {
    fn setattr(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        _size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        // Just return current attrs — we ignore truncate/chmod/etc.
        self.getattr(req, ino, _fh, reply);
    }

    #[instrument(skip_all)]
    fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        self.apply_pending_remaps();
        if parent != FUSE_ROOT_ID {
            reply.error(ENOENT);
            return;
        }
        let file = name.to_string_lossy().into_owned();
        if let Some(file) = self.buffers.get(&(req.pid(), file)) {
            reply.entry(&TTL, &file.attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    #[instrument(skip_all)]
    fn getattr(&mut self, req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        self.apply_pending_remaps();
        match ino {
            FUSE_ROOT_ID => reply.attr(&TTL, &self.attr),
            _ => {
                let index = inode_to_index(ino);
                let Some(name) = self.files.get(index) else {
                    reply.error(ENOENT);
                    return;
                };
                if let Some(file) = self.buffers.get(&(req.pid(), name.clone())) {
                    reply.attr(&TTL, &file.attr);
                } else {
                    reply.error(EACCES);
                }
            }
        }
    }

    #[instrument(skip_all)]
    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        self.apply_pending_remaps();
        let index = inode_to_index(ino);
        let Some(file) = self.files.get(index) else {
            reply.error(ENOENT);
            return;
        };

        // Key files by the process and its channel name
        let key = (req.pid(), file.clone());
        let Some(file) = self.buffers.get(&key) else {
            reply.error(EACCES);
            return;
        };

        if flags & O_APPEND == O_APPEND {
            reply.error(EACCES);
            return;
        }

        // Make sure files are opened with valid permissions
        match (file.mode, flags & O_ACCMODE) {
            (ChannelMode::ReadWrite, _)
            | (ChannelMode::ReplayWrites, _)
            | (ChannelMode::FuzzWrites, _)
            | (ChannelMode::ReadOnly, O_RDONLY)
            | (ChannelMode::WriteOnly, O_WRONLY) => {}
            _ => {
                reply.error(EACCES);
                return;
            }
        }

        reply.opened(index as u64, FOPEN_DIRECT_IO);
    }

    #[instrument(skip_all)]
    fn read(
        &mut self,
        req: &Request,
        ino: u64,
        _fh: u64,
        _offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        self.apply_pending_remaps();
        if ino == FUSE_ROOT_ID {
            reply.error(EISDIR);
            return;
        }
        let index = inode_to_index(ino);
        let Some(filename) = self.files.get(index) else {
            reply.error(ENOENT);
            return;
        };

        let key = (req.pid(), filename.clone());
        self.read_message(reply, size as usize, key);
    }

    #[instrument(skip_all)]
    fn write(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        self.apply_pending_remaps();
        if ino == FUSE_ROOT_ID {
            reply.error(EISDIR);
            return;
        }
        let index = inode_to_index(ino);
        let Some(file) = self.files.get(index) else {
            reply.error(ENOENT);
            return;
        };

        let key = (req.pid(), file.clone());
        let Some(file) = self.buffers.get(&key) else {
            reply.error(EACCES);
            return;
        };

        // Drop writes from file, only source of writes will be from the kernel
        if file.mode == ChannelMode::ReplayWrites {
            reply.written(data.len() as u32);
            return;
        }

        let msg = FsMessage::Write(Message {
            id: key,
            data: data.to_vec(),
        });
        if self.fs_side.0.send(msg).is_err() {
            reply.written(0);
            return;
        }
        let Ok(bytes_written) = data.len().try_into() else {
            reply.error(EMSGSIZE);
            return;
        };

        reply.written(bytes_written);
    }

    #[instrument(skip_all)]
    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino != FUSE_ROOT_ID {
            reply.error(ENOENT);
            return;
        }

        // Build full directory listing (static + dynamic entries)
        let mut entries: Vec<(u64, FileType, String)> = vec![
            (FUSE_ROOT_ID, FileType::Directory, ".".to_string()),
            (FUSE_ROOT_ID, FileType::Directory, "..".to_string()),
        ];

        // Dynamically add entries from self.files
        for (i, name) in self.files.iter().enumerate() {
            let inode = index_to_inode(i);
            entries.push((inode, FileType::RegularFile, name.clone()));
        }

        // Serve entries starting from the given offset
        for (i, (inode, file_type, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            let next_offset = (i + 1) as i64;
            if reply.add(inode, next_offset, file_type, name) {
                break;
            }
        }

        reply.ok();
    }
}

fn expand_home(path: &PathBuf) -> PathBuf {
    if let Some(stripped) = path.to_string_lossy().strip_prefix("~/")
        && let Some(home_dir) = home::home_dir()
    {
        return home_dir.join(stripped);
    }
    PathBuf::from(path)
}

fn inode_to_index(inode: u64) -> usize {
    (inode - (FUSE_ROOT_ID + 1)) as usize
}

fn index_to_inode(index: usize) -> u64 {
    index as u64 + (FUSE_ROOT_ID + 1)
}

fn next_inode() -> u64 {
    INODE_GEN.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelMode;
    use crate::file::NexusFile;
    use std::num::NonZeroUsize;
    use std::sync::{Arc, Mutex};

    fn test_file() -> NexusFile {
        NexusFile::new(
            NonZeroUsize::new(1024).unwrap(),
            ChannelMode::ReadWrite,
            next_inode(),
        )
    }

    #[test]
    fn test_apply_pending_remaps_migrates_buffers() {
        let remaps = Arc::new(Mutex::new(Vec::new()));
        let mut fs = NexusFs {
            pending_remaps: remaps.clone(),
            ..Default::default()
        };

        // Insert buffers for PID 100
        fs.buffers.insert((100, "ch_a".into()), test_file());
        fs.buffers.insert((100, "ch_b".into()), test_file());
        // Insert buffer for a different PID that should not move
        fs.buffers.insert((200, "ch_a".into()), test_file());

        // Push a remap: 100 → 300
        remaps.lock().unwrap().push((100, 300));

        fs.apply_pending_remaps();

        // Old keys gone
        assert!(!fs.buffers.contains_key(&(100, "ch_a".into())));
        assert!(!fs.buffers.contains_key(&(100, "ch_b".into())));
        // New keys present
        assert!(fs.buffers.contains_key(&(300, "ch_a".into())));
        assert!(fs.buffers.contains_key(&(300, "ch_b".into())));
        // Unrelated PID untouched
        assert!(fs.buffers.contains_key(&(200, "ch_a".into())));
        // Queue should be drained
        assert!(remaps.lock().unwrap().is_empty());
    }

    #[test]
    fn test_apply_pending_remaps_empty_queue_is_noop() {
        let remaps = Arc::new(Mutex::new(Vec::new()));
        let mut fs = NexusFs {
            pending_remaps: remaps,
            ..Default::default()
        };

        fs.buffers.insert((100, "ch_a".into()), test_file());
        fs.apply_pending_remaps();

        // Nothing should change
        assert!(fs.buffers.contains_key(&(100, "ch_a".into())));
    }

    #[test]
    fn test_apply_pending_remaps_multiple_pairs() {
        let remaps = Arc::new(Mutex::new(Vec::new()));
        let mut fs = NexusFs {
            pending_remaps: remaps.clone(),
            ..Default::default()
        };

        fs.buffers.insert((10, "ch_x".into()), test_file());
        fs.buffers.insert((20, "ch_x".into()), test_file());

        remaps.lock().unwrap().extend([(10, 11), (20, 21)]);
        fs.apply_pending_remaps();

        assert!(fs.buffers.contains_key(&(11, "ch_x".into())));
        assert!(fs.buffers.contains_key(&(21, "ch_x".into())));
        assert!(!fs.buffers.contains_key(&(10, "ch_x".into())));
        assert!(!fs.buffers.contains_key(&(20, "ch_x".into())));
    }

    #[test]
    fn test_apply_pending_remaps_nonexistent_pid_is_harmless() {
        let remaps = Arc::new(Mutex::new(vec![(999, 1000)]));
        let mut fs = NexusFs {
            pending_remaps: remaps,
            ..Default::default()
        };

        fs.buffers.insert((50, "ch_a".into()), test_file());
        fs.apply_pending_remaps();

        // Original buffer still there, no panic
        assert!(fs.buffers.contains_key(&(50, "ch_a".into())));
        assert!(!fs.buffers.contains_key(&(1000, "ch_a".into())));
    }
}
