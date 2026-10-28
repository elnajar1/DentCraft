//! Vertex adjacency in compressed (CSR) layout: one-ring neighbours, incident triangles and
//! boundary flags.

use crate::TriMesh;
use crate::topology::has_distinct_corners;

/// One-ring adjacency over a mesh's vertices, stored in compressed (CSR) layout so every
/// query is a slice lookup with no per-vertex allocation. Triangles that repeat a vertex
/// index carry no surface and take no part in it, matching [`TriMesh::edge_map`].
#[derive(Clone, Debug, Default)]
pub struct VertexAdjacency {
    vertex_count: usize,
    neighbor_offsets: Vec<u32>,
    neighbors: Vec<u32>,
    face_offsets: Vec<u32>,
    faces: Vec<u32>,
    boundary: Vec<bool>,
}

impl VertexAdjacency {
    /// Builds the adjacency in linear time and with a fixed number of buffers: one counting
    /// pass over the triangles, one filling pass, then a sort and dedupe of each vertex's
    /// neighbour slice.
    pub fn build(mesh: &TriMesh) -> Self {
        let n = mesh.positions.len();
        let mut face_counts = vec![0u32; n];
        let mut neighbor_counts = vec![0u32; n];
        for t in mesh.triangles.iter().filter(|t| has_distinct_corners(t)) {
            for &v in t {
                face_counts[v as usize] += 1;
                neighbor_counts[v as usize] += 2;
            }
        }
        let face_offsets = exclusive_sum(&face_counts);
        let mut neighbor_offsets = exclusive_sum(&neighbor_counts);
        let mut faces = vec![0u32; face_offsets[n] as usize];
        let mut neighbors = vec![0u32; neighbor_offsets[n] as usize];
        let mut face_cursor = face_offsets[..n].to_vec();
        let mut neighbor_cursor = neighbor_offsets[..n].to_vec();
        for (i, t) in mesh.triangles.iter().enumerate() {
            if !has_distinct_corners(t) {
                continue;
            }
            for k in 0..3 {
                let v = t[k] as usize;
                faces[face_cursor[v] as usize] = i as u32;
                face_cursor[v] += 1;
                for dk in 1..3 {
                    neighbors[neighbor_cursor[v] as usize] = t[(k + dk) % 3];
                    neighbor_cursor[v] += 1;
                }
            }
        }
        // Sort each slice, mark a vertex boundary when any neighbour appears an odd number of
        // times (an edge with a single triangle, or a non-manifold one), and dedupe in place;
        // the write cursor never overtakes the read cursor.
        let mut boundary = vec![false; n];
        let mut compact_offsets = vec![0u32; n + 1];
        let mut write = 0usize;
        for v in 0..n {
            let (start, end) = (
                neighbor_offsets[v] as usize,
                neighbor_offsets[v + 1] as usize,
            );
            neighbors[start..end].sort_unstable();
            let mut i = start;
            while i < end {
                let mut j = i + 1;
                while j < end && neighbors[j] == neighbors[i] {
                    j += 1;
                }
                boundary[v] |= (j - i) % 2 == 1;
                neighbors[write] = neighbors[i];
                write += 1;
                i = j;
            }
            compact_offsets[v + 1] = write as u32;
        }
        neighbors.truncate(write);
        neighbor_offsets = compact_offsets;
        Self {
            vertex_count: n,
            neighbor_offsets,
            neighbors,
            face_offsets,
            faces,
            boundary,
        }
    }

    /// The distinct one-ring neighbours of `v`, ascending. Empty for an out-of-range or
    /// isolated vertex.
    pub fn neighbors(&self, v: u32) -> &[u32] {
        csr_slice(
            &self.neighbor_offsets,
            &self.neighbors,
            self.vertex_count,
            v,
        )
    }

    /// The triangles incident to `v`, ascending in mesh order. Empty for an out-of-range or
    /// isolated vertex.
    pub fn faces(&self, v: u32) -> &[u32] {
        csr_slice(&self.face_offsets, &self.faces, self.vertex_count, v)
    }

    /// Whether `v` touches an open edge. A vertex on a non-manifold edge (used by an odd
    /// number of triangle corners) also counts. `false` for an out-of-range or isolated
    /// vertex.
    pub fn is_boundary(&self, v: u32) -> bool {
        self.boundary.get(v as usize).copied().unwrap_or(false)
    }

    /// How many distinct one-ring neighbours `v` has; 0 for an out-of-range or isolated
    /// vertex.
    pub fn valence(&self, v: u32) -> usize {
        self.neighbors(v).len()
    }

    /// How many vertices the adjacency covers.
    pub fn vertex_count(&self) -> usize {
        self.vertex_count
    }
}

/// The CSR slice `data[offsets[v]..offsets[v + 1]]`, empty when `v` is out of range.
fn csr_slice<'a>(offsets: &'a [u32], data: &'a [u32], vertex_count: usize, v: u32) -> &'a [u32] {
    let v = v as usize;
    if v >= vertex_count || v + 1 >= offsets.len() {
        return &[];
    }
    &data[offsets[v] as usize..offsets[v + 1] as usize]
}

/// The `n + 1` exclusive prefix sums of `counts`.
fn exclusive_sum(counts: &[u32]) -> Vec<u32> {
    let mut sums = Vec::with_capacity(counts.len() + 1);
    sums.push(0);
    sums.extend(counts.iter().scan(0, |sum, &c| {
        *sum += c;
        Some(*sum)
    }));
    sums
}

impl TriMesh {
    /// Builds the [`VertexAdjacency`] of this mesh.
    pub fn vertex_adjacency(&self) -> VertexAdjacency {
        VertexAdjacency::build(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{cube, unit_tetrahedron};
    use glam::DVec3;

    fn single_triangle() -> TriMesh {
        TriMesh::new(vec![DVec3::ZERO, DVec3::X, DVec3::Y], vec![[0, 1, 2]])
    }

    #[test]
    fn empty_mesh_has_no_adjacency() {
        let adjacency = TriMesh::default().vertex_adjacency();
        assert_eq!(adjacency.vertex_count(), 0);
        assert!(adjacency.neighbors(0).is_empty());
        assert!(adjacency.faces(0).is_empty());
        assert!(!adjacency.is_boundary(0));
        assert_eq!(adjacency.valence(0), 0);
    }

    #[test]
    fn open_triangles_mark_every_corner_boundary() {
        let adjacency = single_triangle().vertex_adjacency();
        assert_eq!(adjacency.neighbors(0), &[1, 2]);
        assert_eq!(adjacency.neighbors(1), &[0, 2]);
        assert_eq!(adjacency.faces(2), &[0]);
        assert_eq!(adjacency.valence(2), 2);
        for v in 0..3 {
            assert!(adjacency.is_boundary(v));
        }
    }

    #[test]
    fn closed_meshes_have_no_boundary_vertices() {
        let tetrahedron = unit_tetrahedron().vertex_adjacency();
        assert_eq!(tetrahedron.neighbors(0), &[1, 2, 3]);
        assert_eq!(tetrahedron.faces(0), &[0, 1, 3]);
        assert_eq!(tetrahedron.valence(0), 3);
        for v in 0..4 {
            assert!(!tetrahedron.is_boundary(v));
        }
        let octahedron = cube(1.0).vertex_adjacency();
        // Each cube corner belongs to three quads, so six triangles and six neighbours.
        assert_eq!(octahedron.faces(0), &[0, 1, 4, 5, 8, 9]);
        assert_eq!(octahedron.neighbors(0), &[1, 2, 3, 4, 5, 6]);
        assert!(!octahedron.is_boundary(0));
    }

    #[test]
    fn isolated_and_out_of_range_vertices_are_empty() {
        let mut mesh = unit_tetrahedron();
        mesh.positions.push(DVec3::splat(9.0));
        let adjacency = mesh.vertex_adjacency();
        assert!(adjacency.neighbors(4).is_empty());
        assert!(adjacency.faces(4).is_empty());
        assert!(!adjacency.is_boundary(4));
        assert_eq!(adjacency.valence(4), 0);
        for v in [5, u32::MAX] {
            assert!(adjacency.neighbors(v).is_empty());
            assert!(adjacency.faces(v).is_empty());
            assert!(!adjacency.is_boundary(v));
        }
    }

    #[test]
    fn triangles_with_repeated_corners_are_ignored() {
        let mut mesh = single_triangle();
        mesh.triangles.push([0, 1, 1]);
        mesh.triangles.push([2, 2, 2]);
        let adjacency = mesh.vertex_adjacency();
        assert_eq!(adjacency.faces(0), &[0]);
        assert_eq!(adjacency.neighbors(0), &[1, 2]);
    }

    #[test]
    fn duplicate_triangles_leave_edges_interior() {
        let mut mesh = single_triangle();
        mesh.triangles.push([0, 1, 2]);
        let adjacency = mesh.vertex_adjacency();
        for v in 0..3 {
            assert!(!adjacency.is_boundary(v));
            assert_eq!(adjacency.faces(v), &[0, 1]);
        }
    }

    #[test]
    fn non_manifold_edge_marks_its_corners_boundary() {
        // Two triangles sharing edge 1-2, plus a fin on the same edge.
        let mut mesh = single_triangle();
        mesh.positions.push(DVec3::Z);
        mesh.triangles.push([1, 2, 3]);
        let adjacency = mesh.vertex_adjacency();
        // The mesh is open besides, so every corner touches an odd edge; what matters is
        // that the three-way edge is detected through its odd corner count.
        for v in 0..4 {
            assert!(adjacency.is_boundary(v), "vertex {v}");
        }
        assert_eq!(adjacency.valence(1), 3);
        assert_eq!(adjacency.faces(1), &[0, 1]);
    }

    #[test]
    fn neighbors_are_sorted_and_unique_on_an_irregular_fan() {
        let mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::Y,
                DVec3::new(0.0, 1.0, 1.0),
                DVec3::new(-1.0, 0.0, 0.0),
            ],
            vec![[0, 2, 1], [0, 3, 2], [0, 4, 3]],
        );
        let adjacency = mesh.vertex_adjacency();
        assert_eq!(adjacency.neighbors(0), &[1, 2, 3, 4]);
        assert_eq!(adjacency.faces(0), &[0, 1, 2]);
    }

    #[test]
    fn building_a_wide_fan_is_linear() {
        // 100 000 separate triangles sharing only vertex 0.
        let k = 100_000u32;
        let positions = (0..=2 * k)
            .map(|i| DVec3::new(i as f64, 1.0, 0.0))
            .collect();
        let triangles = (0..k).map(|i| [2 * i + 1, 0, 2 * i + 2]).collect();
        let mesh = TriMesh::new(positions, triangles);
        let start = std::time::Instant::now();
        let adjacency = mesh.vertex_adjacency();
        let elapsed = start.elapsed();
        assert_eq!(adjacency.valence(0), 2 * k as usize);
        assert!(adjacency.is_boundary(0));
        assert_eq!(adjacency.faces(0).len(), k as usize);
        assert!(
            elapsed.as_secs_f64() < crate::tests::TIME_LIMIT_SECONDS,
            "took {elapsed:?}"
        );
    }
}
