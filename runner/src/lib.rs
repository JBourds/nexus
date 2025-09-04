use config::ast;
use std::{
    fmt::Display,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    str::FromStr,
};
pub mod errors;
use errors::*;

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

/// Move the current process out of its automatically assigned systemd cgroup
/// into a new one within the hierarchy to appease the "no internal processes"
/// rule. Creates subhierarchy for node protocols as well.
fn setup_managed_cgroup() -> PathBuf {
    let pid = std::process::id();
    let parent_cgroup = PathBuf::from(format!("/proc/{pid}/cgroup"));
    let mut buf = String::new();
    let _ = File::open(parent_cgroup).unwrap().read_to_string(&mut buf);
    let cgroup_path = PathBuf::from(format!(
        "/sys/fs/cgroup{}",
        buf.split(":").last().unwrap().trim_end()
    ));

    let kernel_cgroup_path = cgroup_path.join("kernel");
    let node_cgroup_path = kernel_cgroup_path.join("nodes");
    fs::create_dir_all(&node_cgroup_path).unwrap();

    let mut kernel_cgroup_procs = OpenOptions::new()
        .write(true)
        .open(kernel_cgroup_path.join("cgroup.procs"))
        .unwrap();
    let _ = kernel_cgroup_procs
        .write(pid.to_string().as_bytes())
        .unwrap();

    node_cgroup_path
}

/// Execute all the protocols on every node in their own process.
/// Returns a result with a vector of handles to refer to running processes.
pub fn run(sim: &ast::Simulation) -> Result<Vec<RunHandle>, ProtocolError> {
    let mut processes = vec![];
    let node_cgroups = setup_managed_cgroup();
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
