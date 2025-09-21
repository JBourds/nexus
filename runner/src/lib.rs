use config::ast::{self, NodeProtocol};
use std::{
    fmt::Display,
    fs::OpenOptions,
    io::{self, Write},
    num::NonZeroU64,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::FromStr,
};
mod assignment;
pub mod cgroups;
pub mod errors;
use errors::*;

use crate::{
    assignment::{Assignment, CpuAssignment},
    cgroups::{node_cgroup, protocol_cgroup, simulation_cgroup},
};

const BASH: &str = "bash";
const ECHO: &str = "echo";
const TASKSET: &str = "taskset";

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

/// Ensures two things:
///     1. Wrapper shell command gets process ID into the correct cgroup before
///     starting to execute the actual program.
///     2. Protocol gets its CPU assignment applied (affinity & resources)
fn run_protocol(
    p: &NodeProtocol,
    assignment: Option<&Assignment>,
    cgroup: &Path,
) -> io::Result<Child> {
    let mut cmd = Command::new(BASH);
    let procs_file = cgroup.join(cgroups::PROCS);
    let mut script = format!("{ECHO} $$ > {} && ", procs_file.display());
    if let Some(a) = assignment {
        script.push_str(&format!("{TASKSET} --cpu-list {} ", a.set.cpu_list()));
    }
    script.push_str(&format!("{} {}", p.runner.cmd, p.runner.args.join(" ")));
    cmd.current_dir(&p.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .arg("-c")
        .arg(script);
    cmd.spawn()
}

/// Execute all the protocols on every node in their own process.
/// Returns a result with a vector of handles to refer to running processes.
pub fn run(sim: &ast::Simulation) -> Result<(PathBuf, Vec<RunHandle>), ProtocolError> {
    let mut processes = vec![];
    let (sim_cgroup, nodes_cgroup) = simulation_cgroup();
    let mut assignments = CpuAssignment::new();
    for (node_name, node) in &sim.nodes {
        let requested_cycles = node.resources.cpu.requested_cycles();
        let node_assignment = requested_cycles.and_then(|r| assignments.assign(r));
        let protocol_assignment = node_assignment.as_ref().map(|a| {
            a.clone()
                .split_into(node.resources.cpu.cores.map(NonZeroU64::get).unwrap_or(1))
        });
        let root_cgroup = node_cgroup(&nodes_cgroup, node_name, node_assignment);
        for (protocol_name, protocol) in &node.protocols {
            let cgroup = protocol_cgroup(&root_cgroup, protocol_name, protocol_assignment.as_ref());
            let process = run_protocol(protocol, protocol_assignment.as_ref(), &cgroup)
                .expect("Failed to execute process");
            cgroups::move_process(&cgroup, process.id());

            processes.push(RunHandle {
                node: node_name.clone(),
                protocol: protocol_name.clone(),
                process,
            });
        }
    }

    Ok((sim_cgroup, processes))
}
