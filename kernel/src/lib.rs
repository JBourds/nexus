pub mod errors;
mod helpers;
pub mod log;
mod router;
mod types;

use mio::unix::SourceFd;

use helpers::{make_handles, unzip};
use mio::{Events, Interest, Poll, Token};
use rand::{SeedableRng, rngs::StdRng};
use std::os::fd::AsRawFd;
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use std::{collections::HashMap, os::unix::net::UnixDatagram};

use config::ast::{self, TimestepConfig};
use runner::{RunCmd, RunHandle};
use tracing::{error, instrument, warn};
use types::*;

use crate::errors::{ConversionError, KernelError};
use crate::router::Router;

/// Unique identifier for a channel belonging to a node protocol
/// - `fuse::PID`: Process identifier (executing node protocol)
/// - `NodeHandle`: Node the process belongs to.
/// - `ChannelHandle`: Channel the connection is over.
pub type ChannelId = (fuse::PID, NodeHandle, ChannelHandle);
extern crate tracing;

#[allow(unused)]
pub struct Kernel {
    root: PathBuf,
    rng: StdRng,
    timestep: TimestepConfig,
    channels: Vec<Channel>,
    nodes: Vec<Node>,
    handles: Vec<ChannelId>,
    sockets: Vec<UnixDatagram>,
    channel_names: Vec<String>,
    node_names: Vec<String>,
    run_handles: Vec<RunHandle>,
}

impl Kernel {
    pub fn new(
        sim: ast::Simulation,
        files: fuse::KernelChannels,
        run_handles: Vec<RunHandle>,
    ) -> Result<Self, KernelError> {
        let (node_names, nodes) =
            unzip(sim.nodes.into_iter().flat_map(|(handle, nodes)| {
                nodes.into_iter().map(move |node| (handle.clone(), node))
            }));
        let node_handles = make_handles(node_names.clone());
        let (mut channel_names, channels) = unzip(sim.channels);
        let channel_handles = make_handles(channel_names.clone());

        // Internal channels have a higher priority namespace than global channels.
        // These still need to be converted into integer handles. Internal channels
        // are unique within a node, so create a mapping of (node, channel): handle
        // which gets checked first when resolving string handles below.
        let mut new_nodes = vec![];
        let mut internal_node_channel_handles = HashMap::new();
        for (handle, (node_name, node)) in node_names
            .clone()
            .into_iter()
            .zip(nodes.into_iter())
            .enumerate()
        {
            let (new_node, internal_names) =
                Node::from_ast(node, handle, &channel_handles, &node_handles)
                    .map_err(KernelError::KernelInit)?;
            new_nodes.push(new_node);
            channel_names.extend(internal_names.clone());
            for (handle, internal_name) in
                (channel_names.len() - 1..).zip(internal_names.into_iter())
            {
                internal_node_channel_handles.insert((node_name.clone(), internal_name), handle);
            }
        }

        let lookup_channel =
            |pid: fuse::PID, channel_name: String, node: ast::NodeHandle, file: UnixDatagram| {
                let node_handle = *node_handles.get(&node).unwrap();
                internal_node_channel_handles
                    .get(&(node, channel_name.clone()))
                    .or(channel_handles.get(&channel_name))
                    .ok_or(KernelError::KernelInit(
                        ConversionError::ChannelHandleConversion(channel_name),
                    ))
                    .map(|channel_handle| ((pid, node_handle, *channel_handle), file))
            };
        let files = files
            .into_iter()
            .map(|((pid, channel_name), (node, file))| {
                lookup_channel(pid, channel_name, node, file)
            })
            .collect::<Result<HashMap<ChannelId, UnixDatagram>, KernelError>>()?;
        let (handles, sockets) = unzip(files);

        Ok(Self {
            root: sim.params.root,
            rng: StdRng::seed_from_u64(sim.params.seed),
            timestep: sim.params.timestep,
            channels,
            nodes: new_nodes,
            handles,
            sockets,
            channel_names,
            node_names,
            run_handles,
        })
    }

    #[instrument(skip_all)]
    fn check_handles(handles: Vec<RunHandle>) -> Result<Vec<RunHandle>, KernelError> {
        handles
            .into_iter()
            .map(|mut handle| {
                if let Ok(Some(_)) = handle.process.try_wait() {
                    error!("Process prematurely exited");
                    let pid = handle.process.id();
                    let output = handle.process.wait_with_output().unwrap();
                    Err(KernelError::ProcessExit {
                        node: handle.node,
                        node_id: handle.node_id,
                        protocol: handle.protocol,
                        pid,
                        output,
                    })
                } else {
                    Ok(handle)
                }
            })
            .collect::<Result<_, KernelError>>()
    }

    #[instrument(skip_all)]
    #[allow(unused_variables)]
    pub fn run(mut self, cmd: RunCmd, logs: Option<PathBuf>) -> Result<(), KernelError> {
        let delta = self.time_delta();
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
            self.channels,
            self.channel_names,
            self.handles,
            self.sockets,
        );
        let mut frame_time_exceeded: u64 = 0;
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
            self.run_handles = Self::check_handles(self.run_handles)?;

            if let Ok(elapsed) = start.elapsed() {
                if elapsed < delta {
                    std::thread::sleep(delta - elapsed);
                } else {
                    frame_time_exceeded <<= 1;
                    frame_time_exceeded |= 1;
                    match frame_time_exceeded.count_ones() {
                        n if n >= 48 => {
                            warn!(
                                "{n} out of the last {} frames have exceeded the timestep delta. Consider using a longer timestep.",
                                u64::BITS
                            );
                            frame_time_exceeded = 0;
                        }
                        _ => {}
                    }
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
