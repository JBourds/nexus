use anyhow::Result;
use clap::Parser;
use fuse;
use runner;

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
    let files = sim.links.keys().map(ToString::to_string);

    let processes = runner::run(&sim)?;
    let mut protocol_links = vec![];
    for (node_handle, protocol_handle, process) in &processes {
        let node = sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let links = protocol.links();
        let pid = process.id();
        protocol_links.extend(links.into_iter().map(|link| (pid, link)));
    }

    let (sess, mut kernel_links) = fuse::NexusFs::default()
        .with_files(files)
        .with_links(protocol_links)?
        .mount()?;
    println!("{kernel_links:#?}");
    loop {}
    Ok(())
}
