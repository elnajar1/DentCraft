//! Jaw orientation from outward-wound scan surfaces (the loader guarantees winding).
//! Open scans use the area-weighted mean normal to select and sign the axis.
//! A strongly aligned PCA direction removes bias from cropped tooth surfaces; it
//! need not have occlusal as its smallest axis on a quadrant. Closed models need
//! a unique smallest PCA axis and a flat base. Forward requires a full projected
//! arch with strong mirror symmetry and an identifiable closed anterior end.
//! Inward winding is unsupported: it can reverse occlusal, but must not panic.

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
    let (centroid, occlusal) = orientation(mesh)?;
    let forward = anterior(mesh, centroid, occlusal)?;
    let right = forward.cross(occlusal);
    let mut heights: Vec<f64> = surface_vertices(mesh)
        .into_iter()
        .map(|i| (mesh.positions[i] - centroid).dot(occlusal))
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

/// Vertex indices supported by nondegenerate triangles. Isolated points and
/// zero-area faces must not affect bounds, cusp height or the occupancy grid.
fn surface_vertices(mesh: &TriMesh) -> Vec<usize> {
    let mut used = vec![false; mesh.positions.len()];
    for (i, t) in mesh.triangles.iter().enumerate() {
        if mesh.triangle_cross(i).length_squared() > 1e-24 {
            for &v in t {
                used[v as usize] = true;
            }
        }
    }
    used.into_iter()
        .enumerate()
        .filter(|(_, v)| *v)
        .map(|(i, _)| i)
        .collect()
}

/// Estimate the biting direction, including on quadrant and half-arch scans.
/// Requires consistent outward winding. The mean normal must agree within 30 degrees
/// with a principal direction; unlike the closed-model fallback, this need not
/// be the smallest PCA direction. Ambiguous geometry is rejected.
pub fn occlusal_direction(mesh: &TriMesh) -> Result<DVec3, DesignError> {
    orientation(mesh).map(|(_, axis)| axis)
}

fn orientation(mesh: &TriMesh) -> Result<(DVec3, DVec3), DesignError> {
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
    if !area.is_finite() || !centroid_sum.is_finite() || !normal_sum.is_finite() {
        return Err(DesignError::InvalidMesh);
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
    let occlusal = if normal_sum.length() > MEAN_NORMAL_SHARE * area {
        let mean = normal_sum.normalize();
        // The mean identifies WHICH principal direction is occlusal. Cropping
        // a tooth exposes an unbalanced side-normal contribution; project away
        // that bias only when one PCA direction agrees within 30 degrees.
        let index = (0..3)
            .max_by(|&a, &b| {
                vectors[a]
                    .dot(mean)
                    .abs()
                    .total_cmp(&vectors[b].dot(mean).abs())
            })
            .unwrap();
        let candidate = vectors[index];
        if candidate.dot(mean).abs() < 0.866 {
            return Err(DesignError::OrientationAmbiguous);
        }
        candidate * candidate.dot(mean).signum()
    } else {
        if values[1] <= 1.5 * values[0].max(0.0) {
            return Err(DesignError::OrientationAmbiguous);
        }
        let candidate = vectors[0];
        if normal_sum.length() > 0.01 * area && candidate.dot(normal_sum.normalize()).abs() < 0.5 {
            return Err(DesignError::OrientationAmbiguous);
        }
        sign_from_flat_base(mesh, candidate, area)?
    };
    Ok((centroid, occlusal))
}

/// Sign `candidate` so that it points away from the flat base, when the mesh has
/// exactly one: a large planar area perpendicular to the axis at an extreme.
fn sign_from_flat_base(
    mesh: &TriMesh,
    candidate: DVec3,
    total_area: f64,
) -> Result<DVec3, DesignError> {
    let used = surface_vertices(mesh);
    let (lo, hi) = used.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), &i| (lo.min(mesh.positions[i]), hi.max(mesh.positions[i])),
    );
    let slab = 0.02 * lo.distance(hi);
    let mut positive = false;
    let mut negative = false;
    for sign in [1.0, -1.0] {
        let extreme = used
            .iter()
            .map(|&i| sign * mesh.positions[i].dot(candidate))
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
        (true, true) => Err(DesignError::OrientationAmbiguous),
        (false, false) => Err(DesignError::OrientationAmbiguous),
    }
}

/// The anterior direction: the mirror axis of the occlusal-plane projection
/// whose closed (occupied on-axis) end is farther from the projection centroid.
fn anterior(mesh: &TriMesh, centroid: DVec3, occlusal: DVec3) -> Result<DVec3, DesignError> {
    let e0 = occlusal.any_orthonormal_vector();
    let e1 = occlusal.cross(e0);
    let points: Vec<(f64, f64)> = mesh
        .positions
        .iter()
        .map(|p| {
            let d = p - centroid;
            (d.dot(e0), d.dot(e1))
        })
        .collect();
    let used = surface_vertices(mesh);
    let (min_x, max_x) = used
        .iter()
        .map(|&i| points[i])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, p| {
            (acc.0.min(p.0), acc.1.max(p.0))
        });
    let (min_y, max_y) = used
        .iter()
        .map(|&i| points[i])
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
    // Rasterize projected triangle interiors, not vertex samples. Occupied area
    // is invariant to local re-tessellation, including a denser single molar.
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| points[i as usize]);
        let cross = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
            (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
        };
        let area = cross(a, b, c);
        if area.abs() < 1e-12 {
            continue;
        }
        let lo = index_of(a.0.min(b.0).min(c.0), a.1.min(b.1).min(c.1));
        let hi = index_of(a.0.max(b.0).max(c.0), a.1.max(b.1).max(c.1));
        for iy in lo / nx..=hi / nx {
            for ix in lo % nx..=hi % nx {
                let q = (
                    min_x + (ix as f64 + 0.5) * cell,
                    min_y + (iy as f64 + 0.5) * cell,
                );
                if [cross(a, b, q), cross(b, c, q), cross(c, a, q)]
                    .iter()
                    .all(|v| v * area >= 0.0)
                {
                    occupied[ix + iy * nx] = true;
                }
            }
        }
    }
    let cells: Vec<_> = occupied
        .iter()
        .enumerate()
        .filter(|(_, o)| **o)
        .map(|(i, _)| {
            (
                min_x + (i % nx) as f64 * cell + cell * 0.5,
                min_y + (i / nx) as f64 * cell + cell * 0.5,
            )
        })
        .collect();
    if cells.is_empty() {
        return Err(DesignError::OrientationAmbiguous);
    }
    let qbar = cells.iter().fold((0.0, 0.0), |s, p| {
        (
            s.0 + p.0 / cells.len() as f64,
            s.1 + p.1 / cells.len() as f64,
        )
    });
    let occupied_count = cells.len();
    let overlap = |theta: f64, offset: f64| {
        let d = (theta.cos(), theta.sin());
        cells
            .iter()
            .filter(|&&c| {
                let across = -(c.0 - qbar.0) * d.1 + (c.1 - qbar.1) * d.0 - offset;
                let reflected = (c.0 + 2.0 * across * d.1, c.1 - 2.0 * across * d.0);
                (min_x..=max_x).contains(&reflected.0)
                    && (min_y..=max_y).contains(&reflected.1)
                    && occupied[index_of(reflected.0, reflected.1)]
            })
            .count()
    };
    let mut best = (0usize, 0.0f64, 0.0f64);
    let mut worst = usize::MAX;
    for step in (0..180).step_by(2) {
        let theta = PI * step as f64 / 180.0;
        let mut angle_best = 0;
        for shift in (-20..=20).step_by(2) {
            let offset = shift as f64 * cell;
            let score = overlap(theta, offset);
            angle_best = angle_best.max(score);
            if score > best.0 {
                best = (score, theta, offset);
            }
        }
        worst = worst.min(angle_best);
    }
    let coarse = best;
    for angle in -8..=8 {
        for shift in -4..=4 {
            let theta = coarse.1 + (angle as f64 * 0.25).to_radians();
            let offset = coarse.2 + shift as f64 * 0.5 * cell;
            let score = overlap(theta, offset);
            if score > best.0 {
                best = (score, theta, offset);
            }
        }
    }

    if (best.0 - worst) as f64 <= SYMMETRY_CONTRAST * occupied_count as f64 {
        return Err(DesignError::OrientationAmbiguous);
    }
    if (best.0 as f64) < 0.97 * occupied_count as f64 {
        return Err(DesignError::PartialArch);
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
            if (-(c.0 - qbar.0) * d.1 + (c.1 - qbar.1) * d.0 - best.2).abs() > corridor {
                continue;
            }
            let along = (c.0 - qbar.0) * d.0 + (c.1 - qbar.1) * d.1;
            reach[0] = reach[0].max(along);
            reach[1] = reach[1].max(-along);
        }
    }
    let direction = if reach[0] >= reach[1] + 2.0 * cell {
        d
    } else if reach[1] >= reach[0] + 2.0 * cell {
        (-d.0, -d.1)
    } else {
        return Err(DesignError::OrientationAmbiguous);
    };
    let mut forward = e0 * direction.0 + e1 * direction.1;
    forward -= occlusal * forward.dot(occlusal);
    Ok(forward.normalize_or_zero())
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric 3×3 matrix, by
/// cyclic Jacobi rotations to convergence. Exact for diagonal input.
pub(crate) fn eigen3(m: [[f64; 3]; 3]) -> ([f64; 3], [DVec3; 3]) {
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

    use crate::synthetic::arch;

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
    fn regression_partial_arch_never_reports_a_wrong_frame() {
        for (scale, cut) in [(1.3, 8.0), (1.6, 0.0)] {
            let mesh = crate::synthetic::open_arch_cut(scale, cut);
            for seed in 0..3 {
                let rotation = Lcg::new(seed + 73).rotation();
                let mut rotated = mesh.clone();
                for p in &mut rotated.positions {
                    *p = rotation * *p;
                }
                assert_eq!(jaw_frame(&rotated), Err(DesignError::PartialArch));
                let error =
                    angle_between(occlusal_direction(&rotated).unwrap(), rotation * DVec3::Z);
                eprintln!(
                    "partial scale {scale}, cut {cut}, rotation {seed}: occlusal error {error:.6} degrees"
                );
                assert!(error < 3.0, "partial occlusal error {error}");
            }
        }
    }

    #[test]
    fn regression_dense_molar_does_not_rotate_forward() {
        let mesh = crate::synthetic::open_arch_dense_molar();
        let frame = jaw_frame(&mesh).unwrap();
        let error = angle_between(frame.forward, DVec3::Y);
        eprintln!("dense molar forward error {error:.6} degrees");
        assert!(error < 5.0, "dense molar forward error {error}");
        let mut dirty = mesh.clone();
        let duplicate = dirty.positions.len() as u32;
        dirty.positions.push(dirty.positions[0]);
        dirty.positions.push(DVec3::splat(1e6));
        dirty.triangles.push([duplicate; 3]);
        let dirty_frame =
            jaw_frame(&dirty).expect("unused and zero-area geometry cannot define the frame");
        assert!(angle_between(dirty_frame.forward, frame.forward) < 1e-6);
        assert!(dirty_frame.origin.distance(frame.origin) < 1e-9);
    }

    #[test]
    fn regression_inverted_winding_is_finite_or_rejected() {
        // Outward winding is a precondition, not something geometry can infer.
        if let Ok(frame) = jaw_frame(&crate::synthetic::inverted_arch()) {
            assert_frame(&frame);
            assert!(frame.origin.is_finite());
        }
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
            Err(DesignError::OrientationAmbiguous)
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
            Err(DesignError::OrientationAmbiguous)
        ));
        // A closed sphere has neither an open mean normal nor a flat base.
        assert!(matches!(
            jaw_frame(&TriMesh::uv_sphere(5.0, 24, 12)),
            Err(DesignError::OrientationAmbiguous)
        ));
    }

    #[test]
    fn eigen3_sorts_and_diagonalizes() {
        let (tied, _) = eigen3([[1.0, 0.5, 0.0], [0.5, 1.0, 0.0], [0.0, 0.0, 3.0]]);
        assert!(
            (tied[0] - 0.5).abs() < 1e-12,
            "equal diagonal entries: {tied:?}"
        );
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
