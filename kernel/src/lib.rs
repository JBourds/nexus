pub mod errors;
mod helpers;
pub mod log;
mod router;
pub mod sources;
mod types;

use fuse::fs::{ReadSignal, WriteSignal};
use fuse::{KernelChannelHandle, KernelControlFile};

use helpers::{make_handles, unzip};
use rand::{SeedableRng, rngs::StdRng};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use std::{collections::HashMap, os::unix::net::UnixDatagram};

use config::ast::{self, TimestepConfig};
use runner::{RunCmd, RunHandle};
use tracing::{error, instrument, warn};
use types::*;

use crate::errors::{ConversionError, KernelError, SourceError};
use crate::router::Router;
use crate::sources::Source;
extern crate tracing;

/// Unique identifier for a channel belonging to a node protocol
/// - `fuse::PID`: Process identifier (executing node protocol)
/// - `NodeHandle`: Node the process belongs to.
/// - `ChannelHandle`: Channel the connection is over.
pub type ChannelId = (fuse::PID, NodeHandle, ChannelHandle);

pub type ReadControl = KernelControlFile<ReadSignal>;
pub type WriteControl = KernelControlFile<WriteSignal>;
pub type Readers = Vec<ReadControl>;
pub type Writers = Vec<WriteControl>;

#[allow(unused)]
pub struct Kernel {
    root: PathBuf,
    rng: StdRng,
    timestep: TimestepConfig,
    channels: Vec<Channel>,
    nodes: Vec<Node>,
    handles: Vec<ChannelId>,
    sockets: Vec<UnixDatagram>,
    readers: Readers,
    writers: Writers,
    channel_names: Vec<String>,
    node_names: Vec<String>,
    run_handles: Vec<RunHandle>,
}

impl Kernel {
    /// Create the kernel instance.
    ///
    /// # Arguments
    /// * `sim`: Simulation AST.
    /// * `files`: List of mappings from open channels within an executing node
    ///   protocol to the node it belongs to and its unix domain socket pair.
    /// * `run_handles`: Handles used to monitor each executing program.
    pub fn new(
        sim: ast::Simulation,
        files: fuse::KernelChannels,
        run_handles: Vec<RunHandle>,
    ) -> Result<Self, KernelError> {
        let (node_names, nodes) = unzip(sim.nodes);
        let node_handles = make_handles(node_names.clone());

        // We need to resolve any internal channels as new channels over an
        // ideal link.
        let (mut channel_names, channels) = unzip(sim.channels);
        let channel_handles = make_handles(channel_names.clone());
        let mut new_nodes = vec![];
        let mut internal_channels = vec![];
        let mut internal_node_channel_handles = HashMap::new();
        for (handle, (node_name, node)) in node_names
            .clone()
            .into_iter()
            .zip(nodes.into_iter())
            .enumerate()
        {
            let (new_node, new_internals) =
                Node::from_ast(node, handle, &channel_handles, &node_handles)
                    .map_err(KernelError::KernelInit)?;
            let (new_internal_names, new_internal_channels) = unzip(new_internals);
            new_nodes.push(new_node);
            channel_names.extend(new_internal_names.clone());
            internal_channels.extend(new_internal_channels);
            for (handle, internal_name) in
                (channel_names.len() - 1..).zip(new_internal_names.into_iter())
            {
                internal_node_channel_handles.insert((node_name.clone(), internal_name), handle);
            }
        }

        let channels = Channel::from_ast(channels, internal_channels, &new_nodes)
            .map_err(KernelError::KernelInit)?;

        let lookup_channel = |pid: fuse::PID, channel_name: String, handle: KernelChannelHandle| {
            let node_handle = *node_handles.get(&handle.node).unwrap();
            internal_node_channel_handles
                .get(&(handle.node.clone(), channel_name.clone()))
                .or(channel_handles.get(&channel_name))
                .ok_or(KernelError::KernelInit(
                    ConversionError::ChannelHandleConversion(channel_name),
                ))
                .map(|channel_handle| ((pid, node_handle, *channel_handle), handle))
        };
        let files = files
            .into_iter()
            .map(|((pid, channel_name), handle)| lookup_channel(pid, channel_name, handle))
            .collect::<Result<HashMap<ChannelId, KernelChannelHandle>, KernelError>>()?;
        let (handles, files) = unzip(files);
        let (readers, writers, sockets) = files.into_iter().fold(
            (Vec::new(), Vec::new(), Vec::new()),
            |(mut readers, mut writers, mut sockets), handle| {
                readers.push(handle.read);
                writers.push(handle.write);
                sockets.push(handle.file);
                (readers, writers, sockets)
            },
        );

        Ok(Self {
            root: sim.params.root,
            rng: StdRng::seed_from_u64(sim.params.seed),
            timestep: sim.params.timestep,
            channels,
            nodes: new_nodes,
            readers,
            writers,
            handles,
            sockets,
            channel_names,
            node_names,
            run_handles,
        })
    }

    #[instrument(skip_all)]
    #[allow(unused_variables)]
    pub fn run(self, cmd: RunCmd, log: Option<PathBuf>) -> Result<String, KernelError> {
        let delta = self.time_delta();
        let Self {
            root,
            rng,
            timestep,
            channels,
            nodes,
            handles,
            sockets,
            readers,
            writers,
            channel_names,
            node_names,
            mut run_handles,
        } = self;
        let mut source = Self::get_write_source(cmd, &sockets, readers, writers, log)
            .map_err(KernelError::SourceError)?;
        let mut router = Router::new(
            nodes,
            node_names,
            channels,
            channel_names,
            handles,
            sockets,
            timestep,
            rng,
        );

        let mut frame_time_exceeded: u64 = 0;
        for timestep in 0..self.timestep.count.into() {
            let start = SystemTime::now();
            source
                .poll(&mut router, timestep, delta)
                .map_err(KernelError::SourceError)?;
            run_handles = Self::check_handles(run_handles)?;

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
        Ok(Self::make_summary(run_handles))
    }

    fn make_summary(handles: Vec<RunHandle>) -> String {
        let mut summaries = Vec::with_capacity(handles.len());
        // TODO: Figure out how to extract stdout/stderr text here
        for mut handle in handles {
            handle.process.kill().expect("Couldn't kill process.");
            summaries.push(format!(
                "{}.{}:\nstdout: {:?}\nstderr: {:?}\n",
                handle.node,
                handle.protocol,
                handle.process.stdout.take().expect("Expected handle"),
                handle.process.stderr.take().expect("Expected handle"),
            ));
        }
        summaries.join("\n")
    }

    #[instrument(skip_all)]
    fn check_handles(handles: Vec<RunHandle>) -> Result<Vec<RunHandle>, KernelError> {
        let mut process_error = None;
        let mut good_handles = vec![];
        for mut handle in handles {
            if process_error.is_some() {
                let _ = handle.process.kill();
            }
            if let Ok(Some(_)) = handle.process.try_wait() {
                error!("Process prematurely exited");
                let pid = handle.process.id();
                let output = handle.process.wait_with_output().unwrap();
                process_error = Some(KernelError::ProcessExit {
                    node: handle.node,
                    protocol: handle.protocol,
                    pid,
                    output,
                });
            } else {
                good_handles.push(handle);
            }
        }
        if let Some(e) = process_error {
            for mut handle in good_handles {
                let _ = handle.process.kill();
            }
            Err(e)
        } else {
            Ok(good_handles)
        }
    }

    #[instrument(skip_all)]
    fn get_write_source(
        cmd: RunCmd,
        sockets: &[UnixDatagram],
        readers: Readers,
        writers: Writers,
        log: Option<PathBuf>,
    ) -> Result<Source, SourceError> {
        match cmd {
            RunCmd::Simulate => Source::simulated(sockets, readers, writers),
            RunCmd::Replay => {
                let Some(log) = log else {
                    return Err(SourceError::NoReplayLog);
                };
                if !log.exists() {
                    return Err(SourceError::NonexistentReplayLog(log));
                }
                Source::replay(log, readers)
            }
            _ => unreachable!(),
        }
    }

    fn time_delta(&self) -> Duration {
        let length = self.timestep.length.get();
        match self.timestep.unit {
            ast::TimeUnit::Seconds => Duration::from_secs(length),
            ast::TimeUnit::Milliseconds => Duration::from_millis(length),
            ast::TimeUnit::Microseconds => Duration::from_micros(length),
            ast::TimeUnit::Nanoseconds => Duration::from_nanos(length),
        }
    }
}
