use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::Child,
};

use config::ast::{self, NodeProtocol, Resources};

use crate::{
    ProtocolSummary,
    assignment::{Bandwidth, Relative},
    cgroupfs::{CgroupFs, RealCgroupFs},
    errors::ProtocolError,
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
const MIN_UCLAMP: f64 = 0.0;
const MAX_UCLAMP: f64 = 100.0;
const SUBTREE_SUBSYSTEMS: &str = "+cpu +memory";

#[derive(Debug)]
pub struct CgroupController {
    /// root cgroup path
    pub root: PathBuf,
    /// cgroup with nodes that have limited resources
    pub nodes_limited: NodeBucket,
    /// cgroup with nodes that have no specified resource limit
    pub nodes_unlimited: NodeBucket,
    /// filesystem abstraction for cgroup operations
    fs: Box<dyn CgroupFs>,
}

#[derive(Clone, Debug)]
pub struct NodeHandle {
    has_limited_resources: bool,
    key: String,
}

#[derive(Debug)]
pub struct ProtocolHandle {
    /// Uniquely identify node within cgroup hierarchy
    #[allow(dead_code)]
    node_handle: NodeHandle,
    /// Uniquely identify this protocol in cgroup hierarchy
    #[allow(dead_code)]
    index: usize,
    /// Original AST node to preserve information about how to build/run
    ast: NodeProtocol,
    /// Name of the node. Unique identifer within the simulation.
    pub node: ast::NodeHandle,
    /// Name of the protocol. Unique identifier for a process within a node.
    pub protocol: ast::ProtocolHandle,
    /// Handle for the executing process.
    pub process: Option<Child>,
    /// Path to the cgroup directory for this protocol.
    pub cgroup_path: Option<PathBuf>,
}

impl ProtocolHandle {
    pub fn new(
        node_handle: NodeHandle,
        index: usize,
        ast: NodeProtocol,
        node: ast::NodeHandle,
        protocol: ast::ProtocolHandle,
    ) -> Self {
        ProtocolHandle {
            node_handle,
            index,
            ast,
            node,
            protocol,
            process: None,
            cgroup_path: None,
        }
    }

    /// Return the PID of the running process, if any.
    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().map(Child::id)
    }

    /// Kill the current process and respawn it in its cgroup.
    /// Returns `(old_pid, new_pid)` on success.
    pub fn respawn(&mut self) -> Option<(u32, u32)> {
        let old_pid = self.pid()?;
        let cgroup = self.cgroup_path.as_ref()?;
        // Kill and wait for the old process
        if let Some(ref mut child) = self.process {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.process = None;
        // Spawn a fresh process
        self.run(cgroup.clone()).ok()?;
        let new_pid = self.pid()?;
        Some((old_pid, new_pid))
    }

    pub fn running(&mut self) -> bool {
        if let Some(p) = self.process.as_mut() {
            !matches!(p.try_wait(), Ok(Some(_)))
        } else {
            false
        }
    }

    pub fn kill(&mut self) -> io::Result<()> {
        if let Some(p) = self.process.as_mut() {
            p.kill()
        } else {
            Ok(())
        }
    }

    pub fn finish(mut self) -> Result<Option<ProtocolSummary>, io::Error> {
        match self.process.take() {
            Some(mut p) => {
                p.kill()?;
                let output = p.wait_with_output()?;
                Ok(Some(ProtocolSummary {
                    node: self.node,
                    protocol: self.protocol,
                    output,
                }))
            }
            None => Ok(None),
        }
    }

    pub fn run(&mut self, cgroup: impl AsRef<Path>) -> Result<bool, ProtocolError> {
        if self.process.is_none() {
            let process =
                run_protocol(&self.ast, cgroup.as_ref()).map_err(ProtocolError::UnableToRun)?;
            move_process(&RealCgroupFs, cgroup.as_ref(), process.id());
            self.process = Some(process);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[derive(Debug)]
pub struct ProtocolCgroup {
    #[allow(dead_code)]
    path: PathBuf,
}

#[derive(Debug)]
pub struct NodeCgroup {
    path: PathBuf,
    #[allow(dead_code)]
    resources: Resources,
    protocols: Vec<ProtocolCgroup>,
    uclamp_min: f64,
    bandwidth: u64,
    period: u64,
    // cached result of `bandwidth / period += epsilon * bandwidth / period`
    adjustment_threshold: (f64, f64),
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
    fn new(fs: &dyn CgroupFs, root: PathBuf) -> io::Result<Self> {
        fs.create_dir(&root)?;
        enable_subtree_control(fs, &root);
        Ok(Self {
            root,
            nodes: HashMap::new(),
        })
    }
}

impl CgroupController {
    pub fn new() -> io::Result<Self> {
        Self::with_fs(Box::new(RealCgroupFs))
    }

    pub fn with_fs(fs: Box<dyn CgroupFs>) -> io::Result<Self> {
        let pid = std::process::id();
        let root = make_root(&*fs, pid)?;

        let kernel_cgroup_path = root.join(KERNEL);
        fs.create_dir(&kernel_cgroup_path)?;
        move_process(&*fs, &kernel_cgroup_path, pid);
        enable_subtree_control(&*fs, &root);

        let nodes_unlimited = NodeBucket::new(&*fs, root.join(NODES_UNLIMITED))?;
        let nodes_limited = NodeBucket::new(&*fs, root.join(NODES_LIMITED))?;

        let mut obj = Self {
            root,
            nodes_limited,
            nodes_unlimited,
            fs,
        };
        obj.freeze_nodes();
        Ok(obj)
    }

    /// Don't let any node process start until the FUSE fs has been setup
    pub fn freeze_nodes(&mut self) {
        freeze(&*self.fs, &self.nodes_unlimited.root, true);
        freeze(&*self.fs, &self.nodes_limited.root, true);
    }

    /// Don't let any node process start until the FUSE fs has been setup
    pub fn unfreeze_nodes(&mut self) {
        freeze(&*self.fs, &self.nodes_unlimited.root, false);
        freeze(&*self.fs, &self.nodes_limited.root, false);
    }

    /// Set the frozen state of a single node's cgroup by name.
    pub fn set_node_frozen(&mut self, name: &str, frozen: bool) {
        if let Some(cgroup) = self.nodes_unlimited.nodes.get(name) {
            freeze(&*self.fs, &cgroup.path, frozen);
        } else if let Some(cgroup) = self.nodes_limited.nodes.get(name) {
            freeze(&*self.fs, &cgroup.path, frozen);
        }
    }

    pub fn freeze_node(&mut self, name: &str) {
        self.set_node_frozen(name, true);
    }

    pub fn unfreeze_node(&mut self, name: &str) {
        self.set_node_frozen(name, false);
    }

    /// Unfreeze a node's cgroup, then respawn all its protocols.
    /// Returns the list of `(old_pid, new_pid)` pairs.
    pub fn respawn_node(&mut self, name: &str, handles: &mut [ProtocolHandle]) -> Vec<(u32, u32)> {
        self.unfreeze_node(name);
        handles
            .iter_mut()
            .filter(|h| h.node == name)
            .filter_map(|h| h.respawn())
            .collect()
    }

    pub fn add_node(&mut self, name: &str, resources: Resources) -> io::Result<NodeHandle> {
        let has_limited_resources = resources.has_cpu_limit();
        let parent = if has_limited_resources {
            &mut self.nodes_limited
        } else {
            &mut self.nodes_unlimited
        };
        let path = parent.root.join(name);
        self.fs.create_dir(&path)?;
        enable_subtree_control(&*self.fs, &path);
        uclamp_min(&*self.fs, &path, b"max");
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
                uclamp_min: MAX_UCLAMP,
                bandwidth: CPU_BANDWIDTH_MIN,
                period: CPU_PERIOD_MIN,
                adjustment_threshold: (0.0, 0.0),
            },
        );
        Ok(handle)
    }

    pub fn add_protocol(
        &mut self,
        name: &str,
        protocol: &NodeProtocol,
        handle: &NodeHandle,
    ) -> Result<ProtocolHandle, ProtocolError> {
        // Extract what we need before re-borrowing self
        let (cgroup, protocol_count) = {
            let node = self
                .get_node(handle)
                .ok_or_else(|| ProtocolError::NodeNotFound(handle.key.clone()))?;
            (node.path.join(name), node.protocols.len())
        };
        self.fs.create_dir(&cgroup)?;
        let mut proto_handle = ProtocolHandle::new(
            handle.clone(),
            protocol_count,
            protocol.clone(),
            handle.key.clone(),
            name.to_string(),
        );
        proto_handle.cgroup_path = Some(cgroup.clone());
        proto_handle.run(&cgroup)?;
        // Re-borrow to add the protocol cgroup
        let node = self.get_node(handle).expect("node was just looked up");
        node.add(cgroup);
        Ok(proto_handle)
    }

    pub fn assign_cpu_weights(&mut self, relative_assignments: &Relative) {
        for (name, weight) in relative_assignments.weights() {
            let dir = self.nodes_limited.root.join(name);
            if let Err(e) = self
                .fs
                .write_file(&dir, CPU_WEIGHT, weight.to_string().as_bytes())
            {
                eprintln!("Failed to write {CPU_WEIGHT} for {name}: {e}");
            }
        }
    }

    /// Potentially reassign bandwidth/period.
    /// If this gets reassigned too frequently it prevents cpu.max from actually
    /// applying due to the time it takes for the kernel to get up to speed.
    /// Therefore, we set a somewhat arbitrary epsilon where any change smaller
    /// than it will not lead to an update.
    pub fn assign_cpu_bandwidths(&mut self, bandwidth_assignments: &Bandwidth) {
        const EPSILON: f64 = 0.05;
        for (name, &(bandwidth, period)) in bandwidth_assignments.assignments() {
            if let Some(cgroup) = self.nodes_limited.nodes.get_mut(name) {
                let ratio = bandwidth as f64 / period as f64;
                // ignore small fluctuations
                let (low, high) = cgroup.adjustment_threshold;
                if ratio >= low && ratio <= high {
                    continue;
                }
                cgroup.bandwidth = bandwidth;
                cgroup.period = period;
                cgroup.adjustment_threshold = (ratio - EPSILON, ratio + EPSILON);

                if let Err(e) = self.fs.write_file(
                    &cgroup.path,
                    CPU_MAX,
                    format!("{bandwidth} {period}").as_bytes(),
                ) {
                    eprintln!("Failed to write {CPU_MAX} for {name}: {e}");
                }
                // increase uclamp minimum to hint to scheduler current policy is
                // not keeping up
                if bandwidth > period {
                    cgroup.uclamp_min = (cgroup.uclamp_min + 5.0).clamp(MIN_UCLAMP, MAX_UCLAMP);
                    let s = format!("{:.2}", cgroup.uclamp_min);
                    uclamp_min(&*self.fs, &cgroup.path, s.as_bytes());
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

fn uclamp_min(fs: &dyn CgroupFs, cgroup: &Path, bytes: &[u8]) {
    if let Err(e) = fs.write_file(cgroup, UCLAMP_MIN, bytes) {
        eprintln!("Failed to write {UCLAMP_MIN}: {e}");
    }
}

fn make_root(fs: &dyn CgroupFs, pid: u32) -> io::Result<PathBuf> {
    let parent_cgroup = PathBuf::from(format!("/proc/{pid}/cgroup"));
    let buf = fs.read_to_string(&parent_cgroup)?;

    let suffix = buf.rsplit(':').next().unwrap_or("").trim_end();
    Ok(PathBuf::from(format!("/sys/fs/cgroup{suffix}")))
}

fn freeze(fs: &dyn CgroupFs, cgroup: &Path, status: bool) {
    let data = if status { b"1" as &[u8] } else { b"0" };
    if let Err(e) = fs.write_file(cgroup, FREEZE, data) {
        eprintln!("Failed to write {FREEZE} for {}: {e}", cgroup.display());
    }
}

fn enable_subtree_control(fs: &dyn CgroupFs, cgroup: &Path) {
    if let Err(e) = fs.write_file(cgroup, SUBTREE, SUBTREE_SUBSYSTEMS.as_bytes()) {
        eprintln!(
            "Failed to enable subtree control for {}: {e}",
            cgroup.display()
        );
    }
}

fn move_process(fs: &dyn CgroupFs, cgroup: &Path, pid: u32) {
    if let Err(e) = fs.write_file(cgroup, PROCS, pid.to_string().as_bytes()) {
        eprintln!("Failed to move PID {pid} to {}: {e}", cgroup.display());
    }
}

impl Drop for CgroupController {
    fn drop(&mut self) {
        // Clean up per-simulation cgroup directories. Errors are best-effort
        // since the kernel may still have processes in these groups.
        let _ = self.fs.remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cgroupfs::mock::MockCgroupFs;
    use config::ast::{Cpu, Mem};
    use std::num::NonZeroU64;

    fn make_test_controller(mock: MockCgroupFs) -> CgroupController {
        let pid = std::process::id();
        // Seed the /proc file so make_root works
        mock.seed_file(format!("/proc/{pid}/cgroup"), "0::/test_cgroup\n");
        // Seed the root cgroup dir so child dirs can be created
        mock.seed_dir("/sys/fs/cgroup/test_cgroup");

        CgroupController::with_fs(Box::new(mock)).expect("controller creation should succeed")
    }

    fn limited_resources() -> Resources {
        Resources {
            cpu: Cpu {
                hertz: Some(NonZeroU64::new(1_000_000).unwrap()),
                ..Cpu::default()
            },
            mem: Mem::default(),
        }
    }

    #[test]
    fn new_creates_expected_directory_structure() {
        let mock = MockCgroupFs::new();
        let ctrl = make_test_controller(mock);

        assert_eq!(ctrl.root, PathBuf::from("/sys/fs/cgroup/test_cgroup"));
    }

    #[test]
    fn add_node_limited_creates_dir_and_writes_uclamp() {
        let mock = MockCgroupFs::new();
        let mut ctrl = make_test_controller(mock);

        let handle = ctrl.add_node("sensor_node", limited_resources()).unwrap();
        assert!(handle.has_limited_resources);
        assert!(ctrl.nodes_limited.nodes.contains_key("sensor_node"));
    }

    #[test]
    fn add_node_unlimited_goes_to_correct_bucket() {
        let mock = MockCgroupFs::new();
        let mut ctrl = make_test_controller(mock);

        let handle = ctrl.add_node("relay_node", Resources::default()).unwrap();
        assert!(!handle.has_limited_resources);
        assert!(ctrl.nodes_unlimited.nodes.contains_key("relay_node"));
        assert!(!ctrl.nodes_limited.nodes.contains_key("relay_node"));
    }

    #[test]
    fn freeze_writes_correct_values() {
        let mock = MockCgroupFs::new();
        let mut ctrl = make_test_controller(mock);

        ctrl.add_node("n1", Resources::default()).unwrap();
        ctrl.freeze_node("n1");
        ctrl.unfreeze_node("n1");
    }

    #[test]
    fn make_root_parses_cgroup_file() {
        let mock = MockCgroupFs::new();
        mock.seed_file("/proc/1234/cgroup", "0::/user.slice/session-1.scope\n");
        let root = make_root(&mock, 1234).unwrap();
        assert_eq!(
            root,
            PathBuf::from("/sys/fs/cgroup/user.slice/session-1.scope")
        );
    }

    #[test]
    fn make_root_handles_empty_suffix() {
        let mock = MockCgroupFs::new();
        mock.seed_file("/proc/5678/cgroup", "0::\n");
        let root = make_root(&mock, 5678).unwrap();
        assert_eq!(root, PathBuf::from("/sys/fs/cgroup"));
    }

    #[test]
    fn drop_cleans_up() {
        let mock = MockCgroupFs::new();
        let ctrl = make_test_controller(mock);
        drop(ctrl);
        // Reaching here without panics confirms the cleanup path works.
    }
}
