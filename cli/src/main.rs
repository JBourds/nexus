use chrono::{DateTime, Utc};
use fuse::channel::{ChannelMode, NexusChannel};
use kernel::{self, Kernel, sources::Source};
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use runner::ProtocolHandle;
use std::fs::File;
use std::path::Path;
use std::time::SystemTime;
use std::{collections::HashSet, num::NonZeroUsize};
use tracing_subscriber::{EnvFilter, filter, fmt, prelude::*};

use anyhow::{Result, ensure};
use config::ast::{self, ChannelType};
use fuse::{PID, fs::*};

use clap::Parser;

use runner::RunCmd;
use std::path::PathBuf;

const CONFIG: &str = "nexus.toml";

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Configuration toml file for the simulation
    #[arg(short, long)]
    pub config: String,

    /// Location where the NexusFS should be mounted during simulation
    #[arg(short, long)]
    pub nexus_root: Option<PathBuf>,

    /// Command to run
    #[arg(long, default_value_t = RunCmd::Simulate)]
    pub cmd: RunCmd,

    /// Directory where logs to be parsed are. Required in the commands which
    /// use it but has no effect in others.
    #[arg(short, long)]
    pub logs: Option<PathBuf>,
}

fn simulate(args: Args) -> Result<()> {
    let sim = config::parse((&args.config).into())?;
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join(CONFIG))?;
    run(args, sim, root)
}

fn replay(args: Args) -> Result<()> {
    let logs = args.logs.as_ref().expect("could not find log directory");
    let sim = config::deserialize_config(&logs.join(CONFIG))?;
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join(CONFIG))?;
    run(args, sim, root)
}

fn run(args: Args, sim: ast::Simulation, root: PathBuf) -> Result<()> {
    let (write_log, read_log) = setup_logging(root.as_path(), args.cmd)?;
    runner::build(&sim)?;
    let (cgroup_controller, protocol_handles) = runner::run(&sim)?;
    let protocol_channels = make_fs_channels(&sim, &protocol_handles, args.cmd)?;

    let fs = args.nexus_root.map(NexusFs::new).unwrap_or_default();
    #[allow(unused_variables)]
    let (sess, (tx, rx)) = fs
        .with_channels(protocol_channels)?
        .mount()
        .expect("unable to mount file system");
    // Need to join fs thread so the other processes don't get stuck
    // in an uninterruptible sleep state.
    let file_handles = make_file_handles(&sim, &protocol_handles);
    let protocol_handles = Kernel::new(
        sim,
        protocol_handles,
        cgroup_controller,
        file_handles,
        rx,
        tx,
    )?
    .run(args.cmd, args.logs)?;

    println!("Simulation Summary:\n\n{}", summarize(protocol_handles));
    println!("Write Log: {write_log:?}");
    println!("Read Log: {read_log:?}");
    Ok(())
}

fn logs(args: Args) -> Result<()> {
    Source::print_logs(args.logs.unwrap())?;
    Ok(())
}

fn fuzz(args: Args) -> Result<()> {
    todo!()
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        !matches!(args.cmd, RunCmd::Replay | RunCmd::Logs) || args.logs.is_some(),
        format!(
            "Must provide a directory for `logs` argument when running command `{}`",
            args.cmd
        )
    );
    match args.cmd {
        RunCmd::Simulate => simulate(args),
        RunCmd::Replay => replay(args),
        RunCmd::Logs => logs(args),
        RunCmd::Fuzz => fuzz(args),
    }
}

fn summarize(mut handles: Vec<ProtocolHandle>) -> String {
    let mut summaries = Vec::with_capacity(handles.len());
    for handle in handles.iter_mut() {
        handle.process.kill().expect("Couldn't kill process.");
    }
    for mut handle in handles {
        handle.process.kill().expect("Couldn't kill process.");
        let output = handle
            .process
            .wait_with_output()
            .expect("Expected process to be completed.");
        summaries.push(format!(
            "{}.{}:\nstdout: {:?}\nstderr: {:?}\n",
            handle.node,
            handle.protocol,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    summaries.join("\n")
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

fn setup_logging(root: &Path, cmd: RunCmd) -> Result<(PathBuf, PathBuf)> {
    let tx = root.join("tx");
    let rx = root.join("rx");
    let (tx_logfile, rx_logfile) = if cmd == RunCmd::Simulate {
        (Some(make_logfile(&tx)?), Some(make_logfile(&rx)?))
    } else {
        (None, Some(make_logfile(&rx)?))
    };
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_filter(filter::filter_fn(|metadata| {
                    !matches!(metadata.target(), "tx" | "rx")
                }))
                .with_filter(EnvFilter::from_default_env()),
        )
        .with(
            kernel::log::BinaryLogLayer::new(tx_logfile)
                .with_filter(filter::filter_fn(|metadata| metadata.target() == "tx")),
        )
        .with(
            kernel::log::BinaryLogLayer::new(rx_logfile)
                .with_filter(filter::filter_fn(|metadata| metadata.target() == "rx")),
        )
        .init();
    Ok((tx, rx))
}

fn make_logfile(path: impl AsRef<Path>) -> Result<File, std::io::Error> {
    File::options().create(true).append(true).open(&path)
}

fn make_file_handles(
    sim: &ast::Simulation,
    handles: &[runner::ProtocolHandle],
) -> Vec<(PID, ast::NodeHandle, ast::ChannelHandle)> {
    let mut res = vec![];
    for runner::ProtocolHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
        ..
    } in handles
    {
        let node = &sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let pid = process.id();

        for channel in protocol
            .subscribers
            .iter()
            .chain(protocol.publishers.iter())
            .collect::<HashSet<&ast::ChannelHandle>>()
            .into_iter()
        {
            res.push((pid, node_handle.clone(), channel.clone()));
        }
    }
    res
}

fn make_fs_channels(
    sim: &ast::Simulation,
    handles: &[runner::ProtocolHandle],
    run_cmd: RunCmd,
) -> Result<Vec<NexusChannel>, fuse::errors::ChannelError> {
    let mut channels = vec![];
    for runner::ProtocolHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
        ..
    } in handles
    {
        let node = &sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let pid = process.id();

        for channel in protocol
            .subscribers
            .iter()
            .chain(protocol.publishers.iter())
            .collect::<HashSet<&ast::ChannelHandle>>()
            .into_iter()
        {
            let mode = match run_cmd {
                RunCmd::Simulate => {
                    let file_cmd = match (
                        protocol.subscribers.contains(channel),
                        protocol.publishers.contains(channel),
                    ) {
                        (true, true) => O_RDWR,
                        (true, _) => O_RDONLY,
                        (_, true) => O_WRONLY,
                        _ => unreachable!(),
                    };
                    ChannelMode::try_from(file_cmd)?
                }
                RunCmd::Replay => ChannelMode::ReplayWrites,
                RunCmd::Fuzz => ChannelMode::FuzzWrites,
                _ => unreachable!(),
            };

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
