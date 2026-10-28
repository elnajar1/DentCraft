//! Edge adjacency: boundaries, connected parts, closedness and orientation.
//!
//! Triangles that repeat a vertex index carry no surface and take no part in edge adjacency.

use std::collections::{HashMap, HashSet};

use crate::TriMesh;

/// Undirected edges keyed `[low, high]`, each with the triangles that use it, in mesh order.
pub type EdgeMap = HashMap<[u32; 2], Vec<u32>>;

fn edge_key([a, b]: [u32; 2]) -> [u32; 2] {
    if a < b { [a, b] } else { [b, a] }
}

/// The directed edges of a triangle in winding order.
fn directed_edges(t: [u32; 3]) -> [[u32; 2]; 3] {
    [[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]]
}

pub(crate) fn has_distinct_corners(t: &[u32; 3]) -> bool {
    t[0] != t[1] && t[1] != t[2] && t[0] != t[2]
}

impl TriMesh {
    pub fn edge_map(&self) -> EdgeMap {
        let mut map = EdgeMap::with_capacity(self.triangles.len() * 3 / 2);
        for (i, t) in self.triangles.iter().enumerate() {
            if has_distinct_corners(t) {
                for edge in directed_edges(*t) {
                    map.entry(edge_key(edge)).or_default().push(i as u32);
                }
            }
        }
        map
    }

    /// Whether the mesh has a surface and every edge is shared by exactly two triangles.
    pub fn is_closed(&self) -> bool {
        let map = self.edge_map();
        !map.is_empty() && map.values().all(|t| t.len() == 2)
    }

    /// Edges shared by more than two triangles.
    pub fn non_manifold_edge_count(&self) -> usize {
        self.edge_map().values().filter(|t| t.len() > 2).count()
    }

    /// Whether neighbouring triangles wind the same way: no directed edge is used twice.
    pub fn is_consistently_oriented(&self) -> bool {
        let mut seen = HashSet::with_capacity(self.triangles.len() * 3);
        self.triangles
            .iter()
            .filter(|t| has_distinct_corners(t))
            .flat_map(|t| directed_edges(*t))
            .all(|edge| seen.insert(edge))
    }

    /// Parts joined through shared edges, as ascending triangle indices, ordered by their first
    /// triangle. A triangle that repeats a vertex is a part of its own.
    pub fn connected_components(&self) -> Vec<Vec<u32>> {
        let n = self.triangles.len();
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(parent: &mut [u32], mut i: u32) -> u32 {
            while parent[i as usize] != i {
                let grand = parent[parent[i as usize] as usize];
                parent[i as usize] = grand;
                i = grand;
            }
            i
        }
        for triangles in self.edge_map().values() {
            let first = find(&mut parent, triangles[0]);
            for &t in &triangles[1..] {
                let root = find(&mut parent, t);
                parent[root as usize] = first;
            }
        }
        let mut slot = vec![usize::MAX; n];
        let mut parts: Vec<Vec<u32>> = Vec::new();
        for i in 0..n as u32 {
            let root = find(&mut parent, i) as usize;
            if slot[root] == usize::MAX {
                slot[root] = parts.len();
                parts.push(Vec::new());
            }
            parts[slot[root]].push(i);
        }
        parts
    }

    /// Open boundaries: chains of edges used by exactly one triangle, as vertex loops that follow
    /// the winding of the triangles beside them. The first vertex is not repeated at the end. Where
    /// boundaries touch at a vertex they are split into separate loops; a chain that cannot be
    /// closed (inconsistent winding) is returned as it was walked.
    pub fn boundary_loops(&self) -> Vec<Vec<u32>> {
        let map = self.edge_map();
        let edges: Vec<[u32; 2]> = self
            .triangles
            .iter()
            .filter(|t| has_distinct_corners(t))
            .flat_map(|t| directed_edges(*t))
            .filter(|e| map[&edge_key(*e)].len() == 1)
            .collect();
        let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, [a, b]) in edges.iter().enumerate() {
            at.entry(*a).or_default().push(i);
            at.entry(*b).or_default().push(i);
        }
        let mut used = vec![false; edges.len()];
        let mut loops = Vec::new();
        for start in 0..edges.len() {
            if used[start] {
                continue;
            }
            used[start] = true;
            let [first, mut current] = edges[start];
            let mut chain = vec![first];
            while current != first {
                chain.push(current);
                // Prefer an edge leaving `current` along the winding.
                let next = at[&current]
                    .iter()
                    .copied()
                    .filter(|&e| !used[e])
                    .min_by_key(|&e| edges[e][0] != current);
                let Some(e) = next else { break };
                used[e] = true;
                let [a, b] = edges[e];
                current = if a == current { b } else { a };
            }
            loops.push(chain);
        }
        loops
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

    /// Two triangles sharing the edge 1–2, forming a unit square.
    fn square() -> TriMesh {
        TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::new(1.0, 1.0, 0.0)],
            vec![[0, 1, 2], [2, 1, 3]],
        )
    }

    #[test]
    fn empty_mesh_has_no_topology() {
        let mesh = TriMesh::default();
        assert!(mesh.edge_map().is_empty());
        assert!(!mesh.is_closed());
        assert!(mesh.is_consistently_oriented());
        assert!(mesh.connected_components().is_empty());
        assert!(mesh.boundary_loops().is_empty());
        assert_eq!(mesh.non_manifold_edge_count(), 0);
    }

    #[test]
    fn single_triangle_is_one_open_loop_of_three() {
        let mesh = single_triangle();
        assert_eq!(mesh.edge_map().len(), 3);
        assert!(!mesh.is_closed());
        assert_eq!(mesh.boundary_loops(), vec![vec![0, 1, 2]]);
        assert_eq!(mesh.connected_components(), vec![vec![0]]);
    }

    #[test]
    fn square_boundary_follows_the_winding() {
        let mesh = square();
        assert_eq!(mesh.edge_map()[&[1, 2]], vec![0, 1]);
        assert_eq!(mesh.boundary_loops(), vec![vec![0, 1, 3, 2]]);
        assert!(mesh.is_consistently_oriented());
        assert_eq!(mesh.connected_components(), vec![vec![0, 1]]);
    }

    #[test]
    fn closed_meshes_have_no_boundary() {
        for mesh in [unit_tetrahedron(), cube(2.0)] {
            assert!(mesh.is_closed());
            assert!(mesh.is_consistently_oriented());
            assert!(mesh.boundary_loops().is_empty());
            assert_eq!(mesh.non_manifold_edge_count(), 0);
            assert_eq!(mesh.connected_components().len(), 1);
        }
    }

    #[test]
    fn open_cube_has_one_square_hole() {
        let mut mesh = cube(1.0);
        mesh.triangles.truncate(10);
        assert!(!mesh.is_closed());
        let loops = mesh.boundary_loops();
        assert_eq!(loops.len(), 1);
        let mut hole = loops[0].clone();
        hole.sort();
        assert_eq!(hole, vec![1, 3, 5, 7]);
    }

    #[test]
    fn two_disjoint_tetrahedra_are_two_components() {
        let mut mesh = unit_tetrahedron();
        let mut other = unit_tetrahedron();
        for p in &mut other.positions {
            *p += DVec3::splat(3.0);
        }
        mesh.merge(&other);
        assert_eq!(
            mesh.connected_components(),
            vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]
        );
        assert!(mesh.is_closed());
    }

    #[test]
    fn flipped_triangle_breaks_orientation() {
        let mut mesh = unit_tetrahedron();
        mesh.triangles[0].swap(1, 2);
        assert!(!mesh.is_consistently_oriented());
        assert!(mesh.is_closed());
    }

    #[test]
    fn fin_on_an_edge_is_non_manifold() {
        let mut mesh = square();
        mesh.positions.push(DVec3::Z);
        mesh.triangles.push([1, 2, 4]);
        assert_eq!(mesh.non_manifold_edge_count(), 1);
        assert!(!mesh.is_closed());
        assert_eq!(mesh.connected_components().len(), 1);
        // The shared edge is not a boundary; the five outer edges are, in two chains at vertex 1/2.
        let boundary: usize = mesh.boundary_loops().iter().map(Vec::len).sum();
        assert!(boundary >= 5);
    }

    #[test]
    fn triangles_with_repeated_vertices_are_ignored() {
        let mut mesh = single_triangle();
        mesh.triangles.push([0, 1, 1]);
        assert_eq!(mesh.edge_map().len(), 3);
        assert_eq!(mesh.boundary_loops(), vec![vec![0, 1, 2]]);
        assert_eq!(mesh.connected_components(), vec![vec![0], vec![1]]);
    }

    #[test]
    fn duplicate_vertices_split_the_surface() {
        // The square with vertex 2 duplicated: the triangles no longer share an edge.
        let mut mesh = square();
        mesh.positions.push(DVec3::Y);
        mesh.triangles[1] = [4, 1, 3];
        assert_eq!(mesh.connected_components().len(), 2);
        assert_eq!(mesh.boundary_loops().len(), 2);
    }

    #[test]
    fn bowtie_vertex_yields_two_loops() {
        let mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
                DVec3::new(-1.0, 0.0, 0.0),
                DVec3::new(0.0, -1.0, 0.0),
            ],
            vec![[0, 1, 2], [0, 3, 4]],
        );
        let loops = mesh.boundary_loops();
        assert_eq!(loops.len(), 2);
        assert!(loops.iter().all(|l| l.len() == 3));
    }
}
