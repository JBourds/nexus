use serde::Deserialize;
use std::{collections::HashMap, num::NonZeroU64};
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
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Unit(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimestepConfig {
    pub(super) length: Option<u64>,
    pub(super) unit: Option<Unit>,
    pub(super) count: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DistanceTimeVar {
    pub(super) rate: Option<meval::Expr>,
    pub(super) time: Option<Unit>,
    pub(super) distance: Option<Unit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DistanceProbVar {
    pub(super) rate: Option<meval::Expr>,
    pub(super) distance: Option<Unit>,
    pub(super) size: Option<Unit>,
}

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
    pub(super) quantity: u64,
    pub(super) unit: Unit,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerSink {
    pub(super) name: String,
    pub(super) quantity: u64,
    pub(super) unit: Unit,
    pub(super) time: Unit,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerSource {
    pub(super) name: String,
    pub(super) quantity: u64,
    pub(super) unit: Unit,
    pub(super) time: Unit,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Link {
    pub(super) inherit: Option<String>,
    pub(super) signal: Option<Signal>,
    pub(super) packet_loss: Option<DistanceProbVar>,
    pub(super) bit_error: Option<DistanceProbVar>,
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
        max_size: Option<NonZeroU64>,
        read_own_writes: Option<bool>,
    },
    Exclusive {
        ttl: Option<NonZeroU64>,
        unit: Option<Unit>,
        max_size: Option<NonZeroU64>,
        read_own_writes: Option<bool>,
        nbuffered: Option<NonZeroU64>,
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

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Signal {
    pub(super) max_range: Option<f64>,
    pub(super) offset: Option<f64>,
    pub(super) shape: Option<SignalShape>,
    pub(super) unit: Option<Unit>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProtocolName(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Deployment {
    pub(super) position: Option<Coordinate>,
    pub(super) extra_args: Option<Vec<String>>,
    pub(super) charge: Option<Charge>,
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
    pub(super) sources: Option<Vec<PowerSource>>,
    pub(super) sinks: Option<Vec<PowerSink>>,
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
}
