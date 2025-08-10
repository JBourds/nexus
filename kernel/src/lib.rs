pub mod errors;
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};
use tracing::Level;
mod helpers;
pub mod log;
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
use runner::RunCmd;
use tracing::{debug, event, instrument};
use types::*;

use crate::errors::{ConversionError, KernelError};

pub type LinkId = (fuse::PID, LinkHandle);
extern crate tracing;

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
    pub fn new(sim: ast::Simulation, files: fuse::KernelLinks) -> Result<Self, KernelError> {
        let (node_names, nodes) = unzip(sim.nodes);
        let node_handles = make_handles(node_names.clone());
        let (mut link_names, links) = unzip(sim.links);
        let link_handles = make_handles(link_names.clone());
        let links = links
            .into_iter()
            .map(|link| Link::from_ast(link, &link_handles))
            .collect::<Result<_, ConversionError>>()
            .map_err(KernelError::KernelInit)?;

        // Internal links have a higher priority namespace than global links.
        // These still need to be converted into integer handles. Internal links
        // are unique within a node, so create a mapping of (node, link): handle
        // which gets checked first when resolving string handles below.
        let mut new_nodes = vec![];
        let mut internal_node_handles = HashMap::new();
        for (handle, (node_name, node)) in node_names
            .clone()
            .into_iter()
            .zip(nodes.into_iter())
            .enumerate()
        {
            let (new_node, internal_names) =
                Node::from_ast(node, handle, &link_handles, &node_handles)
                    .map_err(KernelError::KernelInit)?;
            new_nodes.push(new_node);
            link_names.extend(internal_names.clone());
            for (handle, internal_name) in (link_names.len() - 1..).zip(internal_names.into_iter())
            {
                internal_node_handles.insert((node_name.clone(), internal_name), handle);
            }
        }

        let lookup_link =
            |pid: fuse::PID, link_name: String, node: ast::NodeHandle, file: UnixDatagram| {
                internal_node_handles
                    .get(&(node, link_name.clone()))
                    .or(link_handles.get(&link_name))
                    .ok_or(KernelError::KernelInit(
                        ConversionError::LinkHandleConversion(link_name),
                    ))
                    .map(|handle| ((pid, *handle), file))
            };
        let files = files
            .into_iter()
            .map(|((pid, link_name), (node, file))| lookup_link(pid, link_name, node, file))
            .collect::<Result<_, KernelError>>()?;

        Ok(Self {
            params: sim.params,
            links,
            nodes: new_nodes,
            files,
            link_names,
            node_names,
        })
    }

    pub fn recv<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        data: &mut [u8],
        pid: fuse::PID,
        link_name: &A,
    ) -> Result<usize, KernelError> {
        socket::recv(socket, data, pid, link_name).map_err(KernelError::FileError)
    }

    pub fn send<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        data: &[u8],
        pid: fuse::PID,
        link_name: A,
    ) -> Result<usize, KernelError> {
        socket::send(socket, data, pid, link_name).map_err(KernelError::FileError)
    }

    #[instrument(skip(socket, data), err)]
    pub fn send_msg<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        data: &[u8],
        pid: fuse::PID,
        timestep: u64,
        link: LinkHandle,
        link_name: &A,
    ) -> Result<usize, KernelError> {
        let len = data.len();
        debug!("Sending {len} byte message");
        let msg_len = len.to_ne_bytes();
        Self::send(socket, &msg_len, pid, link_name)?;

        match Self::send(socket, data, pid, link_name) {
            Ok(n_sent) => {
                event!(target: "tx", Level::INFO, timestep, link, pid, tx = true, data);
                Ok(n_sent)
            }
            err => err,
        }
    }

    #[instrument(skip(socket), err)]
    pub fn recv_msg<A: AsRef<str> + std::fmt::Debug>(
        socket: &mut UnixDatagram,
        pid: fuse::PID,
        timestep: u64,
        link: LinkHandle,
        link_name: &A,
    ) -> Result<Vec<u8>, KernelError> {
        let mut msg_len = [0u8; core::mem::size_of::<usize>()];
        Self::recv(socket, &mut msg_len, pid, link_name)?;
        let required_capacity = usize::from_ne_bytes(msg_len);
        debug!("Receiving {required_capacity} byte message");
        let mut recv_buf = vec![0; required_capacity];
        let data = recv_buf.as_mut_slice();
        Self::recv(socket, data, pid, link_name)?;
        event!(target: "rx", Level::INFO, timestep, link, pid, tx = false, data);
        Ok(recv_buf)
    }

    fn timestep(&self) -> Duration {
        let length = self.params.timestep.length;
        match self.params.timestep.unit {
            ast::TimeUnit::Seconds => Duration::from_secs(length),
            ast::TimeUnit::Milliseconds => Duration::from_millis(length),
            ast::TimeUnit::Microseconds => Duration::from_micros(length),
            ast::TimeUnit::Nanoseconds => Duration::from_nanos(length),
        }
    }

    #[instrument(skip_all)]
    #[allow(unused_variables)]
    pub fn run(mut self, cmd: RunCmd, logs: Option<PathBuf>) -> Result<(), KernelError> {
        let mut send_queue = HashMap::new();
        let pids = self
            .files
            .keys()
            .map(|(pid, _)| *pid)
            .collect::<HashSet<fuse::PID>>();
        let mut timestep = 1;
        let delta = self.timestep();

        loop {
            let start = SystemTime::now();
            for ((pid, handle), socket) in self.files.iter_mut() {
                let (pid, handle) = (*pid, *handle);
                let link_name = &self.link_names[handle];
                match Self::recv_msg(socket, pid, timestep, handle, link_name) {
                    Ok(recv_buf) => {
                        // Deliver message to all other entries
                        let msg = Rc::new(recv_buf);
                        for pid in pids.iter().filter(|their_pid| **their_pid != pid) {
                            debug!("{pid}.{link_name} [TX]: Sending message to {pid}");
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
                    debug!("{pid}.{link_name} [RX]: {}", String::from_utf8_lossy(&msg));
                    Self::send_msg(socket, &msg, pid, timestep, handle, link_name)?;
                }
            }
            if let Ok(elapsed) = start.elapsed() {
                if elapsed < delta {
                    std::thread::sleep(delta - elapsed);
                }
            }
            timestep += 1;
        }

        #[allow(unreachable_code)]
        Ok(())
    }
}
