//! Repair and editing: welding, clean-up, transforms and combining meshes.

use std::collections::HashMap;

use glam::{DMat4, DVec3};

use crate::TriMesh;
use crate::topology::has_distinct_corners;

impl TriMesh {
    /// Merges each vertex into the lowest-indexed earlier kept vertex within `tolerance` (mm) and
    /// drops triangles that collapse. A tolerance that is not a positive finite number merges
    /// only identical positions. Returns the old-to-new vertex index map.
    pub fn weld(&mut self, tolerance: f64) -> Vec<u32> {
        let tolerance = if tolerance > 0.0 && tolerance.is_finite() {
            tolerance
        } else {
            0.0
        };
        let cell_size = if tolerance > 0.0 { tolerance } else { 1.0 };
        let cell = |p: DVec3| (p / cell_size).floor().to_array().map(|c| c as i64);
        let max_distance_squared = tolerance * tolerance;
        let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
        let mut kept: Vec<DVec3> = Vec::new();
        let mut remap = Vec::with_capacity(self.positions.len());
        for &p in &self.positions {
            let [x, y, z] = cell(p);
            let mut found: Option<u32> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let key = [x.wrapping_add(dx), y.wrapping_add(dy), z.wrapping_add(dz)];
                        for &j in grid.get(&key).into_iter().flatten() {
                            if kept[j as usize].distance_squared(p) <= max_distance_squared
                                && found.is_none_or(|f| j < f)
                            {
                                found = Some(j);
                            }
                        }
                    }
                }
            }
            let index = found.unwrap_or_else(|| {
                let j = kept.len() as u32;
                kept.push(p);
                grid.entry([x, y, z]).or_default().push(j);
                j
            });
            remap.push(index);
        }
        self.positions = kept;
        for t in &mut self.triangles {
            *t = t.map(|v| remap[v as usize]);
        }
        self.triangles.retain(has_distinct_corners);
        remap
    }

    /// Removes triangles whose area is at most `min_area` (mm²) or that repeat a vertex.
    /// Returns how many were removed.
    pub fn remove_degenerate_triangles(&mut self, min_area: f64) -> usize {
        let before = self.triangles.len();
        let positions = &self.positions;
        self.triangles.retain(|t| {
            let [a, b, c] = t.map(|v| positions[v as usize]);
            has_distinct_corners(t) && (b - a).cross(c - a).length() * 0.5 > min_area
        });
        before - self.triangles.len()
    }

    /// Removes vertices no triangle uses, keeping the order of the rest. Returns the old-to-new
    /// vertex index map, with `u32::MAX` for removed vertices.
    pub fn remove_unreferenced_vertices(&mut self) -> Vec<u32> {
        let mut remap = vec![u32::MAX; self.positions.len()];
        for t in &self.triangles {
            for &v in t {
                remap[v as usize] = 0;
            }
        }
        let mut next = 0;
        let mut positions = Vec::new();
        for (i, slot) in remap.iter_mut().enumerate() {
            if *slot != u32::MAX {
                *slot = next;
                next += 1;
                positions.push(self.positions[i]);
            }
        }
        self.positions = positions;
        for t in &mut self.triangles {
            *t = t.map(|v| remap[v as usize]);
        }
        remap
    }

    /// Applies an affine transform. A mirroring transform also flips the winding so the surface
    /// keeps facing outwards.
    pub fn transform(&mut self, matrix: &DMat4) {
        for p in &mut self.positions {
            *p = matrix.transform_point3(*p);
        }
        if matrix.determinant() < 0.0 {
            self.flip_orientation();
        }
    }

    /// Appends another mesh's vertices and triangles.
    pub fn merge(&mut self, other: &TriMesh) {
        let offset = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.triangles
            .extend(other.triangles.iter().map(|t| t.map(|v| v + offset)));
    }

    /// Reverses the winding of every triangle.
    pub fn flip_orientation(&mut self) {
        for t in &mut self.triangles {
            t.swap(1, 2);
        }
    }

    /// A new mesh of the given triangles and only the vertices they use, in order of first use.
    /// Panics if an index is out of range.
    pub fn extract_triangles(&self, triangles: &[u32]) -> TriMesh {
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut mesh = TriMesh::default();
        for &i in triangles {
            let t = self.triangles[i as usize].map(|v| {
                *remap.entry(v).or_insert_with(|| {
                    mesh.positions.push(self.positions[v as usize]);
                    (mesh.positions.len() - 1) as u32
                })
            });
            mesh.triangles.push(t);
        }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{cube, unit_tetrahedron};

    /// The cube with every triangle given its own three corners, as STL stores it.
    fn triangle_soup(mesh: &TriMesh) -> TriMesh {
        let mut soup = TriMesh::default();
        for i in 0..mesh.triangle_count() {
            let base = soup.positions.len() as u32;
            soup.positions.extend(mesh.triangle(i));
            soup.triangles.push([base, base + 1, base + 2]);
        }
        soup
    }

    #[test]
    fn weld_rebuilds_a_closed_cube_from_soup() {
        let mut mesh = triangle_soup(&cube(1.0));
        assert_eq!(mesh.vertex_count(), 36);
        assert!(!mesh.is_closed());
        let remap = mesh.weld(1e-6);
        assert_eq!(remap.len(), 36);
        assert_eq!((mesh.vertex_count(), mesh.triangle_count()), (8, 12));
        assert!(mesh.is_closed());
        assert!((mesh.signed_volume() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn weld_merges_within_tolerance_only() {
        let mut mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::Y,
                DVec3::new(0.0, 0.0, 0.004),
                DVec3::new(1.0, 0.0, 0.02),
            ],
            vec![[0, 1, 2], [3, 1, 2], [0, 4, 2]],
        );
        let remap = mesh.weld(0.01);
        assert_eq!(remap, vec![0, 1, 2, 0, 3]);
        // [3, 1, 2] became a copy of [0, 1, 2]; nothing collapsed.
        assert_eq!(mesh.triangles, vec![[0, 1, 2], [0, 1, 2], [0, 3, 2]]);
    }

    #[test]
    fn weld_across_cell_borders_and_drops_collapsed_triangles() {
        let mut mesh = TriMesh::new(
            vec![
                DVec3::splat(0.0999),
                DVec3::splat(0.1001),
                DVec3::new(5.0, 0.0, 0.0),
            ],
            vec![[0, 1, 2]],
        );
        mesh.weld(0.1);
        assert_eq!(mesh.vertex_count(), 2);
        assert!(mesh.triangles.is_empty());
    }

    #[test]
    fn weld_with_zero_tolerance_merges_identical_points_only() {
        let mut mesh = TriMesh::new(
            vec![DVec3::ZERO, DVec3::ZERO, DVec3::new(1e-12, 0.0, 0.0)],
            vec![],
        );
        assert_eq!(mesh.weld(0.0), vec![0, 0, 1]);
        assert_eq!(mesh.clone().weld(f64::NAN), vec![0, 1]);
        let mut empty = TriMesh::default();
        assert!(empty.weld(1.0).is_empty());
    }

    #[test]
    fn degenerate_triangles_are_removed() {
        let mut mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::Y,
                DVec3::X * 2.0,
                DVec3::new(0.0, 1e-4, 0.0),
            ],
            vec![[0, 1, 2], [0, 1, 3], [0, 0, 2], [0, 1, 4]],
        );
        assert_eq!(mesh.remove_degenerate_triangles(0.0), 2);
        assert_eq!(mesh.triangles, vec![[0, 1, 2], [0, 1, 4]]);
        assert_eq!(mesh.remove_degenerate_triangles(1e-3), 1);
        assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
        assert_eq!(TriMesh::default().remove_degenerate_triangles(0.0), 0);
    }

    #[test]
    fn unreferenced_vertices_are_removed_in_order() {
        let mut mesh = TriMesh::new(
            vec![
                DVec3::splat(9.0),
                DVec3::ZERO,
                DVec3::X,
                DVec3::splat(7.0),
                DVec3::Y,
            ],
            vec![[1, 2, 4]],
        );
        let remap = mesh.remove_unreferenced_vertices();
        assert_eq!(remap, vec![u32::MAX, 0, 1, u32::MAX, 2]);
        assert_eq!(mesh.positions, vec![DVec3::ZERO, DVec3::X, DVec3::Y]);
        assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
        let mut empty = TriMesh::default();
        assert!(empty.remove_unreferenced_vertices().is_empty());
    }

    #[test]
    fn mirrored_transform_keeps_volume_positive() {
        let mut mesh = cube(2.0);
        mesh.transform(&DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0)));
        assert!((mesh.signed_volume() - 8.0).abs() < 1e-12);
        assert!(mesh.is_consistently_oriented());
        let b = mesh.bounds().unwrap();
        assert_eq!((b.min.x, b.max.x), (-2.0, 0.0));
    }

    #[test]
    fn rigid_transform_moves_without_flipping() {
        let mut mesh = unit_tetrahedron();
        let m = DMat4::from_translation(DVec3::new(1.0, 2.0, 3.0))
            * DMat4::from_rotation_z(std::f64::consts::FRAC_PI_2);
        mesh.transform(&m);
        assert_eq!(mesh.triangles, unit_tetrahedron().triangles);
        assert!(mesh.positions[1].distance(DVec3::new(1.0, 3.0, 3.0)) < 1e-12);
        assert!((mesh.signed_volume() - 1.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn merge_offsets_indices() {
        let mut mesh = TriMesh::default();
        mesh.merge(&unit_tetrahedron());
        mesh.merge(&unit_tetrahedron());
        assert_eq!((mesh.vertex_count(), mesh.triangle_count()), (8, 8));
        assert_eq!(mesh.triangles[4], [4, 6, 5]);
    }

    #[test]
    fn flip_twice_is_identity() {
        let mut mesh = cube(1.0);
        mesh.flip_orientation();
        assert_eq!(mesh.triangles[0], [0, 3, 2]);
        mesh.flip_orientation();
        assert_eq!(mesh, cube(1.0));
    }

    #[test]
    fn extract_keeps_only_used_vertices() {
        let mesh = cube(1.0);
        let part = mesh.extract_triangles(&[2, 3]);
        assert_eq!(part.triangle_count(), 2);
        assert_eq!(part.vertex_count(), 4);
        assert_eq!(part.triangles, vec![[0, 1, 2], [0, 2, 3]]);
        assert_eq!(part.triangle(0), mesh.triangle(2));
        assert!(mesh.extract_triangles(&[]).positions.is_empty());
    }
}
