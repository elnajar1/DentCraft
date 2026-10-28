//! Hole filling in Liepa's style ("Filling holes in meshes", Liepa 2003): minimum-weight
//! triangulation of each boundary loop, refinement of the patch to the surrounding edge
//! length, and umbrella fairing so the patch blends in.

use std::collections::HashMap;

use glam::DVec3;

use crate::topology::{directed_edges, edge_key};
use crate::{EdgeMap, TriMesh};

/// Loops with more vertices than this fall back from dynamic programming to ear clipping.
const DP_LIMIT: usize = 150;

/// Umbrella-operator passes applied to the interior of each patch.
const FAIRING_PASSES: usize = 20;

/// Refinement splits patch edges longer than this factor of the average rim edge length.
const EDGE_LENGTH_TOLERANCE: f64 = 4.0 / 3.0;

/// A safety cap so refinement cannot run away on pathological input.
const MAX_NEW_VERTICES: usize = 100_000;

/// What [`fill_holes`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FillReport {
    /// Boundary loops that were triangulated, refined and faired.
    pub holes_filled: usize,
    /// Triangles added, after refinement.
    pub triangles_added: usize,
    /// Boundary loops longer than `max_boundary_edges` (or otherwise unfillable), left
    /// untouched.
    pub holes_skipped: usize,
}

/// Fills every open boundary loop of at most `max_boundary_edges` edges (Liepa 2003):
/// minimum-weight triangulation by dynamic programming for loops up to 150 edges — weight
/// is the largest dihedral angle against the neighbouring triangles, with area as the
/// tie-breaker — and quality ear clipping above that. Each patch is then refined by edge
/// splits until its interior edges are close to the average rim edge length, and faired
/// with uniform umbrella passes that move only the new interior vertices. Rim vertices
/// never move, and rim edges are never split.
///
/// New triangles match the winding of the mesh around the hole. Loops above the limit, or
/// with no valid triangulation, are skipped and counted in the report.
pub fn fill_holes(mesh: &mut TriMesh, max_boundary_edges: usize) -> FillReport {
    let loops = mesh.boundary_loops();
    let edges = mesh.edge_map();
    let base = mesh.positions.len() as u32;
    let mut new_positions: Vec<DVec3> = Vec::new();
    let mut new_triangles: Vec<[u32; 3]> = Vec::new();
    let mut report = FillReport::default();
    for hole in loops {
        if hole.len() > max_boundary_edges {
            report.holes_skipped += 1;
            continue;
        }
        let filled = fill_one(
            mesh,
            &edges,
            &hole,
            base,
            &mut new_positions,
            &mut new_triangles,
        );
        if filled {
            report.holes_filled += 1;
        } else {
            report.holes_skipped += 1;
        }
    }
    report.triangles_added = new_triangles.len();
    mesh.positions.extend(new_positions);
    mesh.triangles.extend(new_triangles);
    report
}

/// Fills one loop into the staged buffers. `false` when the loop has no valid
/// triangulation.
fn fill_one(
    mesh: &TriMesh,
    edges: &EdgeMap,
    hole: &[u32],
    base: u32,
    new_positions: &mut Vec<DVec3>,
    new_triangles: &mut Vec<[u32; 3]>,
) -> bool {
    if hole.len() < 3 {
        return false;
    }
    let Some(rim_normals) = rim_normals(mesh, edges, hole) else {
        return false;
    };
    let mut triangles: Vec<[u32; 3]> = if hole.len() <= DP_LIMIT {
        triangulate_dp(mesh, hole, &rim_normals)
    } else {
        triangulate_ears(mesh, hole, &rim_normals)
    }
    .iter()
    .map(|[i, j, k]| [hole[*i], hole[*j], hole[*k]])
    .collect();
    if triangles.is_empty() {
        return false;
    }
    refine(mesh, hole, base, &mut triangles, new_positions);
    fair(mesh, &triangles, base, new_positions);
    new_triangles.extend(triangles);
    true
}

/// For each rim edge `hole[i] → hole[(i + 1) % n]`, the unit normal of the mesh triangle
/// beside it. `None` when an edge has no such triangle (the loop does not follow the mesh
/// winding, or a rim triangle is degenerate).
fn rim_normals(mesh: &TriMesh, edges: &EdgeMap, hole: &[u32]) -> Option<Vec<DVec3>> {
    let mut normals = Vec::with_capacity(hole.len());
    for step in 0..hole.len() {
        let (a, b) = (hole[step], hole[(step + 1) % hole.len()]);
        let triangle = *edges
            .get(&edge_key([a, b]))?
            .iter()
            .find(|&&t| directed_edges(mesh.triangles[t as usize]).contains(&[a, b]))?;
        let normal = mesh.triangle_cross(triangle as usize);
        let length = normal.length();
        if !length.is_finite() || length == 0.0 {
            return None;
        }
        normals.push(normal / length);
    }
    Some(normals)
}

/// The fill-winding normal and area of rim indices `i, j, k`; the winding `[i, j, k]` runs
/// against the loop order so rim edges keep their direction reversed. `None` when the
/// three points are collinear.
fn fill_triangle(
    mesh: &TriMesh,
    hole: &[u32],
    i: usize,
    j: usize,
    k: usize,
) -> Option<(DVec3, f64)> {
    let (a, b, c) = (
        mesh.positions[hole[i] as usize],
        mesh.positions[hole[j] as usize],
        mesh.positions[hole[k] as usize],
    );
    let cross = (b - a).cross(c - a);
    let length = cross.length();
    if !length.is_finite() || length == 0.0 {
        return None;
    }
    Some((cross / length, 0.5 * length))
}

/// If `{a, b}` is a rim edge of the loop, its index into the rim normals.
fn rim_index(a: usize, b: usize, n: usize) -> Option<usize> {
    if b == (a + 1) % n {
        Some(a)
    } else if a == (b + 1) % n {
        Some(b)
    } else {
        None
    }
}

/// The dihedral angle between two unit normals, in `[0, π]` (0 for a flat continuation).
fn fold(normal: DVec3, beside: DVec3) -> f64 {
    beside.dot(normal).clamp(-1.0, 1.0).acos()
}

/// Minimum-weight triangulation of the loop, `(largest dihedral, total area)`
/// lexicographic (Liepa 2003). Returns fill-wound index triples `[i, j, k]` with
/// `i < k < j`.
fn triangulate_dp(mesh: &TriMesh, hole: &[u32], rim_normals: &[DVec3]) -> Vec<[usize; 3]> {
    let n = hole.len();
    // best[i·n + j] is the weight of the optimal angulation of hole[i..=j] and
    // pick[i][j] its apex vertex k; span-1 edges have no cell of their own.
    let mut best = vec![(0.0f64, 0.0f64); n * n];
    let mut pick: Vec<Vec<usize>> = vec![vec![0; n]; n];
    for span in 2..n {
        for i in 0..n - span {
            let j = i + span;
            let mut cell = (f64::INFINITY, f64::INFINITY);
            let mut apex = usize::MAX;
            for k in i + 1..j {
                // Both sub-spans must have a valid angulation already.
                if k > i + 1 && pick[i][k] == 0 {
                    continue;
                }
                if j > k + 1 && pick[k][j] == 0 {
                    continue;
                }
                let Some((normal, area)) = fill_triangle(mesh, hole, i, j, k) else {
                    continue;
                };
                let mut candidate = (best[i * n + k].0, best[i * n + k].1 + area);
                for &(a, b, leaf) in &[(i, k, i), (k, j, k)] {
                    let beside = if b == a + 1 {
                        rim_normals[leaf]
                    } else {
                        let middle = pick[a][b];
                        fill_triangle(mesh, hole, a, b, middle)
                            .expect("picked cells are filled")
                            .0
                    };
                    candidate.0 = candidate.0.max(fold(normal, beside));
                }
                candidate.1 += best[k * n + j].1;
                if candidate < cell && candidate.0.is_finite() {
                    cell = candidate;
                    apex = k;
                }
            }
            if apex != usize::MAX {
                best[i * n + j] = cell;
                pick[i][j] = apex;
            }
        }
    }
    if pick[0][n - 1] == 0 {
        return Vec::new();
    }
    let mut triangles = Vec::new();
    let mut stack = vec![(0usize, n - 1usize)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let k = pick[i][j];
        if k <= i || k >= j {
            return Vec::new();
        }
        triangles.push([i, j, k]);
        stack.push((i, k));
        stack.push((k, j));
    }
    triangles
}

/// Quality ear clipping for long loops: repeatedly clip the convex ear with the smallest
/// `(rim dihedral, area)`. Returns fill-wound index triples.
fn triangulate_ears(mesh: &TriMesh, hole: &[u32], rim_normals: &[DVec3]) -> Vec<[usize; 3]> {
    let n = hole.len();
    // The polygon's orientation, by Newell's method, tells convex ears from reflex ones.
    let orientation: DVec3 = hole
        .iter()
        .zip(hole.iter().cycle().skip(1))
        .map(|(&a, &b)| mesh.positions[a as usize].cross(mesh.positions[b as usize]))
        .sum();
    let mut previous: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut next: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
    // Normals of the triangles clipped onto each cut edge.
    let mut cuts: HashMap<(usize, usize), DVec3> = HashMap::new();
    let mut triangles = Vec::new();
    let mut remaining = n;
    while remaining > 3 {
        let mut best = (f64::INFINITY, f64::INFINITY);
        let mut best_ear = usize::MAX;
        for i in 0..n {
            if next[i] == usize::MAX || previous[i] == usize::MAX || next[i] == i {
                continue;
            }
            let (a, c) = (previous[i], next[i]);
            let Some((normal, _)) = fill_triangle(mesh, hole, a, c, i) else {
                continue;
            };
            // A convex ear keeps the fill orientation, which runs against the loop's own
            // orientation (the Newell normal).
            if normal.dot(orientation) >= 0.0 {
                continue;
            }
            let mut candidate = (0.0f64, 0.0f64);
            for &(x, y) in &[(a, i), (i, c)] {
                let beside = match rim_index(x, y, n) {
                    Some(leaf) => rim_normals[leaf],
                    None => cuts.get(&(x.min(y), x.max(y))).copied().unwrap_or(normal),
                };
                candidate.0 = candidate.0.max(fold(normal, beside));
            }
            candidate.1 =
                mesh.positions[hole[a] as usize].distance_squared(mesh.positions[hole[c] as usize]);
            if candidate < best {
                best = candidate;
                best_ear = i;
            }
        }
        if best_ear == usize::MAX {
            return Vec::new();
        }
        let i = best_ear;
        let (a, c) = (previous[i], next[i]);
        triangles.push([a, c, i]);
        let normal = fill_triangle(mesh, hole, a, c, i)
            .expect("the chosen ear was checked")
            .0;
        cuts.insert((a.min(c), a.max(c)), normal);
        previous[c] = a;
        next[a] = c;
        next[i] = usize::MAX;
        previous[i] = usize::MAX;
        remaining -= 1;
    }
    // The three remaining vertices close the patch.
    let anchor = (0..n).find(|&i| next[i] != usize::MAX).unwrap_or(0);
    if next[anchor] == usize::MAX {
        return Vec::new();
    }
    let i = anchor;
    let (a, c) = (previous[i], next[i]);
    triangles.push([a, c, i]);
    triangles
}

/// Splits patch edges longer than [`EDGE_LENGTH_TOLERANCE`] times the average rim edge at
/// their midpoints until the patch matches the surrounding edge length (best effort, and
/// never more than [`MAX_NEW_VERTICES`] vertices). Rim edges are never split, so every new
/// vertex is strictly inside the patch.
fn refine(
    mesh: &TriMesh,
    hole: &[u32],
    base: u32,
    triangles: &mut Vec<[u32; 3]>,
    new_positions: &mut Vec<DVec3>,
) {
    let n = hole.len();
    let rim: std::collections::HashSet<(u32, u32)> = (0..n)
        .map(|i| {
            let (a, b) = (hole[i], hole[(i + 1) % n]);
            (a.min(b), a.max(b))
        })
        .collect();
    let average_rim = (0..n)
        .map(|i| {
            mesh.positions[hole[i] as usize].distance(mesh.positions[hole[(i + 1) % n] as usize])
        })
        .sum::<f64>()
        / n as f64;
    if !average_rim.is_finite() || average_rim <= 0.0 {
        return;
    }
    let target = average_rim * EDGE_LENGTH_TOLERANCE;
    let mut midpoints: HashMap<(u32, u32), u32> = HashMap::new();
    let edges_of = |t: &[u32; 3]| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
    loop {
        // Mark every interior patch edge longer than the target, then split all of them in
        // one rebuild, so the pass count stays logarithmic in the length ratio.
        let mut long_edges: Vec<(u32, u32)> = triangles
            .iter()
            .flat_map(edges_of)
            .map(|(a, b)| (a.min(b), a.max(b)))
            .filter(|&edge| {
                !rim.contains(&edge)
                    && staged(mesh, base, new_positions, edge.0).distance(staged(
                        mesh,
                        base,
                        new_positions,
                        edge.1,
                    )) > target
            })
            .collect();
        long_edges.sort_unstable();
        long_edges.dedup();
        if long_edges.is_empty() || new_positions.len() + long_edges.len() > MAX_NEW_VERTICES {
            return;
        }
        for edge in &long_edges {
            midpoints.entry(*edge).or_insert_with(|| {
                let m = (staged(mesh, base, new_positions, edge.0)
                    + staged(mesh, base, new_positions, edge.1))
                    * 0.5;
                new_positions.push(m);
                base + new_positions.len() as u32 - 1
            });
        }
        let mut split = Vec::with_capacity(triangles.len() + long_edges.len());
        for t in triangles.drain(..) {
            // m[k] is the midpoint of the edge from t[k] to t[(k + 1) % 3], if marked.
            let m = edges_of(&t).map(|(a, b)| {
                let edge = (a.min(b), a.max(b));
                (long_edges.binary_search(&edge).is_ok()).then(|| midpoints[&edge])
            });
            let (a, b, c) = (t[0], t[1], t[2]);
            match m {
                [None, None, None] => split.push(t),
                [Some(m0), None, None] => {
                    split.push([a, m0, c]);
                    split.push([m0, b, c]);
                }
                [None, Some(m1), None] => {
                    split.push([b, m1, a]);
                    split.push([m1, c, a]);
                }
                [None, None, Some(m2)] => {
                    split.push([c, m2, b]);
                    split.push([m2, a, b]);
                }
                [Some(m0), Some(m1), None] => {
                    split.push([a, m0, m1]);
                    split.push([a, m1, c]);
                    split.push([m0, b, m1]);
                }
                [None, Some(m1), Some(m2)] => {
                    split.push([b, m1, m2]);
                    split.push([b, m2, a]);
                    split.push([m1, c, m2]);
                }
                [Some(m0), None, Some(m2)] => {
                    split.push([c, m2, m0]);
                    split.push([c, m0, b]);
                    split.push([m2, a, m0]);
                }
                [Some(m0), Some(m1), Some(m2)] => {
                    split.push([a, m0, m2]);
                    split.push([m0, b, m1]);
                    split.push([m1, c, m2]);
                    split.push([m0, m1, m2]);
                }
            }
        }
        *triangles = split;
    }
}

/// The position of a vertex, whether it belongs to the original mesh or the staged patch.
fn staged(mesh: &TriMesh, base: u32, new_positions: &[DVec3], v: u32) -> DVec3 {
    if v < base {
        mesh.positions[v as usize]
    } else {
        new_positions[(v - base) as usize]
    }
}

/// Umbrella fairing (uniform Laplacian, Jacobi order) of the patch's interior vertices —
/// the ones the refinement created. Rim vertices stay fixed.
fn fair(mesh: &TriMesh, triangles: &[[u32; 3]], base: u32, new_positions: &mut [DVec3]) {
    let mut neighbors: HashMap<u32, Vec<u32>> = HashMap::new();
    for t in triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            neighbors.entry(a).or_default().push(b);
            neighbors.entry(b).or_default().push(a);
        }
    }
    let mut current = new_positions.to_vec();
    for _ in 0..FAIRING_PASSES {
        let mut updated = current.clone();
        for (&v, ring) in &neighbors {
            if v < base || ring.is_empty() {
                continue;
            }
            let sum = ring
                .iter()
                .map(|&u| staged(mesh, base, &current, u))
                .fold(DVec3::ZERO, |sum, p| sum + p);
            updated[(v - base) as usize] = sum / ring.len() as f64;
        }
        current = updated;
    }
    new_positions.copy_from_slice(&current);
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

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
                let (a, b, d, e) = (at(r, c), at(r, c + 1), at(r + 1, c + 1), at(r + 1, c));
                triangles.push([a, b, d]);
                triangles.push([a, d, e]);
            }
        }
        TriMesh::new(positions, triangles)
    }

    #[test]
    fn empty_mesh_is_a_no_op() {
        let mut mesh = TriMesh::default();
        assert_eq!(fill_holes(&mut mesh, 16), FillReport::default());
        assert_eq!(mesh, TriMesh::default());
    }

    #[test]
    fn sphere_cap_is_filled_closed_and_oriented() {
        let mut mesh = TriMesh::uv_sphere(2.0, 32, 16);
        mesh.triangles.retain(|t| !t.contains(&0));
        assert!(!mesh.is_closed());
        let vertices_before = mesh.vertex_count();
        let report = fill_holes(&mut mesh, 64);
        assert_eq!((report.holes_filled, report.holes_skipped), (1, 0));
        assert!(report.triangles_added > 0);
        assert!(mesh.is_closed());
        assert!(mesh.is_consistently_oriented());
        // The rim is planar, so the patch stays in its plane: every new vertex lies within
        // 5% of the radius.
        for p in &mesh.positions[vertices_before..] {
            assert!((p.length() - 2.0).abs() < 0.1, "{p}");
        }
    }

    #[test]
    fn planar_square_hole_fills_flat() {
        let mut mesh = grid(8, 8, 1.0);
        // Drop the 2×2 quads in the middle; their rim is an 8-edge rectangle while the
        // grid rim runs 28 edges.
        mesh.triangles.retain(|t| {
            !t.iter().all(|&v| {
                let (r, c) = (v / 8, v % 8);
                (3..=5).contains(&r) && (3..=5).contains(&c)
            })
        });
        let triangles_before = mesh.triangle_count();
        let report = fill_holes(&mut mesh, 15);
        assert_eq!((report.holes_filled, report.holes_skipped), (1, 1));
        assert_eq!(
            mesh.triangle_count(),
            triangles_before + report.triangles_added
        );
        // Everything new is exactly on the plane.
        let old_vertices = 8 * 8;
        for p in &mesh.positions[old_vertices..] {
            assert!(p.z.abs() < 1e-12, "{p}");
        }
        // Refinement split patch diagonals, so interior vertices exist.
        assert!(mesh.vertex_count() > old_vertices);
        // Only the outer rim is still open, and the patch blends in.
        let loops = mesh.boundary_loops();
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 28);
        assert!(mesh.is_consistently_oriented());
    }

    #[test]
    fn too_big_holes_are_skipped_and_reported() {
        let mut mesh = TriMesh::uv_sphere(2.0, 32, 16);
        mesh.triangles.retain(|t| !t.contains(&0));
        let before = mesh.clone();
        let report = fill_holes(&mut mesh, 10);
        assert_eq!(
            (
                report.holes_filled,
                report.triangles_added,
                report.holes_skipped
            ),
            (0, 0, 1)
        );
        assert_eq!(mesh, before);
    }

    #[test]
    fn open_grid_rim_is_not_filled() {
        let mut mesh = grid(6, 6, 1.0);
        assert_eq!(mesh.boundary_loops().len(), 1);
        assert_eq!(mesh.boundary_loops()[0].len(), 20);
        let before = mesh.clone();
        let report = fill_holes(&mut mesh, 15);
        assert_eq!((report.holes_filled, report.holes_skipped), (0, 1));
        assert_eq!(mesh, before);
        // A limit above the rim length does close the grid: filling is opt-in by size.
        let report = fill_holes(&mut mesh, 21);
        assert_eq!(report.holes_filled, 1);
        assert!(mesh.is_closed());
        assert!(mesh.is_consistently_oriented());
    }

    #[test]
    fn triangular_hole_fills_exactly() {
        let mut mesh = crate::tests::unit_tetrahedron();
        mesh.triangles.pop();
        assert!(!mesh.is_closed());
        let report = fill_holes(&mut mesh, 8);
        assert_eq!(
            report,
            FillReport {
                holes_filled: 1,
                triangles_added: 1,
                holes_skipped: 0,
            }
        );
        assert!(mesh.is_closed());
        assert!(mesh.is_consistently_oriented());
        // The rim is a planar triangle, so the patch is exactly the missing face.
        assert!((mesh.signed_volume() - 1.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn ear_clipping_fills_a_large_rim() {
        let mut mesh = TriMesh::uv_sphere(2.0, 256, 4);
        mesh.triangles.retain(|t| !t.contains(&0));
        assert!(!mesh.is_closed());
        let report = fill_holes(&mut mesh, 300);
        assert_eq!(report.holes_filled, 1);
        assert!(mesh.is_closed());
        assert!(mesh.is_consistently_oriented());
    }
}
