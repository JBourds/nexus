use anyhow::{Context, Result};
use clap::Parser;
use fuse;
use runner;
use std::os::unix::net::SocketAddr;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    /// Configuration toml file for the simulation
    config: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let sim = config::parse(args.config.into())?;
    let fuse = fuse::NexusFs::default();
    let root = fuse.root();
    let files = sim
        .links
        .keys()
        .map(|link_name| SocketAddr::from_pathname(root.join(link_name)))
        .collect::<std::io::Result<Vec<_>>>()?;
    let fuse = fuse.with_files(files);
    runner::run(sim)?;
    Ok(())
}
