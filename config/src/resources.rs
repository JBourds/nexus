use std::num::NonZeroU64;

use crate::ast::{Cpu, Mem, Resources};

impl Resources {
    pub fn has_cpu_limit(&self) -> bool {
        self.cpu.has_limit()
    }

    pub fn scale_cpu(&mut self, ratio: f64) {
        self.cpu.scale_cycles(ratio);
    }

    pub fn has_mem_limit(&self) -> bool {
        self.mem.has_limit()
    }
}

impl Mem {
    pub fn has_limit(&self) -> bool {
        self.amount.is_some()
    }
}

impl Cpu {
    pub fn scale_cycles(&mut self, ratio: f64) {
        if let Some(hertz) = self.hertz.as_mut() {
            let res = (hertz.get() as f64 * ratio) as u64;
            *hertz = NonZeroU64::new(res).unwrap_or(*hertz);
        }
    }

    pub fn has_limit(&self) -> bool {
        self.hertz.is_some()
    }

    /// Returns the maximum bandwidth limit and period (corresponds to cpu.max
    /// in cgroup limits if a limit should be imposed.
    pub fn requested_cycles(&self) -> Option<u64> {
        let cores = self.cores.map(NonZeroU64::get).unwrap_or(1);
        let rate = self.hertz.map(NonZeroU64::get)?;
        let rate_lshifts = self.unit.lshifts() as u64;
        Some(cores * (rate << rate_lshifts))
    }
}
