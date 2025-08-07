mod errors;
use errors::*;
use fuser::{
    BackgroundSession, FUSE_ROOT_ID, FileAttr, FileType, Filesystem, MountOption, ReplyAttr,
    ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, Request, consts::FOPEN_DIRECT_IO,
};
use libc::{EACCES, EAGAIN, EBADMSG, EBUSY, EISDIR, ENOENT};
use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
use std::cmp::min;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{SendError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, path::PathBuf};

use config::ast;

fn expand_home(path: &PathBuf) -> PathBuf {
    if let Some(stripped) = path.as_os_str().to_str().unwrap().strip_prefix("~/") {
        if let Some(home_dir) = home::home_dir() {
            return home_dir.join(stripped);
        }
    }
    PathBuf::from(path)
}

pub type PID = u32;
pub type LinkId = (PID, ast::LinkHandle);
pub type Mode = i32;
static INODE_GEN: AtomicU64 = AtomicU64::new(FUSE_ROOT_ID + 1);

#[derive(Debug)]
pub struct NexusFile {
    mode: Mode,
    attr: FileAttr,
    sock: UnixDatagram,
}

impl NexusFile {
    fn new(sock: UnixDatagram, mode: Mode) -> Self {
        let now = SystemTime::now();
        let ino = INODE_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                uid: 501,
                gid: 20,
                rdev: 0,
                flags: 0,
                blksize: 512,
            },
            sock,
        }
    }
}

/// Nexus FUSE FS which intercepts the requests from processes to links
/// (implemented as virtual files). Reads/writes to the link files are mapped
/// to unix datagram domain sockets managed by the simulation kernel.
#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    logger: Option<Sender<String>>,
    attr: FileAttr,
    files: Vec<ast::LinkHandle>,
    fs_links: HashMap<LinkId, NexusFile>,
    kernel_links: HashMap<LinkId, UnixDatagram>,
}

const TTL: Duration = Duration::from_secs(1);

impl NexusFs {
    /// Default FS root attribute
    const ROOT_ATTR: FileAttr = FileAttr {
        ino: FUSE_ROOT_ID,
        size: 0,
        blocks: 0,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 501,
        gid: 20,
        rdev: 0,
        flags: 0,
        blksize: 512,
    };

    /// Create FS at root
    pub fn new(root: PathBuf) -> Self {
        let now = SystemTime::now();
        Self {
            root,
            attr: FileAttr {
                atime: now,
                mtime: now,
                ctime: now,
                crtime: now,
                ..Self::ROOT_ATTR
            },
            ..Default::default()
        }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// Builder method to add files to the nexus file system.
    pub fn with_files(mut self, files: impl IntoIterator<Item = ast::LinkHandle>) -> Self {
        self.files.extend(files);
        self
    }

    /// Builder method to pre-allocate the domain socket links.
    pub fn with_links(
        mut self,
        links: impl IntoIterator<Item = (PID, ast::LinkHandle, Mode)>,
    ) -> Result<Self, LinkError> {
        for (pid, handle, mode) in links {
            let (link_side, kernel_side) =
                UnixDatagram::pair().map_err(|_| LinkError::DatagramCreation)?;
            link_side
                .set_nonblocking(true)
                .map_err(|_| LinkError::DatagramCreation)?;
            kernel_side
                .set_nonblocking(true)
                .map_err(|_| LinkError::DatagramCreation)?;
            let key = (pid, handle);
            if self
                .fs_links
                .insert(key.clone(), NexusFile::new(link_side, mode))
                .is_some()
                || self.kernel_links.insert(key, kernel_side).is_some()
            {
                return Err(LinkError::DuplicateLink);
            }
        }
        Ok(self)
    }

    /// Mount the filesystem without blocking, yield the background session it
    /// is mounted in, and return the hash map with one side of the underlying
    /// sockets for the kernel to use.
    pub fn mount(mut self) -> Result<(BackgroundSession, HashMap<LinkId, UnixDatagram>), FsError> {
        let options = vec![
            MountOption::FSName("nexus".to_string()),
            MountOption::AutoUnmount,
        ];
        let root = self.root.clone();
        if !root.exists() {
            fs::create_dir_all(&root).map_err(|_| FsError::CreateDirError(root.clone()))?;
        }
        let kernel_links = core::mem::take(&mut self.kernel_links);
        let sess =
            fuser::spawn_mount2(self, &root, &options).map_err(|_| FsError::MountError(root))?;
        Ok((sess, kernel_links))
    }

    pub fn with_logger(self, logger: Sender<String>) -> Self {
        Self {
            logger: Some(logger),
            ..self
        }
    }

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
            attr: Self::ROOT_ATTR,
            logger: None,
            files: Vec::default(),
            fs_links: HashMap::default(),
            kernel_links: HashMap::default(),
        }
    }
}

fn inode_to_index(inode: u64) -> usize {
    (inode - (FUSE_ROOT_ID + 1)) as usize
}

impl Filesystem for NexusFs {
    fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let _ = self.log("Lookup!".to_string());
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
        let _ = self.log("getattr!".to_string());
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
        let _ = self.log(format!("Open: Inode: {ino}, PID: {}", req.pid()));

        let idx = inode_to_index(ino);
        let Some(file) = self.files.get(idx) else {
            reply.error(ENOENT);
            return;
        };
        let key = (req.pid(), file.clone());
        if !self.fs_links.contains_key(&key) {
            reply.error(EACCES);
            return;
        };

        // TODO: Permission checking based on declared link status
        let access_mode = flags & O_ACCMODE;

        reply.opened(idx as u64, FOPEN_DIRECT_IO);
    }

    fn read(
        &mut self,
        req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        let _ = self.log(format!(
            "Read\n\tSize: {size}\n\tOffset: {offset}, PID: {}",
            req.pid()
        ));
        if ino == FUSE_ROOT_ID {
            reply.error(EISDIR);
            return;
        }
        let file_index = inode_to_index(ino);
        let Some(file) = self.files.get(file_index) else {
            reply.error(ENOENT);
            return;
        };
        let Some(file) = self.fs_links.get(&(req.pid(), file.clone())) else {
            let _ = self.log("EACCES!".to_string());
            reply.error(EACCES);
            return;
        };

        let mut recv_buf = vec![0u8; 1024];
        let recv_size = match file.sock.recv(&mut recv_buf) {
            Ok(n) => {
                let _ = self.log(format!("Received message {n} bytes long"));
                n
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Nothing in the socket
                reply.data(&[]);
                return;
            }
            Err(e) => {
                let _ = self.log(e.to_string());
                reply.error(EBADMSG);
                return;
            }
        };

        // Could underflow if file length is less than local_start
        let read_size = min(size, recv_size as u32);
        println!("Read size: {read_size}");
        let buf = &recv_buf[..read_size as usize];

        let _ = self.log(format!("Received data: {}", String::from_utf8_lossy(buf)));
        reply.data(buf);
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
            let inode = (i + 2) as u64;
            entries.push((inode, FileType::RegularFile, name.clone()));
        }

        // Serve entries starting from the given offset
        for (i, (inode, file_type, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            let next_offset = (i + 1) as i64;
            if reply.add(inode, next_offset, file_type, name) {
                println!("Break: {i}");
                break;
            }
        }

        reply.ok();
    }
}
