use std::collections::HashMap;
use std::io::BufRead;
use std::process::Command;
use sysinfo::{Cpu, CpuRefreshKind, RefreshKind, System};

pub type Frequency = u64;
pub type CpuNum = usize;

#[derive(Debug, Default, Clone)]
pub struct Assignment {
    /// cgroup file: `cpuset.cpus`
    pub set: Cpuset,
    /// cgroup file: `cpu.max`
    pub bandwidth: u64,
    pub period: u64,
}

impl Assignment {
    const PERIOD: u64 = 10_000_000;
}

#[derive(Debug, Default, Clone)]
pub struct Cpuset(String);
impl Cpuset {
    fn from_cpus(cpus: &[usize]) -> Cpuset {
        let mut s = String::new();
        for cpu in cpus {
            s.push_str(&cpu.to_string());
            s.push(',');
        }
        if !s.is_empty() {
            s.pop().unwrap();
        }
        Cpuset(s)
    }
}

impl std::fmt::Display for Cpuset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct CpuAssignment {
    pub cpusets: HashMap<Frequency, Vec<CpuNum>>,
    pub available: HashMap<Frequency, Frequency>,
}

impl CpuAssignment {
    pub fn new() -> Self {
        let cpusets = get_cpusets();
        let available = cpusets
            .iter()
            .map(|(key, cpus)| (*key, *key * cpus.len() as u64))
            .collect();
        Self { cpusets, available }
    }

    /// Given a required number of clock cycles, assign it to a set of
    pub fn assign(&mut self, required: Frequency) -> Option<Assignment> {
        if let Some((key, available)) = self
            .available
            .iter_mut()
            .filter(|(_, available)| **available >= required)
            .max_by_key(|(_, available)| **available)
        {
            let ratio = required as f64 / *available as f64;
            let bandwidth = (ratio * Assignment::PERIOD as f64) as u64;
            *available -= required;

            Some(Assignment {
                set: Cpuset::from_cpus(&self.cpusets[key]),
                bandwidth,
                period: Assignment::PERIOD,
            })
        } else {
            None
        }
    }
}

/// Try this two ways:
///     1, Directly with `lscpu` to query max megahertz.
///     2, If the previous way didn`t work, assume there is no frequency
///     scaling and that we can directly query current frequency.
fn get_cpusets() -> HashMap<Frequency, Vec<CpuNum>> {
    let mut cpusets: HashMap<Frequency, Vec<CpuNum>> = HashMap::new();
    if let Ok(output) = Command::new("lscpu").arg("-e=CPU,MAXMHZ").output() {
        for line in output.stdout.as_slice().lines().skip(1) {
            let line = line.expect("Error reading line from lscpu");
            let split: Vec<_> = line.split_whitespace().collect();
            let [cpu, mhz] = split[..2] else {
                panic!("Couldn't parse CPU number and clock rate from `lscpu` output.");
            };
            let cpu = cpu
                .parse::<usize>()
                .expect("Failed to parse CPU number from `lscpu` output.");
            let mega = f64::from(1u32 << 20);
            let frequency = (mhz
                .parse::<f64>()
                .expect("Failed to parse valid clock rate from `lscpu` output")
                * mega)
                .round() as Frequency;
            cpusets.entry(frequency).or_default().push(cpu);
        }
    } else {
        for (cpu, frequency) in System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
        )
        .cpus()
        .iter()
        .map(Cpu::frequency)
        .enumerate()
        {
            cpusets.entry(frequency).or_default().push(cpu);
        }
    }
    cpusets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignments() {
        const GHZ: u64 = 1 << 30;
        // Make sure this is deterministic and deson't rely on hash map order
        for _ in 0..100 {
            let cpusets: HashMap<_, _> = [
                (5 * GHZ, vec![0, 1]),
                (4 * GHZ, vec![2, 3]),
                (3 * GHZ, vec![4, 5]),
            ]
            .into_iter()
            .collect();
            let available: HashMap<_, _> = cpusets
                .iter()
                .map(|(freq, cpus)| (*freq, *freq * cpus.len() as u64))
                .collect();
            let mut assignments = CpuAssignment { cpusets, available };
            let test = [
                // Allocation goes to the greatest available
                (1 * GHZ, "0,1".to_string()),
                // Edge case - make sure we still get allocation
                (1 * GHZ - 1, "0,1".to_string()),
                (3, "0,1".to_string()),
                // Next allocation goes to next highest cpuset
                (1, "2,3".to_string()),
                (2 * GHZ, "2,3".to_string()),
                // This cpuset is back to largest, exhaust it
                (8 * GHZ - 2, "0,1".to_string()),
                // Slowest CPUs now have the most availability, get it to have one
                // less than 2,3
                (2, "4,5".to_string()),
                // Exhaust remaining capacities
                (6 * GHZ - 1, "2,3".to_string()),
                (6 * GHZ - 2, "4,5".to_string()),
                // Make sure no set is obtained
                (1, "".to_string()),
            ];
            for (input, expected) in test {
                if expected.is_empty() {
                    assert!(
                        assignments.assign(input).is_none(),
                        "Expected to fail creating an assignment for {input}"
                    );
                } else {
                    let assignment = assignments.assign(input).unwrap();
                    assert_eq!(
                        expected, assignment.set.0,
                        "Made assignment for {input}Hz and expected {expected} but got {}",
                        assignment.set
                    );
                }
            }
        }
    }
}
