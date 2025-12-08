use crate::errors::{ChannelError, FsError};
use crate::{ChannelId, KernelChannelHandle, KernelChannels, KernelControlFile, PID};
use config::ast;
use fuser::ReplyWrite;
use tracing::instrument;

use fuser::{
    BackgroundSession, FUSE_ROOT_ID, FileAttr, FileType, Filesystem, MountOption, PollHandle,
    ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, ReplyPoll, Request,
    consts::FOPEN_DIRECT_IO,
};
use libc::{EACCES, EBADMSG, EISDIR, EMSGSIZE, ENOENT, ESHUTDOWN, O_APPEND};
use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
use std::cmp::min;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::num::NonZeroU64;
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};
use std::{collections::HashMap, path::PathBuf};

static INODE_GEN: AtomicU64 = AtomicU64::new(FUSE_ROOT_ID + 1);
const TTL: Duration = Duration::from_secs(1);

/// Nexus FUSE FS which intercepts the requests from processes to links
/// (implemented as virtual files). Reads/writes to the link files are mapped
/// to unix datagram domain sockets managed by the simulation kernel.
#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    attr: FileAttr,
    files: Vec<ast::ChannelHandle>,
    fs_channels: HashMap<ChannelId, NexusFile>,
    kernel_links: KernelChannels,
}

/// Necessary handles to identify each channel.
#[derive(Debug)]
pub struct NexusChannel {
    /// Node's name
    pub node: ast::NodeHandle,
    /// Process ID of the protocol
    pub pid: PID,
    /// Channel name (corresponds to file name shown)
    pub channel: ast::ChannelHandle,
    /// Available link operations
    pub mode: ChannelMode,
    pub max_msg_size: NonZeroU64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChannelMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
    ReplayWrites,
    FuzzWrites,
}

#[derive(Debug)]
struct ControlFile<T> {
    request: Sender<()>,
    ack: Receiver<T>,
}

impl<T> ControlFile<T> {
    fn new(request: Sender<()>, ack: Receiver<T>) -> Self {
        Self { request, ack }
    }
}

#[derive(Debug)]
struct NexusFile {
    mode: ChannelMode,
    attr: FileAttr,
    sock: UnixDatagram,
    max_msg_size: NonZeroU64,
    unread_msg: Option<(usize, Vec<u8>)>,
    read: ControlFile<ReadSignal>,
    write: ControlFile<WriteSignal>,
}

/// Way for the sender to attach information for the FS to use regarding how
/// a file's buffer should be handled (ex. Whether it gets saved and reread in
/// the case of a partial read).
#[derive(Clone, Copy, Debug)]
pub enum ReadSignal {
    Nothing,
    Shared,
    Exclusive,
}

/// Carry information from the simulation kernel to the FS regarding a
/// write operation.
#[derive(Clone, Copy, Debug)]
pub enum WriteSignal {
    Done,
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

impl NexusFile {
    fn new(
        sock: UnixDatagram,
        max_msg_size: NonZeroU64,
        read: ControlFile<ReadSignal>,
        write: ControlFile<WriteSignal>,
        mode: ChannelMode,
        ino: u64,
    ) -> Self {
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
            read,
            write,
            max_msg_size,
            sock,
            unread_msg: None,
        }
    }
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

    /// Builder method to pre-allocate the domain sockets.
    pub fn with_channels(
        mut self,
        channels: impl IntoIterator<Item = NexusChannel>,
    ) -> Result<Self, ChannelError> {
        for NexusChannel {
            pid,
            node,
            channel,
            mode,
            max_msg_size,
        } in channels
        {
            let (fs_side, kernel_side) =
                UnixDatagram::pair().map_err(|_| ChannelError::DatagramCreation)?;
            fs_side
                .set_nonblocking(true)
                .map_err(|_| ChannelError::DatagramCreation)?;
            kernel_side
                .set_nonblocking(true)
                .map_err(|_| ChannelError::DatagramCreation)?;
            let key = (pid, channel.clone());

            let inode = if let Some(index) = self.files.iter().position(|file| **file == channel) {
                index_to_inode(index)
            } else {
                self.files.push(channel);
                next_inode()
            };

            let (fs_read_request, kernel_read_request) = mpsc::channel();
            let (kernel_read_response, fs_read_response) = mpsc::channel();

            let (fs_write_request, kernel_write_request) = mpsc::channel();
            let (kernel_write_response, fs_write_response) = mpsc::channel();

            let fs_read = ControlFile::new(fs_read_request, fs_read_response);
            let fs_write = ControlFile::new(fs_write_request, fs_write_response);
            let kernel_read = KernelControlFile::new(kernel_read_request, kernel_read_response);
            let kernel_write = KernelControlFile::new(kernel_write_request, kernel_write_response);

            if self
                .fs_channels
                .insert(
                    key.clone(),
                    NexusFile::new(fs_side, max_msg_size, fs_read, fs_write, mode, inode),
                )
                .is_some()
                || self
                    .kernel_links
                    .insert(
                        key,
                        KernelChannelHandle {
                            node,
                            read: kernel_read,
                            write: kernel_write,
                            file: kernel_side,
                        },
                    )
                    .is_some()
            {
                return Err(ChannelError::DuplicateChannel);
            }
        }
        Ok(self)
    }

    /// Mount the filesystem without blocking, yield the background session it
    /// is mounted in, and return the hash map with one side of the underlying
    /// sockets for the kernel to use.
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
        let kernel_links = core::mem::take(&mut self.kernel_links);
        let sess =
            fuser::spawn_mount2(self, &root, &options).map_err(|err| FsError::MountError {
                root: root.clone(),
                err,
            })?;
        while !root.exists() {}
        Ok((sess, kernel_links))
    }
}
impl Default for NexusFs {
    fn default() -> Self {
        let root = expand_home(&PathBuf::from("~/nexus"));
        Self {
            root,
            attr: Self::root_attr(),
            files: Vec::default(),
            fs_channels: HashMap::default(),
            kernel_links: HashMap::default(),
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
        let key = (req.pid(), name.to_str().unwrap().to_string());
        if let Some(file) = self.fs_channels.get(&key) {
            reply.entry(&TTL, &file.attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    #[instrument(skip_all)]
    fn poll(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _ph: PollHandle,
        _events: u32,
        _flags: u32,
        reply: ReplyPoll,
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
        let Some(file) = self.fs_channels.get_mut(&(req.pid(), file.clone())) else {
            reply.error(EACCES);
            return;
        };

        // Check if there is already data to read
        if file.unread_msg.is_some() {
            reply.poll(libc::POLLIN.try_into().unwrap());
            return;
        }

        // Main thread could shutdown in the middle of a request
        if file.read.request.send(()).is_err() {
            reply.error(ESHUTDOWN);
            return;
        }
        let mut recv_buf = vec![0; file.max_msg_size.get() as usize];
        let recv_size = match file.sock.recv(&mut recv_buf) {
            Ok(n) => n,
            Err(_) => {
                reply.poll(0);
                return;
            }
        };
        file.unread_msg = Some((recv_size, recv_buf));
        reply.poll(libc::POLLIN.try_into().unwrap());
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
                let key = (req.pid(), name.clone());
                let Some(file) = self.fs_channels.get(&key) else {
                    reply.error(EACCES);
                    return;
                };
                reply.attr(&TTL, &file.attr);
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
        let key = (req.pid(), file.clone());
        let Some(file) = self.fs_channels.get(&key) else {
            reply.error(EACCES);
            return;
        };

        if flags & O_APPEND == O_APPEND {
            reply.error(EACCES);
            return;
        }
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
        let Some(file) = self.files.get(index) else {
            reply.error(ENOENT);
            return;
        };
        let Some(file) = self.fs_channels.get_mut(&(req.pid(), file.clone())) else {
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

        // Main thread could shutdown in the middle of a request
        if file.read.request.send(()).is_err() {
            reply.error(ESHUTDOWN);
            return;
        }
        let allow_incremental_reads = match file.read.ack.recv() {
            Ok(ReadSignal::Shared) => false,
            Ok(ReadSignal::Exclusive) => true,
            Ok(ReadSignal::Nothing) => {
                reply.data(&[]);
                return;
            }
            // Kernel has shutdown, exit gracefully.
            Err(_) => {
                reply.data(&[]);
                return;
            }
        };
        let mut recv_buf = vec![0; file.max_msg_size.get() as usize];
        let recv_size = match file.sock.recv(&mut recv_buf) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                reply.data(&[]);
                return;
            }
            Err(_) => {
                reply.error(EBADMSG);
                return;
            }
        };

        let read_size = min(recv_size, size as usize);
        recv_buf.truncate(recv_size);
        reply.data(&recv_buf[..read_size as usize]);
        if allow_incremental_reads {
            file.unread_msg = Some((read_size, recv_buf));
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
        let Some(file) = self.fs_channels.get(&(req.pid(), file.clone())) else {
            reply.error(EACCES);
            return;
        };

        // Drop writes from file, only source of writes will be from the kernel
        if file.mode == ChannelMode::ReplayWrites {
            reply.written(data.len() as u32);
            return;
        }

        let write_msg = |buf: &[u8]| -> bool {
            match file.sock.send(buf) {
                Ok(n) if n == buf.len() => true,
                Ok(_) => false,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => true,
                Err(_) => false,
            }
        };

        // It's okay if we fail to write even if it's a half write
        // since on reads we don't
        if !write_msg(data) {
            reply.error(EBADMSG);
            return;
        };
        let Ok(bytes_written) = data.len().try_into() else {
            reply.error(EMSGSIZE);
            return;
        };

        // Kernel has shutdown, exit gracefully.
        if file.write.request.send(()).is_err() {
            reply.written(0);
            return;
        }
        let _ = file.write.ack.recv();

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

impl TryFrom<i32> for ChannelMode {
    type Error = ChannelError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            O_RDONLY => Ok(Self::ReadOnly),
            O_WRONLY => Ok(Self::WriteOnly),
            O_RDWR => Ok(Self::ReadWrite),
            _ => Err(Self::Error::InvalidMode(value)),
        }
    }
}
