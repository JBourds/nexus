use serde::Deserialize;
use std::collections::HashMap;
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Simulation {
    pub(super) params: Params,
    pub(super) links: HashMap<String, Link>,
    pub(super) nodes: HashMap<String, Node>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub(super) timestep: Option<TimestepConfig>,
    pub(super) seed: Option<u64>,
    pub(super) root: String,
}

#[derive(Debug, Default, Deserialize)]
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
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rate {
    pub(super) rate: Option<u64>,
    pub(super) data: Option<Unit>,
    pub(super) time: Option<Unit>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LinkName(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Link {
    pub(super) inherit: Option<String>,
    pub(super) signal: Option<Signal>,
    pub(super) transmission: Option<Rate>,
    pub(super) packet_loss: Option<DistanceProbVar>,
    pub(super) bit_error: Option<DistanceProbVar>,
    pub(super) delays: Option<Delays>,
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
pub struct NodeName(pub String);

#[derive(Debug, Default, Deserialize)]
pub struct ProtocolName(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Deployment {
    pub(super) coordinates: Option<Coordinate>,
    pub(super) extra_args: Option<Vec<String>>,
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
pub struct DirectConnection {
    pub(super) node: NodeName,
    pub(super) link: LinkName,
}

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct IndirectConnection(pub String);

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Node {
    pub(super) deployments: Option<Vec<Deployment>>,
    pub(super) internal_names: Option<Vec<ProtocolName>>,
    pub(super) protocols: Option<Vec<NodeProtocol>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NodeProtocol {
    pub(super) name: String,
    pub(super) root: String,
    pub(super) runner: String,
    pub(super) runner_args: Option<Vec<String>>,
    pub(super) accepts: Option<Vec<LinkName>>,
    pub(super) direct: Option<Vec<DirectConnection>>,
    pub(super) indirect: Option<Vec<LinkName>>,
}
