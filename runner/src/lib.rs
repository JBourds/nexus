use config::ast;
use std::process::{Command, Output};
use std::rc::Rc;
use std::thread::{self, JoinHandle};
pub mod errors;
use errors::*;

pub fn run(sim: ast::Simulation) -> Result<(), ProtocolError> {
    let mut handles = vec![];
    for (node_name, node) in sim.nodes {
        for (protocol_name, protocol) in node.protocols {
            handles.push((node_name.clone(), protocol_name, run_protocol(protocol)));
        }
    }

    for (node_name, protocol_name, handle) in handles {
        let res = handle.join().map_err(|e| ProtocolError::RunnerError {
            node_name: node_name.clone(),
            protocol_name: protocol_name.clone(),
            msg: format!("{e:?}"),
        })?;
        println!("{node_name}: {protocol_name} - {res:#?}");
    }

    Ok(())
}

fn run_protocol(protocol: ast::NodeProtocol) -> JoinHandle<Output> {
    let thread_name = format!("{}: [{}]", protocol.runner, protocol.root.to_string_lossy());
    thread::Builder::new()
        .name(thread_name.clone())
        .spawn(move || {
            Command::new(protocol.runner.cmd.as_str())
                .current_dir(protocol.root.as_path())
                .args(protocol.runner.args.as_slice())
                .output()
                .expect("Failed to execute process")
        })
        .expect(&format!(
            "Failed to launch thread: `{}`",
            thread_name.as_str()
        ))
}
