use chrono::{DateTime, Utc};
use kernel::{self, Kernel};
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use std::fs::File;
use std::path::Path;
use std::time::SystemTime;
use std::{collections::HashSet, sync::mpsc};
use tracing_subscriber::{EnvFilter, filter, fmt, prelude::*};

use anyhow::{Result, ensure};
use config::ast;
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
        args.cmd != RunCmd::Playback || args.logs.is_some(),
        format!(
            "Must provide a directory for `logs` argument when running command `{}`",
            args.cmd
        )
    );
    let sim = config::parse(args.config.into())?;
    setup_logging(&sim.params.root, args.cmd)?;
    let run_handles = runner::run(&sim)?;
    let protocol_links = get_fs_links(&sim, &run_handles, args.cmd)?;

    let (tx, _) = mpsc::channel();
    let fs = args.nexus_root.map(NexusFs::new).unwrap_or_default();
    let (sess, kernel_links) = fs.with_links(protocol_links)?.with_logger(tx).mount()?;
    Kernel::new(sim, kernel_links)?.run(args.cmd, args.logs)?;
    sess.join();
    Ok(())
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
        (Some(make_logfile(tx)?), Some(make_logfile(rx)?))
    } else {
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

fn get_fs_links(
    sim: &ast::Simulation,
    handles: &[runner::RunHandle],
    run_cmd: RunCmd,
) -> Result<Vec<NexusLink>, fuse::errors::LinkError> {
    let mut links = vec![];
    for runner::RunHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
    } in handles
    {
        let nodes = sim.nodes.get(node_handle).unwrap();
        for node in nodes {
            let protocol = node.protocols.get(protocol_handle).unwrap();
            let pid = process.id();
            let inbound = protocol.inbound_links();
            let outbound = protocol.outbound_links();

            for link in inbound
                .iter()
                .chain(outbound.iter())
                .collect::<HashSet<&ast::LinkHandle>>()
                .into_iter()
            {
                let mode = match run_cmd {
                    RunCmd::Simulate => {
                        let file_cmd = match (inbound.contains(link), outbound.contains(link)) {
                            (true, true) => O_RDWR,
                            (true, _) => O_RDONLY,
                            (_, true) => O_WRONLY,
                            _ => unreachable!(),
                        };
                        LinkMode::try_from(file_cmd)?
                    }
                    RunCmd::Playback => LinkMode::PlaybackWrites,
                };

                links.push(NexusLink {
                    pid,
                    node: node_handle.clone(),
                    link: link.clone(),
                    mode,
                });
            }
        }
    }
    Ok(links)
}
