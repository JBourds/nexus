use crate::errors::{FsError, LinkError};
use crate::{ChannelId, KernelLinks, PID};
use config::ast;

use fuser::ReplyWrite;
use fuser::{
    BackgroundSession, FUSE_ROOT_ID, FileAttr, FileType, Filesystem, MountOption, ReplyAttr,
    ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, Request, consts::FOPEN_DIRECT_IO,
};
use libc::{EACCES, EBADMSG, EISDIR, EMSGSIZE, ENOENT, O_APPEND};
use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
use std::cmp::min;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SendError, Sender};
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
    #[allow(dead_code)]
    logger: Option<Sender<String>>,
    attr: FileAttr,
    files: Vec<ast::LinkHandle>,
    fs_links: HashMap<ChannelId, NexusFile>,
    kernel_links: KernelLinks,
}

/// Necessary handles to identify each link.
#[derive(Debug)]
pub struct NexusLink {
    /// Node's name
    pub node: ast::NodeHandle,
    /// Process ID of the protocol
    pub pid: PID,
    /// Link name (corresponds to file name shown)
    pub link: ast::LinkHandle,
    /// Available link operations
    pub mode: LinkMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinkMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
    PlaybackWrites,
}

#[derive(Debug)]
struct NexusFile {
    mode: LinkMode,
    attr: FileAttr,
    sock: UnixDatagram,
    unread_msg: Option<(usize, Vec<u8>)>,
}

fn expand_home(path: &PathBuf) -> PathBuf {
    if let Some(stripped) = path.as_os_str().to_str().unwrap().strip_prefix("~/") {
        if let Some(home_dir) = home::home_dir() {
            return home_dir.join(stripped);
        }
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
    fn new(sock: UnixDatagram, mode: LinkMode, ino: u64) -> Self {
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

    /// Builder method to pre-allocate the domain socket links.
    pub fn with_links(
        mut self,
        links: impl IntoIterator<Item = NexusLink>,
    ) -> Result<Self, LinkError> {
        for NexusLink {
            pid,
            node,
            link,
            mode,
        } in links
        {
            let (link_side, kernel_side) =
                UnixDatagram::pair().map_err(|_| LinkError::DatagramCreation)?;
            link_side
                .set_nonblocking(true)
                .map_err(|_| LinkError::DatagramCreation)?;
            kernel_side
                .set_nonblocking(true)
                .map_err(|_| LinkError::DatagramCreation)?;
            let key = (pid, link.clone());

            let inode = if let Some(index) = self.files.iter().position(|file| **file == link) {
                index_to_inode(index)
            } else {
                self.files.push(link);
                next_inode()
            };

            if self
                .fs_links
                .insert(key.clone(), NexusFile::new(link_side, mode, inode))
                .is_some()
                || self.kernel_links.insert(key, (node, kernel_side)).is_some()
            {
                return Err(LinkError::DuplicateLink);
            }
        }
        Ok(self)
    }

    /// Mount the filesystem without blocking, yield the background session it
    /// is mounted in, and return the hash map with one side of the underlying
    /// sockets for the kernel to use.
    pub fn mount(mut self) -> Result<(BackgroundSession, KernelLinks), FsError> {
        let options = vec![
            MountOption::FSName("nexus".to_string()),
            MountOption::AutoUnmount,
        ];
        let root = self.root.clone();
        if !root.exists() {
            fs::create_dir_all(&root).map_err(|_| FsError::CreateDirError(root.clone()))?;
        }
        let kernel_links = core::mem::take(&mut self.kernel_links);
        let sess = fuser::spawn_mount2(self, &root, &options)
            .map_err(|_| FsError::MountError(root.clone()))?;
        while !root.exists() {}
        Ok((sess, kernel_links))
    }

    pub fn with_logger(self, logger: Sender<String>) -> Self {
        Self {
            logger: Some(logger),
            ..self
        }
    }

    #[allow(dead_code)]
    fn log(&self, msg: String) -> Result<(), SendError<String>> {
        if let Some(logger) = &self.logger {
            logger.send(msg)
        } else {
            Ok(())
        }
    }
}

impl Default for NexusFs {
    fn default() -> Self {
        let root = expand_home(&PathBuf::from("~/nexus"));
        Self {
            root,
            attr: Self::root_attr(),
            logger: None,
            files: Vec::default(),
            fs_links: HashMap::default(),
            kernel_links: HashMap::default(),
        }
    }
}

impl Filesystem for NexusFs {
    fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent != FUSE_ROOT_ID {
            reply.error(ENOENT);
            return;
        }
        let key = (req.pid(), name.to_str().unwrap().to_string());
        if let Some(file) = self.fs_links.get(&key) {
            reply.entry(&TTL, &file.attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

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
                let Some(file) = self.fs_links.get(&key) else {
                    reply.error(EACCES);
                    return;
                };
                reply.attr(&TTL, &file.attr);
            }
        }
    }

    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let index = inode_to_index(ino);
        let Some(file) = self.files.get(index) else {
            reply.error(ENOENT);
            return;
        };
        let key = (req.pid(), file.clone());
        let Some(file) = self.fs_links.get(&key) else {
            reply.error(EACCES);
            return;
        };

        if flags & O_APPEND == O_APPEND {
            reply.error(EACCES);
            return;
        }
        match (file.mode, flags & O_ACCMODE) {
            (LinkMode::ReadWrite, _)
            | (LinkMode::PlaybackWrites, _)
            | (LinkMode::ReadOnly, O_RDONLY)
            | (LinkMode::WriteOnly, O_WRONLY) => {}
            _ => {
                reply.error(EACCES);
                return;
            }
        }

        reply.opened(index as u64, FOPEN_DIRECT_IO);
    }

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
        let Some(file) = self.fs_links.get_mut(&(req.pid(), file.clone())) else {
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

        // TODO: Do better than big hardcoded vector
        let mut recv_buf = vec![0; 4096];
        let recv_size = match file.sock.recv(&mut recv_buf) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                reply.data(&[]);
                return;
            }
            Err(e) => {
                eprintln!("{e:#?}");
                reply.error(EBADMSG);
                return;
            }
        };

        // Reads should not be forced to be one shot. Anything unread
        // should be buffered in case the reader wants to read incrementally.
        let read_size = min(recv_size, size as usize);
        reply.data(&recv_buf[..read_size as usize]);
        file.unread_msg = Some((read_size, recv_buf));
    }

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
        let Some(file) = self.fs_links.get(&(req.pid(), file.clone())) else {
            reply.error(EACCES);
            return;
        };

        // Drop writes from file, only source of writes will be from the kernel
        if file.mode == LinkMode::PlaybackWrites {
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
        reply.written(bytes_written);
    }

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

impl TryFrom<i32> for LinkMode {
    type Error = LinkError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            O_RDONLY => Ok(Self::ReadOnly),
            O_WRONLY => Ok(Self::WriteOnly),
            O_RDWR => Ok(Self::ReadWrite),
            _ => Err(Self::Error::InvalidMode(value)),
        }
    }
}
