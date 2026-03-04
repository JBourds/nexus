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
use config::ast::{self, ChannelType, Charge, Cmd, Link, Point};
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

/// Describes how a node's position evolves over time.
///
/// All coordinates are in the node's configured `DistanceUnit`. Velocities and
/// angular rates are per-microsecond (matching the simulator timestep unit).
#[derive(Clone, Debug, Default)]
pub enum MotionPattern {
    /// Node remains stationary at its current `position.point`.
    #[default]
    Static,
    /// Constant-velocity rectilinear motion starting from `initial` at
    /// simulation timestep `start_ts`. `velocity` components are in
    /// dist_unit per microsecond.
    Velocity {
        initial: Point,
        velocity: Point,
        start_ts: u64,
    },
    /// Linear interpolation from `start` to `end` over `duration_us`
    /// microseconds. The node stops at `end` once the duration elapses.
    Linear {
        start: Point,
        end: Point,
        start_ts: u64,
        duration_us: u64,
    },
    /// Circular orbit in the XY plane. `start_angle_deg` is the initial
    /// azimuth (degrees) at `start_ts`; `angular_vel_deg_per_us` is the
    /// rate of rotation (positive = counter-clockwise). The Z coordinate
    /// stays fixed at `center.z`.
    Circle {
        center: Point,
        radius: f64,
        start_angle_deg: f64,
        angular_vel_deg_per_us: f64,
        start_ts: u64,
    },
}

impl MotionPattern {
    /// Compute the position point at `timestep` from this pattern.
    /// Returns `None` for `Static` (no update needed).
    pub fn current_point(&self, timestep: u64) -> Option<Point> {
        match self {
            Self::Static => None,
            Self::Velocity {
                initial,
                velocity,
                start_ts,
            } => {
                let dt = timestep.saturating_sub(*start_ts) as f64;
                Some(Point {
                    x: initial.x + velocity.x * dt,
                    y: initial.y + velocity.y * dt,
                    z: initial.z + velocity.z * dt,
                })
            }
            Self::Linear {
                start,
                end,
                start_ts,
                duration_us,
            } => {
                let dt = timestep.saturating_sub(*start_ts) as f64;
                let t = (dt / *duration_us as f64).min(1.0);
                Some(Point {
                    x: start.x + (end.x - start.x) * t,
                    y: start.y + (end.y - start.y) * t,
                    z: start.z + (end.z - start.z) * t,
                })
            }
            Self::Circle {
                center,
                radius,
                start_angle_deg,
                angular_vel_deg_per_us,
                start_ts,
            } => {
                let dt = timestep.saturating_sub(*start_ts) as f64;
                let angle =
                    (start_angle_deg + angular_vel_deg_per_us * dt).to_radians();
                Some(Point {
                    x: center.x + radius * angle.cos(),
                    y: center.y + radius * angle.sin(),
                    z: center.z,
                })
            }
        }
    }

    /// Serialize this pattern to the text format used by `ctl.pos.motion`.
    pub fn to_spec(&self) -> String {
        match self {
            Self::Static => "none".to_string(),
            Self::Velocity { velocity, .. } => {
                format!("velocity {} {} {}", velocity.x, velocity.y, velocity.z)
            }
            Self::Linear {
                start,
                end,
                duration_us,
                ..
            } => {
                format!(
                    "linear {} {} {} {} {} {} {}",
                    start.x, start.y, start.z,
                    end.x, end.y, end.z,
                    duration_us
                )
            }
            Self::Circle {
                center,
                radius,
                angular_vel_deg_per_us,
                ..
            } => {
                format!(
                    "circle {} {} {} {} {}",
                    center.x, center.y, center.z,
                    radius,
                    angular_vel_deg_per_us
                )
            }
        }
    }
}

/// The kernel-usable form of a node which includes its simulation state used
/// by control files.
#[derive(Clone, Debug)]
#[allow(unused)]
pub struct Node {
    pub charge: Option<Charge>,
    pub position: ast::Position,
    pub motion: MotionPattern,
    pub start: SystemTime,
    pub protocols: Vec<NodeProtocol>,
}

#[derive(Clone, Debug)]
#[allow(unused)]
pub struct NodeProtocol {
    pub root: PathBuf,
    pub runner: Cmd,
    pub subscribers: HashSet<ChannelHandle>,
    pub publishers: HashSet<ChannelHandle>,
}

impl Node {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::Node,
        handle: NodeHandle,
        channel_handles: &HashMap<ast::ChannelHandle, ChannelHandle>,
        node_handles: &HashMap<ast::NodeHandle, ChannelHandle>,
    ) -> Result<(Self, Vec<(ast::ChannelHandle, Channel)>), ConversionError> {
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
                charge: node.charge,
                start: node.start,
                position: node.position,
                motion: MotionPattern::Static,
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
        Ok(Self {
            root: node.root,
            runner: node.runner,
            subscribers,
            publishers,
        })
    }
}
