//! Private projected-margin validation and distance queries.

use glam::{DVec2, DVec3};

use crate::{DesignError, geometry::Frame};

pub(crate) struct Polygon {
    pub points: Vec<DVec2>,
    pub margin: Vec<DVec3>,
    pub min: DVec2,
    pub max: DVec2,
}

impl Polygon {
    pub fn new(margin: &[DVec3], frame: Frame) -> Result<Self, DesignError> {
        let mut clean = Vec::new();
        for &point in margin {
            if !point.is_finite() {
                return Err(DesignError::InvalidMargin);
            }
            if clean.last() != Some(&point) {
                clean.push(point);
            }
        }
        if clean.len() > 1 && clean.first() == clean.last() {
            clean.pop();
        }
        if clean.len() < 3 {
            return Err(DesignError::InvalidMargin);
        }
        let points: Vec<_> = clean.iter().map(|&p| frame.project(p)).collect();
        let min = points.iter().copied().reduce(DVec2::min).unwrap();
        let max = points.iter().copied().reduce(DVec2::max).unwrap();
        let scale = (max - min).length();
        if !scale.is_finite() || scale == 0.0 {
            return Err(DesignError::InvalidMargin);
        }
        let tolerance = scale * 1e-12;
        let n = points.len();
        let mut area = 0.0;
        for i in 0..n {
            let a = points[i];
            let b = points[(i + 1) % n];
            let c = points[(i + 2) % n];
            if a.distance(b) <= tolerance {
                return Err(DesignError::InvalidMargin);
            }
            // Adjacent edges may meet but must not double back over one another.
            if (b - a).perp_dot(c - b).abs() <= tolerance * scale && (b - a).dot(c - b) < 0.0 {
                return Err(DesignError::InvalidMargin);
            }
            area += a.perp_dot(b);
            for j in i + 2..n {
                if (j + 1) % n == i {
                    continue;
                }
                if intersects(a, b, points[j], points[(j + 1) % n], tolerance * scale) {
                    return Err(DesignError::InvalidMargin);
                }
            }
        }
        if area.abs() <= tolerance * scale {
            return Err(DesignError::InvalidMargin);
        }
        Ok(Self {
            points,
            margin: clean,
            min,
            max,
        })
    }

    pub fn edges(&self) -> impl Iterator<Item = (DVec2, DVec2)> + '_ {
        self.points
            .iter()
            .copied()
            .zip(self.points.iter().copied().cycle().skip(1))
            .take(self.points.len())
    }

    pub fn contains(&self, p: DVec2) -> bool {
        let mut inside = false;
        for (a, b) in self.edges() {
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
                inside = !inside;
            }
        }
        inside
    }

    pub fn distance(&self, p: DVec2) -> f64 {
        self.edges()
            .map(|(a, b)| {
                let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                p.distance_squared(a + t * (b - a))
            })
            .fold(f64::INFINITY, f64::min)
            .sqrt()
    }
}

pub(crate) fn intersects(a: DVec2, b: DVec2, c: DVec2, d: DVec2, epsilon: f64) -> bool {
    if a.max(b).x < c.min(d).x
        || c.max(d).x < a.min(b).x
        || a.max(b).y < c.min(d).y
        || c.max(d).y < a.min(b).y
    {
        return false;
    }
    let ab = [(b - a).perp_dot(c - a), (b - a).perp_dot(d - a)];
    let cd = [(d - c).perp_dot(a - c), (d - c).perp_dot(b - c)];
    let straddles = |v: [f64; 2]| {
        (v[0] <= epsilon && v[1] >= -epsilon) || (v[1] <= epsilon && v[0] >= -epsilon)
    };
    straddles(ab) && straddles(cd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concave_polygon_distance_and_duplicate_cleanup() {
        let points = [
            DVec3::ZERO,
            DVec3::X * 2.0,
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::Y * 2.0,
            DVec3::ZERO,
            DVec3::ZERO,
        ];
        let polygon = Polygon::new(&points, Frame::new(DVec3::ZERO, DVec3::Z)).unwrap();
        assert_eq!(polygon.points.len(), 4);
        assert!(polygon.contains(DVec2::splat(0.3)));
        assert!(!polygon.contains(DVec2::splat(1.6)));
        assert!((polygon.distance(DVec2::splat(0.3)) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn overlapping_and_zero_area_margins_fail() {
        let frame = Frame::new(DVec3::ZERO, DVec3::Z);
        for points in [
            vec![DVec3::ZERO, DVec3::X, DVec3::X * 2.0],
            vec![DVec3::ZERO, DVec3::X * 2.0, DVec3::X, DVec3::Y],
            vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Z, DVec3::Y],
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::X, DVec3::ONE],
        ] {
            assert!(matches!(
                Polygon::new(&points, frame),
                Err(DesignError::InvalidMargin)
            ));
        }
    }
}
