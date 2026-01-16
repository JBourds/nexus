use std::{collections::BTreeMap, num::NonZeroU64};

/// Assigns a PID to specific CPU core(s).
/// Tries to evenly distribute assignments across all available cores.
pub struct AffinityAssignment {
    /// amount of frequency assigned to each CPU
    assignments: BTreeMap<usize, u64>,
}

impl AffinityAssignment {
    pub fn new(available_cores: &[usize]) -> Self {
        Self {
            assignments: available_cores.iter().map(|&id| (id, 0)).collect(),
        }
    }

    /// assign the requested resources to the returned CPU core(s)
    pub fn assign_node(&mut self, resources: &config::ast::Resources) -> Option<usize> {
        if let Some(cycles) = resources.cpu.hertz
            && !self.assignments.is_empty()
        {
            let cores = resources.cpu.cores.map(NonZeroU64::get).unwrap_or(1);
            let required_cycles = cores * cycles.get();

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

/// Assigning relative weights to groups.
/// This is necessary for the failure case where CPUs are all running at the
/// same rate in order to keep relative scheduling the same.
pub struct RelativeAssignment {}

impl RelativeAssignment {}

/// Assigns absolute bandwidths based on CPU frequency.
pub struct BandwidthAssignment {}
