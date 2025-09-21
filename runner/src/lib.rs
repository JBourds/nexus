use config::ast::{self};
use std::{
    fmt::Display,
    fs::OpenOptions,
    io::Write,
    num::NonZeroU64,
    process::{Child, Command, Stdio},
    str::FromStr,
};
mod assignment;
mod cgroups;
pub mod errors;
use errors::*;

use crate::{
    assignment::CpuAssignment,
    cgroups::{node_cgroup, protocol_cgroup, simulation_cgroup},
};

#[derive(Debug)]
pub struct RunHandle {
    /// Name of the node. Unique identifer within the simulation.
    pub node: ast::NodeHandle,
    /// Name of the protocol. Unique identifier for a process within a node.
    pub protocol: ast::ProtocolHandle,
    /// Handle for the executing process.
    pub process: Child,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RunCmd {
    Simulate,
    Replay,
    Logs,
    Fuzz,
}

impl FromStr for RunCmd {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "simulate" => Ok(RunCmd::Simulate),
            "replay" => Ok(RunCmd::Replay),
            "logs" => Ok(RunCmd::Logs),
            "fuzz" => Ok(RunCmd::Fuzz),
            _ => Err(format!("Invalid mode: {}", s)),
        }
    }
}

impl Display for RunCmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunCmd::Simulate => write!(f, "simulate"),
            RunCmd::Replay => write!(f, "replay"),
            RunCmd::Logs => write!(f, "logs"),
            RunCmd::Fuzz => write!(f, "fuzz"),
        }
    }
}

/// Execute all the protocols on every node in their own process.
/// Returns a result with a vector of handles to refer to running processes.
pub fn run(sim: &ast::Simulation) -> Result<Vec<RunHandle>, ProtocolError> {
    let mut processes = vec![];
    let sim_cgroup = simulation_cgroup();
    let mut assignments = CpuAssignment::new();
    for (node_name, node) in &sim.nodes {
        let requested_cycles = node.resources.cpu.requested_cycles();
        let node_assignment = requested_cycles.and_then(|r| assignments.assign(r));
        let protocol_assignment = node_assignment.as_ref().map(|a| {
            a.clone()
                .split_into(node.resources.cpu.cores.map(NonZeroU64::get).unwrap_or(1))
        });
        let root_cgroup = node_cgroup(&sim_cgroup, node_name, node_assignment);
        for (protocol_name, protocol) in &node.protocols {
            let cgroup = protocol_cgroup(&root_cgroup, protocol_name, protocol_assignment.as_ref());
            let mut cgroup_file = OpenOptions::new()
                .write(true)
                .open(cgroup.join("cgroup.procs"))
                .unwrap();
            let process = protocol_assignment
                .as_ref()
                .map_or(
                    Command::new(protocol.runner.cmd.as_str())
                        .current_dir(protocol.root.as_path())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .stdin(Stdio::null())
                        .args(protocol.runner.args.as_slice())
                        .spawn(),
                    |a| {
                        a.start(
                            protocol.runner.cmd.as_str(),
                            protocol.root.as_path(),
                            protocol.runner.args.as_slice(),
                        )
                    },
                )
                .expect("Failed to execute process");
            let _ = cgroup_file
                .write(process.id().to_string().as_bytes())
                .unwrap();

            processes.push(RunHandle {
                node: node_name.clone(),
                protocol: protocol_name.clone(),
                process,
            });
        }
    }

    Ok(processes)
}
