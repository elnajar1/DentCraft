//! Surface area and enclosed volume.

use crate::TriMesh;

impl TriMesh {
    /// Total triangle area, in mm².
    pub fn surface_area(&self) -> f64 {
        // `Sum` starts from -0.0, which an empty mesh would report.
        (0..self.triangles.len())
            .map(|i| self.triangle_cross(i).length())
            .fold(0.0, |sum, a| sum + a)
            * 0.5
    }

    /// Enclosed volume in mm³ by the divergence theorem: positive for a closed mesh wound
    /// counter-clockwise seen from outside, negative when wound inwards. Meaningless for open
    /// meshes.
    pub fn signed_volume(&self) -> f64 {
        // Measuring from a vertex rather than the origin keeps far-away scans accurate.
        let Some(&origin) = self.positions.first() else {
            return 0.0;
        };
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| self.positions[v as usize] - origin);
                a.dot(b.cross(c))
            })
            .sum::<f64>()
            / 6.0
    }
}

#[cfg(test)]
mod tests {
    use crate::TriMesh;
    use crate::tests::{cube, unit_tetrahedron};
    use glam::DVec3;

    #[test]
    fn cube_area_and_volume() {
        let s = 2.5;
        let mesh = cube(s);
        assert!((mesh.surface_area() - 6.0 * s * s).abs() < 1e-12);
        assert!((mesh.signed_volume() - s * s * s).abs() < 1e-12);
    }

    #[test]
    fn far_away_cube_keeps_its_volume() {
        let mut mesh = cube(1.0);
        for p in &mut mesh.positions {
            *p += DVec3::splat(1e6);
        }
        assert!((mesh.signed_volume() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn tetrahedron_volume_and_inward_winding() {
        let mut mesh = unit_tetrahedron();
        assert!((mesh.signed_volume() - 1.0 / 6.0).abs() < 1e-15);
        mesh.flip_orientation();
        assert!((mesh.signed_volume() + 1.0 / 6.0).abs() < 1e-15);
    }

    #[test]
    fn empty_and_degenerate_meshes_measure_zero() {
        let empty = TriMesh::default();
        assert_eq!((empty.surface_area(), empty.signed_volume()), (0.0, 0.0));
        assert!(empty.surface_area().is_sign_positive());
        let flat = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::X * 2.0],
            vec![[0, 1, 2], [0, 0, 1]],
        );
        assert_eq!(flat.surface_area(), 0.0);
        assert_eq!(flat.signed_volume(), 0.0);
    }

    #[test]
    fn single_triangle_area() {
        let mesh = TriMesh::new(vec![DVec3::ZERO, DVec3::X, DVec3::Y], vec![[0, 1, 2]]);
        assert!((mesh.surface_area() - 0.5).abs() < 1e-15);
    }
}
