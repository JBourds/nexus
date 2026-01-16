use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::{collections::BTreeMap, fs::File, path::Path};

use crate::cpuset::CpuSet;

const SYSFS_CPUS: &str = "/sys/devices/system/cpu";
const PROCFS_CPUINFO: &str = "/proc/cpuinfo";
const MEGA: f64 = 1_000_000.0;

/// CPUs could be doing frequency scaling or could be static.
/// This enum reflects the best effort reporting and it is up to application
/// code to use this appropriately.
#[derive(Debug)]
pub enum CpuInfo {
    Scaling {
        min_hz: u64,
        max_hz: u64,
        current_hz: u64,
    },
    Static {
        current_hz: u64,
    },
}

fn read_sysfs_u64(path: impl AsRef<Path>) -> Option<u64> {
    let mut s = String::new();
    if File::open(path)
        .and_then(|mut f| f.read_to_string(&mut s))
        .is_ok()
    {
        let s = s.split_whitespace().next().unwrap();
        Some(
            s.parse::<u64>()
                .expect("if the file exists the result will always be valid"),
        )
    } else {
        None
    }
}

impl CpuInfo {
    fn scaling(id: usize) -> Option<Self> {
        let base = Path::new(SYSFS_CPUS)
            .join(format!("cpu{id}"))
            .join("cpufreq");
        let min_hz = read_sysfs_u64(base.join("cpuinfo_min_freq"))?;
        let max_hz = read_sysfs_u64(base.join("cpuinfo_max_freq"))?;
        let current_hz = read_sysfs_u64(base.join("scaling_cur_freq"))?;
        Some(Self::Scaling {
            min_hz,
            max_hz,
            current_hz,
        })
    }
}

pub fn get_cpu_info(cpuset: &CpuSet) -> BTreeMap<usize, CpuInfo> {
    let ids: HashSet<usize> = cpuset.enabled_ids().into_iter().collect();
    parse_scaling_cpuinfo(&ids)
        .or_else(|| parse_static_cpuinfo(&ids))
        .unwrap_or_default()
}

pub fn parse_scaling_cpuinfo(cpuset: &HashSet<usize>) -> Option<BTreeMap<usize, CpuInfo>> {
    let mut cpu_frequencies = BTreeMap::new();
    for &id in cpuset.iter() {
        let info = CpuInfo::scaling(id)?;
        cpu_frequencies.insert(id, info);
    }
    Some(cpu_frequencies)
}

pub fn parse_static_cpuinfo(cpuset: &HashSet<usize>) -> Option<BTreeMap<usize, CpuInfo>> {
    let mut cpu_frequencies = BTreeMap::new();
    let file = File::open(PROCFS_CPUINFO).expect("couldn't open procfs file");
    let reader = BufReader::new(file);
    let mut current = None;
    for line in reader.lines().map_while(Result::ok) {
        if let Some(id) = current {
            let Some(line) = line.strip_prefix("cpu MHz") else {
                continue;
            };
            let line = line
                .trim_start()
                .strip_prefix(":")
                .expect("/proc/cpuinfo file format is incorrect")
                .trim_start();
            let frequency_mhz = line.parse::<f64>().expect("couldn't parse frequency");
            let frequency_hz = (frequency_mhz * MEGA) as u64;
            cpu_frequencies.insert(
                id,
                CpuInfo::Static {
                    current_hz: frequency_hz,
                },
            );
            current = None;
        } else {
            let Some(line) = line.strip_prefix("processor") else {
                continue;
            };
            let line = line.trim_start();
            let line = line
                .trim_start()
                .strip_prefix(":")
                .expect("/proc/cpuinfo file format is incorrect")
                .trim_start();
            let id = line.parse::<usize>().expect("couldn't parse frequency");
            if cpuset.contains(&id) {
                current = Some(id);
            }
        }
    }
    Some(cpu_frequencies)
}
