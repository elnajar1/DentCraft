//! Triangle meshes for dental scans and restorations.
//!
//! Geometry is stored in `f64`. Units are millimetres.

mod bvh;
mod measure;
mod repair;
mod shapes;
mod topology;

pub use bvh::{Bvh, ClosestPoint, RayHit};
pub use topology::EdgeMap;

use glam::DVec3;
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }

    /// Length of the box diagonal.
    pub fn diagonal(&self) -> f64 {
        self.size().length()
    }
}

/// An indexed triangle mesh. Triangles wind counter-clockwise seen from outside.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TriMesh {
    pub positions: Vec<DVec3>,
    pub triangles: Vec<[u32; 3]>,
}

impl TriMesh {
    pub fn new(positions: Vec<DVec3>, triangles: Vec<[u32; 3]>) -> Self {
        Self {
            positions,
            triangles,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Bounds of the vertices, or `None` for a mesh without vertices.
    pub fn bounds(&self) -> Option<Aabb> {
        let first = *self.positions.first()?;
        let (min, max) = self
            .positions
            .iter()
            .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        Some(Aabb { min, max })
    }

    pub fn triangle(&self, index: usize) -> [DVec3; 3] {
        self.triangles[index].map(|v| self.positions[v as usize])
    }

    /// Unnormalised normal of a triangle; its length is twice the triangle's area.
    pub fn triangle_cross(&self, index: usize) -> DVec3 {
        let [a, b, c] = self.triangle(index);
        (b - a).cross(c - a)
    }

    /// Area-weighted vertex normals. Vertices with no area around them get `DVec3::Z`.
    pub fn vertex_normals(&self) -> Vec<DVec3> {
        let mut normals = vec![DVec3::ZERO; self.positions.len()];
        for (i, tri) in self.triangles.iter().enumerate() {
            let n = self.triangle_cross(i);
            for v in tri {
                normals[*v as usize] += n;
            }
        }
        normals
            .into_iter()
            .map(|n| n.try_normalize().unwrap_or(DVec3::Z))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Bound for tests that guard against quadratic time; looser where tests run unoptimised.
    pub(crate) const TIME_LIMIT_SECONDS: f64 = if cfg!(debug_assertions) { 10.0 } else { 2.0 };

    pub(crate) fn unit_tetrahedron() -> TriMesh {
        TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z],
            vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
        )
    }

    /// Axis-aligned cube of edge `s` at the origin, wound counter-clockwise seen from outside.
    pub(crate) fn cube(s: f64) -> TriMesh {
        let positions = (0..8)
            .map(|i| DVec3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64) * s)
            .collect();
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let triangles = quads
            .iter()
            .flat_map(|[a, b, c, d]| [[*a, *b, *c], [*a, *c, *d]])
            .collect();
        TriMesh::new(positions, triangles)
    }

    #[test]
    fn bounds_and_counts() {
        let mesh = unit_tetrahedron();
        let b = mesh.bounds().unwrap();
        assert_eq!((b.min, b.max), (DVec3::ZERO, DVec3::ONE));
        assert_eq!(mesh.triangle_count(), 4);
        assert!(TriMesh::default().bounds().is_none());
    }

    #[test]
    fn normals_point_outwards() {
        let mesh = unit_tetrahedron();
        let normals = mesh.vertex_normals();
        let center = DVec3::splat(0.25);
        for (p, n) in mesh.positions.iter().zip(&normals) {
            assert!((*p - center).dot(*n) > 0.0);
            assert!((n.length() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn isolated_vertex_gets_a_default_normal() {
        let mut mesh = unit_tetrahedron();
        mesh.positions.push(DVec3::splat(5.0));
        assert_eq!(*mesh.vertex_normals().last().unwrap(), DVec3::Z);
    }
}
