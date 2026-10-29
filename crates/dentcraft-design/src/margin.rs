//! Convex-ridge margin tracing. Inputs must have outward consistent winding.
use crate::DesignError;
use dentcraft_mesh::{Bvh, TriMesh, VertexAdjacency, smooth_polyline_on_surface};
use glam::DVec3;
use std::f64::consts::TAU;
use std::sync::Arc;

/// Closed curve: neither list repeats the first entry at the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Margin {
    /// Surface-projected smooth editing samples, with nominal spacing 0.1 mm.
    /// Use `region_boundary` for geometry paired with the region barrier.
    pub points: Vec<DVec3>,
    /// Simple underlying edge loop, used as the region barrier.
    pub vertices: Vec<u32>,
}

impl Margin {
    /// Exact surface boundary of the region barrier. Use this curve for axis
    /// selection and inner-surface generation with `MarginTracer::region`.
    /// `points` is a smoothed tracing/editing curve and can cross either side of
    /// the barrier on noisy scans; it must not be substituted for this boundary.
    /// Preserve `vertices` with the margin instead of tracing the display curve
    /// again: ridge capture is deliberately allowed to move user clicks.
    pub fn region_boundary(&self, mesh: &TriMesh) -> Result<Vec<DVec3>, DesignError> {
        let unique: std::collections::HashSet<_> = self.vertices.iter().collect();
        if self.vertices.len() < 3 || unique.len() != self.vertices.len() {
            return Err(DesignError::InvalidMargin);
        }
        self.vertices
            .iter()
            .map(|&v| {
                mesh.positions
                    .get(v as usize)
                    .copied()
                    .filter(|p| p.is_finite())
                    .ok_or(DesignError::InvalidMesh)
            })
            .collect()
    }
}

/// Reusable immutable geometry cache. Rebuild after changing a mesh.
pub struct MarginTracer {
    mesh: Arc<TriMesh>,
    adjacency: VertexAdjacency,
    /// Smoothed, noise-scaled k1 evidence; this is not a physical curvature.
    ridge_strength: Vec<f64>,
    bvh: Bvh,
}

pub(crate) fn validate_mesh(mesh: &TriMesh) -> Result<(), DesignError> {
    if mesh.triangles.is_empty() {
        return Err(DesignError::EmptyRegion);
    }
    if mesh.positions.iter().any(|p| !p.is_finite())
        || mesh
            .triangles
            .iter()
            .flatten()
            .any(|&v| v as usize >= mesh.positions.len())
    {
        return Err(DesignError::InvalidMesh);
    }
    if !(0..mesh.triangles.len()).any(|i| mesh.triangle_cross(i).length_squared() > 1e-24) {
        return Err(DesignError::DegenerateRegion);
    }
    Ok(())
}

impl MarginTracer {
    /// Cache geometry and principal curvature, smoothing k1 over two one-rings.
    /// Ridge costs are normalized locally for each trace or proposal, so unrelated
    /// sharp model corners cannot flatten the contrast at a rounded margin.
    /// Empty, wholly degenerate and nonfinite meshes are rejected.
    pub fn new(mesh: Arc<TriMesh>) -> Result<Self, DesignError> {
        validate_mesh(&mesh)?;
        let adjacency = mesh.vertex_adjacency();
        let mut denoised = (*mesh).clone();
        denoised.laplacian_smooth(2, 0.25, None, true);
        let curvature = denoised.principal_curvatures();
        // Displacement noise contributes curvature proportional to 1/h^2. Scale
        // by squared cross-ridge support before averaging, so short edges on the
        // ledge do not win merely by amplifying noise. Cap h at 0.1 mm: coarse
        // base triangles must not gain unlimited influence. Geometry and the BVH
        // stay on the original scan; the denoised copy supplies evidence only.
        let mut ridge_strength: Vec<_> = curvature
            .iter()
            .enumerate()
            .map(|(i, pc)| {
                let neighbors = adjacency.neighbors(i as u32);
                let support = neighbors
                    .iter()
                    .map(|&j| {
                        (denoised.positions[j as usize] - denoised.positions[i])
                            .dot(pc.dir1)
                            .abs()
                            .min(0.1)
                    })
                    .sum::<f64>();
                if neighbors.is_empty() {
                    0.0
                } else {
                    pc.k1 * (support / neighbors.len() as f64).powi(2)
                }
            })
            .collect();
        for _ in 0..2 {
            let previous = ridge_strength.clone();
            for (i, strength) in ridge_strength.iter_mut().enumerate() {
                let neighbors = adjacency.neighbors(i as u32);
                if !neighbors.is_empty() {
                    *strength = 0.5 * previous[i]
                        + 0.5 * neighbors.iter().map(|&j| previous[j as usize]).sum::<f64>()
                            / neighbors.len() as f64;
                }
            }
        }
        let bvh = Bvh::build(&mesh);
        Ok(Self {
            mesh,
            adjacency,
            ridge_strength,
            bvh,
        })
    }

    fn component(&self, seed: u32) -> Vec<bool> {
        let mut component = vec![false; self.mesh.positions.len()];
        let mut stack = vec![seed];
        component[seed as usize] = true;
        while let Some(i) = stack.pop() {
            for &j in self.adjacency.neighbors(i) {
                if !component[j as usize] {
                    component[j as usize] = true;
                    stack.push(j);
                }
            }
        }
        component
    }

    /// The mesh the tracer was built for.
    pub fn mesh(&self) -> &Arc<TriMesh> {
        &self.mesh
    }

    fn snap(&self, point: DVec3) -> Result<u32, DesignError> {
        if !point.is_finite() {
            return Err(DesignError::InvalidMargin);
        }
        let hit = self
            .bvh
            .closest_point(&self.mesh, point)
            .ok_or(DesignError::InvalidMargin)?;
        // Closest face supplies a surface-local vertex (avoids isolated vertices).
        Ok(*self.mesh.triangles[hit.triangle as usize]
            .iter()
            .min_by(|&&a, &&b| {
                self.mesh.positions[a as usize]
                    .distance_squared(hit.point)
                    .total_cmp(&self.mesh.positions[b as usize].distance_squared(hit.point))
            })
            .unwrap())
    }

    /// Snap clicks through the nearest surface point, then capture the strongest
    /// convex ridge within 0.45 mm of the snapped point. This local capture allows
    /// imprecise clicks; outside that band the caller must place another click.
    /// Ordered clicks must describe a simple loop on one connected component.
    /// Each leg stays within max(1.5 mm, 0.6 times its chord length) of its
    /// waypoint segment and on its side of the loop centre. Missing ridges use
    /// surface distance; excessive ridge detours fall back to the geodesic.
    /// Routing and projected-crossing errors identify a one-based waypoint leg.
    pub fn trace(&self, clicks: &[DVec3]) -> Result<Margin, DesignError> {
        if clicks.len() < 3 {
            return Err(DesignError::InvalidMargin);
        }
        let component = self.component(self.snap(clicks[0])?);
        for &click in clicks {
            if !component[self.snap(click)? as usize] {
                return Err(DesignError::InvalidMargin);
            }
        }
        let mut waypoints = Vec::with_capacity(clicks.len());
        let mut neighborhood = std::collections::HashSet::new();
        for &click in clicks {
            let v = self.snap(click)?;
            let center = self.mesh.positions[v as usize];
            // Search only the connected local patch, never a nearby separate tooth.
            let mut stack = vec![v];
            let mut seen = std::collections::HashSet::with_capacity(64);
            seen.insert(v);
            let mut best: Option<u32> = None;
            while let Some(a) = stack.pop() {
                if !self.adjacency.is_boundary(a) {
                    neighborhood.insert(a);
                    if best.is_none_or(|b| {
                        self.ridge_strength[a as usize] > self.ridge_strength[b as usize]
                    }) {
                        best = Some(a);
                    }
                }
                for &b in self.adjacency.neighbors(a) {
                    if self.mesh.positions[b as usize].distance_squared(center) <= 0.45_f64.powi(2)
                        && seen.insert(b)
                    {
                        stack.push(b);
                    }
                }
            }
            waypoints.push(best.ok_or(DesignError::InvalidMargin)?);
        }
        let cost = self.local_cost(neighborhood.into_iter());
        self.connect(&waypoints, &cost)
    }

    fn local_cost(&self, vertices: impl Iterator<Item = u32>) -> Vec<f64> {
        let mut strengths: Vec<_> = vertices
            .map(|i| self.ridge_strength[i as usize].max(0.0))
            .collect();
        strengths.sort_by(f64::total_cmp);
        let scale = strengths
            .get(strengths.len() * 95 / 100)
            .copied()
            .unwrap_or(0.0);
        let sharpness = if scale > 1e-9 { 8.0 / scale } else { 0.0 };
        // Keep the 95th percentile above the path toolkit's 1e-6 floor.
        // Using exp(-8) leaves contrast between the ridge and its neighbours.
        let mut costs: Vec<_> = self
            .ridge_strength
            .iter()
            .map(|k| (-sharpness * k.max(0.0)).exp().max(1e-6))
            .collect();
        for (i, c) in costs.iter_mut().enumerate() {
            if self.adjacency.is_boundary(i as u32) {
                *c = 1.0;
            }
        }
        costs
    }

    /// Route a closed loop through `waypoints` over the surface: each leg takes
    /// the `cost`-weighted corridor path, unless a much shorter geodesic exists
    /// (a ridge detour that faded). Routed vertices are blocked for later legs.
    /// Returns the barrier loop, its per-edge waypoint leg and the loop's frame.
    fn route(
        &self,
        waypoints: &[u32],
        cost: &[f64],
    ) -> Result<(Vec<u32>, Vec<usize>, crate::geometry::Frame), DesignError> {
        if waypoints.len() < 3 {
            return Err(DesignError::InvalidMargin);
        }
        let positions: Vec<_> = waypoints
            .iter()
            .map(|&i| self.mesh.positions[i as usize])
            .collect();
        let center = positions.iter().sum::<DVec3>() / positions.len() as f64;
        let axis = loop_axis(&positions)?;
        let mut vertices = Vec::new();
        let mut edge_legs = Vec::new();
        let geodesic_cost = vec![1.0; self.mesh.positions.len()];
        let mut blocked = vec![false; self.mesh.positions.len()];
        for &v in waypoints {
            blocked[v as usize] = true;
        }
        for leg in 0..waypoints.len() {
            let a = waypoints[leg];
            let b = waypoints[(leg + 1) % waypoints.len()];
            let mut path = self
                .corridor_path(
                    a,
                    b,
                    crate::geometry::Frame::new(center, axis),
                    cost,
                    &blocked,
                    true,
                )
                .or_else(|| {
                    self.corridor_path(
                        a,
                        b,
                        crate::geometry::Frame::new(center, axis),
                        cost,
                        &blocked,
                        false,
                    )
                })
                .ok_or(DesignError::MarginLeg {
                    leg: leg + 1,
                    reason: "no surface path inside the waypoint corridor",
                })?;
            if let Some(short) = self.corridor_path(
                a,
                b,
                crate::geometry::Frame::new(center, axis),
                &geodesic_cost,
                &blocked,
                false,
            ) {
                let length = |p: &[u32]| {
                    p.windows(2)
                        .map(|e| {
                            self.mesh.positions[e[0] as usize]
                                .distance(self.mesh.positions[e[1] as usize])
                        })
                        .sum::<f64>()
                };
                if length(&path) > 1.4 * length(&short) + 0.3 {
                    path = short;
                }
            }
            for &v in &path[..path.len() - 1] {
                blocked[v as usize] = true;
            }
            edge_legs.extend(std::iter::repeat_n(leg + 1, path.len() - 1));
            vertices.extend_from_slice(&path[..path.len() - 1]);
        }
        let unique: std::collections::HashSet<_> = vertices.iter().collect();
        if vertices.len() < 3 || unique.len() != vertices.len() {
            return Err(DesignError::InvalidMargin);
        }
        let raw: Vec<_> = vertices
            .iter()
            .map(|&i| self.mesh.positions[i as usize])
            .collect();
        let frame = crate::geometry::Frame::new(center, axis);
        validate_trace_curve(&raw, frame, &raw, &edge_legs)?;
        Ok((vertices, edge_legs, frame))
    }

    fn connect(&self, waypoints: &[u32], cost: &[f64]) -> Result<Margin, DesignError> {
        let (vertices, edge_legs, frame) = self.route(waypoints, cost)?;
        let raw: Vec<_> = vertices
            .iter()
            .map(|&i| self.mesh.positions[i as usize])
            .collect();
        // Centre broad ridge plateaus within one short edge, avoiding a full
        // mesh-row jump when two samples have indistinguishable evidence.
        let points: Vec<_> = vertices
            .iter()
            .map(|&v| {
                let neighbors = self.adjacency.neighbors(v);
                let peak = std::iter::once(v)
                    .chain(neighbors.iter().copied())
                    .map(|i| self.ridge_strength[i as usize])
                    .fold(0.0, f64::max);
                let mut sum = DVec3::ZERO;
                let mut weight = 0.0;
                for i in std::iter::once(v).chain(neighbors.iter().copied()) {
                    let strength = self.ridge_strength[i as usize];
                    if strength >= 0.85 * peak
                        && strength > 0.0
                        && !self.adjacency.is_boundary(i)
                        && self.mesh.positions[i as usize]
                            .distance_squared(self.mesh.positions[v as usize])
                            < 0.2 * 0.2
                    {
                        sum += self.mesh.positions[i as usize] * strength;
                        weight += strength;
                    }
                }
                if weight > 0.0 {
                    sum / weight
                } else {
                    self.mesh.positions[v as usize]
                }
            })
            .collect();
        let smooth = smooth_polyline_on_surface(&self.mesh, &self.bvh, &points, 1);
        let mut points = resample(&smooth, 0.1);
        for p in &mut points {
            if let Some(hit) = self.bvh.closest_point(&self.mesh, *p) {
                *p = hit.point;
            }
        }
        validate_trace_curve(&points, frame, &raw, &edge_legs)?;
        Ok(Margin { points, vertices })
    }

    /// Dijkstra restricted to a 3D corridor and the angular wedge of this leg.
    /// The wedge keeps neighbouring legs on their own side of the stump; the
    /// finite length term also gives a geodesic fallback where ridge evidence fades.
    fn corridor_path(
        &self,
        from: u32,
        to: u32,
        frame: crate::geometry::Frame,
        cost: &[f64],
        blocked: &[bool],
        monotone: bool,
    ) -> Option<Vec<u32>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let center = frame.origin;
        let axis = frame.w;
        let a = self.mesh.positions[from as usize];
        let b = self.mesh.positions[to as usize];
        let ab = b - a;
        if ab.length_squared() < 1e-20 {
            return None;
        }
        let width = (0.6 * ab.length()).max(1.5);
        let ra = (a - center).reject_from_normalized(axis);
        let rb = (b - center).reject_from_normalized(axis);
        let sign = ra.cross(rb).dot(axis).signum();
        let allowed = |i: u32| {
            if self.adjacency.is_boundary(i) {
                return false;
            }
            let p = self.mesh.positions[i as usize];
            let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            let r = (p - center).reject_from_normalized(axis);
            p.distance_squared(a + t * ab) <= width * width
                && sign * ra.cross(r).dot(axis) >= -0.03 * ra.length()
                && sign * r.cross(rb).dot(axis) >= -0.03 * rb.length()
                && r.dot(ra + rb) > 0.0
        };
        let mut dist = vec![f64::INFINITY; self.mesh.positions.len()];
        let mut prev = vec![u32::MAX; dist.len()];
        let mut heap = BinaryHeap::new();
        dist[from as usize] = 0.0;
        heap.push(Reverse((0u64, from)));
        while let Some(Reverse((bits, v))) = heap.pop() {
            let d = f64::from_bits(bits);
            if d > dist[v as usize] {
                continue;
            }
            if v == to {
                let mut path = vec![v];
                let mut v = v;
                while v != from {
                    v = prev[v as usize];
                    path.push(v);
                }
                path.reverse();
                return Some(path);
            }
            for &w in self.adjacency.neighbors(v) {
                if w != to && (blocked[w as usize] || !allowed(w)) {
                    continue;
                }
                let rv = (self.mesh.positions[v as usize] - center).reject_from_normalized(axis);
                let rw = (self.mesh.positions[w as usize] - center).reject_from_normalized(axis);
                if monotone && sign * rv.cross(rw).dot(axis) < -1e-10 {
                    continue;
                }
                let length =
                    self.mesh.positions[v as usize].distance(self.mesh.positions[w as usize]);
                let next = d + length * (0.02 + 0.5 * (cost[v as usize] + cost[w as usize]));
                if next < dist[w as usize] {
                    dist[w as usize] = next;
                    prev[w as usize] = v;
                    heap.push(Reverse((next.to_bits(), w)));
                }
            }
        }
        None
    }

    /// Propose a single-tooth margin, then refine its centre and best-fit axis.
    /// Follow the stump wall to bound the search, then jointly select a closed
    /// ring of ridge candidates with at most 1 mm radial or axial sector jumps.
    /// The axial window scales with the local preparation, capped at 12 mm.
    /// The final projected loop must be simple, enclose the seed and measure
    /// 8–45 mm. Missing or ambiguous surface corridors return `InvalidMargin`.
    pub fn propose(&self, seed: DVec3, axis: DVec3) -> Result<Margin, DesignError> {
        if !axis.is_finite() || axis.length_squared() < 1e-24 {
            return Err(DesignError::InvalidAxis);
        }
        let axis = axis.normalize();
        let seed_vertex = self.snap(seed)?;
        let center = self.mesh.positions[seed_vertex as usize];
        // Exclude disconnected fragments. Connected neighbouring teeth are
        // excluded separately by the wall outline and radial visibility tests.
        let component = self.component(seed_vertex);
        let first = self.sector_ridges(center, axis, &component, None, None)?;
        let first_cost = self.local_cost(
            self.mesh
                .positions
                .iter()
                .enumerate()
                .filter(|(i, p)| {
                    component[*i]
                        && (**p - center).cross(axis).length() < 8.0
                        && !self.adjacency.is_boundary(*i as u32)
                        && first.iter().any(|&j| {
                            (**p - self.mesh.positions[j as usize]).length_squared() < 0.5 * 0.5
                        })
                })
                .map(|(i, _)| i as u32),
        );
        let first_margin = self
            .connect(&first, &first_cost)
            .map_err(|_| DesignError::InvalidMargin)?;
        let points = first_margin.points;
        let centroid = points.iter().sum::<DVec3>() / points.len() as f64;
        let mut cov = [[0.0; 3]; 3];
        for p in &points {
            let d = *p - centroid;
            for r in 0..3 {
                for c in 0..3 {
                    cov[r][c] += d[r] * d[c];
                }
            }
        }
        let (values, vectors) = crate::orient::eigen3(cov);
        if values[1] <= 1.5 * values[0].max(0.0) {
            return Err(DesignError::InvalidMargin);
        }
        let mut refined = vectors[0];
        if refined.dot(axis) < 0.0 {
            refined = -refined;
        }
        if refined.dot(axis) < 0.5 {
            return Err(DesignError::InvalidMargin);
        }
        let second = self.sector_ridges(centroid, refined, &component, Some(1.5), Some(&points))?;
        let local = self
            .mesh
            .positions
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                component[*i]
                    && (**p - centroid).dot(refined).abs() < 0.5
                    && (**p - centroid).cross(refined).length() <= 8.0
                    && !self.adjacency.is_boundary(*i as u32)
            })
            .map(|(i, _)| i as u32);
        let cost = self.local_cost(local);
        let margin = self
            .connect(&second, &cost)
            .map_err(|_| DesignError::InvalidMargin)?;
        validate_proposal(&margin, center, axis)?;
        Ok(margin)
    }

    fn sector_ridges(
        &self,
        center: DVec3,
        axis: DVec3,
        component: &[bool],
        band: Option<f64>,
        guide: Option<&[DVec3]>,
    ) -> Result<Vec<u32>, DesignError> {
        let u = axis.any_orthonormal_vector();
        let v = axis.cross(u);
        // The scan's axial extent gives a conservative window, leaving its base
        // out; long dies may require the full 12 mm search.
        let extent = self
            .mesh
            .positions
            .iter()
            .enumerate()
            .filter(|(i, p)| component[*i] && (**p - center).cross(axis).length() < 8.0)
            .map(|(_, p)| (center - *p).dot(axis))
            .fold(0.0, f64::max);
        let depth_limit = (0.75 * extent).clamp(6.0, 12.0);
        let mut sectors: [Vec<(u32, f64, f64)>; 36] = std::array::from_fn(|_| Vec::new());
        for (i, &point) in self.mesh.positions.iter().enumerate() {
            if !component[i] || self.adjacency.is_boundary(i as u32) {
                continue;
            }
            if guide.is_some_and(|points| {
                !points
                    .iter()
                    .any(|p| p.distance_squared(point) <= 1.1 * 1.1)
            }) {
                continue;
            }
            let d = point - center;
            let depth = -d.dot(axis);
            let radius = d.cross(axis).length();
            let axial_ok = band.map_or((0.3..=depth_limit).contains(&depth), |b| depth.abs() <= b);
            if !axial_ok || !(0.5..=8.0).contains(&radius) || self.ridge_strength[i] <= 0.0 {
                continue;
            }
            let angle = d.dot(v).atan2(d.dot(u)).rem_euclid(TAU);
            let sector = ((angle / TAU * 36.0) as usize).min(35);
            sectors[sector].push((i as u32, radius, depth));
        }
        let mut choices = Vec::new();
        for (index, sector) in sectors.iter_mut().enumerate() {
            if band.is_none() {
                let angle = TAU * (index as f64 + 0.5) / 36.0;
                let radial = u * angle.cos() + v * angle.sin();
                if let Some((wall_radius, top, bottom)) =
                    self.wall_outline(center, axis, radial, depth_limit)
                {
                    sector.retain(|&(_, r, d)| r <= wall_radius + 1.1 && d >= top && d <= bottom);
                } else {
                    return Err(DesignError::InvalidMargin);
                }
            }
            sector.retain(|&(i, r, d)| {
                let radial = (self.mesh.positions[i as usize] - center + axis * d).normalize();
                self.bvh
                    .raycast(&self.mesh, center - axis * d, radial)
                    .is_some_and(|hit| {
                        let normal = self
                            .mesh
                            .triangle_cross(hit.triangle as usize)
                            .normalize_or_zero();
                        hit.t >= r - 0.15 || (hit.t >= r - 1.1 && normal.dot(axis).abs() > 0.65)
                    })
            });
            let strongest = sector
                .iter()
                .map(|&(i, _, _)| self.ridge_strength[i as usize])
                .fold(0.0, f64::max);
            sector.retain(|&(i, _, _)| self.ridge_strength[i as usize] >= 0.05 * strongest);
            sector.sort_by(|a, b| {
                self.ridge_strength[b.0 as usize].total_cmp(&self.ridge_strength[a.0 as usize])
            });
            let mut peaks: Vec<(u32, f64, f64)> = Vec::new();
            for &p in sector.iter() {
                if peaks.iter().all(|q| (p.1 - q.1).hypot(p.2 - q.2) > 0.35) {
                    peaks.push(p);
                }
                if peaks.len() == 24 {
                    break;
                }
            }
            if peaks.is_empty() {
                return Err(DesignError::InvalidMargin);
            }
            choices.push(peaks);
        }
        // Closed dynamic programming: enumerate the start candidate and include
        // the closing transition. A hard 1 mm bound applies across the seam too.
        let transition = |a: (u32, f64, f64), b: (u32, f64, f64)| {
            let dr = (a.1 - b.1).abs();
            let dz = (a.2 - b.2).abs();
            if dr > 1.0 || dz > 1.0 {
                f64::INFINITY
            } else {
                3.0 * (dr * dr + dz * dz)
            }
        };
        let unary = |sector: usize, p: (u32, f64, f64)| {
            let peak = self.ridge_strength[choices[sector][0].0 as usize];
            -(self.ridge_strength[p.0 as usize] / peak).min(1.0) - 0.6 * p.2
        };
        let mut best = f64::INFINITY;
        let mut ring = Vec::new();
        for start in 0..choices[0].len() {
            let mut scores = vec![f64::INFINITY; choices[0].len()];
            scores[start] = unary(0, choices[0][start]);
            let mut previous = vec![Vec::new(); choices.len()];
            for sector in 1..choices.len() {
                let mut next = vec![f64::INFINITY; choices[sector].len()];
                previous[sector] = vec![0; next.len()];
                for (j, &p) in choices[sector].iter().enumerate() {
                    for (k, &q) in choices[sector - 1].iter().enumerate() {
                        let score = scores[k] + transition(q, p) + unary(sector, p);
                        if score < next[j] {
                            next[j] = score;
                            previous[sector][j] = k;
                        }
                    }
                }
                scores = next;
            }
            let last = choices.len() - 1;
            for (end, &score) in scores.iter().enumerate() {
                let score = score + transition(choices[last][end], choices[0][start]);
                if score < best {
                    best = score;
                    let mut at = end;
                    let mut path = Vec::new();
                    for sector in (0..=last).rev() {
                        path.push(choices[sector][at].0);
                        if sector > 0 {
                            at = previous[sector][at];
                        }
                    }
                    path.reverse();
                    ring = path;
                }
            }
        }
        if ring.is_empty() {
            Err(DesignError::InvalidMargin)
        } else {
            Ok(ring)
        }
    }

    /// Follow the first surface seen from the stump axis down through the wall.
    /// Once the radial profile flares away from its recent wall slope, allow
    /// only the outer millimetre of that turn, not a later gingival crest.
    fn wall_outline(
        &self,
        center: DVec3,
        axis: DVec3,
        radial: DVec3,
        limit: f64,
    ) -> Option<(f64, f64, f64)> {
        let mut wall = Vec::new();
        for step in 1..=(limit / 0.25) as usize {
            let depth = step as f64 * 0.25;
            let Some(hit) = self.bvh.raycast(&self.mesh, center - axis * depth, radial) else {
                continue;
            };
            let normal = self
                .mesh
                .triangle_cross(hit.triangle as usize)
                .normalize_or_zero();
            let r = hit.t;
            if !(0.3..8.0).contains(&r) {
                continue;
            }
            wall.push((depth, r, normal.dot(axis), normal.dot(radial)));
        }
        let start = wall
            .windows(4)
            .position(|w| w.iter().all(|p| p.2.abs() < 0.65 && p.3 > 0.5))?;
        let end = wall[start + 3];
        let mut radius = end.1;
        let mut bottom = limit;
        for j in start + 4..wall.len() {
            let p = wall[j];
            let before = wall[j - 2];
            let earlier = wall[j - 4];
            let slope = ((before.1 - earlier.1) / (before.0 - earlier.0)).clamp(-0.6, 0.6);
            let expected = before.1 + slope * (p.0 - before.0);
            if p.1 - expected > 0.3 {
                radius = before.1;
                bottom = p.0 + 0.5;
                break;
            }
            radius = p.1;
        }
        Some((radius, end.0 + 0.5, bottom))
    }

    /// A barrier loop that follows an existing closed curve on the scan, such as a
    /// stored margin: every point of the curve snaps to the nearest scan vertex —
    /// nothing is re-captured onto ridges — and consecutive distinct vertices are
    /// joined by shortest surface paths. The loop stays within about one triangle
    /// of the curve, wherever later edits moved it, so the region it bounds is
    /// consistent with the curve itself; the returned `points` are the input curve.
    /// The curve must be simple, closed, at least three points long and lie on one
    /// connected component of the mesh.
    pub fn follow(&self, curve: &[DVec3]) -> Result<Margin, DesignError> {
        if curve.len() < 3 {
            return Err(DesignError::InvalidMargin);
        }
        let component = self.component(self.snap(curve[0])?);
        let mut waypoints = Vec::with_capacity(curve.len());
        for &point in curve {
            let v = self.snap(point)?;
            if !component[v as usize] {
                return Err(DesignError::InvalidMargin);
            }
            if waypoints.last() != Some(&v) {
                waypoints.push(v);
            }
        }
        if waypoints.len() > 1 && waypoints[0] == waypoints[waypoints.len() - 1] {
            waypoints.pop();
        }
        let unique: std::collections::HashSet<u32> = waypoints.iter().copied().collect();
        if waypoints.len() < 3 || unique.len() != waypoints.len() {
            return Err(DesignError::InvalidMargin);
        }
        let geodesic = vec![1.0; self.mesh.positions.len()];
        let (vertices, _, _) = self.route(&waypoints, &geodesic)?;
        Ok(Margin {
            points: curve.to_vec(),
            vertices,
        })
    }

    /// Triangles on the seed side of the closed edge barrier. Invalid loops or seeds
    /// on the barrier return an empty region. The seed must be inside the prep.
    pub fn region(&self, margin: &Margin, seed: DVec3) -> Vec<u32> {
        let Ok(seed) = self.snap(seed) else {
            return Vec::new();
        };
        if margin.vertices.len() < 3
            || margin.vertices.contains(&seed)
            || margin
                .vertices
                .iter()
                .any(|&v| v as usize >= self.mesh.positions.len())
            || margin.vertices.iter().enumerate().any(|(i, &v)| {
                !self
                    .adjacency
                    .neighbors(v)
                    .contains(&margin.vertices[(i + 1) % margin.vertices.len()])
            })
        {
            return Vec::new();
        }
        use std::collections::{HashMap, HashSet};
        let unique: HashSet<_> = margin.vertices.iter().copied().collect();
        if unique.len() != margin.vertices.len() {
            return Vec::new();
        }
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        let barrier: HashSet<_> = (0..margin.vertices.len())
            .map(|i| {
                key(
                    margin.vertices[i],
                    margin.vertices[(i + 1) % margin.vertices.len()],
                )
            })
            .collect();
        let mut edges = HashMap::<(u32, u32), Vec<usize>>::new();
        for (i, t) in self.mesh.triangles.iter().enumerate() {
            for j in 0..3 {
                edges.entry(key(t[j], t[(j + 1) % 3])).or_default().push(i);
            }
        }
        if edges.values().any(|faces| faces.len() > 2) {
            return Vec::new();
        }
        let Some(start) = self.mesh.triangles.iter().position(|t| t.contains(&seed)) else {
            return Vec::new();
        };
        // Flood faces across NON-LOOP edges. This also classifies triangles with
        // three barrier corners by their actual side, including zig-zag ears.
        let mut inside = vec![false; self.mesh.triangles.len()];
        let mut stack = vec![start];
        inside[start] = true;
        while let Some(i) = stack.pop() {
            let t = self.mesh.triangles[i];
            for j in 0..3 {
                let edge = key(t[j], t[(j + 1) % 3]);
                if barrier.contains(&edge) {
                    continue;
                }
                for &next in &edges[&edge] {
                    if !inside[next] {
                        inside[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        if !barrier.iter().all(|edge| {
            let faces = &edges[edge];
            faces.len() == 2 && inside[faces[0]] != inside[faces[1]]
        }) {
            return Vec::new();
        }
        inside
            .iter()
            .enumerate()
            .filter(|(_, yes)| **yes)
            .map(|(i, _)| i as u32)
            .collect()
    }
}

/// Attribute a rejected projected edge to its original one-based waypoint leg,
/// including crossings introduced by smoothing or projection back to the scan.
fn validate_trace_curve(
    points: &[DVec3],
    frame: crate::geometry::Frame,
    raw: &[DVec3],
    legs: &[usize],
) -> Result<(), DesignError> {
    if crate::polygon::Polygon::new(points, frame).is_ok() {
        return Ok(());
    }
    let projected: Vec<_> = points.iter().map(|&p| frame.project(p)).collect();
    let scale = projected
        .iter()
        .copied()
        .reduce(glam::DVec2::max)
        .unwrap()
        .distance(projected.iter().copied().reduce(glam::DVec2::min).unwrap());
    let mut offending = 0;
    'edges: for i in 0..points.len() {
        let a = projected[i];
        let b = projected[(i + 1) % points.len()];
        let c = projected[(i + 2) % points.len()];
        if a.distance(b) <= scale * 1e-12
            || ((b - a).perp_dot(c - b).abs() <= scale * scale * 1e-12 && (b - a).dot(c - b) < 0.0)
        {
            offending = i;
            break;
        }
        for j in i + 2..points.len() {
            if (j + 1) % points.len() != i
                && crate::polygon::intersects(
                    a,
                    b,
                    projected[j],
                    projected[(j + 1) % points.len()],
                    scale * scale * 1e-12,
                )
            {
                offending = j;
                break 'edges;
            }
        }
    }
    let point = points[offending].lerp(points[(offending + 1) % points.len()], 0.5);
    let edge = (0..raw.len())
        .min_by(|&a, &b| {
            segment_distance_squared(point, raw[a], raw[(a + 1) % raw.len()]).total_cmp(
                &segment_distance_squared(point, raw[b], raw[(b + 1) % raw.len()]),
            )
        })
        .unwrap();
    Err(DesignError::MarginLeg {
        leg: legs[edge],
        reason: "projected path overlaps or crosses another margin edge",
    })
}

fn segment_distance_squared(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance_squared(a + t * ab)
}

fn validate_proposal(margin: &Margin, seed: DVec3, axis: DVec3) -> Result<(), DesignError> {
    let frame = crate::geometry::Frame::new(seed, axis);
    let polygon = crate::polygon::Polygon::new(&margin.points, frame)?;
    let length: f64 = margin
        .points
        .iter()
        .zip(margin.points.iter().cycle().skip(1))
        .map(|(a, b)| a.distance(*b))
        .sum();
    if !(8.0..=45.0).contains(&length) || !polygon.contains(glam::DVec2::ZERO) {
        return Err(DesignError::InvalidMargin);
    }
    Ok(())
}

fn loop_axis(points: &[DVec3]) -> Result<DVec3, DesignError> {
    let center = points.iter().sum::<DVec3>() / points.len() as f64;
    let mut cov = [[0.0; 3]; 3];
    for p in points {
        let d = *p - center;
        for r in 0..3 {
            for c in 0..3 {
                cov[r][c] += d[r] * d[c];
            }
        }
    }
    let (values, vectors) = crate::orient::eigen3(cov);
    if values[1] <= 1e-12 {
        return Err(DesignError::InvalidMargin);
    }
    Ok(vectors[0])
}

fn resample(points: &[DVec3], spacing: f64) -> Vec<DVec3> {
    let n = points.len();
    let lengths: Vec<_> = (0..n)
        .map(|i| points[i].distance(points[(i + 1) % n]))
        .collect();
    let total: f64 = lengths.iter().sum();
    let count = (total / spacing).ceil().max(3.0) as usize;
    let mut edge = 0;
    let mut start = 0.0;
    (0..count)
        .map(|i| {
            let distance = total * i as f64 / count as f64;
            while edge + 1 < n && start + lengths[edge] < distance {
                start += lengths[edge];
                edge += 1;
            }
            let t = if lengths[edge] > 0.0 {
                (distance - start) / lengths[edge]
            } else {
                0.0
            };
            points[edge].lerp(points[(edge + 1) % n], t)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{FinishLine, ToothParams, prepared_tooth};
    use crate::testutil::Lcg;
    use crate::{InnerSurfaceParams, inner_surface};
    use std::collections::HashSet;

    #[test]
    fn design_boundary_keeps_the_region_barrier_on_a_scalloped_noisy_prep() {
        let mut die = crate::prepared_stump(&crate::StumpParams {
            wall_steps: 8,
            ..Default::default()
        })
        .unwrap();
        let vertices: Vec<_> = die
            .margin
            .iter()
            .map(|p| die.mesh.positions.iter().position(|q| q == p).unwrap() as u32)
            .collect();
        let rotation = glam::DQuat::from_axis_angle(DVec3::new(2., 1., 3.).normalize(), 0.61);
        for p in &mut die.mesh.positions {
            let a = p.y.atan2(p.x);
            p.z += 0.6 * (2. * a).cos() + 0.004 * (17. * a).sin();
            *p = rotation * *p;
        }
        let axis = rotation * DVec3::Z;
        let bvh = Bvh::build(&die.mesh);
        // An editing curve captured further up the wall is still on the scan.
        // Pairing it with the old edge barrier admits a cervical skirt outside
        // that curve, especially where the margin rises between sectors.
        let points = vertices
            .iter()
            .map(|&v| {
                let p = die.mesh.positions[v as usize] + axis * 0.8;
                bvh.closest_point(&die.mesh, p).unwrap().point
            })
            .collect();
        let margin = Margin { points, vertices };
        let boundary = margin.region_boundary(&die.mesh).unwrap();
        assert!(
            crate::margin_overhang(&die.mesh, &die.region, &margin.points, axis)
                .unwrap()
                .depth
                > 0.03
        );
        assert_eq!(
            crate::margin_overhang(&die.mesh, &die.region, &boundary, axis)
                .unwrap()
                .depth,
            0.0
        );
        for (&v, &p) in margin.vertices.iter().zip(&boundary) {
            assert_eq!(p, die.mesh.positions[v as usize]);
        }
        let inner = inner_surface(
            &die.mesh,
            &die.region,
            &boundary,
            axis,
            &InnerSurfaceParams {
                grid_step: 0.08,
                ..Default::default()
            },
        )
        .unwrap();
        for (&v, &p) in inner.boundary.iter().zip(&boundary) {
            assert_eq!(inner.mesh.positions[v as usize], p);
        }
        assert!(
            Margin {
                points: vec![],
                vertices: vec![]
            }
            .region_boundary(&die.mesh)
            .is_err()
        );
        assert!(
            Margin {
                points: vec![],
                vertices: vec![0, 0, 1]
            }
            .region_boundary(&die.mesh)
            .is_err()
        );
        assert!(
            Margin {
                points: vec![],
                vertices: vec![0, 1, u32::MAX]
            }
            .region_boundary(&die.mesh)
            .is_err()
        );
    }

    /// 1024 exact samples of the analytic margin ring: radius 4 mm at height 8 mm.
    fn ring(axis: DVec3) -> Vec<DVec3> {
        let u = axis.any_orthonormal_vector();
        let v = axis.cross(u);
        (0..1024)
            .map(|i| {
                let a = TAU * i as f64 / 1024.0;
                (u * a.cos() + v * a.sin()) * 4.0 + axis * 8.0
            })
            .collect()
    }

    /// Distance from a point to one segment.
    fn segment_distance(p: DVec3, a: DVec3, b: DVec3) -> f64 {
        let ab = b - a;
        let length = ab.length_squared();
        if length == 0.0 {
            return p.distance(a);
        }
        let t = ((p - a).dot(ab) / length).clamp(0.0, 1.0);
        p.distance(a + ab * t)
    }

    /// Sampled one-sided Hausdorff distance: the farthest of `points` from the
    /// closed polyline `curve`, measured to its segments.
    fn distance_to_curve(points: &[DVec3], curve: &[DVec3]) -> f64 {
        let n = curve.len();
        assert!(n >= 3);
        points
            .iter()
            .map(|p| {
                (0..n)
                    .map(|i| segment_distance(*p, curve[i], curve[(i + 1) % n]))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max)
    }

    /// Exact distance from a point to the analytic margin circle.
    fn circle_distance(p: DVec3, axis: DVec3) -> f64 {
        let height = p.dot(axis);
        let radial = (p - axis * height).length();
        ((radial - 4.0).powi(2) + (height - 8.0).powi(2)).sqrt()
    }

    /// The farthest any traced geometry sits from the analytic ring.
    fn worst_ring_distance(points: &[DVec3], axis: DVec3) -> f64 {
        points
            .iter()
            .map(|p| circle_distance(*p, axis))
            .fold(0.0, f64::max)
    }

    fn clicks_near(margin: &[DVec3], count: usize, rng: &mut Lcg) -> Vec<DVec3> {
        let n = margin.len();
        (0..count)
            .map(|k| margin[k * n / count] + rng.unit_vec3() * (0.3 * rng.unit()))
            .collect()
    }

    fn hausdorff(a: &[DVec3], b: &[DVec3]) -> f64 {
        // Sample segment interiors too, in full 3D after an oblique rotation.
        let samples = |p: &[DVec3]| -> Vec<DVec3> {
            (0..p.len())
                .flat_map(|i| [p[i], p[i].lerp(p[(i + 1) % p.len()], 0.5)])
                .collect()
        };
        distance_to_curve(&samples(a), b).max(distance_to_curve(&samples(b), a))
    }

    fn check_proposal(p: ToothParams, tilt: f64, offset: f64, allow_error: bool) {
        let clean = prepared_tooth(&ToothParams {
            noise_mm: 0.0,
            ..p.clone()
        })
        .unwrap();
        let die = prepared_tooth(&p).unwrap();
        let rotation = glam::DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 0.71);
        let mut mesh = die.mesh;
        for v in &mut mesh.positions {
            *v = rotation * *v;
        }
        let expected: Vec<_> = clean.margin.iter().map(|v| rotation * *v).collect();
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        let seed =
            rotation * (die.axis * ((12.0 + p.extra_wall) * p.height_scale) + DVec3::X * offset);
        let axis = rotation * (glam::DQuat::from_rotation_y(tilt.to_radians()) * die.axis);
        let result = tracer.propose(seed, axis);
        if allow_error && result == Err(DesignError::InvalidMargin) {
            eprintln!("tilt {tilt}: clear InvalidMargin rejection");
            return;
        }
        assert!(
            result.is_ok(),
            "proposal {p:?}, tilt {tilt}, offset {offset}: {result:?}"
        );
        let error = hausdorff(&expected, &result.unwrap().points);
        eprintln!("proposal {p:?}, tilt {tilt}, offset {offset}: Hausdorff {error:.6} mm");
        assert!(
            error <= 0.05,
            "proposal Hausdorff {error} mm; {p:?}, tilt {tilt}, offset {offset}"
        );
    }

    #[test]
    fn regression_tall_prep_selects_apical_ridge() {
        check_proposal(
            ToothParams {
                height_scale: 1.6,
                ..Default::default()
            },
            0.0,
            0.0,
            false,
        );
        check_proposal(
            ToothParams {
                extra_wall: 4.0,
                ..Default::default()
            },
            0.0,
            0.0,
            false,
        );
    }

    #[test]
    fn regression_rounded_noisy_margin() {
        for finish in [FinishLine::Shoulder, FinishLine::Chamfer] {
            for smooth_passes in [3, 8] {
                for noise_mm in [0.015, 0.02] {
                    check_proposal(
                        ToothParams {
                            finish,
                            smooth_passes,
                            noise_mm,
                            ..Default::default()
                        },
                        0.0,
                        0.0,
                        false,
                    );
                }
            }
        }
    }

    #[test]
    fn regression_tilted_axis_and_offset_seed() {
        check_proposal(ToothParams::default(), 20.0, 0.0, false);
        check_proposal(ToothParams::default(), 30.0, 0.0, true);
        check_proposal(ToothParams::default(), 0.0, 2.0, false);
    }

    #[test]
    fn regression_gingiva_and_neighbour() {
        check_proposal(
            ToothParams {
                gingiva: true,
                neighbour_gap: 0.4,
                ..Default::default()
            },
            0.0,
            0.0,
            false,
        );
    }

    #[test]
    fn regression_trace_uses_local_ridge_strength() {
        for finish in [FinishLine::Shoulder, FinishLine::Chamfer] {
            let p = ToothParams {
                finish,
                smooth_passes: 8,
                noise_mm: 0.02,
                tilt_deg: 23.0,
                ..Default::default()
            };
            let clean = prepared_tooth(&ToothParams {
                noise_mm: 0.0,
                ..p.clone()
            })
            .unwrap();
            let die = prepared_tooth(&p).unwrap();
            let tracer = MarginTracer::new(Arc::new(die.mesh)).unwrap();
            let clicks: Vec<_> = clean
                .margin
                .iter()
                .step_by(p.segments / 4)
                .copied()
                .collect();
            let margin = tracer.trace(&clicks).unwrap();
            let error = hausdorff(&clean.margin, &margin.points);
            eprintln!("trace {finish:?}: Hausdorff {error:.6} mm");
            assert!(error <= 0.05, "trace Hausdorff {error}");
            // A remote, extremely sharp component must not set ridge sharpness.
            let mut distant = TriMesh::uv_sphere(0.001, 16, 10);
            for v in &mut distant.positions {
                *v += DVec3::splat(100.0);
            }
            let mut augmented = (*tracer.mesh).clone();
            augmented.merge(&distant);
            let other = MarginTracer::new(Arc::new(augmented)).unwrap();
            let other_margin = other.trace(&clicks).unwrap();
            assert!(hausdorff(&margin.points, &other_margin.points) < 1e-6);
        }
    }

    #[test]
    fn regression_click_capture_ignores_boundary() {
        let mut mesh = TriMesh::default();
        mesh.positions.push(DVec3::ZERO);
        for i in 0..16 {
            let angle = TAU * i as f64 / 16.0;
            mesh.positions
                .push(DVec3::new(angle.cos(), angle.sin(), 0.0));
            mesh.triangles.push([0, i + 1, (i + 1) % 16 + 1]);
        }
        let clicks: Vec<_> = [1, 5, 9, 13].map(|i| mesh.positions[i]).to_vec();
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        if let Ok(margin) = tracer.trace(&clicks) {
            assert!(
                margin
                    .vertices
                    .iter()
                    .all(|&i| !tracer.adjacency.is_boundary(i)),
                "captured scan boundary"
            );
        }
    }

    #[test]
    fn regression_zigzag_region_excludes_outside_barrier_triangle() {
        let rotation = glam::DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 0.5).normalize(), 1.2);
        let mut rng = Lcg::new(781);
        let positions = (0..49)
            .map(|i| {
                rotation
                    * DVec3::new(
                        (i % 7) as f64,
                        (i / 7) as f64,
                        0.02 * (2.0 * rng.unit() - 1.0),
                    )
            })
            .collect();
        let mut triangles = Vec::new();
        for y in 0..6 {
            for x in 0..6 {
                let a = y * 7 + x;
                triangles.extend([[a, a + 1, a + 8], [a, a + 8, a + 7]]);
            }
        }
        let mesh = TriMesh::new(positions, triangles);
        let outside = mesh
            .triangles
            .iter()
            .position(|t| *t == [8, 16, 15])
            .unwrap() as u32;
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        let margin = Margin {
            points: vec![],
            vertices: vec![
                8, 9, 10, 11, 12, 19, 26, 33, 40, 39, 38, 37, 36, 29, 22, 15, 16,
            ],
        };
        let region = tracer.region(&margin, rotation * DVec3::new(3.0, 3.0, 0.0));
        assert!(!region.is_empty());
        assert!(
            !region.contains(&outside),
            "outside triangle {outside} has three barrier corners"
        );
        let complement = tracer.region(&margin, DVec3::ZERO);
        assert!(complement.contains(&outside));
        assert_eq!(region.len() + complement.len(), tracer.mesh.triangles.len());
        let loop_points: Vec<_> = margin
            .vertices
            .iter()
            .map(|&i| tracer.mesh.positions[i as usize])
            .collect();
        // Check face interiors and non-barrier edge midpoints, not just corners.
        for (i, _) in tracer.mesh.triangles.iter().enumerate() {
            let [a, b, c] = tracer.mesh.triangle(i);
            for p in [
                (a + b + c) / 3.0,
                (a + b) / 2.0,
                (b + c) / 2.0,
                (c + a) / 2.0,
            ] {
                if distance_to_curve(&[p], &loop_points) < 1e-9 {
                    continue;
                }
                let local = rotation.inverse() * p;
                let expected = local.x > 1.0
                    && local.x < 5.0
                    && local.y > 1.0
                    && local.y < 5.0
                    && !(local.x < 2.0 && local.y < 2.0 && local.y > local.x);
                assert_eq!(
                    region.contains(&(i as u32)),
                    expected,
                    "face {i}, sample {p:?}"
                );
            }
        }
    }

    #[test]
    fn four_imprecise_clicks_trace_the_finish_line() {
        for (finish, tilt) in [
            (FinishLine::Shoulder, 0.0),
            (FinishLine::Shoulder, 12.0),
            (FinishLine::Chamfer, 0.0),
            (FinishLine::Chamfer, 12.0),
        ] {
            let die = prepared_tooth(&ToothParams {
                finish,
                tilt_deg: tilt,
                ..Default::default()
            })
            .unwrap();
            let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
            let mut rng = Lcg::new(101 + tilt as u64 + u64::from(finish == FinishLine::Chamfer));
            for _ in 0..3 {
                let clicks = clicks_near(&die.margin, 4, &mut rng);
                let margin = tracer.trace(&clicks).unwrap();
                let ring = ring(die.axis);
                let to_curve = distance_to_curve(&ring, &margin.points);
                assert!(
                    to_curve <= 0.05,
                    "{finish:?} tilt {tilt}: ring to curve {to_curve}"
                );
                let to_ring = worst_ring_distance(&margin.points, die.axis);
                assert!(
                    to_ring <= 0.05,
                    "{finish:?} tilt {tilt}: curve to ring {to_ring}"
                );
                // The underlying vertex loop sits on the ring: the concave inner
                // corner of the shoulder never attracts the path.
                for &v in &margin.vertices {
                    assert!(
                        circle_distance(die.mesh.positions[v as usize], die.axis) <= 0.05,
                        "{finish:?} tilt {tilt}: vertex {v} off the ring"
                    );
                }
                let spacing = margin
                    .points
                    .windows(2)
                    .map(|w| w[0].distance(w[1]))
                    .chain([margin.points[0].distance(*margin.points.last().unwrap())])
                    .fold(0.0, f64::max);
                assert!(spacing <= 0.105, "{finish:?}: spacing {spacing}");
            }
        }
    }

    #[test]
    fn shoulder_inner_corner_clicks_still_find_the_outer_edge() {
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        let n = die.margin.len();
        // Clicks on the flat ledge, displaced toward the concave inner corner.
        let clicks: Vec<DVec3> = (0..4)
            .map(|k| {
                let m = die.margin[k * n / 4];
                let inward = (m - die.axis * m.dot(die.axis)).normalize();
                m - inward * 0.25
            })
            .collect();
        let margin = tracer.trace(&clicks).unwrap();
        assert!(distance_to_curve(&ring(die.axis), &margin.points) <= 0.05);
        assert!(worst_ring_distance(&margin.points, die.axis) <= 0.05);
    }

    #[test]
    fn regression_margin_validation_and_leg_diagnostics() {
        let rotation = glam::DQuat::from_axis_angle(DVec3::new(2.0, 3.0, 1.0).normalize(), 0.8);
        let axis = rotation * DVec3::Z;
        let circle = |radius| Margin {
            points: (0..64)
                .map(|i| {
                    let a = TAU * i as f64 / 64.0;
                    rotation * DVec3::new(radius * a.cos(), radius * a.sin(), 0.0)
                })
                .collect(),
            vertices: vec![],
        };
        assert!(validate_proposal(&circle(3.0), axis * 5.0, axis).is_ok());
        for radius in [0.5, 8.0] {
            assert_eq!(
                validate_proposal(&circle(radius), DVec3::ZERO, axis),
                Err(DesignError::InvalidMargin)
            );
        }
        assert_eq!(
            validate_proposal(&circle(3.0), rotation * DVec3::X * 5.0, axis),
            Err(DesignError::InvalidMargin)
        );
        let bow = [
            DVec3::ZERO,
            DVec3::new(2.0, 2.0, 0.0),
            DVec3::Y * 2.0,
            DVec3::X * 2.0,
        ]
        .map(|p| rotation * p);
        assert!(matches!(
            validate_trace_curve(
                &bow,
                crate::geometry::Frame::new(DVec3::ZERO, axis),
                &bow,
                &[1, 1, 2, 2]
            ),
            Err(DesignError::MarginLeg { leg: 2, .. })
        ));
    }

    #[test]
    fn regression_trace_without_ridges_and_degenerate_faces() {
        let mut die = prepared_tooth(&ToothParams::default()).unwrap();
        // Open scan base, a duplicate vertex, and a zero-area face must not
        // disrupt a healthy connected surface corridor around the preparation.
        die.mesh
            .triangles
            .retain(|t| t.iter().all(|&v| die.mesh.positions[v as usize].z > 0.1));
        let duplicate = die.mesh.positions.len() as u32;
        die.mesh.positions.push(die.mesh.positions[0]);
        die.mesh.triangles.push([duplicate, duplicate, duplicate]);
        let mut tracer = MarginTracer::new(Arc::new(die.mesh)).unwrap();
        tracer.ridge_strength.fill(0.0);
        let clicks: Vec<_> = die.margin.iter().step_by(32).copied().collect();
        let margin = tracer.trace(&clicks).unwrap();
        validate_proposal(&margin, die.axis * 12.0, die.axis).unwrap();
        for point in &margin.points {
            let hit = tracer.bvh.closest_point(&tracer.mesh, *point).unwrap();
            assert!(hit.point.distance(*point) < 1e-8);
        }
        let repeated = [
            clicks[0], clicks[1], clicks[1], clicks[2], clicks[4], clicks[6],
        ];
        assert!(matches!(
            tracer.trace(&repeated),
            Err(DesignError::MarginLeg { leg: 2, .. })
        ));
    }

    #[test]
    fn regression_full_arch_outer_crest() {
        let mut die = full_arch_prep();
        let rotation = glam::DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 0.71);
        for p in &mut die.mesh.positions {
            *p = rotation * *p;
        }
        for p in &mut die.margin {
            *p = rotation * *p;
        }
        die.axis = rotation * die.axis;
        let tracer = MarginTracer::new(Arc::new(die.mesh)).unwrap();
        let strongest = |lo: f64, hi: f64| {
            tracer
                .mesh
                .positions
                .iter()
                .enumerate()
                .filter(|(_, p)| (lo..hi).contains(&p.dot(die.axis)))
                .map(|(i, _)| tracer.ridge_strength[i])
                .fold(0.0, f64::max)
        };
        assert!(
            strongest(11.5, 12.1) > strongest(7.8, 8.2),
            "fixture top must be stronger than the margin"
        );
        let margin = tracer.propose(die.axis * 12.0, die.axis).unwrap();
        assert!(
            hausdorff(&die.margin, &margin.points) < 0.15,
            "proposal followed the neighbouring crest"
        );
    }

    #[test]
    fn regression_full_arch_manual_corridors() {
        let die = full_arch_prep();
        let mut tracer = MarginTracer::new(Arc::new(die.mesh)).unwrap();
        // Prescribed evidence isolates routing from curvature estimation: a weak
        // margin quadrant and a strong connected neighbour reached along two
        // radial creases. Distances and all paths still use the generated mesh.
        for (i, p) in tracer.mesh.positions.iter().enumerate() {
            let r = p.truncate().length();
            let angle = p.y.atan2(p.x).rem_euclid(TAU);
            let on_margin = (r - 4.0).hypot(p.z - 8.0) < 0.08;
            let spoke = angle < 0.06 || (angle - TAU / 4.0).abs() < 0.06;
            tracer.ridge_strength[i] = if on_margin {
                if (0.15..TAU / 4.0 - 0.15).contains(&angle) {
                    0.05
                } else {
                    0.2
                }
            } else if r > 6.8 || (spoke && p.z < 7.91) {
                1.0
            } else {
                0.0
            };
        }
        let clicks: Vec<_> = die.margin.iter().step_by(64).copied().collect();
        let waypoints: Vec<_> = clicks.iter().map(|&p| tracer.snap(p).unwrap()).collect();
        let local = tracer
            .mesh
            .positions
            .iter()
            .enumerate()
            .filter(|(_, p)| clicks.iter().any(|c| c.distance(**p) < 0.45))
            .map(|(i, _)| i as u32);
        let costs = tracer.local_cost(local);
        let unbounded =
            dentcraft_mesh::path_through(&tracer.mesh, &tracer.adjacency, &waypoints, true, |v| {
                costs[v as usize]
            })
            .unwrap();
        let old_points: Vec<_> = unbounded
            .vertices
            .iter()
            .map(|&v| tracer.mesh.positions[v as usize])
            .collect();
        let excursion = distance_to_curve(&old_points, &die.margin);
        eprintln!("unbounded excursion {excursion}");
        assert!(
            excursion > 0.5,
            "fixture must attract an unconstrained path to the neighbour"
        );
        let margin = tracer.trace(&clicks).unwrap();
        assert!(hausdorff(&die.margin, &margin.points) < 0.15);
    }

    fn full_arch_prep() -> crate::PreparedStump {
        let mut die = prepared_tooth(&ToothParams {
            smooth_passes: 8,
            noise_mm: 0.01,
            ..Default::default()
        })
        .unwrap();
        // A connected gingival band, with a sharper, lower crest inside the old
        // 8 mm search annulus. Keep the tooth and its finish line untouched.
        for p in &mut die.mesh.positions {
            let r = p.truncate().length();
            if p.z > 11.5 && r > 1.0 {
                // A narrow rounded top edge has stronger evidence than the
                // smoothed margin, as on a worn or irregular occlusal table.
                let wall_r = 3.1 - 3.3 * 6.0_f64.to_radians().tan();
                let new_r = wall_r + (r - wall_r) * 0.16;
                p.x *= new_r / r;
                p.y *= new_r / r;
                p.z = 12.0 - (12.0 - p.z) * 0.16;
            }
            if p.z < 7.5 && r > 4.0 {
                let new_r = if p.z > 6.8 {
                    4.03 + (7.5 - p.z) / 0.7 * 3.0
                } else {
                    7.03
                };
                p.x *= new_r / r;
                p.y *= new_r / r;
            }
        }
        die
    }

    #[test]
    fn propose_from_an_occlusal_seed_finds_the_margin() {
        for (finish, tilt) in [
            (FinishLine::Shoulder, 0.0),
            (FinishLine::Chamfer, 0.0),
            (FinishLine::Shoulder, 12.0),
        ] {
            let die = prepared_tooth(&ToothParams {
                finish,
                tilt_deg: tilt,
                ..Default::default()
            })
            .unwrap();
            let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
            let margin = tracer.propose(die.axis * 12.0, die.axis).unwrap();
            let to_curve = distance_to_curve(&ring(die.axis), &margin.points);
            assert!(to_curve <= 0.1, "{finish:?} tilt {tilt}: {to_curve}");
            let to_ring = worst_ring_distance(&margin.points, die.axis);
            assert!(to_ring <= 0.1, "{finish:?} tilt {tilt}: {to_ring}");
            assert!(margin.vertices.len() >= 27);
            assert_eq!(
                tracer.propose(die.axis * 12.0, DVec3::ZERO),
                Err(DesignError::InvalidAxis)
            );
            assert_eq!(
                tracer.propose(die.axis * 12.0, DVec3::NAN),
                Err(DesignError::InvalidAxis)
            );
        }
    }

    #[test]
    fn region_is_the_preparation_and_excludes_the_base() {
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        let margin = tracer.propose(die.axis * 12.0, die.axis).unwrap();
        let prep: HashSet<u32> = die.region.iter().copied().collect();
        let region = tracer.region(&margin, die.axis * 12.0);
        assert!(!region.is_empty());
        for &t in &region {
            assert!(prep.contains(&t), "triangle {t} is outside the prep");
        }
        let covered = region.iter().filter(|t| prep.contains(*t)).count();
        assert!(
            covered as f64 >= 0.98 * die.region.len() as f64,
            "{covered} of {}",
            die.region.len()
        );
        // A seed on the base plate floods the other side of the margin barrier.
        let outside = tracer.region(&margin, DVec3::ZERO);
        assert!(!outside.is_empty());
        assert!(outside.iter().all(|t| !prep.contains(t)));
    }

    #[test]
    fn invalid_clicks_and_meshes_are_rejected() {
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        assert_eq!(
            tracer.trace(&die.margin[..2]),
            Err(DesignError::InvalidMargin)
        );
        // One click on a disconnected island: no path can connect the loop.
        let mut split = die.mesh.clone();
        let island = TriMesh::new(
            vec![
                DVec3::splat(50.0),
                DVec3::new(51.0, 50.0, 50.0),
                DVec3::new(50.0, 51.0, 50.0),
                DVec3::new(50.0, 50.0, 51.0),
            ],
            vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
        );
        split.merge(&island);
        let split_tracer = MarginTracer::new(Arc::new(split)).unwrap();
        let mut clicks = clicks_near(&die.margin, 4, &mut Lcg::new(5));
        clicks.push(DVec3::splat(50.0));
        assert_eq!(split_tracer.trace(&clicks), Err(DesignError::InvalidMargin));
        // Degenerate tracer inputs.
        assert!(matches!(
            MarginTracer::new(Arc::new(TriMesh::default())),
            Err(DesignError::EmptyRegion)
        ));
        let mut nonfinite = die.mesh.clone();
        nonfinite.positions[0] = DVec3::NAN;
        assert!(matches!(
            MarginTracer::new(Arc::new(nonfinite)),
            Err(DesignError::InvalidMesh)
        ));
        let collinear = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::new(2.0, 0.0, 0.0)],
            vec![[0, 1, 2]],
        );
        assert!(matches!(
            MarginTracer::new(Arc::new(collinear)),
            Err(DesignError::DegenerateRegion)
        ));
    }

    #[test]
    fn tracer_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MarginTracer>();
    }

    #[test]
    fn inner_surface_boundary_is_the_proposed_margin() {
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        let margin = tracer.propose(die.axis * 12.0, die.axis).unwrap();
        let region = tracer.region(&margin, die.axis * 12.0);
        let params = InnerSurfaceParams {
            grid_step: 0.05,
            ..Default::default()
        };
        let inner = inner_surface(&die.mesh, &region, &margin.points, die.axis, &params).unwrap();
        assert_eq!(inner.boundary.len(), margin.points.len());
        for (&b, &point) in inner.boundary.iter().zip(&margin.points) {
            assert_eq!(inner.mesh.positions[b as usize], point);
        }
    }

    #[test]
    #[ignore]
    fn perf_margin_propose() {
        let die = prepared_tooth(&ToothParams {
            segments: 512,
            wall_steps: 200,
            ..Default::default()
        })
        .unwrap();
        assert!(die.mesh.triangle_count() >= 300_000);
        let start = std::time::Instant::now();
        let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        let margin = tracer.propose(die.axis * 12.0, die.axis).unwrap();
        let elapsed = start.elapsed();
        println!(
            "MarginTracer::new + propose on {} triangles: {elapsed:?}",
            die.mesh.triangle_count()
        );
        assert!(!margin.points.is_empty());
        assert!(elapsed.as_secs_f64() < 1.0, "took {elapsed:?}");
    }
}
