use kernel::Kernel;
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::mpsc,
};

use anyhow::Result;
use config::ast;

use clap::Parser;

use runner::RunMode;
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

    #[arg(short, long, default_value_t = RunMode::Simulate)]
    pub mode: RunMode,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let sim = config::parse(args.config.into())?;
    let run_handles = runner::run(&sim)?;
    let protocol_links = get_fs_links(&sim, &run_handles, args.mode)?;

    let (tx, _) = mpsc::channel();
    let fs = args.nexus_root.map(fuse::NexusFs::new).unwrap_or_default();
    let (sess, kernel_links) = fs.with_links(protocol_links)?.with_logger(tx).mount()?;
    Kernel::new(sim, kernel_links)?.run(args.mode)?;
    sess.join();
    Ok(())
}

fn get_fs_links(
    sim: &ast::Simulation,
    handles: &[runner::RunHandle],
    run_mode: RunMode,
) -> Result<Vec<fuse::NexusLink>, fuse::errors::LinkError> {
    let mut links = vec![];
    for runner::RunHandle {
        node: node_handle,
        protocol: protocol_handle,
        process,
    } in handles
    {
        let node = sim.nodes.get(node_handle).unwrap();
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
            let mode = match run_mode {
                RunMode::Simulate => {
                    let file_mode = match (inbound.contains(link), outbound.contains(link)) {
                        (true, true) => O_RDWR,
                        (true, _) => O_RDONLY,
                        (_, true) => O_WRONLY,
                        _ => unreachable!(),
                    };
                    fuse::LinkMode::try_from(file_mode)?
                }
                RunMode::Playback => fuse::LinkMode::PlaybackWrites,
            };

            links.push(fuse::NexusLink {
                pid,
                link: link.clone(),
                mode,
            });
        }
    }
    Ok(links)
}
