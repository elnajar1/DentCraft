//! Jaw orientation: an occlusal/anterior frame for full-arch scans.
//!
//! [`jaw_frame`] estimates an orthonormal right-handed frame from geometry alone.
//! The occlusal normal comes from an area-weighted PCA of the triangle centroids
//! (the smallest axis). Its sign comes from the area-weighted mean normal, which
//! for an open intraoral scan — a surface seen from the occlusal side — points
//! toward the biting surface; when the mean normal cancels (a closed model scan),
//! the flat base gives the sign instead: the extreme with a large planar area
//! perpendicular to the axis is the base, and the occlusal direction points away
//! from it. The forward direction is found in the occlusal plane: the arch has a
//! mirror symmetry axis (a rotation search that maximizes the overlap of an
//! occupancy grid with its reflection), and its closed end — occupied cells on
//! that axis — is the anterior (incisal) end. `right` is `forward × occlusal`,
//! which is the patient's right when `forward` runs from the molars toward the
//! incisors; the ordered basis `(right, forward, occlusal)` is right-handed.

use std::f64::consts::PI;

use dentcraft_mesh::TriMesh;
use glam::DVec3;

use crate::DesignError;
use crate::margin::validate_mesh;

/// Maximum occupancy grid cells; larger projections are rejected, not allocated.
const MAX_GRID_CELLS: usize = 1_000_000;
/// The best mirror overlap must beat the worst by this fraction of the occupied
/// cells, or the shape has no unique symmetry axis and `forward` is undefined.
const SYMMETRY_CONTRAST: f64 = 0.05;
/// A planar cap at an extreme counts as a base only above this share of the area.
const BASE_AREA_SHARE: f64 = 0.05;
/// Mean-normal strength above which the scan counts as an open occlusal surface.
const MEAN_NORMAL_SHARE: f64 = 0.1;

/// Orthonormal right-handed jaw frame in scan coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JawFrame {
    /// The area centroid of the scan, raised to the height of the cusp tips
    /// (the 95th percentile of vertex heights along `occlusal`).
    pub origin: DVec3,
    /// Unit vector from the jaw toward its biting surface.
    pub occlusal: DVec3,
    /// Unit vector in the occlusal plane pointing toward the incisors.
    pub forward: DVec3,
    /// `forward × occlusal`: the patient's right when `forward` points from the
    /// molars toward the incisors. Completes the right-handed basis, so
    /// `right × forward = occlusal`.
    pub right: DVec3,
}

/// Estimate the jaw frame of a full-arch scan.
///
/// Errors: empty, nonfinite or wholly degenerate meshes; a smallest PCA axis
/// that is not unique (a rod has no preferred plane); a closed model without a
/// flat base to sign the occlusal direction (a sphere); and shapes without a
/// unique mirror axis or a unique closed end (a disc has no anterior direction).
pub fn jaw_frame(mesh: &TriMesh) -> Result<JawFrame, DesignError> {
    validate_mesh(mesh)?;
    let mut area = 0.0;
    let mut centroid_sum = DVec3::ZERO;
    let mut normal_sum = DVec3::ZERO;
    for i in 0..mesh.triangles.len() {
        let cross = mesh.triangle_cross(i);
        let triangle_area = 0.5 * cross.length();
        if triangle_area == 0.0 {
            continue;
        }
        let [a, b, c] = mesh.triangle(i);
        centroid_sum += (a + b + c) / 3.0 * triangle_area;
        normal_sum += cross * 0.5;
        area += triangle_area;
    }
    let centroid = centroid_sum / area;
    let mut covariance = [[0.0; 3]; 3];
    for i in 0..mesh.triangles.len() {
        let triangle_area = 0.5 * mesh.triangle_cross(i).length();
        if triangle_area == 0.0 {
            continue;
        }
        let [a, b, c] = mesh.triangle(i);
        let d = (a + b + c) / 3.0 - centroid;
        for r in 0..3 {
            for c in 0..3 {
                covariance[r][c] += triangle_area * d[r] * d[c];
            }
        }
    }
    let (values, vectors) = eigen3(covariance);
    let trace = values.iter().sum::<f64>();
    if values[1] - values[0] <= 1e-6 * trace.max(f64::MIN_POSITIVE) {
        return Err(DesignError::InvalidParameter(
            "the occlusal axis is not unique",
        ));
    }
    let mut occlusal = vectors[0].normalize_or_zero();
    if normal_sum.length() > MEAN_NORMAL_SHARE * area {
        if normal_sum.dot(occlusal) < 0.0 {
            occlusal = -occlusal;
        }
    } else {
        occlusal = sign_from_flat_base(mesh, occlusal, area)?;
    }
    let forward = anterior(mesh, centroid, occlusal)?;
    let right = forward.cross(occlusal);
    let mut heights: Vec<f64> = mesh
        .positions
        .iter()
        .map(|p| (p - centroid).dot(occlusal))
        .collect();
    heights.sort_by(f64::total_cmp);
    let index = (0.95 * heights.len().saturating_sub(1) as f64).round() as usize;
    let origin = centroid + occlusal * heights[index.min(heights.len() - 1)];
    Ok(JawFrame {
        origin,
        occlusal,
        forward,
        right,
    })
}

/// Sign `candidate` so that it points away from the flat base, when the mesh has
/// exactly one: a large planar area perpendicular to the axis at an extreme.
fn sign_from_flat_base(
    mesh: &TriMesh,
    candidate: DVec3,
    total_area: f64,
) -> Result<DVec3, DesignError> {
    let slab = 0.02
        * mesh
            .bounds()
            .expect("a validated mesh has a bounding box")
            .diagonal();
    let mut positive = false;
    let mut negative = false;
    for sign in [1.0, -1.0] {
        let extreme = mesh
            .positions
            .iter()
            .map(|p| sign * p.dot(candidate))
            .fold(f64::NEG_INFINITY, f64::max);
        let mut area = 0.0;
        for i in 0..mesh.triangles.len() {
            let cross = mesh.triangle_cross(i);
            let Some(normal) = cross.try_normalize() else {
                continue;
            };
            let [a, b, c] = mesh.triangle(i);
            let centroid = (a + b + c) / 3.0;
            if sign * normal.dot(candidate) > 0.9
                && sign * centroid.dot(candidate) >= extreme - slab
            {
                area += 0.5 * cross.length();
            }
        }
        if area >= BASE_AREA_SHARE * total_area {
            positive |= sign > 0.0;
            negative |= sign < 0.0;
        }
    }
    match (positive, negative) {
        (true, false) => Ok(-candidate),
        (false, true) => Ok(candidate),
        (true, true) => Err(DesignError::InvalidParameter(
            "the occlusal sign is ambiguous: flat caps on both extremes",
        )),
        (false, false) => Err(DesignError::InvalidParameter(
            "the occlusal sign is undefined: closed model without a flat base",
        )),
    }
}

/// The anterior direction: the mirror axis of the occlusal-plane projection
/// whose closed (occupied on-axis) end is farther from the projection centroid.
fn anterior(mesh: &TriMesh, centroid: DVec3, occlusal: DVec3) -> Result<DVec3, DesignError> {
    let e0 = occlusal.any_orthonormal_vector();
    let e1 = occlusal.cross(e0);
    let mut points: Vec<(f64, f64)> = mesh
        .positions
        .iter()
        .map(|p| {
            let d = p - centroid;
            (d.dot(e0), d.dot(e1))
        })
        .collect();
    let count = points.len() as f64;
    let qbar = points.iter().fold((0.0, 0.0), |acc, p| {
        (acc.0 + p.0 / count, acc.1 + p.1 / count)
    });
    for p in &mut points {
        *p = (p.0 - qbar.0, p.1 - qbar.1);
    }
    let (min_x, max_x) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, p| {
            (acc.0.min(p.0), acc.1.max(p.0))
        });
    let (min_y, max_y) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, p| {
            (acc.0.min(p.1), acc.1.max(p.1))
        });
    let span = (max_x - min_x).max(max_y - min_y).max(f64::MIN_POSITIVE);
    let cell = span / 150.0;
    let nx = ((max_x - min_x) / cell).ceil() as usize + 1;
    let ny = ((max_y - min_y) / cell).ceil() as usize + 1;
    if nx.checked_mul(ny).is_none_or(|n| n > MAX_GRID_CELLS) {
        return Err(DesignError::GridTooLarge);
    }
    let index_of = |x: f64, y: f64| {
        let ix = (((x - min_x) / cell) as usize).min(nx - 1);
        let iy = (((y - min_y) / cell) as usize).min(ny - 1);
        ix + iy * nx
    };
    let mut occupied = vec![false; nx * ny];
    for p in &points {
        occupied[index_of(p.0, p.1)] = true;
    }
    let occupied_count = occupied.iter().filter(|o| **o).count();
    let mut best = (0usize, 0.0f64);
    let mut worst = usize::MAX;
    for step in 0..180 {
        let theta = PI * step as f64 / 180.0;
        let d = (theta.cos(), theta.sin());
        let mut overlap = 0;
        for iy in 0..ny {
            for ix in 0..nx {
                if !occupied[ix + iy * nx] {
                    continue;
                }
                let c = (
                    min_x + (ix as f64 + 0.5) * cell,
                    min_y + (iy as f64 + 0.5) * cell,
                );
                let dot = c.0 * d.0 + c.1 * d.1;
                let reflected = (2.0 * dot * d.0 - c.0, 2.0 * dot * d.1 - c.1);
                if (min_x - cell..=max_x + cell).contains(&reflected.0)
                    && (min_y - cell..=max_y + cell).contains(&reflected.1)
                    && occupied[index_of(reflected.0, reflected.1)]
                {
                    overlap += 1;
                }
            }
        }
        if overlap > best.0 {
            best = (overlap, theta);
        }
        worst = worst.min(overlap);
    }
    if occupied_count == 0 || (best.0 - worst) as f64 <= SYMMETRY_CONTRAST * occupied_count as f64 {
        return Err(DesignError::InvalidParameter(
            "forward is undefined: no unique mirror axis",
        ));
    }
    let d = (best.1.cos(), best.1.sin());
    let corridor = (0.06 * (max_x - min_x).hypot(max_y - min_y)).max(2.5 * cell);
    let mut reach = [f64::NEG_INFINITY; 2];
    for iy in 0..ny {
        for ix in 0..nx {
            if !occupied[ix + iy * nx] {
                continue;
            }
            let c = (
                min_x + (ix as f64 + 0.5) * cell,
                min_y + (iy as f64 + 0.5) * cell,
            );
            if (-c.1 * d.0 + c.0 * d.1).abs() > corridor {
                continue;
            }
            let along = c.0 * d.0 + c.1 * d.1;
            reach[0] = reach[0].max(along);
            reach[1] = reach[1].max(-along);
        }
    }
    let direction = if reach[0] >= reach[1] + 2.0 * cell {
        d
    } else if reach[1] >= reach[0] + 2.0 * cell {
        (-d.0, -d.1)
    } else {
        return Err(DesignError::InvalidParameter(
            "forward is undefined: no unique closed end",
        ));
    };
    let mut forward = e0 * direction.0 + e1 * direction.1;
    forward -= occlusal * forward.dot(occlusal);
    Ok(forward.normalize_or_zero())
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric 3×3 matrix, by
/// cyclic Jacobi rotations to convergence. Exact for diagonal input.
fn eigen3(m: [[f64; 3]; 3]) -> ([f64; 3], [DVec3; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let norm: f64 = a.iter().flatten().map(|x| x.abs()).sum();
    for _ in 0..60 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off <= 1e-15 * norm.max(f64::MIN_POSITIVE) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            let theta = 0.5 * (a[q][q] - a[p][p]) / a[p][q];
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for (row, vrow) in a.iter_mut().zip(v.iter_mut()) {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
                let (vkp, vkq) = (vrow[p], vrow[q]);
                vrow[p] = c * vkp - s * vkq;
                vrow[q] = s * vkp + c * vkq;
            }
            let (prefix, rest) = a.split_at_mut(q);
            let (row_p, row_q) = (&mut prefix[p], &mut rest[0]);
            for (left, right) in row_p.iter_mut().zip(row_q.iter_mut()) {
                let (apk, aqk) = (*left, *right);
                *left = c * apk - s * aqk;
                *right = s * apk + c * aqk;
            }
        }
    }
    let mut order = [0usize, 1, 2];
    order.sort_by(|i, j| a[*i][*i].total_cmp(&a[*j][*j]));
    let values = order.map(|i| a[i][i]);
    let vectors = order.map(|i| DVec3::new(v[0][i], v[1][i], v[2][i]).normalize_or_zero());
    (values, vectors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Lcg;
    use glam::DQuat;
    use std::f64::consts::TAU;

    /// Angle between unit vectors, in degrees.
    fn angle_between(a: DVec3, b: DVec3) -> f64 {
        a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// A synthetic full arch: a horseshoe gingiva band with 14 ellipsoid teeth in
    /// mirror-symmetric stations. The open variant is the occlusal-side scan
    /// surface (the band's underside is missing, its mean normal points up); the
    /// closed variant is a stone model: the same band as a closed tube over a
    /// solid horseshoe base with a flat bottom, teeth sunk into it. Components
    /// overlap geometrically, like a merged scan; each is oriented outward.
    fn arch(closed: bool) -> TriMesh {
        let mut mesh = TriMesh::default();
        const STATIONS: usize = 40;
        let phi = |k: usize| (-25.0 + 230.0 * k as f64 / (STATIONS - 1) as f64).to_radians();
        let midline = |a: f64| DVec3::new(22.0 * a.cos(), 30.0 * a.sin(), 0.0);
        let radial = |a: f64| DVec3::new(a.cos(), a.sin(), 0.0);
        // Gingiva band: rings of cross-sections along the midline. The open
        // variant keeps the upper half arc only (the scanned side); the closed
        // variant uses the full ellipse and caps the two arm ends.
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
            // Solid horseshoe base: prism over the annular footprint between an
            // outer and inner half-ellipse, from z = -7 up to z = -1, so the
            // band (bottom at z = -3) is buried in it. Flat bottom, curved top
            // edges: a stone model base.
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
        // Teeth: 14 ellipsoids in mirror-symmetric stations, sunk into the band.
        for k in 0..7 {
            for sign in [1.0, -1.0] {
                let a = (90.0 + sign * (8.0 + 16.0 * k as f64)).to_radians();
                let center = midline(a) + DVec3::Z * if closed { 0.0 } else { 1.0 };
                let mut tooth = TriMesh::uv_sphere(1.0, 16, 10);
                for p in &mut tooth.positions {
                    *p = DVec3::new(p.x * 3.5, p.y * 3.5, p.z * 5.5) + center;
                }
                mesh.merge(&tooth);
            }
        }
        orient_components_outward(&mut mesh);
        mesh
    }

    /// Flip any component whose triangles point inward. For a closed component
    /// the sign of sum (centroid - component centroid) · normal is exact; for the
    /// open band it is strongly positive when outward.
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

    /// Assert orthonormality and right-handedness of the frame.
    fn assert_frame(frame: &JawFrame) {
        for v in [frame.occlusal, frame.forward, frame.right] {
            assert!((v.length() - 1.0).abs() < 1e-9, "{v:?}");
        }
        assert!(frame.occlusal.dot(frame.forward).abs() < 1e-9);
        assert!(frame.right.dot(frame.occlusal).abs() < 1e-9);
        assert!((frame.right.cross(frame.forward) - frame.occlusal).length() < 1e-9);
        assert!((frame.forward.cross(frame.occlusal) - frame.right).length() < 1e-9);
    }

    #[test]
    fn arch_frames_survive_random_rigid_rotations() {
        for closed in [false, true] {
            let base = arch(closed);
            for seed in 0..5u64 {
                let mut mesh = base.clone();
                let rotation = Lcg::new(seed ^ 0x9E37_79B9_7F4A_7C15).rotation();
                for p in &mut mesh.positions {
                    *p = rotation * *p;
                }
                let frame = jaw_frame(&mesh).expect("the arch has a jaw frame");
                assert_frame(&frame);
                let occlusal_error = angle_between(frame.occlusal, rotation * DVec3::Z);
                assert!(
                    occlusal_error < 3.0,
                    "{occlusal_error} (closed {closed}, seed {seed})"
                );
                let forward_error = angle_between(frame.forward, rotation * DVec3::Y);
                assert!(
                    forward_error < 5.0,
                    "{forward_error} (closed {closed}, seed {seed})"
                );
                let right_error = angle_between(frame.right, rotation * DVec3::X);
                assert!(
                    right_error < 5.0,
                    "{right_error} (closed {closed}, seed {seed})"
                );
                // The origin sits at cusp-tip height: no vertex far above it, and
                // the mean vertex clearly below it.
                let heights: Vec<f64> = mesh
                    .positions
                    .iter()
                    .map(|p| (p - frame.origin).dot(frame.occlusal))
                    .collect();
                let top = heights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let mean = heights.iter().sum::<f64>() / heights.len() as f64;
                assert!(
                    (-1.0..=1.0).contains(&top),
                    "origin off cusp height: top {top}"
                );
                assert!(mean < 0.0, "origin below cusp height: mean {mean}");
            }
        }
    }

    #[test]
    fn flat_disc_has_no_forward_direction() {
        // A filled flat disc: every diameter is a mirror axis, so the anterior
        // end cannot be defined.
        let mut disc = TriMesh::default();
        let inside = |x: f64, y: f64| (DVec3::new(x, y, 0.0).truncate().length() - 19.5) < 0.0;
        let mut index = [[u32::MAX; 41]; 41];
        for (r, row) in index.iter_mut().enumerate() {
            for (c, slot) in row.iter_mut().enumerate() {
                let (x, y) = (c as f64 - 20.0, r as f64 - 20.0);
                if inside(x, y) {
                    *slot = disc.positions.len() as u32;
                    disc.positions.push(DVec3::new(x, y, 0.0));
                }
            }
        }
        for rows in index.windows(2) {
            for (top, bottom) in rows[0].windows(2).zip(rows[1].windows(2)) {
                let (a, b, d, e) = (top[0], top[1], bottom[1], bottom[0]);
                if [a, b, d, e].iter().all(|v| *v != u32::MAX) {
                    disc.triangles.push([a, b, d]);
                    disc.triangles.push([a, d, e]);
                }
            }
        }
        assert!(matches!(
            jaw_frame(&disc),
            Err(DesignError::InvalidParameter(_))
        ));
    }

    #[test]
    fn empty_and_degenerate_meshes_are_rejected() {
        assert_eq!(
            jaw_frame(&TriMesh::default()),
            Err(DesignError::EmptyRegion)
        );
        let collinear = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::new(2.0, 0.0, 0.0)],
            vec![[0, 1, 2]],
        );
        assert_eq!(jaw_frame(&collinear), Err(DesignError::DegenerateRegion));
        // A rod has no unique occlusal plane.
        let mut rod = TriMesh::default();
        for k in 0..10u32 {
            for i in 0..24u32 {
                let a = TAU * i as f64 / 24.0;
                rod.positions
                    .push(DVec3::new(a.cos(), a.sin(), k as f64 * 3.0));
            }
        }
        for k in 0..9u32 {
            for i in 0..24u32 {
                let at = |ring: u32, seg: u32| ring * 24 + seg % 24;
                rod.triangles
                    .push([at(k, i), at(k, i + 1), at(k + 1, i + 1)]);
                rod.triangles
                    .push([at(k, i), at(k + 1, i + 1), at(k + 1, i)]);
            }
        }
        assert!(matches!(
            jaw_frame(&rod),
            Err(DesignError::InvalidParameter(_))
        ));
        // A closed sphere has neither an open mean normal nor a flat base.
        assert!(matches!(
            jaw_frame(&TriMesh::uv_sphere(5.0, 24, 12)),
            Err(DesignError::InvalidParameter(_))
        ));
    }

    #[test]
    fn eigen3_sorts_and_diagonalizes() {
        let (values, vectors) = eigen3([[4.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -2.0]]);
        assert_eq!(values, [-2.0, 1.0, 4.0]);
        assert_eq!(vectors[0], DVec3::Z);
        assert_eq!(vectors[1], DVec3::Y);
        assert_eq!(vectors[2], DVec3::X);
        let m = [[2.0, 0.3, -0.2], [0.3, 1.0, 0.4], [-0.2, 0.4, 0.5]];
        let (values, vectors) = eigen3(m);
        for i in 0..3 {
            for j in 0..3 {
                assert!((vectors[i].dot(vectors[j]) - f64::from(i == j)).abs() < 1e-9);
            }
            let v = vectors[i];
            let transform = DVec3::new(
                m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
                m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
                m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
            );
            assert!((transform - v * values[i]).length() < 1e-9);
        }
        assert!(values[0] <= values[1] && values[1] <= values[2]);
    }

    /// The Lcg helper must be deterministic and produce unit quaternions.
    #[test]
    fn lcg_rotation_is_deterministic_and_unit() {
        let mut a = Lcg::new(7);
        let mut b = Lcg::new(7);
        for _ in 0..10 {
            let qa = a.rotation();
            let qb = b.rotation();
            assert_eq!(qa, qb);
            assert!((qa.length() - 1.0).abs() < 1e-12);
        }
        let q = DQuat::from_xyzw(0.0, 0.0, 0.0, 1.0);
        assert_eq!(Lcg::new(1).rotation(), Lcg::new(1).rotation());
        assert_ne!(Lcg::new(1).rotation(), q);
    }
}
