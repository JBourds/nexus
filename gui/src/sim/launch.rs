use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossbeam_channel::Sender;
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, filter, fmt};

use config::ast::{self, ChannelType};
use fuse::PID;
use fuse::channel::{ChannelMode, NexusChannel};
use fuse::fs::*;
use kernel::Kernel;
use runner::ProtocolHandle;
use runner::cli::RunCmd;
use trace::format::TraceHeader;
use trace::writer::TraceWriter;

use crate::sim::bridge::{GuiEvent, ReloadableSimLayer, SimSinks};
use crate::sim::controller::SimController;

/// Global sinks handle, initialised once on first simulation launch.
static SINKS: OnceLock<SimSinks> = OnceLock::new();

/// Ensure the global tracing subscriber is installed exactly once.
/// Returns a clone of the shared `SimSinks` handle.
fn ensure_global_subscriber() -> SimSinks {
    let sinks = SINKS.get_or_init(|| {
        let sinks = SimSinks::new();

        let sim_layer = ReloadableSimLayer::new(sinks.clone());
        let sim_filter =
            filter::filter_fn(|metadata| matches!(metadata.target(), "tx" | "rx" | "drop"));
        let fmt_filter =
            filter::filter_fn(|metadata| !matches!(metadata.target(), "tx" | "rx" | "drop"));

        let subscriber = tracing_subscriber::registry()
            .with(sim_layer.with_filter(sim_filter))
            .with(
                fmt::layer()
                    .with_filter(fmt_filter)
                    .with_filter(EnvFilter::from_default_env()),
            );

        let dispatch = tracing::dispatcher::Dispatch::new(subscriber);
        // Safe: only called once via OnceLock.
        let _ = tracing::dispatcher::set_global_default(dispatch);

        sinks
    });
    sinks.clone()
}

/// Launch a simulation on a background thread.
///
/// Returns the `SimController` for polling events and the simulation directory
/// where `trace.nxs` will be written.
pub fn launch_simulation(
    sim: ast::Simulation,
    fs_root: Option<PathBuf>,
) -> Result<(SimController, PathBuf)> {
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join("nexus.toml"))?;

    let (gui_tx, gui_rx) = crossbeam_channel::unbounded();
    let abort = Arc::new(AtomicBool::new(false));

    let sinks = ensure_global_subscriber();

    let sim_clone = sim.clone();
    let root_clone = root.clone();
    let gui_tx_clone = gui_tx.clone();

    let handle = std::thread::Builder::new()
        .name("nexus-sim".into())
        .spawn(move || {
            if let Err(e) =
                run_simulation(sim_clone, root_clone, fs_root, gui_tx_clone.clone(), sinks.clone())
            {
                let _ = gui_tx_clone.send(GuiEvent::SimulationError(format!("{e:#}")));
            } else {
                let _ = gui_tx_clone.send(GuiEvent::SimulationComplete);
            }
            // Clear sinks so the TraceWriter is flushed/dropped and the
            // GUI sender is released — ready for the next run.
            sinks.clear();
        })
        .context("failed to spawn simulation thread")?;

    Ok((SimController::new(gui_rx, abort, handle), root))
}

fn run_simulation(
    sim: ast::Simulation,
    root: PathBuf,
    fs_root: Option<PathBuf>,
    gui_tx: Sender<GuiEvent>,
    sinks: SimSinks,
) -> Result<()> {
    let trace_path = root.join("trace.nxs");
    let header = TraceHeader {
        node_names: {
            let mut names: Vec<_> = sim.nodes.keys().cloned().collect();
            names.sort();
            names
        },
        channel_names: {
            let mut names: Vec<_> = sim.channels.keys().cloned().collect();
            names.sort();
            names
        },
        timestep_count: sim.params.timestep.count.get(),
    };

    let writer = TraceWriter::create(&trace_path, &header)
        .context("failed to create trace writer")?;

    // Install sinks for this simulation run.
    sinks.install(gui_tx, writer);

    run_inner(sim, fs_root)
}

fn run_inner(sim: ast::Simulation, fs_root: Option<PathBuf>) -> Result<()> {
    let _ = ctrlc::set_handler(|| {});

    runner::build(&sim)?;
    let runc = runner::run(&sim)?;

    let protocol_channels = make_fs_channels(&sim, &runc.handles)?;
    let fs = fs_root.map(NexusFs::new).unwrap_or_default();

    let file_handles = make_file_handles(&sim, &runc.handles);

    let (sess, (tx, rx)) = fs
        .add_processes(&runc.handles)
        .add_channels(protocol_channels)?
        .mount()
        .map_err(|e| anyhow::anyhow!("unable to mount FUSE filesystem: {e:?}"))?;

    let kernel = Kernel::new(sim, runc, file_handles, rx, tx)?;
    let _protocol_handles = kernel.run(RunCmd::Simulate)?;

    drop(sess);
    Ok(())
}

fn make_sim_dir(sim_root: &Path) -> Result<PathBuf> {
    let datetime: DateTime<Utc> = SystemTime::now().into();
    let datetime = datetime.format("%Y-%m-%d_%H:%M:%S").to_string();
    let root = sim_root.join(&datetime);
    if !root.exists() {
        std::fs::create_dir_all(&root)?;
    }
    Ok(root)
}

fn make_file_handles(
    sim: &ast::Simulation,
    handles: &[ProtocolHandle],
) -> Vec<(PID, ast::NodeHandle, ast::ChannelHandle)> {
    let mut res = vec![];
    for ProtocolHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
        ..
    } in handles
    {
        let node = sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let pid = process.as_ref().unwrap().id();

        for channel in protocol
            .subscribers
            .iter()
            .chain(protocol.publishers.iter())
            .collect::<HashSet<&ast::ChannelHandle>>()
            .into_iter()
            .chain(control_files().iter())
        {
            res.push((pid, node_handle.clone(), channel.clone()));
        }
    }
    res
}

fn make_fs_channels(
    sim: &ast::Simulation,
    handles: &[ProtocolHandle],
) -> Result<Vec<NexusChannel>, fuse::errors::ChannelError> {
    let mut channels = vec![];
    for ProtocolHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
        ..
    } in handles
    {
        let node = sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let pid = process.as_ref().unwrap().id();

        for channel in protocol
            .subscribers
            .iter()
            .chain(protocol.publishers.iter())
            .collect::<HashSet<&ast::ChannelHandle>>()
            .into_iter()
        {
            let file_cmd = match (
                protocol.subscribers.contains(channel),
                protocol.publishers.contains(channel),
            ) {
                (true, true) => O_RDWR,
                (true, _) => O_RDONLY,
                (_, true) => O_WRONLY,
                _ => unreachable!(),
            };
            let mode = ChannelMode::try_from(file_cmd)?;

            channels.push(NexusChannel {
                pid,
                node: node_handle.clone(),
                channel: channel.clone(),
                mode,
                max_msg_size: sim
                    .channels
                    .get(channel)
                    .map(|ch| ch.r#type.max_buf_size())
                    .unwrap_or(ChannelType::MSG_MAX_DEFAULT),
            });
        }
    }
    Ok(channels)
}
