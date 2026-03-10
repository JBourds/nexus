use anyhow::{Context, Result, bail};

use crate::ast::{
    ClockUnit, DataRate, DataUnit, DistanceUnit, EnergyUnit, PowerRate, PowerUnit, TimeUnit,
};

/// Parse a duration string like `"6h"`, `"30m"`, `"1s"`, `"500ms"`, `"100us"`
/// into microseconds.
pub fn parse_duration_to_us(s: &str) -> Result<u64> {
    let s = s.trim();
    let num_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(num_end);
    let num: u64 = num_str
        .parse()
        .context(format!("invalid number in duration \"{s}\""))?;
    let factor: u64 = match unit_str {
        "h" => 3_600_000_000,
        "m" => 60_000_000,
        "s" => 1_000_000,
        "ms" => 1_000,
        "us" => 1,
        _ => bail!("unknown duration unit \"{unit_str}\" in \"{s}\"; expected h, m, s, ms, or us"),
    };
    Ok(num * factor)
}

impl DataUnit {
    /// Return the left shift ratio of left / right with a boolean
    /// flag to indicate whether it was the left (true) or right
    /// (false) which is the numerator in the expression.
    pub fn ratio(left: Self, right: Self) -> (bool, usize) {
        let left = left.lshifts();
        let right = right.lshifts();
        let left_greater = left > right;
        let ratio = std::cmp::max(left, right) - std::cmp::min(left, right);
        (left_greater, ratio)
    }

    pub fn lshifts(&self) -> usize {
        match self {
            Self::Bit => 0,
            Self::Kilobit => 10,
            Self::Megabit => 20,
            Self::Gigabit => 30,
            Self::Byte => 3,
            Self::Kilobyte => 13,
            Self::Megabyte => 23,
            Self::Gigabyte => 33,
        }
    }
}

impl ClockUnit {
    pub fn lshifts(&self) -> usize {
        match self {
            Self::Hertz => 0,
            Self::Kilohertz => 10,
            Self::Megahertz => 20,
            Self::Gigahertz => 30,
        }
    }
}

impl PowerUnit {
    /// Return the log_10 ratio of left / right with a boolean
    /// flag to indicate whether it was the left (true) or right
    /// (false) which is the numerator in the expression.
    pub fn ratio(left: Self, right: Self) -> (bool, usize) {
        let left = left.power();
        let right = right.power();
        let left_greater = left > right;
        let ratio = std::cmp::max(left, right) - std::cmp::min(left, right);
        (left_greater, ratio)
    }

    pub fn power(&self) -> usize {
        match self {
            Self::NanoWatt => 0,
            Self::MicroWatt => 3,
            Self::MilliWatt => 6,
            Self::Watt => 9,
            Self::KiloWatt => 12,
            Self::MegaWatt => 15,
            Self::GigaWatt => 18,
        }
    }
}

impl TimeUnit {
    /// Return the log_10 ratio of left / right with a boolean
    /// flag to indicate whether it was the left (true) or right
    /// (false) which is the numerator in the expression.
    pub fn ratio(left: Self, right: Self) -> (bool, usize) {
        let left = left.power();
        let right = right.power();
        let left_greater = left > right;
        let ratio = std::cmp::max(left, right) - std::cmp::min(left, right);
        (left_greater, ratio)
    }

    pub fn power(&self) -> usize {
        match self {
            Self::Seconds => 0,
            Self::Milliseconds => 3,
            Self::Microseconds => 6,
            Self::Nanoseconds => 9,
            _ => unimplemented!("power() only supported on time intervals with SI prefices."),
        }
    }
}

impl DistanceUnit {
    /// Return the log_10 ratio of left / right with a boolean
    /// flag to indicate whether it was the left (true) or right
    /// (false) which is the numerator in the expression.
    pub fn ratio(left: Self, right: Self) -> (bool, usize) {
        let left = left.power();
        let right = right.power();
        let left_greater = left > right;
        let ratio = std::cmp::max(left, right) - std::cmp::min(left, right);
        (left_greater, ratio)
    }

    pub fn power(&self) -> usize {
        match self {
            Self::Millimeters => 0,
            Self::Centimeters => 2,
            Self::Meters => 4,
            Self::Kilometers => 7,
        }
    }
}

impl EnergyUnit {
    /// Convert `quantity` in this unit to nanojoules.
    pub fn to_nj(self, quantity: u64) -> u64 {
        match self {
            Self::NanoJoule => quantity,
            Self::MicroJoule => quantity * 1_000,
            Self::MilliJoule => quantity * 1_000_000,
            Self::Joule => quantity * 1_000_000_000,
            Self::KiloJoule => quantity * 1_000_000_000_000,
            Self::MicroWattHour => quantity * 3_600,
            Self::MilliWattHour => quantity * 3_600_000,
            Self::WattHour => quantity * 3_600_000_000,
            Self::KiloWattHour => quantity * 3_600_000_000_000,
        }
    }
}

impl PowerUnit {
    /// Factor to convert a value in this unit to nanowatts.
    pub fn to_nw_factor(self) -> u64 {
        match self {
            Self::NanoWatt => 1,
            Self::MicroWatt => 1_000,
            Self::MilliWatt => 1_000_000,
            Self::Watt => 1_000_000_000,
            Self::KiloWatt => 1_000_000_000_000,
            Self::MegaWatt => 1_000_000_000_000_000,
            Self::GigaWatt => 1_000_000_000_000_000_000,
        }
    }
}

impl TimeUnit {
    /// Factor to convert a value in this unit to nanoseconds.
    pub fn to_ns_factor(self) -> u64 {
        match self {
            Self::Hours => 3_600_000_000_000,
            Self::Minutes => 60_000_000_000,
            Self::Seconds => 1_000_000_000,
            Self::Milliseconds => 1_000_000,
            Self::Microseconds => 1_000,
            Self::Nanoseconds => 1,
        }
    }
}

impl PowerRate {
    /// Convert this rate to nanojoules per timestep of `timestep_ns` nanoseconds.
    ///
    /// Formula: energy_nj = rate_nw × timestep_ns / time_ns
    pub fn nj_per_timestep(&self, timestep_ns: u64) -> u64 {
        let rate_nw = self.rate * self.unit.to_nw_factor();
        let time_ns = self.time.to_ns_factor();
        rate_nw * timestep_ns / time_ns
    }
}

impl Default for DataRate {
    fn default() -> Self {
        Self {
            // Needs to be < i64::MAX because of TOML limitation
            rate: i64::MAX as u64,
            data: DataUnit::default(),
            time: TimeUnit::default(),
        }
    }
}
