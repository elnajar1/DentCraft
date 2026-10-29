//! A constrained height-field intaglio surface, with exact margin positions.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
};

use dentcraft_mesh::TriMesh;
use glam::{DVec2, DVec3};
use rayon::prelude::*;
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Point2, Triangulation};

use crate::{
    DesignError,
    geometry::{Frame, Region, unit_axis},
    polygon::Polygon,
};

/// Cement spacing and sampling, all in millimetres. Gaps and widths must be finite
/// and nonnegative; `grid_step` must be in (0, 0.1] mm.
#[derive(Clone, Debug)]
pub struct InnerSurfaceParams {
    /// Full cement gap away from the margin.
    pub cement_gap: f64,
    /// Gap within the seal band; the exact boundary itself always remains on the input margin.
    pub seal_gap: f64,
    /// Width of the seal band along the blocked-out die surface from the margin.
    pub margin_seal_width: f64,
    /// Surface distance of the smoothstep blend; zero gives a step.
    pub transition_width: f64,
    /// Projected grid spacing and axial refinement spacing, at most 0.1 mm.
    pub grid_step: f64,
    /// Horizontal scan-noise tolerance used to reject an unseatable margin.
    pub min_undercut_depth: f64,
}

impl Default for InnerSurfaceParams {
    fn default() -> Self {
        Self {
            cement_gap: 0.05,
            seal_gap: 0.0,
            margin_seal_width: 0.8,
            transition_width: 0.5,
            grid_step: 0.025,
            min_undercut_depth: 0.03,
        }
    }
}

/// An open crown inner surface with normals directed toward the die (negative axial component).
#[derive(Clone, Debug)]
pub struct InnerSurface {
    /// Intaglio triangles, wound toward the die. This is not a closed crown solid.
    pub mesh: TriMesh,
    /// Boundary indices in input margin order, after removing exact consecutive duplicates
    /// and an optional repeated closing point. Positions are copied bit-for-bit.
    pub boundary: Vec<u32>,
    /// Required normal clearance per vertex. Boundary targets are exactly zero;
    /// face/edge targets are their barycentric/linear interpolations.
    pub target_gaps: Vec<f64>,
    /// Unit insertion axis over which `mesh` is a height field; crown assembly
    /// uses it for every die-related test.
    pub axis: DVec3,
}

/// Block out undercuts by first-hit rays from above the region, then offset the
/// sampled envelope by a varying-radius ball and triangulate inside the projected margin.
///
/// Seal and transition distances follow the first-hit envelope. Dijkstra uses
/// 3D edge lengths (sqrt(projected_step² + height_difference²)), with exact
/// point-to-margin-polyline distances for the first interior samples. Steep
/// edges are split at axial-height levels spaced by grid_step. The sampled
/// ball is followed by exact closest-point clearance correction, moving only
/// along +axis. Every vertex, edge midpoint and centroid is then checked against
/// the interpolated vertex targets, to 1 µm, including signed clearance at zero
/// gap — vertices are re-checked in every round because a raise alongside a
/// wall that leans over them (a fold or the edge of a scan hole) reduces their
/// clearance again. A deficient sample raises its triangle's lowest movable
/// vertices first, so a blocked-out cliff closes over the overhang lip rather
/// than lifting its top.
///
/// Boundary positions remain bit-exact. A margin overhang beyond the horizontal
/// noise threshold is rejected, except within the margin seal band
/// ([`crate::SEAL_BAND_MM`] around the margin line, the crossing zone of the
/// barrier and the smooth curve). Missing grid hits inherit nearby valid
/// heights. A grid step greater than 0.1 mm is rejected. Allocation/refinement
/// limits and failure to converge return errors, never an unchecked surface.
pub fn inner_surface(
    mesh: &TriMesh,
    region: &[u32],
    margin: &[DVec3],
    axis: DVec3,
    params: &InnerSurfaceParams,
) -> Result<InnerSurface, DesignError> {
    validate(params)?;
    let axis = unit_axis(axis)?;
    let frame = Frame::new(*margin.first().ok_or(DesignError::InvalidMargin)?, axis);
    let polygon = Polygon::new(margin, frame)?;
    let prep = Region::new(mesh, region)?;
    reject_margin_undercut(&polygon, frame, &prep, params.min_undercut_depth)?;
    let mut grid = Grid::new(&polygon, params)?;
    grid.cast(&polygon, frame, &prep);
    grid.fill_holes()?;
    let (mut result, distances) = triangulate(&polygon, frame, &grid, &prep)?;
    grid.set_distances(&result.mesh, &distances, frame);
    let heights = grid.offset(params);
    let boundary_len = result.boundary.len();
    let adjusted = result
        .mesh
        .positions
        .par_iter()
        .enumerate()
        .map(|(i, &point)| {
            if i < boundary_len {
                return Ok((point, 0.0));
            }
            let target = gap(distances[i], params);
            let mut start = point + frame.w * target;
            if let Some(j) = grid.exact_index(frame.project(point)) {
                start += frame.w * (heights[j] - frame.height(start)).max(0.0);
            }
            let mut adjusted = clear_point(start, target, frame, &prep)?;
            let mut target = target;
            // Raising moves the closest contact up the wall. Honour the gap there
            // too: exact distance to the margin bounds surface distance from below.
            // Never reduce the envelope's target or an already established height.
            for _ in 0..24 {
                if target >= params.cement_gap.max(params.seal_gap) {
                    break;
                }
                let hit = prep.bvh.closest_point(&prep.mesh, adjusted).unwrap();
                let next = target.max(gap(margin_distance(hit.point, &polygon.margin), params));
                if next <= target + 1e-7 {
                    break;
                }
                target = next;
                adjusted += frame.w * (target - (adjusted - point).dot(frame.w)).max(0.0);
                adjusted = clear_point(adjusted, target, frame, &prep)?;
            }
            Ok((adjusted, target))
        })
        .collect::<Result<Vec<_>, DesignError>>()?;
    for (i, (point, target)) in adjusted.into_iter().enumerate() {
        result.mesh.positions[i] = point;
        result.target_gaps[i] = target;
    }
    correct_faces(&mut result, frame, &prep)?;
    Ok(result)
}

fn validate(p: &InnerSurfaceParams) -> Result<(), DesignError> {
    if [
        p.cement_gap,
        p.seal_gap,
        p.margin_seal_width,
        p.transition_width,
    ]
    .iter()
    .any(|v| !v.is_finite() || *v < 0.0)
        || !p.grid_step.is_finite()
        || p.grid_step <= 0.0
        || p.grid_step > 0.1
        || !p.min_undercut_depth.is_finite()
        || !(0.0..=1.0).contains(&p.min_undercut_depth)
    {
        return Err(DesignError::InvalidParameter(
            "inner surface gaps, widths or grid_step",
        ));
    }
    Ok(())
}

fn gap(distance: f64, p: &InnerSurfaceParams) -> f64 {
    if distance <= p.margin_seal_width {
        return p.seal_gap;
    }
    if p.transition_width == 0.0 {
        return p.cement_gap;
    }
    let t = ((distance - p.margin_seal_width) / p.transition_width).clamp(0.0, 1.0);
    p.seal_gap + (p.cement_gap - p.seal_gap) * t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy)]
struct Cell {
    inside: bool,
    distance: f64,
    height: f64,
}

struct Grid {
    origin: DVec2,
    step: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Cell>,
}

impl Grid {
    fn exact_index(&self, p: DVec2) -> Option<usize> {
        let q = (p - self.origin) / self.step;
        let r = q.round();
        if q.distance(r) > 1e-7
            || r.min_element() < 0.0
            || r.x >= self.nx as f64
            || r.y >= self.ny as f64
        {
            None
        } else {
            Some(r.y as usize * self.nx + r.x as usize)
        }
    }

    fn set_distances(&mut self, mesh: &TriMesh, distances: &[f64], frame: Frame) {
        for (&p, &distance) in mesh.positions.iter().zip(distances) {
            if let Some(i) = self.exact_index(frame.project(p)) {
                self.cells[i].distance = distance;
            }
        }
    }

    fn new(polygon: &Polygon, p: &InnerSurfaceParams) -> Result<Self, DesignError> {
        let size = (polygon.max - polygon.min) / p.grid_step;
        let count = (size.x.ceil() + 1.0) * (size.y.ceil() + 1.0);
        let radius = (p.cement_gap.max(p.seal_gap) / p.grid_step).ceil();
        if !count.is_finite()
            || count > 2_000_000.0
            || count * (2.0 * radius + 1.0).powi(2) > 200_000_000.0
        {
            return Err(DesignError::GridTooLarge);
        }
        let nx = size.x.ceil() as usize + 1;
        let ny = size.y.ceil() as usize + 1;
        Ok(Self {
            origin: polygon.min,
            step: p.grid_step,
            nx,
            ny,
            cells: vec![
                Cell {
                    inside: false,
                    distance: 0.0,
                    height: f64::NAN
                };
                nx * ny
            ],
        })
    }

    fn point(&self, i: usize) -> DVec2 {
        self.origin + DVec2::new((i % self.nx) as f64, (i / self.nx) as f64) * self.step
    }

    fn cast(&mut self, polygon: &Polygon, frame: Frame, prep: &Region) {
        let ceiling = prep
            .mesh
            .positions
            .iter()
            .map(|&p| frame.height(p))
            .fold(f64::NEG_INFINITY, f64::max)
            + 1.0;
        let (origin, step, nx) = (self.origin, self.step, self.nx);
        self.cells.par_iter_mut().enumerate().for_each(|(i, cell)| {
            let p = origin + DVec2::new((i % nx) as f64, (i / nx) as f64) * step;
            cell.inside = polygon.contains(p);
            if cell.inside {
                cell.distance = polygon.distance(p);
                if let Some(hit) = prep
                    .bvh
                    .raycast(&prep.mesh, frame.lift(p, ceiling), -frame.w)
                {
                    cell.height = frame.height(hit.point);
                }
            }
        });
    }

    fn fill_holes(&mut self) -> Result<(), DesignError> {
        if !self.cells.iter().any(|c| c.inside && c.height.is_finite()) {
            return Err(DesignError::NoSurfaceSamples);
        }
        if self.cells.iter().all(|c| !c.inside || c.height.is_finite()) {
            return Ok(());
        }
        let mut queue: VecDeque<_> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.inside && c.height.is_finite())
            .map(|(i, _)| i)
            .collect();
        while let Some(i) = queue.pop_front() {
            let (x, y) = (i % self.nx, i / self.nx);
            for j in [
                (x > 0).then(|| i - 1),
                (x + 1 < self.nx).then_some(i + 1),
                (y > 0).then(|| i - self.nx),
                (y + 1 < self.ny).then_some(i + self.nx),
            ]
            .into_iter()
            .flatten()
            {
                if self.cells[j].inside && !self.cells[j].height.is_finite() {
                    self.cells[j].height = self.cells[i].height;
                    queue.push_back(j);
                }
            }
        }
        if self.cells.iter().any(|c| c.inside && !c.height.is_finite()) {
            return Err(DesignError::NoSurfaceSamples);
        }
        Ok(())
    }

    fn offset(&self, params: &InnerSurfaceParams) -> Vec<f64> {
        self.cells
            .par_iter()
            .enumerate()
            .map(|(i, cell)| {
                if !cell.inside {
                    return f64::NAN;
                }
                let g = gap(cell.distance, params);
                let radius = (g / self.step).floor() as usize;
                let (x, y) = (i % self.nx, i / self.nx);
                let mut height = cell.height + g;
                for qy in y.saturating_sub(radius)..=(y + radius).min(self.ny - 1) {
                    for qx in x.saturating_sub(radius)..=(x + radius).min(self.nx - 1) {
                        let d2 = ((qx as f64 - x as f64).powi(2) + (qy as f64 - y as f64).powi(2))
                            * self.step.powi(2);
                        let other = self.cells[qy * self.nx + qx];
                        if other.inside && d2 <= g * g {
                            height = height.max(other.height + (g * g - d2).max(0.0).sqrt());
                        }
                    }
                }
                height
            })
            .collect()
    }
}

fn margin_distance(point: DVec3, margin: &[DVec3]) -> f64 {
    margin
        .iter()
        .zip(margin.iter().cycle().skip(1))
        .map(|(&a, &b)| {
            let t = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
            point.distance(a + (b - a) * t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Slice steep triangles into axial-height strips before triangulating each
/// strip. Shared-edge intersections are cached, so refinement is conforming.
/// Unlike repeated triangle quartering this keeps the wall's circumferential
/// spacing: only the direction with a large height change is refined.
fn refine(
    mesh: &mut TriMesh,
    distances: &mut Vec<f64>,
    frame: Frame,
    step: f64,
    prep: &Region,
) -> Result<(), DesignError> {
    use std::collections::HashMap;
    // A planar patch already has exact linear interpolation, including its
    // immutable boundary; its ordinary grid needs no axial refinement.
    let normal = prep.mesh.triangle_cross(0).normalize();
    let origin = prep.mesh.triangle(0)[0];
    if prep
        .mesh
        .positions
        .iter()
        .all(|&p| (p - origin).dot(normal).abs() < 1e-10)
    {
        return Ok(());
    }
    let ceiling = prep
        .mesh
        .positions
        .iter()
        .map(|&p| frame.height(p))
        .fold(f64::NEG_INFINITY, f64::max)
        + 1.0;
    {
        let original_len = mesh.positions.len();
        let heights: Vec<_> = mesh
            .positions
            .iter()
            .map(|&p| {
                let h = frame.height(p);
                let snapped = (h / step).round() * step;
                if (h - snapped).abs() < 1e-9 {
                    snapped
                } else {
                    h
                }
            })
            .collect();
        let mut cache = HashMap::new();
        let mut edge_samples = Vec::new();
        let mut triangles = Vec::with_capacity(mesh.triangles.len() * 2);
        for &tri in &mesh.triangles {
            let h = tri.map(|v| heights[v as usize]);
            let lo = h.into_iter().fold(f64::INFINITY, f64::min);
            let hi = h.into_iter().fold(f64::NEG_INFINITY, f64::max);
            if hi - lo <= 1e-9 {
                triangles.push(tri);
                continue;
            }
            let levels = ((lo / step).floor() as i64 + 1)..=((hi / step).ceil() as i64 - 1);
            // Boundary of each strip, in the original triangle's winding.
            let mut remaining = tri.to_vec();
            let mut local_heights = h.to_vec();
            let mut original_edges = vec![(tri[0], tri[0]), (tri[1], tri[1]), (tri[2], tri[2])];
            for level in levels {
                let cut = level as f64 * step;
                let mut lower = Vec::new();
                let mut upper = Vec::new();
                let mut upper_h = Vec::new();
                let mut upper_edges = Vec::new();
                for j in 0..remaining.len() {
                    let k = (j + 1) % remaining.len();
                    let a = remaining[j];
                    let (ha, hb) = (local_heights[j], local_heights[k]);
                    if ha <= cut {
                        lower.push(a);
                    }
                    if ha >= cut {
                        upper.push(a);
                        upper_h.push(ha);
                        upper_edges.push(original_edges[j]);
                    }
                    if (ha < cut && hb > cut) || (hb < cut && ha > cut) {
                        let mut endpoints = [
                            original_edges[j].0,
                            original_edges[j].1,
                            original_edges[k].0,
                            original_edges[k].1,
                        ];
                        endpoints.sort_unstable();
                        let edge = (endpoints[0], endpoints[3]);
                        let index = *cache.entry((edge.0, edge.1, level)).or_insert_with(|| {
                            let t = (cut - heights[edge.0 as usize])
                                / (heights[edge.1 as usize] - heights[edge.0 as usize]);
                            let point = mesh.positions[edge.0 as usize]
                                .lerp(mesh.positions[edge.1 as usize], t);
                            let index = mesh.positions.len() as u32;
                            mesh.positions.push(point);
                            edge_samples.push((edge, cut, t));
                            distances.push(
                                distances[edge.0 as usize] * (1.0 - t)
                                    + distances[edge.1 as usize] * t,
                            );
                            index
                        });
                        lower.push(index);
                        upper.push(index);
                        upper_h.push(cut);
                        upper_edges.push(edge);
                    }
                }
                for j in 1..lower.len().saturating_sub(1) {
                    triangles.push([lower[0], lower[j], lower[j + 1]]);
                }
                remaining = upper;
                local_heights = upper_h;
                original_edges = upper_edges;
            }
            for j in 1..remaining.len().saturating_sub(1) {
                triangles.push([remaining[0], remaining[j], remaining[j + 1]]);
            }
        }
        if mesh.positions.len() > 2_000_000 {
            return Err(DesignError::RefinementLimit);
        }
        mesh.triangles = triangles;
        if mesh.positions.len() == original_len {
            return Ok(());
        }
        // Level points on one edge are solved in order of their linear parameter and
        // kept strictly increasing along the edge. The envelope is discontinuous at a
        // blocked-out overhang (a vertical cliff) and need not be monotone, so free
        // per-level root finding can pile several levels onto the jump in any order;
        // the strip polygons would then fold and break the height field. Ordered
        // points on the triangle's edges keep every strip a convex, CCW polygon.
        let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (k, &(edge, _, _)) in edge_samples.iter().enumerate() {
            by_edge.entry(edge).or_default().push(k);
        }
        let mut groups: Vec<_> = by_edge.into_iter().collect();
        groups.sort_unstable_by_key(|&(edge, _)| edge);
        let positions = &mesh.positions;
        let edge_distances = &*distances;
        let solved: Vec<_> = groups
            .par_iter_mut()
            .flat_map_iter(|((a, b), samples)| {
                let (a, b) = (*a as usize, *b as usize);
                samples.sort_unstable_by(|&i, &j| edge_samples[i].2.total_cmp(&edge_samples[j].2));
                let start = positions[a];
                let edge = positions[b] - start;
                let count = samples.len();
                // At least 10 nm of projected separation between consecutive points.
                let length = frame.project(start).distance(frame.project(start + edge));
                let separation = (1e-5 / length).min(0.5 / (count + 1) as f64);
                let mut previous = 0.0;
                samples
                    .iter()
                    .enumerate()
                    .map(|(rank, &k)| {
                        let (_, cut, initial_t) = edge_samples[k];
                        let (mut lo, mut hi) =
                            ordered_parameter_bounds(previous, separation, count - rank);
                        let mut t = initial_t.clamp(lo, hi);
                        let mut point = start + edge * t;
                        let mut solved_t = t;
                        for _ in 0..24 {
                            let p = frame.project(start + edge * t);
                            let Some(hit) =
                                prep.bvh
                                    .raycast(&prep.mesh, frame.lift(p, ceiling), -frame.w)
                            else {
                                break;
                            };
                            point = hit.point;
                            solved_t = t;
                            let error = frame.height(hit.point) - cut;
                            if error.abs() < 1e-9 {
                                break;
                            }
                            if (error > 0.0) == (heights[b] > heights[a]) {
                                hi = t;
                            } else {
                                lo = t;
                            }
                            let normal = prep.mesh.triangle_cross(hit.triangle as usize);
                            let derivative =
                                edge.dot(frame.w) - normal.dot(edge) / normal.dot(frame.w);
                            let next = t - error / derivative;
                            t = if next.is_finite() && next > lo && next < hi {
                                next
                            } else {
                                (lo + hi) * 0.5
                            };
                        }
                        previous = solved_t;
                        let distance =
                            edge_distances[a] * (1.0 - solved_t) + edge_distances[b] * solved_t;
                        (original_len + k, point, distance)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        for (index, point, distance) in solved {
            mesh.positions[index] = point;
            distances[index] = distance;
        }
    }
    Ok(())
}

fn ordered_parameter_bounds(previous: f64, separation: f64, remaining: usize) -> (f64, f64) {
    let hi = 1.0 - separation * remaining as f64;
    // Repeated addition and the reserved-tail multiplication can differ by an
    // ulp when preceding roots reached their upper bound. Collapse that tiny
    // interval; keep the same reserved space and ordering for later samples.
    ((previous + separation).min(hi), hi)
}

/// Reject when the region cannot seat on the margin along `axis`: the worst
/// horizontal extension beyond the projected margin (region vertices, edge
/// midpoints and centroids) must stay within `threshold`. Samples within the
/// [`crate::SEAL_BAND_MM`] band around the margin line are the barrier/curve
/// crossing zone and do not count (see [`crate::margin_overhang::measure_polygon`]).
fn reject_margin_undercut(
    polygon: &Polygon,
    frame: Frame,
    prep: &Region,
    threshold: f64,
) -> Result<(), DesignError> {
    let overhang = crate::margin_overhang::measure_polygon(prep, polygon, frame);
    if overhang.depth > threshold + prep.tolerance {
        Err(DesignError::MarginUndercut {
            depth: overhang.depth,
            sector_deg: overhang.sector_deg,
        })
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
struct DistanceEntry(f64, usize);
impl Eq for DistanceEntry {}
impl PartialOrd for DistanceEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for DistanceEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .0
            .total_cmp(&self.0)
            .then_with(|| other.1.cmp(&self.1))
    }
}

fn surface_distances(mesh: &TriMesh, boundary: usize, polygon: &Polygon) -> Vec<f64> {
    let mut neighbours = vec![Vec::new(); mesh.positions.len()];
    let mut first = vec![false; mesh.positions.len()];
    for tri in &mesh.triangles {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            let (a, b) = (a as usize, b as usize);
            let length = mesh.positions[a].distance(mesh.positions[b]);
            neighbours[a].push((b, length));
            neighbours[b].push((a, length));
            if a < boundary {
                first[b] = true;
            }
            if b < boundary {
                first[a] = true;
            }
        }
    }
    let mut distances = vec![f64::INFINITY; mesh.positions.len()];
    let mut queue = BinaryHeap::new();
    for i in 0..mesh.positions.len() {
        if i < boundary || first[i] {
            let d = if i < boundary {
                0.0
            } else {
                margin_distance(mesh.positions[i], &polygon.margin)
            };
            distances[i] = d;
            queue.push(DistanceEntry(d, i));
        }
    }
    while let Some(DistanceEntry(d, i)) = queue.pop() {
        if d > distances[i] {
            continue;
        }
        for &(j, length) in &neighbours[i] {
            let next = d + length;
            if next < distances[j] {
                distances[j] = next;
                queue.push(DistanceEntry(next, j));
            }
        }
    }
    distances
}

fn signed_clearance(point: DVec3, prep: &Region) -> (f64, DVec3) {
    let hit = prep.bvh.closest_point(&prep.mesh, point).unwrap();
    let normal = prep.mesh.triangle_cross(hit.triangle as usize).normalize();
    let delta = point - hit.point;
    let sign = if delta.dot(normal) < 0.0 { -1.0 } else { 1.0 };
    let gradient = if hit.distance > 1e-10 {
        delta / hit.distance * sign
    } else {
        normal
    };
    (hit.distance * sign, gradient)
}

/// Exact closest-point queries remove the ball stencil's normal-distance error.
/// Newton steps use the local signed-distance gradient. Limit steps and recheck:
/// at a line angle the nearest face can change. Heights never decrease.
fn clear_point(
    mut point: DVec3,
    target: f64,
    frame: Frame,
    prep: &Region,
) -> Result<DVec3, DesignError> {
    for _ in 0..64 {
        let (distance, normal) = signed_clearance(point, prep);
        if distance >= target - 1e-7 {
            return Ok(point);
        }
        let raise = ((target - distance) / normal.dot(frame.w).max(0.01)).min(1.0);
        let candidate = point + frame.w * raise;
        let actual = signed_clearance(candidate, prep).0;
        if (actual - target).abs() <= 1e-7 {
            return Ok(candidate);
        }
        if actual > target + 1e-7 {
            let mut lo = 0.0;
            let mut hi = raise;
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                if signed_clearance(point + frame.w * mid, prep).0 >= target {
                    hi = mid;
                } else {
                    lo = mid;
                }
                if hi - lo < 1e-7 {
                    break;
                }
            }
            return Ok(point + frame.w * hi);
        }
        point = candidate;
    }
    Err(DesignError::RefinementLimit)
}

/// A supporting plane is a rigorous lower bound on outside distance. Certify
/// against every prep vertex once; nonconvex faces fall back to BVH queries.
/// This avoids millions of repeated closest-point traversals on planar facets.
fn supporting_planes(prep: &Region) -> Vec<Option<(DVec3, f64)>> {
    use std::collections::HashMap;
    let mut cache = HashMap::new();
    (0..prep.mesh.triangles.len())
        .map(|i| {
            let normal = prep.mesh.triangle_cross(i).normalize();
            let offset = normal.dot(prep.mesh.triangle(i)[0]);
            let key = [normal.x, normal.y, normal.z, offset].map(|v| (v * 1e9).round() as i64);
            *cache.entry(key).or_insert_with(|| {
                prep.mesh
                    .positions
                    .iter()
                    .all(|p| normal.dot(*p) <= offset + 1e-10)
                    .then_some((normal, offset + 1e-10))
            })
        })
        .collect()
}

fn correct_faces(
    surface: &mut InnerSurface,
    frame: Frame,
    prep: &Region,
) -> Result<(), DesignError> {
    let boundary = surface.boundary.len();
    let planes = supporting_planes(prep);
    let vertex_planes: Vec<_> = surface
        .mesh
        .positions
        .par_iter()
        .map(|&point| {
            let hit = prep.bvh.closest_point(&prep.mesh, point).unwrap();
            planes[hit.triangle as usize]
        })
        .collect();
    for _ in 0..64 {
        let corrections = surface
            .mesh
            .triangles
            .par_iter()
            .filter_map(|tri| {
                let ids = tri.map(|i| i as usize);
                let points = ids.map(|i| surface.mesh.positions[i]);
                let gaps = ids.map(|i| surface.target_gaps[i]);
                let mut shifts = [0.0_f64; 3];
                // Vertices included: a raise that cleared one sample can push a
                // vertex alongside a leaning wall (a fold over a scan hole) back
                // into the die; only a re-check catches that.
                for weights in [
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.5, 0.5, 0.0],
                    [0.0, 0.5, 0.5],
                    [0.5, 0.0, 0.5],
                    [1.0 / 3.0; 3],
                ] {
                    let point = (0..3).map(|i| points[i] * weights[i]).sum::<DVec3>();
                    let target = (0..3).map(|i| gaps[i] * weights[i]).sum::<f64>();
                    // Fast path: skip when every local supporting plane certifies
                    // the sample. A single certifying plane is only a bound on the
                    // unsigned distance; on the open region patch a sample can sit
                    // behind its own wall face in the finish-line valley while a
                    // neighbouring face's plane vouches for it, so all three local
                    // planes (and a non-supporting face) must force the exact query.
                    if ids.iter().all(|&i| {
                        vertex_planes[i].is_some_and(|(n, d)| n.dot(point) - d >= target - 0.0008)
                    }) {
                        continue;
                    }
                    if signed_clearance(point, prep).0 >= target - 0.0008 {
                        continue;
                    }
                    let corrected = match clear_point(point, target, frame, prep) {
                        Ok(point) => point,
                        Err(error) => return Some(Err(error)),
                    };
                    let deficit = (corrected - point).dot(frame.w);
                    let Some(raises) = fill_lowest(
                        points.map(|p| frame.height(p)),
                        weights,
                        ids.map(|i| i >= boundary),
                        deficit,
                    ) else {
                        // The sample lies entirely on the exact margin boundary
                        // and cannot be raised. The smoothed curve crosses the
                        // barrier's triangles, so against the region alone it
                        // can read up to the crossing noise inside although it
                        // lies on the scan; tolerate the seal band, reject a
                        // margin genuinely buried in the preparation.
                        if signed_clearance(point, prep).0 >= target - crate::SEAL_BAND_MM {
                            continue;
                        }
                        return Some(Err(DesignError::RefinementLimit));
                    };
                    for i in 0..3 {
                        shifts[i] = shifts[i].max(raises[i]);
                    }
                }
                shifts.iter().any(|&d| d > 0.0).then_some(Ok((ids, shifts)))
            })
            .collect::<Result<Vec<_>, DesignError>>()?;
        if corrections.is_empty() {
            return Ok(());
        }
        let mut raises = vec![0.0_f64; surface.mesh.positions.len()];
        for (ids, shifts) in corrections {
            for i in 0..3 {
                raises[ids[i]] = raises[ids[i]].max(shifts[i]);
            }
        }
        if raises.iter().all(|&d| d == 0.0) {
            return Ok(());
        }
        for (point, raise) in surface.mesh.positions.iter_mut().zip(raises) {
            *point += frame.w * raise;
        }
    }
    Err(DesignError::RefinementLimit)
}

/// Axial raises of a triangle's movable vertices that lift the sample with
/// barycentric `weights` by `deficit`, raising the lowest vertices first (water
/// filling to a common level). A uniform raise would also lift the top of a
/// near-vertical face at a blocked-out cliff; its midpoint under the overhang lip
/// would stay deficient, and repeated rounds would climb the whole column above
/// the die. Filling from below closes the cliff over the lip instead. The
/// sample's axial height is linear in the vertex heights, so it rises by exactly
/// `deficit`. `None` if no weighted vertex may move (all on the exact margin).
fn fill_lowest(
    heights: [f64; 3],
    weights: [f64; 3],
    movable: [bool; 3],
    deficit: f64,
) -> Option<[f64; 3]> {
    let mut order: Vec<usize> = (0..3).filter(|&i| movable[i] && weights[i] > 0.0).collect();
    if order.is_empty() {
        return None;
    }
    order.sort_by(|&a, &b| heights[a].total_cmp(&heights[b]));
    // Raise the k lowest vertices to a common level, adding the next vertex
    // once the level reaches its height.
    let mut weight = 0.0;
    let mut lifted = 0.0; // Sum of w_i * h_i over the vertices being raised.
    let mut level = f64::NAN;
    for (k, &i) in order.iter().enumerate() {
        weight += weights[i];
        lifted += weights[i] * heights[i];
        level = (deficit + lifted) / weight;
        if order.get(k + 1).is_none_or(|&next| level <= heights[next]) {
            break;
        }
    }
    let mut raises = [0.0; 3];
    for &i in &order {
        raises[i] = (level - heights[i]).max(0.0);
    }
    Some(raises)
}

#[derive(Clone, Copy)]
struct Vertex {
    point: Point2<f64>,
    index: u32,
}

impl HasPosition for Vertex {
    type Scalar = f64;
    fn position(&self) -> Point2<f64> {
        self.point
    }
}

fn triangulate(
    polygon: &Polygon,
    frame: Frame,
    grid: &Grid,
    prep: &Region,
) -> Result<(InnerSurface, Vec<f64>), DesignError> {
    let mut mesh = TriMesh {
        positions: polygon.margin.clone(),
        triangles: Vec::new(),
    };
    let mut vertices: Vec<_> = polygon
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| Vertex {
            point: Point2::new(p.x, p.y),
            index: i as u32,
        })
        .collect();
    let n = vertices.len();
    for (i, cell) in grid.cells.iter().enumerate() {
        if cell.inside && cell.distance > grid.step * 0.5 {
            let p = grid.point(i);
            vertices.push(Vertex {
                point: Point2::new(p.x, p.y),
                index: mesh.positions.len() as u32,
            });
            mesh.positions.push(frame.lift(p, cell.height));
        }
    }
    let edges = (0..n).map(|i| [i, (i + 1) % n]).collect();
    let mut conflict = false;
    let cdt =
        ConstrainedDelaunayTriangulation::<Vertex>::try_bulk_load_cdt(vertices, edges, |_| {
            conflict = true
        })
        .map_err(|_| DesignError::Triangulation)?;
    if conflict {
        return Err(DesignError::Triangulation);
    }
    for face in cdt.inner_faces() {
        let v = face.vertices().map(|v| *v.data());
        let center = v
            .iter()
            .map(|v| DVec2::new(v.point.x, v.point.y))
            .sum::<DVec2>()
            / 3.0;
        if polygon.contains(center) {
            // Spade's finite faces are CCW in (u,v); reverse them for crown-solid outward normals.
            mesh.triangles.push([v[0].index, v[2].index, v[1].index]);
        }
    }
    if mesh.triangles.is_empty() {
        return Err(DesignError::Triangulation);
    }
    let mut distances = surface_distances(&mesh, n, polygon);
    refine(&mut mesh, &mut distances, frame, grid.step, prep)?;
    Ok((
        InnerSurface {
            target_gaps: vec![0.0; mesh.positions.len()],
            mesh,
            boundary: (0..n as u32).collect(),
            axis: frame.w,
        },
        distances,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cliff_roots_at_reserved_tail_do_not_invert_the_next_interval() {
        for count in [16, 157, 800, 2048] {
            let separation = 0.0001_f64.min(0.5 / (count + 1) as f64);
            let mut previous = 0.0;
            for rank in 0..count {
                let (lo, hi) = ordered_parameter_bounds(previous, separation, count - rank);
                assert!(lo <= hi && hi < 1.0 && lo > previous);
                assert!(lo - previous >= separation - 1e-14);
                // A discontinuous wall can drive consecutive roots to their
                // upper bounds: formerly an ulp inversion panicked in clamp.
                let t = 1.0_f64.clamp(lo, hi);
                previous = t;
            }
        }
    }
    use crate::{StumpParams, prepared_stump};
    use dentcraft_mesh::Bvh;

    fn plane() -> (TriMesh, Vec<u32>, Vec<DVec3>) {
        let margin = vec![
            DVec3::ZERO,
            DVec3::X * 3.0,
            DVec3::new(3.0, 3.0, 0.0),
            DVec3::Y * 3.0,
        ];
        (
            TriMesh::new(margin.clone(), vec![[0, 1, 2], [0, 2, 3]]),
            vec![0, 1],
            margin,
        )
    }

    fn assert_disk(surface: &InnerSurface, axis: DVec3) {
        assert_eq!(surface.mesh.connected_components().len(), 1);
        let loops = surface.mesh.boundary_loops();
        assert_eq!(loops.len(), 1);
        let mut actual = loops[0].clone();
        actual.sort_unstable();
        let mut expected = surface.boundary.clone();
        expected.sort_unstable();
        assert_eq!(actual, expected);
        assert!(surface.mesh.is_consistently_oriented());
        assert_eq!(surface.mesh.non_manifold_edge_count(), 0);
        for i in 0..surface.mesh.triangles.len() {
            let n = surface.mesh.triangle_cross(i);
            assert!(n.length() > 0.0);
            assert!(n.dot(axis) <= 1e-9);
        }
    }

    #[test]
    fn stump_boundary_gap_seal_and_topology() {
        let die = prepared_stump(&StumpParams::default()).unwrap();
        let params = InnerSurfaceParams::default();
        let result = inner_surface(&die.mesh, &die.region, &die.margin, DVec3::Z, &params).unwrap();
        for (&i, p) in result.boundary.iter().zip(&die.margin) {
            assert_eq!(
                result.mesh.positions[i as usize]
                    .to_array()
                    .map(f64::to_bits),
                p.to_array().map(f64::to_bits)
            );
        }
        let prep = die.mesh.extract_triangles(&die.region);
        let bvh = Bvh::build(&prep);

        let (mut top_count, mut seal_count) = (0, 0);
        let (mut top_error, mut seal_error) = (0.0_f64, 0.0_f64);
        for &point in &result.mesh.positions[die.margin.len()..] {
            let hit = bvh.closest_point(&prep, point).unwrap();
            let distance = margin_distance(hit.point, &die.margin);
            if distance > params.margin_seal_width + params.transition_width + 0.1 {
                let actual = bvh.closest_point(&prep, point).unwrap().distance;
                top_error = top_error.max((actual - params.cement_gap).abs());
                top_count += 1;
            } else if distance < params.margin_seal_width {
                let actual = bvh.closest_point(&prep, point).unwrap().distance;
                seal_error = seal_error.max((actual - params.seal_gap).abs());
                seal_count += 1;
            }
        }
        eprintln!("flat-top gap max error {top_error:.9} mm; seal max error {seal_error:.9} mm");
        assert!(top_count > 100 && seal_count > 100);
        assert!(top_error <= 0.005);
        assert!(seal_error <= 0.005);
        assert_disk(&result, DVec3::Z);
    }

    #[test]
    fn divergent_stump_margin_is_rejected() {
        let die = prepared_stump(&StumpParams {
            top_radius: 3.5,
            tilt_deg: 15.0,
            ..Default::default()
        })
        .unwrap();
        let params = InnerSurfaceParams {
            grid_step: 0.08,
            ..Default::default()
        };
        assert!(matches!(
            inner_surface(&die.mesh, &die.region, &die.margin, die.axis * 3.0, &params),
            Err(DesignError::MarginUndercut { depth, .. }) if depth > 0.4
        ));
    }

    #[test]
    fn bad_margins_and_parameters_return_errors() {
        let (mesh, region, margin) = plane();
        let p = InnerSurfaceParams::default();
        let bow = vec![margin[0], margin[2], margin[1], margin[3]];
        for points in [&margin[..2], &bow, &[]] {
            assert!(matches!(
                inner_surface(&mesh, &region, points, DVec3::Z, &p),
                Err(DesignError::InvalidMargin)
            ));
        }
        assert!(matches!(
            inner_surface(&mesh, &[], &margin, DVec3::Z, &p),
            Err(DesignError::EmptyRegion)
        ));
        assert!(matches!(
            inner_surface(&mesh, &region, &margin, DVec3::ZERO, &p),
            Err(DesignError::InvalidAxis)
        ));
        for step in [0.0, -1.0, 0.1001, 0.3, 1.0, 5.0, f64::NAN] {
            assert!(matches!(
                inner_surface(
                    &mesh,
                    &region,
                    &margin,
                    DVec3::Z,
                    &InnerSurfaceParams {
                        grid_step: step,
                        ..p.clone()
                    }
                ),
                Err(DesignError::InvalidParameter(_))
            ));
        }
        assert!(matches!(
            inner_surface(
                &mesh,
                &region,
                &margin,
                DVec3::Z,
                &InnerSurfaceParams {
                    grid_step: 1e-8,
                    ..p
                }
            ),
            Err(DesignError::GridTooLarge)
        ));
    }

    #[test]
    fn duplicates_clockwise_and_zero_area_faces() {
        let (mut mesh, mut region, mut margin) = plane();
        mesh.positions.push(mesh.positions[0]);
        mesh.triangles.push([0, 4, 1]);
        region.push(2);
        let p = InnerSurfaceParams {
            grid_step: 0.1,
            ..Default::default()
        };
        assert!(matches!(
            inner_surface(&mesh, &[2], &margin, DVec3::Z, &p),
            Err(DesignError::DegenerateRegion)
        ));
        margin.reverse();
        margin.insert(1, margin[0]);
        margin.push(margin[0]);
        let result = inner_surface(&mesh, &region, &margin, DVec3::Z, &p).unwrap();
        assert_eq!(result.boundary.len(), 4);
        assert_disk(&result, DVec3::Z);
    }

    #[test]
    fn nonzero_seal_and_smooth_transition_on_plane() {
        let (mesh, region, margin) = plane();
        let p = InnerSurfaceParams {
            seal_gap: 0.02,
            grid_step: 0.1,
            ..Default::default()
        };
        let result = inner_surface(&mesh, &region, &margin, DVec3::Z, &p).unwrap();
        let polygon = Polygon::new(&margin, Frame::new(DVec3::ZERO, DVec3::Z)).unwrap();
        for point in &result.mesh.positions[4..] {
            let expected = gap(polygon.distance(point.truncate()), &p);
            assert!((point.z - expected).abs() < 1e-12);
        }
        assert_eq!(gap(0.8, &p), 0.02);
        assert!((gap(1.05, &p) - 0.035).abs() < 1e-12);
        assert_eq!(gap(1.5, &p), 0.05);
        assert_disk(&result, DVec3::Z);
    }

    #[test]
    fn missing_scan_samples_fill_from_neighbours() {
        let (mut mesh, _, margin) = plane();
        mesh.triangles.pop();
        let p = InnerSurfaceParams {
            grid_step: 0.1,
            ..Default::default()
        };
        let result = inner_surface(&mesh, &[0], &margin, DVec3::Z, &p).unwrap();
        assert_disk(&result, DVec3::Z);
        assert!(
            result
                .mesh
                .positions
                .iter()
                .all(|v| v.is_finite() && v.z >= 0.0 && v.z <= 0.05 + 1e-12)
        );
        for point in &mut mesh.positions {
            point.x += 100.0;
        }
        assert!(matches!(
            inner_surface(&mesh, &[0], &margin, DVec3::Z, &p),
            Err(DesignError::NoSurfaceSamples)
        ));
    }

    #[test]
    fn sampled_ball_offsets_a_sloped_plane_in_normal_distance() {
        let (mut mesh, region, mut margin) = plane();
        for point in &mut mesh.positions {
            point.z = point.x * 0.5;
        }
        for point in &mut margin {
            point.z = point.x * 0.5;
        }
        let params = InnerSurfaceParams {
            margin_seal_width: 0.0,
            transition_width: 0.0,
            ..Default::default()
        };
        let result = inner_surface(&mesh, &region, &margin, DVec3::Z, &params).unwrap();
        let mut max_error = 0.0_f64;
        let mut count = 0;
        for p in &result.mesh.positions[4..] {
            if p.x > 0.1 && p.x < 2.9 && p.y > 0.1 && p.y < 2.9 {
                let distance = (p.z - 0.5 * p.x) / 1.25_f64.sqrt();
                max_error = max_error.max((distance - params.cement_gap).abs());
                count += 1;
            }
        }
        eprintln!("sampled ball, slope 0.5: max normal-gap error {max_error:.9} mm");
        assert!(count > 100);
        assert!(max_error < 0.005);
        assert_disk(&result, DVec3::Z);
    }

    #[test]
    fn concave_margin_keeps_only_interior_faces() {
        let margin = vec![
            DVec3::ZERO,
            DVec3::X * 3.0,
            DVec3::new(3.0, 1.0, 0.0),
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(1.0, 3.0, 0.0),
            DVec3::Y * 3.0,
        ];
        let mesh = TriMesh::new(
            margin.clone(),
            vec![[0, 1, 3], [1, 2, 3], [0, 3, 5], [3, 4, 5]],
        );
        let params = InnerSurfaceParams {
            grid_step: 0.1,
            ..Default::default()
        };
        let result = inner_surface(&mesh, &[0, 1, 2, 3], &margin, DVec3::Z, &params).unwrap();
        assert_disk(&result, DVec3::Z);
        let area: f64 = (0..result.mesh.triangles.len())
            .map(|i| -result.mesh.triangle_cross(i).z * 0.5)
            .sum();
        assert!((area - 5.0).abs() < 1e-12);
    }

    #[test]
    fn undercut_above_margin_is_blocked_out_with_clear_faces() {
        let mut die = prepared_stump(&StumpParams::default()).unwrap();
        // A local concavity creates a real overhang entirely inside the margin.
        for p in &mut die.mesh.positions {
            if p.z > 1.0 && p.z < 4.85 {
                let dent = 0.55 * (1.0 - ((p.z - 2.8) / 0.8).abs()).max(0.0);
                let radius = p.truncate().length();
                p.x *= (radius - dent) / radius;
                p.y *= (radius - dent) / radius;
            }
        }
        assert!(
            crate::undercut_map(&die.mesh, &die.region, die.axis)
                .unwrap()
                .undercut_area
                > 0.0
        );
        let params = InnerSurfaceParams {
            grid_step: 0.1,
            ..Default::default()
        };
        let result = inner_surface(&die.mesh, &die.region, &die.margin, die.axis, &params).unwrap();
        assert_disk(&result, die.axis);
        let closed = Region::new(
            &die.mesh,
            &(0..die.mesh.triangles.len() as u32).collect::<Vec<_>>(),
        )
        .unwrap();
        for tri in &result.mesh.triangles {
            let points = tri.map(|i| result.mesh.positions[i as usize]);
            let gaps = tri.map(|i| result.target_gaps[i as usize]);
            for weights in ALL_WEIGHTS {
                let p = (0..3).map(|i| points[i] * weights[i]).sum::<DVec3>();
                let g = (0..3).map(|i| gaps[i] * weights[i]).sum::<f64>();
                assert!(signed_clearance(p, &closed).0 >= g - 0.001);
            }
        }
    }

    /// Sample weights of every vertex, edge midpoint and centroid.
    const ALL_WEIGHTS: [[f64; 3]; 7] = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.5, 0.5, 0.0],
        [0.0, 0.5, 0.5],
        [0.5, 0.0, 0.5],
        [1.0 / 3.0; 3],
    ];

    /// A flat floor with a fin that rises vertically, folds left over the floor
    /// and back right, stopping just short of a column to its right: the surface
    /// is not single-valued along the axis there, and the fold's top is open
    /// (like a scan hole above an overhang). The first-hit envelope jumps from
    /// the fold's top to the floor between two grid columns — a blocked-out
    /// cliff whose face is the fin's vertical wall. Closing that cliff raises
    /// the low vertex beside the wall until it stands beside the fold's tip;
    /// unless vertices are re-checked after such raises, the vertex ends up
    /// inside the fin's clearance — the deficit this test pins.
    #[test]
    fn raises_beside_a_leaning_fold_are_rechecked_at_vertices() {
        let margin = vec![
            DVec3::new(-4.0, -4.0, 0.0),
            DVec3::new(4.0, -4.0, 0.0),
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(-4.0, 4.0, 0.0),
        ];
        let mut positions = margin.clone();
        let mut triangles = vec![[0, 1, 2], [0, 2, 3]];
        // (x, z) outline of the fin, with the side its faces bound (the wall
        // bounds space to its right, the fold the space above it).
        let outline = [
            (0.735, 0.0, DVec3::X),
            (0.735, 2.9, DVec3::X),
            (0.4, 2.95, DVec3::Z),
            (0.79, 2.95, DVec3::Z),
        ];
        for window in outline.windows(2) {
            let (a, b) = (window[0], window[1]);
            let (base, outward) = (positions.len() as u32, a.2);
            // Quad between the sections, wound so the faces point `outward`.
            let [p, q, r, s] = [
                DVec3::new(a.0, -0.5, a.1),
                DVec3::new(a.0, 0.5, a.1),
                DVec3::new(b.0, 0.5, b.1),
                DVec3::new(b.0, -0.5, b.1),
            ];
            let flip = (q - p).cross(r - q).dot(outward) < 0.0;
            for point in [p, q, r, s] {
                positions.push(point);
            }
            if flip {
                triangles.push([base, base + 2, base + 1]);
                triangles.push([base, base + 3, base + 2]);
            } else {
                triangles.push([base, base + 1, base + 2]);
                triangles.push([base, base + 2, base + 3]);
            }
        }
        let mesh = TriMesh::new(positions, triangles);
        let region: Vec<u32> = (0..mesh.triangles.len() as u32).collect();
        assert!(
            crate::undercut_map(&mesh, &region, DVec3::Z)
                .unwrap()
                .undercut_area
                > 0.0
        );
        let params = InnerSurfaceParams {
            margin_seal_width: 0.0,
            transition_width: 0.0,
            grid_step: 0.1,
            ..Default::default()
        };
        let result = inner_surface(&mesh, &region, &margin, DVec3::Z, &params).unwrap();
        assert_disk(&result, DVec3::Z);
        let closed = Region::new(&mesh, &region).unwrap();
        for tri in &result.mesh.triangles {
            let points = tri.map(|i| result.mesh.positions[i as usize]);
            let gaps = tri.map(|i| result.target_gaps[i as usize]);
            for weights in ALL_WEIGHTS {
                let p = (0..3).map(|i| points[i] * weights[i]).sum::<DVec3>();
                let g = (0..3).map(|i| gaps[i] * weights[i]).sum::<f64>();
                let (gap, _) = signed_clearance(p, &closed);
                assert!(gap >= g - 0.001, "{p:?} gap {gap} target {g}");
            }
        }
    }

    #[test]
    fn positive_seal_faces_use_interpolated_boundary_targets() {
        let (mesh, region, margin) = plane();
        let params = InnerSurfaceParams {
            seal_gap: 0.02,
            grid_step: 0.1,
            ..Default::default()
        };
        let surface = inner_surface(&mesh, &region, &margin, DVec3::Z, &params).unwrap();
        assert!(surface.target_gaps[..4].iter().all(|&g| g == 0.0));
        assert!(
            surface.target_gaps[4..]
                .iter()
                .all(|&g| g >= params.seal_gap)
        );
        let mut boundary_edges = 0;
        for tri in &surface.mesh.triangles {
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                let (a, b) = (a as usize, b as usize);
                let midpoint = (surface.mesh.positions[a] + surface.mesh.positions[b]) * 0.5;
                let target = (surface.target_gaps[a] + surface.target_gaps[b]) * 0.5;
                assert!(midpoint.z >= target - 0.001);
                if (a < 4) != (b < 4) && (target - 0.01).abs() < 1e-12 {
                    boundary_edges += 1;
                    assert!((midpoint.z - 0.01).abs() < 1e-12);
                }
            }
        }
        assert!(boundary_edges > 0);
    }

    #[test]
    fn steep_envelope_is_refined_in_surface_space() {
        let die = prepared_stump(&StumpParams::default()).unwrap();
        let frame = Frame::new(die.margin[0], die.axis);
        let polygon = Polygon::new(&die.margin, frame).unwrap();
        let prep = Region::new(&die.mesh, &die.region).unwrap();
        let params = InnerSurfaceParams {
            grid_step: 0.1,
            ..Default::default()
        };
        let mut grid = Grid::new(&polygon, &params).unwrap();
        grid.cast(&polygon, frame, &prep);
        grid.fill_holes().unwrap();
        let (surface, _) = triangulate(&polygon, frame, &grid, &prep).unwrap();
        for tri in &surface.mesh.triangles {
            let heights = tri.map(|i| frame.height(surface.mesh.positions[i as usize]));
            let span = heights.into_iter().fold(f64::NEG_INFINITY, f64::max)
                - heights.into_iter().fold(f64::INFINITY, f64::min);
            assert!(span <= params.grid_step + 1e-6, "axial spacing {span}");
        }
        assert_disk(&surface, die.axis);
    }

    #[test]
    #[ignore]
    fn perf_inner_surface_10mm() {
        let die = prepared_stump(&StumpParams {
            base_radius: 5.0,
            top_radius: 4.6,
            plate_half_width: 7.0,
            ..Default::default()
        })
        .unwrap();
        let start = std::time::Instant::now();
        let result = inner_surface(
            &die.mesh,
            &die.region,
            &die.margin,
            die.axis,
            &InnerSurfaceParams::default(),
        )
        .unwrap();
        let elapsed = start.elapsed().as_secs_f64();
        eprintln!(
            "perf_inner_surface_10mm: {elapsed:.6} s; {} vertices, {} triangles",
            result.mesh.positions.len(),
            result.mesh.triangles.len()
        );
        assert!(elapsed < 1.5, "inner surface exceeded 1.5 s: {elapsed}");
        assert_disk(&result, die.axis);
    }
}
