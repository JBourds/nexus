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

/// Trait for unit types that scale by powers of 10.
/// Provides a shared `ratio()` implementation.
pub trait DecimalScaled: Copy {
    /// Log10 exponent relative to the smallest unit in this family.
    fn power(self) -> usize;

    /// Return `(left_is_larger, exponent_difference)` between two units.
    /// The caller computes `10^exponent_difference` to get the scaling factor.
    fn ratio(left: Self, right: Self) -> (bool, usize) {
        let l = left.power();
        let r = right.power();
        (l > r, l.abs_diff(r))
    }
}

impl DecimalScaled for PowerUnit {
    fn power(self) -> usize {
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

impl DecimalScaled for TimeUnit {
    fn power(self) -> usize {
        match self {
            Self::Seconds => 0,
            Self::Milliseconds => 3,
            Self::Microseconds => 6,
            Self::Nanoseconds => 9,
            _ => unimplemented!("power() only supported on time intervals with SI prefixes."),
        }
    }
}

impl DecimalScaled for DistanceUnit {
    fn power(self) -> usize {
        match self {
            Self::Millimeters => 0,
            Self::Centimeters => 1,
            Self::Meters => 3,
            Self::Kilometers => 6,
        }
    }
}

impl EnergyUnit {
    /// Convert `quantity` in this unit to nanojoules (saturating on overflow).
    pub fn to_nj(self, quantity: u64) -> u64 {
        match self {
            Self::NanoJoule => quantity,
            Self::MicroJoule => quantity.saturating_mul(1_000),
            Self::MilliJoule => quantity.saturating_mul(1_000_000),
            Self::Joule => quantity.saturating_mul(1_000_000_000),
            Self::KiloJoule => quantity.saturating_mul(1_000_000_000_000),
            Self::MicroWattHour => quantity.saturating_mul(3_600),
            Self::MilliWattHour => quantity.saturating_mul(3_600_000),
            Self::WattHour => quantity.saturating_mul(3_600_000_000),
            Self::KiloWattHour => quantity.saturating_mul(3_600_000_000_000),
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
        let rate_nw = self.rate.saturating_mul(self.unit.to_nw_factor());
        let time_ns = self.time.to_ns_factor();
        rate_nw.saturating_mul(timestep_ns) / time_ns
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
