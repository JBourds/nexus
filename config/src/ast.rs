use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::path::PathBuf;

pub type LinkHandle = String;
pub type ChannelHandle = String;
pub type NodeHandle = String;
pub type ProtocolHandle = String;

#[derive(Clone, Debug)]
pub struct Simulation {
    pub params: Params,
    pub channels: HashMap<ChannelHandle, Channel>,
    pub nodes: HashMap<NodeHandle, Vec<Node>>,
}

#[derive(Clone, Debug, Default)]
pub struct Link {
    pub signal: Signal,
    pub bit_error: DistanceProbVar,
    pub packet_loss: DistanceProbVar,
    pub delays: DelayCalculator,
}

#[derive(Clone, Debug, Default)]
pub struct Channel {
    pub link: Link,
    pub r#type: ChannelType,
}

#[derive(Clone, Debug)]
pub enum ChannelType {
    /// No channel buffering other than transmission time,
    /// allow reading during transmissions.
    Shared {
        /// Time to live once it has reached destination
        ttl: Option<NonZeroU64>,
        /// Time unit `ttl` is in
        unit: TimeUnit,
        /// Maximum message size in bytes
        max_size: NonZeroU64,
        /// Should a sender be able to read their own writes?
        read_own_writes: bool,
    },
    /// Buffer some number of messages at a time.
    Exclusive {
        /// Time to live once it has reached destination
        ttl: Option<NonZeroU64>,
        /// Time unit `ttl` is in
        unit: TimeUnit,
        /// Maximum message size in bytes.
        max_size: NonZeroU64,
        /// Number of buffered messages per node. If None, is infinite.
        nbuffered: Option<NonZeroU64>,
    },
}

impl ChannelType {
    pub const MSG_MAX_DEFAULT: NonZeroU64 = NonZeroU64::new(4096).unwrap();

    pub fn ttl(&self) -> Option<NonZeroU64> {
        match self {
            ChannelType::Shared { ttl, .. } => *ttl,
            ChannelType::Exclusive { ttl, .. } => *ttl,
        }
    }

    pub fn max_buffered(&self) -> Option<NonZeroU64> {
        match self {
            ChannelType::Shared { .. } => Some(NonZeroU64::new(1).unwrap()),
            ChannelType::Exclusive { nbuffered, .. } => *nbuffered,
        }
    }

    pub fn max_buf_size(&self) -> NonZeroU64 {
        match self {
            ChannelType::Shared { max_size, .. } => *max_size,
            ChannelType::Exclusive { max_size, .. } => *max_size,
        }
    }

    pub fn delivers_to_self(&self) -> bool {
        match self {
            ChannelType::Shared {
                read_own_writes, ..
            } => *read_own_writes,
            _ => false,
        }
    }
}

impl Default for ChannelType {
    fn default() -> Self {
        Self::Exclusive {
            ttl: None,
            unit: TimeUnit::Seconds,
            nbuffered: None,
            max_size: Self::MSG_MAX_DEFAULT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub position: Position,
    pub internal_names: Vec<ChannelHandle>,
    pub protocols: HashMap<ProtocolHandle, NodeProtocol>,
}

#[derive(Clone, Debug)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub runner: Cmd,
    pub outbound: HashSet<ChannelHandle>,
    pub inbound: HashSet<ChannelHandle>,
}

#[derive(Clone, Debug)]
pub struct Cmd {
    pub cmd: String,
    pub args: Vec<String>,
}

#[derive(Clone, Default, Debug)]
pub struct Position {
    pub orientation: Orientation,
    pub point: Point,
    pub unit: DistanceUnit,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Orientation {
    pub az: f64,
    pub el: f64,
    pub roll: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Signal {
    pub range: ConnectionRange,
    pub shape: SignalShape,
    pub unit: DistanceUnit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignalShape {
    Omnidirectional,
    Cone,
    Direct,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnectionRange {
    pub maximum: Option<f64>,
    pub offset: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimestepConfig {
    pub length: u64,
    pub unit: TimeUnit,
    pub count: NonZeroU64,
}

#[derive(Clone, Debug)]
pub struct Params {
    pub timestep: TimestepConfig,
    pub seed: u64,
    pub root: PathBuf,
}

#[derive(Clone, Default)]
pub struct DelayCalculator {
    pub transmission: Rate,
    pub processing: Rate,
    pub propagation: DistanceTimeVar,
    pub ts_config: TimestepConfig,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delays {
    pub transmission: Rate,
    pub processing: Rate,
    pub propagation: DistanceTimeVar,
}

/// Expression of `x` (distance) which is equal to the duration in `unit`s
/// for an event to occur (ex. Bits to propagate).
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceTimeVar {
    pub rate: meval::Expr,
    pub time: TimeUnit,
    pub distance: DistanceUnit,
}

/// Expression of `x` in `distance` units and `y` in `size` units which equals
/// the probability of an event happening given a distance and payload size.
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceProbVar {
    pub rate: meval::Expr,
    pub distance: DistanceUnit,
    pub size: DataUnit,
}

impl DistanceProbVar {
    /// Simulates a single sampling of a probability variable using distance
    /// and data amounts ("x" and "y").
    pub fn sample(
        &self,
        distance: f64,
        distance_unit: DistanceUnit,
        data: u64,
        data_unit: DataUnit,
        rng: &mut rand::rngs::StdRng,
    ) -> bool {
        let func = self.rate.clone().bind2("x", "y").unwrap();
        let (should_scale_down, ratio) = DistanceUnit::ratio(self.distance, distance_unit);
        let scalar = 10u64
            .checked_pow(ratio.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        let distance = if should_scale_down {
            distance / scalar
        } else {
            distance * scalar
        };
        let (should_scale_down, lshifts) = DataUnit::ratio(self.size, data_unit);
        let scalar = 1u64
            .checked_shl(lshifts.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        let data = if should_scale_down {
            data as f64 / scalar
        } else {
            data as f64 * scalar
        };
        let prob = func(distance, data).clamp(0.0, 1.0);
        let random: f64 = rng.random_range(0.0..=1.0);
        prob <= random
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub rate: u64,
    pub data: DataUnit,
    pub time: TimeUnit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeUnit {
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DistanceUnit {
    Millimeters,
    Centimeters,
    Meters,
    Kilometers,
}

impl Position {
    /// Return 3D euclidean distance between two points
    /// after converting to a common unit system.
    pub fn distance(from: &Self, to: &Self) -> (f64, DistanceUnit) {
        let (from_greater, ratio) = DistanceUnit::ratio(from.unit, to.unit);
        let scalar = 10.0_f64.powi(ratio as i32);
        let unit = if from_greater { from.unit } else { to.unit };
        let scale = |(x, y, z), scale_up| {
            if scale_up {
                (x * scalar, y * scalar, z * scalar)
            } else {
                (x, y, z)
            }
        };

        let (from_x, from_y, from_z) =
            scale((from.point.x, from.point.y, from.point.z), !from_greater);
        let (to_x, to_y, to_z) = scale((to.point.x, to.point.y, to.point.z), from_greater);

        let x = from_x - to_x;
        let y = from_y - to_y;
        let z = from_z - to_z;
        ((x * x + y * y + z * z).sqrt(), unit)
    }
}

impl DelayCalculator {
    /// Determine how many timesteps are required to delay for based on the
    /// distance of the transmission and amount of data to transmit.
    ///
    /// Params:
    /// - `distance`: Distance in `distance_unit`s.
    /// - `amount`: Amount of data in `data_unit`s.
    ///
    /// Returns:
    /// - Number of timeseps to delay.
    pub fn timestep_delay(
        &self,
        distance: f64,
        amount: u64,
        data_unit: DataUnit,
        distance_unit: DistanceUnit,
    ) -> u64 {
        let (proc_num, proc_den) =
            Self::timesteps_required(amount, data_unit, self.processing, self.ts_config);
        let (trans_num, trans_den) =
            Self::timesteps_required(amount, data_unit, self.transmission, self.ts_config);
        let prop_timesteps = self.propagation_timesteps_f64(distance, distance_unit);
        let mut num = proc_num * trans_den + trans_num * proc_den;
        let den = proc_den * trans_den;
        // If this takes any time at all, make sure the numerator has something
        // so the event doesn't happen instantaneously.
        let added_timesteps = prop_timesteps * den as f64;
        if added_timesteps as u64 == 0 && added_timesteps > 0.0 {
            num += 1
        } else {
            num += (prop_timesteps * den as f64) as u64;
        }
        num.div_ceil(den)
    }

    pub fn processing_timesteps_u64(&self, amount: u64, data_unit: DataUnit) -> (u64, u64) {
        Self::timesteps_required(amount, data_unit, self.processing, self.ts_config)
    }

    pub fn transmission_timesteps_u64(&self, amount: u64, data_unit: DataUnit) -> (u64, u64) {
        Self::timesteps_required(amount, data_unit, self.transmission, self.ts_config)
    }

    pub fn processing_timesteps_f64(&self, amount: u64, data_unit: DataUnit) -> f64 {
        let (num, den) =
            Self::timesteps_required(amount, data_unit, self.processing, self.ts_config);
        num as f64 / den as f64
    }

    pub fn transmission_timesteps_f64(&self, amount: u64, data_unit: DataUnit) -> f64 {
        let (num, den) =
            Self::timesteps_required(amount, data_unit, self.transmission, self.ts_config);
        num as f64 / den as f64
    }

    pub fn propagation_timesteps_f64(&self, distance: f64, unit: DistanceUnit) -> f64 {
        let func = self.propagation.rate.clone().bind("x").unwrap();
        // Number of `distance_unit` / `time_unit` for value of `distance`
        let (should_scale_down, ratio) = DistanceUnit::ratio(self.propagation.distance, unit);
        // Scale distance units
        let scalar = 10u64
            .checked_pow(ratio.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        let distance = if should_scale_down {
            distance / scalar
        } else {
            distance * scalar
        };
        let time_units = func(distance);

        // Scale time units
        let (should_scale_down, time_ratio) =
            TimeUnit::ratio(self.propagation.time, self.ts_config.unit);
        let scalar = 10_u64
            .checked_pow(time_ratio.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        if should_scale_down {
            time_units / scalar
        } else {
            time_units * scalar
        }
    }
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
            DataUnit::Bit => 0,
            DataUnit::Kilobit => 10,
            DataUnit::Megabit => 20,
            DataUnit::Gigabit => 30,
            DataUnit::Byte => 3,
            DataUnit::Kilobyte => 13,
            DataUnit::Megabyte => 23,
            DataUnit::Gigabyte => 33,
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
            TimeUnit::Seconds => 0,
            TimeUnit::Milliseconds => 3,
            TimeUnit::Microseconds => 6,
            TimeUnit::Nanoseconds => 9,
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
            DistanceUnit::Millimeters => 0,
            DistanceUnit::Centimeters => 2,
            DistanceUnit::Meters => 4,
            DistanceUnit::Kilometers => 7,
        }
    }
}

// Manual trait impls

impl std::fmt::Debug for DelayCalculator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DelayCalculator {{ .. }}")
    }
}

impl std::fmt::Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.cmd, self.args.join(" "))
    }
}

impl Default for TimestepConfig {
    fn default() -> Self {
        Self {
            length: Self::DEFAULT_TIMESTEP_LEN,
            unit: TimeUnit::default(),
            count: Self::DEFAULT_TIMESTEP_COUNT,
        }
    }
}

impl Default for DistanceTimeVar {
    fn default() -> Self {
        Self {
            rate: "0".parse().unwrap(),
            time: Default::default(),
            distance: Default::default(),
        }
    }
}

impl Default for DistanceProbVar {
    fn default() -> Self {
        Self {
            rate: "0".parse().unwrap(),
            distance: DistanceUnit::default(),
            size: DataUnit::default(),
        }
    }
}

impl Default for SignalShape {
    fn default() -> Self {
        Self::Omnidirectional
    }
}

impl Default for Rate {
    fn default() -> Self {
        Self {
            rate: u64::MAX,
            data: DataUnit::default(),
            time: TimeUnit::default(),
        }
    }
}

impl Default for DataUnit {
    fn default() -> Self {
        Self::Bit
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_calculator() {
        let ts_config = TimestepConfig {
            length: 1,
            unit: TimeUnit::Seconds,
            count: NonZeroU64::new(1000000).unwrap(),
        };
        let transmission = Rate {
            rate: 200,
            data: DataUnit::Bit,
            time: TimeUnit::Seconds,
        };
        let processing = Rate {
            rate: 200,
            data: DataUnit::Bit,
            time: TimeUnit::Seconds,
        };
        let propagation = DistanceTimeVar {
            rate: "5 * x".parse().unwrap(),
            time: TimeUnit::Seconds,
            distance: DistanceUnit::Kilometers,
        };
        let delays = Delays {
            transmission,
            processing,
            propagation,
        };
        let mut calculator = DelayCalculator::validate(delays, ts_config).unwrap();
        use DataUnit::*;
        use DistanceUnit::*;
        let tests = [
            // Data unit conversions
            (0.0, 200, Byte, Kilometers, (2.0 * 8.0_f64).ceil() as u64),
            (
                0.0,
                200,
                Kilobit,
                Kilometers,
                (2.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Kilobyte,
                Kilometers,
                (2.0 * 8.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Megabit,
                Kilometers,
                (2.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Megabyte,
                Kilometers,
                (2.0 * 8.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Gigabit,
                Kilometers,
                (2.0 * 1024.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Gigabyte,
                Kilometers,
                (2.0 * 8.0 * 1024.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            // Distance conversions (propagation distances)
            (0.0, 0, Bit, Millimeters, 0),
            (0.001, 0, Bit, Millimeters, 1),
            (1.0, 0, Bit, Millimeters, 1),
            (100.0, 0, Bit, Millimeters, 1),
            (100.0 * 100.0, 0, Bit, Millimeters, 1),
            (100.0 * 100.0 * 99.0, 0, Bit, Millimeters, 1),
            (100.0 * 100.0 * 100.0, 0, Bit, Millimeters, 1),
            (100.0 * 100.0 * 200.0, 0, Bit, Millimeters, 1),
            (100.0 * 100.0 * 201.0, 0, Bit, Millimeters, 2),
            (100.0 * 100.0 * 300.0, 0, Bit, Millimeters, 2),
            (100.0 * 100.0 * 400.0, 0, Bit, Millimeters, 2),
            (100.0 * 100.0 * 400.0001, 0, Bit, Millimeters, 2),
            (100.0 * 100.0 * 1000.0, 0, Bit, Millimeters, 5),
            (100.0 * 100.0 * 1001.0, 0, Bit, Millimeters, 6),
            // Full pipeline (numerator/denominator conversions)
            (0.0001, 0, Bit, Kilometers, 1),
            (0.0, 1, Bit, Kilometers, 1),
            (0.0, 100, Bit, Kilometers, 1),
            (1.0, 0, Bit, Kilometers, 5),
            (1.0, 200, Bit, Kilometers, 7),
            (1.4, 200, Bit, Kilometers, 9),
            (1.9, 200, Bit, Kilometers, 12),
            (2.0, 200, Bit, Kilometers, 12),
            // Conversions on both units
            (
                0.0001,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64,
            ),
            (
                1.0,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64,
            ),
            (
                100.0,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64 + 1,
            ),
            (
                1000.0,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64 + 5,
            ),
        ];
        for (distance, amount, data_unit, distance_unit, expected) in tests {
            assert_eq!(
                calculator.timestep_delay(distance, amount, data_unit, distance_unit),
                expected
            );
        }

        // Test nonlinear expressions
        calculator.propagation = DistanceTimeVar {
            rate: "5 * x^2".parse().unwrap(),
            time: TimeUnit::Seconds,
            distance: DistanceUnit::Meters,
        };
        let tests = [
            // Distance conversions (propagation distances)
            (0.1, 0, Bit, Millimeters, 1),
            (1.0, 0, Bit, Millimeters, 1),
            (100.0, 0, Bit, Millimeters, 1),
            (10000.0, 0, Bit, Millimeters, 5),
            (0.1, 0, Bit, Centimeters, 1),
            (1.0, 0, Bit, Centimeters, 1),
            (100.0, 0, Bit, Centimeters, 5),
            (10000.0, 0, Bit, Centimeters, 50000),
            (0.1, 0, Bit, Meters, 1),
            (1.0, 0, Bit, Meters, 5),
            (100.0, 0, Bit, Meters, 50000),
            (10000.0, 0, Bit, Meters, 500000000),
            (0.1, 0, Bit, Kilometers, 50000),
            (1.0, 0, Bit, Kilometers, 5000000),
            (100.0, 0, Bit, Kilometers, 50000000000),
        ];
        for (distance, amount, data_unit, distance_unit, expected) in tests {
            dbg!((distance, amount, data_unit, distance_unit, expected));
            assert_eq!(
                calculator.timestep_delay(distance, amount, data_unit, distance_unit),
                expected
            );
        }
    }
}
