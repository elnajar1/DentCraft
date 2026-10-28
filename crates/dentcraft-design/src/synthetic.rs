//! Independently generated demonstration dies; no scans or external tooth libraries.

use std::f64::consts::TAU;

use dentcraft_mesh::TriMesh;
use glam::{DQuat, DVec3};

use crate::DesignError;

/// Dimensions of a conical preparation with a top chamfer and shoulder below the margin.
#[derive(Clone, Debug)]
pub struct StumpParams {
    /// Wall radius at the margin, in mm.
    pub base_radius: f64,
    /// Radius of the unchamfered cone at its top; larger than the base gives negative taper.
    pub top_radius: f64,
    /// Height from the margin to the flat top, in mm.
    pub height: f64,
    /// Axial and nominal radial width of the top chamfer, in mm.
    pub top_chamfer: f64,
    /// Margin height above the bottom of the plate, in mm (must exceed 0.5).
    pub margin_height: f64,
    /// Half-width of the square plate, in mm; must enclose the shoulder and stump.
    pub plate_half_width: f64,
    /// Circumferential segments, at least 96 and divisible by eight (to include square corners).
    pub segments: usize,
    /// Number of axial wall subdivisions, at least one.
    pub wall_steps: usize,
    /// Rotation of the entire die about X, in degrees.
    pub tilt_deg: f64,
}

impl Default for StumpParams {
    fn default() -> Self {
        Self {
            base_radius: 3.0,
            top_radius: 3.0 - 4.0 * 6.0_f64.to_radians().tan(),
            height: 4.0,
            top_chamfer: 0.15,
            margin_height: 1.0,
            plate_half_width: 5.0,
            segments: 120,
            wall_steps: 24,
            tilt_deg: 0.0,
        }
    }
}

/// A closed demonstration die and its open preparation patch.
#[derive(Clone, Debug)]
pub struct PreparedStump {
    /// The complete closed die, including the square base plate.
    pub mesh: TriMesh,
    /// Exact margin vertices, counter-clockwise about the stump axis, without a repeated endpoint.
    pub margin: Vec<DVec3>,
    /// Triangle indices of the preparation: everything at and above the margin.
    pub region: Vec<u32>,
    /// Unit axis after the requested rotation.
    pub axis: DVec3,
}

/// Build a faceted truncated cone, chamfered top, shoulder ring and 0.3 mm thick square plate.
/// The flat top is exactly planar. Invalid dimensions return [`DesignError::InvalidParameter`].
pub fn prepared_stump(params: &StumpParams) -> Result<PreparedStump, DesignError> {
    validate(params)?;
    let n = params.segments;
    let mut mesh = TriMesh::default();
    add_ring(&mut mesh, n, params.plate_half_width, 0.0, true);
    add_ring(&mut mesh, n, params.plate_half_width, 0.3, true);
    add_ring(
        &mut mesh,
        n,
        params.base_radius + 0.3,
        params.margin_height - 0.2,
        false,
    );
    let margin_start = mesh.positions.len();
    add_ring(
        &mut mesh,
        n,
        params.base_radius,
        params.margin_height,
        false,
    );
    let prep_start = mesh.triangles.len();
    for k in 1..=params.wall_steps {
        let z = (params.height - params.top_chamfer) * k as f64 / params.wall_steps as f64;
        let radius =
            params.base_radius + (params.top_radius - params.base_radius) * z / params.height;
        add_ring(&mut mesh, n, radius, params.margin_height + z, false);
    }
    add_ring(
        &mut mesh,
        n,
        params.top_radius - params.top_chamfer,
        params.margin_height + params.height,
        false,
    );
    let top_start = mesh.positions.len() - n;
    let center = mesh.positions.len() as u32;
    mesh.positions
        .push(DVec3::Z * (params.margin_height + params.height));
    for i in 0..n {
        mesh.triangles.push([
            center,
            (top_start + i) as u32,
            (top_start + (i + 1) % n) as u32,
        ]);
    }
    let region = (prep_start as u32..mesh.triangles.len() as u32).collect();
    let bottom = mesh.positions.len() as u32;
    mesh.positions.push(DVec3::ZERO);
    for i in 0..n {
        mesh.triangles
            .push([bottom, ((i + 1) % n) as u32, i as u32]);
    }
    let rotation = DQuat::from_rotation_x(params.tilt_deg.to_radians());
    for point in &mut mesh.positions {
        *point = rotation * *point;
    }
    let margin = mesh.positions[margin_start..margin_start + n].to_vec();
    Ok(PreparedStump {
        mesh,
        margin,
        region,
        axis: rotation * DVec3::Z,
    })
}

fn validate(p: &StumpParams) -> Result<(), DesignError> {
    let values = [
        p.base_radius,
        p.top_radius,
        p.height,
        p.top_chamfer,
        p.margin_height,
        p.plate_half_width,
        p.tilt_deg,
    ];
    if values.iter().any(|v| !v.is_finite())
        || p.base_radius <= 0.0
        || p.top_chamfer <= 0.0
        || p.top_radius <= p.top_chamfer
        || p.height <= p.top_chamfer
        || p.margin_height <= 0.5
        || p.plate_half_width <= (p.base_radius + 0.3).max(p.top_radius)
        || p.segments < 96
        || p.segments % 8 != 0
        || p.wall_steps == 0
        || p.segments
            .checked_mul(p.wall_steps.saturating_add(6))
            .is_none_or(|n| n > 2_000_000)
    {
        return Err(DesignError::InvalidParameter(
            "stump dimensions or sampling",
        ));
    }
    Ok(())
}

fn add_ring(mesh: &mut TriMesh, n: usize, radius: f64, z: f64, square: bool) {
    let start = mesh.positions.len();
    for i in 0..n {
        let (sin, cos) = (TAU * i as f64 / n as f64).sin_cos();
        let r = if square {
            radius / sin.abs().max(cos.abs())
        } else {
            radius
        };
        mesh.positions.push(DVec3::new(r * cos, r * sin, z));
    }
    if start >= n {
        for i in 0..n {
            let a = (start - n + i) as u32;
            let b = (start - n + (i + 1) % n) as u32;
            let c = (start + (i + 1) % n) as u32;
            let d = (start + i) as u32;
            mesh.triangles.extend([[a, b, c], [a, c, d]]);
        }
    }
}

/// Finish-line profile of the independently generated preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishLine {
    /// A flat 0.7 mm ledge at the margin height with a 0.2 mm rounded concave
    /// inner corner joining the axial wall; the margin is the convex outer edge.
    Shoulder,
    /// A concave quarter circle of radius 0.9 mm from the axial wall down to the
    /// margin; the margin is again the convex outer edge where the flare of the
    /// unprepared root meets the chamfer.
    Chamfer,
}

/// Sampling and orientation of a realistic rotational preparation, in millimetres.
#[derive(Clone, Debug)]
pub struct ToothParams {
    pub finish: FinishLine,
    pub segments: usize,
    pub wall_steps: usize,
    pub tilt_deg: f64,
}

impl Default for ToothParams {
    fn default() -> Self {
        Self {
            finish: FinishLine::Shoulder,
            segments: 256,
            wall_steps: 64,
            tilt_deg: 0.0,
        }
    }
}

/// A realistic rotational preparation on an unprepared root.
///
/// From the bottom up: a square base plate (half-width 6 mm, 0.3 mm thick), a root
/// that flares outward downward by 3° up to radius 4 mm at height 8 mm, the chosen
/// finish line, a 6° tapered axial wall and a rounded occlusal top with a flat top
/// disc at height 12 mm. The margin is the convex outer edge of the finish line:
/// the ring at radius 4 mm, height 8 mm, returned exactly (rotated by `tilt_deg`
/// about X). The prep region contains every triangle at and above the margin ring
/// (finish line, wall, occlusal cap) and nothing of the plate or root. Invalid
/// sampling or tilt returns [`DesignError::InvalidParameter`].
pub fn prepared_tooth(p: &ToothParams) -> Result<PreparedStump, DesignError> {
    if p.segments < 96
        || p.segments % 8 != 0
        || p.wall_steps < 2
        || !p.tilt_deg.is_finite()
        || p.segments
            .checked_mul(p.wall_steps.saturating_add(160))
            .is_none_or(|n| n > 2_000_000)
    {
        return Err(DesignError::InvalidParameter("tooth sampling or tilt"));
    }
    let n = p.segments;
    let mut mesh = TriMesh::default();
    add_ring(&mut mesh, n, 6.0, 0.0, true);
    add_ring(&mut mesh, n, 6.0, 0.3, true);
    for i in 0..=80 {
        let z = 0.5 + 7.5 * i as f64 / 80.0;
        add_ring(
            &mut mesh,
            n,
            4.0 + (8.0 - z) * 3.0_f64.to_radians().tan(),
            z,
            false,
        );
    }
    let margin_start = mesh.positions.len() - n;
    let prep_start = mesh.triangles.len();
    let wall_z = match p.finish {
        FinishLine::Shoulder => {
            for i in 1..=14 {
                add_ring(&mut mesh, n, 4.0 - 0.7 * i as f64 / 14.0, 8.0, false);
            }
            for i in 1..=10 {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / 10.0;
                add_ring(
                    &mut mesh,
                    n,
                    3.3 - 0.2 * a.sin(),
                    8.0 + 0.2 * (1.0 - a.cos()),
                    false,
                );
            }
            8.2
        }
        FinishLine::Chamfer => {
            for i in 1..=24 {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / 24.0;
                add_ring(
                    &mut mesh,
                    n,
                    4.0 - 0.9 * a.sin(),
                    8.0 + 0.9 * (1.0 - a.cos()),
                    false,
                );
            }
            8.9
        }
    };
    let slope = 6.0_f64.to_radians().tan();
    for i in 1..=p.wall_steps {
        let z = wall_z + (11.5 - wall_z) * i as f64 / p.wall_steps as f64;
        add_ring(&mut mesh, n, 3.1 - (z - wall_z) * slope, z, false);
    }
    let r = 3.1 - (11.5 - wall_z) * slope;
    for i in 1..=16 {
        let a = std::f64::consts::FRAC_PI_2 * i as f64 / 16.0;
        add_ring(
            &mut mesh,
            n,
            r - 0.5 * (1.0 - a.cos()),
            11.5 + 0.5 * a.sin(),
            false,
        );
    }
    let last = mesh.positions.len() - n;
    let top = mesh.positions.len() as u32;
    mesh.positions.push(DVec3::Z * 12.0);
    for i in 0..n {
        mesh.triangles
            .push([top, (last + i) as u32, (last + (i + 1) % n) as u32]);
    }
    let region = (prep_start as u32..mesh.triangles.len() as u32).collect();
    let bottom = mesh.positions.len() as u32;
    mesh.positions.push(DVec3::ZERO);
    for i in 0..n {
        mesh.triangles
            .push([bottom, ((i + 1) % n) as u32, i as u32]);
    }
    let rotation = DQuat::from_rotation_x(p.tilt_deg.to_radians());
    for v in &mut mesh.positions {
        *v = rotation * *v;
    }
    let margin = mesh.positions[margin_start..margin_start + n].to_vec();
    Ok(PreparedStump {
        mesh,
        margin,
        region,
        axis: rotation * DVec3::Z,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_stump_has_exact_margin_and_flat_top() {
        let p = StumpParams::default();
        let die = prepared_stump(&p).unwrap();
        assert!(die.mesh.is_closed());
        assert!(die.mesh.is_consistently_oriented());
        assert_eq!(die.mesh.connected_components().len(), 1);
        assert_eq!(die.margin.len(), 120);
        for point in &die.margin {
            assert_eq!(point.z, p.margin_height);
            assert!(die.mesh.positions.contains(point));
            assert!((point.truncate().length() - p.base_radius).abs() < 1e-12);
        }
        let prep = die.mesh.extract_triangles(&die.region);
        assert_eq!(prep.boundary_loops().len(), 1);
        for i in 0..prep.triangles.len() {
            assert!(prep.triangle_cross(i).length() > 0.0);
        }
        for &i in die.region.iter().rev().take(p.segments) {
            assert!(
                die.mesh
                    .triangle(i as usize)
                    .iter()
                    .all(|v| v.z == p.margin_height + p.height)
            );
        }
    }

    #[test]
    fn tilted_negative_taper_and_invalid_dimensions() {
        let mut p = StumpParams {
            top_radius: 3.5,
            tilt_deg: 15.0,
            ..Default::default()
        };
        let die = prepared_stump(&p).unwrap();
        assert!((die.axis.dot(DVec3::Z).acos().to_degrees() - 15.0).abs() < 1e-12);
        assert!(die.mesh.is_closed());
        p.height = 0.0;
        assert!(prepared_stump(&p).is_err());
        p.height = f64::NAN;
        assert!(prepared_stump(&p).is_err());
        p = StumpParams {
            segments: 95,
            ..Default::default()
        };
        assert!(prepared_stump(&p).is_err());
    }

    #[test]
    fn prepared_tooth_is_closed_with_the_margin_as_outer_edge() {
        for finish in [FinishLine::Shoulder, FinishLine::Chamfer] {
            let p = ToothParams {
                finish,
                tilt_deg: 12.0,
                ..Default::default()
            };
            let die = prepared_tooth(&p).unwrap();
            assert!(die.mesh.is_closed(), "{finish:?}");
            assert!(die.mesh.is_consistently_oriented(), "{finish:?}");
            assert_eq!(die.mesh.connected_components().len(), 1);
            assert_eq!(die.margin.len(), p.segments);
            for &point in &die.margin {
                assert!((point.dot(die.axis) - 8.0).abs() < 1e-9, "{finish:?}");
                let radial = point - die.axis * point.dot(die.axis);
                assert!((radial.length() - 4.0).abs() < 1e-9, "{finish:?}");
            }
            // The prep region is exactly at and above the margin ring.
            for &t in &die.region {
                for v in die.mesh.triangles[t as usize] {
                    assert!(
                        die.mesh.positions[v as usize].dot(die.axis) >= 8.0 - 1e-9,
                        "{finish:?}"
                    );
                }
            }
            assert!(!die.region.is_empty());
            // The prep patch is a disc whose only boundary is the margin ring.
            let prep = die.mesh.extract_triangles(&die.region);
            let loops = prep.boundary_loops();
            assert_eq!(loops.len(), 1, "{finish:?}");
            assert_eq!(loops[0].len(), p.segments);
            for &v in &loops[0] {
                assert!(
                    die.margin.contains(&prep.positions[v as usize]),
                    "{finish:?}"
                );
            }
        }
    }

    #[test]
    fn prepared_tooth_rejects_invalid_sampling() {
        assert!(
            prepared_tooth(&ToothParams {
                segments: 95,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            prepared_tooth(&ToothParams {
                wall_steps: 1,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            prepared_tooth(&ToothParams {
                tilt_deg: f64::NAN,
                ..Default::default()
            })
            .is_err()
        );
    }
}
