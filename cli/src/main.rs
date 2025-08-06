use anyhow::{Context, Result};
use clap::Parser;
use config::ast;
use fuse;
use runner;
use std::collections::HashMap;
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
    let files = sim.links.keys().map(ToString::to_string);
    let fuse = fuse.with_files(files);
    runner::run(sim)?;
    Ok(())
}
