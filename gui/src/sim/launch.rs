use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossbeam_channel::Sender;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, filter, fmt};

use config::ast::{self, ChannelType};
use fuse::PID;
use fuse::channel::{ChannelMode, NexusChannel};
use fuse::fs::*;
use kernel::KernelBuilder;
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
        let sim_filter = filter::filter_fn(|metadata| {
            matches!(
                metadata.target(),
                "tx" | "rx" | "drop" | "battery" | "movement" | "motion"
            )
        });
        let fmt_filter = filter::filter_fn(|metadata| {
            !matches!(
                metadata.target(),
                "tx" | "rx" | "drop" | "battery" | "movement" | "motion"
            )
        });

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
) -> Result<(SimController, PathBuf, Arc<AtomicU64>)> {
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join("nexus.toml"))?;

    let (gui_tx, gui_rx) = crossbeam_channel::unbounded();
    let abort = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let time_dilation = Arc::new(AtomicU64::new(sim.params.time_dilation.to_bits()));

    let sinks = ensure_global_subscriber();

    let sim_clone = sim.clone();
    let root_clone = root.clone();
    let gui_tx_clone = gui_tx.clone();
    let abort_clone = abort.clone();
    let pause_clone = pause.clone();
    let td_clone = time_dilation.clone();

    let handle = std::thread::Builder::new()
        .name("nexus-sim".into())
        .spawn(move || {
            if let Err(e) = run_simulation(
                sim_clone,
                root_clone,
                fs_root,
                gui_tx_clone.clone(),
                sinks.clone(),
                abort_clone,
                pause_clone,
                td_clone,
            ) {
                let _ = gui_tx_clone.send(GuiEvent::SimulationError(format!("{e:#}")));
            } else {
                let _ = gui_tx_clone.send(GuiEvent::SimulationComplete);
            }
            // Clear sinks so the TraceWriter is flushed/dropped and the
            // GUI sender is released — ready for the next run.
            sinks.clear();
        })
        .context("failed to spawn simulation thread")?;

    Ok((
        SimController::new(gui_rx, abort, pause, handle),
        root,
        time_dilation,
    ))
}

#[allow(clippy::too_many_arguments)]
fn run_simulation(
    sim: ast::Simulation,
    root: PathBuf,
    fs_root: Option<PathBuf>,
    gui_tx: Sender<GuiEvent>,
    sinks: SimSinks,
    abort: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    time_dilation: Arc<AtomicU64>,
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
        node_max_nj: {
            let mut names: Vec<_> = sim.nodes.keys().cloned().collect();
            names.sort();
            names
                .iter()
                .map(|n| sim.nodes[n].energy.charge.as_ref().map(|c| c.unit.to_nj(c.max)))
                .collect()
        },
    };

    let writer =
        TraceWriter::create(&trace_path, &header).context("failed to create trace writer")?;

    // Install sinks for this simulation run.
    sinks.install(gui_tx, writer);

    run_inner(sim, fs_root, abort, pause, time_dilation)
}

fn run_inner(
    sim: ast::Simulation,
    fs_root: Option<PathBuf>,
    abort: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    time_dilation: Arc<AtomicU64>,
) -> Result<()> {
    let _ = ctrlc::set_handler(|| {});

    runner::build(&sim)?;
    let runc = runner::run(&sim)?;

    let protocol_channels = make_fs_channels(&sim, &runc.handles)?;
    let (remap_tx, remap_rx) = std::sync::mpsc::channel();
    let fs = fs_root
        .map(|root| NexusFs::new(root, remap_rx))
        .unwrap_or_default();

    let file_handles = make_file_handles(&sim, &runc.handles);
    let pids: Vec<u32> = runc.handles.iter().filter_map(|h| h.pid()).collect();

    let (sess, (tx, rx)) = fs
        .add_processes(&pids)
        .add_channels(protocol_channels)?
        .mount()
        .map_err(|e| anyhow::anyhow!("unable to mount FUSE filesystem: {e:?}"))?;

    let kernel = KernelBuilder::new(
        sim,
        runc,
        file_handles,
        rx,
        tx,
        remap_tx,
    )
    .abort_flag(abort)
    .pause_flag(pause)
    .time_dilation(time_dilation)
    .build()?;
    let _protocol_handles = kernel.run(RunCmd::Simulate {
        config: PathBuf::new(),
    })?;

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
            let mode = ChannelMode::from_permissions(
                protocol.subscribers.contains(channel),
                protocol.publishers.contains(channel),
            );

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
