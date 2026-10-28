//! Edge adjacency: boundaries, connected parts, closedness and orientation.
//!
//! Triangles that repeat a vertex index carry no surface and take no part in edge adjacency.

use std::collections::{HashMap, HashSet};

use crate::TriMesh;

/// Undirected edges keyed `[low, high]`, each with the triangles that use it, in mesh order.
pub type EdgeMap = HashMap<[u32; 2], Vec<u32>>;

pub(crate) fn edge_key([a, b]: [u32; 2]) -> [u32; 2] {
    if a < b { [a, b] } else { [b, a] }
}

/// The directed edges of a triangle in winding order.
pub(crate) fn directed_edges(t: [u32; 3]) -> [[u32; 2]; 3] {
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
        // Edge `3·t + k` is edge `k` of triangle `t` in winding order.
        let edge = |id: usize| directed_edges(self.triangles[id / 3])[id % 3];
        let edges: Vec<usize> = (0..self.triangles.len() * 3)
            .filter(|&id| {
                has_distinct_corners(&self.triangles[id / 3]) && map[&edge_key(edge(id))].len() == 1
            })
            .collect();
        let mut used = vec![false; self.triangles.len() * 3];
        // Boundary edges leaving and entering each vertex, built only when the fan walk fails;
        // used ones are dropped lazily.
        type Lists = HashMap<u32, Vec<usize>>;
        let mut lists: Option<(Lists, Lists)> = None;
        let build = || {
            let (mut out, mut into): (Lists, Lists) = Default::default();
            for &id in edges.iter().rev() {
                let [a, b] = edge(id);
                out.entry(a).or_default().push(id);
                into.entry(b).or_default().push(id);
            }
            (out, into)
        };
        fn take(lists: &mut Lists, v: u32, used: &[bool]) -> Option<usize> {
            let list = lists.get_mut(&v)?;
            while let Some(&e) = list.last() {
                if !used[e] {
                    return Some(e);
                }
                list.pop();
            }
            None
        }
        let mut loops = Vec::new();
        for &start in &edges {
            if used[start] {
                continue;
            }
            used[start] = true;
            let [first, mut current] = edge(start);
            let mut arrived_by = Some(start);
            let mut chain = vec![first];
            while current != first {
                chain.push(current);
                // Prefer the edge leaving `current` beside the one we came in on, then any edge
                // along the winding, then one against it.
                let beside = arrived_by
                    .and_then(|e| self.next_in_fan(&map, current, e))
                    .filter(|&e| !used[e]);
                let next = match beside {
                    Some(e) => Some((e, true)),
                    None => {
                        let (out, into) = lists.get_or_insert_with(build);
                        take(out, current, &used)
                            .map(|e| (e, true))
                            .or_else(|| take(into, current, &used).map(|e| (e, false)))
                    }
                };
                let Some((e, along)) = next else { break };
                used[e] = true;
                arrived_by = along.then_some(e);
                current = edge(e)[usize::from(along)];
            }
            loops.push(chain);
        }
        loops
    }

    /// The boundary edge leaving `v` that is met by turning around `v` from the boundary edge
    /// `incoming` (ending at `v`), through triangles that share an edge at `v`. `None` where the
    /// fan is non-manifold or the edge found does not leave `v`.
    fn next_in_fan(&self, map: &EdgeMap, v: u32, incoming: usize) -> Option<usize> {
        let mut triangle = (incoming / 3) as u32;
        let mut prev = directed_edges(self.triangles[triangle as usize])[incoming % 3][0];
        for _ in 0..self.triangles.len() {
            let t = self.triangles[triangle as usize];
            let w = *t.iter().find(|&&c| c != v && c != prev)?;
            match map.get(&edge_key([v, w]))?.as_slice() {
                [_] => {
                    let k = directed_edges(t).iter().position(|&e| e == [v, w])?;
                    return Some(triangle as usize * 3 + k);
                }
                &[x, y] => triangle = if x == triangle { y } else { x },
                _ => return None,
            }
            prev = w;
        }
        None
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

    #[test]
    fn squares_touching_at_a_corner_yield_two_loops() {
        // Each square is two triangles, so a walk around either one passes through vertex 0.
        let mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::new(1.0, 0.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(0.0, 1.0, 0.0),
                DVec3::new(-1.0, 0.0, 0.0),
                DVec3::new(-1.0, -1.0, 0.0),
                DVec3::new(0.0, -1.0, 0.0),
            ],
            vec![[3, 0, 2], [0, 4, 5], [0, 5, 6], [0, 1, 2]],
        );
        assert_eq!(
            mesh.boundary_loops(),
            vec![vec![3, 0, 1, 2], vec![0, 4, 5, 6]]
        );
    }

    #[test]
    fn wide_fan_at_one_vertex_is_linear() {
        // 100 000 separate triangles sharing only vertex 0.
        let k = 100_000u32;
        let positions = (0..=2 * k)
            .map(|i| DVec3::new(i as f64, 1.0, 0.0))
            .collect();
        let triangles = (0..k).map(|i| [2 * i + 1, 0, 2 * i + 2]).collect();
        let mesh = TriMesh::new(positions, triangles);
        let start = std::time::Instant::now();
        let loops = mesh.boundary_loops();
        let elapsed = start.elapsed();
        assert_eq!(loops.len(), k as usize);
        assert!(loops.iter().all(|l| l.len() == 3));
        assert!(
            elapsed.as_secs_f64() < crate::tests::TIME_LIMIT_SECONDS,
            "took {elapsed:?}"
        );
    }
}
