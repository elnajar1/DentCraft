//! Convex-ridge margin tracing. Inputs must have outward consistent winding.
use crate::DesignError;
use dentcraft_mesh::{
    Bvh, PrincipalCurvature, TriMesh, VertexAdjacency, inside_closed_path, path_through,
    region_triangles, ridge_cost, smooth_polyline_on_surface,
};
use glam::DVec3;
use std::f64::consts::TAU;

/// Closed curve: neither list repeats the first entry at the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Margin {
    /// Surface-projected smooth samples, with nominal spacing 0.1 mm.
    pub points: Vec<DVec3>,
    /// Simple underlying edge loop, used as the region barrier.
    pub vertices: Vec<u32>,
}

/// Reusable immutable geometry cache. Rebuild after changing a mesh.
pub struct MarginTracer<'a> {
    mesh: &'a TriMesh,
    adjacency: VertexAdjacency,
    curvature: Vec<PrincipalCurvature>,
    cost: Vec<f64>,
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

impl<'a> MarginTracer<'a> {
    /// Precompute adjacency, signed principal curvatures, ridge cost and BVH.
    /// The ridge sharpness is adaptive: `12 / sharpest k1`, so the strongest
    /// convex edge lands just above the path toolkit's `MIN_COST` clamp
    /// (`exp(-12) ≈ 6e-6`). A fixed sharpness either saturates that clamp —
    /// which erases the contrast between the margin and its neighbouring
    /// tessellation rings — or is too weak to glue the path to the ridge.
    /// Empty, wholly degenerate and nonfinite meshes are rejected before toolkit calls.
    pub fn new(mesh: &'a TriMesh) -> Result<Self, DesignError> {
        validate_mesh(mesh)?;
        let adjacency = mesh.vertex_adjacency();
        let curvature = mesh.principal_curvatures();
        let sharpest = curvature
            .iter()
            .map(|pc| pc.k1.max(0.0))
            .fold(0.0, f64::max);
        let sharpness = if sharpest > 1e-9 {
            (12.0 / sharpest).clamp(0.1, 6.0)
        } else {
            0.0
        };
        let cost = ridge_cost(&curvature, sharpness);
        let bvh = Bvh::build(mesh);
        Ok(Self {
            mesh,
            adjacency,
            curvature,
            cost,
            bvh,
        })
    }

    fn snap(&self, point: DVec3) -> Result<u32, DesignError> {
        if !point.is_finite() {
            return Err(DesignError::InvalidMargin);
        }
        let hit = self
            .bvh
            .closest_point(self.mesh, point)
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
        for &click in clicks {
            let v = self.snap(click)?;
            let center = self.mesh.positions[v as usize];
            // Search only the connected local patch, never a nearby separate tooth.
            let mut stack = vec![v];
            let mut seen = std::collections::HashSet::with_capacity(64);
            seen.insert(v);
            let mut best = v;
            while let Some(a) = stack.pop() {
                if self.curvature[a as usize].k1 > self.curvature[best as usize].k1 {
                    best = a;
                }
                for &b in self.adjacency.neighbors(a) {
                    if self.mesh.positions[b as usize].distance_squared(center) <= 0.45_f64.powi(2)
                        && seen.insert(b)
                    {
                        stack.push(b);
                    }
                }
            }
            waypoints.push(best);
        }
        self.connect(&waypoints)
    }

    fn connect(&self, waypoints: &[u32]) -> Result<Margin, DesignError> {
        if waypoints.len() < 3 {
            return Err(DesignError::InvalidMargin);
        }
        let path = path_through(self.mesh, &self.adjacency, waypoints, true, |v| {
            self.cost[v as usize]
        })
        .ok_or(DesignError::InvalidMargin)?;
        let mut vertices = path.vertices;
        vertices.pop();
        let unique: std::collections::HashSet<_> = vertices.iter().collect();
        if vertices.len() < 3 || unique.len() != vertices.len() {
            return Err(DesignError::InvalidMargin);
        }
        let points: Vec<_> = vertices
            .iter()
            .map(|&v| self.mesh.positions[v as usize])
            .collect();
        let smooth = smooth_polyline_on_surface(self.mesh, &self.bvh, &points, 1);
        let mut points = resample(&smooth, 0.1);
        for p in &mut points {
            if let Some(hit) = self.bvh.closest_point(self.mesh, *p) {
                *p = hit.point;
            }
        }
        Ok(Margin { points, vertices })
    }

    /// Propose a single-tooth margin in 36 sectors. Search radius is 8 mm,
    /// axial depth is 0.3..6 mm below the surface-snapped occlusal seed.
    /// These limits intentionally exclude remote model bases; this is a proposal,
    /// not a detector for arbitrary full-jaw geometry or preparations over 6 mm tall.
    /// Reject radial outliers beyond 25% of the median and require 27 sectors.
    pub fn propose(&self, seed: DVec3, axis: DVec3) -> Result<Margin, DesignError> {
        if !axis.is_finite() || axis.length_squared() < 1e-24 {
            return Err(DesignError::InvalidAxis);
        }
        let axis = axis.normalize();
        let center = self.mesh.positions[self.snap(seed)? as usize];
        let u = axis.any_orthonormal_vector();
        let v = axis.cross(u);
        let mut sectors: [Option<(u32, f64)>; 36] = [None; 36];
        for (i, &point) in self.mesh.positions.iter().enumerate() {
            let d = point - center;
            let depth = -d.dot(axis);
            let radius = d.cross(axis).length();
            if !(0.3..=6.0).contains(&depth) || !(0.5..=8.0).contains(&radius) {
                continue;
            }
            let score = self.curvature[i].k1;
            if score <= 0.0 || self.adjacency.is_boundary(i as u32) {
                continue;
            }
            let angle = d.dot(v).atan2(d.dot(u)).rem_euclid(TAU);
            let sector = ((angle / TAU * 36.0) as usize).min(35);
            if sectors[sector].is_none_or(|(old, _)| score > self.curvature[old as usize].k1) {
                sectors[sector] = Some((i as u32, radius));
            }
        }
        let mut radii: Vec<_> = sectors.iter().flatten().map(|&(_, r)| r).collect();
        if radii.len() < 27 {
            return Err(DesignError::InvalidMargin);
        }
        radii.sort_by(f64::total_cmp);
        let median = radii[radii.len() / 2];
        let waypoints: Vec<_> = sectors
            .into_iter()
            .flatten()
            .filter(|&(_, r)| (r - median).abs() <= median * 0.25)
            .map(|(i, _)| i)
            .collect();
        if waypoints.len() < 27 {
            return Err(DesignError::InvalidMargin);
        }
        self.connect(&waypoints)
    }

    /// Triangles on the seed side of the vertex barrier. Invalid loops or seeds
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
        inside_closed_path(self.mesh, &self.adjacency, &margin.vertices, seed)
            .map(|inside| region_triangles(self.mesh, &inside))
            .unwrap_or_default()
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
            let tracer = MarginTracer::new(&die.mesh).unwrap();
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
        let tracer = MarginTracer::new(&die.mesh).unwrap();
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
            let tracer = MarginTracer::new(&die.mesh).unwrap();
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
        let tracer = MarginTracer::new(&die.mesh).unwrap();
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
        let tracer = MarginTracer::new(&die.mesh).unwrap();
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
        let split_tracer = MarginTracer::new(&split).unwrap();
        let mut clicks = clicks_near(&die.margin, 4, &mut Lcg::new(5));
        clicks.push(DVec3::splat(50.0));
        assert_eq!(split_tracer.trace(&clicks), Err(DesignError::InvalidMargin));
        // Degenerate tracer inputs.
        assert!(matches!(
            MarginTracer::new(&TriMesh::default()),
            Err(DesignError::EmptyRegion)
        ));
        let mut nonfinite = die.mesh.clone();
        nonfinite.positions[0] = DVec3::NAN;
        assert!(matches!(
            MarginTracer::new(&nonfinite),
            Err(DesignError::InvalidMesh)
        ));
        let collinear = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::new(2.0, 0.0, 0.0)],
            vec![[0, 1, 2]],
        );
        assert!(matches!(
            MarginTracer::new(&collinear),
            Err(DesignError::DegenerateRegion)
        ));
    }

    #[test]
    fn inner_surface_boundary_is_the_proposed_margin() {
        let die = prepared_tooth(&ToothParams::default()).unwrap();
        let tracer = MarginTracer::new(&die.mesh).unwrap();
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
        let tracer = MarginTracer::new(&die.mesh).unwrap();
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
