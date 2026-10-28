//! Colour maps that turn a per-vertex scalar (undercut depth, distance to the antagonist,
//! thickness) into sRGB RGBA colours for [`crate::MeshData::with_colors`].
//!
//! Both palettes are colour-blind safe and keep their contrast on the dark and the light theme,
//! because the colours land on the lit mesh rather than on the background:
//!
//! - [`diverging`]: blue – light grey – red, sampled from Kenneth Moreland's "cool to warm" map
//!   ("Diverging Color Maps for Scientific Visualization", 2009). For signed values such as
//!   penetration (blue) versus clearance (red).
//! - [`sequential`]: viridis (Stéfan van der Walt and Nathaniel Smith, CC0), dark purple through
//!   teal to yellow, with lightness rising monotonically. For magnitudes such as thickness.

/// Cool-to-warm anchors at 0, ¼, ½, ¾ and 1 (sRGB, 0..255).
const DIVERGING: [[u8; 3]; 5] = [
    [59, 76, 192],
    [141, 176, 254],
    [221, 221, 221],
    [244, 154, 123],
    [180, 4, 38],
];

/// Viridis anchors at every eighth (sRGB, 0..255).
const SEQUENTIAL: [[u8; 3]; 9] = [
    [68, 1, 84],
    [71, 44, 122],
    [59, 81, 139],
    [44, 113, 142],
    [33, 144, 141],
    [39, 173, 129],
    [92, 200, 99],
    [170, 220, 50],
    [253, 231, 37],
];

/// Diverging colour of `value`: `min` and below is blue, the middle of the range is light grey,
/// `max` and above is red. Opaque sRGB RGBA, 0..1. A value that is not finite, or an empty range,
/// gives the middle colour.
pub fn diverging(value: f32, min: f32, max: f32) -> [f32; 4] {
    sample(&DIVERGING, unit(value, min, max))
}

/// Sequential colour of `value`: `min` and below is dark purple, `max` and above is yellow.
/// Opaque sRGB RGBA, 0..1. A value that is not finite, or an empty range, gives the middle colour.
pub fn sequential(value: f32, min: f32, max: f32) -> [f32; 4] {
    sample(&SEQUENTIAL, unit(value, min, max))
}

/// Position of `value` in `min..max`, clamped to 0..1.
fn unit(value: f32, min: f32, max: f32) -> f32 {
    let t = (value - min) / (max - min);
    if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

/// Piecewise-linear interpolation between evenly spaced anchors.
fn sample(anchors: &[[u8; 3]], t: f32) -> [f32; 4] {
    let x = t * (anchors.len() - 1) as f32;
    let i = (x.floor() as usize).min(anchors.len() - 2);
    let f = x - i as f32;
    let (a, b) = (anchors[i], anchors[i + 1]);
    let channel = |k: usize| (f32::from(a[k]) * (1.0 - f) + f32::from(b[k]) * f) / 255.0;
    [channel(0), channel(1), channel(2), 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 4], b: [u8; 3]) -> bool {
        (0..3).all(|k| (a[k] - f32::from(b[k]) / 255.0).abs() < 1e-5) && a[3] == 1.0
    }

    /// Relative luminance of an sRGB colour.
    fn luminance(c: [f32; 4]) -> f32 {
        let lin = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
    }

    #[test]
    fn diverging_runs_blue_grey_red_and_clamps() {
        assert!(close(diverging(-1.0, -1.0, 1.0), DIVERGING[0]));
        assert!(close(diverging(0.0, -1.0, 1.0), DIVERGING[2]));
        assert!(close(diverging(1.0, -1.0, 1.0), DIVERGING[4]));
        assert_eq!(diverging(-5.0, -1.0, 1.0), diverging(-1.0, -1.0, 1.0));
        assert_eq!(diverging(5.0, -1.0, 1.0), diverging(1.0, -1.0, 1.0));
        let blue = diverging(-0.8, -1.0, 1.0);
        let red = diverging(0.8, -1.0, 1.0);
        assert!(blue[2] > blue[0] && red[0] > red[2]);
    }

    #[test]
    fn sequential_gets_lighter_along_the_range() {
        assert!(close(sequential(0.0, 0.0, 2.0), SEQUENTIAL[0]));
        assert!(close(sequential(2.0, 0.0, 2.0), SEQUENTIAL[8]));
        let mut last = -1.0;
        for i in 0..=100 {
            let l = luminance(sequential(i as f32, 0.0, 100.0));
            assert!(l > last, "luminance falls at {i}");
            last = l;
        }
    }

    #[test]
    fn degenerate_input_gives_the_middle_colour() {
        let middle = diverging(0.0, -1.0, 1.0);
        assert_eq!(diverging(f32::NAN, -1.0, 1.0), middle);
        assert_eq!(diverging(3.0, 2.0, 2.0), middle);
        assert_eq!(
            sequential(f32::INFINITY, 0.0, 0.0),
            sequential(0.5, 0.0, 1.0)
        );
        // Reversed ranges flip the map.
        assert_eq!(diverging(1.0, 1.0, -1.0), diverging(-1.0, -1.0, 1.0));
    }
}
