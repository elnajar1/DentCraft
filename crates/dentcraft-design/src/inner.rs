//! A constrained height-field intaglio surface, with exact margin positions.

use std::collections::VecDeque;

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
/// and nonnegative; `grid_step` must be finite and positive.
#[derive(Clone, Debug)]
pub struct InnerSurfaceParams {
    /// Full cement gap away from the margin.
    pub cement_gap: f64,
    /// Gap within the seal band; the exact boundary itself always remains on the input margin.
    pub seal_gap: f64,
    /// Width of the seal band, measured in the projected plane.
    pub margin_seal_width: f64,
    /// Width of the smoothstep blend from seal gap to cement gap; zero gives a step.
    pub transition_width: f64,
    /// Spacing of the envelope and ball-offset grid.
    pub grid_step: f64,
}

impl Default for InnerSurfaceParams {
    fn default() -> Self {
        Self {
            cement_gap: 0.05,
            seal_gap: 0.0,
            margin_seal_width: 0.8,
            transition_width: 0.5,
            grid_step: 0.025,
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
}

/// Block out undercuts by first-hit rays from above the region, then offset the
/// sampled envelope by a varying-radius ball and triangulate inside the projected margin.
///
/// Seal distance is **2D projected distance**, an approximation to distance along the
/// surface. The ball maximum uses grid samples, so sloped-surface gap accuracy depends
/// on `grid_step`; flat areas are exact up to roundoff. This is not a continuous offset.
/// Missing interior hits inherit the nearest valid grid sample in four-neighbour graph
/// distance (deterministic breadth-first propagation, constrained to the polygon).
/// A disconnected unsampled grid island, or no hits at all, returns `NoSurfaceSamples`.
///
/// Margin vertices remain exact, even for a nonzero seal gap or an undercut margin.
/// Samples within half a grid step of the boundary are omitted from triangulation.
/// The polygon must be simple; clockwise and counter-clockwise order are accepted.
/// Empty/all-degenerate regions and zero/nonfinite axes return errors. Zero-area
/// triangles are ignored. At most two million bounding-box grid cells and 200 million
/// offset stencil visits are allowed, to reject impractical allocations before work.
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
    let mut grid = Grid::new(&polygon, params)?;
    grid.cast(&polygon, frame, &prep);
    grid.fill_holes()?;
    let heights = grid.offset(params);
    triangulate(&polygon, frame, &grid, &heights)
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
    heights: &[f64],
) -> Result<InnerSurface, DesignError> {
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
            mesh.positions.push(frame.lift(p, heights[i]));
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
    Ok(InnerSurface {
        mesh,
        boundary: (0..n as u32).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let polygon = Polygon::new(&die.margin, Frame::new(DVec3::ZERO, DVec3::Z)).unwrap();
        let (mut top_count, mut seal_count) = (0, 0);
        let (mut top_error, mut seal_error) = (0.0_f64, 0.0_f64);
        for &point in &result.mesh.positions[die.margin.len()..] {
            let distance = polygon.distance(point.truncate());
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
    fn divergent_stump_is_a_height_field() {
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
        let result =
            inner_surface(&die.mesh, &die.region, &die.margin, die.axis * 3.0, &params).unwrap();
        assert_disk(&result, die.axis);
        for (&i, &point) in result.boundary.iter().zip(&die.margin) {
            assert_eq!(result.mesh.positions[i as usize], point);
        }
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
        for step in [0.0, -1.0, f64::NAN] {
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
            grid_step: 0.15,
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
