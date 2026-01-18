use config::ast::{self, Cmd, NodeProtocol};
use std::{
    fmt::Display,
    io,
    path::Path,
    process::{Child, Command, Stdio},
    str::FromStr,
};
pub mod assignment;
pub mod cgroups;
pub mod errors;
use errors::*;

use crate::assignment::{AffinityBuilder, RelativeBuilder};
pub use crate::cgroups::*;

const BASH: &str = "bash";
const ECHO: &str = "echo";

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
fn run_protocol(p: &NodeProtocol, cgroup: &Path) -> io::Result<Child> {
    let mut cmd = Command::new(BASH);
    let procs_file = cgroup.join(cgroups::PROCS);
    let mut script = format!("{ECHO} $$ > {} && ", procs_file.display());
    script.push_str(&format!("{} {}", p.runner.cmd, p.runner.args.join(" ")));
    cmd.current_dir(&p.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .arg("-c")
        .arg(script);
    cmd.spawn()
}

pub fn build(sim: &ast::Simulation) -> Result<(), errors::ProtocolError> {
    struct Ctx<'a> {
        node: &'a str,
        protocol: &'a str,
        cmd: &'a Cmd,
        root: &'a Path,
        handle: Child,
    }
    println!("Building programs");
    let handles = sim
        .nodes
        .iter()
        .flat_map(|(node_name, node)| {
            node.protocols
                .iter()
                .filter(|(_, p)| !p.build.cmd.is_empty())
                .map(|(protocol_name, p)| {
                    let cmd = p.build.cmd.as_str();
                    let args = p.build.args.as_slice();
                    let root = p.root.as_path();
                    Command::new(cmd)
                        .current_dir(root)
                        .args(args)
                        .stdout(Stdio::null())
                        .spawn()
                        .map(|handle| Ctx {
                            node: node_name,
                            protocol: protocol_name,
                            cmd: &p.build,
                            root,
                            handle,
                        })
                })
        })
        .collect::<io::Result<Vec<_>>>()
        .map_err(errors::ProtocolError::UnableToRun)?;
    let mut errors = vec![];
    for mut ctx in handles {
        let exit_code = ctx
            .handle
            .wait()
            .map_err(errors::ProtocolError::UnableToRun)?;
        if !exit_code.success() {
            // First fialure
            if errors.is_empty() {
                eprintln!("\nError building programs:\n");
            }
            errors.push(errors::Error::new(
                ctx.node.to_string(),
                ctx.protocol.to_string(),
                ctx.root.to_path_buf(),
                format!("Command: {} ({exit_code})", ctx.cmd),
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors::ProtocolError::BuildErrors(errors))
    }
}

/// Execute all the protocols on every node in their own process.
/// Returns a result with a vector of handles to refer to running processes.
pub fn run(
    sim: &ast::Simulation,
) -> Result<(CgroupController, Vec<ProtocolHandle>), ProtocolError> {
    let mut cgroup_controller = CgroupController::new();
    let mut handles = Vec::new();
    let mut affinity_builder = AffinityBuilder::new();
    let mut relative_builder = RelativeBuilder::new();
    for (node_name, node) in &sim.nodes {
        affinity_builder.add_node(node_name, &node.resources);
        relative_builder.add_node(node_name, &node.resources);
        let handle = cgroup_controller.add_node(node_name, node.resources.clone());
        for (protocol_name, protocol) in &node.protocols {
            let protocol_handle = cgroup_controller.add_protocol(protocol_name, protocol, &handle);
            affinity_builder.add_protocol(node_name, protocol_handle.process.id());
            handles.push(protocol_handle);
        }
    }
    let _ = affinity_builder.build();
    let relative_assignment = relative_builder.build(CPU_WEIGHT_MIN, CPU_WEIGHT_MAX);
    cgroup_controller.make_cpu_weight_assignments(&relative_assignment);

    Ok((cgroup_controller, handles))
}
