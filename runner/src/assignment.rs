use std::collections::{BTreeMap, HashMap};

use config::ast::{self, Resources};
use cpuutils::{
    cpufreq::{CoreInfo, CpuInfo},
    cpuset::CpuSet,
};

use crate::{CPU_BANDWIDTH_MIN, CPU_MAX_SCALAR_DIFFERENCE, CPU_PERIOD_MIN};

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
        let mut affinity = Affinity::new(cpuset);
        let mut enabler = CpuSet::default();
        for (name, (resources, pids)) in self.nodes {
            // assign all protocols to the same CPU
            let cpu_id = affinity
                .assign_node(&name, &resources)
                .expect("must have at least one core to run on");
            enabler.enable_cpu(cpu_id).expect("couldn't enable CPU");
            for pid in pids {
                enabler
                    .set_affinity(pid)
                    .expect("error setting CPU affinity");
            }
            enabler.disable_cpu(cpu_id).expect("couldn't enable CPU");
        }
        affinity
    }
}

/// Assigns a PID to specific CPU core(s).
/// Tries to evenly distribute assignments across all available cores.
#[derive(Debug)]
pub struct Affinity {
    /// amount of frequency assigned to each CPU
    usage: BTreeMap<usize, u64>,
    /// assignment of each node to a CPU core and required number of cycles
    pub assignments: HashMap<ast::NodeHandle, (usize, u64)>,
    /// cached cpuset for the parent process
    pub cpuset: CpuSet,
}

impl Affinity {
    pub fn new(cpuset: CpuSet) -> Self {
        let usage = cpuset.enabled_ids().iter().map(|&id| (id, 0)).collect();
        Self {
            usage,
            assignments: HashMap::default(),
            cpuset,
        }
    }

    /// assign the requested resources to the returned CPU core(s)
    pub fn assign_node(
        &mut self,
        name: &ast::NodeHandle,
        resources: &config::ast::Resources,
    ) -> Option<usize> {
        if let Some(required_cycles) = resources.cpu.requested_cycles() {
            // look for CPU with most resources free.
            let mut best_cpu = 0;
            let mut min_used = u64::MAX;
            for (&id, &used) in self.usage.iter() {
                if used < min_used {
                    best_cpu = id;
                    min_used = used;
                }
            }
            if let Some(used) = self.usage.get_mut(&best_cpu) {
                *used += required_cycles;
            }
            self.assignments
                .insert(name.clone(), (best_cpu, required_cycles));

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

#[derive(Debug, Default)]
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
#[derive(Debug)]
pub struct Bandwidth {
    /// mapping from node handle to:
    ///     - bandwidth (numerator)
    ///     - period (denominator)
    assignments: HashMap<ast::NodeHandle, (u64, u64)>,
}

impl Bandwidth {
    pub fn new(affinity: &Affinity, cpuinfo: &CpuInfo) -> Self {
        let mut assignments = HashMap::new();
        Self::refresh_inner(&mut assignments, affinity, cpuinfo);
        Self { assignments }
    }

    pub fn refresh(&mut self, affinity: &Affinity, cpuinfo: &CpuInfo) {
        Self::refresh_inner(&mut self.assignments, affinity, cpuinfo);
    }

    pub fn assignments(&self) -> &HashMap<ast::NodeHandle, (u64, u64)> {
        &self.assignments
    }

    fn refresh_inner(
        assignments: &mut HashMap<ast::NodeHandle, (u64, u64)>,
        affinity: &Affinity,
        cpuinfo: &CpuInfo,
    ) {
        for (node, (core, required_cycles)) in affinity.assignments.iter() {
            // NOTE: using `max_frequency` here seems to give a more consistent
            // result than getting the current frequency with `frequency` since
            // we set `cpu.uclamp.min` to max causing the CPU to run at max
            // capacity once it actually runs the task.
            if let Some(current_frequency) = cpuinfo.cores.get(core).map(CoreInfo::max_frequency) {
                let ratio = *required_cycles as f64 / current_frequency as f64;
                // try to minimize the period as much as possible to ensure
                // scheduling requests are honored on tighter timeframes
                let (bandwidth, period) = if ratio >= 1.0 {
                    let period = CPU_PERIOD_MIN;
                    let bandwidth = std::cmp::min(
                        (CPU_PERIOD_MIN as f64 * ratio) as u64,
                        CPU_PERIOD_MIN * CPU_MAX_SCALAR_DIFFERENCE,
                    );
                    (bandwidth, period)
                } else {
                    let period = std::cmp::min(
                        (CPU_PERIOD_MIN as f64 / ratio) as u64,
                        CPU_BANDWIDTH_MIN * CPU_MAX_SCALAR_DIFFERENCE,
                    );
                    let bandwidth = CPU_BANDWIDTH_MIN;
                    (bandwidth, period)
                };
                let _ = assignments.insert(node.clone(), (bandwidth, period));
            } else {
                unreachable!("couldn't find CPU core");
            }
        }
    }
}
