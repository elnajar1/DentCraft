//! Bounding volume hierarchy over a mesh's triangles, for ray casts and closest-point queries.

use glam::DVec3;

use crate::{Aabb, TriMesh};

const LEAF_SIZE: usize = 4;

/// Where a ray first meets the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub triangle: u32,
    /// Distance along the ray in multiples of its direction vector.
    pub t: f64,
    pub point: DVec3,
    /// Weights of the triangle's three corners at `point`.
    pub barycentric: DVec3,
}

/// The point of the mesh nearest to a query point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClosestPoint {
    pub triangle: u32,
    pub point: DVec3,
    pub distance: f64,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Aabb,
    /// Leaf: first slot in `order`. Interior: index of the left child; the right one follows.
    first: u32,
    /// Triangles in a leaf; 0 for an interior node.
    count: u32,
}

/// Axis-aligned box tree over the triangles of one mesh. Queries must pass that same mesh,
/// unmodified; rebuild after editing it.
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

impl Bvh {
    /// Builds the tree, splitting each node at the median triangle centre along its longest axis.
    pub fn build(mesh: &TriMesh) -> Self {
        let n = mesh.triangle_count();
        if n == 0 {
            return Self::default();
        }
        let boxes: Vec<Aabb> = (0..n).map(|i| triangle_bounds(mesh.triangle(i))).collect();
        let mut order: Vec<u32> = (0..n as u32).collect();
        let empty = Node {
            bounds: boxes[0],
            first: 0,
            count: 0,
        };
        let mut nodes = vec![empty];
        let mut stack = vec![(0usize, 0usize, n)];
        while let Some((node, start, end)) = stack.pop() {
            let range = &mut order[start..end];
            nodes[node].bounds = range
                .iter()
                .map(|&t| boxes[t as usize])
                .reduce(union)
                .expect("node ranges are never empty");
            if range.len() <= LEAF_SIZE {
                nodes[node].first = start as u32;
                nodes[node].count = range.len() as u32;
                continue;
            }
            let (lo, hi) =
                range
                    .iter()
                    .fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(lo, hi), &t| {
                        let c = boxes[t as usize].center();
                        (lo.min(c), hi.max(c))
                    });
            let axis = (hi - lo).max_position();
            let mid = range.len() / 2;
            range.select_nth_unstable_by(mid, |&a, &b| {
                let ca = boxes[a as usize].center()[axis];
                ca.total_cmp(&boxes[b as usize].center()[axis])
            });
            let left = nodes.len();
            nodes.extend([empty, empty]);
            nodes[node].first = left as u32;
            stack.push((left, start, start + mid));
            stack.push((left + 1, start + mid, end));
        }
        // Pad the boxes slightly so rounding never culls a hit on a triangle's edge.
        let pad = DVec3::splat(nodes[0].bounds.diagonal() * 1e-9 + f64::MIN_POSITIVE);
        for node in &mut nodes {
            node.bounds.min -= pad;
            node.bounds.max += pad;
        }
        Self { nodes, order }
    }

    fn leaf(&self, node: &Node) -> &[u32] {
        &self.order[node.first as usize..(node.first + node.count) as usize]
    }

    /// Nearest hit of the ray `origin + t·dir`, `t ≥ 0`, on either side of a triangle.
    /// `dir` need not be unit length; a zero direction hits nothing.
    pub fn raycast(&self, mesh: &TriMesh, origin: DVec3, dir: DVec3) -> Option<RayHit> {
        if self.nodes.is_empty() || dir == DVec3::ZERO {
            return None;
        }
        let mut best: Option<RayHit> = None;
        let mut stack = vec![(0u32, 0.0)];
        while let Some((index, entry)) = stack.pop() {
            if best.is_some_and(|b| entry > b.t) {
                continue;
            }
            let node = &self.nodes[index as usize];
            if node.count > 0 {
                for &triangle in self.leaf(node) {
                    if let Some(hit) = ray_triangle(mesh, triangle, origin, dir) {
                        if best.is_none_or(|b| hit.t < b.t) {
                            best = Some(hit);
                        }
                    }
                }
                continue;
            }
            let children = [node.first, node.first + 1].map(|c| {
                let t = ray_box(&self.nodes[c as usize].bounds, origin, dir);
                (c, t)
            });
            let [near, far] = if children[1].1 < children[0].1 {
                [children[1], children[0]]
            } else {
                children
            };
            for (child, t) in [far, near] {
                if let Some(t) = t {
                    stack.push((child, t));
                }
            }
        }
        best
    }

    /// The point of the mesh nearest to `p`.
    pub fn closest_point(&self, mesh: &TriMesh, p: DVec3) -> Option<ClosestPoint> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best: Option<(u32, DVec3)> = None;
        let mut best_distance_squared = f64::INFINITY;
        let mut stack = vec![(0u32, 0.0)];
        while let Some((index, distance_squared)) = stack.pop() {
            if distance_squared > best_distance_squared {
                continue;
            }
            let node = &self.nodes[index as usize];
            if node.count > 0 {
                for &triangle in self.leaf(node) {
                    let q = closest_on_triangle(mesh.triangle(triangle as usize), p);
                    let d = q.distance_squared(p);
                    if d < best_distance_squared {
                        best_distance_squared = d;
                        best = Some((triangle, q));
                    }
                }
                continue;
            }
            let children = [node.first, node.first + 1]
                .map(|c| (c, box_distance_squared(&self.nodes[c as usize].bounds, p)));
            let [near, far] = if children[1].1 < children[0].1 {
                [children[1], children[0]]
            } else {
                children
            };
            stack.push(far);
            stack.push(near);
        }
        best.map(|(triangle, point)| ClosestPoint {
            triangle,
            point,
            distance: best_distance_squared.sqrt(),
        })
    }
}

fn triangle_bounds([a, b, c]: [DVec3; 3]) -> Aabb {
    Aabb {
        min: a.min(b).min(c),
        max: a.max(b).max(c),
    }
}

fn union(a: Aabb, b: Aabb) -> Aabb {
    Aabb {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    }
}

/// Distance along the ray at which it enters the box, if it does at `t ≥ 0`.
fn ray_box(b: &Aabb, origin: DVec3, dir: DVec3) -> Option<f64> {
    let (mut near, mut far) = (0.0f64, f64::INFINITY);
    for axis in 0..3 {
        let (o, d) = (origin[axis], dir[axis]);
        if d == 0.0 {
            if o < b.min[axis] || o > b.max[axis] {
                return None;
            }
            continue;
        }
        let t1 = (b.min[axis] - o) / d;
        let t2 = (b.max[axis] - o) / d;
        near = near.max(t1.min(t2));
        far = far.min(t1.max(t2));
    }
    (near <= far).then_some(near)
}

fn box_distance_squared(b: &Aabb, p: DVec3) -> f64 {
    (b.min - p).max(p - b.max).max(DVec3::ZERO).length_squared()
}

/// Watertight intersection (Woop, Benthin and Wald, JCGT 2013), accepting hits from either side.
/// Each vertex is moved into a sheared frame where the ray runs along +z from the origin, so a
/// vertex or edge shared by two triangles is evaluated identically by both and a ray through it
/// cannot slip between them.
fn ray_triangle(mesh: &TriMesh, triangle: u32, origin: DVec3, dir: DVec3) -> Option<RayHit> {
    let kz = dir.abs().max_position();
    let (mut kx, mut ky) = ((kz + 1) % 3, (kz + 2) % 3);
    if dir[kz] < 0.0 {
        // Keep the winding of the sheared triangle.
        std::mem::swap(&mut kx, &mut ky);
    }
    let shear = DVec3::new(dir[kx] / dir[kz], dir[ky] / dir[kz], 1.0 / dir[kz]);
    let [a, b, c] = mesh.triangle(triangle as usize).map(|p| {
        let p = p - origin;
        DVec3::new(
            p[kx] - shear.x * p[kz],
            p[ky] - shear.y * p[kz],
            shear.z * p[kz],
        )
    });
    let mut u = c.x * b.y - c.y * b.x;
    let mut v = a.x * c.y - a.y * c.x;
    let mut w = b.x * a.y - b.y * a.x;
    if u == 0.0 || v == 0.0 || w == 0.0 {
        // On an edge or vertex: settle the signs exactly.
        u = diff_of_products(c.x, b.y, c.y, b.x);
        v = diff_of_products(a.x, c.y, a.y, c.x);
        w = diff_of_products(b.x, a.y, b.y, a.x);
    }
    if (u < 0.0 || v < 0.0 || w < 0.0) && (u > 0.0 || v > 0.0 || w > 0.0) {
        return None;
    }
    // Zero for a ray parallel to the plane or a triangle without area.
    let det = u + v + w;
    let t = (u * a.z + v * b.z + w * c.z) / det;
    // Rejects det = 0, hits behind the origin and non-finite input.
    if !(t >= 0.0 && t.is_finite()) {
        return None;
    }
    Some(RayHit {
        triangle,
        t,
        point: origin + dir * t,
        barycentric: DVec3::new(u, v, w) / det,
    })
}

/// `a·b − c·d` with a few ulps of error (Kahan), so its sign is exact barring underflow.
fn diff_of_products(a: f64, b: f64, c: f64, d: f64) -> f64 {
    let cd = c * d;
    let error = (-c).mul_add(d, cd);
    a.mul_add(b, -cd) + error
}

fn closest_on_segment(a: DVec3, b: DVec3, p: DVec3) -> DVec3 {
    let ab = b - a;
    let length_squared = ab.length_squared();
    if length_squared == 0.0 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / length_squared).clamp(0.0, 1.0)
}

/// Closest point on a triangle by Voronoi regions (Ericson, Real-Time Collision Detection, 5.1.5).
fn closest_on_triangle([a, b, c]: [DVec3; 3], p: DVec3) -> DVec3 {
    let (ab, ac) = (b - a, c - a);
    if ab.cross(ac).length_squared() == 0.0 {
        return [
            closest_on_segment(a, b, p),
            closest_on_segment(b, c, p),
            closest_on_segment(c, a, p),
        ]
        .into_iter()
        .min_by(|x, y| x.distance_squared(p).total_cmp(&y.distance_squared(p)))
        .expect("three candidates");
    }
    let ap = p - a;
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::cube;

    /// Deterministic pseudo-random numbers (Knuth's MMIX linear congruential generator).
    struct Lcg(u64);

    impl Lcg {
        fn unit(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.unit()
        }

        fn vec(&mut self, half: f64) -> DVec3 {
            DVec3::new(
                self.range(-half, half),
                self.range(-half, half),
                self.range(-half, half),
            )
        }
    }

    fn sphere() -> TriMesh {
        let mesh = TriMesh::uv_sphere(10.0, 48, 24);
        assert!(mesh.triangle_count() >= 2000);
        mesh
    }

    fn brute_raycast(mesh: &TriMesh, origin: DVec3, dir: DVec3) -> Option<RayHit> {
        (0..mesh.triangle_count() as u32)
            .filter_map(|t| ray_triangle(mesh, t, origin, dir))
            .min_by(|a, b| a.t.total_cmp(&b.t))
    }

    fn brute_closest(mesh: &TriMesh, p: DVec3) -> f64 {
        (0..mesh.triangle_count())
            .map(|t| closest_on_triangle(mesh.triangle(t), p).distance(p))
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn bvh_raycast_matches_brute_force_on_sphere() {
        let mesh = sphere();
        let bvh = Bvh::build(&mesh);
        let mut rng = Lcg(7);
        let mut hits = 0;
        for i in 0..400 {
            // Half the rays start inside the sphere, half outside aiming roughly at it.
            let origin = if i % 2 == 0 {
                rng.vec(5.0)
            } else {
                rng.vec(25.0)
            };
            let dir = if i % 2 == 0 {
                rng.vec(1.0)
            } else {
                (rng.vec(8.0) - origin) * rng.range(0.1, 3.0)
            };
            let expected = brute_raycast(&mesh, origin, dir);
            let actual = bvh.raycast(&mesh, origin, dir);
            match (expected, actual) {
                (None, None) => {}
                (Some(e), Some(a)) => {
                    hits += 1;
                    assert!((e.t - a.t).abs() < 1e-9, "ray {i}: {e:?} vs {a:?}");
                    assert!(a.point.distance(e.point) < 1e-9);
                    let [p0, p1, p2] = mesh.triangle(a.triangle as usize);
                    let b = a.barycentric;
                    assert!((p0 * b.x + p1 * b.y + p2 * b.z).distance(a.point) < 1e-9);
                    assert!((b.x + b.y + b.z - 1.0).abs() < 1e-12);
                }
                (e, a) => panic!("ray {i}: brute force {e:?}, bvh {a:?}"),
            }
        }
        assert!(hits >= 250, "only {hits} rays hit");
    }

    #[test]
    fn rays_through_shared_vertices_and_edges_never_leak() {
        let mesh = sphere();
        let bvh = Bvh::build(&mesh);
        let mut targets = mesh.positions.clone();
        for [a, b] in mesh.edge_map().into_keys() {
            let [p, q] = [a, b].map(|v| mesh.positions[v as usize]);
            targets.extend([0.5, 0.25, 0.75].map(|s| p.lerp(q, s)));
        }
        let origins = [
            DVec3::ZERO,
            DVec3::new(1.5, -2.25, 0.75),
            DVec3::new(-3.0, 2.0, -4.5),
            DVec3::new(0.1, 0.2, 6.0),
        ];
        for origin in origins {
            for &target in &targets {
                // The mesh is convex, so the first hit is the target itself, at t = 1.
                let hit = bvh.raycast(&mesh, origin, target - origin);
                let hit = hit.unwrap_or_else(|| panic!("ray {origin} -> {target} leaked"));
                assert!(
                    hit.t > 0.0 && (hit.t - 1.0).abs() < 1e-9,
                    "{origin} -> {target}"
                );
            }
        }
    }

    #[test]
    fn bvh_closest_point_matches_brute_force_on_sphere() {
        let mesh = sphere();
        let bvh = Bvh::build(&mesh);
        let mut rng = Lcg(11);
        for i in 0..300 {
            let p = rng.vec(15.0);
            let actual = bvh.closest_point(&mesh, p).unwrap();
            let expected = brute_closest(&mesh, p);
            assert!((actual.distance - expected).abs() < 1e-9, "point {i}");
            assert!((actual.point.distance(p) - actual.distance).abs() < 1e-9);
            let on_triangle = closest_on_triangle(mesh.triangle(actual.triangle as usize), p);
            assert!(on_triangle.distance(actual.point) < 1e-9);
        }
    }

    #[test]
    fn bvh_on_empty_mesh_finds_nothing() {
        let mesh = TriMesh::default();
        let bvh = Bvh::build(&mesh);
        assert!(bvh.raycast(&mesh, DVec3::ZERO, DVec3::X).is_none());
        assert!(bvh.closest_point(&mesh, DVec3::ZERO).is_none());
    }

    #[test]
    fn bvh_raycast_hits_both_faces_and_reports_barycentrics() {
        let mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::new(2.0, 0.0, 0.0),
                DVec3::new(0.0, 2.0, 0.0),
            ],
            vec![[0, 1, 2]],
        );
        let bvh = Bvh::build(&mesh);
        let from_above = bvh
            .raycast(&mesh, DVec3::new(0.5, 0.5, 4.0), DVec3::new(0.0, 0.0, -2.0))
            .unwrap();
        assert_eq!(from_above.t, 2.0);
        assert_eq!(from_above.point, DVec3::new(0.5, 0.5, 0.0));
        assert_eq!(from_above.barycentric, DVec3::new(0.5, 0.25, 0.25));
        let from_below = bvh.raycast(&mesh, DVec3::new(0.5, 0.5, -1.0), DVec3::Z);
        assert_eq!(from_below.unwrap().t, 1.0);
        // Pointing away, parallel, or with no direction: no hit.
        assert!(
            bvh.raycast(&mesh, DVec3::new(0.5, 0.5, 1.0), DVec3::Z)
                .is_none()
        );
        assert!(
            bvh.raycast(&mesh, DVec3::new(-1.0, 0.5, 0.0), DVec3::X)
                .is_none()
        );
        assert!(
            bvh.raycast(&mesh, DVec3::new(0.5, 0.5, 1.0), DVec3::ZERO)
                .is_none()
        );
    }

    #[test]
    fn bvh_axis_parallel_ray_along_a_box_face() {
        let mesh = cube(1.0);
        let bvh = Bvh::build(&mesh);
        let hit = bvh
            .raycast(&mesh, DVec3::new(0.0, 0.5, -3.0), DVec3::Z)
            .unwrap();
        assert!((hit.t - 3.0).abs() < 1e-12);
        let inside = bvh
            .raycast(&mesh, DVec3::splat(0.5), DVec3::new(0.0, 0.0, -1.0))
            .unwrap();
        assert_eq!(inside.t, 0.5);
    }

    #[test]
    fn bvh_closest_point_on_cube_regions() {
        let mesh = cube(1.0);
        let bvh = Bvh::build(&mesh);
        let corner = bvh.closest_point(&mesh, DVec3::splat(2.0)).unwrap();
        assert_eq!(corner.point, DVec3::ONE);
        assert!((corner.distance - 3f64.sqrt()).abs() < 1e-12);
        let face = bvh
            .closest_point(&mesh, DVec3::new(0.5, 0.25, -2.0))
            .unwrap();
        assert!(face.point.distance(DVec3::new(0.5, 0.25, 0.0)) < 1e-12);
        let inside = bvh.closest_point(&mesh, DVec3::new(0.5, 0.5, 0.9)).unwrap();
        assert!((inside.distance - 0.1).abs() < 1e-12);
    }

    #[test]
    fn bvh_handles_degenerate_triangles() {
        let mesh = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::X * 2.0, DVec3::Y],
            vec![[0, 1, 2], [0, 0, 3]],
        );
        let bvh = Bvh::build(&mesh);
        let near = bvh.closest_point(&mesh, DVec3::new(1.5, 1.0, 0.0)).unwrap();
        assert!(near.point.distance(DVec3::new(1.5, 0.0, 0.0)) < 1e-12);
        assert!(
            bvh.raycast(&mesh, DVec3::new(0.5, 0.0, 1.0), -DVec3::Z)
                .is_none()
        );
    }
}
