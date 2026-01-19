use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Child,
};

use config::ast::{self, NodeProtocol, Resources};

use crate::{
    assignment::{Bandwidth, Relative},
    run_protocol,
};

pub const NODES_LIMITED: &str = "nodes_limited";
pub const NODES_UNLIMITED: &str = "nodes_unlimited";
pub const KERNEL: &str = "kernel";
pub const PROCS: &str = "cgroup.procs";
pub const FREEZE: &str = "cgroup.freeze";
pub const SUBTREE: &str = "cgroup.subtree_control";
pub const UCLAMP_MIN: &str = "cpu.uclamp.min";
pub const UCLAMP_MAX: &str = "cpu.uclamp.max";
pub const CPU_MAX: &str = "cpu.max";
pub const CPU_MAX_SCALAR_DIFFERENCE: u64 = 1000;
pub const CPU_BANDWIDTH_MIN: u64 = 1_000;
// True max is much larger but that's not a case we would ever
// run into. This is already way larger than what is needed.
pub const CPU_BANDWIDTH_MAX: u64 = 1_000_000_000;
pub const CPU_PERIOD_MIN: u64 = 1_000;
pub const CPU_PERIOD_MAX: u64 = 1_000_000;
pub const CPU_WEIGHT: &str = "cpu.weight";
pub const CPU_WEIGHT_MIN: u64 = 1;
pub const CPU_WEIGHT_MAX: u64 = 10_000;
const SUBTREE_SUBSYSTEMS: &str = "+cpu +memory";

#[derive(Debug)]
pub struct CgroupController {
    /// root cgroup path
    pub root: PathBuf,
    /// cgroup with nodes that have limited resources
    pub nodes_limited: NodeBucket,
    /// cgroup with nodes that have no specified resource limit
    pub nodes_unlimited: NodeBucket,
}

#[derive(Clone, Debug)]
pub struct NodeHandle {
    has_limited_resources: bool,
    key: String,
}

#[derive(Debug)]
pub struct ProtocolHandle {
    /// Uniquely identify node within cgroup hierarchy
    node_handle: NodeHandle,
    /// Uniquely identify this protocol in cgroup hierarchy
    index: usize,
    /// Name of the node. Unique identifer within the simulation.
    pub node: ast::NodeHandle,
    /// Name of the protocol. Unique identifier for a process within a node.
    pub protocol: ast::ProtocolHandle,
    /// Handle for the executing process.
    pub process: Child,
}

#[derive(Debug)]
pub struct ProtocolCgroup {
    path: PathBuf,
}

#[derive(Debug)]
pub struct NodeCgroup {
    path: PathBuf,
    resources: Resources,
    protocols: Vec<ProtocolCgroup>,
    uclamp_min: f64,
    uclamp_max: f64,
}

#[derive(Debug)]
pub struct NodeBucket {
    root: PathBuf,
    nodes: HashMap<ast::NodeHandle, NodeCgroup>,
}

impl NodeCgroup {
    pub fn add(&mut self, path: PathBuf) {
        self.protocols.push(ProtocolCgroup { path });
    }
}

impl NodeBucket {
    fn new(root: PathBuf) -> Self {
        fs::create_dir(&root).unwrap();
        enable_subtree_control(&root);
        Self {
            root,
            nodes: HashMap::new(),
        }
    }
}

impl Default for CgroupController {
    fn default() -> Self {
        Self::new()
    }
}

impl CgroupController {
    pub fn new() -> Self {
        let pid = std::process::id();
        let root = make_root(pid);

        let kernel_cgroup_path = root.join(KERNEL);
        fs::create_dir(&kernel_cgroup_path).unwrap();
        move_process(&kernel_cgroup_path, pid);
        enable_subtree_control(&root);

        let nodes_unlimited = NodeBucket::new(root.join(NODES_UNLIMITED));
        let nodes_limited = NodeBucket::new(root.join(NODES_LIMITED));

        let mut obj = Self {
            root,
            nodes_limited,
            nodes_unlimited,
        };
        obj.freeze_nodes();
        obj
    }

    /// Don't let any node process start until the FUSE fs has been setup
    pub fn freeze_nodes(&mut self) {
        freeze(&self.nodes_unlimited.root, true);
        freeze(&self.nodes_limited.root, true);
    }

    /// Don't let any node process start until the FUSE fs has been setup
    pub fn unfreeze_nodes(&mut self) {
        freeze(&self.nodes_unlimited.root, false);
        freeze(&self.nodes_limited.root, false);
    }

    pub fn add_node(&mut self, name: &str, resources: Resources) -> NodeHandle {
        let has_limited_resources = resources.has_cpu_limit();
        let parent = if has_limited_resources {
            &mut self.nodes_limited
        } else {
            &mut self.nodes_unlimited
        };
        let path = parent.root.join(name);
        fs::create_dir(&path).expect("couldn't create cgroup path when adding node");
        let handle = NodeHandle {
            has_limited_resources,
            key: name.to_string(),
        };
        parent.nodes.insert(
            name.to_string(),
            NodeCgroup {
                path,
                resources,
                protocols: Vec::new(),
                uclamp_min: 0.0,
                uclamp_max: 100.0,
            },
        );
        handle
    }

    pub fn add_protocol(
        &mut self,
        name: &str,
        protocol: &NodeProtocol,
        handle: &NodeHandle,
    ) -> ProtocolHandle {
        let node = self
            .get_node(handle)
            .expect("couldn't find node from handle.");
        let path = node.path.join(name);
        fs::create_dir(&path).expect("couldn't create cgroup path when adding protocol");
        let process = run_protocol(protocol, &path).expect("couldn't execute process");

        move_process(&path, process.id());
        let handle = ProtocolHandle {
            node_handle: handle.clone(),
            index: node.protocols.len(),
            node: handle.key.clone(),
            protocol: name.to_string(),
            process,
        };
        node.add(path);

        handle
    }

    pub fn assign_cpu_weights(&mut self, relative_assignments: &Relative) {
        for (name, weight) in relative_assignments.weights() {
            let path = self.nodes_limited.root.join(name).join(CPU_WEIGHT);
            let mut f = OpenOptions::new().write(true).open(path).unwrap();
            let _ = f
                .write(weight.to_string().as_bytes())
                .expect("unable to write cpu weight to cpu.weight file");
        }
    }

    pub fn assign_cpu_bandwidths(&mut self, bandwidth_assignments: &Bandwidth) {
        for (name, (bandwidth, period)) in bandwidth_assignments.assignments() {
            if let Some(cgroup) = self.nodes_limited.nodes.get_mut(name) {
                let cpu_max_path = cgroup.path.join(CPU_MAX);
                let mut f = OpenOptions::new().write(true).open(cpu_max_path).unwrap();
                let _ = f
                    .write(format!("{bandwidth} {period}").as_bytes())
                    .expect("unable to write cpu weight to cpu.weight file");
                // increase uclamp minimum to hint to scheduler current policy is
                // not keeping up
                if bandwidth > period {
                    cgroup.uclamp_min = (cgroup.uclamp_min + 5.0).clamp(0.0, 100.0);
                    let s = format!("{:.2}", cgroup.uclamp_min);
                    uclamp_min(&cgroup.path, s.as_bytes());
                }
            }
        }
    }

    fn get_node(&mut self, handle: &NodeHandle) -> Option<&mut NodeCgroup> {
        if handle.has_limited_resources {
            self.nodes_limited.nodes.get_mut(&handle.key)
        } else {
            self.nodes_unlimited.nodes.get_mut(&handle.key)
        }
    }
}

fn uclamp_min(cgroup: &Path, bytes: &[u8]) {
    let path = cgroup.join(UCLAMP_MIN);
    let mut f = OpenOptions::new().write(true).open(path).unwrap();
    let _ = f
        .write(bytes)
        .expect("unable to write minimum usage to cpu.uclamp.min");
}

fn make_root(pid: u32) -> PathBuf {
    let parent_cgroup = PathBuf::from(format!("/proc/{pid}/cgroup"));
    let mut buf = String::new();
    let _ = File::open(parent_cgroup).unwrap().read_to_string(&mut buf);

    PathBuf::from(format!(
        "/sys/fs/cgroup{}",
        buf.split(":").last().unwrap().trim_end()
    ))
}

fn freeze(cgroup: &Path, status: bool) {
    let _ = OpenOptions::new()
        .write(true)
        .open(cgroup.join(FREEZE))
        .unwrap()
        .write(if status { "1" } else { "0" }.as_bytes())
        .unwrap();
}

fn enable_subtree_control(cgroup: &Path) {
    let _ = OpenOptions::new()
        .write(true)
        .open(cgroup.join(SUBTREE))
        .unwrap()
        .write(SUBTREE_SUBSYSTEMS.as_bytes())
        .unwrap();
}

fn move_process(cgroup: &Path, pid: u32) {
    let _ = OpenOptions::new()
        .write(true)
        .open(cgroup.join(PROCS))
        .unwrap()
        .write(pid.to_string().as_bytes())
        .unwrap();
}
