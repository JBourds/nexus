use std::collections::{BTreeMap, HashMap};

use config::ast::{self, Resources};
use cpuutils::cpuset::CpuSet;

/// Builder struct which tracks the requested resources for each
/// node and the PIDs of protocols running on that node.
#[derive(Debug, Default)]
pub struct AffinityBuilder {
    nodes: HashMap<ast::NodeHandle, (Resources, Vec<u32>)>,
}

impl AffinityBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(
        &mut self,
        name: &ast::NodeHandle,
        resources: &Resources,
    ) -> Option<(Resources, Vec<u32>)> {
        if resources.has_cpu_limit() {
            self.nodes
                .insert(name.to_string(), (resources.clone(), Vec::new()))
        } else {
            None
        }
    }

    pub fn add_protocol(&mut self, name: &ast::NodeHandle, pid: u32) {
        if let Some((_, pids)) = self.nodes.get_mut(name) {
            pids.push(pid);
        }
    }

    pub fn build(self) -> Affinity {
        let mut cpuset = CpuSet::default();
        cpuset
            .get_current_affinity()
            .expect("couldn't get parent process CPU affinity");
        let mut affinity = Affinity::new(&cpuset.enabled_ids());
        cpuset.clear();
        for (_, (resources, pids)) in self.nodes {
            // assign all protocols to the same CPU
            let cpu_id = affinity
                .assign_node(&resources)
                .expect("must have at least one core to run on");
            cpuset.enable_cpu(cpu_id).expect("couldn't enable CPU");
            for pid in pids {
                cpuset
                    .set_affinity(pid)
                    .expect("error setting CPU affinity");
            }
            cpuset.disable_cpu(cpu_id).expect("couldn't enable CPU");
        }
        affinity
    }
}

/// Assigns a PID to specific CPU core(s).
/// Tries to evenly distribute assignments across all available cores.
pub struct Affinity {
    /// amount of frequency assigned to each CPU
    assignments: BTreeMap<usize, u64>,
}

impl Affinity {
    pub fn new(available_cores: &[usize]) -> Self {
        Self {
            assignments: available_cores.iter().map(|&id| (id, 0)).collect(),
        }
    }

    /// assign the requested resources to the returned CPU core(s)
    pub fn assign_node(&mut self, resources: &config::ast::Resources) -> Option<usize> {
        if let Some(required_cycles) = resources.cpu.requested_cycles()
            && !self.assignments.is_empty()
        {
            // look for CPU with most resources free.
            let mut best_cpu = 0;
            let mut min_used = u64::MAX;
            for (&id, &used) in self.assignments.iter() {
                if used < min_used {
                    best_cpu = id;
                    min_used = used;
                }
            }
            if let Some(used) = self.assignments.get_mut(&best_cpu) {
                *used += required_cycles;
            }

            Some(best_cpu)
        } else {
            None
        }
    }
}

/// Assigning relative weights to groups for CPU usage.
/// This is necessary for the failure case where CPUs are all running at the
/// same rate in order to keep relative scheduling the same.
#[derive(Debug, Default)]
pub struct RelativeBuilder {
    labels: Vec<ast::NodeHandle>,
    requirements: Vec<u64>,
}

#[derive(Default)]
pub struct Relative {
    weights: HashMap<ast::NodeHandle, u64>,
}

impl Relative {
    pub fn weights(&self) -> &HashMap<ast::NodeHandle, u64> {
        &self.weights
    }
}

impl RelativeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, name: &ast::NodeHandle, resources: &Resources) {
        if let Some(requirement) = resources.cpu.requested_cycles() {
            self.labels.push(name.to_string());
            self.requirements.push(requirement);
        }
    }

    pub fn build(self, low: u64, high: u64) -> Relative {
        if self.requirements.is_empty() {
            return Relative::default();
        }
        let max = self.requirements.iter().max().copied().unwrap();
        let downscale = high as f64 / max as f64;
        // scale everythin within range
        let weights: Vec<u64> = self
            .requirements
            .iter()
            .map(|&v| {
                // downscale
                let scaled = (v as f64 * downscale) as u64;
                // clamp bottom range
                if scaled < low { low } else { scaled }
            })
            .collect();
        Relative {
            weights: self.labels.into_iter().zip(weights).collect(),
        }
    }
}

/// Assigns absolute bandwidths based on CPU frequency.
pub struct Bandwidth {}
