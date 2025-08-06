mod errors;
use errors::*;
use fuser::{
    FUSE_ROOT_ID, FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData,
    ReplyDirectory, ReplyEntry, Request,
};
use libc::ENOENT;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::net::{SocketAddr, UnixDatagram};
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

/// Nexus FUSE FS which intercepts the requests from processes to links
/// (implemented as virtual files). Reads/writes to the link files are mapped
/// to unix datagram domain sockets managed by the simulation kernel.
#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    attr: FileAttr,
    files: HashSet<ast::LinkHandle>,
    links: HashMap<(PID, ast::LinkHandle), NexusLink>,
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

    /// Builder method to add files to the nexus file system.
    pub fn with_files(mut self, files: impl IntoIterator<Item = ast::LinkHandle>) -> Self {
        self.files.extend(files);
        self
    }

    /// Builder method to pre-allocate the domain socket links.
    pub fn with_links(
        mut self,
        links: impl IntoIterator<Item = (PID, ast::LinkHandle)>,
    ) -> Result<Self, LinkError> {
        for key in links {
            let link = NexusLink::new()?;
            if self.links.insert(key, link).is_some() {
                return Err(LinkError::DuplicateLink);
            }
        }
        Ok(self)
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn mount(self) -> Result<(), FsError> {
        let options = vec![
            MountOption::FSName("nexus".to_string()),
            MountOption::AutoUnmount,
        ];
        let root = self.root.clone();
        if !root.exists() {
            fs::create_dir_all(&root).map_err(|_| FsError::CreateDirError(root.clone()))?;
        }
        fuser::mount2(self, &root, &options).map_err(|_| FsError::MountError(root))
    }
}

impl Default for NexusFs {
    fn default() -> Self {
        let root = expand_home(&PathBuf::from("~/nexus"));
        Self {
            root,
            attr: Self::ROOT_ATTR,
            files: HashSet::default(),
            links: HashMap::default(),
        }
    }
}

impl Filesystem for NexusFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == FUSE_ROOT_ID && self.files.contains(name.to_str().unwrap()) {
            println!("File \"{name:?}\" is in the directory!");
            reply.error(ENOENT);
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        match ino {
            FUSE_ROOT_ID => reply.attr(&TTL, &self.attr),
            _ => reply.error(ENOENT),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        _size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        if ino != FUSE_ROOT_ID {
            println!("This is where here will someday be content!");
            reply.error(ENOENT);
        } else {
            println!("Listing the root!");
            reply.error(ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino != 1 {
            reply.error(ENOENT);
            return;
        }

        let mut entries = vec![
            (FUSE_ROOT_ID, FileType::Directory, "."),
            (FUSE_ROOT_ID, FileType::Directory, ".."),
        ];
        for entry in self
            .files
            .iter()
            .enumerate()
            .map(|(inode, s)| ((inode + 2) as u64, FileType::RegularFile, s.as_str()))
        {
            entries.push(entry);
        }

        for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            // i + 1 means the index of the next entry
            if reply.add(entry.0, (i + 1) as i64, entry.1, entry.2) {
                break;
            }
        }
        reply.ok();
    }
}

/// Datagram pipe which links the sending/receiving ends together.
#[derive(Debug)]
struct DatagramPipe {
    tx: UnixDatagram,
    rx: UnixDatagram,
}

impl DatagramPipe {
    fn new(tx: UnixDatagram, rx: UnixDatagram) -> Self {
        Self { tx, rx }
    }
}

/// The underlying representation of a single link with a tx/rx datagram socket
/// managed by the simulation kernel.
///
/// rx: Messages sent by the simulation kernel to be received by client.
/// tx: Messages sent by the client to be managed by simulation kernel.
#[derive(Debug)]
struct NexusLink {
    tx: DatagramPipe,
    rx: DatagramPipe,
}

impl NexusLink {
    fn new() -> Result<Self, LinkError> {
        let tx = UnixDatagram::pair()
            .map_err(|_| LinkError::DatagramCreation)
            .map(|(tx, rx)| DatagramPipe::new(tx, rx))?;
        let rx = UnixDatagram::pair()
            .map_err(|_| LinkError::DatagramCreation)
            .map(|(tx, rx)| DatagramPipe::new(tx, rx))?;
        Ok(Self { tx, rx })
    }
}
