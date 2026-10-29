//! Pure helpers of the margin line editor: control points along a closed curve.

use glam::DVec3;

/// Control points per millimetre of margin length.
const MM_PER_CONTROL: f64 = 1.5;

/// Length of a closed curve in millimetres.
pub fn curve_length(curve: &[DVec3]) -> f64 {
    if curve.len() < 2 {
        return 0.0;
    }
    (0..curve.len())
        .map(|i| curve[i].distance(curve[(i + 1) % curve.len()]))
        .sum()
}

/// About one control point per 1.5 mm of margin, between 16 and 24.
pub fn default_control_count(curve_length_mm: f64) -> usize {
    let count = if curve_length_mm.is_finite() {
        (curve_length_mm / MM_PER_CONTROL).round().max(0.0) as usize
    } else {
        0
    };
    count.clamp(16, 24)
}

/// `count` points evenly spaced by arc length along the closed `curve`, the first at
/// `curve[0]`. `count` is clamped to `3..=curve.len()`; a curve with fewer than three points
/// gives none.
pub fn control_points(curve: &[DVec3], count: usize) -> Vec<DVec3> {
    if curve.len() < 3 {
        return Vec::new();
    }
    let count = count.clamp(3, curve.len());
    let total = curve_length(curve);
    if total <= 0.0 {
        return vec![curve[0]; count];
    }
    let step = total / count as f64;
    let mut points = Vec::with_capacity(count);
    let (mut segment, mut walked) = (0, 0.0);
    for k in 0..count {
        // A small tolerance keeps corners that land exactly on a target from drifting.
        let target = step * k as f64;
        loop {
            let a = curve[segment % curve.len()];
            let b = curve[(segment + 1) % curve.len()];
            let length = a.distance(b);
            if walked + length >= target - 1e-12 || segment + 1 >= curve.len() {
                let t = if length > 0.0 {
                    ((target - walked) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                points.push(a.lerp(b, t));
                break;
            }
            walked += length;
            segment += 1;
        }
    }
    points
}

/// Index at which to insert `p` so that it lands on the nearest segment of the closed polygon:
/// between `i` and `i + 1` gives `i + 1`, the closing segment gives `controls.len()`.
pub fn insertion_index(controls: &[DVec3], p: DVec3) -> usize {
    if controls.len() < 2 {
        return controls.len();
    }
    let mut best = (f64::INFINITY, controls.len());
    for i in 0..controls.len() {
        let a = controls[i];
        let b = controls[(i + 1) % controls.len()];
        let ab = b - a;
        let len2 = ab.length_squared();
        let t = if len2 > 0.0 {
            ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = p.distance_squared(a + ab * t);
        if d < best.0 {
            best = (d, i + 1);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<DVec3> {
        // Corners every 10 mm, with a midpoint on each side.
        let corners = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::new(0.0, 10.0, 0.0),
        ];
        let mut curve = Vec::new();
        for i in 0..4 {
            curve.push(corners[i]);
            curve.push(corners[i].lerp(corners[(i + 1) % 4], 0.5));
        }
        curve
    }

    #[test]
    fn length_of_a_closed_curve() {
        assert!((curve_length(&square()) - 40.0).abs() < 1e-12);
        assert_eq!(curve_length(&[]), 0.0);
        assert_eq!(curve_length(&[DVec3::X]), 0.0);
    }

    #[test]
    fn square_gives_evenly_spaced_controls_including_corners() {
        let controls = control_points(&square(), 4);
        let corners = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::new(0.0, 10.0, 0.0),
        ];
        assert_eq!(controls.len(), 4);
        for (c, e) in controls.iter().zip(corners) {
            assert!(c.distance(e) < 1e-9, "{c:?} vs {e:?}");
        }
        let eight = control_points(&square(), 8);
        assert_eq!(eight.len(), 8);
        for k in 0..8 {
            let d = eight[k].distance(eight[(k + 1) % 8]);
            assert!((d - 5.0).abs() < 1e-9, "spacing {d}");
        }
    }

    #[test]
    fn count_is_clamped() {
        let curve = square();
        assert_eq!(control_points(&curve, 0).len(), 3);
        assert_eq!(control_points(&curve, 2).len(), 3);
        assert_eq!(control_points(&curve, 1000).len(), curve.len());
        assert_eq!(default_control_count(0.0), 16);
        assert_eq!(default_control_count(30.0), 20);
        assert_eq!(default_control_count(1000.0), 24);
        assert_eq!(default_control_count(f64::NAN), 16);
    }

    #[test]
    fn empty_and_short_curves_give_nothing() {
        assert!(control_points(&[], 5).is_empty());
        assert!(control_points(&[DVec3::X, DVec3::Y], 5).is_empty());
    }

    #[test]
    fn insertion_lands_on_the_nearest_segment() {
        let controls = [
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(10.0, 0.0, 0.0),
            DVec3::new(10.0, 10.0, 0.0),
            DVec3::new(0.0, 10.0, 0.0),
        ];
        assert_eq!(insertion_index(&controls, DVec3::new(5.0, -0.5, 0.0)), 1);
        assert_eq!(insertion_index(&controls, DVec3::new(10.5, 5.0, 0.0)), 2);
        assert_eq!(insertion_index(&controls, DVec3::new(5.0, 10.5, 0.0)), 3);
        // The closing segment from the last control back to the first.
        assert_eq!(insertion_index(&controls, DVec3::new(-0.5, 5.0, 0.0)), 4);
        assert_eq!(insertion_index(&[], DVec3::X), 0);
        assert_eq!(insertion_index(&[DVec3::ZERO], DVec3::X), 1);
    }
}
