//! Paths along mesh edges: an A* search with a straight-line heuristic, waypoint chaining,
//! ridge costs for "intelligent scissors" margin lines, and polyline smoothing on the
//! surface.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use glam::DVec3;

use crate::{Bvh, PrincipalCurvature, TriMesh, VertexAdjacency};

/// Vertex costs are clamped to at least this so every edge weight stays positive.
pub const MIN_COST: f64 = 1e-6;

/// A path along mesh edges.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfacePath {
    /// Vertex indices; consecutive ones are joined by an edge. A closed path repeats its
    /// first vertex at the end.
    pub vertices: Vec<u32>,
    /// The sum of the edge weights `|e| · (cost(a) + cost(b)) / 2`, which is the geometric
    /// length when every cost is 1.
    pub length: f64,
}

/// Clamps a cost to the documented positive minimum; non-finite costs count as the minimum.
fn clamp_cost(cost: f64) -> f64 {
    if cost.is_finite() && cost > MIN_COST {
        cost
    } else {
        MIN_COST
    }
}

/// `f64` bits under the total order of `total_cmp`, so priorities can sit in a heap.
fn ordered_bits(x: f64) -> u64 {
    let bits = x.to_bits();
    if bits & (1 << 63) == 0 {
        bits + (1 << 63)
    } else {
        !bits
    }
}

/// A min-heap entry; ordering goes through [`ordered_bits`].
#[derive(PartialEq, Eq)]
struct Entry(u64, u32);

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0).then(self.1.cmp(&other.1))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The cheapest path from `from` to `to` along mesh edges, by A* with a straight-line
/// heuristic scaled by the cheapest vertex cost (plain Dijkstra when costs are equal). The
/// weight of an edge is its length times the average of its endpoints' costs, and each cost
/// is clamped to at least [`MIN_COST`] so weights stay positive.
///
/// The search stops as soon as `to` is settled. Returns `None` for an out-of-range
/// endpoint, or when the two vertices lie in different components or non-finite geometry
/// blocks every route. `from == to` gives a single-vertex path of length 0.
pub fn shortest_path(
    mesh: &TriMesh,
    adjacency: &VertexAdjacency,
    from: u32,
    to: u32,
    cost: impl Fn(u32) -> f64,
) -> Option<SurfacePath> {
    let n = mesh.positions.len();
    if from as usize >= n || to as usize >= n {
        return None;
    }
    if from == to {
        return Some(SurfacePath {
            vertices: vec![from],
            length: 0.0,
        });
    }
    let costs: Vec<f64> = (0..n as u32).map(|v| clamp_cost(cost(v))).collect();
    let min_cost = costs.iter().copied().fold(f64::INFINITY, f64::min);
    let target = mesh.positions[to as usize];
    let heuristic = |v: u32| mesh.positions[v as usize].distance(target) * min_cost;
    let mut dist = vec![f64::INFINITY; n];
    let mut prev = vec![u32::MAX; n];
    let mut settled = vec![false; n];
    dist[from as usize] = 0.0;
    let mut heap = BinaryHeap::from([Reverse(Entry(ordered_bits(heuristic(from)), from))]);
    while let Some(Reverse(Entry(_, v))) = heap.pop() {
        if settled[v as usize] {
            continue;
        }
        settled[v as usize] = true;
        if v == to {
            let mut vertices = vec![to];
            let mut walk = to;
            while prev[walk as usize] != u32::MAX {
                walk = prev[walk as usize];
                vertices.push(walk);
            }
            vertices.reverse();
            return Some(SurfacePath {
                vertices,
                length: dist[to as usize],
            });
        }
        for &u in adjacency.neighbors(v) {
            let edge = mesh.positions[u as usize] - mesh.positions[v as usize];
            let length = edge.length();
            if !length.is_finite() {
                continue;
            }
            let weight = length * (costs[v as usize] + costs[u as usize]) * 0.5;
            let candidate = dist[v as usize] + weight;
            if candidate < dist[u as usize] {
                dist[u as usize] = candidate;
                prev[u as usize] = v;
                heap.push(Reverse(Entry(ordered_bits(candidate + heuristic(u)), u)));
            }
        }
    }
    None
}

/// The cheapest path through `waypoints` in order, by chaining [`shortest_path`] segments.
/// Joint vertices are not repeated. With `closed` the path also runs from the last waypoint
/// back to the first, and the returned vertex list ends on the first waypoint again.
/// Returns `None` when any leg is unreachable (or the waypoint list is empty).
pub fn path_through(
    mesh: &TriMesh,
    adjacency: &VertexAdjacency,
    waypoints: &[u32],
    closed: bool,
    cost: impl Fn(u32) -> f64,
) -> Option<SurfacePath> {
    if waypoints.is_empty() {
        return None;
    }
    let legs = waypoints.len() - usize::from(!closed);
    let mut vertices = vec![waypoints[0]];
    let mut length = 0.0;
    for i in 0..legs {
        let (a, b) = (waypoints[i], waypoints[(i + 1) % waypoints.len()]);
        let leg = shortest_path(mesh, adjacency, a, b, &cost)?;
        vertices.pop();
        vertices.extend_from_slice(&leg.vertices);
        length += leg.length;
    }
    Some(SurfacePath { vertices, length })
}

/// A per-vertex path cost that is low on convex ridges and high on flat or concave areas:
/// `exp(-sharpness · max(k1, 0))`, clamped to at least [`MIN_COST`], so a search steered by
/// it follows the sharpest convex ridge between waypoints. A `sharpness` below zero acts as
/// zero, which makes every cost 1. Feed the result to [`shortest_path`] as its `cost`.
pub fn ridge_cost(curvature: &[PrincipalCurvature], sharpness: f64) -> Vec<f64> {
    let sharpness = sharpness.max(0.0);
    curvature
        .iter()
        .map(|pc| clamp_cost((-sharpness * pc.k1.max(0.0)).exp()))
        .collect()
}

/// Smooths a closed polyline on the surface: each of `iterations` passes moves every point
/// half of the way to the midpoint of its two neighbours (uniform Laplacian, Jacobi order)
/// and projects it back onto the mesh through `bvh`, keeping the point count. This turns a
/// zig-zag vertex path into a clean margin curve.
///
/// Fewer than 3 points or zero iterations return the points unchanged; a point the `bvh`
/// cannot place (no triangles) stays as it was.
pub fn smooth_polyline_on_surface(
    mesh: &TriMesh,
    bvh: &Bvh,
    points: &[DVec3],
    iterations: u32,
) -> Vec<DVec3> {
    let mut current = points.to_vec();
    if current.len() < 3 || iterations == 0 {
        return current;
    }
    let mut next = vec![DVec3::ZERO; current.len()];
    for _ in 0..iterations {
        let n = current.len();
        for (i, point) in current.iter().enumerate() {
            let previous = current[(i + n - 1) % n];
            let following = current[(i + 1) % n];
            next[i] = (point * 2.0 + previous + following) * 0.25;
        }
        for point in &mut next {
            if let Some(hit) = bvh.closest_point(mesh, *point) {
                *point = hit.point;
            }
        }
        std::mem::swap(&mut current, &mut next);
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::SQRT_2;

    /// Flat grid in the XY plane wound so the normals point at +Z; quads carry the
    /// diagonal from `(r, c)` to `(r + 1, c + 1)`.
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

    /// An open cylinder along Z with a flat fan cap at the top, giving a sharp circular
    /// rim edge at `z = height / 2`.
    fn capped_cylinder(radius: f64, height: f64, segments: u32, rows: u32) -> TriMesh {
        let positions: Vec<DVec3> = (0..rows)
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
            .chain([DVec3::new(0.0, 0.0, height / 2.0)])
            .collect();
        let at = |r: u32, s: u32| r * segments + s % segments;
        let mut triangles = Vec::with_capacity((segments * (rows - 1) * 2 + segments) as usize);
        for r in 0..rows - 1 {
            for s in 0..segments {
                let (a, b) = (at(r, s), at(r, s + 1));
                let (c, d) = (at(r + 1, s), at(r + 1, s + 1));
                triangles.push([a, b, d]);
                triangles.push([a, d, c]);
            }
        }
        let top = (positions.len() - 1) as u32;
        for s in 0..segments {
            triangles.push([at(rows - 1, s), at(rows - 1, s + 1), top]);
        }
        TriMesh::new(positions, triangles)
    }

    fn unit(_: u32) -> f64 {
        1.0
    }

    /// Every consecutive pair of path vertices must be one-ring neighbours.
    fn assert_walks_edges(mesh: &TriMesh, path: &SurfacePath) {
        let adjacency = mesh.vertex_adjacency();
        for pair in path.vertices.windows(2) {
            assert!(
                adjacency.neighbors(pair[0]).contains(&pair[1]),
                "{pair:?} are not neighbours"
            );
        }
    }

    #[test]
    fn grid_path_matches_the_grid_distance() {
        let mesh = grid(9, 9, 1.0);
        let adjacency = mesh.vertex_adjacency();
        let diagonal = shortest_path(&mesh, &adjacency, 0, 8 * 9 + 8, unit).unwrap();
        // The grid diagonals run corner to corner, so this path is the straight diagonal.
        assert_eq!(diagonal.vertices.len(), 9);
        assert!((diagonal.length - 8.0 * SQRT_2).abs() < 1e-12);
        assert_walks_edges(&mesh, &diagonal);

        let offset = shortest_path(&mesh, &adjacency, 0, 8 * 9 + 3, unit).unwrap();
        // Three diagonal steps and five vertical ones are the cheapest the grid allows.
        assert!(
            (offset.length - (3.0 * SQRT_2 + 5.0)).abs() < 1e-12,
            "{}",
            offset.length
        );
        assert_walks_edges(&mesh, &offset);
        // No surface path can beat the straight line through the mesh.
        let straight = mesh.positions[0].distance(mesh.positions[8 * 9 + 3]);
        assert!(offset.length >= straight - 1e-12);
    }

    #[test]
    fn ridge_cost_is_low_on_convex_ridges_only() {
        let concave = PrincipalCurvature {
            k1: -4.0,
            k2: -5.0,
            dir1: DVec3::X,
            dir2: DVec3::Y,
        };
        let flat = PrincipalCurvature {
            k1: 0.0,
            k2: 0.0,
            dir1: DVec3::X,
            dir2: DVec3::Y,
        };
        let ridge = PrincipalCurvature {
            k1: 3.0,
            k2: 0.0,
            dir1: DVec3::X,
            dir2: DVec3::Y,
        };
        let curvature = [concave, flat, ridge];
        assert_eq!(ridge_cost(&curvature, 1.0), [1.0, 1.0, (-3.0f64).exp()]);
        // Only k1 earns the discount; k2 stays ignored.
        assert_eq!(ridge_cost(&[concave], 10.0), [1.0]);
        assert_eq!(ridge_cost(&curvature, 0.0), [1.0, 1.0, 1.0]);
        assert!(ridge_cost(&[], 5.0).is_empty());
        // Sharpness that underflows exp still leaves a positive floor.
        assert_eq!(ridge_cost(&[ridge], 10.0), [MIN_COST]);
        assert_eq!(ridge_cost(&[ridge], 1e9), [MIN_COST]);
    }

    #[test]
    fn ridge_search_stays_on_the_sharp_rim() {
        let (segments, rows) = (48u32, 5u32);
        let mesh = capped_cylinder(2.0, 2.0, segments, rows);
        assert!(mesh.is_consistently_oriented());
        let adjacency = mesh.vertex_adjacency();
        let curvature = mesh.principal_curvatures();
        let cost = ridge_cost(&curvature, 20.0);
        let rim = |s: u32| (rows - 1) * segments + s % segments;
        // Half way around the rim.
        let path = shortest_path(&mesh, &adjacency, rim(0), rim(segments / 2), |v| {
            cost[v as usize]
        })
        .expect("rim vertices are connected");
        assert!(path.length < 1.0, "weighted length {}", path.length);
        assert!(
            path.vertices
                .iter()
                .all(|v| ((rows - 1) * segments..rows * segments).contains(v))
        );
    }

    #[test]
    fn unreachable_targets_return_none() {
        let mut mesh = crate::tests::unit_tetrahedron();
        let mut far = crate::tests::unit_tetrahedron();
        for p in &mut far.positions {
            *p += DVec3::splat(3.0);
        }
        mesh.merge(&far);
        let adjacency = mesh.vertex_adjacency();
        assert!(shortest_path(&mesh, &adjacency, 0, 5, unit).is_none());
        // Out-of-range endpoints, and an empty mesh.
        assert!(shortest_path(&mesh, &adjacency, 0, 99, unit).is_none());
        assert!(shortest_path(&mesh, &adjacency, 99, 0, unit).is_none());
        let empty = TriMesh::default();
        assert!(shortest_path(&empty, &empty.vertex_adjacency(), 0, 0, unit).is_none());
    }

    #[test]
    fn from_equals_to_is_a_single_vertex() {
        let mesh = grid(4, 4, 1.0);
        let adjacency = mesh.vertex_adjacency();
        let path = shortest_path(&mesh, &adjacency, 5, 5, unit).unwrap();
        assert_eq!(
            path,
            SurfacePath {
                vertices: vec![5],
                length: 0.0,
            }
        );
    }

    #[test]
    fn zero_and_non_finite_costs_are_clamped() {
        let mesh = grid(5, 5, 1.0);
        let adjacency = mesh.vertex_adjacency();
        let path = shortest_path(&mesh, &adjacency, 0, 24, |v| {
            if v == 3 {
                0.0
            } else if v == 7 {
                f64::NAN
            } else {
                1.0
            }
        })
        .unwrap();
        // Zero and NaN costs clamp to the minimum instead of breaking the search, and the
        // discounted vertex pulls the route off the pure diagonal.
        assert_eq!((path.vertices[0], *path.vertices.last().unwrap()), (0, 24));
        assert_walks_edges(&mesh, &path);
        assert!(
            path.length < 4.0 * SQRT_2,
            "weighted length {}",
            path.length
        );
    }

    #[test]
    fn open_path_through_joins_without_repeats() {
        let mesh = grid(6, 6, 1.0);
        let adjacency = mesh.vertex_adjacency();
        let at = |r: u32, c: u32| r * 6 + c;
        let path = path_through(
            &mesh,
            &adjacency,
            &[at(0, 0), at(5, 0), at(5, 5)],
            false,
            unit,
        )
        .unwrap();
        assert_eq!(*path.vertices.first().unwrap(), at(0, 0));
        assert_eq!(*path.vertices.last().unwrap(), at(5, 5));
        assert_eq!(path.vertices.iter().filter(|&&v| v == at(5, 0)).count(), 1);
        assert_walks_edges(&mesh, &path);
        assert!((path.length - 10.0).abs() < 1e-12);
    }

    #[test]
    fn closed_path_through_returns_to_the_first_vertex() {
        let mesh = grid(6, 6, 1.0);
        let adjacency = mesh.vertex_adjacency();
        let at = |r: u32, c: u32| r * 6 + c;
        let waypoints = [at(0, 0), at(5, 0), at(5, 5)];
        let path = path_through(&mesh, &adjacency, &waypoints, true, unit).unwrap();
        assert_eq!(path.vertices.first(), path.vertices.last());
        assert_eq!(*path.vertices.first().unwrap(), at(0, 0));
        assert_walks_edges(&mesh, &path);
        assert!((path.length - (10.0 + 5.0 * SQRT_2)).abs() < 1e-12);
        // A single open waypoint is a point; unreachable legs and empty lists give None.
        let point = path_through(&mesh, &adjacency, &[at(2, 2)], false, unit).unwrap();
        assert_eq!(point.vertices, vec![at(2, 2)]);
        let mut split = mesh.clone();
        split.triangles.clear();
        assert!(path_through(&split, &split.vertex_adjacency(), &[0, 1], false, unit).is_none());
        assert!(path_through(&mesh, &adjacency, &[], false, unit).is_none());
    }

    /// Total length of a closed polyline.
    fn polyline_length(points: &[DVec3]) -> f64 {
        points.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>()
            + points[points.len() - 1].distance(points[0])
    }

    #[test]
    fn smoothing_shortens_a_zig_zag_on_the_plane() {
        let mesh = grid(20, 20, 1.0);
        let bvh = Bvh::build(&mesh);
        // A closed star: points alternate between circles of radius 2 and 4.
        let points: Vec<DVec3> = (0..16)
            .map(|i| {
                let angle = std::f64::consts::TAU * i as f64 / 16.0;
                let radius = if i % 2 == 0 { 4.0 } else { 2.0 };
                DVec3::new(
                    10.0 + radius * angle.cos(),
                    10.0 + radius * angle.sin(),
                    0.0,
                )
            })
            .collect();
        let before = polyline_length(&points);
        let smoothed = smooth_polyline_on_surface(&mesh, &bvh, &points, 10);
        assert_eq!(smoothed.len(), points.len());
        let after = polyline_length(&smoothed);
        assert!(after < before, "length {before} -> {after}");
        assert!(smoothed.iter().all(|p| p.z.abs() < 1e-9));
    }

    #[test]
    fn smoothing_keeps_tiny_and_lazy_inputs_unchanged() {
        let mesh = grid(5, 5, 1.0);
        let bvh = Bvh::build(&mesh);
        let two = [DVec3::new(1.0, 1.0, 0.0), DVec3::new(2.0, 2.0, 0.0)];
        assert_eq!(smooth_polyline_on_surface(&mesh, &bvh, &two, 5), two);
        let triangle = [
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(2.0, 1.0, 0.0),
            DVec3::new(1.5, 2.0, 0.0),
        ];
        assert_eq!(
            smooth_polyline_on_surface(&mesh, &bvh, &triangle, 0),
            triangle
        );
        assert!(smooth_polyline_on_surface(&mesh, &bvh, &[], 4).is_empty());
        // Without triangles to project onto, the pass still smooths and keeps the count.
        let empty = TriMesh::default();
        let drifted = smooth_polyline_on_surface(&empty, &Bvh::build(&empty), &triangle, 3);
        assert_eq!(drifted.len(), triangle.len());
        assert_ne!(drifted, triangle);
    }

    #[test]
    #[ignore]
    fn perf_path_1m() {
        let mesh = grid(501, 1001, 1.0);
        assert_eq!(mesh.triangle_count(), 1_000_000);
        let adjacency = mesh.vertex_adjacency();
        let start = std::time::Instant::now();
        let path = shortest_path(&mesh, &adjacency, 0, 501 * 1001 - 1, unit).unwrap();
        let elapsed = start.elapsed();
        // 500 diagonal steps and 500 vertical ones: the cheapest route across this grid.
        assert!((path.length - 500.0 * (SQRT_2 + 1.0)).abs() < 1e-6);
        println!("shortest_path over 1M triangles: {elapsed:?}");
        assert!(elapsed.as_secs_f64() < 1.0, "took {elapsed:?}");
    }
}
