//! Independently generated demonstration dies; no scans or external tooth libraries.

use std::f64::consts::{PI, TAU};

use dentcraft_mesh::TriMesh;
use glam::{DQuat, DVec3};

use crate::DesignError;

/// Independently generated anatomical cap in the +Z occlusal, +Y facial frame.
/// The cervical ellipse is centered at zero. Four smooth Gaussian cusp bumps
/// modulate an ellipsoidal dome; no external anatomy data is used. Returns the
/// open cap and its counter-clockwise cervical loop. Dimensions are in mm.
pub fn crown_template(
    radius: f64,
    height: f64,
    segments: usize,
    rings: usize,
) -> Result<(TriMesh, Vec<u32>), DesignError> {
    if !radius.is_finite()
        || !height.is_finite()
        || radius <= 0.0
        || height <= 0.0
        || segments < 16
        || rings < 4
        || segments.checked_mul(rings).is_none_or(|n| n > 100_000)
    {
        return Err(DesignError::InvalidParameter(
            "crown template dimensions or sampling",
        ));
    }
    let mut mesh = TriMesh::default();
    for j in 0..rings {
        let phi = std::f64::consts::FRAC_PI_2 * j as f64 / rings as f64;
        let r = radius * phi.cos();
        for i in 0..segments {
            let angle = TAU * i as f64 / segments as f64;
            let (x, y) = (r * angle.cos(), r * angle.sin());
            let bump: f64 = [-1.0, 1.0]
                .iter()
                .flat_map(|&sx| {
                    [-1.0, 1.0].iter().map(move |&sy| {
                        let d2 =
                            (x / radius - sx * 0.38).powi(2) + (y / radius - sy * 0.38).powi(2);
                        0.35 * (-d2 / 0.065).exp()
                    })
                })
                .sum();
            mesh.positions.push(DVec3::new(
                x,
                y * 0.92,
                height * phi.sin() + bump * phi.sin(),
            ));
        }
        if j > 0 {
            for i in 0..segments {
                let a = ((j - 1) * segments + i) as u32;
                let b = ((j - 1) * segments + (i + 1) % segments) as u32;
                let c = (j * segments + (i + 1) % segments) as u32;
                let d = (j * segments + i) as u32;
                mesh.triangles.extend([[a, b, c], [a, c, d]]);
            }
        }
    }
    let top = mesh.positions.len() as u32;
    mesh.positions
        .push(DVec3::Z * (height + 4.0 * 0.35 * (-2.0 * 0.38_f64.powi(2) / 0.065).exp()));
    for i in 0..segments {
        mesh.triangles.push([
            ((rings - 1) * segments + i) as u32,
            ((rings - 1) * segments + (i + 1) % segments) as u32,
            top,
        ]);
    }
    Ok((mesh, (0..segments as u32).collect()))
}

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
        || !p.segments.is_multiple_of(8)
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

/// Sampling, finishing and scan environment of a realistic rotational preparation.
#[derive(Clone, Debug)]
pub struct ToothParams {
    pub finish: FinishLine,
    pub segments: usize,
    pub wall_steps: usize,
    pub tilt_deg: f64,
    /// Vertical stretch of the whole die: the margin moves to `8 · height_scale` and
    /// the occlusal top to `12 · height_scale` along the axis; radii stay unchanged.
    /// A stretched finish line is a duller crease, like a tall preparation.
    pub height_scale: f64,
    /// Extra axial wall height in mm (a tall prep of `3.3 + extra_wall` mm), with the
    /// margin still at height 8 and the finish line unstretched.
    pub extra_wall: f64,
    /// Laplacian smoothing passes over the whole die: a rounded finish-line crease.
    pub smooth_passes: u32,
    /// Amplitude in mm of deterministic normal noise, added after smoothing.
    pub noise_mm: f64,
    /// Replace the bare root and plate with a gingiva cuff around the root below the
    /// margin: a rounded cuff crest 1.6 mm below the margin, a horizontal slab out to
    /// radius 9.5 and an outer wall down to the base plane at height 0.3.
    pub gingiva: bool,
    /// Clear gap in mm to a neighbouring unprepared tooth centred mesially at height
    /// 6.5; `0` adds no neighbour. The neighbour is a separate component.
    pub neighbour_gap: f64,
}

impl Default for ToothParams {
    fn default() -> Self {
        Self {
            finish: FinishLine::Shoulder,
            segments: 256,
            wall_steps: 64,
            tilt_deg: 0.0,
            height_scale: 1.0,
            extra_wall: 0.0,
            smooth_passes: 0,
            noise_mm: 0.0,
            gingiva: false,
            neighbour_gap: 0.0,
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
/// (finish line, wall, occlusal cap) and nothing of the plate or root. With
/// `gingiva` the plate and bare root are replaced by a cuff, slab and outer wall;
/// with `neighbour_gap` a separate ellipsoid tooth is merged beside the root. The
/// mesh is closed, and [`ToothParams::smooth_passes`], [`ToothParams::noise_mm`]
/// and [`ToothParams::height_scale`] are applied before the tilt, so `margin` holds
/// the processed ring positions. Invalid sampling or parameters returns
/// [`DesignError::InvalidParameter`].
pub fn prepared_tooth(p: &ToothParams) -> Result<PreparedStump, DesignError> {
    if p.segments < 96
        || !p.segments.is_multiple_of(8)
        || p.wall_steps < 2
        || !p.tilt_deg.is_finite()
        || !(0.5..=3.0).contains(&p.height_scale)
        || !(0.0..=6.0).contains(&p.extra_wall)
        || p.smooth_passes > 16
        || !(0.0..=0.05).contains(&p.noise_mm)
        || !(0.0..=2.0).contains(&p.neighbour_gap)
        || p.segments
            .checked_mul(p.wall_steps.saturating_add(200))
            .is_none_or(|n| n > 2_000_000)
    {
        return Err(DesignError::InvalidParameter("tooth sampling or tilt"));
    }
    let n = p.segments;
    let root_radius = |z: f64| 4.0 + (8.0 - z) * 3.0_f64.to_radians().tan();
    let mut mesh = TriMesh::default();
    if p.gingiva {
        add_ring(&mut mesh, n, 9.5, 0.3, false);
        for i in 1..=6 {
            add_ring(&mut mesh, n, 9.5, 0.3 + 5.7 * i as f64 / 6.0, false);
        }
        let foot = root_radius(6.4);
        for i in (0..=8).rev() {
            let a = std::f64::consts::FRAC_PI_2 * i as f64 / 8.0;
            add_ring(
                &mut mesh,
                n,
                foot + (4.5 - foot) * (1.0 - a.cos()),
                6.4 - 0.4 * a.sin(),
                false,
            );
        }
        for i in 1..=24 {
            let z = 6.4 + 1.6 * i as f64 / 24.0;
            add_ring(&mut mesh, n, root_radius(z), z, false);
        }
    } else {
        add_ring(&mut mesh, n, 6.0, 0.0, true);
        add_ring(&mut mesh, n, 6.0, 0.3, true);
        for i in 0..=80 {
            let z = 0.5 + 7.5 * i as f64 / 80.0;
            add_ring(&mut mesh, n, root_radius(z), z, false);
        }
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
    let shoulder_z = 11.5 + p.extra_wall;
    let apex_z = 12.0 + p.extra_wall;
    for i in 1..=p.wall_steps {
        let z = wall_z + (shoulder_z - wall_z) * i as f64 / p.wall_steps as f64;
        add_ring(&mut mesh, n, 3.1 - (z - wall_z) * slope, z, false);
    }
    let r = 3.1 - (shoulder_z - wall_z) * slope;
    for i in 1..=16 {
        let a = std::f64::consts::FRAC_PI_2 * i as f64 / 16.0;
        add_ring(
            &mut mesh,
            n,
            r - 0.5 * (1.0 - a.cos()),
            shoulder_z + 0.5 * a.sin(),
            false,
        );
    }
    let last = mesh.positions.len() - n;
    let top = mesh.positions.len() as u32;
    mesh.positions.push(DVec3::Z * apex_z);
    for i in 0..n {
        mesh.triangles
            .push([top, (last + i) as u32, (last + (i + 1) % n) as u32]);
    }
    let region = (prep_start as u32..mesh.triangles.len() as u32).collect();
    // Base cap: a fan to the plate ring at height 0, or to the outer wall's foot.
    let bottom = mesh.positions.len() as u32;
    if p.gingiva {
        mesh.positions.push(DVec3::new(0.0, 0.0, 0.3));
        for i in 0..n {
            mesh.triangles
                .push([bottom, ((i + 1) % n) as u32, i as u32]);
        }
    } else {
        mesh.positions.push(DVec3::ZERO);
        for i in 0..n {
            mesh.triangles
                .push([bottom, ((i + 1) % n) as u32, i as u32]);
        }
    }
    if p.neighbour_gap > 0.0 {
        let distance = root_radius(6.5) + p.neighbour_gap + 3.5;
        let mut neighbour = TriMesh::uv_sphere(1.0, 24, 12);
        for q in &mut neighbour.positions {
            *q = DVec3::new(q.x * 3.5, q.y * 3.5, q.z * 5.5) + DVec3::new(0.0, distance, 6.5);
        }
        mesh.merge(&neighbour);
    }
    if p.smooth_passes > 0 {
        mesh.laplacian_smooth(p.smooth_passes, 0.5, None, true);
    }
    if p.height_scale != 1.0 {
        for v in &mut mesh.positions {
            *v = DVec3::new(v.x, v.y, v.z * p.height_scale);
        }
    }
    if p.noise_mm > 0.0 {
        let normals = mesh.vertex_normals();
        let mut state = 0x5EED_51D0_5EED_51D0_u64;
        for (v, normal) in mesh.positions.iter_mut().zip(&normals) {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let unit = ((state >> 11) as f64) / ((1_u64 << 53) as f64);
            *v += normal * ((2.0 * unit - 1.0) * p.noise_mm);
        }
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

/// A synthetic full arch: a horseshoe gingiva band with 14 ellipsoid teeth in
/// mirror-symmetric stations. The open variant is the occlusal-side scan surface (the
/// band's underside is missing, its mean normal points up); the closed variant is a
/// stone model: the same band as a closed tube over a solid horseshoe base with a flat
/// bottom, teeth sunk into it. Components overlap geometrically, like a merged scan;
/// each is oriented outward.
///
/// All heights are scaled by `height_scale` (taller gingiva and teeth), and with
/// `dense_molar` the most posterior right molar is meshed at higher density instead of
/// being re-meshed coarsely, the way a re-scanned tooth appears in a merged model.
fn build_arch(closed: bool, dense_molar: bool, height_scale: f64) -> TriMesh {
    let mut mesh = TriMesh::default();
    const STATIONS: usize = 40;
    let phi = |k: usize| (-25.0 + 230.0 * k as f64 / (STATIONS - 1) as f64).to_radians();
    let midline = |a: f64| DVec3::new(22.0 * a.cos(), 30.0 * a.sin(), 0.0);
    let radial = |a: f64| DVec3::new(a.cos(), a.sin(), 0.0);
    // Gingiva band: rings of cross-sections along the midline. The open variant keeps
    // the upper half arc only (the scanned side); the closed variant uses the full
    // ellipse and caps the two arm ends.
    let sections: usize = if closed { 16 } else { 12 };
    let cross = |j: usize| {
        if closed {
            let psi = TAU * j as f64 / sections as f64;
            (4.0 * psi.cos(), 3.0 * psi.sin())
        } else {
            let psi = -std::f64::consts::FRAC_PI_2 + PI * j as f64 / (sections - 1) as f64;
            (4.0 * psi.sin(), 3.0 * psi.cos())
        }
    };
    let mut ring_start = Vec::with_capacity(STATIONS);
    for k in 0..STATIONS {
        let a = phi(k);
        let center = midline(a);
        let n = radial(a);
        ring_start.push(mesh.positions.len());
        for j in 0..sections {
            let (along, up) = cross(j);
            mesh.positions.push(center + n * along + DVec3::Z * up);
        }
    }
    for k in 0..STATIONS - 1 {
        for j in 0..sections {
            let next = (j + 1) % sections;
            if !closed && next == 0 {
                continue;
            }
            let a = (ring_start[k] + j) as u32;
            let b = (ring_start[k] + next) as u32;
            let c = (ring_start[k + 1] + next) as u32;
            let d = (ring_start[k + 1] + j) as u32;
            mesh.triangles.extend([[a, b, c], [a, c, d]]);
        }
    }
    if closed {
        // Cap the tube's two arm ends with fans.
        for k in [0, STATIONS - 1] {
            let center_index = mesh.positions.len() as u32;
            mesh.positions.push(midline(phi(k)));
            for j in 0..sections {
                mesh.triangles.push([
                    center_index,
                    (ring_start[k] + (j + 1) % sections) as u32,
                    (ring_start[k] + j) as u32,
                ]);
            }
        }
        // Solid horseshoe base: prism over the annular footprint between an outer
        // and inner half-ellipse, from z = -7 up to z = -1, so the band (bottom at
        // z = -3) is buried in it. Flat bottom, curved top edges: a stone model base.
        let outer = |a: f64| (27.5 * a.cos(), 35.5 * a.sin());
        let inner = |a: f64| (16.5 * a.cos(), 24.5 * a.sin());
        let boundary: Vec<(f64, f64)> = (0..STATIONS)
            .map(|k| outer(phi(k)))
            .chain((0..STATIONS).rev().map(|k| inner(phi(k))))
            .collect();
        let m = boundary.len();
        let top_start = mesh.positions.len();
        for p in &boundary {
            mesh.positions.push(DVec3::new(p.0, p.1, -1.0));
        }
        let bottom_start = mesh.positions.len();
        for p in &boundary {
            mesh.positions.push(DVec3::new(p.0, p.1, -7.0));
        }
        for i in 0..m {
            let a = (top_start + i) as u32;
            let b = (top_start + (i + 1) % m) as u32;
            let c = (bottom_start + (i + 1) % m) as u32;
            let d = (bottom_start + i) as u32;
            mesh.triangles.extend([[a, b, c], [a, c, d]]);
        }
        for i in 0..STATIONS - 1 {
            let o0 = (top_start + i) as u32;
            let o1 = (top_start + i + 1) as u32;
            let i1 = (top_start + m - 2 - i) as u32;
            let i0 = (top_start + m - 1 - i) as u32;
            // Top cap: normal +Z; bottom cap: flipped.
            mesh.triangles.extend([[o0, i0, i1], [o0, i1, o1]]);
            let b_o0 = (bottom_start + i) as u32;
            let b_o1 = (bottom_start + i + 1) as u32;
            let b_i1 = (bottom_start + m - 2 - i) as u32;
            let b_i0 = (bottom_start + m - 1 - i) as u32;
            mesh.triangles
                .extend([[b_o0, b_i1, b_i0], [b_o0, b_o1, b_i1]]);
        }
    }
    // Teeth: 14 ellipsoids in mirror-symmetric stations, sunk into the band. The
    // dense molar keeps its shape but is sampled at roughly 4x the vertex density.
    for k in 0..7 {
        for sign in [1.0, -1.0] {
            let a = (90.0 + sign * (8.0 + 16.0 * k as f64)).to_radians();
            let center = midline(a) + DVec3::Z * if closed { 0.0 } else { 1.0 };
            let dense = dense_molar && k == 6 && sign > 0.0;
            let mut tooth = if dense {
                TriMesh::uv_sphere(1.0, 32, 24)
            } else {
                TriMesh::uv_sphere(1.0, 16, 10)
            };
            for p in &mut tooth.positions {
                *p = DVec3::new(p.x * 3.5, p.y * 3.5, p.z * 5.5) + center;
            }
            mesh.merge(&tooth);
        }
    }
    orient_components_outward(&mut mesh);
    if height_scale != 1.0 {
        for p in &mut mesh.positions {
            *p = DVec3::new(p.x, p.y, p.z * height_scale);
        }
    }
    mesh
}

/// The full arch of [`build_arch`] at unit height.
pub fn arch(closed: bool) -> TriMesh {
    build_arch(closed, false, 1.0)
}

/// The open arch with every height scaled by `height_scale`.
pub fn open_arch_scaled(height_scale: f64) -> TriMesh {
    build_arch(false, false, height_scale)
}

/// The open arch, heights scaled by `height_scale`, cut to the triangles fully beyond
/// `keep_above` on X: a half arch (`keep_above` 0) or a quadrant (`keep_above` 8).
pub fn open_arch_cut(height_scale: f64, keep_above: f64) -> TriMesh {
    let mut mesh = build_arch(false, false, height_scale);
    mesh.triangles
        .retain(|t| t.iter().all(|&v| mesh.positions[v as usize].x > keep_above));
    let mut remap = vec![u32::MAX; mesh.positions.len()];
    let mut positions = Vec::new();
    for t in &mut mesh.triangles {
        for v in t {
            let i = *v as usize;
            if remap[i] == u32::MAX {
                remap[i] = positions.len() as u32;
                positions.push(mesh.positions[i]);
            }
            *v = remap[i];
        }
    }
    mesh.positions = positions;
    mesh
}

/// The open arch with the most posterior right molar meshed at higher density.
pub fn open_arch_dense_molar() -> TriMesh {
    build_arch(false, true, 1.0)
}

/// The open arch with every triangle's winding flipped, as an incorrectly
/// re-oriented scan would be.
pub fn inverted_arch() -> TriMesh {
    let mut mesh = build_arch(false, false, 1.0);
    for t in &mut mesh.triangles {
        t.swap(1, 2);
    }
    mesh
}

/// Flip any component whose triangles point inward. For a closed component the sign
/// of sum (centroid - component centroid) · normal is exact; for the open band it is
/// strongly positive when outward.
fn orient_components_outward(mesh: &mut TriMesh) {
    for component in mesh.connected_components() {
        let mut centroid_sum = DVec3::ZERO;
        for &t in &component {
            let [a, b, c] = mesh.triangles[t as usize].map(|v| mesh.positions[v as usize]);
            centroid_sum += a + b + c;
        }
        let center = centroid_sum / (3.0 * component.len() as f64);
        let mut signed = 0.0;
        for &t in &component {
            let [a, b, c] = mesh.triangles[t as usize].map(|v| mesh.positions[v as usize]);
            signed += (b - a).cross(c - a).dot((a + b + c) / 3.0 - center);
        }
        if signed < 0.0 {
            for &t in &component {
                mesh.triangles[t as usize].swap(1, 2);
            }
        }
    }
}

/// Rounded box scan generated from six stitched cube-face grids. `radius` is
/// the fillet radius; dimensions are half-extents in mm. No external tooth data.
pub fn rounded_box(
    half_extents: DVec3,
    radius: f64,
    subdivisions: usize,
) -> Result<TriMesh, DesignError> {
    if !half_extents.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || radius >= half_extents.min_element()
        || !(2..=256).contains(&subdivisions)
    {
        return Err(DesignError::InvalidParameter(
            "rounded box dimensions or sampling",
        ));
    }
    let mut mesh = TriMesh::default();
    let mut vertices = std::collections::BTreeMap::new();
    let n = subdivisions as i32;
    let core = half_extents - DVec3::splat(radius);
    for axis in 0..3 {
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        for sign in [-1, 1] {
            let mut grid = Vec::new();
            for j in 0..=n {
                for i in 0..=n {
                    let mut key = [0; 3];
                    key[axis] = sign * n;
                    key[u] = 2 * i - n;
                    key[v] = 2 * j - n;
                    let index = *vertices.entry(key).or_insert_with(|| {
                        let cube = DVec3::new(key[0] as f64, key[1] as f64, key[2] as f64)
                            / n as f64
                            * half_extents;
                        let base = cube.clamp(-core, core);
                        let p = base + (cube - base).normalize() * radius;
                        let index = mesh.positions.len() as u32;
                        mesh.positions.push(p);
                        index
                    });
                    grid.push(index);
                }
            }
            let width = subdivisions + 1;
            for j in 0..subdivisions {
                for i in 0..subdivisions {
                    let a = grid[j * width + i];
                    let b = grid[j * width + i + 1];
                    let c = grid[(j + 1) * width + i + 1];
                    let d = grid[(j + 1) * width + i];
                    let mut faces = [[a, b, c], [a, c, d]];
                    if sign < 0 {
                        for t in &mut faces {
                            t.swap(1, 2);
                        }
                    }
                    mesh.triangles.extend(faces);
                }
            }
        }
    }
    Ok(mesh)
}

/// A row of `count` adjacent prepared stumps on one connected intraoral-like scan
/// sheet, in a local +Z occlusal frame before `rotation`. Stumps stand
/// `spacing` mm apart along `row` (in the XY plane). Each has an occlusal table
/// at height 12 to radius 2.2, a steep wall down to a 0.6 mm wide shoulder
/// whose narrow rounded outer edge, the margin, is the circle of radius 3.6 at
/// height 8.22, then a drop to the gingiva at height 5.5. Between neighbours the
/// scan has a ragged hole where the scanner could not see, starting
/// `hole_clearance` mm outside the margins; the heightfield carries
/// deterministic `noise_mm` vertical noise. Returns the mesh, each stump's top
/// centre and its true margin (sampled every degree), all rotated.
#[cfg(test)]
pub(crate) fn prepared_row(
    count: usize,
    spacing: f64,
    row: f64,
    hole_clearance: f64,
    noise_mm: f64,
    rotation: DQuat,
) -> (TriMesh, Vec<DVec3>, Vec<Vec<DVec3>>) {
    const MARGIN_R: f64 = 3.6;
    const STEP: f64 = 0.1;
    let dir = DVec3::new(row.cos(), row.sin(), 0.0);
    let centres: Vec<DVec3> = (0..count)
        .map(|k| dir * spacing * (k as f64 - 0.5 * (count - 1) as f64))
        .collect();
    let profile = |rho: f64| {
        if rho <= 2.2 {
            12.0
        } else if rho <= 3.0 {
            12.0 - 3.4 * (rho - 2.2) / 0.8
        } else if rho <= MARGIN_R - 0.05 {
            8.6 - 0.3 * (rho - 3.0) / (MARGIN_R - 0.05 - 3.0)
        } else if rho <= MARGIN_R + 0.05 {
            // A narrow rounded convex edge from the ledge into the drop.
            let t = (rho - (MARGIN_R - 0.05)) / 0.1;
            8.3 - 0.055 * t - 0.2 * t * t
        } else {
            (8.045 - 2.5 * (rho - MARGIN_R - 0.05)).max(5.5)
        }
    };
    let hash = |i: i64, j: i64, salt: i64| {
        let mut h = (i.wrapping_mul(73_856_093) ^ j.wrapping_mul(19_349_663) ^ salt) as u64;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        (h % 10_000) as f64 / 10_000.0 * 2.0 - 1.0
    };
    let half_length = spacing * 0.5 * (count - 1) as f64 + 6.0;
    let half_width = 6.5;
    let nx = (2.0 * half_length / STEP) as i64;
    let ny = (2.0 * half_width / STEP) as i64;
    let mut positions = Vec::new();
    for j in 0..=ny {
        for i in 0..=nx {
            let along = -half_length + i as f64 * STEP;
            let across = -half_width + j as f64 * STEP;
            let p = dir * along + DVec3::new(-dir.y, dir.x, 0.0) * across;
            let z = centres
                .iter()
                .map(|c| profile((p - *c).length()))
                .fold(5.5, f64::max);
            positions.push(DVec3::new(p.x, p.y, z + noise_mm * hash(i, j, 7)));
        }
    }
    let index = |i: i64, j: i64| (j * (nx + 1) + i) as u32;
    let mut triangles = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            let corner = positions[index(i, j) as usize];
            let mut near: Vec<f64> = centres
                .iter()
                .map(|c| (corner.truncate().extend(0.0) - *c).length())
                .collect();
            near.sort_by(f64::total_cmp);
            let across = (corner.truncate().extend(0.0)).dot(DVec3::new(-dir.y, dir.x, 0.0));
            // Interproximal hole: close to two stumps, ragged at its edge.
            let clearance = MARGIN_R + hole_clearance + 0.25 * (hash(i, j, 3) + 1.0);
            if near.len() > 1
                && near[1] < MARGIN_R + 1.6
                && near[0] > clearance
                && across.abs() < 3.2
            {
                continue;
            }
            let (a, b, c, d) = (
                index(i, j),
                index(i + 1, j),
                index(i + 1, j + 1),
                index(i, j + 1),
            );
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
    }
    let mut mesh = TriMesh::new(positions, triangles);
    for p in &mut mesh.positions {
        *p = rotation * *p;
    }
    let tops = centres
        .iter()
        .map(|c| rotation * (*c + DVec3::Z * 12.0))
        .collect();
    let margins = centres
        .iter()
        .map(|c| {
            (0..360)
                .map(|k| {
                    let a = (k as f64).to_radians();
                    // The rounded edge bends evenly; its middle is the margin.
                    let r = MARGIN_R;
                    rotation * (*c + DVec3::new(r * a.cos(), r * a.sin(), profile(r)))
                })
                .collect()
        })
        .collect();
    (mesh, tops, margins)
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
        for bad in [
            ToothParams {
                height_scale: 0.4,
                ..Default::default()
            },
            ToothParams {
                extra_wall: 7.0,
                ..Default::default()
            },
            ToothParams {
                smooth_passes: 17,
                ..Default::default()
            },
            ToothParams {
                noise_mm: 0.06,
                ..Default::default()
            },
            ToothParams {
                neighbour_gap: 2.5,
                ..Default::default()
            },
        ] {
            assert!(prepared_tooth(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn tall_scaled_gingiva_and_neighbour_variants_stay_closed() {
        let variants = [
            ToothParams {
                height_scale: 1.6,
                ..Default::default()
            },
            ToothParams {
                extra_wall: 4.0,
                ..Default::default()
            },
            ToothParams {
                smooth_passes: 8,
                noise_mm: 0.02,
                ..Default::default()
            },
            ToothParams {
                gingiva: true,
                ..Default::default()
            },
            ToothParams {
                neighbour_gap: 0.4,
                ..Default::default()
            },
        ];
        for p in &variants {
            let die = prepared_tooth(p).unwrap();
            assert!(die.mesh.is_closed(), "{p:?}");
            assert!(die.mesh.is_consistently_oriented(), "{p:?}");
            let components = die.mesh.connected_components().len();
            assert_eq!(components, usize::from(p.neighbour_gap > 0.0) + 1, "{p:?}");
            assert_eq!(die.margin.len(), p.segments, "{p:?}");
        }
        // The scaled die keeps radius 4 and moves the margin to 12.8.
        let scaled = prepared_tooth(&ToothParams {
            height_scale: 1.6,
            ..Default::default()
        })
        .unwrap();
        for &point in &scaled.margin {
            assert!((point.z - 12.8).abs() < 1e-9);
            assert!((point.truncate().length() - 4.0).abs() < 1e-9);
        }
    }

    #[test]
    fn arch_variants_keep_expected_extent() {
        let full = arch(false);
        assert!(full.triangles.len() > 4000);
        let quadrant = open_arch_cut(1.3, 8.0);
        assert!(
            quadrant.triangles.len() > 500,
            "{}",
            quadrant.triangles.len()
        );
        assert!(
            quadrant
                .triangles
                .iter()
                .all(|t| { t.iter().all(|&v| quadrant.positions[v as usize].x > 8.0) })
        );
        let half = open_arch_cut(1.6, 0.0);
        assert!(half.triangles.len() > quadrant.triangles.len());
        // The dense molar adds many vertices without changing the occupied footprint.
        let dense = open_arch_dense_molar();
        assert_eq!(dense.positions.len() - full.positions.len(), 592);
        let inverted = inverted_arch();
        assert_eq!(inverted.triangles.len(), full.triangles.len());
    }
}
