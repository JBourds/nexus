use config::ast;
use std::process::{Child, Command, Stdio};
pub mod errors;
use errors::*;

pub fn run(
    sim: &ast::Simulation,
) -> Result<Vec<(ast::NodeHandle, ast::LinkHandle, Child)>, ProtocolError> {
    let mut processes = vec![];
    for (node_name, node) in &sim.nodes {
        for (protocol_name, protocol) in &node.protocols {
            let handle = Command::new(protocol.runner.cmd.as_str())
                .current_dir(protocol.root.as_path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .args(protocol.runner.args.as_slice())
                .spawn()
                .expect("Failed to execute process");
            processes.push((node_name.clone(), protocol_name.clone(), handle));
        }
    }

    Ok(processes)
}
