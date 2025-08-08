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

    let (tx, rx) = mpsc::channel();
    let fs = args.nexus_root.map(fuse::NexusFs::new).unwrap_or_default();
    #[allow(unused_variables)]
    let (sess, mut kernel_links) = fs.with_links(protocol_links)?.with_logger(tx).mount()?;

    let mut send_queue = HashMap::new();
    let pids = run_handles
        .iter()
        .map(|handle| handle.process.id())
        .collect::<HashSet<_>>();

    loop {
        for runner::RunHandle {
            node: node_handle,
            protocol: protocol_handle,
            process,
        } in run_handles.iter()
        {
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
