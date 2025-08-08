pub mod errors;
mod helpers;
mod types;
use fuse::errors::SocketError;
use fuse::socket;
use helpers::{make_handles, unzip};

use std::{
    collections::{HashMap, HashSet},
    io,
    os::unix::net::UnixDatagram,
    rc::Rc,
};

use config::ast::{self, Params};
use runner::RunMode;
use types::*;

use crate::errors::{ConversionError, KernelError};

pub type LinkId = (fuse::PID, LinkHandle);

#[derive(Debug)]
#[allow(unused)]
pub struct Kernel {
    params: Params,
    links: Vec<Link>,
    nodes: Vec<Node>,
    files: HashMap<LinkId, UnixDatagram>,
    link_names: Vec<String>,
    node_names: Vec<String>,
}

impl Kernel {
    pub fn new(
        sim: ast::Simulation,
        files: HashMap<fuse::LinkId, UnixDatagram>,
    ) -> Result<Self, KernelError> {
        let (node_names, nodes) = unzip(sim.nodes);
        let node_handles = make_handles(node_names.clone());
        let (link_names, links) = unzip(sim.links);
        let link_handles = make_handles(link_names.clone());
        let links = links
            .into_iter()
            .map(|link| Link::from_ast(link, &link_handles))
            .collect::<Result<_, ConversionError>>()
            .map_err(KernelError::KernelInit)?;
        let nodes = nodes
            .into_iter()
            .map(|node| Node::from_ast(node, &link_handles, &node_handles))
            .collect::<Result<_, ConversionError>>()
            .map_err(KernelError::KernelInit)?;
        let files = files
            .into_iter()
            .map(|((pid, link_name), file)| {
                link_handles
                    .get(&link_name)
                    .ok_or(KernelError::KernelInit(
                        ConversionError::LinkHandleConversion(link_name),
                    ))
                    .map(|handle| ((pid, *handle), file))
            })
            .collect::<Result<_, KernelError>>()?;
        Ok(Self {
            params: sim.params,
            links,
            nodes,
            files,
            link_names,
            node_names,
        })
    }

    pub fn recv(
        socket: &mut UnixDatagram,
        data: &mut [u8],
        pid: fuse::PID,
        link_name: impl AsRef<str>,
    ) -> Result<usize, KernelError> {
        socket::recv(socket, data, pid, link_name).map_err(KernelError::FileError)
    }

    pub fn send(
        socket: &mut UnixDatagram,
        data: &[u8],
        pid: fuse::PID,
        link_name: impl AsRef<str>,
    ) -> Result<usize, KernelError> {
        socket::send(socket, data, pid, link_name).map_err(KernelError::FileError)
    }

    pub fn send_msg(
        socket: &mut UnixDatagram,
        data: &[u8],
        pid: fuse::PID,
        link_name: &impl AsRef<str>,
    ) -> Result<usize, KernelError> {
        let msg_len = data.len().to_ne_bytes();
        Self::send(socket, &msg_len, pid, link_name)?;
        Self::send(socket, data, pid, link_name)
    }

    pub fn recv_msg(
        socket: &mut UnixDatagram,
        pid: fuse::PID,
        link_name: &impl AsRef<str>,
    ) -> Result<Vec<u8>, KernelError> {
        let mut msg_len = [0u8; core::mem::size_of::<usize>()];
        Self::recv(socket, &mut msg_len, pid, link_name)?;
        let required_capacity = usize::from_ne_bytes(msg_len);
        let mut recv_buf = vec![0; required_capacity];
        Self::recv(socket, recv_buf.as_mut_slice(), pid, link_name)?;
        Ok(recv_buf)
    }

    pub fn run(mut self, _mode: RunMode) -> Result<(), KernelError> {
        let mut send_queue = HashMap::new();
        let pids = self
            .files
            .keys()
            .map(|(pid, _)| *pid)
            .collect::<HashSet<fuse::PID>>();

        loop {
            for ((pid, handle), socket) in self.files.iter_mut() {
                let (pid, handle) = (*pid, *handle);
                let link_name = &self.link_names[handle];
                println!("{pid}.{link_name}");

                match Self::recv_msg(socket, pid, link_name) {
                    Ok(recv_buf) => {
                        // Deliver message to all other entries
                        let msg = Rc::new(recv_buf);
                        println!("{pid}.{link_name} [TX]: {}", String::from_utf8_lossy(&msg));
                        for pid in pids.iter().filter(|their_pid| **their_pid != pid) {
                            send_queue
                                .entry((*pid, handle))
                                .or_insert(Vec::new())
                                .push(Rc::clone(&msg));
                        }
                    }
                    Err(KernelError::FileError(SocketError::NothingToRead)) => {}
                    Err(KernelError::FileError(SocketError::SocketReadError { ioerr, .. }))
                        if ioerr.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => {
                        return Err(e);
                    }
                }

                // Handle inbound connections
                for msg in send_queue
                    .remove_entry(&(pid, handle))
                    .map(|(_, val)| val)
                    .unwrap_or_default()
                {
                    println!("{pid}.{link_name} [RX]: {}", String::from_utf8_lossy(&msg));
                    Self::send_msg(socket, &msg, pid, link_name)?;
                }

                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }

        #[allow(unreachable_code)]
        Ok(())
    }
}
