use crate::channel::{ChannelMode, NexusChannel};
use crate::errors::{ChannelError, FsError};
use crate::file::{NexusFile, default_attr};
use crate::{ChannelId, FsMessage, KernelMessage, SleepEvent};
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

use crate::ctrl_files::*;

const TTL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub(crate) enum FsEntryKind {
    Directory,
    RegularFile,
    ControlFile(ControlFile),
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

/// FUSE filesystem. Generic over the message type pushed on FUSE events so
/// the kernel can hand in a `Sender<RouterInput>` (FUSE events arrive at the
/// router in one mpsc hop, no forwarder thread). Defaults to `FsMessage` so
/// tests and the in-tree default constructor work without the kernel.
#[derive(Debug)]
pub struct NexusFs<T = FsMessage>
where
    T: From<FsMessage> + Send + 'static,
{
    root: PathBuf,
    attr: FileAttr,
    entries: Vec<FsEntry>,
    /// (parent_inode -> (child_name -> entry_index)) lookup index. Replaces
    /// the previous O(N) linear scan of `entries`. With thousands of nodes
    /// and channels, the scan dominated every FUSE syscall.
    by_parent: HashMap<u64, HashMap<String, usize>>,
    /// Per-process file buffers keyed by `(PID, entry_index)`. 
    buffers: HashMap<(u32, usize), NexusFile>,
    /// `(sender into kernel/router, receiver of replies from kernel)`. The
    /// sender is provided by the caller; for the kernel-driven flow it is a
    /// clone of the router's input channel so FUSE events go directly to the
    /// router rather than incurring another hop at the kernel.
    fs_side: (
        crossbeam_channel::Sender<T>,
        mpsc::Receiver<KernelMessage>,
    ),
    /// Reply channel handed back to the kernel on `mount()`. Held in an
    /// Option so `mount` can take ownership without consuming the rest of
    /// `Self`.
    kernel_reply_tx: Option<mpsc::Sender<KernelMessage>>,
    /// Receiver for (old_pid, new_pid) pairs sent by the router.
    remap_rx: mpsc::Receiver<(u32, u32)>,
    /// Per-instance inode counter (must not be static; the GUI reuses
    /// the process across simulation runs).
    inode_gen: AtomicU64,
    /// Cache mapping thread IDs to their thread group ID (process ID).
    /// Allows pthreads to access the same FUSE files as the main thread.
    tgid_cache: HashMap<u32, u32>,
}

impl<T> NexusFs<T>
where
    T: From<FsMessage> + Send + 'static,
{
    /// Construct a NexusFs that pushes FUSE events through `fs_to_kernel_tx`.
    /// The sender is typically a clone of the router's input channel, so the
    /// FUSE thread reaches the router in one mpsc hop. `root` defaults to
    /// `~/nexus` when `None`.
    pub fn new(
        root: Option<PathBuf>,
        remap_rx: mpsc::Receiver<(u32, u32)>,
        fs_to_kernel_tx: crossbeam_channel::Sender<T>,
    ) -> Self {
        let root = root.unwrap_or_else(|| expand_home(&PathBuf::from("~/nexus")));
        let (kernel_reply_tx, fs_reply_rx) = mpsc::channel::<KernelMessage>();
        Self {
            root,
            attr: Self::root_attr(),
            entries: Vec::default(),
            by_parent: HashMap::default(),
            buffers: HashMap::default(),
            fs_side: (fs_to_kernel_tx, fs_reply_rx),
            kernel_reply_tx: Some(kernel_reply_tx),
            remap_rx,
            inode_gen: AtomicU64::new(FUSE_ROOT_ID + 1),
            tgid_cache: HashMap::new(),
        }
    }

    /// Drain the remap channel and migrate FUSE buffer entries from
    /// old PIDs to new PIDs.
    fn apply_pending_remaps(&mut self) {
        while let Ok((old_pid, new_pid)) = self.remap_rx.try_recv() {
            let keys_to_migrate: Vec<usize> = self
                .buffers
                .keys()
                .filter(|(pid, _)| *pid == old_pid)
                .map(|(_, idx)| *idx)
                .collect();
            for idx in keys_to_migrate {
                if let Some(file) = self.buffers.remove(&(old_pid, idx)) {
                    self.buffers.insert((new_pid, idx), file);
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

    /// Find or create an entry in the tree. Returns `(inode, entry_index)`.
    /// Maintains the (parent_inode, name) -> entry_index index so that
    /// `lookup` does not need to scan `entries` linearly.
    fn get_or_make_entry(
        &mut self,
        name: String,
        parent_inode: u64,
        kind: FsEntryKind,
        path: String,
    ) -> (u64, usize) {
        if let Some(&index) = self.by_parent.get(&parent_inode).and_then(|m| m.get(&name)) {
            (index_to_inode(index), index)
        } else {
            let index = self.entries.len();
            self.by_parent
                .entry(parent_inode)
                .or_default()
                .insert(name.clone(), index);
            self.entries.push(FsEntry {
                name,
                parent_inode,
                kind,
                path,
            });
            let inode = self.inode_gen.fetch_add(1, Ordering::Relaxed);
            (inode, index)
        }
    }

    /// Create a directory entry at `parent_inode` and populate it with
    /// the given sub-files, creating buffer entries for each PID.
    fn add_directory_with_subfiles(
        &mut self,
        dir_name: &str,
        parent_inode: u64,
        subfiles: &[(&str, ChannelMode, FsEntryKind)],
        pids: &[u32],
    ) {
        let (dir_inode, _) = self.get_or_make_entry(
            dir_name.to_string(),
            parent_inode,
            FsEntryKind::Directory,
            dir_name.to_string(),
        );
        for &(subfile_name, mode, kind) in subfiles {
            let file_path = format!("{dir_name}/{subfile_name}");
            let (file_inode, file_idx) =
                self.get_or_make_entry(subfile_name.to_string(), dir_inode, kind, file_path);
            for &pid in pids {
                self.buffers.insert(
                    (pid, file_idx),
                    NexusFile::new(NonZeroUsize::new(1000).unwrap(), mode, file_inode),
                );
            }
        }
    }

    pub fn add_processes(mut self, pids: &[u32]) -> Self {
        // Flat control files (energy, position, power)
        for (file, mode, kind) in CONTROL_FILES.iter() {
            let (inode, idx) =
                self.get_or_make_entry(file.to_string(), FUSE_ROOT_ID, *kind, file.to_string());
            for &pid in pids {
                self.buffers.insert(
                    (pid, idx),
                    NexusFile::new(NonZeroUsize::new(1000).unwrap(), *mode, inode),
                );
            }
        }

        // ctl.time/ directory with sub-files
        self.add_directory_with_subfiles("ctl.time", FUSE_ROOT_ID, &TIME_SUBFILES, pids);

        // ctl.elapsed/ directory with sub-files
        self.add_directory_with_subfiles("ctl.elapsed", FUSE_ROOT_ID, &ELAPSED_SUBFILES, pids);

        // ctl.sleep.relative/ directory with sub-files
        self.add_directory_with_subfiles(
            "ctl.sleep.relative",
            FUSE_ROOT_ID,
            &SLEEP_RELATIVE_SUBFILES,
            pids,
        );

        // ctl.sleep.absolute/ directory with sub-files
        self.add_directory_with_subfiles(
            "ctl.sleep.absolute",
            FUSE_ROOT_ID,
            &SLEEP_ABSOLUTE_SUBFILES,
            pids,
        );

        // ctl.pos/ directory with sub-files
        self.add_directory_with_subfiles("ctl.pos", FUSE_ROOT_ID, &POS_SUBFILES, pids);

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
            let (dir_inode, _) = self.get_or_make_entry(
                channel.clone(),
                FUSE_ROOT_ID,
                FsEntryKind::Directory,
                channel.clone(),
            );

            // Create "channel" sub-file (data read/write)
            let data_path = format!("{channel}/channel");
            let (data_inode, data_idx) = self.get_or_make_entry(
                "channel".to_string(),
                dir_inode,
                FsEntryKind::RegularFile,
                data_path,
            );
            if self
                .buffers
                .insert(
                    (pid, data_idx),
                    NexusFile::new(max_msg_size, mode, data_inode),
                )
                .is_some()
            {
                return Err(ChannelError::DuplicateChannel);
            }

            // Both these files are read-only
            for name in ["rssi", "snr"] {
                let subfile_path = format!("{channel}/{name}");
                let (subfile_inode, subfile_idx) = self.get_or_make_entry(
                    name.to_string(),
                    dir_inode,
                    FsEntryKind::RegularFile,
                    subfile_path,
                );
                self.buffers.insert(
                    (pid, subfile_idx),
                    NexusFile::new(
                        NonZeroUsize::new(64).unwrap(),
                        ChannelMode::ReadOnly,
                        subfile_inode,
                    ),
                );
            }
        }
        Ok(self)
    }

    /// Request a message from the kernel for the channel identified by `id`.
    /// Performs blocking I/O on the channel to send a request and receive the
    /// response from the kernel.
    fn pull_message(
        fs_side: &mut (
            crossbeam_channel::Sender<T>,
            mpsc::Receiver<KernelMessage>,
        ),
        id: ChannelId,
    ) -> Result<KernelMessage, FsError> {
        fs_side
            .0
            .send(
                FsMessage::Read(Message {
                    id,
                    data: Vec::new(),
                })
                .into(),
            )
            .map_err(|e| FsError::KernelShutdown(Box::new(e)))?;
        fs_side
            .1
            .recv()
            .map_err(|e| FsError::KernelShutdown(Box::new(e)))
    }

    /// Mount the filesystem without blocking, yield the background session it
    /// is mounted in, and return the kernel's reply sender.
    pub fn mount(mut self) -> Result<(BackgroundSession, mpsc::Sender<KernelMessage>), FsError> {
        let mut options = vec![MountOption::FSName("nexus".to_string())];
        if std::env::var_os("NEXUS_FUSE_ALLOW_OTHER").is_some() {
            options.push(MountOption::AllowOther);
        }
        let root = self.root.clone();
        if !root.exists() {
            fs::create_dir_all(&root).map_err(|err| FsError::CreateDirError {
                dir: root.clone(),
                err,
            })?;
        }
        let kernel_reply_tx = self
            .kernel_reply_tx
            .take()
            .expect("kernel reply sender already consumed");
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

        Ok((sess, kernel_reply_tx))
    }

    fn read_message(
        &mut self,
        reply: ReplyData,
        size: usize,
        buf_key: (u32, usize),
        msg_id: ChannelId,
    ) {
        // Get the file containing all message buffers
        let Some(file) = self.buffers.get_mut(&buf_key) else {
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
        if let Ok(msg) = Self::pull_message(&mut self.fs_side, msg_id) {
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

impl Default for NexusFs<FsMessage> {
    fn default() -> Self {
        let root = expand_home(&PathBuf::from("~/nexus"));
        let (fs_tx, _kernel_rx) = crossbeam_channel::unbounded::<FsMessage>();
        let (kernel_reply_tx, fs_reply_rx) = mpsc::channel::<KernelMessage>();
        Self {
            root,
            attr: Self::root_attr(),
            entries: Vec::default(),
            by_parent: HashMap::default(),
            buffers: HashMap::default(),
            fs_side: (fs_tx, fs_reply_rx),
            kernel_reply_tx: Some(kernel_reply_tx),
            remap_rx: mpsc::channel().1,
            inode_gen: AtomicU64::new(FUSE_ROOT_ID + 1),
            tgid_cache: HashMap::new(),
        }
    }
}

impl<T> Filesystem for NexusFs<T>
where
    T: From<FsMessage> + Send + 'static,
{
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

        // O(1) index lookup keyed by (parent_inode, name).
        let index_opt = self
            .by_parent
            .get(&parent)
            .and_then(|m| m.get(name_str.as_ref()))
            .copied();
        let Some(index) = index_opt else {
            reply.error(ENOENT);
            return;
        };
        let entry = &self.entries[index];

        let inode = index_to_inode(index);
        match entry.kind {
            FsEntryKind::Directory => {
                reply.entry(
                    &TTL,
                    &default_attr(inode, FileType::Directory, 0o755, 0, 0),
                    0,
                );
            }
            FsEntryKind::RegularFile | FsEntryKind::ControlFile(_) => {
                if let Some(file) = self.buffers.get(&(pid, index)) {
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
                        reply.attr(&TTL, &default_attr(ino, FileType::Directory, 0o755, 0, 0));
                    }
                    FsEntryKind::RegularFile | FsEntryKind::ControlFile(_) => {
                        if let Some(file) = self.buffers.get(&(pid, index)) {
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

        // Buffer key is (pid, entry_index); no String allocation.
        let Some(file) = self.buffers.get(&(pid, index)) else {
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

        // Path is only cloned for the kernel-bound message id; the buffer
        // lookup uses the entry index directly.
        let msg_id = (pid, entry.path.clone());
        self.read_message(reply, size as usize, (pid, index), msg_id);
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

        let Some(file) = self.buffers.get(&(pid, index)) else {
            reply.error(EACCES);
            return;
        };

        match entry.kind {
            FsEntryKind::Directory => {
                reply.error(EISDIR);
            }
            FsEntryKind::ControlFile(ControlFile::SleepRelative(unit))
            | FsEntryKind::ControlFile(ControlFile::SleepAbsolute(unit)) => {
                let is_relative = matches!(
                    entry.kind,
                    FsEntryKind::ControlFile(ControlFile::SleepRelative(_))
                );
                let Ok(bytes_consumed) = data.len().try_into() else {
                    reply.error(EMSGSIZE);
                    return;
                };
                let parsed = String::from_utf8_lossy(data).trim().parse::<u64>();
                match parsed {
                    Ok(val) => {
                        let msg = FsMessage::Sleep(SleepEvent {
                            pid,
                            val,
                            unit,
                            is_relative,
                            bytes_consumed,
                            reply,
                        });
                        self.fs_side
                            .0
                            .send(msg.into())
                            .expect("failed to send message to kernel");
                    }
                    Err(_) => {
                        // Acknowledge the bytes so the caller's write loop
                        // doesn't spin forever on a malformed payload. The
                        // sleep just doesn't happen.
                        reply.written(bytes_consumed);
                    }
                }
            }
            _ => {
                // Drop writes from file, only source of writes will be from the kernel
                if file.mode == ChannelMode::ReplayWrites {
                    reply.written(data.len() as u32);
                    return;
                }

                let msg = FsMessage::Write(Message {
                    id: (pid, entry.path.clone()),
                    data: data.to_vec(),
                });
                self.fs_side
                    .0
                    .send(msg.into())
                    .expect("failed to send message to kernel");
                let Ok(bytes_written) = data.len().try_into() else {
                    reply.error(EMSGSIZE);
                    return;
                };

                reply.written(bytes_written);
            }
        }
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

        // Add children of this directory using the (parent -> name -> index)
        // index, avoiding a full scan over every entry in the simulation.
        if let Some(children) = self.by_parent.get(&ino) {
            for (_, &i) in children.iter() {
                let entry = &self.entries[i];
                let file_type = match entry.kind {
                    FsEntryKind::Directory => FileType::Directory,
                    FsEntryKind::RegularFile | FsEntryKind::ControlFile(_) => FileType::RegularFile,
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

    // Buffer keys are (pid, entry_index) — pick small synthetic indices.
    const ENT_A: usize = 1;
    const ENT_B: usize = 2;

    #[test]
    fn test_apply_pending_remaps_migrates_buffers() {
        let (tx, rx) = mpsc::channel();
        let mut fs = NexusFs {
            remap_rx: rx,
            ..Default::default()
        };

        // Insert buffers for PID 100
        fs.buffers.insert((100, ENT_A), test_file(1));
        fs.buffers.insert((100, ENT_B), test_file(1));
        // Insert buffer for a different PID that should not move
        fs.buffers.insert((200, ENT_A), test_file(1));

        // Send a remap: 100 -> 300
        tx.send((100, 300)).unwrap();

        fs.apply_pending_remaps();

        // Old keys gone
        assert!(!fs.buffers.contains_key(&(100, ENT_A)));
        assert!(!fs.buffers.contains_key(&(100, ENT_B)));
        // New keys present
        assert!(fs.buffers.contains_key(&(300, ENT_A)));
        assert!(fs.buffers.contains_key(&(300, ENT_B)));
        // Unrelated PID untouched
        assert!(fs.buffers.contains_key(&(200, ENT_A)));
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

        fs.buffers.insert((100, ENT_A), test_file(1));
        fs.apply_pending_remaps();

        // Nothing should change
        assert!(fs.buffers.contains_key(&(100, ENT_A)));
    }

    #[test]
    fn test_apply_pending_remaps_multiple_pairs() {
        let (tx, rx) = mpsc::channel();
        let mut fs = NexusFs {
            remap_rx: rx,
            ..Default::default()
        };

        fs.buffers.insert((10, ENT_A), test_file(1));
        fs.buffers.insert((20, ENT_A), test_file(1));

        tx.send((10, 11)).unwrap();
        tx.send((20, 21)).unwrap();
        fs.apply_pending_remaps();

        assert!(fs.buffers.contains_key(&(11, ENT_A)));
        assert!(fs.buffers.contains_key(&(21, ENT_A)));
        assert!(!fs.buffers.contains_key(&(10, ENT_A)));
        assert!(!fs.buffers.contains_key(&(20, ENT_A)));
    }

    #[test]
    fn test_apply_pending_remaps_nonexistent_pid_is_harmless() {
        let (tx, rx) = mpsc::channel();
        tx.send((999, 1000)).unwrap();
        let mut fs = NexusFs {
            remap_rx: rx,
            ..Default::default()
        };

        fs.buffers.insert((50, ENT_A), test_file(1));
        fs.apply_pending_remaps();

        // Original buffer still there, no panic
        assert!(fs.buffers.contains_key(&(50, ENT_A)));
        assert!(!fs.buffers.contains_key(&(1000, ENT_A)));
    }

    /// Helper for tests: locate an entry's index by full path (slow scan,
    /// only used in #[cfg(test)] code).
    fn buffer_for(fs: &NexusFs, pid: u32, path: &str) -> bool {
        let Some(idx) = fs.entries.iter().position(|e| e.path == path) else {
            return false;
        };
        fs.buffers.contains_key(&(pid, idx))
    }

    #[test]
    fn test_add_processes_creates_directories_and_files() {
        let fs = NexusFs::default().add_processes(&[100]);

        // Should have directory entries for ctl.time and ctl.elapsed
        assert!(
            fs.entries
                .iter()
                .any(|e| e.name == "ctl.time" && matches!(e.kind, FsEntryKind::Directory))
        );
        assert!(
            fs.entries
                .iter()
                .any(|e| e.name == "ctl.elapsed" && matches!(e.kind, FsEntryKind::Directory))
        );

        // Should have sub-file entries
        assert!(buffer_for(&fs, 100, "ctl.time/us"));
        assert!(buffer_for(&fs, 100, "ctl.time/ns"));
        assert!(buffer_for(&fs, 100, "ctl.elapsed/s"));
        assert!(buffer_for(&fs, 100, "ctl.elapsed/ns"));
        assert!(buffer_for(&fs, 100, "ctl.pos/x"));
        assert!(buffer_for(&fs, 100, "ctl.pos/motion"));

        // Should have ctl.pos directory
        assert!(
            fs.entries
                .iter()
                .any(|e| e.name == "ctl.pos" && matches!(e.kind, FsEntryKind::Directory))
        );

        // Flat control files should still exist
        assert!(buffer_for(&fs, 100, "ctl.energy_left"));
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
        assert!(
            fs.entries
                .iter()
                .any(|e| e.name == "lora" && matches!(e.kind, FsEntryKind::Directory))
        );

        // Should have lora/channel, lora/rssi, lora/snr buffer entries
        assert!(buffer_for(&fs, 100, "lora/channel"));
        assert!(buffer_for(&fs, 100, "lora/rssi"));
        assert!(buffer_for(&fs, 100, "lora/snr"));
    }

    fn rx_is_empty(rx: &mpsc::Receiver<(u32, u32)>) -> bool {
        rx.try_recv().is_err()
    }
}
