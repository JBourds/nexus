use config::ast;
use std::{
    fmt::Display,
    process::{Child, Command, Stdio},
    str::FromStr,
};
pub mod errors;
use errors::*;

pub struct RunHandle {
    pub node: ast::NodeHandle,
    pub protocol: ast::ProtocolHandle,
    pub process: Child,
}

#[derive(Debug, Clone, Copy)]
pub enum RunMode {
    Simulate,
    Playback,
}

impl FromStr for RunMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "simulate" => Ok(RunMode::Simulate),
            "playback" => Ok(RunMode::Playback),
            _ => Err(format!("Invalid mode: {}", s)),
        }
    }
}

impl Display for RunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunMode::Simulate => write!(f, "simulate"),
            RunMode::Playback => write!(f, "playback"),
        }
    }
}

pub fn run(sim: &ast::Simulation) -> Result<Vec<RunHandle>, ProtocolError> {
    let mut processes = vec![];
    for (node_name, node) in &sim.nodes {
        for (protocol_name, protocol) in &node.protocols {
            let process = Command::new(protocol.runner.cmd.as_str())
                .current_dir(protocol.root.as_path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .args(protocol.runner.args.as_slice())
                .spawn()
                .expect("Failed to execute process");
            processes.push(RunHandle {
                node: node_name.clone(),
                protocol: protocol_name.clone(),
                process,
            });
        }
    }

    Ok(processes)
}
