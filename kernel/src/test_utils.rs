#[cfg(test)]
pub(crate) mod helpers {
    use config::ast::Point;

    pub fn pt(x: f64, y: f64, z: f64) -> Point {
        Point { x, y, z }
    }

    pub fn assert_point_near(actual: Point, expected: Point, eps: f64) {
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
}
