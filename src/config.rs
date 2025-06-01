use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;

pub fn parse(text: String) -> Result<Simulation> {
    toml::from_str(text.as_str()).context("Failed to parse simulation parameters from config file.")
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Simulation {
    params: Params,
    links: HashMap<String, Link>,
    nodes: HashMap<String, Node>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Params {
    timestep_length: f32,
    timesteps: u64,
    seed: u16,
    root: String,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            timestep_length: 0.01,
            timesteps: 1_000_000,
            seed: 42,
            root: String::from("~/testnet/simulations"),
        }
    }
}

type Modifier = String;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Delay {
    modifier: Modifier,
    avg: f32,
    std: f32,
}

impl Default for Delay {
    fn default() -> Self {
        Self {
            modifier: String::from("flat"),
            avg: 0.0,
            std: 0.0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct LinkName(String);

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Link {
    inherit: Option<String>,
    next: Option<String>,
    bit_error: f32,
    intermediaries: u32,
    packet_loss: f32,
    packet_loss_mod: Modifier,
    trans_rate: f64,
    queue_delay: Delay,
    processing_delay: Delay,
    connection_delay: Delay,
    propagation_delay: Delay,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            inherit: None,
            next: None,
            bit_error: 0.0,
            intermediaries: 0,
            packet_loss: 0.0,
            packet_loss_mod: Modifier::from("flat"),
            trans_rate: f64::INFINITY,
            queue_delay: Delay::default(),
            processing_delay: Delay::default(),
            connection_delay: Delay::default(),
            propagation_delay: Delay::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct NodeName(String);

#[derive(Debug, Default, Deserialize)]
pub struct ProtocolName(String);

#[derive(Debug, Deserialize)]
pub struct Position {
    x: i64,
    y: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct IndirectConnection {
    max_range: u64,
    modifier_offset: u64,
    link: LinkName,
}

#[derive(Debug, Default, Deserialize)]
pub struct DirectConnection {
    node: NodeName,
    link: LinkName,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Node {
    positions: Vec<Position>,
    internal_names: Vec<ProtocolName>,
    protocols: Vec<NodeProtocol>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct NodeProtocol {
    name: String,
    root: String,
    runner: String,
    accepts: Vec<ProtocolName>,
    direct: Vec<DirectConnection>,
    indirect: Vec<IndirectConnection>,
}
