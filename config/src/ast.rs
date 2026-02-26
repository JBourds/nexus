use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::SystemTime;

pub type LinkHandle = String;
pub type ChannelHandle = String;
pub type NodeHandle = String;
pub type ProtocolHandle = String;
pub type SinkHandle = String;
pub type SourceHandle = String;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Simulation {
    pub params: Params,
    pub channels: HashMap<ChannelHandle, Channel>,
    pub nodes: HashMap<NodeHandle, Node>,
    pub sinks: HashMap<SinkHandle, PowerRate>,
    pub sources: HashMap<SourceHandle, PowerRate>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Link {
    pub medium: Medium,
    pub bit_error: RssiProbExpr,
    pub packet_loss: RssiProbExpr,
    pub delays: DelayCalculator,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Channel {
    pub link: Link,
    pub r#type: ChannelType,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChannelType {
    /// No channel buffering other than transmission & propagation time (because
    /// this is a shared medium, there can only be one source of truth for what
    /// data can be read). If multiple nodes write at once or during overlapping
    /// periods, the result is the bitwise OR of writes.
    Shared {
        /// Time to live once it has reached destination
        ttl: Option<NonZeroU64>,
        /// Time unit `ttl` is in
        unit: TimeUnit,
        /// Should a sender be able to read their own writes?
        read_own_writes: bool,
        /// Maximum message size in bytes.
        max_size: NonZeroUsize,
    },
    /// Buffer some number of messages at a time for each node.
    Exclusive {
        /// Time to live once it has reached destination
        ttl: Option<NonZeroU64>,
        /// Time unit `ttl` is in
        unit: TimeUnit,
        /// Maximum message size in bytes.
        max_size: NonZeroUsize,
        /// Number of buffered messages per node. If None, is infinite.
        nbuffered: Option<NonZeroUsize>,
        /// Should a sender be able to read their own writes?
        /// eg. In an internal link.
        read_own_writes: bool,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Node {
    pub position: Position,
    pub charge: Option<Charge>,
    pub protocols: HashMap<ProtocolHandle, NodeProtocol>,
    pub internal_names: Vec<ChannelHandle>,
    pub resources: Resources,
    pub sinks: HashSet<SinkHandle>,
    pub sources: HashSet<SourceHandle>,
    pub start: SystemTime,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Resources {
    pub cpu: Cpu,
    pub mem: Mem,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Cpu {
    pub cores: Option<NonZeroU64>,
    /// If this is None, don't apply any rate limiting
    pub hertz: Option<NonZeroU64>,
    pub unit: ClockUnit,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Mem {
    /// If this is None, don't apply a memory limit
    pub amount: Option<NonZeroU64>,
    pub unit: DataUnit,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Charge {
    pub max: u64,
    pub quantity: u64,
    pub unit: PowerUnit,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub build: Cmd,
    pub runner: Cmd,
    pub publishers: HashSet<ChannelHandle>,
    pub subscribers: HashSet<ChannelHandle>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Cmd {
    pub cmd: String,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Position {
    pub orientation: Orientation,
    pub point: Point,
    pub unit: DistanceUnit,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct Orientation {
    pub az: f64,
    pub el: f64,
    pub roll: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Signal {
    pub range: ConnectionRange,
    pub shape: SignalShape,
    pub unit: DistanceUnit,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Medium {
    /// Uses Friis transmission model:
    /// P_r / P_t = G_t G_r (λ / (4 π d))^s
    ///   - Converted into RSSI (dB form)
    Wireless {
        /// Shape of the wireless signal
        shape: SignalShape,
        wavelength_meters: f64,
        gain: f64,
        /// Minimum RSSI strength the receiver can pick up on.
        rx_min_dbm: f64,
        /// Range of transmission strength [low, high] in dBm
        tx_min_dbm: f64,
        tx_max_dbm: f64,
    },
    /// Uses a RLGC model
    /// https://triblemany.github.io/archives/afb86e77/transmission-line
    Wired {
        /// Minimum RSSI strength the receiver can pick up on.
        rx_min_dbm: f64,
        /// Range of transmission strength [low, high] in dBm
        tx_min_dbm: f64,
        tx_max_dbm: f64,
        /// Series resistance per unit length (Ω/m)
        r: f64,
        /// Series inductance per unit length (F/m)
        l: f64,
        /// Shunt capacitance per unit length (C/m)
        c: f64,
        /// Shunt conductance per unit length (S/m)
        /// - represents dielectric loss
        g: f64,
        /// Frequency
        f: f64,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum SignalShape {
    #[default]
    Omnidirectional,
    Cone,
    Direct,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnectionRange {
    pub maximum: Option<f64>,
    pub offset: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct TimestepConfig {
    pub length: NonZeroU64,
    pub unit: TimeUnit,
    pub count: NonZeroU64,
    pub start: SystemTime,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Params {
    pub timestep: TimestepConfig,
    pub seed: u64,
    pub root: PathBuf,
    pub time_dilation: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DelayCalculator {
    pub transmission: DataRate,
    pub processing: DataRate,
    pub propagation: DistanceTimeVar,
    pub ts_config: TimestepConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Delays {
    pub transmission: DataRate,
    pub processing: DataRate,
    pub propagation: DistanceTimeVar,
}

/// Expression of `x` (distance) which is equal to the duration in `unit`s
/// for an event to occur (ex. Bits to propagate).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DistanceTimeVar {
    pub rate: String,
    pub time: TimeUnit,
    pub distance: DistanceUnit,
}

/// Calculates probability using `x` as the RSSI variable
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RssiProbExpr {
    pub(crate) expr: String,
    pub(crate) noise_floor_dbm: f64,
}

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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum ClockUnit {
    #[default]
    Hertz,
    Kilohertz,
    Megahertz,
    Gigahertz,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum DataUnit {
    Bit,
    Kilobit,
    Megabit,
    Gigabit,
    #[default]
    Byte,
    Kilobyte,
    Megabyte,
    Gigabyte,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum PowerUnit {
    NanoWatt,
    MicroWatt,
    MilliWatt,
    #[default]
    Watt,
    KiloWatt,
    MegaWatt,
    GigaWatt,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum TimeUnit {
    Hours,
    Minutes,
    #[default]
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum DistanceUnit {
    Millimeters,
    Centimeters,
    Meters,
    #[default]
    Kilometers,
}

// Manual trait impls

impl std::fmt::Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.cmd, self.args.join(" "))
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
