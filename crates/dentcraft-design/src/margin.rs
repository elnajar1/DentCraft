//! Convex-ridge margin tracing. Inputs must have outward consistent winding.
use crate::DesignError;
use dentcraft_mesh::{Bvh, TriMesh, VertexAdjacency, smooth_polyline_on_surface};
use glam::DVec3;
use std::f64::consts::TAU;
use std::sync::Arc;

#[path = "margin_smooth.rs"]
mod smooth;

/// How far a followed curve may stray from a scan vertex before coming back to
/// it and still count as noise around that vertex rather than touching itself.
const FOLLOW_EXCURSION_MM: f64 = 0.5;

/// Angular sectors of a proposal's ridge search (10 degrees each).
const SECTORS: usize = 36;
/// Sectors a proposal may find no ridge in, in total, and the most sector
/// steps between two sectors with ridges that the ring bridges (a run of k
/// empty sectors is k + 1 steps). Next to a neighbouring preparation the scan
/// is often ragged or shows the neighbour's wall instead of this stump's.
const MAX_EMPTY_SECTORS: usize = 12;
const MAX_SECTOR_GAP: usize = 7;
/// Waypoints a proposal may drop from its ridge ring when a leg cannot be
/// routed, and the fewest it keeps.
const MAX_DROPPED_WAYPOINTS: usize = 6;
const MIN_RING_WAYPOINTS: usize = 12;
/// What counts as stump wall in a proposal's first pass: the largest
/// |normal . axis| and the smallest outward |normal . radial| of the wall hits.
/// A proposal first asks for a steep wall, as on intraoral scans. When that
/// fails it retries with a tapered wall: on a stone model the incisors'
/// lingual and proximal faces slope 30-60 degrees in the arch's occlusal frame,
/// and an off-centre seed sees the far wall of a molar obliquely.
const STEEP_WALL: (f64, f64) = (0.65, 0.5);
const TAPERED_WALL: (f64, f64) = (0.9, 0.3);

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

    /// Join a detected ring of ridge waypoints. A leg that finds no surface path
    /// in its corridor, or crosses another leg, starts or ends at a waypoint the
    /// ring took from a fold or a frayed scan edge: drop the leg's end waypoint,
    /// or its start when the same start fails again, and retry a few times.
    /// Unlike clicked waypoints, ring waypoints are only evidence.
    fn connect_ring(&self, ring: &[u32], cost: &[f64]) -> Result<Margin, DesignError> {
        let mut ring = ring.to_vec();
        let mut failed_start = None;
        let mut dropped = 0;
        loop {
            match self.connect(&ring, cost) {
                Err(DesignError::MarginLeg { leg, .. })
                    if dropped < MAX_DROPPED_WAYPOINTS && ring.len() > MIN_RING_WAYPOINTS =>
                {
                    let start = ring[leg - 1];
                    let index = if failed_start == Some(start) {
                        leg - 1
                    } else {
                        leg % ring.len()
                    };
                    failed_start = Some(start);
                    ring.remove(index);
                    dropped += 1;
                }
                result => return result,
            }
        }
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
    /// Sectors without ridge evidence (a ragged scan or a neighbouring
    /// preparation) are bridged if few and narrow enough.
    /// The final projected loop must be simple, enclose the seed and measure
    /// 8–45 mm. When no steep stump wall bounds the search, the search is
    /// retried accepting tapered walls (see `TAPERED_WALL`). When both fail,
    /// both are retried from the top of the stump (`stump_top`) instead of the
    /// clicked point: a click on a face of the stump, off its top, leaves the
    /// seed's axis line near one wall. A loop found from the stump top must
    /// also enclose the click, so a click on the gum or low on a wall never
    /// returns a neighbour's margin. Failures return the first pass's
    /// `MarginDetect`, naming the stage that gave up.
    pub fn propose(&self, seed: DVec3, axis: DVec3) -> Result<Margin, DesignError> {
        self.propose_pass(seed, axis).map(|(margin, _)| margin)
    }

    /// `propose`, also returning which pass found the margin: 0 is the
    /// steep-wall pass from the click, 1 the tapered-wall pass from the click,
    /// 2 and 3 the steep- and tapered-wall passes from the stump top. A pass
    /// above 0 is a fallback: its loop can sit above the finish line.
    pub fn propose_pass(&self, seed: DVec3, axis: DVec3) -> Result<(Margin, usize), DesignError> {
        if !axis.is_finite() || axis.length_squared() < 1e-24 {
            return Err(DesignError::InvalidAxis);
        }
        let axis = axis.normalize();
        let seed_vertex = self.snap(seed)?;
        // Exclude disconnected fragments. Connected neighbouring teeth are
        // excluded separately by the wall outline and radial visibility tests.
        let component = self.component(seed_vertex);
        let clicked = self.mesh.positions[seed_vertex as usize];
        let attempts = [
            (STEEP_WALL, false),
            (TAPERED_WALL, false),
            (STEEP_WALL, true),
            (TAPERED_WALL, true),
        ];
        let mut top = None;
        let mut first_error = None;
        for (pass, (wall, recentre)) in attempts.into_iter().enumerate() {
            let center = if recentre {
                let top = *top.get_or_insert_with(|| self.stump_top(seed_vertex, axis, &component));
                if top.distance_squared(clicked) < 0.1 * 0.1 {
                    // Same centre as the passes before: nothing new to try.
                    break;
                }
                top
            } else {
                clicked
            };
            let enclose = recentre.then_some(clicked);
            match self.propose_with_wall(center, enclose, axis, &component, wall) {
                Err(error @ DesignError::MarginDetect { .. }) => {
                    first_error.get_or_insert(error);
                }
                result => return result.map(|margin| (margin, pass)),
            }
        }
        Err(first_error.expect("the first attempt always runs"))
    }

    /// One detection pass around `center`, a surface point of `component`
    /// (`axis` normalized). `enclose`, when set, is a point the loop must also
    /// enclose: the click, when `center` is not the click.
    fn propose_with_wall(
        &self,
        center: DVec3,
        enclose: Option<DVec3>,
        axis: DVec3,
        component: &[bool],
        wall: (f64, f64),
    ) -> Result<Margin, DesignError> {
        let first = self.sector_ridges(center, axis, component, None, None, wall)?;
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
        let first_margin =
            self.connect_ring(&first, &first_cost)
                .map_err(|_| DesignError::MarginDetect {
                    stage: "no surface route joins the first ring of ridges",
                })?;
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
            return Err(DesignError::MarginDetect {
                stage: "the first ring is not flat enough to refine the axis",
            });
        }
        let mut refined = vectors[0];
        if refined.dot(axis) < 0.0 {
            refined = -refined;
        }
        if refined.dot(axis) < 0.5 {
            return Err(DesignError::MarginDetect {
                stage: "the first ring tilts more than 60 degrees from the axis",
            });
        }
        let second =
            self.sector_ridges(centroid, refined, component, Some(1.5), Some(&points), wall)?;
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
            .connect_ring(&second, &cost)
            .map_err(|_| DesignError::MarginDetect {
                stage: "no surface route joins the refined ring of ridges",
            })?;
        validate_proposal(&margin, center, axis).map_err(|_| DesignError::MarginDetect {
            stage: "the margin is not a simple 8-45 mm loop around the seed",
        })?;
        let encloses = |m: &Margin| enclose.is_none_or(|p| validate_proposal(m, p, axis).is_ok());
        if !encloses(&margin) {
            return Err(DesignError::MarginDetect {
                stage: "the margin found from the stump top does not enclose the click",
            });
        }
        Ok(self
            .smoothed(&margin, center, axis)
            .filter(|m| encloses(m))
            .unwrap_or(margin))
    }

    /// The top of the stump under a click anywhere on it: the occlusal-most
    /// point reachable from the click without descending more than 0.5 mm,
    /// within 5 mm (then up the stump above it, when it stands higher), then
    /// the surface point above the centroid of the top-down outline of the
    /// stump's top 2 mm (projected-area weighted). Only the click's own
    /// component counts. Falls back to the clicked vertex when no such point is
    /// found.
    fn stump_top(&self, seed: u32, axis: DVec3, component: &[bool]) -> DVec3 {
        let positions = &self.mesh.positions;
        let clicked = positions[seed as usize];
        let flood = |start: u32, keep: &dyn Fn(DVec3) -> bool| {
            let mut inside = vec![false; positions.len()];
            let mut stack = vec![start];
            inside[start as usize] = true;
            while let Some(i) = stack.pop() {
                for &j in self.adjacency.neighbors(i) {
                    if !inside[j as usize] && keep(positions[j as usize]) {
                        inside[j as usize] = true;
                        stack.push(j);
                    }
                }
            }
            inside
        };
        let height = |p: DVec3| (p - clicked).dot(axis);
        // The highest vertex of `inside`; of equally high ones, the nearest to
        // the click (a flat top or plane is its own top).
        let highest = |inside: &[bool]| {
            (0..positions.len())
                .filter(|&i| inside[i])
                .max_by(|&a, &b| {
                    let (pa, pb) = (positions[a], positions[b]);
                    height(pa).total_cmp(&height(pb)).then_with(|| {
                        pb.distance_squared(clicked)
                            .total_cmp(&pa.distance_squared(clicked))
                    })
                })
        };
        let reach = flood(seed, &|p| {
            height(p) >= -0.5 && p.distance_squared(clicked) <= 5.0 * 5.0
        });
        let Some(mut top) = highest(&reach) else {
            return clicked;
        };
        let cap_of = |top: usize| {
            let top_point = positions[top];
            flood(top as u32, &|p| {
                let d = p - top_point;
                d.dot(axis) >= -2.0 && d.reject_from_normalized(axis).length_squared() <= 7.0 * 7.0
            })
        };
        let mut cap = cap_of(top);
        // A stump standing more than 5 mm above the click: the flood stopped
        // on its wall, and the cap above that point reaches the real top.
        if let Some(summit) = highest(&cap)
            && height(positions[summit]) > height(positions[top]) + 0.5
        {
            top = summit;
            cap = cap_of(top);
        }
        let top_point = positions[top];
        let (mut area, mut moment) = (0.0, DVec3::ZERO);
        for (t, triangle) in self.mesh.triangles.iter().enumerate() {
            if triangle.iter().all(|&v| cap[v as usize]) {
                let projected = self.mesh.triangle_cross(t).dot(axis).abs() * 0.5;
                let centroid = triangle
                    .iter()
                    .map(|&v| positions[v as usize])
                    .sum::<DVec3>()
                    / 3.0;
                area += projected;
                moment += centroid * projected;
            }
        }
        if area < 1e-9 {
            return clicked;
        }
        let mut centroid = moment / area;
        // Refine with the centroid of the stump's cross-section 2.5 mm below
        // its top, seen from the current centre: the cap of a proclined or
        // bevelled incisor lies to one side of the stump below it.
        let u = axis.any_orthonormal_vector();
        let v = axis.cross(u);
        for _ in 0..3 {
            let origin = centroid + axis * ((top_point - centroid).dot(axis) - 2.5);
            let hits: Vec<DVec3> = (0..SECTORS)
                .filter_map(|k| {
                    let angle = TAU * (k as f64 + 0.5) / SECTORS as f64;
                    let radial = u * angle.cos() + v * angle.sin();
                    let hit = self.bvh.raycast(&self.mesh, origin, radial)?;
                    let normal = self.mesh.triangle_cross(hit.triangle as usize);
                    let own = self.mesh.triangles[hit.triangle as usize]
                        .iter()
                        .all(|&v| component[v as usize]);
                    (hit.t < 8.0 && normal.dot(radial) > 0.0 && own).then_some(hit.point)
                })
                .collect();
            if hits.len() < SECTORS * 3 / 4 {
                break;
            }
            centroid = hits.iter().sum::<DVec3>() / hits.len() as f64;
        }
        let above = centroid + axis * ((top_point - centroid).dot(axis) + 1.0);
        match self.bvh.raycast(&self.mesh, above, -axis) {
            Some(hit)
                if hit.t <= 1.0 + 2.5
                    && self.mesh.triangles[hit.triangle as usize]
                        .iter()
                        .all(|&v| component[v as usize]) =>
            {
                hit.point
            }
            _ => clicked,
        }
    }

    /// The detected loop as a smooth surface curve: low-pass filtered in arc
    /// length and projected back onto the scan, with its edge loop re-routed
    /// along the new curve by `follow`. `None` (keep the detected loop) when the
    /// smoothed curve cannot be followed or is no longer a valid proposal.
    fn smoothed(&self, margin: &Margin, seed: DVec3, axis: DVec3) -> Option<Margin> {
        let points = smooth::low_pass_on_surface(&self.mesh, &self.bvh, &margin.points, axis)?;
        let smoothed = self.follow(&points).ok()?;
        validate_proposal(&smoothed, seed, axis).ok()?;
        Some(smoothed)
    }

    fn sector_ridges(
        &self,
        center: DVec3,
        axis: DVec3,
        component: &[bool],
        band: Option<f64>,
        guide: Option<&[DVec3]>,
        wall: (f64, f64),
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
        let mut sectors: [Vec<(u32, f64, f64)>; SECTORS] = std::array::from_fn(|_| Vec::new());
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
            let sector = ((angle / TAU * SECTORS as f64) as usize).min(SECTORS - 1);
            sectors[sector].push((i as u32, radius, depth));
        }
        let mut choices = Vec::new();
        for (index, sector) in sectors.iter_mut().enumerate() {
            if band.is_none() {
                let angle = TAU * (index as f64 + 0.5) / SECTORS as f64;
                let radial = u * angle.cos() + v * angle.sin();
                if let Some((wall_radius, top, bottom)) =
                    self.wall_outline(center, axis, radial, depth_limit, wall)
                {
                    sector.retain(|&(_, r, d)| r <= wall_radius + 1.1 && d >= top && d <= bottom);
                } else {
                    // No stump wall seen in this direction: a ragged gap or a
                    // neighbouring preparation. The ring bridges it below.
                    sector.clear();
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
            if !peaks.is_empty() {
                choices.push((index, peaks));
            }
        }
        join_ring(&choices, |i| self.ridge_strength[i as usize])
    }

    /// Follow the first surface seen from the stump axis down through the wall,
    /// which starts at four consecutive hits within the `wall` bounds.
    /// Once the radial profile flares away from its recent wall slope, allow
    /// only the outer millimetre of that turn, not a later gingival crest.
    fn wall_outline(
        &self,
        center: DVec3,
        axis: DVec3,
        radial: DVec3,
        limit: f64,
        (max_axial, min_radial): (f64, f64),
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
            .position(|w| w.iter().all(|p| p.2.abs() < max_axial && p.3 > min_radial))?;
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
        for (i, &point) in curve.iter().enumerate() {
            let v = self.snap(point)?;
            if !component[v as usize] {
                return Err(DesignError::InvalidMargin);
            }
            // A noisy curve sampled finer than the scan's triangles can wander off a
            // vertex and back (A, B, A): drop such a short excursion. A curve that
            // returns to a vertex from further away touches itself and is rejected.
            // Back at the first vertex for good, with the rest of the curve staying
            // on it or close to it, the loop closes; a return to it mid-curve is
            // like any other.
            if let Some(k) = waypoints.iter().position(|&w| w == v) {
                let at = self.mesh.positions[v as usize];
                if k == 0
                    && curve[i + 1..].iter().all(|&p| {
                        p.distance(at) <= FOLLOW_EXCURSION_MM || self.snap(p).is_ok_and(|w| w == v)
                    })
                {
                    continue;
                }
                if waypoints[k..]
                    .iter()
                    .any(|&w| self.mesh.positions[w as usize].distance(at) > FOLLOW_EXCURSION_MM)
                {
                    return Err(DesignError::InvalidMargin);
                }
                waypoints.truncate(k + 1);
            } else {
                waypoints.push(v);
            }
        }
        if waypoints.len() < 3 {
            return Err(DesignError::InvalidMargin);
        }
        // Consecutive snapped vertices are often only one edge apart, and every
        // other waypoint blocks the path; where the scan's triangles do not run
        // along the curve, a leg's narrow corridor can then be closed. Retry with
        // waypoints thinned to a minimum spacing, which leaves each leg room
        // while the geodesic between them stays within a chord's sagitta of the
        // curve.
        let geodesic = vec![1.0; self.mesh.positions.len()];
        let mut result = self.route(&waypoints, &geodesic);
        for spacing in [0.3, 0.6, 1.0] {
            if result.is_ok() {
                break;
            }
            let thinned = self.thin(&waypoints, spacing);
            if thinned.len() >= 3 {
                result = self.route(&thinned, &geodesic);
            }
        }
        let (vertices, _, _) = result?;
        Ok(Margin {
            points: curve.to_vec(),
            vertices,
        })
    }

    /// Keeps the first waypoint and then every waypoint at least `spacing` mm
    /// from the last one kept (and from the first, closing the loop).
    fn thin(&self, waypoints: &[u32], spacing: f64) -> Vec<u32> {
        let at = |v: u32| self.mesh.positions[v as usize];
        let mut kept = vec![waypoints[0]];
        for &v in &waypoints[1..] {
            if at(v).distance(at(*kept.last().unwrap())) >= spacing
                && at(v).distance(at(waypoints[0])) >= spacing
            {
                kept.push(v);
            }
        }
        kept
    }

    /// Triangles on the seed side of the closed edge barrier, flooded from the
    /// scan face nearest the seed, so a seed beside the margin on a coarse scan
    /// starts on its own side even when its nearest vertex is on the loop.
    ///
    /// Scans of plaster models have holes and loose sheets. A barrier edge on a
    /// hole (one face) blocks nothing and is accepted; across a non-manifold
    /// edge (more than two faces) the flood continues only onto faces that run
    /// the edge in the opposite direction, the consistently wound continuation
    /// of the surface. Invalid loops, a seed off the scan and a loop that does
    /// not bound the flood return an empty region: the flood reaches both sides
    /// of a barrier edge (the loop does not separate the scan), or it reaches no
    /// barrier edge at all. The seed must be inside the prep.
    pub fn region(&self, margin: &Margin, seed: DVec3) -> Vec<u32> {
        if !seed.is_finite()
            || margin.vertices.len() < 3
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
        let Some(start) = self.bvh.closest_point(&self.mesh, seed) else {
            return Vec::new();
        };
        let start = start.triangle as usize;
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
        // Faces of each undirected edge, with whether the face runs it from the
        // lower to the higher vertex index.
        let mut edges = HashMap::<(u32, u32), Vec<(usize, bool)>>::new();
        for (i, t) in self.mesh.triangles.iter().enumerate() {
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                edges.entry(key(a, b)).or_default().push((i, a < b));
            }
        }
        // Flood faces across NON-LOOP edges. This also classifies triangles with
        // three barrier corners by their actual side, including zig-zag ears.
        let mut inside = vec![false; self.mesh.triangles.len()];
        let mut stack = vec![start];
        inside[start] = true;
        while let Some(i) = stack.pop() {
            let t = self.mesh.triangles[i];
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                let edge = key(a, b);
                if barrier.contains(&edge) {
                    continue;
                }
                if let Some(next) = self.across(i, a < b, &edges[&edge])
                    && !inside[next]
                {
                    inside[next] = true;
                    stack.push(next);
                }
            }
        }
        let mut bounded = false;
        let mut leaks = false;
        for edge in &barrier {
            let faces = &edges[edge];
            let reached =
                |forward: bool| faces.iter().any(|&(face, f)| inside[face] && f == forward);
            leaks |= if faces.len() == 2 {
                inside[faces[0].0] && inside[faces[1].0]
            } else {
                reached(true) && reached(false)
            };
            bounded |= faces.iter().any(|&(face, _)| inside[face]);
        }
        if !bounded {
            return Vec::new();
        }
        if leaks {
            match self.split_at_leak(margin, &barrier, &edges, start) {
                Some(side) => inside = side,
                None => return Vec::new(),
            }
        }
        let region: Vec<u32> = inside
            .iter()
            .enumerate()
            .filter(|(_, yes)| **yes)
            .map(|(i, _)| i as u32)
            .collect();
        // The loop's vector area is the largest area of its projection on a
        // plane: the area of the disc it spans.
        let n = margin.vertices.len();
        let corner = |k: usize| self.mesh.positions[margin.vertices[k % n] as usize];
        let disc = (0..n)
            .map(|i| corner(i).cross(corner(i + 1)))
            .sum::<DVec3>()
            .length()
            / 2.0;
        let area: f64 = region
            .iter()
            .map(|&f| {
                let [a, b, c] = self.mesh.triangle(f as usize);
                (b - a).cross(c - a).length() / 2.0
            })
            .sum();
        if !(area >= REGION_AREA_MIN * disc && area <= REGION_AREA_MAX * disc) {
            return Vec::new();
        }
        region
    }

    /// The face the flood crosses to from face `from` over one of its edges,
    /// given whether `from` runs that edge from the lower to the higher vertex
    /// index, and the edge's faces. A manifold edge leads to its other face. On
    /// an edge with more than two faces only those running the edge the other
    /// way continue the consistently wound surface, and of these the one that
    /// turns least from `from` is the surface itself: a loose flap of scan
    /// hanging off the edge turns away from it.
    fn across(&self, from: usize, forward: bool, faces: &[(usize, bool)]) -> Option<usize> {
        if faces.len() <= 2 {
            return faces.iter().map(|&(f, _)| f).find(|&f| f != from);
        }
        let normal = |f: usize| {
            let [a, b, c] = self.mesh.triangle(f);
            (b - a).cross(c - a).normalize_or_zero()
        };
        let here = normal(from);
        let mut best: Option<(usize, f64)> = None;
        for &(f, fw) in faces {
            if f == from || fw == forward {
                continue;
            }
            let alignment = here.dot(normal(f));
            if best.is_none_or(|(_, b)| alignment > b) {
                best = Some((f, alignment));
            }
        }
        best.map(|(f, _)| f)
    }

    /// The flood from inside a margin loop reached its far side: the scan has a
    /// handle through the loop, such as neighbouring surfaces fused into a
    /// bridge. Grow both sides of the loop at once, by surface distance between
    /// face centroids, and give each face to the side that reaches it first, so
    /// the handle is cut where the two fronts meet. Returns the faces on the
    /// start face's side, or None when that cut is not short beside the loop:
    /// then the loop itself does not separate the scan (it runs around a tube).
    fn split_at_leak(
        &self,
        margin: &Margin,
        barrier: &std::collections::HashSet<(u32, u32)>,
        edges: &std::collections::HashMap<(u32, u32), Vec<(usize, bool)>>,
        start: usize,
    ) -> Option<Vec<bool>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let key = |a: u32, b: u32| (a.min(b), a.max(b));
        let centroid = |f: usize| self.mesh.triangle(f).iter().sum::<DVec3>() / 3.0;
        // Side 1 runs the loop edges forward (the loop's left), side 2 backward.
        let mut side = vec![0u8; self.mesh.triangles.len()];
        let mut dist = vec![f64::INFINITY; side.len()];
        let mut heap = BinaryHeap::new();
        let n = margin.vertices.len();
        let mut loop_length = 0.0;
        for i in 0..n {
            let (a, b) = (margin.vertices[i], margin.vertices[(i + 1) % n]);
            loop_length +=
                self.mesh.positions[a as usize].distance(self.mesh.positions[b as usize]);
            for &(face, forward) in &edges[&key(a, b)] {
                if side[face] == 0 {
                    side[face] = if forward == (a < b) { 1 } else { 2 };
                    dist[face] = 0.0;
                    heap.push(Reverse((0u64, face)));
                }
            }
        }
        while let Some(Reverse((bits, i))) = heap.pop() {
            if f64::from_bits(bits) > dist[i] {
                continue;
            }
            let t = self.mesh.triangles[i];
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                let edge = key(a, b);
                if barrier.contains(&edge) {
                    continue;
                }
                let Some(next) = self.across(i, a < b, &edges[&edge]) else {
                    continue;
                };
                let d = dist[i] + centroid(i).distance(centroid(next));
                if d < dist[next] {
                    dist[next] = d;
                    side[next] = side[i];
                    heap.push(Reverse((d.to_bits(), next)));
                }
            }
        }
        // Length of the surface edges between the two sides, off the loop.
        let cut: f64 = edges
            .iter()
            .filter(|(edge, faces)| {
                !barrier.contains(edge)
                    && faces.iter().any(|&(f, fw)| {
                        side[f] != 0
                            && self
                                .across(f, fw, faces)
                                .is_some_and(|g| side[g] != 0 && side[g] != side[f])
                    })
            })
            .map(|(&(a, b), _)| {
                self.mesh.positions[a as usize].distance(self.mesh.positions[b as usize])
            })
            .sum();
        if side[start] == 0 || cut > LEAK_CUT_FRACTION * loop_length {
            return None;
        }
        Some(side.iter().map(|&s| s == side[start]).collect())
    }
}

/// The longest cut, as a fraction of the margin loop's length, that
/// `MarginTracer::region` makes through a handle of the scan that joins the two
/// sides of the loop.
const LEAK_CUT_FRACTION: f64 = 0.25;

/// The bounds of a plausible prep region's area, as multiples of the area of
/// the disc its margin loop spans (see `MarginTracer::region`).
const REGION_AREA_MIN: f64 = 0.8;
const REGION_AREA_MAX: f64 = 6.0;

/// A sector's ridge candidate: scan vertex, radius from the stump axis and
/// depth below the seed, both in mm.
type RingCandidate = (u32, f64, f64);

/// The largest radius change the ring accepts between two members one sector
/// step apart, and the extra it allows per further step across a gap. A gap of
/// g steps allows 1 + 0.3 (g - 1) mm, not g mm: a stump's outline bends slowly,
/// and a step onto a neighbouring stump's wall is a jump of several mm however
/// many sectors it is spread over.
const RING_STEP_RADIUS_MM: f64 = 1.0;
const RING_GAP_RADIUS_MM: f64 = 0.3;
/// Cost of leaving a non-empty sector out of the ring. It outweighs the best
/// unary reward of a member, so a ring keeps a sector unless its transitions
/// cost several mm² more than bridging it.
const RING_SKIP_COST: f64 = 4.0;

/// Join per-sector ridge candidates into the cheapest closed ring by dynamic
/// programming. `choices` lists the non-empty sectors in increasing order with
/// their candidates; `strength` gives a candidate's ridge strength. Returns the
/// ring's vertices in sector order.
///
/// A few sectors may lack evidence as long as no gap spans more than
/// MAX_SECTOR_GAP sector steps (a run of k empty sectors is k + 1 steps): the
/// ring bridges the gap and the surface route follows the scan across it. A
/// sector holding only a neighbouring stump's ridges breaks the radius bound,
/// so the ring may also skip non-empty sectors, at RING_SKIP_COST each, as long
/// as no gap exceeds MAX_SECTOR_GAP steps and the ring keeps evidence from all
/// but MAX_EMPTY_SECTORS sectors. Rings rank by total cost, skips included.
fn join_ring(
    choices: &[(usize, Vec<RingCandidate>)],
    strength: impl Fn(u32) -> f64,
) -> Result<Vec<u32>, DesignError> {
    if choices.len() + MAX_EMPTY_SECTORS < SECTORS {
        return Err(DesignError::MarginDetect {
            stage: "too few directions around the seed show a margin ridge",
        });
    }
    let gap =
        |from: usize, to: usize| (choices[to].0 + SECTORS - choices[from].0 - 1) % SECTORS + 1;
    if (0..choices.len()).any(|k| gap(k, (k + 1) % choices.len()) > MAX_SECTOR_GAP) {
        return Err(DesignError::MarginDetect {
            stage: "a wide arc around the seed shows no margin ridge",
        });
    }
    // The radius and depth bounds apply across the seam too. The cost keeps the
    // per-step slope penalty.
    let transition = |steps: usize, a: RingCandidate, b: RingCandidate| {
        let dr = (a.1 - b.1).abs();
        let dz = (a.2 - b.2).abs();
        let steps = steps as f64;
        if dr > RING_STEP_RADIUS_MM + RING_GAP_RADIUS_MM * (steps - 1.0) || dz > steps {
            f64::INFINITY
        } else {
            3.0 * (dr * dr + dz * dz) / steps
        }
    };
    let peaks: Vec<f64> = choices
        .iter()
        .map(|c| c.1.iter().map(|p| strength(p.0)).fold(0.0, f64::max))
        .collect();
    let unary = |sector: usize, p: RingCandidate| {
        let peak = peaks[sector].max(f64::MIN_POSITIVE);
        -(strength(p.0) / peak).min(1.0) - 0.6 * p.2
    };
    let n = choices.len();
    let max_skips = n - (SECTORS - MAX_EMPTY_SECTORS);
    let skip_cost = |skipped: usize| RING_SKIP_COST * skipped as f64;
    let mut best = f64::INFINITY;
    let mut ring = Vec::new();
    // Every valid ring includes a sector below MAX_SECTOR_GAP: otherwise the
    // step from its last sector across the seam would be too long. Starting at
    // choice s skips the s choices before it.
    for s in (0..n).take_while(|&s| choices[s].0 < MAX_SECTOR_GAP) {
        for start in 0..choices[s].1.len() {
            // scores[k][j]: (skips, cost) of the cheapest chain from the start
            // ending at candidate j of choice k; previous[k][j] its predecessor.
            let mut scores: Vec<Vec<(usize, f64)>> = choices
                .iter()
                .map(|c| vec![(usize::MAX, f64::INFINITY); c.1.len()])
                .collect();
            let mut previous = vec![Vec::new(); n];
            scores[s][start] = (s, skip_cost(s) + unary(s, choices[s].1[start]));
            for k in s + 1..n {
                previous[k] = vec![(0, 0); choices[k].1.len()];
                for from in (s..k).rev() {
                    let steps = gap(from, k);
                    if steps > MAX_SECTOR_GAP {
                        break;
                    }
                    let skipped = k - from - 1;
                    for (j, &p) in choices[k].1.iter().enumerate() {
                        for (i, &q) in choices[from].1.iter().enumerate() {
                            let before = scores[from][i];
                            if before.0 == usize::MAX {
                                continue;
                            }
                            let cost = before.1
                                + skip_cost(skipped)
                                + transition(steps, q, p)
                                + unary(k, p);
                            if cost < scores[k][j].1 {
                                scores[k][j] = (before.0 + skipped, cost);
                                previous[k][j] = (from, i);
                            }
                        }
                    }
                }
            }
            for last in s + 1..n {
                let steps = gap(last, s);
                if steps > MAX_SECTOR_GAP {
                    continue;
                }
                for (end, &(skips, cost)) in scores[last].iter().enumerate() {
                    if skips == usize::MAX {
                        continue;
                    }
                    let skips = skips + (n - 1 - last);
                    let cost = cost
                        + skip_cost(n - 1 - last)
                        + transition(steps, choices[last].1[end], choices[s].1[start]);
                    if skips <= max_skips && cost < best {
                        best = cost;
                        let (mut k, mut at) = (last, end);
                        let mut path = vec![choices[k].1[at].0];
                        while k != s {
                            (k, at) = previous[k][at];
                            path.push(choices[k].1[at].0);
                        }
                        path.reverse();
                        ring = path;
                    }
                }
            }
        }
    }
    if ring.is_empty() {
        Err(DesignError::MarginDetect {
            stage: "the ridges around the seed do not join into a ring",
        })
    } else {
        Ok(ring)
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
        if allow_error && matches!(result, Err(DesignError::MarginDetect { .. })) {
            eprintln!("tilt {tilt}: clear MarginDetect rejection");
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

    /// A click on the axial wall 2 mm below the top, as on a stone die whose
    /// top the user missed: the seed's axis line runs beside the wall, and the
    /// proposal must re-centre on the stump top to find the margin.
    #[test]
    fn propose_from_a_click_on_the_stump_wall() {
        for (finish, gingiva) in [(FinishLine::Shoulder, true), (FinishLine::Chamfer, false)] {
            let p = ToothParams {
                finish,
                gingiva,
                smooth_passes: 2,
                noise_mm: 0.02,
                ..Default::default()
            };
            let clean = prepared_tooth(&ToothParams {
                noise_mm: 0.0,
                ..p.clone()
            })
            .unwrap();
            let die = prepared_tooth(&p).unwrap();
            let rotation =
                glam::DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 0.71);
            let mut mesh = die.mesh;
            for v in &mut mesh.positions {
                *v = rotation * *v;
            }
            let expected: Vec<_> = clean.margin.iter().map(|v| rotation * *v).collect();
            let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
            // Wall radius is about 2.9 mm at height 10: the click snaps onto
            // the wall, not the top disc at height 12.
            let seed = rotation * (die.axis * 10.0 + DVec3::new(3.2, 0.4, 0.0));
            let axis = rotation * die.axis;
            let result = tracer.propose_pass(seed, axis);
            assert!(result.is_ok(), "{finish:?}: {result:?}");
            let (margin, pass) = result.unwrap();
            // Only a pass from the stump top finds it, and says so.
            assert!(pass >= 2, "{finish:?}: found by pass {pass}");
            let error = hausdorff(&expected, &margin.points);
            assert!(error <= 0.05, "{finish:?}: Hausdorff {error} mm");
        }
    }

    /// A 0.1 mm heightfield `z(x, y)` over `[x0, x1] x [y0, y1]`, outward (+Z)
    /// winding.
    fn heightfield(x: (f64, f64), y: (f64, f64), z: impl Fn(f64, f64) -> f64) -> TriMesh {
        let nx = ((x.1 - x.0) / 0.1).round() as u32;
        let ny = ((y.1 - y.0) / 0.1).round() as u32;
        let mut positions = Vec::new();
        for j in 0..=ny {
            for i in 0..=nx {
                let (px, py) = (x.0 + 0.1 * i as f64, y.0 + 0.1 * j as f64);
                positions.push(DVec3::new(px, py, z(px, py)));
            }
        }
        let index = |i: u32, j: u32| j * (nx + 1) + i;
        let mut triangles = Vec::new();
        for j in 0..ny {
            for i in 0..nx {
                let (a, b) = (index(i, j), index(i + 1, j));
                let (c, d) = (index(i + 1, j + 1), index(i, j + 1));
                triangles.push([a, b, c]);
                triangles.push([a, c, d]);
            }
        }
        TriMesh::new(positions, triangles)
    }

    /// Height of a stump centred on the Z axis at radial distance `rho`: an
    /// occlusal table at `top` to radius 2.2, a steep wall down to a shoulder
    /// whose outer edge, the margin, is the circle of radius 3.6 near height
    /// 8.2, then a drop to the gingiva at 5.5.
    fn stump_height(rho: f64, top: f64) -> f64 {
        if rho <= 2.2 {
            top
        } else if rho <= 3.0 {
            top - (top - 8.6) * (rho - 2.2) / 0.8
        } else if rho <= 3.55 {
            8.6 - 0.3 * (rho - 3.0) / 0.55
        } else {
            (8.3 - 2.5 * (rho - 3.55)).max(5.5)
        }
    }

    /// A prepared stump at the origin (top 12) beside a taller one at x = 8.2
    /// (top 15) on one connected gingiva sheet, as a premolar prep beside the
    /// neighbour's higher proximal face.
    fn prep_beside_a_taller_neighbour() -> MarginTracer {
        let neighbour = DVec3::new(8.2, 0.0, 0.0);
        let mesh = heightfield((-7.0, 15.2), (-7.0, 7.0), |x, y| {
            let p = DVec3::new(x, y, 0.0);
            stump_height(p.length(), 12.0).max(stump_height((p - neighbour).length(), 15.0))
        });
        MarginTracer::new(Arc::new(mesh)).unwrap()
    }

    /// Re-centring a click on the stump top must not carry it onto the taller
    /// neighbour: from the gum or the low wall near it, the flood up to the
    /// highest point within 5 mm ends on the neighbour's wall. The proposal is
    /// then an error or the clicked stump's own margin, never the neighbour's.
    #[test]
    fn a_click_beside_a_taller_neighbour_never_returns_the_neighbours_margin() {
        let tracer = prep_beside_a_taller_neighbour();
        let neighbour = DVec3::new(8.2, 0.0, 0.0);
        // Both clicks are nearer the neighbour's tall wall than the prep's top.
        for click in [DVec3::new(5.0, 4.2, 5.5), DVec3::new(4.9, 3.6, 5.5)] {
            match tracer.propose_pass(click, DVec3::Z) {
                Err(DesignError::MarginDetect { .. }) => {}
                Ok((margin, pass)) => {
                    let centre = margin.points.iter().sum::<DVec3>() / margin.points.len() as f64;
                    assert!(
                        centre.truncate().length() < 1.0,
                        "click {click}: pass {pass} returned the loop around {centre}, \
                         {:.1} mm from the neighbour",
                        (centre - neighbour).truncate().length()
                    );
                }
                Err(e) => panic!("click {click}: {e}"),
            }
        }
        // The prep itself is still found from its top, by the first pass.
        let (margin, pass) = tracer
            .propose_pass(DVec3::new(0.0, 0.0, 12.0), DVec3::Z)
            .unwrap();
        assert_eq!(pass, 0);
        let worst = margin
            .points
            .iter()
            .map(|p| (p.truncate().length() - 3.6).abs())
            .fold(0.0, f64::max);
        assert!(worst < 0.3, "prep margin strays {worst} mm");
    }

    fn top_of(tracer: &MarginTracer, click: DVec3, axis: DVec3) -> DVec3 {
        let seed = tracer.snap(click).unwrap();
        tracer.stump_top(seed, axis, &tracer.component(seed))
    }

    #[test]
    fn stump_top_on_a_flat_plane_is_the_click() {
        let tracer = MarginTracer::new(Arc::new(heightfield(
            (-10.0, 10.0),
            (-10.0, 10.0),
            |_, _| 0.0,
        )))
        .unwrap();
        let click = DVec3::new(0.3, -0.2, 0.0);
        let top = top_of(&tracer, click, DVec3::Z);
        assert!(top.distance(click) < 0.1, "flat plane: top {top}");
    }

    #[test]
    fn stump_top_of_a_click_on_the_top_is_the_click() {
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let tracer = MarginTracer::new(Arc::new(die.mesh)).unwrap();
        let click = die.axis * 12.0;
        let top = top_of(&tracer, click, die.axis);
        assert!(top.distance(click) < 0.1, "top {top}");
    }

    #[test]
    fn stump_top_of_a_stump_more_than_5_mm_above_the_click() {
        // Top at height 16, click on the wall at 9: the 5 mm flood stops on the
        // wall, below the top.
        let die = prepared_tooth(&ToothParams {
            extra_wall: 4.0,
            ..Default::default()
        })
        .unwrap();
        let tracer = MarginTracer::new(Arc::new(die.mesh)).unwrap();
        let click = die.axis * 9.0 + DVec3::new(3.3, 0.4, 0.0);
        let seed = tracer.snap(click).unwrap();
        let clicked = tracer.mesh.positions[seed as usize];
        assert!(16.0 - clicked.dot(die.axis) > 5.0, "click {clicked}");
        let top = top_of(&tracer, click, die.axis);
        let lateral = top.reject_from_normalized(die.axis).length();
        assert!(lateral < 0.3, "top {top} is {lateral} mm off the axis");
        assert!((top.dot(die.axis) - 16.0).abs() < 0.3, "top {top}");
    }

    #[test]
    fn stump_top_of_a_click_on_a_detached_fragment_stays_on_it() {
        // A loose 2 x 2 mm sheet floating 1 mm over the stump top, off its
        // axis: the stump below must neither re-centre it nor be returned.
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let mut mesh = die.mesh;
        let base = mesh.positions.len() as u32;
        let sheet = heightfield((0.5, 2.5), (-1.0, 1.0), |_, _| 13.0);
        mesh.positions.extend(&sheet.positions);
        mesh.triangles.extend(
            sheet
                .triangles
                .iter()
                .map(|t| [t[0] + base, t[1] + base, t[2] + base]),
        );
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        let centre = DVec3::new(1.5, 0.0, 13.0);
        for click in [DVec3::new(0.7, -0.8, 13.0), centre] {
            let seed = tracer.snap(click).unwrap();
            let component = tracer.component(seed);
            let top = tracer.stump_top(seed, DVec3::Z, &component);
            let on = tracer.snap(top).unwrap();
            assert!(
                component[on as usize],
                "click {click}: top {top} left the sheet"
            );
            assert!(top.distance(centre) < 0.2, "click {click}: top {top}");
        }
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
        // A seed on the base plate floods the other side of the margin barrier,
        // the base and gum: far larger than a prep inside that margin could be.
        assert!(tracer.region(&margin, DVec3::ZERO).is_empty());
    }

    #[test]
    fn region_survives_scan_holes_and_loose_sheets_at_the_margin() {
        // Plaster-model scans have holes and loose sheets (non-manifold edges),
        // also right at the margin, and a seed beside the margin on a coarse
        // scan can have a loop vertex as its nearest vertex.
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let clean = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        let margin = clean.propose(die.axis * 12.0, die.axis).unwrap();
        let reference: HashSet<u32> = clean.region(&margin, die.axis * 12.0).into_iter().collect();
        assert!(!reference.is_empty());
        let mesh = &die.mesh;
        let n = margin.vertices.len();
        let faces_of_loop_edge = |i: usize| -> Vec<usize> {
            let (a, b) = (margin.vertices[i], margin.vertices[(i + 1) % n]);
            (0..mesh.triangles.len())
                .filter(|&f| mesh.triangles[f].contains(&a) && mesh.triangles[f].contains(&b))
                .collect()
        };
        let side = |i: usize, inside: bool| {
            let faces = faces_of_loop_edge(i);
            assert_eq!(faces.len(), 2);
            *faces
                .iter()
                .find(|&&f| reference.contains(&(f as u32)) == inside)
                .unwrap()
        };
        // Holes: one loop edge loses its outside face, another its inside face.
        let dropped = [side(0, false), side(n / 3, true)];
        // Loose sheets: a fin on a loop edge, one on an edge just inside the
        // margin and one on the base far below it.
        let fin_on_loop = (margin.vertices[2 * n / 3], margin.vertices[2 * n / 3 + 1]);
        let near = margin.vertices[n / 2];
        let fin_inside = (0..mesh.triangles.len())
            .filter(|&f| reference.contains(&(f as u32)) && mesh.triangles[f].contains(&near))
            .flat_map(|f| mesh.triangles[f])
            .find(|w| !margin.vertices.contains(w))
            .map(|w| (near, w))
            .unwrap();
        let base = (0..mesh.triangles.len())
            .min_by(|&a, &b| {
                let low = |f: usize| mesh.triangle(f).iter().sum::<DVec3>().dot(die.axis);
                low(a).total_cmp(&low(b))
            })
            .unwrap();
        let fin_base = (mesh.triangles[base][0], mesh.triangles[base][1]);
        let mut positions = mesh.positions.clone();
        let mut triangles = Vec::new();
        let mut original = Vec::new();
        for (f, t) in mesh.triangles.iter().enumerate() {
            if !dropped.contains(&f) {
                triangles.push(*t);
                original.push(Some(f as u32));
            }
        }
        let fins = triangles.len();
        for (a, b) in [fin_on_loop, fin_inside, fin_base] {
            let pa = positions[a as usize];
            let pb = positions[b as usize];
            positions.push((pa + pb) / 2.0 + 0.4 * die.axis.any_orthonormal_vector());
            triangles.push([a, b, positions.len() as u32 - 1]);
            original.push(None);
        }
        let tracer = MarginTracer::new(Arc::new(TriMesh::new(positions, triangles))).unwrap();
        // Beside loop vertex n/4 on an inside face: that vertex is its nearest.
        let v = margin.vertices[n / 4];
        let seed_face = (0..mesh.triangles.len())
            .find(|&f| {
                reference.contains(&(f as u32))
                    && mesh.triangles[f].contains(&v)
                    && !dropped.contains(&f)
            })
            .unwrap();
        let seed = mesh.triangles[seed_face]
            .iter()
            .map(|&w| mesh.positions[w as usize] * if w == v { 0.9 } else { 0.05 })
            .sum::<DVec3>();
        let region = tracer.region(&margin, seed);
        assert!(!region.is_empty(), "no region on the damaged scan");
        let mut found = HashSet::new();
        for &f in &region {
            match original[f as usize] {
                Some(old) => {
                    assert!(reference.contains(&old), "face {old} is outside the margin");
                    found.insert(old);
                }
                None => assert_eq!(f as usize, fins + 1, "fin {f} is not inside the margin"),
            }
        }
        for &f in &reference {
            assert!(
                found.contains(&f) || dropped.contains(&(f as usize)),
                "prep face {f} is missing"
            );
        }
    }

    /// A planar grid of unit cells, `cols` by `rows` vertices, minus the cells
    /// listed, as positions and triangles.
    fn holed_grid(cols: u32, rows: u32, holes: &[(u32, u32)]) -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let positions = (0..cols * rows)
            .map(|i| DVec3::new((i % cols) as f64, (i / cols) as f64, 0.0))
            .collect();
        let mut triangles = Vec::new();
        for y in 0..rows - 1 {
            for x in 0..cols - 1 {
                if !holes.contains(&(x, y)) {
                    let a = y * cols + x;
                    triangles.extend([[a, a + 1, a + cols + 1], [a, a + cols + 1, a + cols]]);
                }
            }
        }
        (positions, triangles)
    }

    #[test]
    fn region_cuts_a_thin_handle_joining_the_two_sides_of_the_margin() {
        // A scan whose surfaces fuse into a thin tube from inside the margin to
        // outside it: the loop no longer separates the scan, but only through
        // the tube, which the region cuts.
        use std::f64::consts::PI;
        let cols = 21;
        let (mut positions, mut triangles) = holed_grid(cols, cols, &[(9, 9), (9, 1)]);
        let plane = triangles.len();
        let square = |x: u32, y: u32| [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)];
        let mut rings: Vec<[u32; 4]> = vec![square(9, 9).map(|(x, y)| y * cols + x)];
        let segments = 6;
        for k in 1..segments {
            let t = k as f64 / segments as f64;
            rings.push(square(9, 9).map(|(x, y)| {
                let p = DVec3::new(x as f64, y as f64 - 8.0 * t, 3.0 * (t * PI).sin());
                positions.push(p);
                positions.len() as u32 - 1
            }));
        }
        rings.push(square(9, 1).map(|(x, y)| y * cols + x));
        for pair in rings.windows(2) {
            let (r, s) = (pair[0], pair[1]);
            for i in 0..4 {
                let j = (i + 1) % 4;
                triangles.extend([[r[i], r[j], s[j]], [r[i], s[j], s[i]]]);
            }
        }
        let mut vertices: Vec<u32> = (5..15).map(|x| 5 * cols + x).collect();
        vertices.extend((5..15).map(|y| y * cols + 15));
        vertices.extend((6..=15).rev().map(|x| 15 * cols + x));
        vertices.extend((6..=15).rev().map(|y| y * cols + 5));
        let margin = Margin {
            points: vec![],
            vertices,
        };
        let mesh = TriMesh::new(positions, triangles);
        let tracer = MarginTracer::new(Arc::new(mesh.clone())).unwrap();
        let region: HashSet<u32> = tracer
            .region(&margin, DVec3::new(12.3, 12.6, 0.0))
            .into_iter()
            .collect();
        assert!(!region.is_empty(), "the handle emptied the region");
        for f in 0..plane {
            let c = mesh.triangle(f).iter().sum::<DVec3>() / 3.0;
            let inside = (5.0..15.0).contains(&c.x) && (5.0..15.0).contains(&c.y);
            assert_eq!(
                region.contains(&(f as u32)),
                inside,
                "plane face {f} at {c}"
            );
        }
    }

    #[test]
    fn region_rejects_a_loop_that_does_not_separate_the_scan() {
        // A loop around the tube of a torus bounds nothing: its two sides are
        // one surface, joined everywhere along the far side of the ring.
        let (around, tube) = (24u32, 12u32);
        let positions = (0..around * tube)
            .map(|i| {
                let u = TAU * (i / tube) as f64 / around as f64;
                let v = TAU * (i % tube) as f64 / tube as f64;
                let r = 10.0 + 3.0 * v.cos();
                DVec3::new(r * u.cos(), r * u.sin(), 3.0 * v.sin())
            })
            .collect();
        let mut triangles = Vec::new();
        for i in 0..around {
            for j in 0..tube {
                let at = |i: u32, j: u32| (i % around) * tube + j % tube;
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                triangles.extend([[a, b, c], [a, c, d]]);
            }
        }
        let tracer = MarginTracer::new(Arc::new(TriMesh::new(positions, triangles))).unwrap();
        let margin = Margin {
            points: vec![],
            vertices: (0..tube).collect(),
        };
        assert!(
            tracer
                .region(&margin, DVec3::new(-13.0, 0.0, 0.0))
                .is_empty()
        );
    }

    /// The vertices of a `holed_grid` with `cols` columns around the square
    /// from (x0, y0) to (x1, y1), as a margin loop.
    fn square_loop(cols: u32, (x0, y0): (u32, u32), (x1, y1): (u32, u32)) -> Margin {
        let mut vertices: Vec<u32> = (x0..x1).map(|x| y0 * cols + x).collect();
        vertices.extend((y0..y1).map(|y| y * cols + x1));
        vertices.extend((x0 + 1..=x1).rev().map(|x| y1 * cols + x));
        vertices.extend((y0 + 1..=y1).rev().map(|y| y * cols + x0));
        Margin {
            points: vec![],
            vertices,
        }
    }

    #[test]
    fn region_excludes_a_scan_flap_hanging_from_the_wall_over_the_ditch() {
        // A loose flap of scan attached to the prep wall just above the margin
        // and hanging out and down past it, over the ditch: its edge on the
        // wall has three faces, and the flap is not the wall's continuation
        // whichever way it is wound. A zero-area sliver on the same edge is
        // not either.
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let clean = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        let margin = clean.propose(die.axis * 12.0, die.axis).unwrap();
        let reference: HashSet<u32> = clean.region(&margin, die.axis * 12.0).into_iter().collect();
        assert!(!reference.is_empty());
        let mesh = &die.mesh;
        let below = mesh.positions[margin.vertices[0] as usize];
        let wall = (0..mesh.triangles.len())
            .filter(|&f| {
                reference.contains(&(f as u32))
                    && mesh.triangles[f]
                        .iter()
                        .all(|v| !margin.vertices.contains(v))
            })
            .min_by(|&a, &b| {
                let gap = |f: usize| (mesh.triangle(f).iter().sum::<DVec3>() / 3.0).distance(below);
                gap(a).total_cmp(&gap(b))
            })
            .unwrap();
        let [p, q, _] = mesh.triangles[wall];
        let (pp, pq) = (mesh.positions[p as usize], mesh.positions[q as usize]);
        let mid = (pp + pq) / 2.0;
        let out = (mid - die.axis * mid.dot(die.axis)).normalize();
        let step = 0.6 * out - 0.6 * die.axis;
        assert!(
            (mid + 2.0 * step - below).dot(die.axis) < -0.3,
            "the flap does not hang past the margin"
        );
        for flipped in [false, true] {
            let mut positions = mesh.positions.clone();
            let mut triangles = mesh.triangles.clone();
            let flap = triangles.len();
            let mut add = |p: DVec3| {
                positions.push(p);
                positions.len() as u32 - 1
            };
            let (p1, q1) = (add(pp + step), add(pq + step));
            let (p2, q2) = (add(pp + 2.0 * step), add(pq + 2.0 * step));
            let sliver = add(mid);
            for mut t in [
                [q, p, p1],
                [q, p1, q1],
                [q1, p1, p2],
                [q1, p2, q2],
                [q, p, sliver],
            ] {
                if flipped {
                    t.swap(0, 1);
                }
                triangles.push(t);
            }
            let tracer = MarginTracer::new(Arc::new(TriMesh::new(positions, triangles))).unwrap();
            let region: HashSet<u32> = tracer
                .region(&margin, die.axis * 12.0)
                .into_iter()
                .collect();
            for f in flap..flap + 5 {
                assert!(
                    !region.contains(&(f as u32)),
                    "flap face {f} (flipped {flipped}) is in the region"
                );
            }
            assert_eq!(region, reference, "flipped {flipped}");
        }
    }

    #[test]
    fn region_rejects_a_cut_through_a_neighbour_beyond_a_short_handle() {
        // A short handle from just inside the margin to a neighbour that is
        // close in space but far round the scan from the margin: a strip of
        // the scan folds back over the prep in a hairpin, like a neighbouring
        // tooth across a narrow gap. Grown at once, the inside of the loop
        // reaches the neighbour through the handle before the outside comes
        // round the fold, so the two meet at the fold, not in the handle. That
        // cut is longer than a handle's beside the loop, so the region is
        // rejected rather than taking the neighbour.
        use std::f64::consts::PI;
        let (cols, rows, fold, lap) = (31u32, 15u32, 14u32, 3u32);
        let height = 2.0;
        let (hx, hy) = (11u32, 7u32);
        let top = 2 * fold + lap - 1 - hx;
        let mut holes = vec![(hx, hy), (top, hy)];
        holes.extend((fold..cols).flat_map(|x| (0..3).chain(12..rows).map(move |y| (x, y))));
        let (mut positions, mut triangles) = holed_grid(cols, rows, &holes);
        // Columns past `fold` turn over a half circle and run back on top.
        for p in &mut positions {
            let x = p.x;
            let (f, l) = (fold as f64, lap as f64);
            *p = if x <= f {
                *p
            } else if x < f + l {
                let a = PI * (x - f) / l;
                DVec3::new(
                    f + height / 2.0 * a.sin(),
                    p.y,
                    height / 2.0 * (1.0 - a.cos()),
                )
            } else {
                DVec3::new(f - (x - f - l), p.y, height)
            };
        }
        let grid = triangles.len();
        let at = |x: u32, y: u32| y * cols + x;
        let lower = [
            at(hx, hy),
            at(hx + 1, hy),
            at(hx + 1, hy + 1),
            at(hx, hy + 1),
        ];
        let upper = [
            at(top + 1, hy),
            at(top, hy),
            at(top, hy + 1),
            at(top + 1, hy + 1),
        ];
        let middle = lower.map(|v| {
            positions.push(positions[v as usize] + DVec3::Z * height / 2.0);
            positions.len() as u32 - 1
        });
        for (r, s) in [(lower, middle), (middle, upper)] {
            for i in 0..4 {
                let j = (i + 1) % 4;
                triangles.extend([[r[i], r[j], s[j]], [r[i], s[j], s[i]]]);
            }
        }
        let mesh = TriMesh::new(positions, triangles);
        let tracer = MarginTracer::new(Arc::new(mesh.clone())).unwrap();
        let margin = square_loop(cols, (2, 2), (12, 12));
        let region = tracer.region(&margin, DVec3::new(6.3, 6.6, 0.0));
        for &f in &region {
            let c = mesh.triangle(f as usize).iter().sum::<DVec3>() / 3.0;
            let inside = c.z < 1e-9 && (2.0..12.0).contains(&c.x) && (2.0..12.0).contains(&c.y);
            assert!(
                inside || f as usize >= grid,
                "face {f} at {c} outside the margin is in the region"
            );
        }
    }

    #[test]
    fn region_rejects_a_lone_flap_on_the_margin_as_the_start() {
        // A seed on a loose flap standing on a loop edge floods only that flap:
        // it touches the loop and reaches one side of it, but it is no prep.
        let cols = 21;
        let (mut positions, mut triangles) = holed_grid(cols, cols, &[]);
        let margin = square_loop(cols, (5, 5), (15, 15));
        let (a, b) = (margin.vertices[3], margin.vertices[4]);
        positions.push((positions[a as usize] + positions[b as usize]) / 2.0 + DVec3::Z * 0.8);
        triangles.push([a, b, positions.len() as u32 - 1]);
        let tracer = MarginTracer::new(Arc::new(TriMesh::new(positions, triangles))).unwrap();
        let tip = tracer.mesh.positions[tracer.mesh.positions.len() - 1];
        assert!(tracer.region(&margin, tip - DVec3::Z * 0.1).is_empty());
        assert_eq!(tracer.region(&margin, DVec3::new(9.3, 9.6, 0.0)).len(), 200);
    }

    #[test]
    fn region_rejects_an_implausible_area_beside_the_margin_disc() {
        // A prep's area is a little over the disc its margin spans (a short
        // wide molar) to a few times it (a tall narrow incisor stump). The far
        // side of a small loop on a large scan is far more, and a region with
        // most of it missing is far less.
        let outside = |cells: u32| {
            let (positions, triangles) = holed_grid(cells + 1, cells + 1, &[]);
            let tracer = MarginTracer::new(Arc::new(TriMesh::new(positions, triangles))).unwrap();
            let margin = square_loop(cells + 1, (3, 3), (7, 7));
            assert_eq!(tracer.region(&margin, DVec3::new(5.3, 5.6, 0.0)).len(), 32);
            tracer.region(&margin, DVec3::new(0.3, 0.6, 0.0))
        };
        // 84 and 105 square cells beside a disc of 16.
        assert_eq!(outside(10).len(), 2 * 84);
        assert!(outside(11).is_empty());
        let holed = |missing: u32| {
            let holes: Vec<_> = (0..missing).map(|i| (8 + i % 5, 8 + i / 5)).collect();
            let (positions, triangles) = holed_grid(21, 21, &holes);
            let tracer = MarginTracer::new(Arc::new(TriMesh::new(positions, triangles))).unwrap();
            tracer.region(
                &square_loop(21, (5, 5), (15, 15)),
                DVec3::new(5.3, 5.6, 0.0),
            )
        };
        // 85 and 75 square cells inside a disc of 100.
        assert_eq!(holed(15).len(), 2 * 85);
        assert!(holed(25).is_empty());
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
    fn follow_drops_short_excursions_of_a_noisy_stored_curve() {
        // A tilted, noisy prep with its gum and a neighbour. The stored curve is
        // the proposed loop, but every seventh vertex it wanders onto an adjacent
        // scan vertex and back, as a smoothed 0.1 mm curve does across triangles
        // larger than its spacing; it also passes the start vertex once more.
        // The curve used to be rejected as touching itself, which left the
        // Insertion axis step without a region on real full-arch scans.
        let die = prepared_tooth(&ToothParams {
            tilt_deg: 17.0,
            noise_mm: 0.02,
            gingiva: true,
            ..Default::default()
        })
        .unwrap();
        let mesh = Arc::new(die.mesh.clone());
        let tracer = MarginTracer::new(mesh.clone()).unwrap();
        let seed = die.axis * 12.0;
        let margin = tracer.propose(seed, die.axis).unwrap();
        let reference = tracer.region(&tracer.follow(&margin.points).unwrap(), seed);
        assert!(!reference.is_empty());
        let on_loop: HashSet<u32> = margin.vertices.iter().copied().collect();
        let mut curve = Vec::new();
        let mut excursions = 0;
        for (i, &v) in margin.vertices.iter().enumerate() {
            let at = mesh.positions[v as usize];
            curve.push(at);
            let aside = mesh
                .triangles
                .iter()
                .filter(|t| t.contains(&v))
                .flatten()
                .copied()
                .find(|&w| !on_loop.contains(&w) && mesh.positions[w as usize].distance(at) < 0.4);
            if let (0, Some(w)) = (i % 7, aside) {
                curve.push(mesh.positions[w as usize]);
                curve.push(at);
                excursions += 1;
            }
        }
        curve.push(curve[0]);
        assert!(excursions >= 5, "{excursions} excursions");
        let followed = tracer.follow(&curve).unwrap();
        assert_eq!(followed.points, curve);
        let region = tracer.region(&followed, seed);
        let difference = region.len().abs_diff(reference.len());
        assert!(
            difference * 50 <= reference.len(),
            "{} vs {} triangles",
            region.len(),
            reference.len()
        );
        // A curve that comes back to a vertex from far away touches itself.
        let mut touching = margin.points.clone();
        touching.insert(touching.len() / 2, margin.points[touching.len() / 4]);
        assert!(tracer.follow(&touching).is_err());
        // So does one that comes back to its first vertex halfway round, without
        // crossing itself, and goes on.
        let mut touching = margin.points.clone();
        touching.insert(touching.len() / 2, margin.points[0]);
        assert!(tracer.follow(&touching).is_err());
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

    /// Full-arch intraoral scans prepare every tooth, and the scan is ragged
    /// between neighbours: directions facing a neighbour show no stump wall or
    /// no ridge. Detection must bridge them and still follow each margin,
    /// measured in 3D at curve samples and between them, both ways.
    #[test]
    fn propose_on_a_row_of_adjacent_preps_with_ragged_gaps() {
        let rotation = glam::DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 0.71);
        let axis = rotation * DVec3::Z;
        let (mesh, tops, margins) =
            crate::synthetic::prepared_row(3, 8.2, 0.4, -0.2, 0.02, rotation);
        let tracer = MarginTracer::new(Arc::new(mesh)).unwrap();
        let edge: Vec<DVec3> = (0..tracer.mesh.positions.len() as u32)
            .filter(|&i| tracer.adjacency.is_boundary(i))
            .map(|i| tracer.mesh.positions[i as usize])
            .collect();
        for (k, (top, truth)) in tops.iter().zip(&margins).enumerate() {
            let margin = tracer
                .propose(*top, axis)
                .unwrap_or_else(|e| panic!("stump {k}: {e}"));
            let points = &margin.points;
            let n = points.len();
            let mut samples = points.clone();
            samples.extend((0..n).map(|i| points[i].lerp(points[(i + 1) % n], 0.5)));
            // Where the ragged interproximal hole reaches the margin, the margin
            // was not scanned: compare only true-margin samples at least 0.3 mm
            // from the scan's open edge, and the detected samples nearest to them.
            let m = truth.len();
            let on_scan: Vec<bool> = truth
                .iter()
                .map(|p| edge.iter().all(|e| e.distance_squared(*p) > 0.3 * 0.3))
                .collect();
            let scanned: Vec<DVec3> = (0..m).filter(|&i| on_scan[i]).map(|i| truth[i]).collect();
            assert!(
                scanned.len() * 5 >= m * 3,
                "stump {k}: holes removed 40% of the margin"
            );
            let near_scanned: Vec<DVec3> = samples
                .iter()
                .copied()
                .filter(|s| {
                    let i = (0..m)
                        .min_by(|&a, &b| {
                            segment_distance(*s, truth[a], truth[(a + 1) % m])
                                .total_cmp(&segment_distance(*s, truth[b], truth[(b + 1) % m]))
                        })
                        .unwrap();
                    on_scan[i] && on_scan[(i + 1) % m]
                })
                .collect();
            let off = distance_to_curve(&near_scanned, truth);
            let missed = distance_to_curve(&scanned, points);
            eprintln!(
                "stump {k}: off the margin {off:.3} mm, margin missed by {missed:.3} mm, {} of {m} true samples scanned",
                scanned.len()
            );
            // The ridge detector sits up to about 0.2 mm inside this heightfield's
            // rounded edge on every stump alike, isolated or not.
            assert!(off <= 0.3, "stump {k}: detected curve strays {off} mm");
            assert!(
                missed <= 0.3,
                "stump {k}: true margin missed by {missed} mm"
            );
        }
    }

    /// Ring-selection input over all sectors: `candidates(sector)` lists the
    /// (radius, depth) of that sector's ridges; candidate k of sector s has id
    /// 10 s + k. Sectors with no candidate are left out, as `sector_ridges` does.
    fn ring_input(
        candidates: impl Fn(usize) -> Vec<(f64, f64)>,
    ) -> Vec<(usize, Vec<RingCandidate>)> {
        (0..SECTORS)
            .filter_map(|s| {
                let list: Vec<RingCandidate> = candidates(s)
                    .into_iter()
                    .enumerate()
                    .map(|(k, (r, d))| ((10 * s + k) as u32, r, d))
                    .collect();
                (!list.is_empty()).then_some((s, list))
            })
            .collect()
    }

    fn ring_sectors(ring: &[u32]) -> Vec<usize> {
        ring.iter().map(|&id| id as usize / 10).collect()
    }

    #[test]
    fn ring_skips_a_lone_sector_on_a_neighbour() {
        let choices = ring_input(|s| vec![if s == 15 { (6.5, 1.0) } else { (4.0, 1.0) }]);
        let ring = join_ring(&choices, |_| 1.0).unwrap();
        let expected: Vec<usize> = (0..SECTORS).filter(|&s| s != 15).collect();
        assert_eq!(ring_sectors(&ring), expected);
    }

    #[test]
    fn ring_does_not_bulge_onto_a_neighbour_across_a_gap() {
        // Sectors 10-12 show nothing and 13-15 only a neighbouring stump's
        // ridge 2.5 mm further out. Bridging 9 -> 16 skips three sectors; the
        // ring through the neighbour (9 -> 13 -> 14 -> 15 -> 18) skips only
        // two, but it is not this stump's margin, at the same depth or deeper.
        for neighbour_depth in [1.0, 2.0] {
            let choices = ring_input(|s| match s {
                10..=12 => vec![],
                13..=15 => vec![(6.5, neighbour_depth)],
                _ => vec![(4.0, 1.0)],
            });
            let ring = join_ring(&choices, |_| 1.0).unwrap();
            let expected: Vec<usize> = (0..SECTORS).filter(|s| !(10..=15).contains(s)).collect();
            assert_eq!(ring_sectors(&ring), expected, "depth {neighbour_depth}");
        }
    }

    #[test]
    fn ring_keeps_every_sector_of_a_noisy_oval_stump() {
        let mut rng = Lcg::new(7);
        let noise: Vec<(f64, f64)> = (0..SECTORS)
            .map(|_| (0.6 * rng.unit() - 0.3, 0.6 * rng.unit() - 0.3))
            .collect();
        let choices = ring_input(|s| {
            let angle = TAU * (s as f64 + 0.5) / SECTORS as f64;
            vec![(
                4.5 + 0.8 * (2.0 * angle).cos() + noise[s].0,
                1.0 + 0.5 * angle.sin() + noise[s].1,
            )]
        });
        let ring = join_ring(&choices, |_| 1.0).unwrap();
        assert_eq!(ring_sectors(&ring), (0..SECTORS).collect::<Vec<_>>());
    }

    #[test]
    fn ring_prefers_the_stump_candidate_over_a_neighbour_in_the_same_sector() {
        // Sectors 13-15 also hold a neighbour's deeper ridge 2.5 mm further out,
        // listed first: the ring stays on the stump's own candidate (k = 1).
        let choices = ring_input(|s| match s {
            13..=15 => vec![(6.5, 2.0), (4.0, 1.0)],
            _ => vec![(4.0, 1.0)],
        });
        let ring = join_ring(&choices, |_| 1.0).unwrap();
        assert_eq!(ring_sectors(&ring), (0..SECTORS).collect::<Vec<_>>());
        assert_eq!(&ring[13..16], &[131, 141, 151]);
    }

    #[test]
    fn ring_rejects_degenerate_and_sparse_input() {
        assert!(join_ring(&[], |_| 1.0).is_err());
        assert!(join_ring(&[(4, vec![(40, 4.0, 1.0)])], |_| 1.0).is_err());
        // Zero ridge strength everywhere still yields a finite ring.
        let all = ring_input(|_| vec![(4.0, 1.0)]);
        assert_eq!(join_ring(&all, |_| 0.0).unwrap().len(), SECTORS);
        // Exactly MAX_EMPTY_SECTORS empty sectors are bridged; more are not.
        let sparse = ring_input(|s| if s % 3 == 0 { vec![] } else { vec![(4.0, 1.0)] });
        assert_eq!(join_ring(&sparse, |_| 1.0).unwrap().len(), 24);
        let sparse = ring_input(|s| if s % 2 == 0 { vec![] } else { vec![(4.0, 1.0)] });
        assert!(join_ring(&sparse, |_| 1.0).is_err());
        // A gap longer than MAX_SECTOR_GAP steps.
        let arc = ring_input(|s| {
            if (20..27).contains(&s) {
                vec![]
            } else {
                vec![(4.0, 1.0)]
            }
        });
        assert!(join_ring(&arc, |_| 1.0).is_err());
        // No member may close the ring: every sector jumps 2 mm from the last.
        let zigzag = ring_input(|s| vec![(if s % 2 == 0 { 3.0 } else { 5.0 }, 1.0)]);
        assert!(join_ring(&zigzag, |_| 1.0).is_err());
    }
}
