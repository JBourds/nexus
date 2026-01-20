use config::ast::{self, Cmd, NodeProtocol};
use cpuutils::cpufreq::get_cpu_info;
use std::{
    io,
    path::Path,
    process::{Child, Command, Output, Stdio},
};
pub mod assignment;
pub mod cgroups;
pub mod cli;
pub mod errors;
use errors::*;

use crate::assignment::{Affinity, AffinityBuilder, Bandwidth, Relative, RelativeBuilder};
pub use crate::cgroups::*;

const BASH: &str = "bash";
const ECHO: &str = "echo";

#[derive(Debug)]
pub struct RunController {
    pub cgroups: CgroupController,
    pub affinity: Affinity,
    pub weights: Relative,
    pub bandwidth: Bandwidth,
    pub handles: Vec<ProtocolHandle>,
}

#[derive(Debug)]
pub struct ProtocolSummary {
    pub node: ast::NodeHandle,
    pub protocol: ast::ProtocolHandle,
    pub output: Output,
}

struct BuildCtx<'a> {
    node: &'a str,
    protocol: &'a str,
    cmd: &'a Cmd,
    root: &'a Path,
    handle: Child,
}

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

fn build_protocol<'a>(
    node_name: &'a ast::NodeHandle,
    protocol_name: &'a ast::ProtocolHandle,
    p: &'a NodeProtocol,
) -> io::Result<BuildCtx<'a>> {
    let cmd = p.build.cmd.as_str();
    let args = p.build.args.as_slice();
    let root = p.root.as_path();
    Command::new(cmd)
        .current_dir(root)
        .args(args)
        .stdout(Stdio::null())
        .spawn()
        .map(|handle| BuildCtx {
            node: node_name,
            protocol: protocol_name,
            cmd: &p.build,
            root,
            handle,
        })
}

fn check_builds<'a>(contexts: Vec<BuildCtx<'a>>) -> Vec<errors::Error> {
    let mut errors = vec![];
    for mut ctx in contexts {
        let exit_code = ctx.handle.wait().expect("cannot wait for process");
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
    errors
}

/// Walk the simulation AST and build each program.
pub fn build(sim: &ast::Simulation) -> Result<(), errors::ProtocolError> {
    println!("Building programs");
    let contexts = sim
        .nodes
        .iter()
        .flat_map(|(node_name, node)| {
            node.protocols
                .iter()
                .filter(|(_, p)| !p.build.cmd.is_empty())
                .map(|(protocol_name, p)| build_protocol(node_name, protocol_name, p))
        })
        .collect::<io::Result<Vec<_>>>()
        .map_err(errors::ProtocolError::UnableToRun)?;
    let errors = check_builds(contexts);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors::ProtocolError::BuildErrors(errors))
    }
}

/// Execute all the protocols on every node in their own process.
/// Returns a result with a vector of handles to refer to running processes.
pub fn run(sim: &ast::Simulation) -> Result<RunController, ProtocolError> {
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
            affinity_builder.add_protocol(
                node_name,
                protocol_handle
                    .process
                    .as_ref()
                    .expect("there should always be a process here")
                    .id(),
            );
            handles.push(protocol_handle);
        }
    }
    let affinity_assignments = affinity_builder.build();
    let relative_assignments = relative_builder.build(CPU_WEIGHT_MIN, CPU_WEIGHT_MAX);
    cgroup_controller.assign_cpu_weights(&relative_assignments);
    let cpuinfo = get_cpu_info(&affinity_assignments.cpuset);
    let bandwidth_assignments = Bandwidth::new(&affinity_assignments, &cpuinfo);
    cgroup_controller.assign_cpu_bandwidths(&bandwidth_assignments);

    Ok(RunController {
        cgroups: cgroup_controller,
        affinity: affinity_assignments,
        weights: relative_assignments,
        bandwidth: bandwidth_assignments,
        handles,
    })
}
