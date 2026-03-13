use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
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

use crate::sim::bridge::{GuiEvent, OutputStream, ReloadableSimLayer, SimSinks};
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
                "tx" | "rx" | "drop" | "battery" | "movement" | "motion" | "timestep"
            )
        });
        let fmt_filter = filter::filter_fn(|metadata| {
            !matches!(
                metadata.target(),
                "tx" | "rx" | "drop" | "battery" | "movement" | "motion" | "timestep"
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
                .map(|n| {
                    sim.nodes[n]
                        .energy
                        .charge
                        .as_ref()
                        .map(|c| c.unit.to_nj(c.max))
                })
                .collect()
        },
    };

    let writer =
        TraceWriter::create(&trace_path, &header).context("failed to create trace writer")?;

    // Install sinks for this simulation run.
    sinks.install(gui_tx.clone(), writer);

    run_inner(sim, root, fs_root, gui_tx, abort, pause, time_dilation)
}

fn run_inner(
    sim: ast::Simulation,
    sim_dir: PathBuf,
    fs_root: Option<PathBuf>,
    gui_tx: Sender<GuiEvent>,
    abort: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    time_dilation: Arc<AtomicU64>,
) -> Result<()> {
    let _ = ctrlc::set_handler(|| {});

    let _ = gui_tx.send(GuiEvent::BuildStarted);
    runner::build(&sim)?;
    let _ = gui_tx.send(GuiEvent::BuildComplete);
    let mut runc = runner::run(&sim)?;

    // Freeze all processes so they don't try to access FUSE files before
    // the filesystem is mounted.
    runc.cgroups.freeze_nodes();

    // Take stdout/stderr from child processes before the kernel consumes the
    // handles. Spawn per-stream reader threads that send lines to the GUI and
    // write them to per-node files in the simulation output directory.
    let reader_threads = spawn_output_readers(&mut runc.handles, &sim_dir, &gui_tx);

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

    // FUSE is mounted; unfreeze processes so they can access their files.
    runc.cgroups.unfreeze_nodes();

    let kernel = KernelBuilder::new(sim, runc, file_handles, rx, tx, remap_tx)
        .abort_flag(abort)
        .pause_flag(pause)
        .time_dilation(time_dilation)
        .build()?;
    let protocol_handles = kernel.run(RunCmd::Simulate {
        config: PathBuf::new(),
    })?;

    // Kill processes first so their pipes close, unblocking reader threads.
    for handle in protocol_handles {
        let _ = handle.finish();
    }

    // Now join reader threads (they will see EOF and exit).
    for thread in reader_threads {
        let _ = thread.join();
    }

    drop(sess);
    Ok(())
}

/// Take stdout/stderr from each child process and spawn reader threads that
/// send lines to the GUI and write them to per-node files in `sim_dir`.
fn spawn_output_readers(
    handles: &mut [ProtocolHandle],
    sim_dir: &Path,
    gui_tx: &Sender<GuiEvent>,
) -> Vec<JoinHandle<()>> {
    let mut threads = Vec::new();

    for handle in handles.iter_mut() {
        let process = match handle.process.as_mut() {
            Some(p) => p,
            None => continue,
        };
        let node = handle.node.clone();
        let protocol = handle.protocol.clone();

        // Stdout reader
        if let Some(stdout) = process.stdout.take() {
            let tx = gui_tx.clone();
            let node = node.clone();
            let protocol = protocol.clone();
            let path = sim_dir.join(format!("{node}.stdout.txt"));
            threads.push(std::thread::spawn(move || {
                read_stream(stdout, &path, &tx, &node, &protocol, OutputStream::Stdout);
            }));
        }

        // Stderr reader
        if let Some(stderr) = process.stderr.take() {
            let tx = gui_tx.clone();
            let node = node.clone();
            let protocol = protocol.clone();
            let path = sim_dir.join(format!("{node}.stderr.txt"));
            threads.push(std::thread::spawn(move || {
                read_stream(stderr, &path, &tx, &node, &protocol, OutputStream::Stderr);
            }));
        }
    }

    threads
}

/// Read lines from a stream, writing each to a file and sending to the GUI.
fn read_stream<R: std::io::Read>(
    stream: R,
    file_path: &Path,
    gui_tx: &Sender<GuiEvent>,
    node: &str,
    protocol: &str,
    output_stream: OutputStream,
) {
    let reader = BufReader::new(stream);
    let mut file = std::fs::File::create(file_path).ok();

    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Some(ref mut f) = file {
            let _ = writeln!(f, "{line}");
        }
        let _ = gui_tx.send(GuiEvent::ProcessOutputLine {
            node: node.to_string(),
            protocol: protocol.to_string(),
            stream: output_stream,
            line,
        });
    }
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
