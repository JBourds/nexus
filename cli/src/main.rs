use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
};

use anyhow::Result;
use clap::Parser;
use config::ast;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Configuration toml file for the simulation
    #[arg(short, long)]
    config: String,

    /// Location where the NexusFS should be mounted during simulation
    #[arg(short, long)]
    nexus_root: Option<PathBuf>,
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
    let fs = args.nexus_root.map(fuse::NexusFs::new).unwrap_or_default();
    let root = fs.root().clone();
    #[allow(unused_variables)]
    let (sess, mut kernel_links) = fs.with_links(protocol_links)?.with_logger(tx).mount()?;
    while !root.exists() {}

    let mut send_queue = HashMap::new();
    let pids = processes
        .iter()
        .map(|(_, _, process)| process.id())
        .collect::<HashSet<_>>();

    loop {
        for (node_handle, protocol_handle, process) in processes.iter() {
            while let Ok(msg) = rx.try_recv() {
                println!("{msg}");
            }

            println!("{node_handle}.{protocol_handle}");
            for ((pid, handle), socket) in kernel_links
                .iter_mut()
                .filter(|((pid, _), _)| *pid == process.id())
            {
                // Handle all outbound connections
                let mut msg_len = [0u8; core::mem::size_of::<usize>()];
                if let Ok(received) = socket.recv(&mut msg_len) {
                    if received != core::mem::size_of::<usize>() {
                        eprintln!(
                            "Received {received} for message header but expected {}",
                            core::mem::size_of::<usize>()
                        );
                    }
                    let required_capacity = usize::from_ne_bytes(msg_len);
                    let mut recv_buf = vec![0; required_capacity];
                    if let Ok(received) = socket.recv(recv_buf.as_mut_slice()) {
                        if received != required_capacity {
                            eprintln!(
                                "Error: received {received} but expected {required_capacity}"
                            );
                        } else {
                            println!("Received: {}", String::from_utf8_lossy(&recv_buf));
                        }
                    }
                    let msg = Rc::new(recv_buf);

                    // Deliver message to all other entries
                    for pid in pids.iter().filter(|their_pid| *their_pid != pid) {
                        send_queue
                            .entry((*pid, handle.clone()))
                            .or_insert(Vec::new())
                            .push(Rc::clone(&msg));
                    }
                }

                // Handle inbound connections
                for msg in send_queue
                    .remove_entry(&(*pid, handle.clone()))
                    .map(|(_, val)| val)
                    .unwrap_or_default()
                {
                    let msg_len = msg.len().to_ne_bytes();
                    socket.send(&msg_len)?;
                    socket.send(&msg)?;
                }
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    #[allow(unreachable_code)]
    sess.join();
    Ok(())
}
