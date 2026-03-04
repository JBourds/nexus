//! This module translates the `config` crate's AST types into ones better
//! suited for high performance simulation and augments them with kernel
//! specific functionality.
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::SystemTime,
};

use crate::helpers::unzip;
use crate::{errors::ConversionError, helpers::make_handles};
use config::ast::{self, ChannelEnergy, ChannelType, Cmd, Link, TimestepConfig};
use tracing::instrument;

pub type ChannelHandle = usize;
pub type NodeHandle = usize;

#[derive(Debug)]
pub struct Channel {
    #[allow(unused)]
    pub link: Link,
    #[allow(unused)]
    pub r#type: ChannelType,
    pub subscribers: HashSet<NodeHandle>,
    pub publishers: HashSet<NodeHandle>,
}

impl Channel {
    /// Combine top-level channels and internal channels into one vector
    /// which can be indexed by the usize handles from the resolver.
    ///
    /// * `channels`: Top-level channels from config.
    /// * `internal_channels`: All internal channels created from nodes.
    /// * `nodes`: Set of nodes to use when constructing pub/sub lists.
    #[instrument]
    pub(super) fn from_ast(
        channels: Vec<ast::Channel>,
        internal_channels: Vec<Self>,
        nodes: &[Node],
    ) -> Result<Vec<Self>, ConversionError> {
        let mut channels = channels
            .into_iter()
            .map(|ch| Channel {
                link: ch.link,
                r#type: ch.r#type,
                subscribers: HashSet::new(),
                publishers: HashSet::new(),
            })
            .chain(internal_channels.into_iter())
            .collect::<Vec<_>>();
        for (node_handle, node) in nodes.iter().enumerate() {
            for protocol in node.protocols.iter() {
                for channel_index in protocol.subscribers.iter().copied() {
                    channels[channel_index].subscribers.insert(node_handle);
                }
                for channel_index in protocol.publishers.iter().copied() {
                    channels[channel_index].publishers.insert(node_handle);
                }
            }
        }
        Ok(channels)
    }

    pub(super) fn new_internal(handle: NodeHandle) -> Self {
        let set = HashSet::from([handle]);
        Self {
            link: Link::default(),
            r#type: ChannelType::new_internal(),
            subscribers: set.clone(),
            publishers: set,
        }
    }
}

/// Runtime energy tracking state for a node with a battery.
#[derive(Clone, Debug)]
pub struct EnergyState {
    /// Current charge in nanojoules. Can go negative (node dead when <= 0).
    pub charge_nj: i64,
    /// Maximum capacity in nanojoules.
    pub max_nj: u64,
    /// Per-timestep ambient generation in nJ (positive = generates energy).
    pub ambient_nj_per_ts: i64,
    /// Per-timestep drain in nJ for each named power state (positive = drains).
    pub power_states_nj: HashMap<String, i64>,
    /// Currently active power state.
    pub current_state: Option<String>,
    /// Charge level in nJ at which a dead node is restarted.
    pub restart_threshold_nj: Option<u64>,
    /// Whether this node is currently dead (charge depleted, waiting to recover).
    pub is_dead: bool,
}

impl EnergyState {
    pub fn from_node(node: &ast::Node, ts_config: &TimestepConfig) -> Option<Self> {
        let charge = node.charge.as_ref()?;
        let max_nj = charge.unit.to_nj(charge.max);
        let charge_nj = charge.unit.to_nj(charge.quantity) as i64;
        let timestep_ns = ts_config.length.get() as i64 * ts_config.unit.to_ns_factor();
        let ambient_nj_per_ts = node
            .ambient_rate
            .as_ref()
            .map_or(0, |r| r.nj_per_timestep(timestep_ns));
        let power_states_nj = node
            .power_states
            .iter()
            .map(|(name, rate)| (name.clone(), rate.nj_per_timestep(timestep_ns)))
            .collect();
        let restart_threshold_nj = node.restart_threshold.map(|t| (t * max_nj as f64) as u64);
        let is_dead = charge_nj <= 0;
        Some(EnergyState {
            charge_nj,
            max_nj,
            ambient_nj_per_ts,
            power_states_nj,
            current_state: node.initial_state.clone(),
            restart_threshold_nj,
            is_dead,
        })
    }
}

/// The kernel-usable form of a node which includes its simulation state used
/// by control files.
#[derive(Clone, Debug)]
pub struct Node {
    pub energy: Option<EnergyState>,
    pub position: ast::Position,
    pub start: SystemTime,
    pub protocols: Vec<NodeProtocol>,
}

#[derive(Clone, Debug)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub runner: Cmd,
    pub subscribers: HashSet<ChannelHandle>,
    pub publishers: HashSet<ChannelHandle>,
    /// Per-channel energy costs keyed by integer channel handle.
    pub channel_energy: HashMap<ChannelHandle, ChannelEnergy>,
}

impl Node {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::Node,
        handle: NodeHandle,
        channel_handles: &HashMap<ast::ChannelHandle, ChannelHandle>,
        node_handles: &HashMap<ast::NodeHandle, ChannelHandle>,
        ts_config: &TimestepConfig,
    ) -> Result<(Self, Vec<(ast::ChannelHandle, Channel)>), ConversionError> {
        // Compute energy state before moving any fields out of node.
        let energy = EnergyState::from_node(&node, ts_config);

        // Internal have their own namespace, copy the hashmap
        // and overwrite any existing links with internal names.
        let new_handles = node
            .internal_names
            .clone()
            .into_iter()
            .map(|name| (name, Channel::new_internal(handle)))
            .collect::<Vec<_>>();
        let channel_handles = if !new_handles.is_empty() {
            &channel_handles
                .clone()
                .into_iter()
                .chain(
                    make_handles(node.internal_names)
                        .into_iter()
                        .map(|(name, handle)| (name, handle + channel_handles.len())),
                )
                .collect::<HashMap<ast::ChannelHandle, ChannelHandle>>()
        } else {
            channel_handles
        };

        let (_, protocols) = unzip(node.protocols);
        let protocols = protocols
            .into_iter()
            .map(|protocol| NodeProtocol::from_ast(protocol, handle, channel_handles, node_handles))
            .collect::<Result<_, ConversionError>>()?;
        Ok((
            Self {
                protocols,
                energy,
                start: node.start,
                position: node.position,
            },
            new_handles,
        ))
    }
}

impl NodeProtocol {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::NodeProtocol,
        handle: NodeHandle,
        channel_handles: &HashMap<ast::ChannelHandle, ChannelHandle>,
        node_handles: &HashMap<ast::NodeHandle, ChannelHandle>,
    ) -> Result<Self, ConversionError> {
        let map_channel_handles =
            |handles: HashSet<ast::ChannelHandle>| -> Result<_, ConversionError> {
                handles
                    .into_iter()
                    .map(|name| {
                        channel_handles
                            .get(&name)
                            .copied()
                            .ok_or(ConversionError::ChannelHandleConversion(name))
                    })
                    .collect::<Result<_, ConversionError>>()
            };
        let subscribers = map_channel_handles(node.subscribers)?;
        let publishers = map_channel_handles(node.publishers)?;
        let channel_energy = node
            .channel_energy
            .into_iter()
            .map(|(name, energy)| {
                channel_handles
                    .get(&name)
                    .copied()
                    .ok_or(ConversionError::ChannelHandleConversion(name))
                    .map(|ch| (ch, energy))
            })
            .collect::<Result<_, ConversionError>>()?;
        Ok(Self {
            root: node.root,
            runner: node.runner,
            subscribers,
            publishers,
            channel_energy,
        })
    }
}
