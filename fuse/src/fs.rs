use crate::channel::{ChannelMode, NexusChannel};
use crate::errors::{ChannelError, FsError};
use crate::file::{NexusFile, default_attr};
use crate::{ChannelId, FsChannels, FsMessage, KernelChannels, KernelMessage};
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
use std::time::{Duration, SystemTime};
use std::{collections::HashMap, path::PathBuf};

const TTL: Duration = Duration::from_secs(1);

/// Flat control files that remain at the root level (not in subdirectories).
pub const CONTROL_FILES: [(&str, ChannelMode); 3] = [
    ("ctl.energy_left", ChannelMode::ReadOnly),
    ("ctl.energy_state", ChannelMode::ReadWrite),
    ("ctl.power_flows", ChannelMode::ReadWrite),
];

/// Sub-files under the `ctl.time/` directory.
pub const TIME_SUBFILES: [(&str, ChannelMode); 4] = [
    ("s", ChannelMode::ReadWrite),
    ("ms", ChannelMode::ReadWrite),
    ("us", ChannelMode::ReadWrite),
    ("ns", ChannelMode::ReadWrite),
];

/// Sub-files under the `ctl.elapsed/` directory.
pub const ELAPSED_SUBFILES: [(&str, ChannelMode); 4] = [
    ("s", ChannelMode::ReadOnly),
    ("ms", ChannelMode::ReadOnly),
    ("us", ChannelMode::ReadOnly),
    ("ns", ChannelMode::ReadOnly),
];

/// Sub-files under the `ctl.pos/` directory.
pub const POS_SUBFILES: [(&str, ChannelMode); 10] = [
    ("x", ChannelMode::ReadWrite),
    ("y", ChannelMode::ReadWrite),
    ("z", ChannelMode::ReadWrite),
    ("az", ChannelMode::ReadWrite),
    ("el", ChannelMode::ReadWrite),
    ("roll", ChannelMode::ReadWrite),
    ("dx", ChannelMode::WriteOnly),
    ("dy", ChannelMode::WriteOnly),
    ("dz", ChannelMode::WriteOnly),
    ("motion", ChannelMode::ReadWrite),
];

/// Sub-files under each channel directory (e.g., `lora/`).
pub const CHANNEL_SUBFILES: [(&str, ChannelMode); 3] = [
    ("channel", ChannelMode::ReadWrite), // mode overridden per channel
    ("rssi", ChannelMode::ReadOnly),
    ("snr", ChannelMode::ReadOnly),
];

/// Returns all control file paths for the resolver to inject into channel names.
pub fn control_files() -> Vec<String> {
    let mut files = Vec::new();
    for (name, _) in CONTROL_FILES.iter() {
        files.push(name.to_string());
    }
    for (name, _) in TIME_SUBFILES.iter() {
        files.push(format!("ctl.time/{name}"));
    }
    for (name, _) in ELAPSED_SUBFILES.iter() {
        files.push(format!("ctl.elapsed/{name}"));
    }
    for (name, _) in POS_SUBFILES.iter() {
        files.push(format!("ctl.pos/{name}"));
    }
    files
}

#[derive(Debug, Clone)]
enum FsEntryKind {
    Directory,
    RegularFile,
}

/// A single entry in the virtual filesystem tree.
#[derive(Debug, Clone)]
struct FsEntry {
    /// Display name within its parent directory.
    name: String,
    /// Inode of the parent (FUSE_ROOT_ID for top-level entries).
    parent_inode: u64,
    /// Whether this is a directory or regular file.
    kind: FsEntryKind,
    /// Full path key used for buffer lookups (e.g., "lora/channel", "ctl.time/us").
    path: String,
}

#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    attr: FileAttr,
    entries: Vec<FsEntry>,
    buffers: HashMap<ChannelId, NexusFile>,
    fs_side: FsChannels,
    kernel_side: Option<KernelChannels>,
    /// Receiver for (old_pid, new_pid) pairs sent by the router.
    remap_rx: mpsc::Receiver<(u32, u32)>,
    /// Per-instance inode counter (must not be static; the GUI reuses
    /// the process across simulation runs).
    inode_gen: AtomicU64,
    /// Cache mapping thread IDs to their thread group ID (process ID).
    /// Allows pthreads to access the same FUSE files as the main thread.
    tgid_cache: HashMap<u32, u32>,
}

impl NexusFs {
    pub fn new(root: PathBuf, remap_rx: mpsc::Receiver<(u32, u32)>) -> Self {
        Self {
            root,
            attr: Self::root_attr(),
            remap_rx,
            ..Default::default()
        }
    }

    /// Drain the remap channel and migrate FUSE buffer entries from
    /// old PIDs to new PIDs.
    fn apply_pending_remaps(&mut self) {
        while let Ok((old_pid, new_pid)) = self.remap_rx.try_recv() {
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

    /// Resolve a thread ID to its thread group ID (TGID / process ID).
    /// The TGID is what `Child::id()` returns for the main thread and is
    /// the key used in `self.buffers`. Pthreads have distinct TIDs but
    /// share their parent's TGID, so this lets them access the same files.
    fn resolve_tgid(&mut self, tid: u32) -> u32 {
        if let Some(&tgid) = self.tgid_cache.get(&tid) {
            return tgid;
        }
        let tgid = read_tgid(tid).unwrap_or(tid);
        self.tgid_cache.insert(tid, tgid);
        tgid
    }

    /// Find or create an entry in the tree. Returns the inode.
    fn get_or_make_entry(
        &mut self,
        name: String,
        parent_inode: u64,
        kind: FsEntryKind,
        path: String,
    ) -> u64 {
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.name == name && e.parent_inode == parent_inode)
        {
            index_to_inode(index)
        } else {
            self.entries.push(FsEntry {
                name,
                parent_inode,
                kind,
                path,
            });
            self.inode_gen.fetch_add(1, Ordering::Relaxed)
        }
    }

    /// Create a directory entry at `parent_inode` and populate it with
    /// the given sub-files, creating buffer entries for each PID.
    fn add_directory_with_subfiles(
        &mut self,
        dir_name: &str,
        dir_path: &str,
        parent_inode: u64,
        subfiles: &[(&str, ChannelMode)],
        pids: &[u32],
    ) {
        let dir_inode = self.get_or_make_entry(
            dir_name.to_string(),
            parent_inode,
            FsEntryKind::Directory,
            dir_path.to_string(),
        );
        for &(subfile_name, mode) in subfiles {
            let file_path = format!("{dir_path}/{subfile_name}");
            let file_inode = self.get_or_make_entry(
                subfile_name.to_string(),
                dir_inode,
                FsEntryKind::RegularFile,
                file_path.clone(),
            );
            for &pid in pids {
                self.buffers.insert(
                    (pid, file_path.clone()),
                    NexusFile::new(NonZeroUsize::new(1000).unwrap(), mode, file_inode),
                );
            }
        }
    }

    pub fn add_processes(mut self, pids: &[u32]) -> Self {
        // Flat control files (energy, position, power)
        for (file, mode) in CONTROL_FILES.iter() {
            let inode = self.get_or_make_entry(
                file.to_string(),
                FUSE_ROOT_ID,
                FsEntryKind::RegularFile,
                file.to_string(),
            );
            for &pid in pids {
                self.buffers.insert(
                    (pid, file.to_string()),
                    NexusFile::new(NonZeroUsize::new(1000).unwrap(), *mode, inode),
                );
            }
        }

        // ctl.time/ directory with sub-files
        self.add_directory_with_subfiles("ctl.time", "ctl.time", FUSE_ROOT_ID, &TIME_SUBFILES, pids);

        // ctl.elapsed/ directory with sub-files
        self.add_directory_with_subfiles(
            "ctl.elapsed",
            "ctl.elapsed",
            FUSE_ROOT_ID,
            &ELAPSED_SUBFILES,
            pids,
        );

        // ctl.pos/ directory with sub-files
        self.add_directory_with_subfiles(
            "ctl.pos",
            "ctl.pos",
            FUSE_ROOT_ID,
            &POS_SUBFILES,
            pids,
        );

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
            // Create directory for the channel (e.g., "lora/")
            let dir_inode = self.get_or_make_entry(
                channel.clone(),
                FUSE_ROOT_ID,
                FsEntryKind::Directory,
                channel.clone(),
            );

            // Create "channel" sub-file (data read/write)
            let data_path = format!("{channel}/channel");
            let data_inode = self.get_or_make_entry(
                "channel".to_string(),
                dir_inode,
                FsEntryKind::RegularFile,
                data_path.clone(),
            );
            let data_key = (pid, data_path);
            if self
                .buffers
                .insert(data_key.clone(), NexusFile::new(max_msg_size, mode, data_inode))
                .is_some()
            {
                return Err(ChannelError::DuplicateChannel);
            }

            // Create "rssi" sub-file (read-only)
            let rssi_path = format!("{channel}/rssi");
            let rssi_inode = self.get_or_make_entry(
                "rssi".to_string(),
                dir_inode,
                FsEntryKind::RegularFile,
                rssi_path.clone(),
            );
            self.buffers.insert(
                (pid, rssi_path),
                NexusFile::new(NonZeroUsize::new(64).unwrap(), ChannelMode::ReadOnly, rssi_inode),
            );

            // Create "snr" sub-file (read-only)
            let snr_path = format!("{channel}/snr");
            let snr_inode = self.get_or_make_entry(
                "snr".to_string(),
                dir_inode,
                FsEntryKind::RegularFile,
                snr_path.clone(),
            );
            self.buffers.insert(
                (pid, snr_path),
                NexusFile::new(NonZeroUsize::new(64).unwrap(), ChannelMode::ReadOnly, snr_inode),
            );
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
            entries: Vec::default(),
            buffers: HashMap::default(),
            fs_side: (fs_tx, fs_rx),
            kernel_side: Some((kernel_tx, kernel_rx)),
            remap_rx: mpsc::channel().1,
            inode_gen: AtomicU64::new(FUSE_ROOT_ID + 1),
            tgid_cache: HashMap::new(),
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
        // Just return current attrs -- we ignore truncate/chmod/etc.
        self.getattr(req, ino, _fh, reply);
    }

    #[instrument(skip_all)]
    fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        self.apply_pending_remaps();
        let pid = self.resolve_tgid(req.pid());
        let name_str = name.to_string_lossy();

        // Find entry matching (parent, name)
        let found = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, e)| e.parent_inode == parent && e.name == name_str.as_ref());

        let Some((index, entry)) = found else {
            reply.error(ENOENT);
            return;
        };

        let inode = index_to_inode(index);
        match entry.kind {
            FsEntryKind::Directory => {
                reply.entry(
                    &TTL,
                    &default_attr(inode, FileType::Directory, 0o755, 0, 0),
                    0,
                );
            }
            FsEntryKind::RegularFile => {
                let path = entry.path.clone();
                if let Some(file) = self.buffers.get(&(pid, path)) {
                    reply.entry(&TTL, &file.attr, 0);
                } else {
                    reply.error(ENOENT);
                }
            }
        }
    }

    #[instrument(skip_all)]
    fn getattr(&mut self, req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        self.apply_pending_remaps();
        let pid = self.resolve_tgid(req.pid());
        match ino {
            FUSE_ROOT_ID => reply.attr(&TTL, &self.attr),
            _ => {
                let index = inode_to_index(ino);
                let Some(entry) = self.entries.get(index) else {
                    reply.error(ENOENT);
                    return;
                };
                match entry.kind {
                    FsEntryKind::Directory => {
                        reply.attr(
                            &TTL,
                            &default_attr(ino, FileType::Directory, 0o755, 0, 0),
                        );
                    }
                    FsEntryKind::RegularFile => {
                        if let Some(file) = self.buffers.get(&(pid, entry.path.clone())) {
                            reply.attr(&TTL, &file.attr);
                        } else {
                            reply.error(EACCES);
                        }
                    }
                }
            }
        }
    }

    #[instrument(skip_all)]
    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        self.apply_pending_remaps();
        let pid = self.resolve_tgid(req.pid());
        let index = inode_to_index(ino);
        let Some(entry) = self.entries.get(index) else {
            reply.error(ENOENT);
            return;
        };

        if matches!(entry.kind, FsEntryKind::Directory) {
            reply.error(EISDIR);
            return;
        }

        // Key files by the process and its path
        let key = (pid, entry.path.clone());
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
        let pid = self.resolve_tgid(req.pid());
        if ino == FUSE_ROOT_ID {
            reply.error(EISDIR);
            return;
        }
        let index = inode_to_index(ino);
        let Some(entry) = self.entries.get(index) else {
            reply.error(ENOENT);
            return;
        };

        if matches!(entry.kind, FsEntryKind::Directory) {
            reply.error(EISDIR);
            return;
        }

        let key = (pid, entry.path.clone());
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
        let pid = self.resolve_tgid(req.pid());
        if ino == FUSE_ROOT_ID {
            reply.error(EISDIR);
            return;
        }
        let index = inode_to_index(ino);
        let Some(entry) = self.entries.get(index) else {
            reply.error(ENOENT);
            return;
        };

        if matches!(entry.kind, FsEntryKind::Directory) {
            reply.error(EISDIR);
            return;
        }

        let key = (pid, entry.path.clone());
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
        // Check that the inode is either ROOT or a known directory
        if ino != FUSE_ROOT_ID {
            let index = inode_to_index(ino);
            match self.entries.get(index) {
                Some(entry) if matches!(entry.kind, FsEntryKind::Directory) => {}
                _ => {
                    reply.error(ENOENT);
                    return;
                }
            }
        }

        // Build full directory listing (static + dynamic entries)
        let mut dir_entries: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (
                if ino == FUSE_ROOT_ID {
                    FUSE_ROOT_ID
                } else {
                    // Find parent inode for ".."
                    let index = inode_to_index(ino);
                    self.entries[index].parent_inode
                },
                FileType::Directory,
                "..".to_string(),
            ),
        ];

        // Add children of this directory
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.parent_inode == ino {
                let file_type = match entry.kind {
                    FsEntryKind::Directory => FileType::Directory,
                    FsEntryKind::RegularFile => FileType::RegularFile,
                };
                dir_entries.push((index_to_inode(i), file_type, entry.name.clone()));
            }
        }

        // Serve entries starting from the given offset
        for (i, (inode, file_type, name)) in
            dir_entries.into_iter().enumerate().skip(offset as usize)
        {
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

/// Read the thread group ID for a given thread ID from /proc.
fn read_tgid(tid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{tid}/status")).ok()?;
    for line in status.lines() {
        if let Some(val) = line.strip_prefix("Tgid:") {
            return val.trim().parse().ok();
        }
    }
    None
}

fn inode_to_index(inode: u64) -> usize {
    (inode - (FUSE_ROOT_ID + 1)) as usize
}

fn index_to_inode(index: usize) -> u64 {
    index as u64 + (FUSE_ROOT_ID + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelMode;
    use crate::file::NexusFile;
    use std::num::NonZeroUsize;

    fn test_file(inode: u64) -> NexusFile {
        NexusFile::new(
            NonZeroUsize::new(1024).unwrap(),
            ChannelMode::ReadWrite,
            inode,
        )
    }

    #[test]
    fn test_apply_pending_remaps_migrates_buffers() {
        let (tx, rx) = mpsc::channel();
        let mut fs = NexusFs {
            remap_rx: rx,
            ..Default::default()
        };

        // Insert buffers for PID 100
        fs.buffers.insert((100, "ch_a".into()), test_file(1));
        fs.buffers.insert((100, "ch_b".into()), test_file(1));
        // Insert buffer for a different PID that should not move
        fs.buffers.insert((200, "ch_a".into()), test_file(1));

        // Send a remap: 100 -> 300
        tx.send((100, 300)).unwrap();

        fs.apply_pending_remaps();

        // Old keys gone
        assert!(!fs.buffers.contains_key(&(100, "ch_a".into())));
        assert!(!fs.buffers.contains_key(&(100, "ch_b".into())));
        // New keys present
        assert!(fs.buffers.contains_key(&(300, "ch_a".into())));
        assert!(fs.buffers.contains_key(&(300, "ch_b".into())));
        // Unrelated PID untouched
        assert!(fs.buffers.contains_key(&(200, "ch_a".into())));
        // Channel should be drained
        assert!(rx_is_empty(&fs.remap_rx));
    }

    #[test]
    fn test_apply_pending_remaps_empty_queue_is_noop() {
        let (_tx, rx) = mpsc::channel();
        let mut fs = NexusFs {
            remap_rx: rx,
            ..Default::default()
        };

        fs.buffers.insert((100, "ch_a".into()), test_file(1));
        fs.apply_pending_remaps();

        // Nothing should change
        assert!(fs.buffers.contains_key(&(100, "ch_a".into())));
    }

    #[test]
    fn test_apply_pending_remaps_multiple_pairs() {
        let (tx, rx) = mpsc::channel();
        let mut fs = NexusFs {
            remap_rx: rx,
            ..Default::default()
        };

        fs.buffers.insert((10, "ch_x".into()), test_file(1));
        fs.buffers.insert((20, "ch_x".into()), test_file(1));

        tx.send((10, 11)).unwrap();
        tx.send((20, 21)).unwrap();
        fs.apply_pending_remaps();

        assert!(fs.buffers.contains_key(&(11, "ch_x".into())));
        assert!(fs.buffers.contains_key(&(21, "ch_x".into())));
        assert!(!fs.buffers.contains_key(&(10, "ch_x".into())));
        assert!(!fs.buffers.contains_key(&(20, "ch_x".into())));
    }

    #[test]
    fn test_apply_pending_remaps_nonexistent_pid_is_harmless() {
        let (tx, rx) = mpsc::channel();
        tx.send((999, 1000)).unwrap();
        let mut fs = NexusFs {
            remap_rx: rx,
            ..Default::default()
        };

        fs.buffers.insert((50, "ch_a".into()), test_file(1));
        fs.apply_pending_remaps();

        // Original buffer still there, no panic
        assert!(fs.buffers.contains_key(&(50, "ch_a".into())));
        assert!(!fs.buffers.contains_key(&(1000, "ch_a".into())));
    }

    #[test]
    fn test_add_processes_creates_directories_and_files() {
        let fs = NexusFs::default().add_processes(&[100]);

        // Should have directory entries for ctl.time and ctl.elapsed
        assert!(fs
            .entries
            .iter()
            .any(|e| e.name == "ctl.time" && matches!(e.kind, FsEntryKind::Directory)));
        assert!(fs
            .entries
            .iter()
            .any(|e| e.name == "ctl.elapsed" && matches!(e.kind, FsEntryKind::Directory)));

        // Should have sub-file entries
        assert!(fs.buffers.contains_key(&(100, "ctl.time/us".into())));
        assert!(fs.buffers.contains_key(&(100, "ctl.time/ns".into())));
        assert!(fs.buffers.contains_key(&(100, "ctl.elapsed/s".into())));
        assert!(fs.buffers.contains_key(&(100, "ctl.elapsed/ns".into())));
        assert!(fs.buffers.contains_key(&(100, "ctl.pos/x".into())));
        assert!(fs.buffers.contains_key(&(100, "ctl.pos/motion".into())));

        // Should have ctl.pos directory
        assert!(fs
            .entries
            .iter()
            .any(|e| e.name == "ctl.pos" && matches!(e.kind, FsEntryKind::Directory)));

        // Flat control files should still exist
        assert!(fs.buffers.contains_key(&(100, "ctl.energy_left".into())));
    }

    #[test]
    fn test_add_channels_creates_directory_with_subfiles() {
        let fs = NexusFs::default().add_processes(&[100]);
        let channels = vec![NexusChannel {
            pid: 100,
            node: "node1".to_string(),
            channel: "lora".to_string(),
            mode: ChannelMode::ReadWrite,
            max_msg_size: NonZeroUsize::new(256).unwrap(),
        }];
        let fs = fs.add_channels(channels).unwrap();

        // Should have a "lora" directory
        assert!(fs
            .entries
            .iter()
            .any(|e| e.name == "lora" && matches!(e.kind, FsEntryKind::Directory)));

        // Should have lora/channel, lora/rssi, lora/snr buffer entries
        assert!(fs.buffers.contains_key(&(100, "lora/channel".into())));
        assert!(fs.buffers.contains_key(&(100, "lora/rssi".into())));
        assert!(fs.buffers.contains_key(&(100, "lora/snr".into())));
    }

    fn rx_is_empty(rx: &mpsc::Receiver<(u32, u32)>) -> bool {
        rx.try_recv().is_err()
    }
}
