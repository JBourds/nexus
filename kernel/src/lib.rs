pub mod errors;
mod events;
mod helpers;
pub mod log;
mod resolver;
pub(crate) mod router;
pub mod sources;
mod status;
mod test_utils;
pub mod types;

pub use router::RouterInput;

use fuse::PID;
use helpers::{make_handles, unzip};

use rand::{SeedableRng, rngs::StdRng};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

use config::ast::{self, TimeUnit, TimestepConfig};
use runner::{ProtocolHandle, RunController, cli::RunCmd};
use tracing::{instrument, warn};
use types::*;

use crate::sources::Source;
use crate::{
    errors::{KernelError, SourceError},
    events::Event,
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

pub struct Kernel {
    root: PathBuf,
    rng: StdRng,
    timestep: TimestepConfig,
    time_dilation: Arc<AtomicU64>,
    channels: ResolvedChannels,
    runc: RunController,
    /// Reply channel handed off to the router; replies flow kernel -> FUSE.
    tx: mpsc::Sender<fuse::KernelMessage>,
    /// Sender into the router's input channel. Cloned from the same channel
    /// the FUSE filesystem already holds, so the kernel main thread shares
    /// the router's wakeup queue with FUSE events.
    router_input_tx: mpsc::Sender<RouterInput>,
    router_input_rx: mpsc::Receiver<RouterInput>,
    remap_tx: mpsc::Sender<(u32, u32)>,
    abort: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
}

/// Builder for constructing a `Kernel` with optional flags.
pub struct KernelBuilder {
    sim: ast::Simulation,
    runc: RunController,
    file_handles: Vec<(PID, ast::NodeHandle, ast::ChannelHandle)>,
    router_input_tx: mpsc::Sender<RouterInput>,
    router_input_rx: mpsc::Receiver<RouterInput>,
    tx: mpsc::Sender<fuse::KernelMessage>,
    remap_tx: mpsc::Sender<(u32, u32)>,
    abort: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
    time_dilation: Option<Arc<AtomicU64>>,
}

impl KernelBuilder {
    pub fn new(
        sim: ast::Simulation,
        runc: RunController,
        file_handles: Vec<(PID, ast::NodeHandle, ast::ChannelHandle)>,
        router_input_tx: mpsc::Sender<RouterInput>,
        router_input_rx: mpsc::Receiver<RouterInput>,
        tx: mpsc::Sender<fuse::KernelMessage>,
        remap_tx: mpsc::Sender<(u32, u32)>,
    ) -> Self {
        Self {
            sim,
            runc,
            file_handles,
            router_input_tx,
            router_input_rx,
            tx,
            remap_tx,
            abort: None,
            pause: None,
            time_dilation: None,
        }
    }

    pub fn abort_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.abort = Some(flag);
        self
    }

    pub fn pause_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.pause = Some(flag);
        self
    }

    pub fn time_dilation(mut self, td: Arc<AtomicU64>) -> Self {
        self.time_dilation = Some(td);
        self
    }

    pub fn build(self) -> Result<Kernel, KernelError> {
        let sim = self.sim;
        // Sort nodes lexicographically for deterministic ordering
        let mut sorted_nodes: Vec<(ast::NodeHandle, ast::Node)> = sim.nodes.into_iter().collect();
        sorted_nodes.sort_by_key(|(name, _)| name.clone());
        let (node_names, nodes) = unzip(sorted_nodes);
        let node_handles: HashMap<String, NodeHandle> = make_handles(node_names.clone())
            .into_iter()
            .map(|(name, idx)| (name, NodeIdx(idx)))
            .collect();
        let channels = ResolvedChannels::try_resolve(
            sim.channels,
            node_names,
            nodes,
            &node_handles,
            self.file_handles,
            &sim.params.timestep,
        )?;
        Ok(Kernel {
            root: sim.params.root,
            rng: StdRng::seed_from_u64(sim.params.seed),
            timestep: sim.params.timestep,
            time_dilation: self
                .time_dilation
                .unwrap_or_else(|| Arc::new(AtomicU64::new(sim.params.time_dilation.to_bits()))),
            channels,
            runc: self.runc,
            router_input_tx: self.router_input_tx,
            router_input_rx: self.router_input_rx,
            tx: self.tx,
            remap_tx: self.remap_tx,
            abort: self.abort,
            pause: self.pause,
        })
    }
}

impl Kernel {
    /// Emit timestep updates at a capped rate (~10 FPS for most configurations).
    /// For a 1 us timestep, 10000 steps = 10 ms wall-clock, giving ~100 FPS of updates
    /// which the GUI can batch. For ms/s timesteps, every step is fine.
    fn update_interval(unit: TimeUnit) -> u64 {
        match unit {
            ast::TimeUnit::Nanoseconds => 10_000_000,
            ast::TimeUnit::Microseconds => 10_000,
            ast::TimeUnit::Milliseconds => 10,
            ast::TimeUnit::Seconds | ast::TimeUnit::Minutes | ast::TimeUnit::Hours => 1,
        }
    }

    #[instrument(skip_all)]
    #[allow(unused_variables)]
    pub fn run(self, cmd: RunCmd) -> Result<Vec<ProtocolHandle>, KernelError> {
        const RESOURCE_UPDATE_INTERVAL: u64 = 100;
        const MIN_DILATION: f64 = 0.01;
        const FLAG_PAUSE_LEN: u64 = 10;
        // Health checks iterate every protocol PID via try_wait(); at high N
        // this dominates the kernel main loop and starves the routing server
        // of poll opportunities. Premature exits don't need millisecond
        // detection. 50ms wall is plenty.
        const HEALTH_CHECK_INTERVAL: Duration = Duration::from_millis(50);

        let base_delta = self.time_delta();
        let Self {
            root,
            rng,
            timestep,
            time_dilation,
            channels,
            runc,
            tx,
            router_input_tx,
            router_input_rx,
            remap_tx,
            abort,
            pause,
        } = self;
        let mut event_queue = BTreeMap::new();
        // Shared simulated-timestep counter. The kernel main thread writes,
        // the router reads on Tick. Replaces the embedded timestep that used
        // to flow in `Poll(u64)`, eliminating the synchronous reply that
        // made every spin-loop iteration a blocking IPC round-trip.
        let current_ts = Arc::new(AtomicU64::new(0));
        let (energy_tx, energy_rx) = mpsc::channel::<router::EnergyEvents>();
        let mut routing_server = {
            let source = Self::get_write_source(cmd).map_err(KernelError::SourceError)?;
            RoutingServer::serve(
                tx,
                channels,
                timestep,
                rng,
                source,
                remap_tx,
                current_ts.clone(),
                energy_tx,
                router_input_tx,
                router_input_rx,
            )
        }?;
        let mut status_server = StatusServer::serve(time_dilation.clone(), runc)?;
        queue_event(
            &mut event_queue,
            RESOURCE_UPDATE_INTERVAL,
            Event::UpdateResources,
        );

        let ts_update_interval = Self::update_interval(timestep.unit);
        let mut prev_speed = f64::from_bits(time_dilation.load(Ordering::Relaxed));
        let mut last_health_check = std::time::Instant::now()
            .checked_sub(HEALTH_CHECK_INTERVAL)
            .unwrap_or_else(std::time::Instant::now);
        let mut next_tick_at = std::time::Instant::now();
        'outer: for timestep in 0..self.timestep.count.into() {
            if abort.as_ref().is_some_and(|a| a.load(Ordering::Relaxed)) {
                break;
            }
            if timestep % ts_update_interval == 0 {
                tracing::event!(target: "timestep", tracing::Level::TRACE, timestep = timestep);
            }
            // Spin-wait while paused, checking abort each iteration.
            while pause.as_ref().is_some_and(|p| p.load(Ordering::Relaxed)) {
                if abort.as_ref().is_some_and(|a| a.load(Ordering::Relaxed)) {
                    break 'outer;
                }
                std::thread::sleep(Duration::from_millis(FLAG_PAUSE_LEN));
            }

            let speed = f64::from_bits(time_dilation.load(Ordering::Relaxed));
            let delta = base_delta.div_f64(speed.max(MIN_DILATION));

            if let Some(events) = dequeue(&mut event_queue, timestep) {
                for event in events {
                    match event {
                        Event::UpdateResources => {
                            status_server.update_resources()?;
                            queue_event(
                                &mut event_queue,
                                timestep + RESOURCE_UPDATE_INTERVAL,
                                Event::UpdateResources,
                            );
                        }
                    }
                }
            } else if speed != prev_speed {
                status_server.update_resources()?;
                prev_speed = speed;
            }

            // Throttled health check: at high N, the synchronous round-trip
            // plus N `try_wait()` syscalls dominates the kernel main loop.
            // Bound it to once per HEALTH_CHECK_INTERVAL wall-clock.
            if last_health_check.elapsed() >= HEALTH_CHECK_INTERVAL {
                last_health_check = std::time::Instant::now();
                match status_server.check_health()? {
                    status::messages::StatusMessage::Ok => {}
                    status::messages::StatusMessage::PrematureExit => {
                        break 'outer;
                    }
                    status::messages::StatusMessage::Respawned { .. } => {}
                }
            }

            // Publish the new timestep, then signal the router. The router
            // reads `current_ts` to learn the value; we do not wait for a
            // reply.
            current_ts.store(timestep, Ordering::Release);
            routing_server.tick()?;

            // Drain any energy events the router has pushed since last tick.
            for events in energy_rx.try_iter() {
                for name in events.depleted {
                    status_server.freeze_node(name)?;
                }
                for name in events.recovered {
                    if let status::messages::StatusMessage::Respawned { pid_changes, .. } =
                        status_server.respawn_node(name)?
                        && !pid_changes.is_empty()
                    {
                        routing_server.remap_pids(pid_changes)?;
                    }
                }
            }

            // Pace simulated time to wall-clock time. Catch up missed ticks
            // (no sleep) if we have already fallen behind.
            next_tick_at += delta;
            let now = std::time::Instant::now();
            if next_tick_at > now {
                std::thread::sleep(next_tick_at - now);
            } else {
                next_tick_at = now;
            }
        }

        // Handle any outstanding FS requests so it can be cleanly unmounted
        let run_handles = status_server.shutdown()?;
        routing_server.shutdown()?;

        Ok(run_handles)
    }

    #[instrument(skip_all)]
    fn get_write_source(cmd: RunCmd) -> Result<Source, SourceError> {
        match cmd {
            RunCmd::Simulate { .. } => Source::simulated(),
            RunCmd::Replay { logs } => {
                // Prefer unified trace format if available, fall back to legacy
                let trace_file = logs.join("trace.nxs");
                if trace_file.exists() {
                    return Source::replay_trace(trace_file);
                }
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

fn queue_event(queue: &mut BTreeMap<u64, Vec<Event>>, at: u64, event: Event) {
    let list = queue.entry(at).or_default();
    list.push(event);
}

fn dequeue(queue: &mut BTreeMap<u64, Vec<Event>>, now: u64) -> Option<Vec<Event>> {
    if queue.iter().next().is_some_and(|(&time, _)| time <= now) {
        queue.pop_first().map(|(_, events)| events)
    } else {
        None
    }
}
