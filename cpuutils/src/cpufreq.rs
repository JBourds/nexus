use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::{collections::BTreeMap, fs::File, path::Path};

use crate::cpuset::CpuSet;

const SYSFS_CPUS: &str = "/sys/devices/system/cpu";
const PROCFS_CPUINFO: &str = "/proc/cpuinfo";
const MEGA: f64 = 1_000_000.0;

#[derive(Debug, Default)]
pub struct CpuInfo {
    uses_scaling: bool,
    pub cores: BTreeMap<usize, CoreInfo>,
}

impl CpuInfo {
    pub fn max_core_id(&self) -> Option<usize> {
        self.cores.keys().max().copied()
    }

    pub fn ncores(&self) -> usize {
        self.cores.len()
    }

    pub fn refresh(&mut self) {
        if self.uses_scaling {
            refresh_scaling_cpuinfo(&mut self.cores);
        } else {
            refresh_static_cpuinfo(&mut self.cores);
        }
    }
}

/// CPUs could be doing frequency scaling or could be static.
/// This enum reflects the best effort reporting and it is up to application
/// code to use this appropriately.
#[derive(Debug)]
pub enum CoreInfo {
    Scaling {
        min_hz: u64,
        max_hz: u64,
        current_hz: u64,
    },
    Static {
        current_hz: u64,
    },
}

impl CoreInfo {
    pub fn frequency(&self) -> u64 {
        match self {
            CoreInfo::Scaling { current_hz, .. } => *current_hz,
            CoreInfo::Static { current_hz } => *current_hz,
        }
    }
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

impl CoreInfo {
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

pub fn get_cpu_info(cpuset: &CpuSet) -> CpuInfo {
    let ids: HashSet<usize> = cpuset.enabled_ids().into_iter().collect();
    if let Some(cores) = parse_scaling_cpuinfo(&ids) {
        CpuInfo {
            uses_scaling: true,
            cores,
        }
    } else if let Some(cores) = parse_static_cpuinfo(&ids) {
        CpuInfo {
            uses_scaling: false,
            cores,
        }
    } else {
        CpuInfo::default()
    }
}

pub fn parse_scaling_cpuinfo(cpuset: &HashSet<usize>) -> Option<BTreeMap<usize, CoreInfo>> {
    let mut cpu_frequencies = BTreeMap::new();
    for &id in cpuset.iter() {
        let info = CoreInfo::scaling(id)?;
        cpu_frequencies.insert(id, info);
    }
    Some(cpu_frequencies)
}

fn refresh_scaling_cpuinfo(cores: &mut BTreeMap<usize, CoreInfo>) {
    for (&id, info) in cores.iter_mut() {
        *info = CoreInfo::scaling(id).expect("CPU core no longer available");
    }
}

fn iter_cpuinfo_hz() -> impl Iterator<Item = (usize, u64)> {
    let file = File::open(PROCFS_CPUINFO).expect("couldn't open procfs file");
    let reader = BufReader::new(file);

    let mut current: Option<usize> = None;

    reader
        .lines()
        .map_while(Result::ok)
        .filter_map(move |line| {
            if let Some(id) = current {
                let line = line.strip_prefix("cpu MHz")?;
                let mhz: f64 = line
                    .trim_start()
                    .strip_prefix(":")?
                    .trim_start()
                    .parse()
                    .ok()?;

                current = None;
                Some((id, (mhz * MEGA) as u64))
            } else {
                let line = line.strip_prefix("processor")?;
                let id: usize = line
                    .trim_start()
                    .strip_prefix(":")?
                    .trim_start()
                    .parse()
                    .ok()?;

                current = Some(id);
                None
            }
        })
}

pub fn parse_static_cpuinfo(cpuset: &HashSet<usize>) -> Option<BTreeMap<usize, CoreInfo>> {
    Some(
        iter_cpuinfo_hz()
            .filter(|(id, _)| cpuset.contains(id))
            .map(|(id, hz)| (id, CoreInfo::Static { current_hz: hz }))
            .collect(),
    )
}

pub fn refresh_static_cpuinfo(map: &mut BTreeMap<usize, CoreInfo>) {
    iter_cpuinfo_hz().for_each(|(id, hz)| {
        if let Some(info) = map.get_mut(&id) {
            *info = CoreInfo::Static { current_hz: hz };
        }
    })
}
