use crate::channel::{ChannelMode, NexusChannel};
use crate::errors::{ChannelError, FsError};
use crate::file::NexusFile;
use crate::{ChannelId, FsChannels, FsMessage, KernelChannels, KernelMessage};
use config::ast::{self, ProtocolHandle};
use fuser::ReplyWrite;
use std::num::NonZeroUsize;
use std::process::Child;
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

static INODE_GEN: AtomicU64 = AtomicU64::new(FUSE_ROOT_ID + 1);
const TTL: Duration = Duration::from_secs(1);

pub const CONTROL_FILES: [(&str, ChannelMode); 4] = [
    ("time", ChannelMode::ReadOnly),
    ("energy_state", ChannelMode::WriteOnly),
    ("energy_left", ChannelMode::ReadOnly),
    ("position", ChannelMode::ReadWrite),
];

#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    attr: FileAttr,
    files: Vec<ast::ChannelHandle>,
    buffers: HashMap<ChannelId, NexusFile>,
    fs_side: FsChannels,
    kernel_side: Option<KernelChannels>,
}

impl NexusFs {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            attr: Self::root_attr(),
            ..Default::default()
        }
    }

    fn root_attr() -> FileAttr {
        let now = SystemTime::now();
        FileAttr {
            ino: FUSE_ROOT_ID,
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
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

    pub fn add_processes(mut self, handles: &[runner::ProtocolHandle]) -> Self {
        for (file, mode) in CONTROL_FILES.iter() {
            let inode = self.get_or_make_inode(file.to_string());
            for pid in handles
                .iter()
                .filter_map(|h| h.process.as_ref().map(Child::id))
            {
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

    fn get_time(fs_side: &mut FsChannels, id: ChannelId) -> Result<KernelMessage, FsError> {
        fs_side
            .0
            .send(FsMessage::Time(Message {
                id,
                data: Vec::new(),
            }))
            .map_err(|e| FsError::KernelShutdown(Box::new(e)))?;
        fs_side
            .1
            .recv()
            .map_err(|e| FsError::KernelShutdown(Box::new(e)))
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
            MountOption::AutoUnmount,
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
        while !root.exists() {}
        Ok((sess, kernel_side))
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
        }
    }
}

impl Filesystem for NexusFs {
    #[instrument(skip_all)]
    fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent != FUSE_ROOT_ID {
            reply.error(ENOENT);
            return;
        }
        let file = name.to_str().unwrap().to_string();
        if let Some(file) = self.buffers.get(&(req.pid(), file)) {
            reply.entry(&TTL, &file.attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    #[instrument(skip_all)]
    fn getattr(&mut self, req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
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
        // check for control files
        match filename.as_str() {
            "time" => {
                if let Ok(msg) = Self::get_time(&mut self.fs_side, key.clone()) {
                    reply.data(msg.data());
                    return;
                } else {
                    reply.data(&[]);
                    return;
                };
            }
            _ => {}
        }

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
            let read_size = min(remaining, size as usize);
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
            let read_size = min(msg.len(), size as usize);
            reply.data(&msg[..read_size as usize]);
            if allow_incremental_reads && read_size < msg.len() {
                file.unread_msg = Some((read_size, msg));
            }
        } else {
            reply.data(&[]);
        }
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
                eprintln!("break!");
                break;
            }
        }

        reply.ok();
    }
}

fn expand_home(path: &PathBuf) -> PathBuf {
    if let Some(stripped) = path.as_os_str().to_str().unwrap().strip_prefix("~/")
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
