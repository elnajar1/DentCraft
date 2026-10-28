//! Discrete curvature per vertex.
//!
//! Mean curvature comes from the cotangent Laplacian with mixed Voronoi areas (Meyer,
//! Desbrun, Schröder and Barr 2003). Principal curvatures come from a least-squares fit of
//! the directional normal curvature over each one-ring, the per-edge estimate of Rusinkiewicz
//! (2004) combined with a quadratic fit.
//!
//! Signs assume triangles wound counter-clockwise seen from outside: convex regions (a sphere
//! with outward normals) are positive. Flipping the mesh flips both signs.

use glam::DVec3;

use crate::TriMesh;
use crate::topology::has_distinct_corners;

/// Principal curvatures and directions at a vertex, with `k1 ≥ k2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrincipalCurvature {
    /// Largest principal curvature; positive where the surface is convex.
    pub k1: f64,
    /// Smallest principal curvature.
    pub k2: f64,
    /// Direction of `k1`, unit length and tangent. A direction is a line, not an arrow:
    /// `-dir1` is the same answer.
    pub dir1: DVec3,
    /// Direction of `k2`, unit length, tangent and perpendicular to `dir1`.
    pub dir2: DVec3,
}

impl PrincipalCurvature {
    /// An isotropic estimate (`k1 == k2`) with an arbitrary tangent frame, used where the
    /// one-ring is too poor to fit (isolated, boundary-chain or degenerate vertices).
    fn umbilic(curvature: f64, u: DVec3, v: DVec3) -> Self {
        Self {
            k1: curvature,
            k2: curvature,
            dir1: u,
            dir2: v,
        }
    }
}

impl TriMesh {
    /// Signed mean curvature per vertex, `H = (k1 + k2) / 2`, from the cotangent Laplacian
    /// with mixed Voronoi areas (Meyer et al. 2003): convex is positive.
    ///
    /// Boundary vertices get a one-sided estimate from the triangles they have. Triangles
    /// with a repeated corner or zero area, and vertices whose whole neighbourhood is
    /// degenerate, give `0.0`; the result is always finite.
    pub fn mean_curvature(&self) -> Vec<f64> {
        let n = self.positions.len();
        if n == 0 {
            return Vec::new();
        }
        let normals = self.vertex_normals();
        // Per vertex: the cotangent-weighted neighbour displacement and the mixed area.
        let mut laplace = vec![DVec3::ZERO; n];
        let mut area = vec![0.0; n];
        for t in self.triangles.iter().filter(|t| has_distinct_corners(t)) {
            let [a, b, c] = t.map(|v| self.positions[v as usize]);
            let cross = (b - a).cross(c - a);
            let twice_area = cross.length();
            if !twice_area.is_finite() || twice_area == 0.0 {
                continue;
            }
            let cot_a = (b - a).dot(c - a) / twice_area;
            let cot_b = (a - b).dot(c - b) / twice_area;
            let cot_c = (a - c).dot(b - c) / twice_area;
            let (ia, ib, ic) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (ab, ac, bc) = (b - a, c - a, c - b);
            laplace[ia] += ab * cot_c + ac * cot_b;
            laplace[ib] += ab * -cot_c + bc * cot_a;
            laplace[ic] += ac * -cot_b + bc * -cot_a;
            // Mixed areas: Voronoi for non-obtuse triangles; an obtuse triangle gives half
            // its area to its obtuse corner and a quarter to each of the others (Meyer et
            // al. 2003, eq. 4).
            let obtuse = [cot_a < 0.0, cot_b < 0.0, cot_c < 0.0];
            let triangle_area = twice_area / 2.0;
            let corner_area = |obtuse_corner: bool| {
                if obtuse_corner {
                    triangle_area / 2.0
                } else {
                    triangle_area / 4.0
                }
            };
            let (area_a, area_b, area_c) = if obtuse.into_iter().any(|o| o) {
                (
                    corner_area(obtuse[0]),
                    corner_area(obtuse[1]),
                    corner_area(obtuse[2]),
                )
            } else {
                (
                    (cot_b * ac.length_squared() + cot_c * ab.length_squared()) / 8.0,
                    (cot_a * bc.length_squared() + cot_c * ab.length_squared()) / 8.0,
                    (cot_a * bc.length_squared() + cot_b * ac.length_squared()) / 8.0,
                )
            };
            area[ia] += area_a;
            area[ib] += area_b;
            area[ic] += area_c;
        }
        (0..n)
            .map(|i| {
                if area[i] > 0.0 {
                    let h = -laplace[i].dot(normals[i]) / (4.0 * area[i]);
                    if h.is_finite() { h } else { 0.0 }
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// Principal curvatures and directions per vertex: each edge gives a directional
    /// curvature estimate `e · (n_j - n_i) / |e|²` (Rusinkiewicz 2004) and the per-vertex
    /// curvature tensor `k(d) = a·x² + b·xy + c·y²` is fitted over the one-ring by least
    /// squares in a tangent frame, then diagonalised.
    ///
    /// Vertices with fewer than three usable edge directions, a singular fit, or a
    /// non-finite (or absurd, beyond 10¹² mm⁻¹) result fall back to an isotropic estimate
    /// `k1 = k2 = H` from [`TriMesh::mean_curvature`], with an arbitrary tangent frame.
    /// Boundary vertices are fitted over whatever neighbours they have. The result has one
    /// entry per vertex and is always finite.
    pub fn principal_curvatures(&self) -> Vec<PrincipalCurvature> {
        let n = self.positions.len();
        if n == 0 {
            return Vec::new();
        }
        let adjacency = self.vertex_adjacency();
        let normals = self.vertex_normals();
        let mean = self.mean_curvature();
        let mut out = Vec::with_capacity(n);
        for (i, &n_hat) in normals.iter().enumerate() {
            let (u, v) = tangent_basis(n_hat);
            let mut normal_eq = [[0.0f64; 3]; 3];
            let mut rhs = [0.0f64; 3];
            let mut samples = 0;
            for &j in adjacency.neighbors(i as u32) {
                let e = self.positions[j as usize] - self.positions[i];
                let edge_squared = e.length_squared();
                let tangent = e - n_hat * e.dot(n_hat);
                let tangent_squared = tangent.length_squared();
                if !edge_squared.is_finite() || edge_squared == 0.0 || tangent_squared == 0.0 {
                    continue;
                }
                let curvature = e.dot(normals[j as usize] - n_hat) / edge_squared;
                if !curvature.is_finite() {
                    continue;
                }
                let d = tangent / tangent_squared.sqrt();
                let x = d.dot(u);
                let y = d.dot(v);
                let basis = [x * x, x * y, y * y];
                for (r, &br) in basis.iter().enumerate() {
                    rhs[r] += br * curvature;
                    for (c, &bc) in basis.iter().enumerate() {
                        normal_eq[r][c] += br * bc;
                    }
                }
                samples += 1;
            }
            let fit = if samples >= 3 {
                solve3(normal_eq, rhs)
                    .filter(|coef| coef.iter().all(|k| k.is_finite() && k.abs() < 1e12))
            } else {
                None
            };
            out.push(match fit {
                Some([a, b, c]) => {
                    let (k1, k2, dir1, dir2) = diagonalize(a, b, c, u, v, n_hat);
                    if k1.is_finite() && k2.is_finite() {
                        PrincipalCurvature { k1, k2, dir1, dir2 }
                    } else {
                        PrincipalCurvature::umbilic(mean[i], u, v)
                    }
                }
                None => PrincipalCurvature::umbilic(mean[i], u, v),
            });
        }
        out
    }
}

/// A right-handed orthonormal frame `(u, v)` in the tangent plane of the unit vector `n`.
/// The axis least aligned with `n` seeds it, so the frame is never degenerate.
fn tangent_basis(n: DVec3) -> (DVec3, DVec3) {
    let (x, y, z) = (n.x.abs(), n.y.abs(), n.z.abs());
    let axis = if x <= y && x <= z {
        DVec3::X
    } else if y <= z {
        DVec3::Y
    } else {
        DVec3::Z
    };
    let u = (axis - n * axis.dot(n)).normalize_or_zero();
    (u, n.cross(u))
}

/// Eigen-decomposition of the symmetric 2×2 curvature tensor `[[a, b/2], [b/2, c]]` given in
/// the tangent frame `(u, v)` with normal `n`. Returns `k1 ≥ k2` with unit, tangent,
/// orthogonal directions; `dir2` is `n × dir1`.
fn diagonalize(a: f64, b: f64, c: f64, u: DVec3, v: DVec3, n: DVec3) -> (f64, f64, DVec3, DVec3) {
    let half_b = b * 0.5;
    let scale = a.abs() + c.abs() + half_b.abs();
    if scale == 0.0 {
        // Flat fit: every direction has curvature 0.
        return (0.0, 0.0, u, v);
    }
    let half_t = (a + c) * 0.5;
    let disc = ((a - c) * 0.5).hypot(half_b);
    let (k1, k2) = (half_t + disc, half_t - disc);
    // Eigenvector of k1: (a - k1)·x + half_b·y = 0.
    let (ex, ey) = (half_b, k1 - a);
    let len = ex.hypot(ey);
    let dir1 = if len > 1e-9 * scale {
        (u * ex + v * ey).normalize_or_zero()
    } else if a >= c {
        u
    } else {
        v
    };
    let dir2 = n.cross(dir1);
    (k1, k2, dir1, dir2)
}

/// Solves `a · x = b` for a symmetric 3×3 by Gaussian elimination with partial pivoting.
/// `None` when the system is singular to working precision or trivially zero.
fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let mut m = a;
    let mut rhs = b;
    let scale = m.iter().flatten().map(|x| x.abs()).fold(0.0f64, f64::max);
    if !scale.is_finite() || scale == 0.0 {
        return None;
    }
    for col in 0..3 {
        let pivot = (col..3).max_by(|r1, r2| m[*r1][col].abs().total_cmp(&m[*r2][col].abs()))?;
        if m[pivot][col].abs() < scale * 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        rhs.swap(col, pivot);
        let pivot_row = m[col];
        for row in col + 1..3 {
            let f = m[row][col] / pivot_row[col];
            for (value, pivot_value) in m[row][col..].iter_mut().zip(&pivot_row[col..]) {
                *value -= f * pivot_value;
            }
            rhs[row] -= f * rhs[col];
        }
    }
    let mut x = [0.0; 3];
    for row in (0..3).rev() {
        let mut s = rhs[row];
        for c in row + 1..3 {
            s -= m[row][c] * x[c];
        }
        x[row] = s / m[row][row];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    /// Open cylinder along Z, `rows` rings from `-height/2` to `height/2`, wound
    /// counter-clockwise seen from outside.
    fn open_cylinder(radius: f64, height: f64, segments: u32, rows: u32) -> TriMesh {
        let positions = (0..rows)
            .flat_map(|r| {
                (0..segments).map(move |s| {
                    let phi = std::f64::consts::TAU * s as f64 / segments as f64;
                    DVec3::new(
                        radius * phi.cos(),
                        radius * phi.sin(),
                        height * (r as f64 / (rows - 1) as f64 - 0.5),
                    )
                })
            })
            .collect();
        let at = |r: u32, s: u32| r * segments + s % segments;
        let mut triangles = Vec::with_capacity((segments * (rows - 1) * 2) as usize);
        for r in 0..rows - 1 {
            for s in 0..segments {
                let (a, b) = (at(r, s), at(r, s + 1));
                let (c, d) = (at(r + 1, s), at(r + 1, s + 1));
                triangles.push([a, b, d]);
                triangles.push([a, d, c]);
            }
        }
        TriMesh::new(positions, triangles)
    }

    /// Flat grid in the XY plane wound so the normals point at +Z.
    fn grid(cols: u32, rows: u32, spacing: f64) -> TriMesh {
        let positions = (0..rows)
            .flat_map(|r| {
                (0..cols).map(move |c| DVec3::new(c as f64 * spacing, r as f64 * spacing, 0.0))
            })
            .collect();
        let at = |r: u32, c: u32| r * cols + c;
        let mut triangles = Vec::with_capacity(((cols - 1) * (rows - 1) * 2) as usize);
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let (a, b, d, cc) = (at(r, c), at(r, c + 1), at(r + 1, c + 1), at(r + 1, c));
                triangles.push([a, b, d]);
                triangles.push([a, d, cc]);
            }
        }
        TriMesh::new(positions, triangles)
    }

    #[test]
    fn empty_mesh_gives_no_curvature() {
        let mesh = TriMesh::default();
        assert!(mesh.mean_curvature().is_empty());
        assert!(mesh.principal_curvatures().is_empty());
    }

    #[test]
    fn sphere_mean_curvature_is_inverse_radius() {
        let radius = 5.0;
        let (segments, rings) = (64, 32);
        let mesh = TriMesh::uv_sphere(radius, segments, rings);
        let mean = mesh.mean_curvature();
        // The pole rows (the two pole vertices and the rings beside them) sit on fans and
        // skinny triangles whose area-weighted normals are too crude; the task excludes them.
        let band = radius * (1.5 * std::f64::consts::PI / rings as f64).cos();
        for (i, &h) in mean.iter().enumerate() {
            if mesh.positions[i].z.abs() >= band {
                continue;
            }
            assert!(
                (h - 0.2).abs() < 0.2 * 0.03,
                "mean curvature {h} outside 3% of 0.2 at vertex {i}"
            );
        }
    }

    #[test]
    fn sphere_principal_curvatures_are_isotropic() {
        let radius = 5.0;
        let (segments, rings) = (64, 32);
        let mesh = TriMesh::uv_sphere(radius, segments, rings);
        let normals = mesh.vertex_normals();
        let principal = mesh.principal_curvatures();
        // The two pole rows beside the poles sit on fans whose skinny triangles skew the
        // per-edge normal-curvature estimate; the task excludes them.
        let band = radius * (2.5 * std::f64::consts::PI / rings as f64).cos();
        for (i, p) in mesh.positions.iter().enumerate() {
            if p.z.abs() >= band {
                continue;
            }
            let pc = principal[i];
            assert!(
                (pc.k1 - 0.2).abs() < 0.2 * 0.05,
                "k1 {} at vertex {i}",
                pc.k1
            );
            assert!(
                (pc.k2 - 0.2).abs() < 0.2 * 0.05,
                "k2 {} at vertex {i}",
                pc.k2
            );
            assert!(pc.k1 >= pc.k2);
            assert!((pc.dir1.length() - 1.0).abs() < 1e-9);
            assert!((pc.dir2.length() - 1.0).abs() < 1e-9);
            assert!(pc.dir1.dot(pc.dir2).abs() < 1e-9);
            assert!(pc.dir1.dot(normals[i]).abs() < 1e-9);
            assert!(pc.dir2.dot(normals[i]).abs() < 1e-9);
            assert!(pc.dir1.cross(pc.dir2).dot(*p) > 0.0);
        }
    }

    #[test]
    fn cylinder_curvatures_split_along_and_around() {
        let radius = 2.0;
        let mesh = open_cylinder(radius, 4.0, 64, 9);
        let mean = mesh.mean_curvature();
        let principal = mesh.principal_curvatures();
        for (i, pc) in principal.iter().enumerate() {
            // Every vertex of the open side is a boundary vertex; the fit still works there.
            assert!(
                (pc.k1 - 0.5).abs() < 0.5 * 0.05,
                "k1 {} at vertex {i}",
                pc.k1
            );
            assert!(pc.k2.abs() < 0.02, "k2 {} at vertex {i}", pc.k2);
            // dir1 runs around the circumference, dir2 along the axis (signs are arbitrary).
            let tilt = 5.0f64.to_radians().sin();
            assert!(pc.dir1.z.abs() < tilt, "dir1 {pc:?}");
            assert!((pc.dir2.z.abs() - 1.0).abs() < tilt);
            assert!(
                (mean[i] - 0.25).abs() < 0.01,
                "mean {} at vertex {i}",
                mean[i]
            );
        }
    }

    #[test]
    fn flat_grid_has_no_curvature() {
        let mesh = grid(12, 12, 0.25);
        assert!(mesh.mean_curvature().iter().all(|&h| h.abs() < 1e-9));
        for pc in mesh.principal_curvatures() {
            assert!(pc.k1.abs() < 1e-9 && pc.k2.abs() < 1e-9, "{pc:?}");
            assert!((pc.dir1.length() - 1.0).abs() < 1e-9);
            assert!((pc.dir2.length() - 1.0).abs() < 1e-9);
            assert!(pc.dir1.z.abs() < 1e-9 && pc.dir2.z.abs() < 1e-9);
        }
    }

    #[test]
    fn degenerate_neighbourhoods_stay_finite() {
        let mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::Y,
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(3.0, 0.0, 0.0),
                DVec3::new(4.0, 0.0, 0.0),
                DVec3::splat(9.0),
            ],
            vec![
                [0, 1, 2],
                [2, 1, 3],
                [4, 5, 6], // zero-area: collinear corners
            ],
        );
        let mean = mesh.mean_curvature();
        let principal = mesh.principal_curvatures();
        assert_eq!(mean.len(), mesh.vertex_count());
        assert!(mean.iter().all(|h| h.is_finite()));
        assert_eq!(mean[7], 0.0); // isolated vertex
        assert!(
            principal
                .iter()
                .all(|pc| pc.k1.is_finite() && pc.k2.is_finite())
        );
        assert!(principal[7].k1 == 0.0 && principal[7].k2 == 0.0);
        // The zero-area triangle contributes nothing to its corners.
        assert!(mean[4] == 0.0 && mean[5] == 0.0 && mean[6] == 0.0);
    }
}
