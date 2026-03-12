use crate::ast::{DistanceUnit, Position};
use crate::units::DecimalScaled;

impl Position {
    /// Return 3D euclidean distance between two points
    /// after converting to a common unit system.
    pub fn distance(from: &Self, to: &Self) -> (f64, DistanceUnit) {
        let (from_greater, ratio) = DistanceUnit::ratio(from.unit, to.unit);
        let scalar = 10.0_f64.powi(ratio as i32);
        let unit = if from_greater { from.unit } else { to.unit };
        let scale = |(x, y, z), scale_up| {
            if scale_up {
                (x * scalar, y * scalar, z * scalar)
            } else {
                (x, y, z)
            }
        };

        let (from_x, from_y, from_z) =
            scale((from.point.x, from.point.y, from.point.z), !from_greater);
        let (to_x, to_y, to_z) = scale((to.point.x, to.point.y, to.point.z), from_greater);

        let x = from_x - to_x;
        let y = from_y - to_y;
        let z = from_z - to_z;
        ((x * x + y * y + z * z).sqrt(), unit)
    }
}
