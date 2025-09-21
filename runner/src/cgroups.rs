use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::assignment::Assignment;

/// Move the current process out of its automatically assigned systemd cgroup
/// into a new one within the hierarchy to appease the "no internal processes"
/// rule. Creates subhierarchy for node protocols as well.
pub(crate) fn simulation_cgroup() -> PathBuf {
    let pid = std::process::id();
    let parent_cgroup = PathBuf::from(format!("/proc/{pid}/cgroup"));
    let mut buf = String::new();
    let _ = File::open(parent_cgroup).unwrap().read_to_string(&mut buf);
    let cgroup_path = PathBuf::from(format!(
        "/sys/fs/cgroup{}",
        buf.split(":").last().unwrap().trim_end()
    ));

    let kernel_cgroup_path = cgroup_path.join("kernel");
    fs::create_dir(&kernel_cgroup_path).unwrap();

    let _ = OpenOptions::new()
        .write(true)
        .open(kernel_cgroup_path.join("cgroup.procs"))
        .unwrap()
        .write(pid.to_string().as_bytes())
        .unwrap();
    let _ = OpenOptions::new()
        .write(true)
        .open(cgroup_path.join("cgroup.subtree_control"))
        .unwrap()
        .write("+cpu +memory".as_bytes())
        .unwrap();

    cgroup_path
}

pub(crate) fn node_cgroup(parent: &Path, name: &str, assignment: Option<Assignment>) -> PathBuf {
    let new_cgroup = parent.join(name);
    fs::create_dir(&new_cgroup).unwrap();
    let _ = OpenOptions::new()
        .write(true)
        .open(new_cgroup.join("cgroup.subtree_control"))
        .unwrap()
        .write("+cpu +memory".as_bytes())
        .unwrap();
    if let Some(assignment) = assignment {
        let arg = format!("{} {}", assignment.bandwidth, assignment.period);
        // TODO: Fix errors when one of these values is out of bounds
        let _ = OpenOptions::new()
            .write(true)
            .open(new_cgroup.join("cpu.max"))
            .unwrap()
            .write(arg.as_bytes())
            .unwrap();
    }

    new_cgroup
}

pub(crate) fn protocol_cgroup(
    node_cgroup: &Path,
    name: &str,
    assignment: Option<&Assignment>,
) -> PathBuf {
    let new_cgroup = node_cgroup.join(name);
    fs::create_dir(&new_cgroup).unwrap();
    if let Some(assignment) = assignment {
        let _ = OpenOptions::new()
            .write(true)
            .open(new_cgroup.join("cpu.max"))
            .unwrap()
            .write(format!("{} {}", assignment.bandwidth, assignment.period).as_bytes())
            .unwrap();
    }

    new_cgroup
}
