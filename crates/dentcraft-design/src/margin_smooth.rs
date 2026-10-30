//! Low-pass smoothing of a detected margin loop. Ridge detection on a scan picks
//! the finish line vertex by vertex, so the raw loop carries scan noise and
//! sector-to-sector ridge choices as wiggles of a few tenths of a millimetre over
//! a millimetre or two of arc. A finish line is a smooth curve on the die: the
//! loop is filtered in its arc length and each sample is put back on the surface.
//!
//! The loop is split along the die's axis. Its outline across the axis is what a
//! crown is built on, and outline wiggles break the build, so that component
//! loses features shorter than [`STOP_WAVELENGTH_MM`] of arc and keeps those
//! longer than [`PASS_WAVELENGTH_MM`] unchanged. Its height along the axis
//! carries the interproximal scallops, whose pointed tips are short features:
//! the height is filtered alike ([`AXIAL_STOP_WAVELENGTH_MM`],
//! [`AXIAL_PASS_WAVELENGTH_MM`]) but kept near the detected height where that
//! would move it more than [`KEEP_FROM_MM`] or at a sharp peak or valley
//! ([`PEAK_FROM_MM`]); the outline is never pulled back. The filter works on
//! the curve itself, so it needs no centre the outline is star-shaped about and
//! treats concave (kidney-shaped) outlines the same way.

use dentcraft_mesh::{Bvh, TriMesh};
use glam::DVec3;
use std::f64::consts::TAU;

/// Outline features at least this long (mm of arc per period) pass unchanged.
const PASS_WAVELENGTH_MM: f64 = 3.0;
/// Outline features at most this long are removed; between the two the gain
/// falls along a raised cosine, which keeps the filter from ringing.
const STOP_WAVELENGTH_MM: f64 = 1.5;
/// Height features at least this long pass unchanged. Height wiggles between
/// 0.75 and 3 mm fold a real canine's crown as outline wiggles do, so the height
/// is filtered as the outline is; scallop tips are kept by the bound below.
const AXIAL_PASS_WAVELENGTH_MM: f64 = 3.0;
/// Height features at most this long are removed.
const AXIAL_STOP_WAVELENGTH_MM: f64 = 1.5;
/// Sample spacing of the smoothed loop, the margin's nominal editing spacing.
const SPACING_MM: f64 = 0.1;
/// Harmonics a loop must keep to be worth smoothing: shorter loops would shrink
/// towards their centre, so they are left as they are.
const MIN_HARMONICS: usize = 3;
/// Height filtering moves a sample at most this far along the axis from the
/// detected loop's own sample at the same arc length before its height starts
/// falling back to the detected one: scan-noise wiggles move samples less.
const KEEP_FROM_MM: f64 = 0.1;
/// A sample whose height the filter would move this far or further keeps the
/// detected height: the tip of a pointed scallop, which the filter rounds off
/// along a steep wall where projecting back onto the scan does not recover it.
/// Between the two the detected height's weight rises smoothly, so the curve
/// stays continuous.
const KEEP_FULLY_MM: f64 = 0.15;
/// A pointed scallop tip is a sharp peak of a long feature: its detected height
/// stands far above (or a valley far below) the mean of the heights this far
/// along the loop either side, while noise at the lengths the height filter
/// removes stands a few hundredths of a millimetre off it. The filter lowers
/// such a tip by less than [`KEEP_FROM_MM`] but, with the detected loop's own
/// sampling missing the very tip, by enough to matter.
const PEAK_REACH_MM: f64 = 0.75;
/// A sample standing this far off the mean of its neighbours at
/// [`PEAK_REACH_MM`] starts keeping its detected height...
const PEAK_FROM_MM: f64 = 0.3;
/// ...and keeps it fully from this far on.
const PEAK_FULLY_MM: f64 = 0.5;

/// The loop `points` (closed, not repeating its first point) low-pass filtered
/// in arc length as described in the module documentation, split along `axis`,
/// resampled every 0.1 mm and projected onto the closest point of `mesh`.
///
/// Closest point moves a sample along the surface normal, which keeps the
/// split: on a steep axial wall it moves the sample across the axis, keeping the
/// height (and so a kept scallop tip), and on a flat shoulder along the axis,
/// keeping the smoothed outline. Projecting along the axis instead would graze
/// steep walls and land far below or above the finish line.
///
/// `None` for loops with fewer than three points, nonfinite points, a zero or
/// nonfinite `axis`, or shorter than three stop wavelengths, which keep no shape
/// after filtering.
pub(crate) fn low_pass_on_surface(
    mesh: &TriMesh,
    bvh: &Bvh,
    points: &[DVec3],
    axis: DVec3,
) -> Option<Vec<DVec3>> {
    let filtered = low_pass(points, axis)?;
    Some(
        filtered
            .into_iter()
            .map(|p| bvh.closest_point(mesh, p).map_or(p, |hit| hit.point))
            .collect(),
    )
}

/// The filter of [`low_pass_on_surface`] without the projection.
fn low_pass(points: &[DVec3], axis: DVec3) -> Option<Vec<DVec3>> {
    let axis = axis.try_normalize()?;
    if points.len() < 3 || points.iter().any(|p| !p.is_finite()) {
        return None;
    }
    let length: f64 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.distance(*b))
        .sum();
    if ((length / STOP_WAVELENGTH_MM).floor() as usize) < MIN_HARMONICS {
        return None;
    }
    let samples = super::resample(points, SPACING_MM);
    let height: Vec<DVec3> = samples.iter().map(|p| axis * p.dot(axis)).collect();
    let outline: Vec<DVec3> = samples.iter().zip(&height).map(|(p, h)| *p - *h).collect();
    let outline = filter(&outline, length, PASS_WAVELENGTH_MM, STOP_WAVELENGTH_MM);
    let smooth_height = filter(
        &height,
        length,
        AXIAL_PASS_WAVELENGTH_MM,
        AXIAL_STOP_WAVELENGTH_MM,
    );
    let n = samples.len();
    let reach = ((PEAK_REACH_MM / SPACING_MM).round() as usize).min(n / 2);
    Some(
        (0..n)
            .map(|j| {
                let (smooth, detected) = (smooth_height[j], height[j]);
                let moved = smoothstep(
                    (smooth.distance(detected) - KEEP_FROM_MM) / (KEEP_FULLY_MM - KEEP_FROM_MM),
                );
                let beside = (height[(j + n - reach) % n] + height[(j + reach) % n]) / 2.0;
                let peak = smoothstep(
                    (detected.distance(beside) - PEAK_FROM_MM) / (PEAK_FULLY_MM - PEAK_FROM_MM),
                );
                outline[j] + smooth.lerp(detected, moved.max(peak))
            })
            .collect(),
    )
}

/// 0 below 0, 1 above 1 and a cubic with flat ends between.
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `samples`, every 0.1 mm along a closed curve `length` mm long, with features
/// shorter than `stop` mm of arc removed and those longer than `pass` kept.
fn filter(samples: &[DVec3], length: f64, pass: f64, stop: f64) -> Vec<DVec3> {
    let n = samples.len();
    let top = (length / stop).floor() as usize;
    // Real signal: the cosine and sine sums of harmonic k give its amplitude, and
    // `top` is far below n / 2 at 0.1 mm spacing, so no harmonic aliases.
    let mean = samples.iter().sum::<DVec3>() / n as f64;
    let mut out = vec![mean; n];
    let pass = length / pass;
    for k in 1..=top {
        let gain = if (k as f64) <= pass {
            1.0
        } else {
            let t = (k as f64 - pass) / (top as f64 + 1.0 - pass);
            0.5 * (1.0 + (std::f64::consts::PI * t).cos())
        };
        if gain <= 0.0 {
            continue;
        }
        let (mut cos_sum, mut sin_sum) = (DVec3::ZERO, DVec3::ZERO);
        for (j, p) in samples.iter().enumerate() {
            let phase = TAU * (k * j % n) as f64 / n as f64;
            cos_sum += *p * phase.cos();
            sin_sum += *p * phase.sin();
        }
        let scale = 2.0 * gain / n as f64;
        for (j, o) in out.iter_mut().enumerate() {
            let phase = TAU * (k * j % n) as f64 / n as f64;
            *o += (cos_sum * phase.cos() + sin_sum * phase.sin()) * scale;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MarginTracer;
    use crate::synthetic::{ToothParams, prepared_tooth};
    use crate::testutil::Lcg;
    use glam::DQuat;
    use std::sync::Arc;

    /// Distance from `p` (in the die frame) to the unprocessed margin ring: the
    /// circle of radius 4 mm at height 8 mm about +Z.
    fn ring_distance(p: DVec3) -> f64 {
        (DVec3::new(p.x, p.y, 0.0).length() - 4.0).hypot(p.z - 8.0)
    }

    /// Distance from `p` to the closed polyline `curve`, measured to its segments.
    fn curve_distance(p: DVec3, curve: &[DVec3]) -> f64 {
        (0..curve.len())
            .map(|i| {
                super::super::segment_distance_squared(p, curve[i], curve[(i + 1) % curve.len()])
            })
            .fold(f64::INFINITY, f64::min)
            .sqrt()
    }

    /// Every vertex and segment midpoint of the closed polyline.
    fn with_midpoints(curve: &[DVec3]) -> Vec<DVec3> {
        (0..curve.len())
            .flat_map(|i| [curve[i], curve[i].lerp(curve[(i + 1) % curve.len()], 0.5)])
            .collect()
    }

    /// Whether every ray from `centre` in the plane perpendicular to `axis`
    /// crosses the closed loop once: its polar angle turns one way, strictly,
    /// through one full turn.
    fn star_shaped(points: &[DVec3], centre: DVec3, axis: DVec3) -> bool {
        let u = axis.any_orthonormal_vector();
        let v = axis.cross(u);
        let angle = |p: DVec3| (p - centre).dot(v).atan2((p - centre).dot(u));
        let steps: Vec<f64> = (0..points.len())
            .map(|i| {
                let d = angle(points[(i + 1) % points.len()]) - angle(points[i]);
                (d + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI
            })
            .collect();
        let total: f64 = steps.iter().sum();
        (total.abs() - TAU).abs() < 1e-6 && steps.iter().all(|s| s * total > 0.0)
    }

    /// A rounded shoulder prep whose scan carries isotropic noise and whose
    /// finish line jitters ±0.08 mm along the axis at 0.7 and 1.1 mm of arc,
    /// as ridges picked sector by sector on a real scan do, seen along an axis
    /// that is not aligned with any grid direction.
    #[test]
    fn proposed_margin_is_a_smooth_curve_on_the_true_finish_line() {
        let die = prepared_tooth(&ToothParams {
            smooth_passes: 3,
            ..Default::default()
        })
        .unwrap();
        let mut rng = Lcg::new(0x5eed);
        let rotation = DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 0.71);
        // The true margin is the rounded die's own finish line, before any noise.
        let truth = die.margin;
        let mut mesh = die.mesh;
        for p in &mut mesh.positions {
            let theta = p.y.atan2(p.x);
            let near = (1.0 - ring_distance(*p) / 0.6).max(0.0);
            let jitter = 0.12 * (29.0 * theta + 0.4).sin();
            *p += DVec3::Z * (near * jitter) + rng.unit_vec3() * (0.015 * rng.unit());
            *p = rotation * *p;
        }
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        let margin = tracer
            .propose(rotation * DVec3::new(0.3, -0.2, 12.0), rotation * DVec3::Z)
            .unwrap();
        let curve: Vec<DVec3> = margin
            .points
            .iter()
            .map(|p| rotation.inverse() * *p)
            .collect();
        // Both ways, at vertices and segment midpoints: mean and worst 3D distance.
        let stats = |from: &[DVec3], to: &[DVec3]| {
            let d: Vec<f64> = with_midpoints(from)
                .into_iter()
                .map(|p| curve_distance(p, to))
                .collect();
            (
                d.iter().sum::<f64>() / d.len() as f64,
                d.iter().copied().fold(0.0, f64::max),
            )
        };
        let (mean_to, max_to) = stats(&curve, &truth);
        let (mean_from, max_from) = stats(&truth, &curve);
        let centre = curve.iter().sum::<DVec3>() / curve.len() as f64;
        let star = star_shaped(&curve, centre, DVec3::Z);
        eprintln!(
            "curve to truth mean {mean_to:.4} max {max_to:.4}; truth to curve mean {mean_from:.4} max {max_from:.4}; star {star}"
        );
        // The raw ridge loop follows the jitter: worst 0.13 mm, mean 0.07 mm.
        assert!(
            max_to <= 0.11 && max_from <= 0.11,
            "worst {max_to} / {max_from} mm"
        );
        assert!(
            mean_to <= 0.055 && mean_from <= 0.055,
            "mean {mean_to} / {mean_from} mm"
        );
        assert!(star, "smoothed loop is not star-shaped about its centre");
        // The region barrier still follows the smoothed curve.
        let boundary = margin.region_boundary(tracer.mesh()).unwrap();
        let worst = boundary
            .iter()
            .map(|p| curve_distance(*p, &margin.points))
            .fold(0.0, f64::max);
        assert!(worst < 0.3, "region barrier is {worst} mm from the curve");
    }

    /// A shoulder prep whose finish line rises to a pointed interproximal
    /// scallop 2.5 mm high over 1 mm of arc either side, as between incisors, on
    /// a scan with isotropic noise seen along an axis aligned with no grid
    /// direction. The detected loop follows the true finish line with scan-like
    /// jitter. The low-pass filter alone rounds the tip off by about 0.2 mm along
    /// the axis, on a wall where projecting back onto the scan does not recover
    /// it; the stored margin must keep the tip on the finish line.
    #[test]
    fn smoothing_keeps_a_pointed_scallop_on_the_finish_line() {
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        // The tip is on a margin vertex of the die, so the true finish line has it.
        let (tip, height, half_width) = (TAU * 37.0 / 256.0, 2.5, 1.0);
        let rise = |theta: f64| {
            let turn = (theta - tip + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
            height * (1.0 - (4.0 * turn).abs() / half_width).max(0.0)
        };
        // The whole die above the core rises with the finish line.
        let lift = |p: DVec3| {
            let weight = ((p.truncate().length() - 1.0) / 1.5).clamp(0.0, 1.0);
            p + DVec3::Z * (weight * rise(p.y.atan2(p.x)))
        };
        let truth: Vec<DVec3> = die.margin.iter().map(|p| lift(*p)).collect();
        let mut rng = Lcg::new(0x5ca1);
        let rotation = DQuat::from_axis_angle(DVec3::new(-2.0, 1.0, 4.0).normalize(), 0.83);
        let mut mesh = die.mesh;
        for p in &mut mesh.positions {
            *p = rotation * (lift(*p) + rng.unit_vec3() * (0.015 * rng.unit()));
        }
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        // The detected loop: the true finish line every 0.1 mm, jittering
        // ±0.04 mm along the axis at 0.7 mm of arc, put on the scan.
        let detected: Vec<DVec3> = super::super::resample(&truth, 0.1)
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let jitter = 0.04 * (TAU * i as f64 / 7.0).sin() + 0.01 * (rng.unit() - 0.5);
                let p = rotation * (p + DVec3::Z * jitter);
                tracer.bvh.closest_point(tracer.mesh(), p).unwrap().point
            })
            .collect();
        let margin = super::super::Margin {
            points: detected,
            vertices: Vec::new(),
        };
        let smoothed = tracer
            .smoothed(
                &margin,
                rotation * DVec3::new(0.2, 0.3, 12.0),
                rotation * DVec3::Z,
            )
            .unwrap();
        let back = |curve: &[DVec3]| -> Vec<DVec3> {
            curve.iter().map(|p| rotation.inverse() * *p).collect()
        };
        let curve = back(&smoothed.points);
        let peak = DVec3::new(4.0 * tip.cos(), 4.0 * tip.sin(), 8.0 + height);
        // True 3D distances near the tip, at vertices and between them, both ways.
        let near_tip = |from: &[DVec3], to: &[DVec3]| {
            with_midpoints(from)
                .into_iter()
                .filter(|p| p.distance(peak) <= 1.0)
                .map(|p| curve_distance(p, to))
                .fold(0.0, f64::max)
        };
        let tip_error = curve_distance(peak, &curve);
        let (to_truth, from_truth) = (near_tip(&curve, &truth), near_tip(&truth, &curve));
        let everywhere = worst_both_ways(&curve, &truth);
        eprintln!(
            "scallop: tip {tip_error:.4}, near tip {to_truth:.4} / {from_truth:.4}, everywhere {everywhere:.4} mm"
        );
        // Without the bound: 0.22 mm at the tip.
        assert!(
            tip_error <= 0.1,
            "tip is {tip_error} mm off the finish line"
        );
        assert!(
            to_truth <= 0.1 && from_truth <= 0.1,
            "near the tip {to_truth} / {from_truth} mm"
        );
        assert!(everywhere <= 0.1, "{everywhere} mm off the finish line");
    }

    /// Distance across `axis` (Z) from `p` to the closed polyline `curve`, both
    /// flattened onto the plane perpendicular to the axis.
    fn outline_distance(p: DVec3, curve: &[DVec3]) -> f64 {
        let flat = |q: DVec3| DVec3::new(q.x, q.y, 0.0);
        (0..curve.len())
            .map(|i| {
                super::super::segment_distance_squared(
                    flat(p),
                    flat(curve[i]),
                    flat(curve[(i + 1) % curve.len()]),
                )
            })
            .fold(f64::INFINITY, f64::min)
            .sqrt()
    }

    /// An anterior-like rounded die whose finish line rises to pointed
    /// interproximal scallops mesially and distally, on a scan with isotropic
    /// noise seen along an axis aligned with no grid direction. The detected loop
    /// follows the true finish line with ±0.1 mm of radial jitter across the
    /// axis, put on the scan, as ridge picks wandering across the crease give.
    /// A crown is built on the margin's outline across the axis: the smoothed
    /// outline must be star-shaped about its centre and close to the true one
    /// everywhere, including along the scallops, while their tips keep their
    /// height.
    #[test]
    fn smoothing_removes_outline_noise_along_a_scalloped_margin() {
        let die = prepared_tooth(&ToothParams {
            smooth_passes: 3,
            ..Default::default()
        })
        .unwrap();
        let tips = [TAU * 37.0 / 256.0, TAU * 165.0 / 256.0];
        let (height, half_width) = (2.0, 1.0);
        let rise = |theta: f64| {
            tips.iter()
                .map(|tip| {
                    let turn =
                        (theta - tip + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
                    height * (1.0 - (4.0 * turn).abs() / half_width).max(0.0)
                })
                .sum::<f64>()
        };
        let lift = |p: DVec3| {
            let weight = ((p.truncate().length() - 1.0) / 1.5).clamp(0.0, 1.0);
            p + DVec3::Z * (weight * rise(p.y.atan2(p.x)))
        };
        let truth: Vec<DVec3> = die.margin.iter().map(|p| lift(*p)).collect();
        let mut rng = Lcg::new(0x0a71);
        let rotation = DQuat::from_axis_angle(DVec3::new(3.0, -1.0, 2.0).normalize(), 0.67);
        let mut mesh = die.mesh;
        for p in &mut mesh.positions {
            *p = rotation * (lift(*p) + rng.unit_vec3() * (0.015 * rng.unit()));
        }
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        let detected: Vec<DVec3> = super::super::resample(&truth, 0.1)
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let radial = DVec3::new(p.x, p.y, 0.0).normalize();
                let jitter = 0.07 * (TAU * i as f64 / 7.0).sin() + 0.03 * (2.0 * rng.unit() - 1.0);
                let p = rotation * (p + radial * jitter);
                tracer.bvh.closest_point(tracer.mesh(), p).unwrap().point
            })
            .collect();
        let back = |curve: &[DVec3]| -> Vec<DVec3> {
            curve.iter().map(|p| rotation.inverse() * *p).collect()
        };
        let worst_across = |from: &[DVec3], to: &[DVec3]| {
            with_midpoints(from)
                .into_iter()
                .map(|p| outline_distance(p, to))
                .fold(0.0, f64::max)
        };
        let raw = back(&detected);
        let raw_across = worst_across(&raw, &truth).max(worst_across(&truth, &raw));
        let margin = super::super::Margin {
            points: detected,
            vertices: Vec::new(),
        };
        let smoothed = tracer
            .smoothed(
                &margin,
                rotation * DVec3::new(0.2, 0.3, 12.0),
                rotation * DVec3::Z,
            )
            .unwrap();
        let curve = back(&smoothed.points);
        let across = worst_across(&curve, &truth).max(worst_across(&truth, &curve));
        let everywhere = worst_both_ways(&curve, &truth);
        let tip_error = tips
            .iter()
            .map(|tip| {
                // The tips are on margin vertices of the die.
                let towards = DVec3::new(tip.cos(), tip.sin(), 0.0);
                let off = |p: &DVec3| towards.cross(*p).z.abs();
                let peak = truth
                    .iter()
                    .filter(|p| p.dot(towards) > 0.0)
                    .min_by(|a, b| off(a).total_cmp(&off(b)))
                    .unwrap();
                curve_distance(*peak, &curve)
            })
            .fold(0.0, f64::max);
        let centre = curve.iter().sum::<DVec3>() / curve.len() as f64;
        let star = star_shaped(&curve, centre, DVec3::Z);
        eprintln!(
            "outline noise: detected {raw_across:.4}, smoothed across {across:.4}, 3D {everywhere:.4}, tips {tip_error:.4} mm; star {star}"
        );
        assert!(star, "smoothed outline is not star-shaped about its centre");
        assert!(
            across <= 0.05,
            "smoothed outline is {across} mm across the axis from the true one"
        );
        assert!(everywhere <= 0.12, "{everywhere} mm off the finish line");
        assert!(tip_error <= 0.12, "a scallop tip is {tip_error} mm off");
    }

    /// A closed curve sampled every 0.01 mm, carrying `jitter` mm of in-plane
    /// normal jitter at `waves` periods along it.
    fn sampled(curve: impl Fn(f64) -> DVec3, jitter: f64, waves: f64) -> (Vec<DVec3>, Vec<DVec3>) {
        let coarse = (0..4000)
            .map(|i| curve(TAU * i as f64 / 4000.0))
            .collect::<Vec<_>>();
        let length: f64 = (0..coarse.len())
            .map(|i| coarse[i].distance(coarse[(i + 1) % coarse.len()]))
            .sum();
        let n = (length / 0.01) as usize;
        let truth: Vec<DVec3> = (0..n).map(|i| curve(TAU * i as f64 / n as f64)).collect();
        let noisy = (0..n)
            .step_by(7)
            .map(|i| {
                let t = (truth[(i + 1) % n] - truth[(i + n - 1) % n]).normalize();
                let normal = DVec3::Z.cross(t);
                truth[i] + normal * (jitter * (waves * TAU * i as f64 / n as f64).sin())
            })
            .collect();
        (truth, noisy)
    }

    fn worst_both_ways(a: &[DVec3], b: &[DVec3]) -> f64 {
        let one = |from: &[DVec3], to: &[DVec3]| {
            with_midpoints(from)
                .into_iter()
                .map(|p| curve_distance(p, to))
                .fold(0.0, f64::max)
        };
        one(a, b).max(one(b, a))
    }

    /// A horseshoe outline is star-shaped about no centre; the filter keeps its
    /// shape (it is not forced round) and removes 0.6 mm jitter.
    #[test]
    fn low_pass_keeps_an_outline_that_is_not_star_shaped() {
        // Centre line: an arc of radius 8 over 240 degrees; offset 2.5 mm both
        // ways and closed by semicircular caps.
        let horseshoe = |s: f64| {
            let (arc, cap, span) = (8.0, 2.5, 4.0 * TAU / 6.0);
            let lengths = [
                (arc + cap) * span,
                cap * std::f64::consts::PI,
                (arc - cap) * span,
            ];
            let total = lengths[0] + 2.0 * lengths[1] + lengths[2];
            let mut d = s / TAU * total;
            let at = |a: f64, r: f64| DVec3::new(r * a.cos(), r * a.sin(), 3.0);
            let end = |a: f64| DVec3::new(arc * a.cos(), arc * a.sin(), 3.0);
            if d < lengths[0] {
                return at(-span / 2.0 + d / (arc + cap), arc + cap);
            }
            d -= lengths[0];
            if d < lengths[1] {
                let a = span / 2.0;
                let phi = d / cap;
                let out = DVec3::new(a.cos(), a.sin(), 0.0);
                let tangent = DVec3::new(-a.sin(), a.cos(), 0.0);
                return end(a) + (out * phi.cos() + tangent * phi.sin()) * cap;
            }
            d -= lengths[1];
            if d < lengths[2] {
                return at(span / 2.0 - d / (arc - cap), arc - cap);
            }
            d -= lengths[2];
            let a = -span / 2.0;
            let phi = d / cap;
            let out = DVec3::new(a.cos(), a.sin(), 0.0);
            let tangent = DVec3::new(-a.sin(), a.cos(), 0.0);
            end(a) + (-out * phi.cos() - tangent * phi.sin()) * cap
        };
        let (truth, noisy) = sampled(horseshoe, 0.08, 140.0);
        let smooth = low_pass(&noisy, DVec3::Z).unwrap();
        let error = worst_both_ways(&smooth, &truth);
        let centre = smooth.iter().sum::<DVec3>() / smooth.len() as f64;
        eprintln!("horseshoe: {error:.4} mm");
        assert!(
            error <= 0.04,
            "horseshoe smoothed {error} mm from the true outline"
        );
        assert!(!star_shaped(&smooth, centre, DVec3::Z));
    }

    #[test]
    fn low_pass_handles_duplicates_and_degenerate_loops() {
        let circle = |t: f64| DVec3::new(4.0 * t.cos(), 4.0 * t.sin(), 8.0);
        let (truth, noisy) = sampled(circle, 0.08, 29.0);
        let smooth = low_pass(&noisy, DVec3::Z).unwrap();
        let error = worst_both_ways(&smooth, &truth);
        assert!(error <= 0.01, "circle smoothed {error} mm from the truth");
        // Repeated points add no arc length, so they change nothing.
        let doubled: Vec<DVec3> = noisy.iter().flat_map(|p| [*p, *p]).collect();
        let again = low_pass(&doubled, DVec3::Z).unwrap();
        assert_eq!(again.len(), smooth.len());
        assert!(
            again
                .iter()
                .zip(&smooth)
                .all(|(a, b)| a.distance(*b) < 1e-9)
        );
        // A loop under three stop wavelengths long, too few or nonfinite points.
        let tiny: Vec<DVec3> = (0..40)
            .map(|i| circle(TAU * i as f64 / 40.0) * 0.15)
            .collect();
        assert!(low_pass(&tiny, DVec3::Z).is_none());
        assert!(low_pass(&noisy[..2], DVec3::Z).is_none());
        let mut broken = noisy.clone();
        broken[3].x = f64::NAN;
        assert!(low_pass(&broken, DVec3::Z).is_none());
        let flat = vec![DVec3::ONE; 10];
        assert!(low_pass(&flat, DVec3::Z).is_none());
        // An axis with no direction splits nothing.
        assert!(low_pass(&noisy, DVec3::ZERO).is_none());
        assert!(low_pass(&noisy, DVec3::new(0.0, f64::NAN, 1.0)).is_none());
        // The axis need not be a unit vector.
        let scaled = low_pass(&noisy, DVec3::Z * 3.0).unwrap();
        assert!(
            scaled
                .iter()
                .zip(&smooth)
                .all(|(a, b)| a.distance(*b) < 1e-9)
        );
    }
}
