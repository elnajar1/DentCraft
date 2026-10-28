//! Smoothing with uniform weights: plain Laplacian and Taubin's volume-preserving pair.

use glam::DVec3;

use crate::TriMesh;

impl TriMesh {
    /// Moves every free vertex a fraction `lambda` of the way to the average of its
    /// one-ring neighbours (uniform weights, in place). Each pass reads the positions as
    /// they were before it began (Jacobi order), so the result does not depend on vertex
    /// order. `lambda = 0` leaves the mesh unchanged.
    ///
    /// Vertices on an open boundary stay where they are unless `move_boundary` is `true`;
    /// free boundary vertices then take part in the averaging. Vertices marked `true` in
    /// `locked` never move, whatever `move_boundary` says. `locked` must hold exactly one
    /// entry per vertex; any other length panics. Isolated vertices have no neighbours and
    /// never move.
    pub fn laplacian_smooth(
        &mut self,
        iterations: u32,
        lambda: f64,
        locked: Option<&[bool]>,
        move_boundary: bool,
    ) {
        self.smooth_passes(iterations, &[lambda], locked, move_boundary);
    }

    /// Laplacian smoothing that keeps volume (Taubin 1995): each iteration runs one pass
    /// with `lambda` followed by one with `mu`, which requires `mu < -lambda` (commonly
    /// 0.5 and −0.53) so the shrink of the first pass is paid back by the second. Boundary
    /// and locked vertices follow the rules of [`TriMesh::laplacian_smooth`].
    pub fn taubin_smooth(
        &mut self,
        iterations: u32,
        lambda: f64,
        mu: f64,
        locked: Option<&[bool]>,
        move_boundary: bool,
    ) {
        self.smooth_passes(iterations, &[lambda, mu], locked, move_boundary);
    }

    /// Runs `iterations` rounds of one pass per entry of `scales`, swapping between the
    /// mesh's positions and one scratch buffer allocated up front.
    fn smooth_passes(
        &mut self,
        iterations: u32,
        scales: &[f64],
        locked: Option<&[bool]>,
        move_boundary: bool,
    ) {
        if let Some(mask) = locked {
            assert!(
                mask.len() == self.positions.len(),
                "locked mask has {} entries but the mesh has {} vertices",
                mask.len(),
                self.positions.len()
            );
        }
        if iterations == 0 || self.positions.is_empty() {
            return;
        }
        let adjacency = self.vertex_adjacency();
        let free: Vec<bool> = (0..self.positions.len())
            .map(|v| {
                let held = adjacency.is_boundary(v as u32) && !move_boundary;
                let locked = locked.is_some_and(|mask| mask[v]);
                !held && !locked
            })
            .collect();
        let mut scratch: Vec<DVec3> = Vec::with_capacity(self.positions.len());
        for _ in 0..iterations {
            for &scale in scales {
                scratch.clear();
                for (v, &p) in self.positions.iter().enumerate() {
                    let neighbors = adjacency.neighbors(v as u32);
                    if free[v] && !neighbors.is_empty() {
                        let mut sum = DVec3::ZERO;
                        for &j in neighbors {
                            sum += self.positions[j as usize];
                        }
                        scratch.push(p + (sum / neighbors.len() as f64 - p) * scale);
                    } else {
                        scratch.push(p);
                    }
                }
                std::mem::swap(&mut self.positions, &mut scratch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    /// A tiny deterministic generator for test noise: a 64-bit LCG, no `rand`.
    struct Lcg(u64);

    impl Lcg {
        /// Uniform in (-1, 1).
        fn unit(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 52) as f64 * 2.0 - 1.0
        }
    }

    /// A sphere with deterministic radial noise.
    fn noisy_sphere(radius: f64, segments: u32, rings: u32, amplitude: f64, seed: u64) -> TriMesh {
        let mut mesh = TriMesh::uv_sphere(radius, segments, rings);
        let mut rng = Lcg(seed);
        for p in &mut mesh.positions {
            *p += p.try_normalize().unwrap_or(DVec3::Z) * (rng.unit() * amplitude);
        }
        mesh
    }

    /// Root-mean-square radial error against a sphere of `radius`.
    fn rms_radial_error(mesh: &TriMesh, radius: f64) -> f64 {
        (mesh
            .positions
            .iter()
            .map(|p| (p.length() - radius).powi(2))
            .sum::<f64>()
            / mesh.positions.len() as f64)
            .sqrt()
    }

    /// Open cylinder along Z, wound counter-clockwise seen from outside; every vertex is on
    /// the boundary.
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

    #[test]
    fn empty_mesh_is_a_no_op() {
        let mut mesh = TriMesh::default();
        mesh.laplacian_smooth(5, 0.5, None, false);
        mesh.taubin_smooth(5, 0.5, -0.53, None, false);
        mesh.laplacian_smooth(5, 0.5, Some(&[]), false);
        assert_eq!(mesh, TriMesh::default());
    }

    #[test]
    fn zero_iterations_or_scale_change_nothing() {
        let mesh = noisy_sphere(2.0, 24, 12, 0.05, 1);
        let mut held = mesh.clone();
        held.laplacian_smooth(0, 0.9, None, false);
        assert_eq!(held, mesh);
        held.laplacian_smooth(7, 0.0, None, false);
        assert_eq!(held, mesh);
    }

    #[test]
    fn smoothing_reduces_noise_on_a_sphere() {
        let mut mesh = noisy_sphere(2.0, 48, 24, 0.04, 42);
        let before = rms_radial_error(&mesh, 2.0);
        mesh.laplacian_smooth(5, 0.6, None, false);
        let after = rms_radial_error(&mesh, 2.0);
        assert!(after < 0.8 * before, "RMS {before} -> {after}");
    }

    #[test]
    fn taubin_holds_volume_where_laplacian_shrinks() {
        let v0 = TriMesh::uv_sphere(2.0, 64, 32).signed_volume();
        let mut taubin = TriMesh::uv_sphere(2.0, 64, 32);
        taubin.taubin_smooth(20, 0.5, -0.53, None, false);
        assert!(
            (taubin.signed_volume() / v0 - 1.0).abs() < 0.02,
            "Taubin volume drift {}",
            taubin.signed_volume() / v0
        );
        let mut laplacian = TriMesh::uv_sphere(2.0, 64, 32);
        laplacian.laplacian_smooth(20, 0.5, None, false);
        assert!(
            laplacian.signed_volume() / v0 < 0.98,
            "Laplacian kept volume {}",
            laplacian.signed_volume() / v0
        );
    }

    #[test]
    fn locked_vertices_are_bit_identical() {
        let mut mesh = noisy_sphere(2.0, 32, 16, 0.05, 7);
        let locked: Vec<bool> = (0..mesh.vertex_count()).map(|i| i % 7 == 0).collect();
        let snapshot: Vec<DVec3> = mesh
            .positions
            .iter()
            .enumerate()
            .filter(|(i, _)| locked[*i])
            .map(|(_, p)| *p)
            .collect();
        mesh.laplacian_smooth(6, 0.7, Some(&locked), false);
        let after: Vec<DVec3> = mesh
            .positions
            .iter()
            .enumerate()
            .filter(|(i, _)| locked[*i])
            .map(|(_, p)| *p)
            .collect();
        assert_eq!(after, snapshot);
        // Locking everything freezes everything.
        let all: Vec<bool> = vec![true; mesh.vertex_count()];
        let frozen = mesh.clone();
        mesh.laplacian_smooth(6, 0.7, Some(&all), false);
        mesh.taubin_smooth(6, 0.5, -0.53, Some(&all), true);
        assert_eq!(mesh, frozen);
    }

    #[test]
    fn boundary_vertices_stay_fixed_by_default() {
        let (segments, rows) = (24, 5);
        let mut mesh = open_cylinder(1.0, 2.0, segments, rows);
        let before = mesh.clone();
        mesh.laplacian_smooth(8, 0.6, None, false);
        mesh.taubin_smooth(8, 0.5, -0.53, None, false);
        // Only the first and last rings are boundary vertices; they are bit-identical.
        let ring = |r: usize| r * segments as usize..(r + 1) * segments as usize;
        for r in ring(0).chain(ring(rows as usize - 1)) {
            assert_eq!(mesh.positions[r], before.positions[r]);
        }
        // The free rings in between have moved.
        assert!(ring(1).any(|i| mesh.positions[i] != before.positions[i]));

        // Freeing the boundary pulls the end rings inwards too.
        mesh.laplacian_smooth(8, 0.6, None, true);
        assert_ne!(mesh, before);
        assert!(ring(0).all(|i| mesh.positions[i] != before.positions[i]));
    }

    #[test]
    #[should_panic(expected = "locked mask has 3 entries but the mesh has 5 vertices")]
    fn locked_mask_of_the_wrong_length_panics() {
        let mut mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::Y,
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::Z,
            ],
            vec![[0, 1, 2], [2, 1, 3]],
        );
        let short = [true, true, true];
        mesh.laplacian_smooth(1, 0.5, Some(&short), false);
        mesh.taubin_smooth(1, 0.5, -0.53, Some(&short), false);
    }

    #[test]
    fn degenerate_input_smooths_without_panic() {
        let mut mesh = TriMesh::new(
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
            vec![[0, 1, 2], [2, 1, 3], [4, 5, 6]],
        );
        let isolated = mesh.positions[7];
        mesh.laplacian_smooth(3, 0.5, None, false);
        mesh.taubin_smooth(3, 0.5, -0.53, None, false);
        assert!(mesh.positions.iter().all(|p| p.is_finite()));
        assert_eq!(mesh.positions[7], isolated);
    }
}
