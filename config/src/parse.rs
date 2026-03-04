use serde::Deserialize;
use std::{
    collections::HashMap,
    num::{NonZeroU64, NonZeroUsize},
};
use toml::value::Datetime;
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Simulation {
    pub(super) params: Params,
    pub(super) links: HashMap<String, Link>,
    pub(super) nodes: HashMap<String, Node>,
    pub(super) channels: HashMap<String, Channel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub(super) timestep: Option<TimestepConfig>,
    pub(super) seed: Option<u64>,
    pub(super) root: String,
    pub(super) time_dilation: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Unit(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimestepConfig {
    pub(super) length: Option<u64>,
    pub(super) unit: Option<Unit>,
    pub(super) count: Option<u64>,
    pub(super) start: Option<Datetime>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DistanceTimeVar {
    pub(super) rate: Option<String>,
    pub(super) time: Option<Unit>,
    pub(super) distance: Option<Unit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RssiProbExpr(pub Option<String>);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rate {
    pub(super) rate: Option<u64>,
    pub(super) data: Option<Unit>,
    pub(super) time: Option<Unit>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LinkName(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Charge {
    pub(super) max: Option<u64>,
    pub(super) quantity: u64,
    pub(super) unit: Unit,
}

/// A power rate used for per-node power states and ambient rates.
/// `rate` is always positive; semantics (consumption vs. generation) are
/// determined by the field it appears in.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerRate {
    pub(super) rate: u64,
    pub(super) unit: Unit,
    pub(super) time: Unit,
}

/// One-time energy cost (e.g. per TX or RX on a channel).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Energy {
    pub(super) quantity: u64,
    pub(super) unit: Unit,
}

/// Optional TX/RX energy costs for a single channel within a protocol.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChannelEnergy {
    pub(super) tx: Option<Energy>,
    pub(super) rx: Option<Energy>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Link {
    pub(super) inherit: Option<String>,
    pub(super) medium: Option<Medium>,
    pub(super) packet_loss: Option<RssiProbExpr>,
    pub(super) bit_error: Option<RssiProbExpr>,
    pub(super) delays: Option<Delays>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ChannelName(pub String);

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Channel {
    pub(super) link: Option<LinkName>,
    pub(super) r#type: ChannelType,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "type")]
pub enum ChannelType {
    Shared {
        ttl: Option<NonZeroU64>,
        unit: Option<Unit>,
        max_size: Option<NonZeroUsize>,
        read_own_writes: Option<bool>,
    },
    Exclusive {
        ttl: Option<NonZeroU64>,
        unit: Option<Unit>,
        max_size: Option<NonZeroUsize>,
        read_own_writes: Option<bool>,
        nbuffered: Option<NonZeroUsize>,
    },
}

impl Default for ChannelType {
    fn default() -> Self {
        Self::Exclusive {
            ttl: None,
            unit: None,
            nbuffered: None,
            max_size: None,
            read_own_writes: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Delays {
    pub(super) transmission: Option<Rate>,
    pub(super) processing: Option<Rate>,
    pub(super) propagation: Option<DistanceTimeVar>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SignalShape(pub String);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "type")]
pub enum Medium {
    Wireless {
        shape: Option<SignalShape>,
        wavelength_meters: f64,
        gain_dbi: f64,
        rx_min_dbm: f64,
        tx_min_dbm: f64,
        tx_max_dbm: f64,
    },
    Wired {
        rx_min_dbm: f64,
        tx_min_dbm: f64,
        tx_max_dbm: f64,
        r: f64,
        l: f64,
        c: f64,
        g: f64,
        f: f64,
    },
}

#[derive(Debug, Default, Deserialize)]
pub struct ProtocolName(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Deployment {
    pub(super) position: Option<Coordinate>,
    pub(super) build_args: Option<Vec<String>>,
    pub(super) run_args: Option<Vec<String>>,
    pub(super) charge: Option<Charge>,
    /// Which power state to start in (references a key in `power_states`).
    pub(super) initial_state: Option<String>,
    /// Fraction of max charge [0, 1] at which a dead node restarts.
    pub(super) restart_threshold: Option<f64>,
    /// Optionally let a deployment start with a different time than simulation
    pub(super) start: Option<Datetime>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Point {
    pub(super) x: Option<f64>,
    pub(super) y: Option<f64>,
    pub(super) z: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Orientation {
    pub(super) az: Option<f64>,
    pub(super) el: Option<f64>,
    pub(super) roll: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Coordinate {
    pub(super) point: Option<Point>,
    pub(super) orientation: Option<Orientation>,
    pub(super) unit: Option<Unit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Resources {
    pub(super) clock_rate: Option<NonZeroU64>,
    pub(super) cores: Option<NonZeroU64>,
    pub(super) clock_units: Option<Unit>,
    pub(super) ram: Option<NonZeroU64>,
    pub(super) ram_units: Option<Unit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Node {
    pub(super) resources: Option<Resources>,
    pub(super) deployments: Option<Vec<Deployment>>,
    pub(super) internal_names: Option<Vec<ProtocolName>>,
    pub(super) protocols: Option<Vec<NodeProtocol>>,
    /// Named power consumption/generation states the process can switch
    /// between via `ctl.energy_state`. Positive rate = consumption.
    pub(super) power_states: Option<HashMap<String, PowerRate>>,
    /// Always-on background power rate. Positive = generation (e.g. solar).
    pub(super) ambient_rate: Option<PowerRate>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NodeProtocol {
    pub(super) name: String,
    pub(super) root: String,
    pub(super) runner: String,
    pub(super) runner_args: Option<Vec<String>>,
    pub(super) build: String,
    pub(super) build_args: Option<Vec<String>>,
    pub(super) publishers: Option<Vec<ChannelName>>,
    pub(super) subscribers: Option<Vec<ChannelName>>,
    /// Per-channel energy costs keyed by channel name.
    pub(super) channel_energy: Option<HashMap<String, ChannelEnergy>>,
}
