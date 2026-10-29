//! Compactly supported contact adjustment, in mm, of crowns made by `build_crown`.
//!
//! Signed distances use the closest triangle's outward normal, for both closed
//! teeth and open scans: the back side is negative. This is a local oriented
//! surface distance, not a winding-number solid classification. Open scan edges
//! and inconsistent winding consequently cannot define a global inside/outside.
//! Supply consistently outward-oriented scans covering the contact region.

use dentcraft_mesh::{Bvh, TriMesh};
use glam::DVec3;
use rayon::prelude::*;

use super::{
    Crown, CrownParams, Polar, centroid, check_crossings, check_faces, face_samples,
    validate_surface,
};
use crate::{DesignError, InnerSurface, geometry::unit_axis};

/// Contact distances and bounded deformation controls, all in mm.
#[derive(Clone, Copy, Debug)]
pub struct AdaptParams {
    /// Desired minimum proximal signed distance; negative means light contact.
    pub proximal_contact: f64,
    /// Lower bound on antagonist distance; negative means into occlusion.
    pub occlusal_clearance: f64,
    /// Compact smoothstep falloff radius in original Euclidean coordinates.
    pub influence_radius: f64,
    /// Maximum total displacement of any vertex from its input position.
    pub max_move: f64,
}

impl Default for AdaptParams {
    fn default() -> Self {
        Self {
            proximal_contact: -0.02,
            occlusal_clearance: 0.0,
            influence_radius: 1.5,
            max_move: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AdaptReport {
    /// Minimum sampled outer-surface distance; None when no mesial scan is given.
    pub mesial_contact: Option<f64>,
    /// Minimum sampled outer-surface distance; None when no distal scan is given.
    pub distal_contact: Option<f64>,
    /// Minimum sampled outer-surface distance; None when no antagonist is given.
    pub min_occlusal_distance: Option<f64>,
    /// Original area (mm²) of outer triangles incident to moved vertices.
    pub moved_area: f64,
    /// False if a thickness constraint left an unmet contact. The returned mesh
    /// still respects the crown builder's 1 µm thickness tolerance. Inspect the
    /// distances for residual contact error, including unreachable/max_move cases.
    pub thickness_ok: bool,
}

fn distance(point: DVec3, other: &TriMesh, bvh: &Bvh) -> (f64, DVec3) {
    let Some(hit) = bvh.closest_point(other, point) else {
        return (f64::NAN, DVec3::ZERO);
    };
    let normal = other
        .triangle_cross(hit.triangle as usize)
        .normalize_or_zero();
    let delta = point - hit.point;
    let sign = if delta.dot(normal) < 0.0 { -1.0 } else { 1.0 };
    let gradient = if hit.distance > 1e-12 {
        delta * (sign / hit.distance)
    } else {
        normal
    };
    (hit.distance * sign, gradient)
}

/// Distances in the order of `vertices`. Uses closest-face normal signs (see
/// module documentation). `other_bvh` must have been built on unmodified `other`.
/// This infallible API returns NaN for an invalid index/nonfinite query or an
/// empty/invalid other mesh; an empty vertex selection returns an empty vector.
pub fn signed_distance_map(
    crown: &TriMesh,
    vertices: &[u32],
    other: &TriMesh,
    other_bvh: &Bvh,
) -> Vec<f64> {
    if validate_mesh(other).is_err() {
        return vec![f64::NAN; vertices.len()];
    }
    vertices
        .par_iter()
        .map(|&v| {
            crown.positions.get(v as usize).map_or(f64::NAN, |&p| {
                if p.is_finite() {
                    distance(p, other, other_bvh).0
                } else {
                    f64::NAN
                }
            })
        })
        .collect()
}

fn validate_mesh(mesh: &TriMesh) -> Result<(), DesignError> {
    if mesh.positions.is_empty() || mesh.triangles.is_empty() {
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
    for i in 0..mesh.triangles.len() {
        let area = mesh.triangle_cross(i).length_squared();
        if !area.is_finite() || area < 1e-24 {
            return Err(DesignError::DegenerateRegion);
        }
    }
    Ok(())
}

struct Kernel {
    point: DVec3,
    amplitude: f64,
}

struct KernelTree {
    lo: DVec3,
    hi: DVec3,
    amplitude: f64,
    children: Option<Box<[KernelTree; 2]>>,
    range: std::ops::Range<usize>,
}

impl KernelTree {
    fn build(seeds: &mut [Kernel], offset: usize) -> Self {
        let mut node = Self {
            lo: DVec3::INFINITY,
            hi: DVec3::NEG_INFINITY,
            amplitude: 0.0,
            children: None,
            range: offset..offset + seeds.len(),
        };
        for seed in seeds.iter() {
            node.lo = node.lo.min(seed.point);
            node.hi = node.hi.max(seed.point);
            node.amplitude = node.amplitude.max(seed.amplitude);
        }
        if seeds.len() > 8 {
            let axis = (node.hi - node.lo).max_position();
            let mid = seeds.len() / 2;
            seeds.select_nth_unstable_by(mid, |a, b| a.point[axis].total_cmp(&b.point[axis]));
            let (a, b) = seeds.split_at_mut(mid);
            node.children = Some(Box::new([
                Self::build(a, offset),
                Self::build(b, offset + mid),
            ]));
        }
        node
    }

    fn bound(&self, point: DVec3, radius: f64) -> f64 {
        self.amplitude * falloff(point.distance(point.clamp(self.lo, self.hi)) / radius)
    }

    fn query(&self, point: DVec3, radius: f64, seeds: &[Kernel], best: &mut f64) {
        if self.bound(point, radius) <= *best {
            return;
        }
        if let Some(children) = &self.children {
            let (first, second) =
                if children[0].bound(point, radius) >= children[1].bound(point, radius) {
                    (0, 1)
                } else {
                    (1, 0)
                };
            children[first].query(point, radius, seeds, best);
            children[second].query(point, radius, seeds, best);
        } else {
            for seed in &seeds[self.range.clone()] {
                *best = best.max(seed.amplitude * falloff(point.distance(seed.point) / radius));
            }
        }
    }
}

fn falloff(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

// Maximum of compact C1 kernels in original Euclidean coordinates. A spatial
// tree prunes kernels whose upper bound cannot improve the current value.
// Unlike graph-path distances, this field stays continuous across skinny faces.
fn spread(values: &mut [f64], original: &TriMesh, locked: &[bool], radius: f64) {
    let mut seeds: Vec<_> = values
        .iter()
        .enumerate()
        .filter(|&(v, a)| !locked[v] && *a > 0.00001)
        .map(|(v, &amplitude)| Kernel {
            point: original.positions[v],
            amplitude,
        })
        .collect();
    if seeds.is_empty() {
        values.fill(0.0);
        return;
    }
    let tree = KernelTree::build(&mut seeds, 0);
    values.par_iter_mut().enumerate().for_each(|(v, a)| {
        if locked[v] {
            *a = 0.0;
        } else {
            tree.query(original.positions[v], radius, &seeds, a);
        }
    });
}
struct Thickness<'a> {
    inner: &'a InnerSurface,
    bvh: Bvh,
    margin: Vec<DVec3>,
    axis: DVec3,
    params: &'a CrownParams,
    cache: Vec<Option<ThicknessFace>>,
}

#[derive(Clone, Copy)]
struct ThicknessFace {
    positions: [DVec3; 3],
    normals: [DVec3; 3],
    result: (f64, f64),
}

impl Thickness<'_> {
    // Identical quadrature and normal interpolation to the crown builder.
    fn measure(&mut self, outer: &TriMesh) -> (f64, f64) {
        let normals = outer.vertex_normals();
        self.cache.resize(outer.triangles.len(), None);
        outer
            .triangles
            .par_iter()
            .zip(self.cache.par_iter_mut())
            .map(|(tri, cache)| {
                let p = tri.map(|v| outer.positions[v as usize]);
                let n = tri.map(|v| normals[v as usize]);
                if let Some(cached) = cache
                    && cached.positions == p
                    && cached.normals == n
                {
                    return cached.result;
                }
                let mut minimum = f64::INFINITY;
                let mut deficit = 0.0_f64;
                for w in face_samples(p) {
                    let point = p[0] * w[0] + p[1] * w[1] + p[2] * w[2];
                    let normal = (n[0] * w[0] + n[1] * w[1] + n[2] * w[2]).normalize();
                    let hit = self.bvh.closest_point(&self.inner.mesh, point).unwrap();
                    let inside = self
                        .bvh
                        .raycast(&self.inner.mesh, point + self.axis * 1e-8, self.axis)
                        .is_some();
                    let d = if inside { -hit.distance } else { hit.distance };
                    minimum = minimum.min(d);
                    deficit = deficit.max(
                        self.params
                            .required_thickness(hit.point, normal, self.axis, &self.margin)
                            - d,
                    );
                }
                let result = (minimum, deficit);
                *cache = Some(ThicknessFace {
                    positions: p,
                    normals: n,
                    result,
                });
                result
            })
            .reduce(|| (f64::INFINITY, 0.0), |a, b| (a.0.min(b.0), a.1.max(b.1)))
    }
}

#[derive(Clone, Copy)]
struct ScanFace {
    positions: [DVec3; 3],
    corrections: [f64; 3],
    minimum: f64,
}

struct Scan<'a> {
    mesh: &'a TriMesh,
    bvh: Bvh,
    target: f64,
    grow: bool,
    faces: Vec<Option<ScanFace>>,
    vertices: Vec<(DVec3, (f64, DVec3))>,
}

struct ContactField {
    queries: Vec<(f64, DVec3)>,
    amounts: Vec<f64>,
    minimum: f64,
}

impl Scan<'_> {
    fn inspect(&mut self, outer: &TriMesh, locked: &[bool]) -> ContactField {
        self.vertices
            .resize(outer.positions.len(), (DVec3::NAN, (f64::NAN, DVec3::ZERO)));
        self.vertices
            .par_iter_mut()
            .zip(&outer.positions)
            .for_each(|(cached, &p)| {
                if cached.0 != p {
                    *cached = (p, distance(p, self.mesh, &self.bvh));
                }
            });
        let queries: Vec<_> = self.vertices.iter().map(|v| v.1).collect();
        let mut amounts: Vec<_> = queries
            .iter()
            .map(|q| (self.target - q.0).max(0.0))
            .collect();
        self.faces.resize(outer.triangles.len(), None);
        // Cache validity depends on positions only: the other scan and target
        // stay fixed, and closest-point gradients do not use crown normals.
        self.faces
            .par_iter_mut()
            .zip(&outer.triangles)
            .for_each(|(cache, tri)| {
                let p = tri.map(|v| outer.positions[v as usize]);
                if cache.is_some_and(|c| c.positions == p) {
                    return;
                }
                let mut corrections = [0.0_f64; 3];
                let mut minimum = f64::INFINITY;
                for w in face_samples(p) {
                    let point = p[0] * w[0] + p[1] * w[1] + p[2] * w[2];
                    let (d, gradient) = distance(point, self.mesh, &self.bvh);
                    minimum = minimum.min(d);
                    let missing = self.target - d;
                    if missing <= 0.0001 {
                        continue;
                    }
                    let slopes = tri.map(|v| queries[v as usize].1.dot(gradient).max(0.0));
                    let covered: f64 = (0..3)
                        .map(|i| w[i] * amounts[tri[i] as usize] * slopes[i])
                        .sum();
                    let denominator: f64 = (0..3)
                        .filter(|&i| !locked[tri[i] as usize])
                        .map(|i| (w[i] * slopes[i]).powi(2))
                        .sum();
                    if denominator > 1e-12 {
                        for i in 0..3 {
                            corrections[i] = corrections[i]
                                .max((missing - covered).max(0.0) * w[i] * slopes[i] / denominator);
                        }
                    }
                }
                *cache = Some(ScanFace {
                    positions: p,
                    corrections,
                    minimum,
                });
            });
        let mut minimum = f64::INFINITY;
        for (tri, cache) in outer.triangles.iter().zip(&self.faces) {
            let face = cache.unwrap();
            minimum = minimum.min(face.minimum);
            for (&vertex, &correction) in tri.iter().zip(&face.corrections) {
                let v = vertex as usize;
                amounts[v] = amounts[v].max((self.target - queries[v].0).max(0.0) + correction);
            }
        }
        ContactField {
            queries,
            amounts,
            minimum,
        }
    }

    fn proposal(
        &mut self,
        outer: &TriMesh,
        original: &TriMesh,
        locked: &[bool],
        adapt: &AdaptParams,
    ) -> Vec<DVec3> {
        let ContactField {
            queries,
            mut amounts,
            minimum,
        } = self.inspect(outer, locked);
        let growing = self.grow && minimum > self.target + 0.0001;
        if growing
            && let Some((v, q)) = queries
                .iter()
                .enumerate()
                .filter(|(v, _)| !locked[*v])
                .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            && q.0 - self.target <= adapt.max_move
        {
            amounts[v] = q.0 - self.target;
        }
        spread(&mut amounts, original, locked, adapt.influence_radius);
        amounts
            .into_iter()
            .zip(queries)
            .enumerate()
            .map(|(v, (a, q))| {
                if locked[v] || a <= 0.00001 {
                    DVec3::ZERO
                } else {
                    q.1 * a.min(adapt.max_move) * if growing { -1.0 } else { 1.0 }
                }
            })
            .collect()
    }
}
/// Adapt the outer cap, preserving inner and cervical vertices bit-for-bit.
/// Input must be an unmodified `build_crown` assembly for this `inner` (or an
/// earlier adaptation). Invalid inputs/errors leave the entire crown unchanged.
/// Unreachable or conflicting contacts retain their measured residual distances.
/// A conservative line search stops at the first thickness/fold constraint;
/// this does not promise a global optimum for mutually incompatible scans.
#[allow(clippy::too_many_arguments)]
pub fn adapt_to_neighbours(
    crown: &mut Crown,
    inner: &InnerSurface,
    params: &CrownParams,
    adapt: &AdaptParams,
    mesial: Option<&TriMesh>,
    distal: Option<&TriMesh>,
    antagonist: Option<&TriMesh>,
) -> Result<AdaptReport, DesignError> {
    if !adapt.proximal_contact.is_finite()
        || !adapt.occlusal_clearance.is_finite()
        || !adapt.influence_radius.is_finite()
        || adapt.influence_radius <= 0.0
        || !adapt.max_move.is_finite()
        || adapt.max_move < 0.0
        || [
            params.margin_thickness,
            params.min_wall,
            params.min_occlusal,
            params.thickness_ramp,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0)
        || params.margin_thickness > params.min_wall.min(params.min_occlusal)
    {
        return Err(DesignError::InvalidParameter("contact adaptation controls"));
    }
    validate_mesh(&crown.mesh)?;
    validate_mesh(&inner.mesh)?;
    for scan in [mesial, distal, antagonist].into_iter().flatten() {
        validate_mesh(scan)?;
    }
    let axis = unit_axis(inner.axis)?;
    let inner_edges = validate_surface(&inner.mesh, &inner.boundary)?;
    let offset = inner.mesh.positions.len();
    if crown.mesh.positions.len() <= offset
        || crown.mesh.positions[..offset] != inner.mesh.positions
        || !crown.mesh.triangles.starts_with(&inner.mesh.triangles)
        || crown.margin_vertices != inner.boundary
        || !crown.mesh.is_closed()
        || !crown.mesh.is_consistently_oriented()
        || crown.mesh.signed_volume() <= 0.0
    {
        return Err(DesignError::InvalidCrownSurface);
    }
    let mut outer = TriMesh::new(
        crown.mesh.positions[offset..].to_vec(),
        crown
            .mesh
            .triangles
            .iter()
            .filter(|t| t.iter().all(|&v| v as usize >= offset))
            .map(|t| t.map(|v| v - offset as u32))
            .collect(),
    );
    let loops = outer.boundary_loops();
    if loops.len() != 1 {
        return Err(DesignError::InvalidCrownSurface);
    }
    let edges = validate_surface(&outer, &loops[0])?;
    let original = outer.clone();
    let mut locked = vec![false; outer.positions.len()];
    for &v in &loops[0] {
        locked[v as usize] = true;
    }
    let reference: Vec<_> = (0..outer.triangles.len())
        .map(|i| outer.triangle_cross(i))
        .collect();
    let polar = Polar {
        origin: centroid(&inner.mesh, &inner.boundary).unwrap(),
        x: DVec3::X,
        y: DVec3::Y,
        z: axis,
    };
    check_faces(&outer, &reference, polar)?;
    let mut thickness = Thickness {
        inner,
        bvh: Bvh::build(&inner.mesh),
        margin: inner
            .boundary
            .iter()
            .map(|&v| inner.mesh.positions[v as usize])
            .collect(),
        axis,
        params,
        cache: Vec::new(),
    };
    let initial = thickness.measure(&outer);
    if initial.1 > 0.001 {
        return Err(DesignError::CrownThickness { deficit: initial.1 });
    }
    let mut scans: Vec<_> = [mesial, distal, antagonist]
        .into_iter()
        .enumerate()
        .map(|(i, mesh)| {
            mesh.map(|mesh| Scan {
                mesh,
                bvh: Bvh::build(mesh),
                target: if i == 2 {
                    adapt.occlusal_clearance
                } else {
                    adapt.proximal_contact
                },
                grow: i != 2,
                faces: Vec::new(),
                vertices: Vec::new(),
            })
        })
        .collect();

    let mut thickness_ok = true;
    let mut minimum = initial.0;
    for _ in 0..8 {
        let mut progress = 0.0_f64;
        for scan in scans.iter_mut().flatten() {
            let moves = scan.proposal(&outer, &original, &locked, adapt);
            let previous = outer.positions.clone();
            let proposed: Vec<_> = previous
                .iter()
                .zip(&moves)
                .zip(&original.positions)
                .map(|((&p, &d), &start)| {
                    if d == DVec3::ZERO {
                        p
                    } else {
                        start + (p + d - start).clamp_length_max(adapt.max_move)
                    }
                })
                .collect();
            if previous
                .iter()
                .zip(&proposed)
                .all(|(a, b)| a.distance(*b) < 1e-7)
            {
                continue;
            }
            let mut lo = 0.0;
            let mut hi = 1.0;
            let mut alpha = 1.0;
            let mut accepted_min = minimum;
            // First try the whole field; bisect only when a safety constraint binds.
            for _ in 0..13 {
                for v in 0..previous.len() {
                    outer.positions[v] = if proposed[v] == previous[v] {
                        previous[v]
                    } else {
                        previous[v].lerp(proposed[v], alpha)
                    };
                }
                let valid_faces = check_faces(&outer, &reference, polar).is_ok();
                let measured = if valid_faces {
                    thickness.measure(&outer)
                } else {
                    (0.0, f64::INFINITY)
                };
                if valid_faces && measured.1 <= 0.001 {
                    lo = alpha;
                    accepted_min = measured.0;
                    if alpha == 1.0 {
                        break;
                    }
                } else {
                    if valid_faces {
                        thickness_ok = false;
                    }
                    hi = alpha;
                }
                alpha = (lo + hi) * 0.5;
            }
            for v in 0..previous.len() {
                outer.positions[v] = if lo == 0.0 || proposed[v] == previous[v] {
                    previous[v]
                } else {
                    previous[v].lerp(proposed[v], lo)
                };
                progress = progress.max(previous[v].distance(outer.positions[v]));
            }
            minimum = accepted_min;
        }
        if progress < 0.0001 {
            break;
        }
    }
    check_faces(&outer, &reference, polar)?;
    check_crossings(&outer, &inner.mesh, &thickness.bvh, &edges, &inner_edges)?;
    let distances: Vec<_> = scans
        .iter_mut()
        .map(|s| s.as_mut().map(|s| s.inspect(&outer, &locked).minimum))
        .collect();
    if scans.iter().zip(&distances).all(|(scan, measured)| {
        scan.as_ref().is_none_or(|s| {
            measured.is_some_and(|d| {
                if s.grow {
                    (d - s.target).abs() <= 0.001
                } else {
                    d >= s.target - 0.001
                }
            })
        })
    }) {
        thickness_ok = true;
    }
    let moved_area = original
        .triangles
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.iter()
                .any(|&v| original.positions[v as usize] != outer.positions[v as usize])
        })
        .map(|(i, _)| original.triangle_cross(i).length() * 0.5)
        .sum();
    // Commit only after every geometric check succeeds.
    let mut result = crown.mesh.clone();
    result.positions[offset..].copy_from_slice(&outer.positions);
    if result.signed_volume() <= 0.0 {
        return Err(DesignError::CrownIntersection);
    }
    crown.mesh = result;
    crown.report.min_thickness = minimum;
    Ok(AdaptReport {
        mesial_contact: distances[0],
        distal_contact: distances[1],
        min_occlusal_distance: distances[2],
        moved_area,
        thickness_ok,
    })
}
