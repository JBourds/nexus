pub mod errors;
mod helpers;
pub mod log;
mod resolver;
mod router;
pub mod sources;
mod types;

use fuse::PID;
use helpers::{make_handles, unzip};
use rand::{SeedableRng, rngs::StdRng};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
    time::{Duration, SystemTime},
};

use config::ast::{self, TimestepConfig};
use runner::{RunCmd, RunHandle, cgroups};
use tracing::{error, instrument, warn};
use types::*;

use crate::router::Router;
use crate::sources::Source;
use crate::{
    errors::{KernelError, SourceError},
    resolver::ResolvedChannels,
};
extern crate tracing;

/// Unique identifier for a channel belonging to a node protocol
/// - `fuse::PID`: Process identifier (executing node protocol)
/// - `NodeHandle`: Node the process belongs to.
/// - `ChannelHandle`: Channel the connection is over.
pub type ChannelId = (fuse::PID, NodeHandle, ChannelHandle);
pub type FileHandles = Vec<(u32, String, String)>;

struct KernelServer<H, S, R> {
    handle: JoinHandle<H>,
    tx: mpsc::Sender<S>,
    rx: mpsc::Receiver<R>,
}

impl<H, S, R> KernelServer<H, S, R> {
    fn new(handle: JoinHandle<H>, tx: mpsc::Sender<S>, rx: mpsc::Receiver<R>) -> Self {
        Self { handle, tx, rx }
    }
}

const TX: &str = "tx";

#[allow(unused)]
pub struct Kernel {
    root: PathBuf,
    rng: StdRng,
    timestep: TimestepConfig,
    channels: ResolvedChannels,
    run_handles: Vec<RunHandle>,
    tx: mpsc::Sender<fuse::KernelMessage>,
    rx: mpsc::Receiver<fuse::FsMessage>,
}

impl Kernel {
    /// Create the kernel instance.
    ///
    /// # Arguments
    /// * `sim`: Simulation AST.
    /// * `run_handles`: Handles used to monitor each executing program.
    /// * `rx`: Channel to receive file system requests for.
    /// * `tx`: Channel to deliver kernel responses to the file system.
    pub fn new(
        sim: ast::Simulation,
        run_handles: Vec<RunHandle>,
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
            channels,
            run_handles,
            rx,
            tx,
        })
    }

    #[instrument(skip_all)]
    #[allow(unused_variables)]
    pub fn run(
        self,
        cmd: RunCmd,
        root_cgroup: PathBuf,
        log: Option<PathBuf>,
    ) -> Result<Vec<RunHandle>, KernelError> {
        let delta = self.time_delta();
        let Self {
            root,
            rng,
            timestep,
            channels,
            mut run_handles,
            tx,
            rx,
        } = self;
        let mut router_server = {
            let source = Self::get_write_source(rx, cmd, log).map_err(KernelError::SourceError)?;
            Router::new(tx, channels, timestep, rng, source)
        }?;

        let node_cgroup = cgroups::nodes_cgroup(&root_cgroup);
        cgroups::freeze(&node_cgroup, false);

        for timestep in 0..self.timestep.count.into() {
            let start = SystemTime::now();
            while start.elapsed().is_ok_and(|elapsed| elapsed < delta) {
                router_server.poll(timestep)?;
                run_handles = Self::check_handles(run_handles)?;
            }
            if start.elapsed().is_err() {
                return Err(KernelError::TimestepError(timestep));
            }
        }

        // Handle any outstanding FS requests so it can be cleanly unmounted
        cgroups::freeze(&node_cgroup, true);
        router_server.shutdown()?;

        Ok(run_handles)
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
