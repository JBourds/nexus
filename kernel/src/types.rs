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
use config::ast::{self, ChannelEnergy, ChannelType, Cmd, Link, Point, TimestepConfig};
use tracing::instrument;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeIdx(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelIdx(pub usize);

pub type NodeHandle = NodeIdx;
pub type ChannelHandle = ChannelIdx;

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
            let node_handle = NodeIdx(node_handle);
            for protocol in node.protocols.iter() {
                for channel_index in protocol.subscribers.iter().copied() {
                    channels[channel_index.0].subscribers.insert(node_handle);
                }
                for channel_index in protocol.publishers.iter().copied() {
                    channels[channel_index.0].publishers.insert(node_handle);
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

/// A runtime power flow: either a fixed rate or a time-varying piecewise
/// linear schedule. All values are pre-converted to nJ-per-timestep.
#[derive(Clone, Debug)]
pub enum PowerFlowState {
    /// Constant rate, pre-converted to nJ per timestep.
    Constant { nj_per_ts: u64 },
    /// Piecewise-linear schedule with pre-converted nJ-per-timestep values.
    /// `breakpoints`: `(time_us, nj_per_ts_at_this_rate)` sorted by time.
    PiecewiseLinear {
        breakpoints: Vec<(u64, u64)>,
        repeat_us: Option<u64>,
    },
}

impl PowerFlowState {
    /// Evaluate this flow at the given simulation time in microseconds.
    pub fn nj_per_timestep(&self, current_time_us: u64) -> u64 {
        match self {
            Self::Constant { nj_per_ts } => *nj_per_ts,
            Self::PiecewiseLinear {
                breakpoints,
                repeat_us,
            } => {
                if breakpoints.is_empty() {
                    return 0;
                }
                let t = match repeat_us {
                    Some(period) if *period > 0 => current_time_us % period,
                    _ => current_time_us,
                };
                // Binary search: find first breakpoint with time > t
                let idx = breakpoints.partition_point(|(time, _)| *time <= t);
                if idx == 0 {
                    return breakpoints[0].1;
                }
                if idx >= breakpoints.len() {
                    return breakpoints.last().unwrap().1;
                }
                // Interpolate between breakpoints[idx-1] and breakpoints[idx]
                let (t0, r0) = breakpoints[idx - 1];
                let (t1, r1) = breakpoints[idx];
                if t1 == t0 {
                    return r1;
                }
                let frac = (t - t0) as f64 / (t1 - t0) as f64;
                let result = r0 as f64 + (r1 as f64 - r0 as f64) * frac;
                result.max(0.0) as u64
            }
        }
    }

    /// Create from an AST `PowerFlow` definition, pre-converting rates to nJ/ts.
    pub fn from_ast(flow: &ast::PowerFlow, timestep_ns: u64) -> Self {
        match flow {
            ast::PowerFlow::Constant(rate) => Self::Constant {
                nj_per_ts: rate.nj_per_timestep(timestep_ns),
            },
            ast::PowerFlow::PiecewiseLinear {
                unit,
                time,
                breakpoints,
                repeat_us,
            } => {
                let nw_factor = unit.to_nw_factor();
                let time_ns = time.to_ns_factor();
                let breakpoints = breakpoints
                    .iter()
                    .map(|&(t_us, rate)| {
                        let nj = (rate as u128) * (nw_factor as u128) * (timestep_ns as u128)
                            / (time_ns as u128);
                        let nj = nj as u64;
                        (t_us, nj)
                    })
                    .collect();
                Self::PiecewiseLinear {
                    breakpoints,
                    repeat_us: *repeat_us,
                }
            }
        }
    }
}

/// Runtime energy tracking state for a node with a battery.
#[derive(Clone, Debug)]
pub struct EnergyState {
    /// Current charge in nanojoules. Saturates at 0 (node dead when == 0).
    pub charge_nj: u64,
    /// Maximum capacity in nanojoules.
    pub max_nj: u64,
    /// Named power sources (e.g. solar), applied every timestep.
    pub power_sources: Vec<(String, PowerFlowState)>,
    /// Named power sinks (e.g. MCU baseline), applied every timestep.
    pub power_sinks: Vec<(String, PowerFlowState)>,
    /// Per-timestep drain in nJ for each named power state.
    pub power_states_nj: HashMap<String, u64>,
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
        let charge_nj = charge.unit.to_nj(charge.quantity);
        let timestep_ns = ts_config.length.get() * ts_config.unit.to_ns_factor();
        let power_sources = node
            .power_sources
            .iter()
            .map(|(name, flow)| (name.clone(), PowerFlowState::from_ast(flow, timestep_ns)))
            .collect();
        let power_sinks = node
            .power_sinks
            .iter()
            .map(|(name, flow)| (name.clone(), PowerFlowState::from_ast(flow, timestep_ns)))
            .collect();
        let power_states_nj = node
            .power_states
            .iter()
            .map(|(name, rate)| (name.clone(), rate.nj_per_timestep(timestep_ns)))
            .collect();
        let restart_threshold_nj = node.restart_threshold.map(|t| (t * max_nj as f64) as u64);
        let is_dead = charge_nj == 0;
        Some(EnergyState {
            charge_nj,
            max_nj,
            power_sources,
            power_sinks,
            power_states_nj,
            current_state: node.initial_state.clone(),
            restart_threshold_nj,
            is_dead,
        })
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
    ///
    /// `us_per_step` converts the raw step counter into microseconds so that
    /// velocities (dist_unit/µs) and durations (µs) work correctly.
    pub fn current_point(&self, timestep: u64, us_per_step: u64) -> Option<Point> {
        match self {
            Self::Static => None,
            Self::Velocity {
                initial,
                velocity,
                start_ts,
            } => {
                let dt_us = (timestep.saturating_sub(*start_ts) * us_per_step) as f64;
                Some(Point {
                    x: initial.x + velocity.x * dt_us,
                    y: initial.y + velocity.y * dt_us,
                    z: initial.z + velocity.z * dt_us,
                })
            }
            Self::Linear {
                start,
                end,
                start_ts,
                duration_us,
            } => {
                let dt_us = (timestep.saturating_sub(*start_ts) * us_per_step) as f64;
                let t = (dt_us / *duration_us as f64).min(1.0);
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
                let dt_us = (timestep.saturating_sub(*start_ts) * us_per_step) as f64;
                let angle = (start_angle_deg + angular_vel_deg_per_us * dt_us).to_radians();
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
                end, duration_us, ..
            } => {
                format!("linear {} {} {} {}", end.x, end.y, end.z, duration_us)
            }
            Self::Circle {
                center,
                radius,
                angular_vel_deg_per_us,
                ..
            } => {
                format!(
                    "circle {} {} {} {} {}",
                    center.x, center.y, center.z, radius, angular_vel_deg_per_us
                )
            }
        }
    }
}

/// The kernel-usable form of a node which includes its simulation state used
/// by control files.
#[derive(Clone, Debug)]
pub struct Node {
    pub energy: Option<EnergyState>,
    pub position: ast::Position,
    pub motion: MotionPattern,
    pub start: SystemTime,
    pub protocols: Vec<NodeProtocol>,
    /// Per-channel energy costs keyed by integer channel handle.
    pub channel_energy: HashMap<ChannelHandle, ChannelEnergy>,
}

#[derive(Clone, Debug)]
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
        node_handles: &HashMap<ast::NodeHandle, NodeHandle>,
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
                        .map(|(name, handle)| (name, ChannelIdx(handle + channel_handles.len()))),
                )
                .collect::<HashMap<ast::ChannelHandle, ChannelHandle>>()
        } else {
            channel_handles
        };

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

        let (_, protocols) = unzip(node.protocols);
        let protocols = protocols
            .into_iter()
            .map(|protocol| NodeProtocol::from_ast(protocol, handle, channel_handles, node_handles))
            .collect::<Result<_, ConversionError>>()?;
        Ok((
            Self {
                protocols,
                energy,
                channel_energy,
                start: node.start,
                position: node.position,
                motion: MotionPattern::Static,
            },
            new_handles,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64) -> Point {
        Point { x, y, z }
    }

    fn assert_point_near(actual: Point, expected: Point, eps: f64) {
        assert!(
            (actual.x - expected.x).abs() < eps
                && (actual.y - expected.y).abs() < eps
                && (actual.z - expected.z).abs() < eps,
            "expected ({}, {}, {}), got ({}, {}, {})",
            expected.x,
            expected.y,
            expected.z,
            actual.x,
            actual.y,
            actual.z,
        );
    }

    // Static

    #[test]
    fn static_returns_none() {
        assert!(MotionPattern::Static.current_point(0, 1).is_none());
        assert!(MotionPattern::Static.current_point(1_000_000, 1).is_none());
    }

    // Velocity

    #[test]
    fn velocity_at_start_returns_initial() {
        let m = MotionPattern::Velocity {
            initial: pt(1.0, 2.0, 3.0),
            velocity: pt(0.5, -0.5, 0.0),
            start_ts: 100,
        };
        let p = m.current_point(100, 1).unwrap();
        assert_point_near(p, pt(1.0, 2.0, 3.0), 1e-12);
    }

    #[test]
    fn velocity_linear_displacement() {
        let m = MotionPattern::Velocity {
            initial: pt(0.0, 0.0, 0.0),
            velocity: pt(1.0, 2.0, 3.0),
            start_ts: 0,
        };
        let p = m.current_point(10, 1).unwrap();
        assert_point_near(p, pt(10.0, 20.0, 30.0), 1e-12);
    }

    #[test]
    fn velocity_before_start_saturates_to_initial() {
        let m = MotionPattern::Velocity {
            initial: pt(5.0, 5.0, 5.0),
            velocity: pt(1.0, 1.0, 1.0),
            start_ts: 100,
        };
        // timestep 50 < start_ts 100 → saturating_sub gives 0
        let p = m.current_point(50, 1).unwrap();
        assert_point_near(p, pt(5.0, 5.0, 5.0), 1e-12);
    }

    // Linear

    #[test]
    fn linear_at_start_returns_start_point() {
        let m = MotionPattern::Linear {
            start: pt(0.0, 0.0, 0.0),
            end: pt(10.0, 0.0, 0.0),
            start_ts: 0,
            duration_us: 100,
        };
        let p = m.current_point(0, 1).unwrap();
        assert_point_near(p, pt(0.0, 0.0, 0.0), 1e-12);
    }

    #[test]
    fn linear_midpoint() {
        let m = MotionPattern::Linear {
            start: pt(0.0, 0.0, 0.0),
            end: pt(10.0, 20.0, 0.0),
            start_ts: 0,
            duration_us: 100,
        };
        let p = m.current_point(50, 1).unwrap();
        assert_point_near(p, pt(5.0, 10.0, 0.0), 1e-12);
    }

    #[test]
    fn linear_clamps_at_end() {
        let m = MotionPattern::Linear {
            start: pt(0.0, 0.0, 0.0),
            end: pt(10.0, 0.0, 0.0),
            start_ts: 0,
            duration_us: 100,
        };
        // Well past duration
        let p = m.current_point(500, 1).unwrap();
        assert_point_near(p, pt(10.0, 0.0, 0.0), 1e-12);
    }

    #[test]
    fn linear_3d_interpolation() {
        let m = MotionPattern::Linear {
            start: pt(1.0, 2.0, 3.0),
            end: pt(5.0, 6.0, 7.0),
            start_ts: 10,
            duration_us: 40,
        };
        // t=30 → dt=20, frac=0.5
        let p = m.current_point(30, 1).unwrap();
        assert_point_near(p, pt(3.0, 4.0, 5.0), 1e-12);
    }

    // Circle

    #[test]
    fn circle_at_start_returns_initial_point() {
        // Node at (10, 0, 0) orbiting center (0, 0, 0) → start_angle = 0°
        let m = MotionPattern::Circle {
            center: pt(0.0, 0.0, 0.0),
            radius: 10.0,
            start_angle_deg: 0.0,
            angular_vel_deg_per_us: 1.0,
            start_ts: 0,
        };
        let p = m.current_point(0, 1).unwrap();
        assert_point_near(p, pt(10.0, 0.0, 0.0), 1e-9);
    }

    #[test]
    fn circle_quarter_turn() {
        // 90° at start → at (0, r, 0)
        let m = MotionPattern::Circle {
            center: pt(0.0, 0.0, 0.0),
            radius: 5.0,
            start_angle_deg: 0.0,
            angular_vel_deg_per_us: 1.0,
            start_ts: 0,
        };
        let p = m.current_point(90, 1).unwrap();
        assert_point_near(p, pt(0.0, 5.0, 0.0), 1e-9);
    }

    #[test]
    fn circle_preserves_z() {
        let m = MotionPattern::Circle {
            center: pt(0.0, 0.0, 42.0),
            radius: 1.0,
            start_angle_deg: 0.0,
            angular_vel_deg_per_us: 1.0,
            start_ts: 0,
        };
        let p = m.current_point(180, 1).unwrap();
        assert!((p.z - 42.0).abs() < 1e-12);
    }

    #[test]
    fn circle_full_revolution_returns_to_start() {
        let m = MotionPattern::Circle {
            center: pt(0.0, 0.0, 0.0),
            radius: 7.0,
            start_angle_deg: 45.0,
            angular_vel_deg_per_us: 1.0,
            start_ts: 0,
        };
        let start = m.current_point(0, 1).unwrap();
        let after_360 = m.current_point(360, 1).unwrap();
        assert_point_near(after_360, start, 1e-9);
    }

    // PowerFlowState

    #[test]
    fn piecewise_empty_breakpoints_returns_zero() {
        let flow = PowerFlowState::PiecewiseLinear {
            breakpoints: vec![],
            repeat_us: None,
        };
        assert_eq!(flow.nj_per_timestep(0), 0);
        assert_eq!(flow.nj_per_timestep(1000), 0);
    }

    #[test]
    fn piecewise_single_breakpoint() {
        let flow = PowerFlowState::PiecewiseLinear {
            breakpoints: vec![(0, 42)],
            repeat_us: None,
        };
        assert_eq!(flow.nj_per_timestep(0), 42);
        assert_eq!(flow.nj_per_timestep(9999), 42);
    }

    #[test]
    fn piecewise_interpolates() {
        let flow = PowerFlowState::PiecewiseLinear {
            breakpoints: vec![(0, 100), (100, 200)],
            repeat_us: None,
        };
        assert_eq!(flow.nj_per_timestep(50), 150);
    }

    #[test]
    fn piecewise_repeats() {
        let flow = PowerFlowState::PiecewiseLinear {
            breakpoints: vec![(0, 0), (100, 100)],
            repeat_us: Some(100),
        };
        // t=150 → 150 % 100 = 50 → interpolate to 50
        assert_eq!(flow.nj_per_timestep(150), 50);
    }

    // to_spec

    #[test]
    fn static_to_spec() {
        assert_eq!(MotionPattern::Static.to_spec(), "none");
    }

    #[test]
    fn velocity_to_spec() {
        let m = MotionPattern::Velocity {
            initial: pt(0.0, 0.0, 0.0),
            velocity: pt(1.5, -2.0, 0.0),
            start_ts: 0,
        };
        assert_eq!(m.to_spec(), "velocity 1.5 -2 0");
    }

    #[test]
    fn linear_to_spec() {
        let m = MotionPattern::Linear {
            start: pt(0.0, 0.0, 0.0),
            end: pt(10.0, 20.0, 30.0),
            start_ts: 0,
            duration_us: 1000,
        };
        assert_eq!(m.to_spec(), "linear 10 20 30 1000");
    }

    #[test]
    fn circle_to_spec() {
        let m = MotionPattern::Circle {
            center: pt(1.0, 2.0, 3.0),
            radius: 5.0,
            start_angle_deg: 0.0,
            angular_vel_deg_per_us: 0.5,
            start_ts: 0,
        };
        assert_eq!(m.to_spec(), "circle 1 2 3 5 0.5");
    }
}

impl NodeProtocol {
    #[instrument]
    pub(super) fn from_ast(
        node: ast::NodeProtocol,
        handle: NodeHandle,
        channel_handles: &HashMap<ast::ChannelHandle, ChannelHandle>,
        node_handles: &HashMap<ast::NodeHandle, NodeHandle>,
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
