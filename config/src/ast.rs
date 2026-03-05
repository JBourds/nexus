use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Serde helper that stores `SystemTime` as epoch_secs + epoch_nanos (two flat
/// integers) so that TOML round-trips correctly.
mod system_time_serde {
    use serde::{self, Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[derive(Serialize, Deserialize)]
    struct Epoch {
        epoch_secs: u64,
        epoch_nanos: u32,
    }

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let dur = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        Epoch {
            epoch_secs: dur.as_secs(),
            epoch_nanos: dur.subsec_nanos(),
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let e = Epoch::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + Duration::new(e.epoch_secs, e.epoch_nanos))
    }
}

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
    #[serde(with = "system_time_serde")]
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
    /// Free-space Friis model (far-field).
    /// Pr(dBm) = Pt + Gt + Gr − 20log10(4πd/λ)
    Wireless {
        /// Radiation pattern (e.g., omnidirectional)
        shape: SignalShape,
        /// Carrier wavelength (meters)
        wavelength_meters: f64,
        /// Total antenna gain (Gt + Gr) in dBi
        gain_dbi: f64,
        /// Minimum receivable power (receiver sensitivity) in dBm
        rx_min_dbm: f64,
        /// Allowed transmit power range in dBm
        tx_min_dbm: f64,
        tx_max_dbm: f64,
    },
    /// Distributed transmission line (RLGC) model.
    /// γ = sqrt((R + jωL)(G + jωC))
    Wired {
        /// Minimum receivable power in dBm
        rx_min_dbm: f64,
        /// Allowed transmit power range in dBm
        tx_min_dbm: f64,
        tx_max_dbm: f64,
        /// Series resistance per unit length (Ω/m)
        r: f64,
        /// Series inductance per unit length (H/m)
        l: f64,
        /// Shunt capacitance per unit length (F/m)
        c: f64,
        /// Shunt conductance per unit length (S/m), dielectric loss
        g: f64,
        /// Signal frequency (Hz)
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
    #[serde(with = "system_time_serde")]
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
    pub expr: String,
    pub noise_floor_dbm: f64,
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
