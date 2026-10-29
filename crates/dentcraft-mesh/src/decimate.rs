//! Edge-only quadric simplification (Garland & Heckbert, SIGGRAPH 1997).
//! <https://www.cs.cmu.edu/~garland/quadrics/>
//!
//! Quadrics rank candidates; they are NOT a distance bound. Each original vertex,
//! triangle centroid and edge midpoint has a witness face within the requested
//! distance. Before changing a face, its samples must find witnesses in the new
//! patch. Unchanged faces retain their witnesses, so errors cannot accumulate
//! unnoticed over successive collapses. This is a directed, sampled bound, not a
//! continuous or symmetric Hausdorff bound (nor a self-intersection test).

use std::{cmp::Ordering, collections::BinaryHeap, fmt};

use glam::{DMat3, DVec3};

use crate::TriMesh;

/// Controls simplification. Coordinates and distances are in millimetres.
#[derive(Clone, Debug)]
pub struct DecimateOptions {
    /// Maximum original-to-output sample distance; finite and nonnegative.
    pub max_error: f64,
    /// Stop once this count is reached. Error, locks and topology take priority;
    /// an unreachable count may leave more faces. Never undershoots the target.
    pub target_triangles: Option<usize>,
    /// One flag per input vertex. Locked vertices are never moved or removed.
    pub locked: Option<Vec<bool>>,
    /// Lock all original boundary vertices, including their exact coordinates.
    pub keep_boundary: bool,
}

impl Default for DecimateOptions {
    fn default() -> Self {
        Self {
            max_error: 0.002,
            target_triangles: None,
            locked: None,
            keep_boundary: true,
        }
    }
}

/// Counts and the measured, directed original-vertex-to-output distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecimateReport {
    pub triangles_before: usize,
    pub triangles_after: usize,
    /// Maximum true BVH closest-point distance over ORIGINAL vertices, in mm.
    /// Centroids and edge midpoints also constrain collapses, but are not part
    /// of this reported statistic. Empty surfaces report zero.
    pub max_error: f64,
}

/// Invalid input is rejected before any collapse; the input is never mutated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimateError {
    InvalidMaxError,
    InvalidLockCount,
    InvalidMesh,
    NonManifoldOrInconsistent,
    /// Floating-point validation could not certify the requested distance.
    ErrorBoundExceeded,
}

impl fmt::Display for DecimateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidMaxError => "max_error must be finite and nonnegative",
            Self::InvalidLockCount => "locked must have one flag per vertex",
            Self::InvalidMesh => "invalid indices, nonfinite geometry, zero-area faces, isolated vertices or oversized mesh",
            Self::NonManifoldOrInconsistent => "mesh must be an oriented manifold with no duplicate faces",
            Self::ErrorBoundExceeded => "could not certify the requested distance bound",
        })
    }
}

impl std::error::Error for DecimateError {}

// Symmetric homogeneous 4x4 quadric, stored as A, b, c: x'Ax + 2b'x + c.
#[derive(Clone, Copy, Default)]
struct Quadric {
    a: [f64; 6],
    b: DVec3,
    c: f64,
}

impl Quadric {
    fn plane(n: DVec3, p: DVec3) -> Self {
        let d = -n.dot(p);
        Self {
            a: [
                n.x * n.x,
                n.x * n.y,
                n.x * n.z,
                n.y * n.y,
                n.y * n.z,
                n.z * n.z,
            ],
            b: n * d,
            c: d * d,
        }
    }

    fn sum(self, other: Self) -> Self {
        Self {
            a: std::array::from_fn(|i| self.a[i] + other.a[i]),
            b: self.b + other.b,
            c: self.c + other.c,
        }
    }

    fn matrix(self) -> DMat3 {
        let [xx, xy, xz, yy, yz, zz] = self.a;
        DMat3::from_cols(
            DVec3::new(xx, xy, xz),
            DVec3::new(xy, yy, yz),
            DVec3::new(xz, yz, zz),
        )
    }

    fn cost(self, p: DVec3) -> f64 {
        (p.dot(self.matrix() * p) + 2.0 * self.b.dot(p) + self.c).max(0.0)
    }

    fn optimum(self) -> Option<DVec3> {
        let m = self.matrix();
        let scale = self.a[0] + self.a[3] + self.a[5];
        // Relative test: planar and almost planar systems use the stable fallback.
        if m.determinant().abs() <= 1e-12 * scale.powi(3) {
            return None;
        }
        let p = m.inverse() * -self.b;
        p.is_finite().then_some(p)
    }
}

struct Vertex {
    q: Quadric,
    faces: Vec<usize>,
    rejected: Vec<[u32; 2]>,
    generation: u64,
    locked: bool,
    boundary: bool,
}

#[derive(Clone, Copy)]
struct Candidate {
    edge: [u32; 2],
    generation: [u64; 2],
    position: DVec3,
    cost: f64,
    // Prefer short edges in flat regions instead of making enormous fans.
    length_squared: f64,
}

impl PartialEq for Candidate {
    fn eq(&self, rhs: &Self) -> bool {
        self.cmp(rhs) == Ordering::Equal
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}
impl Ord for Candidate {
    fn cmp(&self, rhs: &Self) -> Ordering {
        rhs.cost
            .total_cmp(&self.cost)
            .then_with(|| rhs.length_squared.total_cmp(&self.length_squared))
            .then_with(|| rhs.edge.cmp(&self.edge))
            .then_with(|| rhs.generation.cmp(&self.generation))
    }
}

struct Simplifier<'a> {
    original: &'a TriMesh,
    mesh: TriMesh,
    vertices: Vec<Vertex>,
    alive: Vec<bool>,
    // IDs below vertex_count are vertices; remaining IDs encode face*4 + sample.
    witnesses: Vec<Vec<u32>>,
    heap: BinaryHeap<Candidate>,
    remaining: usize,
}

/// Simplifies an oriented manifold by topology-preserving edge collapses.
///
/// Faces turning by 90 degrees or more, duplicate faces and zero-area faces are
/// rejected. Boundary/locked positions retain their exact bits; output indices
/// are compacted. Duplicate coordinates are accepted when their indexed faces
/// are nondegenerate and manifold. Zero-area, nonfinite, non-manifold and
/// inconsistently oriented input returns an error. A mesh with no faces is
/// returned unchanged; isolated vertices in a nonempty surface are rejected.
///
/// The heap is ordered by QEM cost, which is not monotonic in true distance.
/// Over-budget candidates are skipped; simplification stops when no admissible
/// edge remains or the target is reached. Targets never override accuracy.
pub fn decimate(
    mesh: &TriMesh,
    options: &DecimateOptions,
) -> Result<(TriMesh, DecimateReport), DecimateError> {
    if !options.max_error.is_finite() || options.max_error < 0.0 {
        return Err(DecimateError::InvalidMaxError);
    }
    if options
        .locked
        .as_ref()
        .is_some_and(|v| v.len() != mesh.vertex_count())
    {
        return Err(DecimateError::InvalidLockCount);
    }
    if mesh.positions.iter().any(|p| !p.is_finite())
        || mesh
            .vertex_count()
            .saturating_add(mesh.triangle_count().saturating_mul(4))
            > u32::MAX as usize
    {
        return Err(DecimateError::InvalidMesh);
    }
    let mut report = DecimateReport {
        triangles_before: mesh.triangle_count(),
        triangles_after: mesh.triangle_count(),
        max_error: 0.0,
    };
    if mesh.is_empty() {
        return Ok((mesh.clone(), report));
    }
    let mut state = Simplifier::new(mesh, options)?;
    state.run(options);
    let result = state.finish();
    let bvh = crate::Bvh::build(&result);
    for &p in &mesh.positions {
        let distance = bvh
            .closest_point(&result, p)
            .ok_or(DecimateError::ErrorBoundExceeded)?
            .distance;
        report.max_error = report.max_error.max(distance);
    }
    if report.max_error > options.max_error + roundoff(mesh) {
        return Err(DecimateError::ErrorBoundExceeded);
    }
    report.triangles_after = result.triangle_count();
    Ok((result, report))
}

// Only floating-point roundoff, not a modelling tolerance. In particular zero
// tolerance is useful on planar meshes. Do not round the measured report to zero.
fn roundoff(mesh: &TriMesh) -> f64 {
    mesh.positions
        .iter()
        .fold(1.0_f64, |s, p| s.max(p.abs().max_element()))
        * 64.0
        * f64::EPSILON
}

fn root(parent: &mut [u32], mut x: usize) -> usize {
    while parent[x] as usize != x {
        parent[x] = parent[parent[x] as usize];
        x = parent[x] as usize;
    }
    x
}

impl<'a> Simplifier<'a> {
    fn new(mesh: &'a TriMesh, options: &DecimateOptions) -> Result<Self, DecimateError> {
        let mut vertices: Vec<_> = (0..mesh.vertex_count())
            .map(|i| Vertex {
                q: Quadric::default(),
                faces: Vec::new(),
                rejected: Vec::new(),
                generation: 0,
                locked: options.locked.as_ref().is_some_and(|v| v[i]),
                boundary: false,
            })
            .collect();
        let mut edges = Vec::with_capacity(mesh.triangle_count() * 3);
        let mut witnesses = Vec::with_capacity(mesh.triangle_count());
        let mut parent: Vec<u32> = (0..mesh.triangle_count() as u32 * 3).collect();
        for (i, &t) in mesh.triangles.iter().enumerate() {
            if t.iter().any(|&v| v as usize >= vertices.len()) {
                return Err(DecimateError::InvalidMesh);
            }
            let cross = mesh.triangle_cross(i);
            let length = cross.length();
            if !length.is_finite() || length == 0.0 {
                return Err(DecimateError::InvalidMesh);
            }
            let q = Quadric::plane(cross / length, mesh.positions[t[0] as usize]);
            if !q.c.is_finite() {
                return Err(DecimateError::InvalidMesh);
            }
            let mut samples = Vec::with_capacity(7);
            for k in 0..4 {
                samples.push(mesh.vertex_count() as u32 + i as u32 * 4 + k);
            }
            for k in 0..3 {
                let v = t[k];
                let vertex = &mut vertices[v as usize];
                if vertex.faces.is_empty() {
                    samples.push(v);
                }
                vertex.faces.push(i);
                vertex.q = vertex.q.sum(q);
                let w = t[(k + 1) % 3];
                edges.push(([v.min(w), v.max(w)], i, v < w));
            }
            witnesses.push(samples);
        }
        edges.sort_unstable_by_key(|e| e.0);
        let mut unique = Vec::with_capacity(edges.len() / 2);
        let mut start = 0;
        while start < edges.len() {
            let key = edges[start].0;
            let mut end = start + 1;
            while end < edges.len() && edges[end].0 == key {
                end += 1;
            }
            if end - start > 2 {
                return Err(DecimateError::NonManifoldOrInconsistent);
            }
            if end - start == 1 {
                for v in key {
                    vertices[v as usize].boundary = true;
                    vertices[v as usize].locked |= options.keep_boundary;
                }
            } else {
                let (_, t, direction) = edges[start];
                let (_, u, other_direction) = edges[start + 1];
                if direction == other_direction {
                    return Err(DecimateError::NonManifoldOrInconsistent);
                }
                for v in key {
                    let a = 3 * t + mesh.triangles[t].iter().position(|&x| x == v).unwrap();
                    let b = 3 * u + mesh.triangles[u].iter().position(|&x| x == v).unwrap();
                    let a = root(&mut parent, a);
                    let b = root(&mut parent, b);
                    parent[a] = b as u32;
                }
            }
            unique.push(key);
            start = end;
        }
        for (v, vertex) in vertices.iter().enumerate() {
            if vertex.faces.is_empty() {
                return Err(DecimateError::InvalidMesh);
            }
            let mut component = None;
            for &t in &vertex.faces {
                let k = mesh.triangles[t]
                    .iter()
                    .position(|&x| x as usize == v)
                    .unwrap();
                let r = root(&mut parent, 3 * t + k);
                if component.is_some_and(|c| c != r) {
                    return Err(DecimateError::NonManifoldOrInconsistent);
                }
                component = Some(r);
            }
        }
        // Oppositely wound copies have valid edge counts but are not a surface.
        let mut faces = mesh.triangles.clone();
        for t in &mut faces {
            t.sort_unstable();
        }
        faces.sort_unstable();
        if faces.windows(2).any(|w| w[0] == w[1]) {
            return Err(DecimateError::NonManifoldOrInconsistent);
        }
        let mut state = Self {
            original: mesh,
            mesh: mesh.clone(),
            vertices,
            alive: vec![true; mesh.triangle_count()],
            witnesses,
            heap: BinaryHeap::new(),
            remaining: mesh.triangle_count(),
        };
        let candidates: Vec<_> = unique
            .into_iter()
            .filter_map(|edge| state.candidate(edge))
            .collect();
        state.heap = BinaryHeap::from(candidates);
        Ok(state)
    }

    fn candidate(&self, mut edge: [u32; 2]) -> Option<Candidate> {
        edge.sort_unstable();
        let [a, b] = edge.map(|v| &self.vertices[v as usize]);
        if a.faces.is_empty() || b.faces.is_empty() || (a.locked && b.locked) {
            return None;
        }
        let [p, r] = edge.map(|v| self.mesh.positions[v as usize]);
        let q = a.q.sum(b.q);
        let position = if a.locked {
            p
        } else if b.locked {
            r
        } else {
            q.optimum().unwrap_or_else(|| {
                [p, r, (p + r) * 0.5]
                    .into_iter()
                    .min_by(|&x, &y| q.cost(x).total_cmp(&q.cost(y)))
                    .unwrap()
            })
        };
        let cost = q.cost(position);
        (position.is_finite() && cost.is_finite()).then_some(Candidate {
            edge,
            generation: [a.generation, b.generation],
            position,
            cost,
            length_squared: p.distance_squared(r),
        })
    }

    fn sample(&self, id: u32) -> DVec3 {
        let n = self.original.vertex_count() as u32;
        if id < n {
            return self.original.positions[id as usize];
        }
        let index = (id - n) as usize;
        let t = self.original.triangle(index / 4);
        match index % 4 {
            0 => (t[0] + t[1] + t[2]) / 3.0,
            k => (t[k - 1] + t[k % 3]) * 0.5,
        }
    }

    fn neighbors(&self, v: u32) -> Vec<u32> {
        let mut neighbors: Vec<_> = self.vertices[v as usize]
            .faces
            .iter()
            .flat_map(|&f| self.mesh.triangles[f])
            .filter(|&w| w != v)
            .collect();
        neighbors.sort_unstable();
        neighbors.dedup();
        neighbors
    }

    fn run(&mut self, options: &DecimateOptions) {
        let target = options.target_triangles.unwrap_or(0);
        let limit = (options.max_error + roundoff(self.original)).powi(2);
        while self.remaining > target {
            let Some(candidate) = self.heap.pop() else {
                break;
            };
            let [mut a, mut b] = candidate.edge;
            if candidate.generation != candidate.edge.map(|v| self.vertices[v as usize].generation)
                || self.vertices[a as usize].faces.is_empty()
                || self.vertices[b as usize].faces.is_empty()
            {
                continue;
            }
            // Record a rejected edge at both ends until its neighbourhood changes.
            // A successful collapse drains these lists below. Entries already in
            // the heap need no update unless an endpoint's quadric changes.
            for v in candidate.edge {
                self.vertices[v as usize].rejected.push(candidate.edge);
            }
            if self.vertices[b as usize].locked {
                std::mem::swap(&mut a, &mut b);
            }
            let mut patch = self.vertices[a as usize].faces.clone();
            patch.extend_from_slice(&self.vertices[b as usize].faces);
            patch.sort_unstable();
            patch.dedup();
            let removed: Vec<_> = patch
                .iter()
                .copied()
                .filter(|&f| {
                    let t = self.mesh.triangles[f];
                    t.contains(&a) && t.contains(&b)
                })
                .collect();
            if removed.is_empty() || removed.len() > 2 || self.remaining - removed.len() < target {
                continue;
            }
            if self.vertices[a as usize].boundary
                && self.vertices[b as usize].boundary
                && removed.len() != 1
            {
                continue;
            }
            let na = self.neighbors(a);
            let nb = self.neighbors(b);
            let mut opposite: Vec<_> = removed
                .iter()
                .flat_map(|&f| self.mesh.triangles[f])
                .filter(|&v| v != a && v != b)
                .collect();
            opposite.sort_unstable();
            // The link condition prevents pinched/non-manifold edges and handles.
            if na
                .iter()
                .copied()
                .filter(|v| nb.binary_search(v).is_ok())
                .collect::<Vec<_>>()
                != opposite
            {
                continue;
            }
            let mut faces = Vec::with_capacity(patch.len() - removed.len());
            let mut keys = Vec::with_capacity(faces.capacity());
            let mut valid = true;
            for &f in &patch {
                if removed.contains(&f) {
                    continue;
                }
                let old = self.mesh.triangle(f);
                let indices = self.mesh.triangles[f].map(|v| if v == b { a } else { v });
                let points = indices.map(|v| {
                    if v == a {
                        candidate.position
                    } else {
                        self.mesh.positions[v as usize]
                    }
                });
                let n = (points[1] - points[0]).cross(points[2] - points[0]);
                let old_n = (old[1] - old[0]).cross(old[2] - old[0]);
                if !n.is_finite() || n.length_squared() == 0.0 || n.dot(old_n) <= 0.0 {
                    valid = false;
                    break;
                }
                let mut key = indices;
                key.sort_unstable();
                keys.push(key);
                faces.push((f, indices, DistanceTriangle::new(points)));
            }
            if !valid || faces.is_empty() {
                continue;
            }
            keys.sort_unstable();
            if keys.windows(2).any(|w| w[0] == w[1]) {
                continue;
            }
            let mut assignments = Vec::new();
            'samples: for &f in &patch {
                let previous = faces.iter().position(|&(face, _, _)| face == f);
                for &id in &self.witnesses[f] {
                    let p = self.sample(id);
                    // A witness need only satisfy the bound, not be the nearest
                    // face. Keeping it where possible avoids a full patch search.
                    let witness = previous
                        .filter(|&slot| faces[slot].2.distance_squared(p) <= limit)
                        .or_else(|| {
                            faces
                                .iter()
                                .position(|(_, _, tri)| tri.distance_squared(p) <= limit)
                        });
                    let Some(slot) = witness else {
                        valid = false;
                        break 'samples;
                    };
                    assignments.push((id, slot));
                }
            }
            if !valid {
                continue;
            }
            // Commit only after all geometry and distance checks have succeeded.
            let mut affected = na;
            affected.extend(nb);
            affected.extend([a, b]);
            affected.sort_unstable();
            affected.dedup();
            for &v in &affected {
                self.vertices[v as usize]
                    .faces
                    .retain(|f| patch.binary_search(f).is_err());
            }
            self.vertices[a as usize].generation += 1;
            self.vertices[b as usize].generation += 1;
            for &f in &patch {
                self.witnesses[f].clear();
            }
            for &f in &removed {
                self.alive[f] = false;
            }
            for &(f, indices, _) in &faces {
                self.mesh.triangles[f] = indices;
                for v in indices {
                    self.vertices[v as usize].faces.push(f);
                }
            }
            for (id, slot) in assignments {
                self.witnesses[faces[slot].0].push(id);
            }
            self.mesh.positions[a as usize] = candidate.position;
            self.vertices[a as usize].q =
                self.vertices[a as usize].q.sum(self.vertices[b as usize].q);
            self.vertices[a as usize].boundary |= self.vertices[b as usize].boundary;
            self.remaining -= removed.len();
            // Reconsider rejected edges whose geometry, witnesses or links may
            // have changed. Other queued edges retain their correct QEM cost.
            let mut edges = Vec::new();
            for v in affected {
                edges.append(&mut self.vertices[v as usize].rejected);
            }
            edges.sort_unstable();
            edges.dedup();
            for edge in &edges {
                for &v in edge {
                    self.vertices[v as usize].rejected.retain(|e| e != edge);
                }
            }
            edges.extend(self.neighbors(a).into_iter().map(|w| [a.min(w), a.max(w)]));
            edges.sort_unstable();
            edges.dedup();
            for edge in edges {
                if let Some(c) = self.candidate(edge) {
                    self.heap.push(c);
                }
            }
        }
    }

    fn finish(self) -> TriMesh {
        let mut map = vec![0; self.vertices.len()];
        let mut positions = Vec::new();
        for (i, vertex) in self.vertices.iter().enumerate() {
            if !vertex.faces.is_empty() {
                map[i] = positions.len() as u32;
                positions.push(self.mesh.positions[i]);
            }
        }
        let triangles = self
            .mesh
            .triangles
            .into_iter()
            .zip(self.alive)
            .filter(|(_, live)| *live)
            .map(|(t, _)| t.map(|v| map[v as usize]))
            .collect();
        TriMesh::new(positions, triangles)
    }
}

// Precomputed triangle projection for the many local witness checks. Distance
// is to the finite triangle, never just its supporting plane.
#[derive(Clone, Copy)]
struct DistanceTriangle {
    p: [DVec3; 3],
    n: DVec3,
    n2: f64,
}

impl DistanceTriangle {
    fn new(p: [DVec3; 3]) -> Self {
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        Self {
            p,
            n,
            n2: n.length_squared(),
        }
    }

    fn distance_squared(self, p: DVec3) -> f64 {
        let [a, b, c] = self.p;
        let height = (p - a).dot(self.n);
        let projected = p - self.n * (height / self.n2);
        if (b - a).cross(projected - a).dot(self.n) >= 0.0
            && (c - b).cross(projected - b).dot(self.n) >= 0.0
            && (a - c).cross(projected - c).dot(self.n) >= 0.0
        {
            return height * height / self.n2;
        }
        [(a, b), (b, c), (c, a)]
            .into_iter()
            .map(|(a, b)| {
                let edge = b - a;
                let t = ((p - a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0);
                p.distance_squared(a + edge * t)
            })
            .fold(f64::INFINITY, f64::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bvh;

    fn grid(n: u32) -> TriMesh {
        let positions = (0..=n)
            .flat_map(|y| {
                (0..=n).map(move |x| DVec3::new(x as f64 / n as f64, y as f64 / n as f64, 0.0))
            })
            .collect();
        let mut triangles = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let a = y * (n + 1) + x;
                triangles.extend([[a, a + 1, a + n + 2], [a, a + n + 2, a + n + 1]]);
            }
        }
        TriMesh::new(positions, triangles)
    }

    // Independent BVH measurements, including interiors and every edge midpoint.
    fn check_error(original: &TriMesh, output: &TriMesh, tolerance: f64) -> (f64, f64) {
        let bvh = Bvh::build(output);
        let distance = |p| bvh.closest_point(output, p).unwrap().distance;
        let vertex_error = original
            .positions
            .iter()
            .map(|&p| distance(p))
            .fold(0.0, f64::max);
        let mut interior_error = 0.0_f64;
        for i in 0..original.triangle_count() {
            let [a, b, c] = original.triangle(i);
            for p in [
                (a + b + c) / 3.0,
                (a + b) * 0.5,
                (b + c) * 0.5,
                (c + a) * 0.5,
            ] {
                interior_error = interior_error.max(distance(p));
            }
        }
        assert!(
            vertex_error <= tolerance,
            "vertex error {vertex_error} > {tolerance}"
        );
        assert!(
            interior_error <= tolerance,
            "interior error {interior_error} > {tolerance}"
        );
        (vertex_error, interior_error)
    }

    fn bits(p: DVec3) -> [u64; 3] {
        p.to_array().map(f64::to_bits)
    }

    #[test]
    fn fine_sphere_accuracy_reduction_and_topology() {
        let mesh = TriMesh::uv_sphere(5.0, 384, 192);
        let options = DecimateOptions::default();
        let (output, report) = decimate(&mesh, &options).unwrap();
        let (vertices, interiors) = check_error(&mesh, &output, 0.002);
        println!(
            "sphere: {} -> {} triangles; vertex error {vertices:.9} mm; interior error {interiors:.9} mm",
            mesh.triangle_count(),
            output.triangle_count()
        );
        assert_eq!(report.max_error, vertices);
        assert_eq!(report.triangles_before, mesh.triangle_count());
        assert_eq!(report.triangles_after, output.triangle_count());
        assert!(
            output.triangle_count() * 5 <= mesh.triangle_count(),
            "less than 80% reduction: {report:?}"
        );
        assert!(output.is_closed());
        assert!(output.is_consistently_oriented());
        assert!(output.signed_volume() > 0.0);
        for i in 0..output.triangle_count() {
            assert!(output.triangle_cross(i).dot(output.triangle(i)[0]) > 0.0);
        }
    }

    #[test]
    fn flat_grid_has_zero_error_and_few_faces() {
        let mesh = grid(20);
        let options = DecimateOptions {
            max_error: 0.0,
            keep_boundary: false,
            ..Default::default()
        };
        let (output, report) = decimate(&mesh, &options).unwrap();
        println!(
            "plane: {} -> {}; error {}",
            mesh.triangle_count(),
            output.triangle_count(),
            report.max_error
        );
        assert!(output.triangle_count() <= 8, "{report:?}");
        assert!(report.max_error < 1e-14);
        check_error(&mesh, &output, 1e-14);
        assert!(output.is_consistently_oriented());
        assert_eq!(output.non_manifold_edge_count(), 0);
    }

    #[test]
    fn boundaries_and_locked_vertices_keep_exact_bits() {
        let mut mesh = grid(16);
        // A non-axis-aligned steep plane, including signed zero on one corner.
        let rotation = DMat3::from_rotation_y(1.3) * DMat3::from_rotation_z(0.37);
        for p in &mut mesh.positions {
            *p = rotation * *p;
        }
        mesh.positions[0].x = -0.0;
        let mut locked = vec![false; mesh.vertex_count()];
        locked[8 * 17 + 8] = true;
        locked[6 * 17 + 5] = true;
        let options = DecimateOptions {
            locked: Some(locked.clone()),
            ..Default::default()
        };
        let (output, _) = decimate(&mesh, &options).unwrap();
        assert!(output.triangle_count() < mesh.triangle_count() / 2);
        let boundary = mesh.boundary_loops().into_iter().flatten();
        for v in boundary.chain(
            locked
                .iter()
                .enumerate()
                .filter_map(|(v, &lock)| lock.then_some(v as u32)),
        ) {
            assert!(
                output
                    .positions
                    .iter()
                    .any(|&p| bits(p) == bits(mesh.positions[v as usize]))
            );
        }
        assert_eq!(
            output.boundary_loops()[0].len(),
            mesh.boundary_loops()[0].len()
        );
        check_error(&mesh, &output, 1e-12);
    }

    #[test]
    fn target_is_honoured_without_overriding_error_or_locks() {
        let mesh = TriMesh::uv_sphere(5.0, 32, 16);
        let options = DecimateOptions {
            max_error: 1.0,
            target_triangles: Some(300),
            ..Default::default()
        };
        let (output, _) = decimate(&mesh, &options).unwrap();
        assert_eq!(output.triangle_count(), 300);
        assert!(output.is_closed());
        let (unchanged, _) = decimate(
            &mesh,
            &DecimateOptions {
                locked: Some(vec![true; mesh.vertex_count()]),
                ..options.clone()
            },
        )
        .unwrap();
        assert_eq!(unchanged, mesh);
        let (bounded, report) = decimate(
            &mesh,
            &DecimateOptions {
                max_error: 1e-10,
                ..options
            },
        )
        .unwrap();
        assert!(bounded.triangle_count() > 300);
        assert!(report.max_error <= 1e-10);
        let (unchanged, _) = decimate(
            &mesh,
            &DecimateOptions {
                target_triangles: Some(mesh.triangle_count() + 1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(unchanged, mesh);
    }

    #[test]
    fn steep_noisy_surface_preserves_original_interior_samples() {
        let mut mesh = grid(32);
        let rotation = DMat3::from_rotation_y(1.4) * DMat3::from_rotation_z(0.61);
        for (i, p) in mesh.positions.iter_mut().enumerate() {
            let noise = ((i * 7919 % 101) as f64 / 100.0 - 0.5) * 0.0006;
            p.z = 0.08 * (p.x * 9.0).sin() * (p.y * 7.0).cos() + noise;
            *p = rotation * *p;
        }
        let (output, report) = decimate(&mesh, &DecimateOptions::default()).unwrap();
        assert!(output.triangle_count() < mesh.triangle_count());
        let (vertices, _) = check_error(&mesh, &output, 0.002);
        assert_eq!(report.max_error, vertices);
        assert_eq!(output.non_manifold_edge_count(), 0);
        assert!(output.is_consistently_oriented());
    }

    #[test]
    fn degenerate_and_invalid_inputs_do_not_panic() {
        let options = DecimateOptions::default();
        let empty = TriMesh::default();
        assert_eq!(decimate(&empty, &options).unwrap().0, empty);
        let cloud = TriMesh::new(vec![DVec3::ZERO, DVec3::ZERO], vec![]);
        assert_eq!(decimate(&cloud, &options).unwrap().0, cloud);
        for triangles in [vec![[0, 1, 2]], vec![[0, 0, 1]], vec![[0, 1, 99]]] {
            let mesh = TriMesh::new(vec![DVec3::ZERO, DVec3::X, DVec3::X], triangles);
            assert_eq!(decimate(&mesh, &options), Err(DecimateError::InvalidMesh));
        }
        let mut mesh = crate::tests::unit_tetrahedron();
        mesh.positions[0].x = f64::NAN;
        assert_eq!(decimate(&mesh, &options), Err(DecimateError::InvalidMesh));
        assert_eq!(
            decimate(
                &empty,
                &DecimateOptions {
                    max_error: -1.0,
                    ..options.clone()
                }
            ),
            Err(DecimateError::InvalidMaxError)
        );
        assert_eq!(
            decimate(
                &empty,
                &DecimateOptions {
                    max_error: f64::NAN,
                    ..options.clone()
                }
            ),
            Err(DecimateError::InvalidMaxError)
        );
        assert_eq!(
            decimate(
                &empty,
                &DecimateOptions {
                    locked: Some(vec![false]),
                    ..options
                }
            ),
            Err(DecimateError::InvalidLockCount)
        );
    }

    #[test]
    fn duplicate_coordinates_are_supported_but_invalid_topology_is_rejected() {
        let mut mesh = crate::tests::unit_tetrahedron();
        // Two disconnected, coincident surfaces still have valid indexed topology.
        let other = mesh.clone();
        mesh.merge(&other);
        let (output, _) = decimate(&mesh, &DecimateOptions::default()).unwrap();
        assert_eq!(output.triangle_count(), 8);
        assert!(output.is_closed());
        let mut duplicate = crate::tests::unit_tetrahedron();
        duplicate.triangles.push(duplicate.triangles[0]);
        assert_eq!(
            decimate(&duplicate, &DecimateOptions::default()),
            Err(DecimateError::NonManifoldOrInconsistent)
        );
        let mut reversed = crate::tests::unit_tetrahedron();
        reversed.triangles[0].swap(0, 1);
        assert_eq!(
            decimate(&reversed, &DecimateOptions::default()),
            Err(DecimateError::NonManifoldOrInconsistent)
        );
        let bowtie = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, -DVec3::X, -DVec3::Y],
            vec![[0, 1, 2], [0, 3, 4]],
        );
        assert_eq!(
            decimate(&bowtie, &DecimateOptions::default()),
            Err(DecimateError::NonManifoldOrInconsistent)
        );
    }

    #[test]
    fn distance_kernel_matches_bvh_and_quadric_fallback_is_finite() {
        let mesh = crate::tests::unit_tetrahedron();
        for i in 0..mesh.triangle_count() {
            let points = mesh.triangle(i);
            let triangle = DistanceTriangle::new(points);
            let single = TriMesh::new(points.to_vec(), vec![[0, 1, 2]]);
            let bvh = Bvh::build(&single);
            for x in -4..=4 {
                for y in -4..=4 {
                    for z in -4..=4 {
                        let p = DVec3::new(x as f64, y as f64, z as f64) * 0.37;
                        let expected = bvh.closest_point(&single, p).unwrap().distance.powi(2);
                        assert!((triangle.distance_squared(p) - expected).abs() < 1e-13);
                    }
                }
            }
        }
        let q = Quadric::plane(DVec3::Z, DVec3::ZERO);
        assert!(q.optimum().is_none());
        assert_eq!(q.cost(DVec3::new(2.0, 3.0, 4.0)), 16.0);
    }

    #[test]
    #[ignore = "million-triangle release benchmark"]
    fn perf_decimate_1m() {
        let mesh = TriMesh::uv_sphere(5.0, 1024, 512);
        let start = std::time::Instant::now();
        let (output, report) = decimate(&mesh, &DecimateOptions::default()).unwrap();
        let elapsed = start.elapsed().as_secs_f64();
        println!(
            "perf_decimate_1m: {elapsed:.3} s; {} -> {} triangles; error {:.9} mm",
            mesh.triangle_count(),
            output.triangle_count(),
            report.max_error
        );
        assert!(report.max_error <= 0.002);
        assert!(output.triangle_count() * 5 <= mesh.triangle_count());
        assert!(
            elapsed < 3.0,
            "million-triangle decimation took {elapsed:.3} s (target < 3 s)"
        );
    }
}
