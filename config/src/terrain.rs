//! 2D terrain model for line-of-sight obstruction and signal attenuation.
//!
//! The terrain system models rectangular and linear obstacles in the XY plane.
//! When computing RSSI between two nodes, a ray is cast from source to
//! destination. For each obstacle the ray intersects, an attenuation value
//! (in dB) is accumulated and subtracted from the free-space RSSI.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ast::DistanceUnit;
use crate::units::DecimalScaled;

/// A fully resolved terrain map ready for ray-cast queries.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TerrainMap {
    obstacles: Vec<ResolvedObstacle>,
    unit: DistanceUnit,
}

/// An obstacle with its attenuation already resolved (no material lookup needed).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResolvedObstacle {
    #[allow(dead_code)]
    name: String,
    attenuation_db: f64,
    shape: ObstacleShape,
}

/// Geometric shape of an obstacle in the XY plane.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ObstacleShape {
    /// Axis-aligned rectangle defined by min/max corners.
    Rect { min: [f64; 2], max: [f64; 2] },
    /// A thin wall defined by two endpoints and a thickness.
    Line {
        start: [f64; 2],
        end: [f64; 2],
        thickness: f64,
    },
}

/// Default attenuation values for common building materials (dB per traversal).
/// Based on ITU-R P.2109 and empirical measurements at 2.4 GHz.
pub fn default_materials() -> HashMap<String, f64> {
    let mut m = HashMap::new();
    m.insert("concrete".into(), 15.0);
    m.insert("brick".into(), 10.0);
    m.insert("wood".into(), 4.0);
    m.insert("glass".into(), 3.0);
    m.insert("foliage".into(), 6.0);
    m.insert("metal".into(), 25.0);
    m.insert("drywall".into(), 3.0);
    m.insert("water".into(), 20.0);
    m
}

impl TerrainMap {
    /// Create a terrain map from a list of obstacles with pre-resolved attenuation.
    pub fn new(obstacles: Vec<(String, f64, ObstacleShape)>, unit: DistanceUnit) -> Self {
        let obstacles = obstacles
            .into_iter()
            .map(|(name, attenuation_db, shape)| ResolvedObstacle {
                name,
                attenuation_db,
                shape,
            })
            .collect();
        Self { obstacles, unit }
    }

    /// Compute total terrain attenuation in dB for a ray between two 2D points.
    ///
    /// Both points are in the terrain's coordinate unit. The returned value is
    /// the sum of attenuation_db for every obstacle the ray intersects.
    pub fn attenuation_db(&self, from: (f64, f64), to: (f64, f64)) -> f64 {
        // Degenerate ray: same point → no obstruction.
        if (from.0 - to.0).abs() < f64::EPSILON && (from.1 - to.1).abs() < f64::EPSILON {
            return 0.0;
        }

        let mut total = 0.0;
        for obs in &self.obstacles {
            if obs.shape.intersects_segment(from, to) {
                total += obs.attenuation_db;
            }
        }
        total
    }

    /// Compute attenuation between two positions, handling unit conversion.
    pub fn attenuation_between_positions(
        &self,
        from_xy: (f64, f64),
        from_unit: DistanceUnit,
        to_xy: (f64, f64),
        to_unit: DistanceUnit,
    ) -> f64 {
        let from = self.convert_to_terrain_unit(from_xy, from_unit);
        let to = self.convert_to_terrain_unit(to_xy, to_unit);
        self.attenuation_db(from, to)
    }

    /// The distance unit used by this terrain map.
    pub fn unit(&self) -> DistanceUnit {
        self.unit
    }

    fn convert_to_terrain_unit(&self, point: (f64, f64), unit: DistanceUnit) -> (f64, f64) {
        if unit == self.unit {
            return point;
        }
        let (unit_is_larger, ratio) = DistanceUnit::ratio(unit, self.unit);
        let scalar = 10.0_f64.powi(ratio as i32);
        if unit_is_larger {
            // unit > self.unit, so multiply to convert to smaller terrain unit
            (point.0 * scalar, point.1 * scalar)
        } else {
            // self.unit > unit, so divide
            (point.0 / scalar, point.1 / scalar)
        }
    }
}

impl ObstacleShape {
    /// Test whether a line segment from `p0` to `p1` intersects this shape.
    pub fn intersects_segment(&self, p0: (f64, f64), p1: (f64, f64)) -> bool {
        match self {
            ObstacleShape::Rect { min, max } => {
                segment_intersects_aabb(p0, p1, (min[0], min[1]), (max[0], max[1]))
            }
            ObstacleShape::Line {
                start,
                end,
                thickness,
            } => {
                // Expand the line into a thin rectangle perpendicular to its direction.
                let wall_rect = line_to_rect(
                    (start[0], start[1]),
                    (end[0], end[1]),
                    *thickness,
                );
                segment_intersects_aabb(p0, p1, wall_rect.0, wall_rect.1)
            }
        }
    }
}

/// Expand a line segment into an axis-aligned bounding box with the given thickness.
fn line_to_rect(
    start: (f64, f64),
    end: (f64, f64),
    thickness: f64,
) -> ((f64, f64), (f64, f64)) {
    let half = thickness / 2.0;
    let min_x = start.0.min(end.0) - half;
    let min_y = start.1.min(end.1) - half;
    let max_x = start.0.max(end.0) + half;
    let max_y = start.1.max(end.1) + half;
    ((min_x, min_y), (max_x, max_y))
}

/// Test whether a line segment from p0 to p1 intersects an axis-aligned
/// bounding box defined by (min, max) using the slab method.
///
/// Returns true if any portion of the segment lies within the AABB.
fn segment_intersects_aabb(
    p0: (f64, f64),
    p1: (f64, f64),
    min: (f64, f64),
    max: (f64, f64),
) -> bool {
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;

    let mut tmin = 0.0_f64;
    let mut tmax = 1.0_f64;

    // X slab
    if dx.abs() < f64::EPSILON {
        // Ray is vertical — check if X is within the box
        if p0.0 < min.0 || p0.0 > max.0 {
            return false;
        }
    } else {
        let inv_d = 1.0 / dx;
        let mut t1 = (min.0 - p0.0) * inv_d;
        let mut t2 = (max.0 - p0.0) * inv_d;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return false;
        }
    }

    // Y slab
    if dy.abs() < f64::EPSILON {
        // Ray is horizontal — check if Y is within the box
        if p0.1 < min.1 || p0.1 > max.1 {
            return false;
        }
    } else {
        let inv_d = 1.0 / dy;
        let mut t1 = (min.1 - p0.1) * inv_d;
        let mut t2 = (max.1 - p0.1) * inv_d;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rect(name: &str, db: f64, min: [f64; 2], max: [f64; 2]) -> (String, f64, ObstacleShape) {
        (name.into(), db, ObstacleShape::Rect { min, max })
    }

    fn make_line(name: &str, db: f64, start: [f64; 2], end: [f64; 2], thickness: f64) -> (String, f64, ObstacleShape) {
        (name.into(), db, ObstacleShape::Line { start, end, thickness })
    }

    // ── Segment-AABB intersection tests ──────────────────────────────────

    #[test]
    fn rect_intersects_segment_through_center() {
        // Ray goes straight through a box
        let result = segment_intersects_aabb(
            (0.0, 5.0), (20.0, 5.0), // horizontal ray at y=5
            (5.0, 0.0), (15.0, 10.0), // box from (5,0) to (15,10)
        );
        assert!(result);
    }

    #[test]
    fn rect_misses_segment_above() {
        // Ray passes above the box
        let result = segment_intersects_aabb(
            (0.0, 15.0), (20.0, 15.0), // horizontal ray at y=15
            (5.0, 0.0), (15.0, 10.0),  // box top is y=10
        );
        assert!(!result);
    }

    #[test]
    fn rect_misses_segment_beside() {
        // Ray passes to the right of the box
        let result = segment_intersects_aabb(
            (20.0, 0.0), (20.0, 10.0), // vertical ray at x=20
            (5.0, 0.0), (15.0, 10.0),  // box right edge is x=15
        );
        assert!(!result);
    }

    #[test]
    fn rect_intersects_diagonal_ray() {
        let result = segment_intersects_aabb(
            (0.0, 0.0), (10.0, 10.0),
            (3.0, 3.0), (7.0, 7.0),
        );
        assert!(result);
    }

    #[test]
    fn rect_misses_short_segment() {
        // Segment ends before reaching the box
        let result = segment_intersects_aabb(
            (0.0, 5.0), (3.0, 5.0),    // short segment
            (5.0, 0.0), (15.0, 10.0),  // box starts at x=5
        );
        assert!(!result);
    }

    #[test]
    fn segment_starting_inside_box() {
        let result = segment_intersects_aabb(
            (10.0, 5.0), (20.0, 5.0),  // starts inside box
            (5.0, 0.0), (15.0, 10.0),
        );
        assert!(result);
    }

    #[test]
    fn vertical_segment_through_box() {
        let result = segment_intersects_aabb(
            (10.0, -5.0), (10.0, 15.0), // vertical through center
            (5.0, 0.0), (15.0, 10.0),
        );
        assert!(result);
    }

    #[test]
    fn vertical_segment_missing_box() {
        let result = segment_intersects_aabb(
            (3.0, -5.0), (3.0, 15.0), // vertical to the left
            (5.0, 0.0), (15.0, 10.0),
        );
        assert!(!result);
    }

    // ── TerrainMap.attenuation_db tests ──────────────────────────────────

    #[test]
    fn zero_obstacles_returns_zero() {
        let terrain = TerrainMap::new(vec![], DistanceUnit::Meters);
        assert_eq!(terrain.attenuation_db((0.0, 0.0), (100.0, 100.0)), 0.0);
    }

    #[test]
    fn same_point_returns_zero() {
        let terrain = TerrainMap::new(
            vec![make_rect("wall", 15.0, [0.0, 0.0], [100.0, 100.0])],
            DistanceUnit::Meters,
        );
        assert_eq!(terrain.attenuation_db((50.0, 50.0), (50.0, 50.0)), 0.0);
    }

    #[test]
    fn ray_through_single_obstacle() {
        let terrain = TerrainMap::new(
            vec![make_rect("building", 15.0, [40.0, 0.0], [60.0, 100.0])],
            DistanceUnit::Meters,
        );
        let loss = terrain.attenuation_db((0.0, 50.0), (100.0, 50.0));
        assert_eq!(loss, 15.0);
    }

    #[test]
    fn ray_missing_obstacle() {
        let terrain = TerrainMap::new(
            vec![make_rect("building", 15.0, [40.0, 60.0], [60.0, 100.0])],
            DistanceUnit::Meters,
        );
        // Ray at y=10, obstacle is at y=60..100
        let loss = terrain.attenuation_db((0.0, 10.0), (100.0, 10.0));
        assert_eq!(loss, 0.0);
    }

    #[test]
    fn ray_through_two_obstacles_accumulates() {
        let terrain = TerrainMap::new(
            vec![
                make_rect("wall1", 10.0, [20.0, 0.0], [30.0, 100.0]),
                make_rect("wall2", 5.0, [60.0, 0.0], [70.0, 100.0]),
            ],
            DistanceUnit::Meters,
        );
        let loss = terrain.attenuation_db((0.0, 50.0), (100.0, 50.0));
        assert_eq!(loss, 15.0);
    }

    #[test]
    fn line_obstacle_intersects() {
        let terrain = TerrainMap::new(
            vec![make_line("wall", 8.0, [50.0, 0.0], [50.0, 100.0], 1.0)],
            DistanceUnit::Meters,
        );
        // Horizontal ray at y=50 crosses the vertical wall at x=50
        let loss = terrain.attenuation_db((0.0, 50.0), (100.0, 50.0));
        assert_eq!(loss, 8.0);
    }

    #[test]
    fn line_obstacle_misses() {
        let terrain = TerrainMap::new(
            vec![make_line("wall", 8.0, [50.0, 60.0], [50.0, 100.0], 1.0)],
            DistanceUnit::Meters,
        );
        // Ray at y=10 should miss the wall which is at y=60..100
        let loss = terrain.attenuation_db((0.0, 10.0), (100.0, 10.0));
        assert_eq!(loss, 0.0);
    }

    #[test]
    fn default_materials_contains_expected_keys() {
        let mats = default_materials();
        assert_eq!(*mats.get("concrete").unwrap(), 15.0);
        assert_eq!(*mats.get("metal").unwrap(), 25.0);
        assert_eq!(*mats.get("glass").unwrap(), 3.0);
        assert_eq!(*mats.get("foliage").unwrap(), 6.0);
        assert!(mats.contains_key("wood"));
        assert!(mats.contains_key("drywall"));
        assert!(mats.contains_key("water"));
        assert!(mats.contains_key("brick"));
    }

    // ── Unit conversion tests ────────────────────────────────────────────

    #[test]
    fn unit_conversion_same_unit() {
        let terrain = TerrainMap::new(
            vec![make_rect("wall", 10.0, [40.0, 0.0], [60.0, 100.0])],
            DistanceUnit::Meters,
        );
        let loss = terrain.attenuation_between_positions(
            (0.0, 50.0), DistanceUnit::Meters,
            (100.0, 50.0), DistanceUnit::Meters,
        );
        assert_eq!(loss, 10.0);
    }

    #[test]
    fn unit_conversion_km_to_m() {
        // Terrain is in meters; obstacle at x=40..60m
        // Nodes in km: (0, 0.05) to (0.1, 0.05) = (0m, 50m) to (100m, 50m)
        let terrain = TerrainMap::new(
            vec![make_rect("wall", 10.0, [40.0, 0.0], [60.0, 100.0])],
            DistanceUnit::Meters,
        );
        let loss = terrain.attenuation_between_positions(
            (0.0, 0.05), DistanceUnit::Kilometers,
            (0.1, 0.05), DistanceUnit::Kilometers,
        );
        assert_eq!(loss, 10.0);
    }
}
