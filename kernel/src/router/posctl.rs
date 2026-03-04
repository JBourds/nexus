//! posctl.rs
//! Handlers for `ctl.pos.*` position and motion control files.
//!
//! # File interface
//!
//! ## Absolute position (read/write)
//! - `ctl.pos.x`, `ctl.pos.y`, `ctl.pos.z` — coordinate in the node's
//!   configured `DistanceUnit`, formatted as a decimal float string.
//! - `ctl.pos.az`, `ctl.pos.el`, `ctl.pos.roll` — orientation in degrees.
//!
//! Writing any of these clears the active motion pattern (sets to `Static`).
//!
//! ## Relative offset (write-only)
//! - `ctl.pos.dx`, `ctl.pos.dy`, `ctl.pos.dz` — add a delta to the
//!   corresponding coordinate in the node's distance unit.  The current
//!   position (accounting for any active motion pattern) is snapshotted first,
//!   then the pattern is reset to `Static`.
//!
//! ## Motion pattern (read/write)
//! - `ctl.pos.motion` — read or set an automated motion pattern.
//!
//!   **Read** returns the current pattern spec string (see below).
//!
//!   **Write** accepts one of:
//!   ```text
//!   none
//!   velocity <vx> <vy> <vz>
//!   linear <tx> <ty> <tz> <dur_us>
//!   circle <cx> <cy> <cz> <radius> <angular_vel_deg_per_us>
//!   ```
//!   Coordinates are in the node's configured `DistanceUnit`; the current
//!   node position is snapshotted as the pattern's start before switching.

use config::ast::Point;
use tracing::{Level, event};

use crate::router::{RouterError, RoutingServer};
use crate::types::MotionPattern;

impl RoutingServer {
    /// Apply the current motion pattern to `node_index`, updating
    /// `position.point` to reflect the current timestep.  No-op for `Static`.
    pub(super) fn apply_motion(&mut self, node_index: usize) {
        let timestep = self.timestep;
        let node = &mut self.channels.nodes[node_index];
        if let Some(new_point) = node.motion.current_point(timestep) {
            node.position.point = new_point;
        }
    }

    /// Apply all active motion patterns, update positions, and emit a
    /// "movement" tracing event for every node that moved.  Called at the
    /// start of each simulation step.
    pub(super) fn apply_all_motions_and_log(&mut self) {
        let timestep = self.timestep;
        for (node_idx, node) in self.channels.nodes.iter_mut().enumerate() {
            let Some(new_point) = node.motion.current_point(timestep) else {
                continue;
            };
            node.position.point = new_point;
            let (x, y, z) = (new_point.x, new_point.y, new_point.z);
            let (az, el, roll) = (
                node.position.orientation.az,
                node.position.orientation.el,
                node.position.orientation.roll,
            );
            event!(
                target: "movement",
                Level::INFO,
                timestep,
                node = node_idx as u64,
                x, y, z, az, el, roll
            );
        }
    }

    /// Read handler for `ctl.pos.x/y/z/az/el/roll`.
    pub fn read_pos(
        &mut self,
        node_index: usize,
        mut msg: fuse::Message,
    ) -> Result<(), RouterError> {
        self.apply_motion(node_index);
        let node = &self.channels.nodes[node_index];
        let component = msg
            .id
            .1
            .strip_prefix("ctl.pos.")
            .expect("must be a pos control file");
        let val: f64 = match component {
            "x" => node.position.point.x,
            "y" => node.position.point.y,
            "z" => node.position.point.z,
            "az" => node.position.orientation.az,
            "el" => node.position.orientation.el,
            "roll" => node.position.orientation.roll,
            _ => return Err(RouterError::InvalidString(msg.id.1.into_bytes())),
        };
        msg.data = val.to_string().into_bytes();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    /// Read handler for `ctl.pos.motion`.
    pub fn read_pos_motion(
        &mut self,
        node_index: usize,
        mut msg: fuse::Message,
    ) -> Result<(), RouterError> {
        let spec = self.channels.nodes[node_index].motion.to_spec();
        msg.data = spec.into_bytes();
        self.tx
            .send(fuse::KernelMessage::Exclusive(msg))
            .map_err(RouterError::FuseSendError)
    }

    /// Write handler for `ctl.pos.x/y/z/az/el/roll` (absolute set).
    /// Resets the motion pattern to `Static`.
    pub fn write_pos(
        &mut self,
        node_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        // Snapshot any in-progress motion before overriding.
        self.apply_motion(node_index);
        let s = String::from_utf8_lossy(&msg.data);
        let val: f64 = s
            .trim()
            .parse()
            .map_err(|_| RouterError::InvalidFloat(msg.data.clone()))?;
        let component = msg
            .id
            .1
            .strip_prefix("ctl.pos.")
            .expect("must be a pos control file");
        let node = &mut self.channels.nodes[node_index];
        match component {
            "x" => node.position.point.x = val,
            "y" => node.position.point.y = val,
            "z" => node.position.point.z = val,
            "az" => node.position.orientation.az = val,
            "el" => node.position.orientation.el = val,
            "roll" => node.position.orientation.roll = val,
            _ => return Err(RouterError::InvalidString(msg.data)),
        }
        node.motion = MotionPattern::Static;
        // Emit movement event with updated position.
        let timestep = self.timestep;
        let node = &self.channels.nodes[node_index];
        let (x, y, z) = (
            node.position.point.x,
            node.position.point.y,
            node.position.point.z,
        );
        let (az, el, roll) = (
            node.position.orientation.az,
            node.position.orientation.el,
            node.position.orientation.roll,
        );
        event!(
            target: "movement",
            Level::INFO,
            timestep,
            node = node_index as u64,
            x, y, z, az, el, roll
        );
        Ok(())
    }

    /// Write handler for `ctl.pos.dx/dy/dz` (relative offset).
    /// Snapshots the current (motion-applied) position, adds the delta, and
    /// resets the motion pattern to `Static`.
    pub fn write_pos_delta(
        &mut self,
        node_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        self.apply_motion(node_index);
        let s = String::from_utf8_lossy(&msg.data);
        let val: f64 = s
            .trim()
            .parse()
            .map_err(|_| RouterError::InvalidFloat(msg.data.clone()))?;
        // Strip "ctl.pos.d" to get the axis character ("x", "y", or "z").
        let axis = msg
            .id
            .1
            .strip_prefix("ctl.pos.d")
            .expect("must be a delta pos control file");
        let node = &mut self.channels.nodes[node_index];
        match axis {
            "x" => node.position.point.x += val,
            "y" => node.position.point.y += val,
            "z" => node.position.point.z += val,
            _ => return Err(RouterError::InvalidString(msg.data)),
        }
        node.motion = MotionPattern::Static;
        let timestep = self.timestep;
        let node = &self.channels.nodes[node_index];
        let (x, y, z) = (
            node.position.point.x,
            node.position.point.y,
            node.position.point.z,
        );
        let (az, el, roll) = (
            node.position.orientation.az,
            node.position.orientation.el,
            node.position.orientation.roll,
        );
        event!(
            target: "movement",
            Level::INFO,
            timestep,
            node = node_index as u64,
            x, y, z, az, el, roll
        );
        Ok(())
    }

    /// Write handler for `ctl.pos.motion`.
    ///
    /// Accepted formats:
    /// ```text
    /// none
    /// velocity <vx> <vy> <vz>
    /// linear <tx> <ty> <tz> <dur_us>
    /// circle <cx> <cy> <cz> <radius> <angular_vel_deg_per_us>
    /// ```
    pub fn write_pos_motion(
        &mut self,
        node_index: usize,
        msg: fuse::Message,
    ) -> Result<(), RouterError> {
        // Snapshot current position before switching patterns.
        self.apply_motion(node_index);
        let s = String::from_utf8_lossy(&msg.data);
        let start_ts = self.timestep;
        let current_point = self.channels.nodes[node_index].position.point;
        let pattern = Self::parse_motion_spec(s.trim(), current_point, start_ts)
            .map_err(|e| RouterError::InvalidMotionPattern(e.to_string()))?;
        self.channels.nodes[node_index].motion = pattern;
        Ok(())
    }

    fn parse_motion_spec(
        s: &str,
        current_point: Point,
        start_ts: u64,
    ) -> Result<MotionPattern, &'static str> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        match parts.as_slice() {
            ["none"] => Ok(MotionPattern::Static),
            ["velocity", vx, vy, vz] => {
                let vx = vx.parse::<f64>().map_err(|_| "invalid vx")?;
                let vy = vy.parse::<f64>().map_err(|_| "invalid vy")?;
                let vz = vz.parse::<f64>().map_err(|_| "invalid vz")?;
                Ok(MotionPattern::Velocity {
                    initial: current_point,
                    velocity: Point { x: vx, y: vy, z: vz },
                    start_ts,
                })
            }
            ["linear", tx, ty, tz, dur_us] => {
                let tx = tx.parse::<f64>().map_err(|_| "invalid tx")?;
                let ty = ty.parse::<f64>().map_err(|_| "invalid ty")?;
                let tz = tz.parse::<f64>().map_err(|_| "invalid tz")?;
                let dur = dur_us.parse::<u64>().map_err(|_| "invalid duration")?;
                Ok(MotionPattern::Linear {
                    start: current_point,
                    end: Point { x: tx, y: ty, z: tz },
                    start_ts,
                    duration_us: dur,
                })
            }
            ["circle", cx, cy, cz, radius, ang_vel] => {
                let cx = cx.parse::<f64>().map_err(|_| "invalid cx")?;
                let cy = cy.parse::<f64>().map_err(|_| "invalid cy")?;
                let cz = cz.parse::<f64>().map_err(|_| "invalid cz")?;
                let radius = radius.parse::<f64>().map_err(|_| "invalid radius")?;
                let ang_vel =
                    ang_vel.parse::<f64>().map_err(|_| "invalid angular velocity")?;
                // Derive the initial angle from the current position relative to
                // the center so the node starts at the right point on the orbit.
                let dx = current_point.x - cx;
                let dy = current_point.y - cy;
                let start_angle_deg = dy.atan2(dx).to_degrees();
                Ok(MotionPattern::Circle {
                    center: Point { x: cx, y: cy, z: cz },
                    radius,
                    start_angle_deg,
                    angular_vel_deg_per_us: ang_vel,
                    start_ts,
                })
            }
            _ => Err("unknown motion pattern; expected: none | velocity | linear | circle"),
        }
    }
}
