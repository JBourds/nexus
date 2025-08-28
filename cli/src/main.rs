use chrono::{DateTime, Utc};
use kernel::{self, Kernel, sources::Source};
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use runner::RunHandle;
use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use std::time::SystemTime;
use tracing_subscriber::{EnvFilter, filter, fmt, prelude::*};

use anyhow::{Result, ensure};
use config::ast::{self, ChannelType};
use fuse::fs::*;

use clap::Parser;

use runner::RunCmd;
use std::path::PathBuf;

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

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        !matches!(args.cmd, RunCmd::Replay | RunCmd::Logs) || args.logs.is_some(),
        format!(
            "Must provide a directory for `logs` argument when running command `{}`",
            args.cmd
        )
    );
    if args.cmd == RunCmd::Logs {
        Source::print_logs(args.logs.unwrap())?;
        return Ok(());
    }
    let sim = config::parse(args.config.into())?;
    setup_logging(&sim.params.root, args.cmd)?;
    let run_handles = runner::run(&sim)?;
    let protocol_channels = get_fs_channels(&sim, &run_handles, args.cmd)?;

    let fs = args.nexus_root.map(NexusFs::new).unwrap_or_default();
    let (sess, kernel_channels) = fs.with_channels(protocol_channels)?.mount()?;
    // Need to join fs thread so the other processes don't get stuck
    // in an uninterruptible sleep state.
    let run_handles = Kernel::new(sim, kernel_channels, run_handles)?.run(args.cmd, args.logs)?;
    sess.join();
    println!("Simulation Summary:\n\n{}", summarize(run_handles));
    Ok(())
}

fn summarize(mut handles: Vec<RunHandle>) -> String {
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

fn setup_logging(sim_root: &Path, cmd: RunCmd) -> Result<()> {
    let datetime: DateTime<Utc> = SystemTime::now().into();
    let datetime = datetime.format("%Y-%m-%d_%H:%M:%S").to_string();
    let root = sim_root.join(&datetime);
    if !root.exists() {
        std::fs::create_dir_all(&root)?;
    }
    let tx = root.join("tx");
    let rx = root.join("rx");
    let (tx_logfile, rx_logfile) = if cmd == RunCmd::Simulate {
        println!("Saving outbound simulation messages to {tx:?}");
        println!("Saving inbound simulation messages to {rx:?}");
        (Some(make_logfile(tx)?), Some(make_logfile(rx)?))
    } else {
        println!("Saving inbound simulation messages to {rx:?}");
        (None, Some(make_logfile(rx)?))
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
    Ok(())
}

fn make_logfile(path: impl AsRef<Path>) -> Result<File, std::io::Error> {
    File::options().create(true).append(true).open(&path)
}

fn get_fs_channels(
    sim: &ast::Simulation,
    handles: &[runner::RunHandle],
    run_cmd: RunCmd,
) -> Result<Vec<NexusChannel>, fuse::errors::ChannelError> {
    let mut channels = vec![];
    for runner::RunHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
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
