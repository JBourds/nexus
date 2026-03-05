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
    pub fn write_pos(&mut self, node_index: usize, msg: fuse::Message) -> Result<(), RouterError> {
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

    pub(crate) fn parse_motion_spec(
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
                    velocity: Point {
                        x: vx,
                        y: vy,
                        z: vz,
                    },
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
                    end: Point {
                        x: tx,
                        y: ty,
                        z: tz,
                    },
                    start_ts,
                    duration_us: dur,
                })
            }
            ["circle", cx, cy, cz, radius, ang_vel] => {
                let cx = cx.parse::<f64>().map_err(|_| "invalid cx")?;
                let cy = cy.parse::<f64>().map_err(|_| "invalid cy")?;
                let cz = cz.parse::<f64>().map_err(|_| "invalid cz")?;
                let radius = radius.parse::<f64>().map_err(|_| "invalid radius")?;
                let ang_vel = ang_vel
                    .parse::<f64>()
                    .map_err(|_| "invalid angular velocity")?;
                // Derive the initial angle from the current position relative to
                // the center so the node starts at the right point on the orbit.
                let dx = current_point.x - cx;
                let dy = current_point.y - cy;
                let start_angle_deg = dy.atan2(dx).to_degrees();
                Ok(MotionPattern::Circle {
                    center: Point {
                        x: cx,
                        y: cy,
                        z: cz,
                    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::RoutingServer;

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

    // parse_motion_spec valid inputs

    #[test]
    fn parse_none() {
        let m = RoutingServer::parse_motion_spec("none", pt(0.0, 0.0, 0.0), 0).unwrap();
        assert!(matches!(m, MotionPattern::Static));
    }

    #[test]
    fn parse_velocity() {
        let m = RoutingServer::parse_motion_spec("velocity 1.5 -2.0 0.0", pt(3.0, 4.0, 5.0), 10)
            .unwrap();
        match m {
            MotionPattern::Velocity {
                initial,
                velocity,
                start_ts,
            } => {
                assert_point_near(initial, pt(3.0, 4.0, 5.0), 1e-12);
                assert_point_near(velocity, pt(1.5, -2.0, 0.0), 1e-12);
                assert_eq!(start_ts, 10);
            }
            _ => panic!("expected Velocity"),
        }
    }

    #[test]
    fn parse_linear() {
        let m =
            RoutingServer::parse_motion_spec("linear 10 20 30 5000", pt(0.0, 0.0, 0.0), 5).unwrap();
        match m {
            MotionPattern::Linear {
                start,
                end,
                start_ts,
                duration_us,
            } => {
                assert_point_near(start, pt(0.0, 0.0, 0.0), 1e-12);
                assert_point_near(end, pt(10.0, 20.0, 30.0), 1e-12);
                assert_eq!(start_ts, 5);
                assert_eq!(duration_us, 5000);
            }
            _ => panic!("expected Linear"),
        }
    }

    #[test]
    fn parse_circle() {
        // Node at (10, 0, 0), center (0, 0, 0) → start_angle = atan2(0, 10) = 0°
        let m =
            RoutingServer::parse_motion_spec("circle 0 0 0 10 0.5", pt(10.0, 0.0, 0.0), 0).unwrap();
        match m {
            MotionPattern::Circle {
                center,
                radius,
                start_angle_deg,
                angular_vel_deg_per_us,
                start_ts,
            } => {
                assert_point_near(center, pt(0.0, 0.0, 0.0), 1e-12);
                assert!((radius - 10.0).abs() < 1e-12);
                assert!((start_angle_deg - 0.0).abs() < 1e-9);
                assert!((angular_vel_deg_per_us - 0.5).abs() < 1e-12);
                assert_eq!(start_ts, 0);
            }
            _ => panic!("expected Circle"),
        }
    }

    #[test]
    fn parse_circle_start_angle_from_position() {
        // Node at (0, 5, 0), center (0, 0, 0) → atan2(5, 0) = 90°
        let m =
            RoutingServer::parse_motion_spec("circle 0 0 0 5 1.0", pt(0.0, 5.0, 0.0), 0).unwrap();
        match m {
            MotionPattern::Circle {
                start_angle_deg, ..
            } => {
                assert!(
                    (start_angle_deg - 90.0).abs() < 1e-9,
                    "expected ~90°, got {}",
                    start_angle_deg
                );
            }
            _ => panic!("expected Circle"),
        }
    }

    // parse_motion_spec invalid inputs

    #[test]
    fn parse_empty_fails() {
        assert!(RoutingServer::parse_motion_spec("", pt(0.0, 0.0, 0.0), 0).is_err());
    }

    #[test]
    fn parse_unknown_keyword_fails() {
        assert!(RoutingServer::parse_motion_spec("zigzag 1 2 3", pt(0.0, 0.0, 0.0), 0).is_err());
    }

    #[test]
    fn parse_velocity_wrong_arg_count_fails() {
        assert!(RoutingServer::parse_motion_spec("velocity 1 2", pt(0.0, 0.0, 0.0), 0).is_err());
    }

    #[test]
    fn parse_velocity_non_numeric_fails() {
        assert!(
            RoutingServer::parse_motion_spec("velocity abc 2 3", pt(0.0, 0.0, 0.0), 0).is_err()
        );
    }

    #[test]
    fn parse_linear_wrong_arg_count_fails() {
        assert!(RoutingServer::parse_motion_spec("linear 1 2 3", pt(0.0, 0.0, 0.0), 0).is_err());
    }

    #[test]
    fn parse_circle_wrong_arg_count_fails() {
        assert!(RoutingServer::parse_motion_spec("circle 0 0 0 5", pt(0.0, 0.0, 0.0), 0).is_err());
    }

    // Round-trip: to_spec → parse → current_point equivalence

    #[test]
    fn velocity_round_trip() {
        let current = pt(1.0, 2.0, 3.0);
        let original = MotionPattern::Velocity {
            initial: current,
            velocity: pt(0.1, -0.2, 0.3),
            start_ts: 50,
        };
        let spec = original.to_spec();
        // parse_motion_spec uses the supplied current_point as initial
        let parsed = RoutingServer::parse_motion_spec(&spec, current, 50).unwrap();
        // Both should produce the same position at timestep 150
        let p1 = original.current_point(150).unwrap();
        let p2 = parsed.current_point(150).unwrap();
        assert_point_near(p1, p2, 1e-9);
    }

    #[test]
    fn linear_round_trip() {
        let current = pt(0.0, 0.0, 0.0);
        let original = MotionPattern::Linear {
            start: current,
            end: pt(10.0, 20.0, 30.0),
            start_ts: 0,
            duration_us: 1000,
        };
        let spec = original.to_spec();
        let parsed = RoutingServer::parse_motion_spec(&spec, current, 0).unwrap();
        for ts in [0, 250, 500, 750, 1000, 2000] {
            let p1 = original.current_point(ts).unwrap();
            let p2 = parsed.current_point(ts).unwrap();
            assert_point_near(p1, p2, 1e-9);
        }
    }
}
