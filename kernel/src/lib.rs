pub mod errors;
mod helpers;
pub mod log;
mod router;
mod types;

use fuse::errors::SocketError;
use mio::unix::SourceFd;

use helpers::{make_handles, unzip};
use mio::{Events, Interest, Poll, Token};
use rand::{SeedableRng, rngs::StdRng};
use std::os::fd::AsRawFd;
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};
use tracing::{Level, info};

use std::{
    collections::{HashMap, HashSet},
    io,
    os::unix::net::UnixDatagram,
    rc::Rc,
};

use config::ast::{self, TimestepConfig};
use runner::RunCmd;
use tracing::{debug, event, instrument};
use types::*;

use crate::errors::{ConversionError, KernelError};
use crate::router::Router;

pub type LinkId = (fuse::PID, LinkHandle);
extern crate tracing;

#[derive(Debug)]
#[allow(unused)]
pub struct Kernel {
    root: PathBuf,
    rng: StdRng,
    timestep: TimestepConfig,
    links: Vec<Link>,
    nodes: Vec<Node>,
    handles: Vec<LinkId>,
    sockets: Vec<UnixDatagram>,
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
            .collect::<Result<HashMap<LinkId, UnixDatagram>, KernelError>>()?;
        let (handles, sockets) = unzip(files);

        Ok(Self {
            root: sim.params.root,
            rng: StdRng::seed_from_u64(sim.params.seed),
            timestep: sim.params.timestep,
            links,
            nodes: new_nodes,
            handles,
            sockets,
            link_names,
            node_names,
        })
    }

    #[instrument(skip_all)]
    #[allow(unused_variables)]
    pub fn run(self, cmd: RunCmd, logs: Option<PathBuf>) -> Result<(), KernelError> {
        let delta = self.time_delta();
        let pids: Vec<_> = self.handles.iter().map(|(pid, handle)| *pid).collect();
        let mut poll = Poll::new().map_err(|_| KernelError::PollCreation)?;
        let mut events = Events::with_capacity(self.sockets.len());
        for (index, sock) in self.sockets.iter().enumerate() {
            poll.registry()
                .register(
                    &mut SourceFd(&sock.as_raw_fd()),
                    Token(index),
                    Interest::READABLE,
                )
                .map_err(|_| KernelError::PollRegistration)?;
        }

        let mut router = Router::new(
            self.nodes,
            self.node_names,
            self.links,
            self.link_names,
            self.handles,
            self.sockets,
        );
        for timestep in 0..self.timestep.count.into() {
            let start = SystemTime::now();
            poll.poll(&mut events, Some(delta))
                .map_err(|_| KernelError::PollError)?;
            for event in &events {
                let Token(index) = event.token();
                router.inbound(index).map_err(KernelError::RouterError)?;
            }
            router.outbound().map_err(KernelError::RouterError)?;
            router.step().map_err(KernelError::RouterError)?;
            if let Ok(elapsed) = start.elapsed() {
                if elapsed < delta {
                    std::thread::sleep(delta - elapsed);
                }
            }
        }

        Ok(())
    }

    fn time_delta(&self) -> Duration {
        let length = self.timestep.length;
        match self.timestep.unit {
            ast::TimeUnit::Seconds => Duration::from_secs(length),
            ast::TimeUnit::Milliseconds => Duration::from_millis(length),
            ast::TimeUnit::Microseconds => Duration::from_micros(length),
            ast::TimeUnit::Nanoseconds => Duration::from_nanos(length),
        }
    }
}
