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
    /// Triangle indices of the walls, top chamfer and flat top, excluding the shoulder below the margin.
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
}
