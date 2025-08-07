use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
use std::{collections::HashSet, sync::mpsc};

use anyhow::Result;
use clap::Parser;
use config::ast;
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
    let processes = runner::run(&sim)?;
    let mut protocol_links = vec![];
    for (node_handle, protocol_handle, process) in &processes {
        let node = sim.nodes.get(node_handle).unwrap();
        let protocol = node.protocols.get(protocol_handle).unwrap();
        let pid = process.id();
        let inbound = protocol.inbound_links();
        let outbound = protocol.outbound_links();
        protocol_links.extend(
            inbound
                .iter()
                .chain(outbound.iter())
                .collect::<HashSet<&ast::LinkHandle>>()
                .into_iter()
                .map(|link| {
                    let mode = match (inbound.contains(link), outbound.contains(link)) {
                        (true, true) => O_RDWR,
                        (true, _) => O_RDONLY,
                        (_, true) => O_WRONLY,
                        _ => unreachable!(),
                    };
                    (pid, link.clone(), mode)
                }),
        );
    }

    let (tx, rx) = mpsc::channel();
    let fs = fuse::NexusFs::default();
    let root = fs.root().clone();
    let (sess, mut kernel_links) = fs.with_links(protocol_links)?.with_logger(tx).mount()?;
    while !root.exists() {}

    for (node_handle, protocol_handle, process) in processes.into_iter().rev() {
        for ((pid, handle), socket) in &mut kernel_links {
            let msg = format!("Hello {handle} [{pid}]!");
            let msg_len = msg.len().to_ne_bytes();
            socket.send(&msg_len)?;
            socket.send(msg.as_bytes())?;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        while let Ok(msg) = rx.try_recv() {
            println!("{msg}");
        }
        println!("{node_handle}.{protocol_handle}");
        let output = process.wait_with_output()?;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            println!("{node_handle}.{protocol_handle}: {line}");
        }
        let lines = String::from_utf8_lossy(&output.stderr);
        for line in lines.lines() {
            println!("{node_handle}.{protocol_handle}: {line}");
        }
    }
    sess.join();
    Ok(())
}
