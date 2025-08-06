mod errors;
use errors::*;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::{collections::HashMap, path::PathBuf};

use config::ast;

pub type PID = u32;

/// Nexus FUSE FS which intercepts the requests from processes to links
/// (implemented as virtual files). Reads/writes to the link files are mapped
/// to unix datagram domain sockets managed by the simulation kernel.
#[derive(Debug)]
pub struct NexusFs {
    root: PathBuf,
    files: Vec<SocketAddr>,
    links: HashMap<(PID, ast::LinkHandle), NexusLink>,
}

impl NexusFs {
    /// Create FS at root
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            ..Default::default()
        }
    }

    /// Builder method to add files to the nexus file system.
    pub fn with_files(mut self, files: impl IntoIterator<Item = SocketAddr>) -> Self {
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
}

impl Default for NexusFs {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/nexus"),
            files: Vec::default(),
            links: HashMap::default(),
        }
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
