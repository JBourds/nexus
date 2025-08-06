mod errors;
use errors::*;
use fuser::{
    BackgroundSession, FUSE_ROOT_ID, FileAttr, FileType, Filesystem, MountOption, ReplyAttr,
    ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use libc::ENOENT;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::net::UnixDatagram;
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

/// Nexus FUSE FS which intercepts the requests from processes to links
/// (implemented as virtual files). Reads/writes to the link files are mapped
/// to unix datagram domain sockets managed by the simulation kernel.
#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    attr: FileAttr,
    files: HashSet<ast::LinkHandle>,
    fs_links: HashMap<LinkId, UnixDatagram>,
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
            let (link_side, kernel_side) =
                UnixDatagram::pair().map_err(|_| LinkError::DatagramCreation)?;
            if self.fs_links.insert(key.clone(), link_side).is_some() {
                return Err(LinkError::DuplicateLink);
            }
            if self.kernel_links.insert(key, kernel_side).is_some() {
                return Err(LinkError::DuplicateLink);
            }
        }
        Ok(self)
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

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
}

impl Default for NexusFs {
    fn default() -> Self {
        let root = expand_home(&PathBuf::from("~/nexus"));
        Self {
            root,
            attr: Self::ROOT_ATTR,
            files: HashSet::default(),
            fs_links: HashMap::default(),
            kernel_links: HashMap::default(),
        }
    }
}

impl Filesystem for NexusFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == FUSE_ROOT_ID && self.files.contains(name.to_str().unwrap()) {
            reply.entry(&TTL, &self.attr, 0);
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
