//! Procedural shapes, mainly for tests and previews.

use glam::DVec3;

use crate::TriMesh;

impl TriMesh {
    /// Closed UV sphere centred on the origin, wound counter-clockwise seen from outside.
    /// `segments` (around the Z axis, at least 3) by `rings` (pole to pole, at least 2) gives
    /// `2 + segments·(rings − 1)` vertices and `2·segments·(rings − 1)` triangles.
    pub fn uv_sphere(radius: f64, segments: u32, rings: u32) -> TriMesh {
        let segments = segments.max(3);
        let rings = rings.max(2);
        let mut positions = vec![DVec3::Z * radius];
        for ring in 1..rings {
            let theta = std::f64::consts::PI * ring as f64 / rings as f64;
            for segment in 0..segments {
                let phi = std::f64::consts::TAU * segment as f64 / segments as f64;
                positions.push(
                    DVec3::new(
                        theta.sin() * phi.cos(),
                        theta.sin() * phi.sin(),
                        theta.cos(),
                    ) * radius,
                );
            }
        }
        let south = positions.len() as u32;
        positions.push(-DVec3::Z * radius);

        let ring_vertex = |ring: u32, segment: u32| 1 + (ring - 1) * segments + segment % segments;
        let mut triangles = Vec::with_capacity((2 * segments * (rings - 1)) as usize);
        for s in 0..segments {
            triangles.push([0, ring_vertex(1, s), ring_vertex(1, s + 1)]);
        }
        for ring in 1..rings - 1 {
            for s in 0..segments {
                let (a, b) = (ring_vertex(ring, s), ring_vertex(ring, s + 1));
                let (c, d) = (ring_vertex(ring + 1, s), ring_vertex(ring + 1, s + 1));
                triangles.push([a, c, d]);
                triangles.push([a, d, b]);
            }
        }
        for s in 0..segments {
            triangles.push([
                ring_vertex(rings - 1, s),
                south,
                ring_vertex(rings - 1, s + 1),
            ]);
        }
        TriMesh::new(positions, triangles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv_sphere_is_closed_and_outward() {
        let mesh = TriMesh::uv_sphere(2.0, 32, 16);
        assert_eq!(mesh.vertex_count(), 2 + 32 * 15);
        assert_eq!(mesh.triangle_count(), 2 * 32 * 15);
        assert!(mesh.is_closed());
        assert!(mesh.is_consistently_oriented());
        let exact = 4.0 / 3.0 * std::f64::consts::PI * 8.0;
        let volume = mesh.signed_volume();
        assert!(volume > 0.95 * exact && volume < exact);
        for p in &mesh.positions {
            assert!((p.length() - 2.0).abs() < 1e-12);
        }
    }

    #[test]
    fn uv_sphere_clamps_tiny_resolutions() {
        let mesh = TriMesh::uv_sphere(1.0, 0, 0);
        assert_eq!((mesh.vertex_count(), mesh.triangle_count()), (5, 6));
        assert!(mesh.is_closed());
        assert!(mesh.signed_volume() > 0.0);
    }
}
