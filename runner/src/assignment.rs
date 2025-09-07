use std::collections::HashMap;
use std::io::BufRead;
use std::process::Command;
use sysinfo::{Cpu, CpuRefreshKind, RefreshKind, System};

pub type Frequency = u64;
pub type CpuNum = usize;
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
