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
    Live {
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
    MsgBuffered {
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
}

impl Default for ChannelType {
    fn default() -> Self {
        Self::MsgBuffered {
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
        let prop_timesteps = self.propagation_delay(distance, distance_unit);
        let mut num = proc_num * trans_den + trans_num * proc_den;
        let den = proc_den * trans_den;
        num += (prop_timesteps * den as f64) as u64;
        num.div_ceil(den)
    }

    fn propagation_delay(&self, distance: f64, unit: DistanceUnit) -> f64 {
        let func = self.propagation.rate.clone().bind("x").unwrap();
        // Number of `distance_unit` / `time_unit` for value of `distance`
        let dist_time_units = func(distance);
        let (distance_prop_greater, distance_ratio) =
            DistanceUnit::ratio(self.propagation.distance, unit);
        // Scale distance units
        let scalar = 10u64
            .checked_pow(distance_ratio.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        let (distance_num, distance_den) = if distance_prop_greater {
            (dist_time_units, scalar)
        } else {
            (dist_time_units * scalar, 1.0)
        };
        // Scale time units
        let (time_prop_greater, time_ratio) =
            TimeUnit::ratio(self.propagation.time, self.ts_config.unit);
        let scalar = 10_u64
            .checked_pow(time_ratio.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        if time_prop_greater {
            distance_num * scalar / distance_den
        } else {
            distance_num / distance_den * scalar
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
            DistanceUnit::Centimeters => 3,
            DistanceUnit::Meters => 6,
            DistanceUnit::Kilometers => 9,
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
            count: NonZeroU64::new(1000).unwrap(),
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
        let calculator = DelayCalculator::validate(delays, ts_config).unwrap();
        let tests = [
            ((0.0001, 0, DataUnit::Bit, DistanceUnit::Kilometers), 1),
            ((0.0, 1, DataUnit::Bit, DistanceUnit::Kilometers), 1),
            ((0.0, 100, DataUnit::Bit, DistanceUnit::Kilometers), 1),
            ((1.0, 0, DataUnit::Bit, DistanceUnit::Kilometers), 5),
            ((1.0, 200, DataUnit::Bit, DistanceUnit::Kilometers), 7),
            ((1.4, 200, DataUnit::Bit, DistanceUnit::Kilometers), 9),
            ((1.9, 200, DataUnit::Bit, DistanceUnit::Kilometers), 12),
            ((2.0, 200, DataUnit::Bit, DistanceUnit::Kilometers), 12),
        ];
        for ((distance, amount, data_unit, distance_unit), expected) in tests.into_iter() {
            assert_eq!(
                calculator.timestep_delay(distance, amount, data_unit, distance_unit),
                expected
            );
        }
    }
}
