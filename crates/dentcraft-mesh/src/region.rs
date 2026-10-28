//! Regions on the surface: flood fill over vertices, triangle selection, and the area
//! enclosed by a closed path — the prep region inside a margin line.

use std::collections::VecDeque;

use crate::{TriMesh, VertexAdjacency};

/// Flood fill over mesh edges from `seeds`: vertex `to` joins the region when it is a
/// neighbour of an inside vertex `from` and `accept(from, to)` allows the step. Returns one
/// flag per mesh vertex; out-of-range seeds are ignored, isolated vertices never join.
pub fn grow_region(
    mesh: &TriMesh,
    adjacency: &VertexAdjacency,
    seeds: &[u32],
    accept: impl Fn(u32, u32) -> bool,
) -> Vec<bool> {
    let n = mesh.positions.len();
    let mut inside = vec![false; n];
    let mut queue = VecDeque::new();
    for &seed in seeds {
        let i = seed as usize;
        if i < n && !inside[i] {
            inside[i] = true;
            queue.push_back(seed);
        }
    }
    while let Some(v) = queue.pop_front() {
        for &u in adjacency.neighbors(v) {
            let i = u as usize;
            if !inside[i] && accept(v, u) {
                inside[i] = true;
                queue.push_back(u);
            }
        }
    }
    inside
}

/// The triangles whose three corners are marked inside, ascending in mesh order. A mask
/// shorter than the vertex list counts its missing entries as outside.
pub fn region_triangles(mesh: &TriMesh, inside: &[bool]) -> Vec<u32> {
    mesh.triangles
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.iter()
                .all(|v| inside.get(*v as usize).copied().unwrap_or(false))
        })
        .map(|(i, _)| i as u32)
        .collect()
}

/// The vertices reachable from `seed` without crossing the closed vertex `path`: the prep
/// region inside a margin line. The path acts as a wall that cannot be stepped over or
/// through, and its own vertices count as inside.
///
/// Returns `None` when `seed` is out of range or the path fails to separate the mesh —
/// every vertex is then reachable from the seed, which happens for open or self-crossing
/// paths. An empty mask comes back for an empty mesh only in that same unseparated case,
/// which a mesh without vertices always is.
pub fn inside_closed_path(
    mesh: &TriMesh,
    adjacency: &VertexAdjacency,
    path: &[u32],
    seed: u32,
) -> Option<Vec<bool>> {
    let n = mesh.positions.len();
    if seed as usize >= n {
        return None;
    }
    let mut barrier = vec![false; n];
    for &v in path {
        if (v as usize) < n {
            barrier[v as usize] = true;
        }
    }
    let mut inside = vec![false; n];
    let mut queue = VecDeque::new();
    inside[seed as usize] = true;
    queue.push_back(seed);
    while let Some(v) = queue.pop_front() {
        for &u in adjacency.neighbors(v) {
            let i = u as usize;
            if !inside[i] && !barrier[i] {
                inside[i] = true;
                queue.push_back(u);
            }
        }
    }
    for (inside, on_path) in inside.iter_mut().zip(&barrier) {
        if *on_path {
            *inside = true;
        }
    }
    // The path separates the mesh only when something is left outside it.
    match inside.iter().any(|&inside| !inside) {
        true => Some(inside),
        false => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::unit_tetrahedron;
    use glam::DVec3;

    #[test]
    fn empty_mesh_has_no_regions() {
        let mesh = TriMesh::default();
        let adjacency = mesh.vertex_adjacency();
        assert!(grow_region(&mesh, &adjacency, &[0], |_, _| true).is_empty());
        assert!(grow_region(&mesh, &adjacency, &[], |_, _| true).is_empty());
        assert!(region_triangles(&mesh, &[]).is_empty());
        assert!(inside_closed_path(&mesh, &adjacency, &[0, 1, 2], 0).is_none());
    }

    #[test]
    fn flood_fills_stop_at_components_and_accept() {
        let mut mesh = unit_tetrahedron();
        let mut far = unit_tetrahedron();
        for p in &mut far.positions {
            *p += DVec3::splat(3.0);
        }
        mesh.merge(&far);
        let adjacency = mesh.vertex_adjacency();
        // Without an accept gate only the seed's own component fills.
        let inside = grow_region(&mesh, &adjacency, &[0], |_, _| true);
        assert_eq!(inside.iter().filter(|&&v| v).count(), 4);
        assert!(inside[0] && !inside[4]);
        // Two seeds in different components fill both.
        let inside = grow_region(&mesh, &adjacency, &[0, 4], |_, _| true);
        assert_eq!(inside.iter().filter(|&&v| v).count(), 8);
        // The gate blocks one vertex: it stays out, everything beside it still fills.
        let inside = grow_region(&mesh, &adjacency, &[1], |_from, to| to != 3);
        assert!(inside[0] && inside[1] && inside[2] && !inside[3]);
        // Out-of-range seeds are ignored; an empty seed list fills nothing.
        assert!(
            grow_region(&mesh, &adjacency, &[99], |_, _| true)
                .iter()
                .all(|v| !*v)
        );
    }

    #[test]
    fn region_triangles_need_all_corners_inside() {
        let mesh = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::new(1.0, 1.0, 0.0)],
            vec![[0, 1, 2], [2, 1, 3]],
        );
        assert_eq!(region_triangles(&mesh, &[true, true, true, false]), vec![0]);
        assert_eq!(region_triangles(&mesh, &[true; 4]), vec![0, 1]);
        assert!(region_triangles(&mesh, &[true, true]).is_empty());
    }

    /// UV sphere with `segments` around and `rings` pole to pole; `ring(r, s)` is the
    /// vertex of ring `r` (1-based below the north pole) at segment `s`.
    fn sphere_with_equator(segments: u32, rings: u32) -> (TriMesh, VertexAdjacency, Vec<u32>) {
        let mesh = TriMesh::uv_sphere(2.0, segments, rings);
        let adjacency = mesh.vertex_adjacency();
        let equator = rings / 2;
        let ring = |r: u32, s: u32| 1 + (r - 1) * segments + s % segments;
        let path: Vec<u32> = (0..segments).map(|s| ring(equator, s)).collect();
        (mesh, adjacency, path)
    }

    #[test]
    fn equator_path_keeps_one_hemisphere_inside() {
        let (segments, rings) = (32u32, 16u32);
        let (mesh, adjacency, path) = sphere_with_equator(segments, rings);
        let north = inside_closed_path(&mesh, &adjacency, &path, 0).expect("equator separates");
        // North pole, rings 1..=8 (ring 8 is the path itself): 1 + 8 · 32 vertices.
        assert_eq!(
            north.iter().filter(|&&v| v).count(),
            1 + 8 * segments as usize
        );
        assert!(north[0]);
        assert!(!north[mesh.vertex_count() - 1]);
        // Path vertices count as inside whichever side is seeded.
        assert!(path.iter().all(|v| north[*v as usize]));
        // Seeding at the south pole flips the hemisphere.
        let south = inside_closed_path(&mesh, &adjacency, &path, mesh.vertex_count() as u32 - 1)
            .expect("equator separates");
        assert_eq!(
            south.iter().filter(|&&v| v).count(),
            1 + 8 * segments as usize
        );
        assert!(!south[0]);
        // The two hemispheres overlap on the path vertices only.
        let path_set: std::collections::HashSet<u32> = path.iter().copied().collect();
        for (i, (north, south)) in north.iter().zip(&south).enumerate() {
            assert!(
                !(*north && *south) || path_set.contains(&(i as u32)),
                "vertex {i}"
            );
        }
    }

    #[test]
    fn open_paths_do_not_separate() {
        let (segments, _rings) = (32u32, 16u32);
        let (mesh, adjacency, mut path) = sphere_with_equator(segments, 16);
        // Half of the equator is not a wall.
        path.truncate(segments as usize / 2);
        assert!(inside_closed_path(&mesh, &adjacency, &path, 0).is_none());
        // Neither is the empty path.
        assert!(inside_closed_path(&mesh, &adjacency, &[], 0).is_none());
        // A seed on the path itself floods around it: nothing is left outside.
        assert!(inside_closed_path(&mesh, &adjacency, &path, path[0]).is_none());
    }

    #[test]
    fn out_of_range_seed_is_rejected() {
        let (mesh, adjacency, path) = sphere_with_equator(32, 16);
        assert!(inside_closed_path(&mesh, &adjacency, &path, 9_999).is_none());
        // Out-of-range path vertices are simply not walls.
        let mut sparse = path.clone();
        sparse.push(9_999);
        assert!(inside_closed_path(&mesh, &adjacency, &sparse, 0).is_some());
    }
}
