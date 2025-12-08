use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct DataRate {
    pub rate: u64,
    pub data: DataUnit,
    pub time: TimeUnit,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PowerRate {
    pub rate: i64,
    pub unit: PowerUnit,
    pub time: TimeUnit,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ClockUnit {
    Hertz,
    Kilohertz,
    Megahertz,
    Gigahertz,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum DataUnit {
    Bit,
    Kilobit,
    Megabit,
    Gigabit,
    Byte,
    Kilobyte,
    Megabyte,
    Gigabyte,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum PowerUnit {
    NanoWattHours,
    MicroWattHours,
    MilliWattHours,
    WattHours,
    KiloWattHours,
    MegaWattHours,
    GigaWattHours,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum TimeUnit {
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum DistanceUnit {
    Millimeters,
    Centimeters,
    Meters,
    Kilometers,
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
            Self::NanoWattHours => 0,
            Self::MicroWattHours => 3,
            Self::MilliWattHours => 6,
            Self::WattHours => 9,
            Self::KiloWattHours => 12,
            Self::MegaWattHours => 15,
            Self::GigaWattHours => 18,
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

impl Default for PowerUnit {
    fn default() -> Self {
        Self::WattHours
    }
}

impl Default for ClockUnit {
    fn default() -> Self {
        Self::Gigahertz
    }
}

impl Default for DataUnit {
    fn default() -> Self {
        Self::Byte
    }
}

impl Default for TimeUnit {
    fn default() -> Self {
        Self::Milliseconds
    }
}

impl Default for DistanceUnit {
    fn default() -> Self {
        Self::Kilometers
    }
}
