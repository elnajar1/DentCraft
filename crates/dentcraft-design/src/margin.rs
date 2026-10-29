//! Convex-ridge margin tracing. Inputs must have outward consistent winding.
use crate::DesignError;
use dentcraft_mesh::{Bvh, TriMesh, VertexAdjacency, path_through, smooth_polyline_on_surface};
use glam::DVec3;
use std::f64::consts::TAU;
use std::sync::Arc;

/// Closed curve: neither list repeats the first entry at the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Margin {
    /// Surface-projected smooth samples, with nominal spacing 0.1 mm.
    pub points: Vec<DVec3>,
    /// Simple underlying edge loop, used as the region barrier.
    pub vertices: Vec<u32>,
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
    pub fn trace(&self, clicks: &[DVec3]) -> Result<Margin, DesignError> {
        if clicks.len() < 3 {
            return Err(DesignError::InvalidMargin);
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

    fn connect(&self, waypoints: &[u32], cost: &[f64]) -> Result<Margin, DesignError> {
        if waypoints.len() < 3 {
            return Err(DesignError::InvalidMargin);
        }
        let path = path_through(&self.mesh, &self.adjacency, waypoints, true, |v| {
            cost[v as usize]
        })
        .ok_or(DesignError::InvalidMargin)?;
        let mut vertices = path.vertices;
        vertices.pop();
        let unique: std::collections::HashSet<_> = vertices.iter().collect();
        if vertices.len() < 3
            || unique.len() != vertices.len()
            || vertices.iter().any(|&i| self.adjacency.is_boundary(i))
        {
            return Err(DesignError::InvalidMargin);
        }
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
        Ok(Margin { points, vertices })
    }

    /// Propose a single-tooth margin, then refine its centre and best-fit axis.
    /// Select the most apical ridge at least half as strong as the best ridge in
    /// each sector; reject radial and axial outliers before connecting the loop.
    /// The axial window scales with the local preparation, capped at 12 mm.
    pub fn propose(&self, seed: DVec3, axis: DVec3) -> Result<Margin, DesignError> {
        if !axis.is_finite() || axis.length_squared() < 1e-24 {
            return Err(DesignError::InvalidAxis);
        }
        let axis = axis.normalize();
        let seed_vertex = self.snap(seed)?;
        let center = self.mesh.positions[seed_vertex as usize];
        // Restrict to the clicked component: an adjacent tooth is never a ridge
        // candidate even when it lies inside the search annulus.
        let mut component = vec![false; self.mesh.positions.len()];
        let mut stack = vec![seed_vertex];
        component[seed_vertex as usize] = true;
        while let Some(i) = stack.pop() {
            for &j in self.adjacency.neighbors(i) {
                if !component[j as usize] {
                    component[j as usize] = true;
                    stack.push(j);
                }
            }
        }
        let first = self.sector_ridges(center, axis, &component, None)?;
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
        let first_margin = self.connect(&first, &first_cost)?;
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
        let second = self.sector_ridges(centroid, refined, &component, Some(1.0))?;
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
        self.connect(&second, &cost)
    }

    fn sector_ridges(
        &self,
        center: DVec3,
        axis: DVec3,
        component: &[bool],
        band: Option<f64>,
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
        let mut winners = Vec::new();
        for sector in &mut sectors {
            let strongest = sector
                .iter()
                .map(|&(i, _, _)| self.ridge_strength[i as usize])
                .fold(0.0, f64::max);
            sector.retain(|&(i, _, _)| self.ridge_strength[i as usize] >= 0.5 * strongest);
            sector.sort_by(|a, b| a.2.total_cmp(&b.2));
            // Adjacent samples above half-height form one ridge, not separate
            // candidates. Use its peak; taking its apical flank biases the line.
            let mut groups: Vec<(u32, f64, f64)> = Vec::new();
            let mut last = f64::NEG_INFINITY;
            for &candidate in sector.iter() {
                if candidate.2 - last > 0.35 {
                    groups.push(candidate);
                } else if let Some(best) = groups.last_mut() {
                    if self.ridge_strength[candidate.0 as usize]
                        > self.ridge_strength[best.0 as usize]
                    {
                        *best = candidate;
                    }
                }
                last = candidate.2;
            }
            if let Some(&winner) = groups.last() {
                winners.push(winner);
            }
        }
        if winners.len() < 27 {
            return Err(DesignError::InvalidMargin);
        }
        let median = |mut values: Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        let radius = median(winners.iter().map(|p| p.1).collect());
        let depth = median(winners.iter().map(|p| p.2).collect());
        // First-pass tolerances admit an offset seed and a tilted axis. The
        // second pass is centred on the margin itself and is deliberately tight.
        let radial_tolerance = if band.is_some() {
            0.25 * radius
        } else {
            0.85 * radius
        };
        let axial_tolerance = if band.is_some() { 0.5 } else { 0.75 * radius };
        winners.retain(|p| {
            (p.1 - radius).abs() <= radial_tolerance && (p.2 - depth).abs() <= axial_tolerance
        });
        if winners.len() < 27 {
            return Err(DesignError::InvalidMargin);
        }
        Ok(winners.into_iter().map(|p| p.0).collect())
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
