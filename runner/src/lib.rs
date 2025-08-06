use config::ast;
use std::process::{Child, Command, Output, Stdio};
pub mod errors;
use errors::*;

pub fn run(sim: ast::Simulation) -> Result<(), ProtocolError> {
    let mut processes = vec![];
    for (node_name, node) in sim.nodes {
        for (protocol_name, protocol) in node.protocols {
            let handle = Command::new(protocol.runner.cmd.as_str())
                .current_dir(protocol.root.as_path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .args(protocol.runner.args.as_slice())
                .spawn()
                .expect("Failed to execute process");
            processes.push((node_name.clone(), protocol_name, handle));
        }
    }

    for (node_name, protocol_name, mut child) in processes {
        let pid = child.id();
        println!("{child:#?}");
        let rc = child.wait().map_err(|e| ProtocolError::RunnerError {
            node_name: node_name.clone(),
            protocol_name: protocol_name.clone(),
            msg: format!("{e:?}"),
        })?;
        let child = child.wait_with_output().unwrap();
        println!("{node_name}: {protocol_name} PID [{pid}] - RC: {rc}");
        println!("stdout: {}", String::from_utf8_lossy(&child.stdout));
        println!("stderr: {}", String::from_utf8_lossy(&child.stderr));
    }

    Ok(())
}
