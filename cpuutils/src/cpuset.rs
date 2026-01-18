use std::fmt::Display;

use libc::{_SC_NPROCESSORS_ONLN, cpu_set_t, pid_t, sched_getaffinity, sched_setaffinity, sysconf};

const BITS_IN_BYTE: usize = 8;

#[derive(Debug)]
pub struct CpuSet {
    set: Vec<u8>,
}

fn get_nprocs() -> Result<usize, ()> {
    let nprocs = unsafe {
        let rc = sysconf(_SC_NPROCESSORS_ONLN);
        if rc == -1 {
            return Err(());
        }
        rc as usize
    };
    Ok(nprocs)
}

fn bytes_needed(nbits: usize) -> usize {
    nbits
        .div_ceil(BITS_IN_BYTE)
        .max(core::mem::size_of::<cpu_set_t>())
}

impl CpuSet {
    pub fn new(num_cpus: usize) -> Self {
        let mut set = vec![0; bytes_needed(num_cpus)];
        set.truncate(num_cpus.div_ceil(BITS_IN_BYTE));
        Self { set }
    }

    pub fn clear(&mut self) -> &mut Self {
        for i in 0..self.set.len() {
            self.set[i] = 0;
        }
        self
    }

    pub fn realloc(&mut self, num_cpus: usize) -> &mut Self {
        let num_bytes = num_cpus / BITS_IN_BYTE;
        if num_bytes <= self.set.len() {
            self.set.truncate(num_bytes);
        } else {
            while self.set.len() < num_bytes {
                self.set.push(0);
            }
        }
        self
    }

    pub fn enable_cpu(&mut self, cpu: usize) -> Result<&mut Self, ()> {
        if self.set_bit(cpu, true) {
            Ok(self)
        } else {
            Err(())
        }
    }

    pub fn disable_cpu(&mut self, cpu: usize) -> Result<&mut Self, ()> {
        if self.set_bit(cpu, false) {
            Ok(self)
        } else {
            Err(())
        }
    }

    /// Get the PID for the currently running process
    pub fn get_current_affinity(&mut self) -> Result<&mut Self, ()> {
        self.get_affinity(0)
    }

    /// Apply the CPU set to a given pid's affinity.
    pub fn set_affinity(&self, pid: u32) -> Result<(), ()> {
        let mask = self.set.as_ptr() as *const cpu_set_t;
        let nbytes = self.cpuset_size();
        let rc = unsafe { sched_setaffinity(pid as pid_t, nbytes, mask) };
        if rc == -1 { Err(()) } else { Ok(()) }
    }

    pub fn get_affinity(&mut self, pid: u32) -> Result<&mut Self, ()> {
        let mask = self.set.as_mut_ptr() as *mut cpu_set_t;
        let nbytes = self.cpuset_size();
        let rc = unsafe { sched_getaffinity(pid as pid_t, nbytes, mask) };
        if rc == -1 { Err(()) } else { Ok(self) }
    }

    /// Get the CPU affinity of the process and return it.
    pub fn with_nprocs() -> Result<Self, ()> {
        let nprocs = get_nprocs()?;
        let mut set = vec![0; bytes_needed(nprocs)];
        set.truncate(nprocs.div_ceil(BITS_IN_BYTE));
        Ok(Self { set })
    }

    pub fn enabled_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        let mut id = 0;
        for byte in self.set.iter().copied() {
            for bit_index in 0..BITS_IN_BYTE {
                let is_set = byte & (1 << bit_index) != 0;
                if is_set {
                    ids.push(id);
                }
                id += 1;
            }
        }
        ids
    }

    fn cpuset_size(&self) -> usize {
        std::cmp::max(self.set.len(), core::mem::size_of::<cpu_set_t>())
    }

    fn set_bit(&mut self, index: usize, value: bool) -> bool {
        let byte_index = index / BITS_IN_BYTE;
        if byte_index >= self.set.len() {
            return false;
        }
        let byte = self.set.get_mut(byte_index).expect("index out of bounds");
        let bit_index = index % BITS_IN_BYTE;
        let mask = 1 << bit_index;
        if value {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        true
    }
}

impl Default for CpuSet {
    fn default() -> Self {
        Self {
            set: vec![0; core::mem::size_of::<cpu_set_t>()],
        }
    }
}

impl Display for CpuSet {
    /// display as comma-separated list of IDs
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<_> = self
            .enabled_ids()
            .into_iter()
            .map(|i| i.to_string())
            .collect();
        f.write_str(&ids.join(","))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_zeroed_set() {
        let cpus = CpuSet::new(16);
        assert_eq!(cpus.set.len(), 2);
        assert!(cpus.set.iter().all(|&b| b == 0));
    }

    #[test]
    fn clear_resets_all_bits() {
        let mut cpus = CpuSet::new(16);
        cpus.enable_cpu(0).unwrap();
        cpus.enable_cpu(7).unwrap();
        cpus.enable_cpu(8).unwrap();

        cpus.clear();
        assert!(cpus.set.iter().all(|&b| b == 0));
    }

    #[test]
    fn enable_and_disable_cpu_bits() {
        let mut cpus = CpuSet::new(16);

        cpus.enable_cpu(3).unwrap();
        cpus.enable_cpu(12).unwrap();

        assert_eq!(cpus.set[0] & (1 << 3), 1 << 3);
        assert_eq!(cpus.set[1] & (1 << 4), 1 << 4);

        cpus.disable_cpu(3).unwrap();
        assert_eq!(cpus.set[0] & (1 << 3), 0);
    }

    #[test]
    fn enable_cpu_out_of_bounds_fails() {
        let mut cpus = CpuSet::new(8);
        assert!(cpus.enable_cpu(8).is_err());
        assert!(cpus.disable_cpu(100).is_err());
    }

    #[test]
    fn realloc_grows_and_shrinks_correctly() {
        let mut cpus = CpuSet::new(8);
        cpus.enable_cpu(3).unwrap();

        cpus.realloc(32);
        assert_eq!(cpus.set.len(), 4);
        assert_eq!(cpus.set[0] & (1 << 3), 1 << 3);

        cpus.realloc(8);
        assert_eq!(cpus.set.len(), 1);
        assert_eq!(cpus.set[0] & (1 << 3), 1 << 3);
    }

    #[test]
    fn display_empty_set_is_empty_string() {
        let cpus = CpuSet::new(16);
        assert_eq!(format!("{}", cpus), "");
    }

    #[test]
    fn display_formats_comma_separated_ids() {
        let mut cpus = CpuSet::new(16);
        cpus.enable_cpu(0).unwrap();
        cpus.enable_cpu(3).unwrap();
        cpus.enable_cpu(8).unwrap();

        let output = format!("{}", cpus);
        assert_eq!(output, "0,3,8");
    }

    #[test]
    fn with_nprocs_allocates_enough_space() {
        let cpus = CpuSet::with_nprocs().expect("with_nprocs failed");
        assert!(!cpus.set.is_empty());
    }

    #[test]
    fn get_get_current_affinity_does_not_fail() {
        let mut cpus = CpuSet::with_nprocs().unwrap();
        cpus.get_current_affinity().expect("get_affinity failed");
    }

    #[test]
    fn set_and_get_affinity_round_trip() {
        let mut cpus = CpuSet::with_nprocs().unwrap();
        cpus.clear();

        // Try enabling CPU 0 — this should be safe on all systems
        cpus.enable_cpu(0).unwrap();

        // Setting affinity may fail on some systems (containers, sandboxed CI)
        if cpus.set_affinity(0).is_ok() {
            let mut readback = CpuSet::with_nprocs().unwrap();
            readback.get_affinity(0).unwrap();

            // CPU 0 should be set
            assert_eq!(readback.set[0] & 1, 1);
        }
    }
}
