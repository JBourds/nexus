pub mod errors;
mod helpers;
pub mod log;
mod resolver;
mod router;
pub mod sources;
mod status;
mod types;

use fuse::PID;
use helpers::{make_handles, unzip};
use rand::{SeedableRng, rngs::StdRng};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::{Duration, SystemTime},
};

use config::ast::{self, TimestepConfig};
use runner::{CgroupController, ProtocolHandle, RunCmd, RunController};
use tracing::{instrument, warn};
use types::*;

use crate::sources::Source;
use crate::{
    errors::{KernelError, SourceError},
    resolver::ResolvedChannels,
};
use crate::{router::RoutingServer, status::StatusServer};
extern crate tracing;

const TX: &str = "tx";

/// Unique identifier for a channel belonging to a node protocol
/// - `fuse::PID`: Process identifier (executing node protocol)
/// - `NodeHandle`: Node the process belongs to.
/// - `ChannelHandle`: Channel the connection is over.
pub type ChannelId = (fuse::PID, NodeHandle, ChannelHandle);
pub type FileHandles = Vec<(u32, String, String)>;

/// Basic interface for any server.
/// - handle: Controlling handle (typically join handle) for server
/// - tx: Channel to send messages to.
/// - rx: Channel to receive messages from.
#[derive(Debug)]
struct KernelServer<H, S, R> {
    handle: H,
    tx: mpsc::Sender<S>,
    rx: mpsc::Receiver<R>,
}

impl<H, S, R> KernelServer<H, S, R> {
    fn new(handle: H, tx: mpsc::Sender<S>, rx: mpsc::Receiver<R>) -> Self {
        Self { handle, tx, rx }
    }
}

#[allow(unused)]
pub struct Kernel {
    root: PathBuf,
    rng: StdRng,
    timestep: TimestepConfig,
    time_dilation: f64,
    channels: ResolvedChannels,
    runc: RunController,
    tx: mpsc::Sender<fuse::KernelMessage>,
    rx: mpsc::Receiver<fuse::FsMessage>,
}

impl Kernel {
    /// Create the kernel instance.
    ///
    /// # Arguments
    /// * `sim`: Simulation AST.
    /// * `runc`: Unified controller with all information for handling runtime
    ///   management of processes.
    /// * `file_handles`: Handles used for each unique file in fuse FS.
    /// * `rx`: Channel to receive file system requests for.
    /// * `tx`: Channel to deliver kernel responses to the file system.
    pub fn new(
        sim: ast::Simulation,
        runc: RunController,
        file_handles: Vec<(PID, ast::NodeHandle, ast::ChannelHandle)>,
        rx: mpsc::Receiver<fuse::FsMessage>,
        tx: mpsc::Sender<fuse::KernelMessage>,
    ) -> Result<Self, KernelError> {
        // CRUCIAL: Sort nodes by their name lexicographically since we are
        // not guaranteed a consistent ordering by hash maps
        let mut sorted_nodes: Vec<(ast::NodeHandle, ast::Node)> = sim.nodes.into_iter().collect();
        sorted_nodes.sort_by_key(|(name, _)| name.clone());
        let (node_names, nodes) = unzip(sorted_nodes);
        let node_handles = make_handles(node_names.clone());
        let channels = ResolvedChannels::try_resolve(
            sim.channels,
            node_names,
            nodes,
            &node_handles,
            file_handles,
        )?;
        Ok(Self {
            root: sim.params.root,
            rng: StdRng::seed_from_u64(sim.params.seed),
            timestep: sim.params.timestep,
            time_dilation: sim.params.time_dilation,
            channels,
            runc,
            rx,
            tx,
        })
    }

    #[instrument(skip_all)]
    #[allow(unused_variables)]
    pub fn run(
        self,
        cmd: RunCmd,
        log: Option<PathBuf>,
    ) -> Result<Vec<ProtocolHandle>, KernelError> {
        let delta = self.time_delta();
        let Self {
            root,
            rng,
            timestep,
            time_dilation,
            channels,
            runc,
            tx,
            rx,
        } = self;
        let mut routing_server = {
            let source = Self::get_write_source(rx, cmd, log).map_err(KernelError::SourceError)?;
            RoutingServer::serve(tx, channels, timestep, rng, source)
        }?;
        let mut status_server = StatusServer::serve(time_dilation, runc)?;

        'outer: for timestep in 0..self.timestep.count.into() {
            let start = SystemTime::now();
            while start.elapsed().is_ok_and(|elapsed| elapsed < delta) {
                // send all commands
                match status_server.check_health()? {
                    status::messages::StatusMessage::Ok => {}
                    status::messages::StatusMessage::PrematureExit => {
                        break 'outer;
                    }
                }
                status_server.update_resources()?;
                routing_server.poll(timestep)?;
            }
            if start.elapsed().is_err() {
                return Err(KernelError::TimestepError(timestep));
            }
        }

        // Handle any outstanding FS requests so it can be cleanly unmounted
        let run_handles = status_server.shutdown()?;
        routing_server.shutdown()?;

        Ok(run_handles)
    }

    #[instrument(skip_all)]
    fn get_write_source(
        rx: Receiver<fuse::FsMessage>,
        cmd: RunCmd,
        logs: Option<PathBuf>,
    ) -> Result<Source, SourceError> {
        match cmd {
            RunCmd::Simulate => Source::simulated(rx),
            RunCmd::Replay => {
                let Some(logs) = logs else {
                    return Err(SourceError::NoReplayLog);
                };
                let logfile = logs.join(TX);
                if !logfile.exists() {
                    return Err(SourceError::NonexistentReplayLog(logfile));
                }
                Source::replay(logfile)
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
            _ => unreachable!(),
        }
    }
}
