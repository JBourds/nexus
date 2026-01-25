use chrono::{DateTime, Utc};
use fuse::channel::{ChannelMode, NexusChannel};
use kernel::{self, Kernel, sources::Source};
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use runner::cli::OutputDestination;
use runner::{ProtocolHandle, ProtocolSummary};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::stdout;
use std::path::Path;
use std::time::{Duration, SystemTime};
use tracing_subscriber::{EnvFilter, filter, fmt, prelude::*};

use anyhow::Result;
use clap::Parser;
use config::ast::{self, ChannelType};
use fuse::{PID, fs::*};

use runner::{cli::Cli, cli::RunCmd};
use std::path::PathBuf;

use crate::output::to_csv;

mod output;

const CONFIG: &str = "nexus.toml";

fn main() -> Result<()> {
    let args = Cli::parse();
    match args.cmd {
        RunCmd::Simulate => simulate(args),
        RunCmd::Replay { .. } => replay(args),
        RunCmd::Logs { .. } => print_logs(args),
        _ => todo!(),
    }
}

fn simulate(args: Cli) -> Result<()> {
    let sim = config::parse((&args.config).into())?;
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join(CONFIG))?;
    run(args, sim, root)
}

fn replay(args: Cli) -> Result<()> {
    let RunCmd::Replay { logs } = &args.cmd else {
        unreachable!()
    };
    let sim = config::deserialize_config(&logs.join(CONFIG))?;
    let root = make_sim_dir(&sim.params.root)?;
    config::serialize_config(&sim, &root.join(CONFIG))?;
    run(args, sim, root)
}

fn print_logs(args: Cli) -> Result<()> {
    let RunCmd::Logs { logs } = &args.cmd else {
        unreachable!()
    };
    Source::print_logs(logs)?;
    Ok(())
}

fn run(args: Cli, sim: ast::Simulation, root: PathBuf) -> Result<()> {
    println!("Simulation Root: {}", root.to_string_lossy());
    #[allow(unused_variables)]
    let (write_log, read_log) = setup_logging(root.as_path(), &args.cmd)?;
    runner::build(&sim)?;
    let mut summaries = vec![];
    for _ in 0..args.n.unwrap_or(1) {
        let runc = runner::run(&sim)?;
        let protocol_channels = make_fs_channels(&sim, &runc.handles, &args.cmd)?;
        let fs = args
            .nexus_root
            .clone()
            .map(NexusFs::new)
            .unwrap_or_default();

        #[allow(unused_variables)]
        let (sess, (tx, rx)) = fs
            .add_processes(&runc.handles)
            .add_channels(protocol_channels)?
            .mount()
            .expect("unable to mount file system");

        // Need to join fs thread so the other processes don't get stuck
        // in an uninterruptible sleep state.
        let file_handles = make_file_handles(&sim, &runc.handles);
        let protocol_handles =
            Kernel::new(sim.clone(), runc, file_handles, rx, tx)?.run(args.cmd.clone())?;
        summaries.extend(get_output(protocol_handles));
    }
    match args.dest {
        OutputDestination::Stdout => {
            to_csv(stdout(), &summaries);
        }
        OutputDestination::File => {
            let path = root.join(format!("output.{}", args.fmt.extension()));
            let f = OpenOptions::new().write(true).create_new(true).open(path)?;
            to_csv(f, &summaries);
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn fuzz(_args: Cli) -> Result<()> {
    todo!()
}

fn get_output(handles: Vec<ProtocolHandle>) -> Vec<ProtocolSummary> {
    handles
        .into_iter()
        .filter_map(ProtocolHandle::finish)
        .collect()
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

fn setup_logging(root: &Path, cmd: &RunCmd) -> Result<(PathBuf, PathBuf)> {
    let tx = root.join("tx");
    let rx = root.join("rx");
    let (tx_logfile, rx_logfile) = if matches!(cmd, RunCmd::Simulate) {
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
        let pid = process.as_ref().unwrap().id();

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
    run_cmd: &RunCmd,
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
        let pid = process.as_ref().unwrap().id();

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
                RunCmd::Replay { .. } => ChannelMode::ReplayWrites,
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
