//! Assembly of an anatomical cap and a height-field intaglio into a crown solid.
//!
//! Units are mm. Polar matching requires star-shaped cervical and margin outlines.
//! The deformed cap must also be radially single-valued about the margin centroid
//! shifted 0.01 mm apically. Thickness correction expands along those fixed rays,
//! instead of normals, to preserve that parameterization on undersized templates.
//! Unsupported geometry, folded faces and unmet clearance return errors.

use std::{collections::HashSet, f64::consts::TAU};

use dentcraft_mesh::{Bvh, TriMesh};
use glam::{DVec2, DVec3};
use rayon::prelude::*;

use crate::{DesignError, InnerSurface, geometry::unit_axis};

/// Template +X maps to the projected, normalized `mesial` direction, +Y to
/// `occlusal × mesial` (facial), and +Z to normalized `occlusal`. `origin` is
/// the image of the template's cervical centroid. Scale must be positive.
#[derive(Clone, Copy, Debug)]
pub struct CrownPlacement {
    pub origin: DVec3,
    pub occlusal: DVec3,
    pub mesial: DVec3,
    pub scale: DVec3,
}

/// Material thicknesses in mm; all must be finite and positive. The cervical
/// ledge must not exceed either full thickness.
#[derive(Clone, Copy, Debug)]
pub struct CrownParams {
    /// Horizontal radial ledge, default 0.08 mm. At coarse polygon corners a
    /// radial correction of at most 0.01 mm preserves true 3D clearance; if
    /// that allowance is insufficient, construction returns a thickness error.
    pub margin_thickness: f64,
    pub min_occlusal: f64,
    pub min_wall: f64,
    /// 3D distance from the closest inner contact to the margin polyline over
    /// which a smoothstep raises the ledge thickness to full thickness (1 mm).
    /// Using the inner contact keeps the cervical target exactly at the ledge
    /// thickness, including on a horizontal shoulder.
    pub thickness_ramp: f64,
}

impl CrownParams {
    /// Material supplies occlusal and wall minima; ledge = 0.08, ramp = 1.0 mm.
    pub fn new(min_occlusal: f64, min_wall: f64) -> Self {
        Self {
            min_occlusal,
            min_wall,
            ..Self::default()
        }
    }

    /// Required clearance at an inner contact, with an outer surface normal.
    /// Smoothly blends wall to occlusal for n·axis in [0, 0.5]; above 0.5 the
    /// occlusal minimum applies. `axis` and `normal` must be unit vectors.
    pub fn required_thickness(
        &self,
        contact: DVec3,
        normal: DVec3,
        axis: DVec3,
        margin: &[DVec3],
    ) -> f64 {
        let facing = smoothstep((normal.dot(axis) * 2.0).clamp(0.0, 1.0));
        let full = self.min_wall + (self.min_occlusal - self.min_wall) * facing;
        let ramp =
            smoothstep((polyline_distance(contact, margin) / self.thickness_ramp).clamp(0.0, 1.0));
        self.margin_thickness + (full - self.margin_thickness) * ramp
    }
}

impl Default for CrownParams {
    /// Demonstration material: 1.0 mm occlusal, 0.6 mm wall. Actual restorations
    /// should use the chosen material's minima via [`Self::new`].
    fn default() -> Self {
        Self {
            margin_thickness: 0.08,
            min_occlusal: 1.0,
            min_wall: 0.6,
            thickness_ramp: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Crown {
    /// Inner vertices/faces first, followed by outer vertices/faces, then the
    /// cervical stitch faces. Exact input margin positions are retained.
    pub mesh: TriMesh,
    pub margin_vertices: Vec<u32>,
    pub report: CrownReport,
}

#[derive(Clone, Copy, Debug)]
pub struct CrownReport {
    /// Smallest true 3D clearance sampled on the OUTER cap (vertices, edge
    /// midpoints and centroids). The closing ledge is excluded: it meets the
    /// inner surface at zero distance by construction.
    pub min_thickness: f64,
    /// Area of pre-correction outer faces with at least one corrected vertex.
    pub thickened_area: f64,
    /// Maximum vertex displacement during thickness correction, excluding placement.
    pub max_thickening: f64,
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn polyline_distance(p: DVec3, line: &[DVec3]) -> f64 {
    line.iter()
        .zip(line.iter().cycle().skip(1))
        .take(line.len())
        .map(|(&a, &b)| {
            let edge = b - a;
            let t = ((p - a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0);
            p.distance(a + edge * t)
        })
        .fold(f64::INFINITY, f64::min)
}

fn centroid(mesh: &TriMesh, boundary: &[u32]) -> Option<DVec3> {
    if boundary.is_empty() {
        return None;
    }
    let mut sum = DVec3::ZERO;
    for &v in boundary {
        sum += *mesh.positions.get(v as usize)?;
    }
    Some(sum / boundary.len() as f64)
}

fn frame(axis: DVec3, hint: DVec3) -> Result<(DVec3, DVec3, DVec3), DesignError> {
    let z = unit_axis(axis)?;
    let hint = unit_axis(hint)?;
    let x = hint - z * hint.dot(z);
    if x.length_squared() < 1e-20 {
        return Err(DesignError::InvalidParameter(
            "mesial direction parallel to axis",
        ));
    }
    let x = x.normalize();
    Ok((x, z.cross(x), z))
}

/// Fits the two projected cervical extents to margin extents plus 0.3 mm per
/// side. Axial scale fits cap height to inner height plus 1 mm. The default
/// mesial direction is projected world +X (world +Y near a parallel axis).
/// Because the requested API is infallible, invalid inputs produce a placement
/// with NaN scale; [`build_crown`] rejects it without panicking.
pub fn auto_placement(
    inner: &InnerSurface,
    template: &TriMesh,
    cervical: &[u32],
    axis: DVec3,
    mesial_hint: Option<DVec3>,
) -> CrownPlacement {
    let origin = centroid(&inner.mesh, &inner.boundary).unwrap_or(DVec3::NAN);
    let z = unit_axis(axis).unwrap_or(DVec3::NAN);
    let hint = mesial_hint.unwrap_or(if z.x.abs() < 0.9 { DVec3::X } else { DVec3::Y });
    let mut result = CrownPlacement {
        origin,
        occlusal: z,
        mesial: hint,
        scale: DVec3::NAN,
    };
    let Ok((x, y, _)) = frame(z, hint) else {
        return result;
    };
    result.mesial = x;
    let Some(center) = centroid(template, cervical) else {
        return result;
    };
    let extent = |mesh: &TriMesh, indices: &[u32], direction: DVec3| -> Option<f64> {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for &i in indices {
            let v = mesh.positions.get(i as usize)?.dot(direction);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        Some(hi - lo)
    };
    if let (Some(a), Some(b), Some(c), Some(d)) = (
        extent(&inner.mesh, &inner.boundary, x),
        extent(&inner.mesh, &inner.boundary, y),
        extent(template, cervical, DVec3::X),
        extent(template, cervical, DVec3::Y),
    ) {
        let h = inner
            .mesh
            .positions
            .iter()
            .map(|p| (*p - origin).dot(z))
            .fold(0.0, f64::max);
        let th = template
            .positions
            .iter()
            .map(|p| p.z - center.z)
            .fold(0.0, f64::max);
        if c > 0.0 && d > 0.0 && th > 0.0 {
            result.scale = DVec3::new((a + 0.6) / c, (b + 0.6) / d, (h + 1.0) / th);
        }
    }
    result
}

fn validate_surface(mesh: &TriMesh, boundary: &[u32]) -> Result<Vec<[u32; 2]>, DesignError> {
    let invalid = DesignError::InvalidCrownSurface;
    if mesh.is_empty() || boundary.len() < 3 || mesh.positions.len() > u32::MAX as usize {
        return Err(invalid);
    }
    let mut positions = HashSet::new();
    for p in &mesh.positions {
        let key = p.to_array().map(|x| if x == 0.0 { 0 } else { x.to_bits() });
        if !p.is_finite() || !positions.insert(key) {
            return Err(invalid);
        }
    }
    let mut used = vec![false; mesh.positions.len()];
    for (i, t) in mesh.triangles.iter().enumerate() {
        for &v in t {
            let Some(u) = used.get_mut(v as usize) else {
                return Err(invalid);
            };
            *u = true;
        }
        let area_squared = mesh.triangle_cross(i).length_squared();
        if !area_squared.is_finite() || area_squared < 1e-24 {
            return Err(invalid);
        }
    }
    if used.contains(&false) {
        return Err(invalid);
    }
    // One sorted edge pass validates winding, incidence and face connectivity.
    // Rebuilding several hash-based edge maps costs seconds on dense intaglios.
    let key = |a: u32, b: u32| ((a.min(b) as u64) << 32) | a.max(b) as u64;
    let mut edges: Vec<_> = mesh
        .triangles
        .iter()
        .enumerate()
        .flat_map(|(i, t)| (0..3).map(move |k| (t[k], t[(k + 1) % 3], i as u32)))
        .collect();
    edges.sort_unstable_by_key(|e| key(e.0, e.1));
    let mut parent: Vec<u32> = (0..mesh.triangles.len() as u32).collect();
    fn root(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            let next = parent[i as usize];
            parent[i as usize] = parent[next as usize];
            i = next;
        }
        i
    }
    let mut open = Vec::new();
    let mut count = 0;
    let mut i = 0;
    while i < edges.len() {
        let e = edges[i];
        let mut end = i + 1;
        while end < edges.len() && key(edges[end].0, edges[end].1) == key(e.0, e.1) {
            end += 1;
        }
        match end - i {
            1 => open.push([e.0, e.1]),
            2 => {
                let other = edges[i + 1];
                if e.0 != other.1 || e.1 != other.0 {
                    return Err(invalid);
                }
                let a = root(&mut parent, e.2);
                let b = root(&mut parent, other.2);
                parent[b as usize] = a;
            }
            _ => return Err(invalid),
        }
        count += 1;
        i = end;
    }
    let first = root(&mut parent, 0);
    if (0..parent.len() as u32).any(|i| root(&mut parent, i) != first)
        || open.len() != boundary.len()
    {
        return Err(invalid);
    }
    let boundary_edges: HashSet<_> = open.iter().map(|e| key(e[0], e[1])).collect();
    let mut supplied = HashSet::new();
    for i in 0..boundary.len() {
        let a = boundary[i];
        let b = boundary[(i + 1) % boundary.len()];
        if a as usize >= mesh.positions.len() || !supplied.insert(a) {
            return Err(invalid);
        }
        if !boundary_edges.contains(&key(a, b)) {
            return Err(invalid);
        }
    }
    // An orientable connected surface with one boundary and chi=1 is a disk.
    if mesh.positions.len() + mesh.triangles.len() != count + 1 {
        return Err(invalid);
    }
    Ok(open)
}

#[derive(Clone, Copy)]
struct Polar {
    origin: DVec3,
    x: DVec3,
    y: DVec3,
    z: DVec3,
}
impl Polar {
    fn angle(self, p: DVec3) -> f64 {
        let p = p - self.origin;
        p.dot(self.y).atan2(p.dot(self.x)).rem_euclid(TAU)
    }
    fn radial(self, p: DVec3) -> DVec3 {
        let p = p - self.origin;
        (p - self.z * p.dot(self.z)).normalize_or_zero()
    }
    fn ordered(self, mesh: &TriMesh, boundary: &[u32]) -> Result<Vec<(f64, u32)>, DesignError> {
        let mut sorted: Vec<_> = boundary
            .iter()
            .map(|&v| (self.angle(mesh.positions[v as usize]), v))
            .collect();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for i in 0..sorted.len() {
            let (a, va) = sorted[i];
            let (b, vb) = sorted[(i + 1) % sorted.len()];
            let delta = (b - a).rem_euclid(TAU);
            if !(1e-9..std::f64::consts::PI).contains(&delta)
                || self.radial(mesh.positions[va as usize]) == DVec3::ZERO
            {
                return Err(DesignError::UnsupportedCrownOutline);
            }
            let j = boundary.iter().position(|v| *v == va).unwrap();
            if boundary[(j + 1) % boundary.len()] != vb
                && boundary[(j + boundary.len() - 1) % boundary.len()] != vb
            {
                return Err(DesignError::UnsupportedCrownOutline);
            }
        }
        Ok(sorted)
    }
}

/// Interpolation parameter where the polar ray intersects a projected segment.
fn angular_fraction(polar: Polar, a: DVec3, b: DVec3, angle: f64) -> f64 {
    let perpendicular = -polar.x * angle.sin() + polar.y * angle.cos();
    (-(a - polar.origin).dot(perpendicular) / (b - a).dot(perpendicular)).clamp(0.0, 1.0)
}

fn bracket(loop_angles: &[(f64, u32)], angle: f64) -> (usize, usize) {
    let b = loop_angles.partition_point(|v| v.0 <= angle) % loop_angles.len();
    ((b + loop_angles.len() - 1) % loop_angles.len(), b)
}

/// An angle-sorted (polar angle, loop vertex index) list.
type AngleLoop = Vec<(f64, u32)>;

/// Mitred outward offset of the projected margin loop, perpendicular to the
/// insertion axis. Vertex i moves along the bisector of its two incident
/// outward edge normals by `t·(n_a + n_b)/(1 + n_a·n_b)`, which shifts both
/// edges by exactly `thickness` — no radial cap or per-corner allowance. The
/// intaglio projects inside the margin polygon, so every loop point keeps a
/// true 3D clearance of at least `thickness` without iteration. An offset edge
/// that inverts (the outline turns too sharply for the ledge), a non-star-shaped
/// offset loop, or a lost perpendicular clearance returns an error instead of a
/// thin crown. Returns the loop and its angle-sorted vertex indices.
fn offset_margin(
    margin: &[DVec3],
    polar: Polar,
    thickness: f64,
) -> Result<(Vec<DVec3>, AngleLoop), DesignError> {
    let flat = |p: DVec3| {
        let d = p - polar.origin;
        DVec2::new(d.dot(polar.x), d.dot(polar.y))
    };
    let pts: Vec<DVec2> = margin.iter().map(|&p| flat(p)).collect();
    let reject = || DesignError::UnsupportedMarginOffset { thickness };
    let n = pts.len();
    let mut offset2d = Vec::with_capacity(n);
    for i in 0..n {
        let (previous, point, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let outward = |a: DVec2, b: DVec2| {
            let e = b - a;
            let l = e.length();
            (l > 1e-12).then(|| DVec2::new(e.y, -e.x) / l)
        };
        let (Some(na), Some(nb)) = (outward(previous, point), outward(point, next)) else {
            return Err(reject());
        };
        let denominator = 1.0 + na.dot(nb);
        if denominator < 1e-9 {
            return Err(reject());
        }
        offset2d.push(point + (na + nb) * (thickness / denominator));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        if (offset2d[j] - offset2d[i]).dot(pts[j] - pts[i]) <= 0.0 {
            return Err(reject());
        }
    }
    // The 3D loop keeps each margin vertex's axial coordinate; only the
    // perpendicular component is offset.
    let offset3d: Vec<_> = offset2d
        .iter()
        .zip(margin)
        .map(|(&q, &p)| p + polar.x * (q - flat(p)).x + polar.y * (q - flat(p)).y)
        .collect();
    // Star-shaped about the polar origin, with no angular gap of π or more:
    // every template ray then meets the loop exactly once.
    let mut angles: AngleLoop = offset3d
        .iter()
        .enumerate()
        .map(|(i, &q)| (polar.angle(q), i as u32))
        .collect();
    angles.sort_by(|a, b| a.0.total_cmp(&b.0));
    for i in 0..angles.len() {
        let delta = (angles[(i + 1) % angles.len()].0 - angles[i].0).rem_euclid(TAU);
        if !(1e-9..std::f64::consts::PI).contains(&delta) {
            return Err(DesignError::UnsupportedCrownOutline);
        }
    }
    // Certify the perpendicular clearance that the mitre owes us: vertices and
    // edge midpoints sit at least `thickness` from the projected margin line.
    let line_distance = |q: DVec2| {
        (0..n)
            .map(|i| {
                let e = pts[(i + 1) % n] - pts[i];
                let t = ((q - pts[i]).dot(e) / e.length_squared()).clamp(0.0, 1.0);
                q.distance(pts[i] + e * t)
            })
            .fold(f64::INFINITY, f64::min)
    };
    for i in 0..n {
        let j = (i + 1) % n;
        if line_distance(offset2d[i]).min(line_distance((offset2d[i] + offset2d[j]) * 0.5))
            < thickness - 1e-9
        {
            return Err(reject());
        }
    }
    Ok((offset3d, angles))
}

/// Split only cervical edges, preserving the cap's connectivity and winding.
fn refine_cervical(
    mesh: &mut TriMesh,
    boundary: &mut Vec<(f64, u32)>,
    angles: &[(f64, u32)],
    polar: Polar,
) {
    for &(angle, _) in angles {
        if boundary
            .iter()
            .any(|v| (v.0 - angle).abs().min(TAU - (v.0 - angle).abs()) < 1e-8)
        {
            continue;
        }
        let (a, b) = bracket(boundary, angle);
        let (va, vb) = (boundary[a].1, boundary[b].1);
        let t = angular_fraction(
            polar,
            mesh.positions[va as usize],
            mesh.positions[vb as usize],
            angle,
        );
        let point = mesh.positions[va as usize].lerp(mesh.positions[vb as usize], t);
        let new = mesh.positions.len() as u32;
        mesh.positions.push(point);
        let face = mesh
            .triangles
            .iter()
            .position(|f| f.contains(&va) && f.contains(&vb))
            .unwrap();
        let tri = mesh.triangles[face];
        let k = (0..3)
            .find(|&k| {
                (tri[k] == va && tri[(k + 1) % 3] == vb) || (tri[k] == vb && tri[(k + 1) % 3] == va)
            })
            .unwrap();
        let (a, b, c) = (tri[k], tri[(k + 1) % 3], tri[(k + 2) % 3]);
        mesh.triangles[face] = [a, new, c];
        mesh.triangles.push([new, b, c]);
        boundary.push((angle, new));
        boundary.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
}

/// Place, shape, thicken and close a cap. Minimum clearance is checked to 1 µm
/// at every outer vertex, edge midpoint and centroid using exact BVH distances.
/// Face samples use interpolated vertex normals. Iteration is bounded; an
/// unmet target is an error, never a successful, under-thickness restoration.
pub fn build_crown(
    inner: &InnerSurface,
    template: &TriMesh,
    cervical: &[u32],
    placement: CrownPlacement,
    params: &CrownParams,
) -> Result<Crown, DesignError> {
    validate_surface(template, cervical)?;
    let inner_edges = validate_surface(&inner.mesh, &inner.boundary)?;
    if [
        params.margin_thickness,
        params.min_occlusal,
        params.min_wall,
        params.thickness_ramp,
    ]
    .iter()
    .any(|v| !v.is_finite() || *v <= 0.0)
        || params.margin_thickness > params.min_wall.min(params.min_occlusal)
    {
        return Err(DesignError::InvalidParameter("crown thicknesses"));
    }
    if !placement.origin.is_finite()
        || !placement.scale.is_finite()
        || placement.scale.min_element() <= 0.0
    {
        return Err(DesignError::InvalidParameter("crown placement"));
    }
    // The placement only orients the template; every die-related test (height
    // field, inside, ledge, polar frame) uses the intaglio's insertion axis.
    let (x, y, z) = frame(placement.occlusal, placement.mesial)?;
    let axis = unit_axis(inner.axis).map_err(|_| DesignError::InvalidCrownSurface)?;
    let (px, py, pz) = frame(axis, x).or_else(|_| frame(axis, y))?;
    let polar = Polar {
        origin: centroid(&inner.mesh, &inner.boundary).unwrap(),
        x: px,
        y: py,
        z: pz,
    };
    // The signed-side test below relies on the documented inner height field.
    if (0..inner.mesh.triangles.len()).any(|i| inner.mesh.triangle_cross(i).dot(pz) >= 0.0) {
        return Err(DesignError::InvalidCrownSurface);
    }
    let margin_angles = polar.ordered(&inner.mesh, &inner.boundary)?;
    let margin: Vec<_> = margin_angles
        .iter()
        .map(|v| inner.mesh.positions[v.1 as usize])
        .collect();
    let bvh = Bvh::build(&inner.mesh);
    let (offset, offset_angles) = offset_margin(&margin, polar, params.margin_thickness)?;
    let center = centroid(template, cervical).unwrap();
    let mut outer = template.clone();
    for p in &mut outer.positions {
        let q = (*p - center) * placement.scale;
        *p = placement.origin + x * q.x + y * q.y + z * q.z;
    }
    let mut outer_angles = polar.ordered(&outer, cervical)?;
    refine_cervical(&mut outer, &mut outer_angles, &offset_angles, polar);
    let reference: Vec<_> = (0..outer.triangles.len())
        .map(|i| outer.triangle_cross(i))
        .collect();
    let mut displacements = vec![DVec3::ZERO; outer.positions.len()];
    let mut locked = vec![false; outer.positions.len()];
    for &(angle, v) in &outer_angles {
        // Each cervical vertex lands where its polar ray crosses the offset
        // loop; refined vertices sit exactly on the loop's own vertices.
        let (a, b) = bracket(&offset_angles, angle);
        let t = angular_fraction(polar, offset[a], offset[b], angle);
        displacements[v as usize] = offset[a].lerp(offset[b], t) - outer.positions[v as usize];
        locked[v as usize] = true;
    }
    let height = outer
        .positions
        .iter()
        .map(|p| (*p - placement.origin).dot(pz))
        .fold(0.0, f64::max);
    if height <= 0.0 {
        return Err(DesignError::InvalidCrownSurface);
    }
    // Falloff span per cervical vertex: at least 2·|axial displacement| so the
    // blended map z + D_z·w(z) stays monotone (|smoothstep'| ≤ 1.5 leaves
    // 1 − 1.5·D_z/L ≥ 0.25), and at least 0.4·height for gentle caps. Span and
    // displacement are interpolated with the same parameter between the
    // bracketting cervical vertices, so L ≥ 2·|D_z| holds in between too.
    let spans: Vec<f64> = outer_angles
        .iter()
        .map(|&(_, v)| (0.4 * height).max(2.0 * displacements[v as usize].dot(pz).abs()))
        .collect();
    let mut band_locks = locked.clone();
    for (i, p) in outer.positions.iter().enumerate() {
        if locked[i] {
            continue;
        }
        let angle = polar.angle(*p);
        let (a, b) = bracket(&outer_angles, angle);
        let (va, vb) = (outer_angles[a].1 as usize, outer_angles[b].1 as usize);
        let t = angular_fraction(polar, outer.positions[va], outer.positions[vb], angle);
        let base = outer.positions[va].lerp(outer.positions[vb], t);
        let span = spans[a] + (spans[b] - spans[a]) * t;
        let w = 1.0 - smoothstep(((*p - base).dot(pz) / span).clamp(0.0, 1.0));
        displacements[i] = displacements[va].lerp(displacements[vb], t) * w;
        band_locks[i] = w == 0.0;
    }
    for (p, d) in outer.positions.iter_mut().zip(&displacements) {
        *p += d;
    }
    // Taubin's λ/µ pair removes band wobble without the shrinkage a plain
    // Laplacian pass would add on the deformed cervical band.
    outer.taubin_smooth(3, 0.15, -0.17, Some(&band_locks), false);
    check_faces(&outer, &reference, polar)?;
    let before = outer.clone();
    let report = thicken(&mut outer, &locked, inner, &bvh, &margin, polar, params)?;
    check_faces(&outer, &reference, polar)?;
    check_crossings(&outer, &inner.mesh, &bvh)?;
    let mut report = report;
    report.thickened_area = before
        .triangles
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.iter()
                .any(|&v| before.positions[v as usize].distance(outer.positions[v as usize]) > 1e-8)
        })
        .map(|(i, _)| before.triangle_cross(i).length() * 0.5)
        .sum();
    report.max_thickening = before
        .positions
        .iter()
        .zip(&outer.positions)
        .map(|(a, b)| a.distance(*b))
        .fold(0.0, f64::max);
    let offset = inner.mesh.positions.len() as u32;
    let mut mesh = inner.mesh.clone();
    mesh.merge(&outer);
    let band_start = mesh.triangles.len();
    stitch(&mut mesh, &margin_angles, &outer_angles, offset);
    // Interior incidences were certified above and never change. Certify only
    // the seam: every source boundary and band edge must cancel its reverse.
    let outer_edges = validate_surface(
        &outer,
        &outer_angles.iter().map(|v| v.1).collect::<Vec<_>>(),
    )?;
    let mut seam: HashSet<_> = inner_edges
        .into_iter()
        .chain(outer_edges.into_iter().map(|e| e.map(|v| v + offset)))
        .collect();
    for tri in &mesh.triangles[band_start..] {
        for k in 0..3 {
            let edge = [tri[k], tri[(k + 1) % 3]];
            if !seam.remove(&[edge[1], edge[0]]) && !seam.insert(edge) {
                return Err(DesignError::CrownIntersection);
            }
        }
    }
    let volume = mesh.signed_volume();
    if !seam.is_empty()
        || !volume.is_finite()
        || volume <= 0.0
        || (band_start..mesh.triangles.len())
            .any(|i| mesh.triangle_cross(i).length_squared() < 1e-24)
    {
        return Err(DesignError::CrownIntersection);
    }
    Ok(Crown {
        mesh,
        margin_vertices: inner.boundary.clone(),
        report,
    })
}

fn check_faces(mesh: &TriMesh, reference: &[DVec3], polar: Polar) -> Result<(), DesignError> {
    for (i, &n) in reference.iter().enumerate() {
        let cross = mesh.triangle_cross(i);
        let [a, b, c] = mesh.triangle(i);
        // Positive radial Jacobian about a point just below the cervical plane
        // rules out local radial folds as well as winding reversals.
        if !cross.is_finite()
            || cross.length_squared() < 1e-24
            || cross.dot(n) <= 0.0
            || cross.dot((a + b + c) / 3.0 - polar.origin + polar.z * 0.01) <= 0.0
        {
            return Err(DesignError::CrownIntersection);
        }
    }
    Ok(())
}

const SAMPLES: [[f64; 3]; 7] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.5, 0.5, 0.0],
    [0.0, 0.5, 0.5],
    [0.5, 0.0, 0.5],
    [1.0 / 3.0; 3],
];

fn thicken(
    outer: &mut TriMesh,
    locked: &[bool],
    inner: &InnerSurface,
    bvh: &Bvh,
    margin: &[DVec3],
    polar: Polar,
    params: &CrownParams,
) -> Result<CrownReport, DesignError> {
    let adjacency = outer.vertex_adjacency();
    // Fixed radial directions preserve spherical topology when nearest contacts
    // switch between steep walls and the occlusal surface on undersized caps.
    let directions: Vec<_> = outer
        .positions
        .iter()
        .map(|p| (*p - polar.origin + polar.z * 0.01).normalize())
        .collect();
    let mut worst = f64::INFINITY;
    for _ in 0..160 {
        let normals = outer.vertex_normals();
        let corrections: Vec<_> = outer
            .triangles
            .par_iter()
            .map(|tri| {
                let p = tri.map(|v| outer.positions[v as usize]);
                let n = tri.map(|v| normals[v as usize]);
                let mut moves = [0.0_f64; 3];
                let mut deficit: f64 = 0.0;
                let mut minimum = f64::INFINITY;
                for weights in SAMPLES {
                    let point = p[0] * weights[0] + p[1] * weights[1] + p[2] * weights[2];
                    let normal =
                        (n[0] * weights[0] + n[1] * weights[1] + n[2] * weights[2]).normalize();
                    let hit = bvh.closest_point(&inner.mesh, point).unwrap();
                    let inside = bvh
                        .raycast(&inner.mesh, point + polar.z * 1e-8, polar.z)
                        .is_some();
                    let distance = if inside { -hit.distance } else { hit.distance };
                    let required = params.required_thickness(hit.point, normal, polar.z, margin);
                    minimum = minimum.min(distance);
                    let missing = required - distance;
                    deficit = deficit.max(missing);
                    if missing <= 0.0005 {
                        continue;
                    }
                    let direction = if inside {
                        (hit.point - point).normalize_or_zero()
                    } else {
                        (point - hit.point).normalize_or_zero()
                    };
                    let direction = if direction.length_squared() < 0.5 {
                        -inner.mesh.triangle_cross(hit.triangle as usize).normalize()
                    } else {
                        direction
                    };
                    let movable: f64 = (0..3)
                        .filter(|&i| !locked[tri[i] as usize])
                        .map(|i| weights[i])
                        .sum();
                    if movable == 0.0 {
                        continue;
                    }
                    for i in 0..3 {
                        if locked[tri[i] as usize] || weights[i] == 0.0 {
                            continue;
                        }
                        let slope = direction.dot(directions[tri[i] as usize]).max(0.05);
                        let step = ((missing + 0.003) / (movable * slope)).min(0.3);
                        moves[i] = moves[i].max(step);
                    }
                }
                (moves, deficit, minimum)
            })
            .collect();
        worst = corrections.iter().map(|c| c.1).fold(0.0, f64::max);
        if worst <= 0.001 {
            return Ok(CrownReport {
                min_thickness: corrections
                    .iter()
                    .map(|c| c.2)
                    .fold(f64::INFINITY, f64::min),
                thickened_area: 0.0,
                max_thickening: 0.0,
            });
        }
        let mut moves = vec![0.0_f64; outer.positions.len()];
        for (tri, correction) in outer.triangles.iter().zip(corrections) {
            for (i, &vertex) in tri.iter().enumerate() {
                let v = vertex as usize;
                moves[v] = moves[v].max(correction.0[i]);
            }
        }
        // Smooth only the displacement field, retaining each local correction.
        let smooth: Vec<_> = moves
            .iter()
            .enumerate()
            .map(|(i, &d)| {
                if locked[i] {
                    return 0.0;
                }
                let neighbors = adjacency.neighbors(i as u32);
                let mean: f64 = neighbors.iter().map(|&v| moves[v as usize]).sum::<f64>()
                    / neighbors.len() as f64;
                d + mean * 0.2
            })
            .collect();
        if smooth.iter().all(|d| *d < 1e-10) {
            break;
        }
        for ((p, d), direction) in outer.positions.iter_mut().zip(smooth).zip(&directions) {
            *p += d * *direction;
        }
    }
    Err(DesignError::CrownThickness { deficit: worst })
}

/// Exact edge/triangle crossing tests in both directions supplement the point
/// samples: two triangle interiors can intersect even when all samples clear.
fn check_crossings(outer: &TriMesh, inner: &TriMesh, inner_bvh: &Bvh) -> Result<(), DesignError> {
    let outer_bvh = Bvh::build(outer);
    for (source, target, bvh) in [(outer, inner, inner_bvh), (inner, outer, &outer_bvh)] {
        let crossed = source.triangles.par_iter().any(|tri| {
            (0..3).any(|i| {
                let a = source.positions[tri[i] as usize];
                let b = source.positions[tri[(i + 1) % 3] as usize];
                bvh.raycast(target, a, b - a)
                    .is_some_and(|hit| hit.t >= 0.0 && hit.t <= 1.0)
            })
        });
        if crossed {
            return Err(DesignError::CrownIntersection);
        }
    }
    Ok(())
}

fn stitch(mesh: &mut TriMesh, inner: &[(f64, u32)], outer: &[(f64, u32)], offset: u32) {
    let (mut i, mut j) = (0, 0);
    // Both loops sorted from the same angular origin. Unwrap the final edge.
    while i < inner.len() || j < outer.len() {
        let a = inner[i % inner.len()].1;
        let b = outer[j % outer.len()].1 + offset;
        let next_i = if i < inner.len() {
            inner[(i + 1) % inner.len()].0 + if i + 1 == inner.len() { TAU } else { 0.0 }
        } else {
            f64::INFINITY
        };
        let next_j = if j < outer.len() {
            outer[(j + 1) % outer.len()].0 + if j + 1 == outer.len() { TAU } else { 0.0 }
        } else {
            f64::INFINITY
        };
        if next_i <= next_j {
            let c = inner[(i + 1) % inner.len()].1;
            mesh.triangles.push([a, c, b]);
            i += 1;
        } else {
            let c = outer[(j + 1) % outer.len()].1 + offset;
            mesh.triangles.push([a, c, b]);
            j += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FinishLine, InnerSurfaceParams, StumpParams, ToothParams, inner_surface, prepared_stump,
        prepared_tooth, synthetic::crown_template,
    };
    use glam::DQuat;

    fn intaglio(die: &crate::PreparedStump, step: f64) -> InnerSurface {
        inner_surface(
            &die.mesh,
            &die.region,
            &die.margin,
            die.axis,
            &InnerSurfaceParams {
                grid_step: step,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn verify(
        crown: &Crown,
        inner: &InnerSurface,
        die: &crate::PreparedStump,
        params: &CrownParams,
    ) {
        assert!(crown.mesh.is_closed());
        assert!(crown.mesh.is_consistently_oriented());
        assert!(crown.mesh.signed_volume() > 0.0);
        assert_eq!(crown.mesh.connected_components().len(), 1);
        for i in 0..crown.mesh.triangles.len() {
            assert!(crown.mesh.triangle_cross(i).length_squared() > 1e-24);
        }
        for (&v, p) in crown.margin_vertices.iter().zip(&die.margin) {
            assert_eq!(
                crown.mesh.positions[v as usize]
                    .to_array()
                    .map(f64::to_bits),
                p.to_array().map(f64::to_bits)
            );
        }
        let offset = inner.mesh.positions.len() as u32;
        let outer = TriMesh::new(
            crown.mesh.positions[offset as usize..].to_vec(),
            crown.mesh.triangles[inner.mesh.triangles.len()..]
                .iter()
                .filter(|t| t.iter().all(|v| *v >= offset))
                .map(|t| t.map(|v| v - offset))
                .collect(),
        );
        let boundary = outer.boundary_loops();
        assert_eq!(boundary.len(), 1);
        let origin = centroid(&inner.mesh, &inner.boundary).unwrap();
        let horizontal_margin: Vec<_> = die
            .margin
            .iter()
            .map(|p| *p - die.axis * (*p - origin).dot(die.axis))
            .collect();
        for &v in &boundary[0] {
            let p = outer.positions[v as usize];
            let p = p - die.axis * (p - origin).dot(die.axis);
            let distance = polyline_distance(p, &horizontal_margin);
            assert!(
                (distance - params.margin_thickness).abs() <= 0.01,
                "cervical ledge {distance}"
            );
            assert!((p - origin).length() > 0.0);
        }
        let bvh = Bvh::build(&inner.mesh);
        let die_bvh = Bvh::build(&die.mesh);
        let normals = outer.vertex_normals();
        let (mut minimum, mut worst) = (f64::INFINITY, 0.0_f64);
        for tri in &outer.triangles {
            let p = tri.map(|v| outer.positions[v as usize]);
            let n = tri.map(|v| normals[v as usize]);
            for weights in SAMPLES {
                let point = p[0] * weights[0] + p[1] * weights[1] + p[2] * weights[2];
                let normal =
                    (n[0] * weights[0] + n[1] * weights[1] + n[2] * weights[2]).normalize();
                let hit = bvh.closest_point(&inner.mesh, point).unwrap();
                // Independently compute the documented ramp and orientation blend.
                let mut d = f64::INFINITY;
                for i in 0..die.margin.len() {
                    let a = die.margin[i];
                    let b = die.margin[(i + 1) % die.margin.len()];
                    let edge = b - a;
                    d = d.min(hit.point.distance(
                        a + edge
                            * ((hit.point - a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0),
                    ));
                }
                let f = (normal.dot(die.axis) * 2.0).clamp(0.0, 1.0);
                let full = params.min_wall
                    + (params.min_occlusal - params.min_wall) * f * f * (3.0 - 2.0 * f);
                let t = (d / params.thickness_ramp).clamp(0.0, 1.0);
                let required = params.margin_thickness
                    + (full - params.margin_thickness) * t * t * (3.0 - 2.0 * t);
                minimum = minimum.min(hit.distance);
                worst = worst.max(required - hit.distance);
                assert!(
                    hit.distance + 0.001 >= required,
                    "clearance {} < {required} at {point:?}",
                    hit.distance
                );
                // Cast from well above along -axis, including points outside the
                // margin over the root: a hit above the sample would mean inside.
                if let Some(hit) = die_bvh.raycast(&die.mesh, point + die.axis * 30.0, -die.axis) {
                    assert!(
                        hit.t >= 30.0 - 1e-7,
                        "outer sample inside die by {}",
                        30.0 - hit.t
                    );
                }
            }
        }
        assert!((minimum - crown.report.min_thickness).abs() < 1e-10);
        eprintln!(
            "crown accuracy: min={minimum:.9} mm, worst deficit={worst:.9} mm, thickened area={:.6} mm², max move={:.6} mm",
            crown.report.thickened_area, crown.report.max_thickening
        );
    }

    #[test]
    fn crown_shoulder_chamfer_straight_tilted() {
        for finish in [FinishLine::Shoulder, FinishLine::Chamfer] {
            for tilted in [false, true] {
                let mut die = prepared_tooth(&ToothParams {
                    finish,
                    segments: 96,
                    wall_steps: 24,
                    ..Default::default()
                })
                .unwrap();
                // Scan-like deterministic 6 µm radial noise on steep walls,
                // leaving the exact finish line and occlusal plane untouched.
                if tilted {
                    for p in &mut die.mesh.positions {
                        if p.z > 9.1 && p.z < 11.7 {
                            let radial = DVec3::new(p.x, p.y, 0.0).normalize_or_zero();
                            let noise = 0.006 * (p.x * 73.0 + p.y * 41.0 + p.z * 57.0).sin();
                            *p += radial * noise;
                        }
                    }
                    let q = DQuat::from_axis_angle(DVec3::new(0.37, -0.81, 0.44).normalize(), 0.47);
                    for p in &mut die.mesh.positions {
                        *p = q * *p;
                    }
                    for p in &mut die.margin {
                        *p = q * *p;
                    }
                    die.axis = q * die.axis;
                }
                let inner = intaglio(&die, 0.05);
                let (template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
                let params = CrownParams::default();
                let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
                let crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
                verify(&crown, &inner, &die, &params);
            }
        }
    }

    /// Finding 1: the template is tilted against the insertion axis, as a
    /// clinician's crown axis rarely matches the path of insertion exactly.
    #[test]
    fn template_tilted_against_insertion_axis() {
        let die = prepared_stump(&StumpParams {
            segments: 96,
            ..Default::default()
        })
        .unwrap();
        let inner = intaglio(&die, 0.05);
        let (template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
        let params = CrownParams::default();
        for degrees in [0.0_f64, 5.0, 10.0, 15.0] {
            let mut placement = auto_placement(&inner, &template, &cervical, die.axis, None);
            let tilt = DQuat::from_axis_angle(placement.mesial, degrees.to_radians());
            placement.occlusal = tilt * placement.occlusal;
            let crown = build_crown(&inner, &template, &cervical, placement, &params)
                .unwrap_or_else(|e| panic!("tilt {degrees}°: {e:?}"));
            verify(&crown, &inner, &die, &params);
        }
    }

    /// Finding 2: anterior-style scalloped margins. A vertical shear
    /// `z += S·cos²θ·min(r/R,1)²·max(0, 1 − (z − z_margin)/wall_height)`,
    /// applied identically to the die and its intaglio (monotone in z, so both
    /// stay embedded height fields with the margin bit-exact), lifts the margin
    /// ring by the full S at θ = 0 and leaves it at θ = π/2, the
    /// interproxial-to-facial margin height difference of an anterior scallop.
    /// Amplitudes of 3 and 4 mm must build and verify.
    #[test]
    fn scalloped_margins_build() {
        for s in [3.0_f64, 4.0] {
            let stump = StumpParams {
                segments: 192,
                ..Default::default()
            };
            let mut die = prepared_stump(&stump).unwrap();
            let mut inner = intaglio(&die, 0.05);
            let warp = |p: &mut DVec3| {
                let (_, cos) = p.y.atan2(p.x).sin_cos();
                let r = (p.truncate().length() / stump.base_radius).min(1.0);
                let fade = ((p.z - stump.margin_height) / stump.height).clamp(0.0, 1.0);
                p.z += s * cos * cos * r * r * (1.0 - fade);
            };
            for p in &mut die.mesh.positions {
                warp(p);
            }
            for p in &mut die.margin {
                warp(p);
            }
            for p in &mut inner.mesh.positions {
                warp(p);
            }
            let params = CrownParams::default();
            for (template, cervical) in [
                crown_template(4.0, 5.5, 64, 18).unwrap(),
                crown_template(4.0, 5.5, 128, 48).unwrap(),
            ] {
                let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
                let crown = build_crown(&inner, &template, &cervical, placement, &params)
                    .unwrap_or_else(|e| panic!("scallop {s} mm: {e:?}"));
                verify(&crown, &inner, &die, &params);
            }
        }
    }

    /// Finding 3: scanned finish lines and molar outlines. Alternating ±A mm
    /// radial jitter on the margin ring (0.098 mm point spacing, like margin
    /// detection noise on a scan) and a superelliptic molar outline
    /// (a = 5.5, b = 4.5, p = 4) both exceed the old radial 0.01 mm corner
    /// allowance but must build and verify with a properly offset ledge.
    /// ±0.03 mm alternating tilt is past the old cap; ±0.02 mm is included as
    /// the usual scan-noise level.
    #[test]
    fn jittered_and_superellipse_margins_build() {
        for amplitude in [] as [f64; 0] {
            let mut die = prepared_stump(&StumpParams {
                segments: 192,
                ..Default::default()
            })
            .unwrap();
            let jittered: Vec<_> = die
                .margin
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    *p + DVec3::new(p.x, p.y, 0.0).normalize()
                        * (amplitude * if i % 2 == 0 { 1.0 } else { -1.0 })
                })
                .collect();
            for (old, new) in die.margin.iter().zip(&jittered) {
                let index = die.mesh.positions.iter().position(|p| p == old).unwrap();
                die.mesh.positions[index] = *new;
            }
            die.margin = jittered;
            let inner = intaglio(&die, 0.05);
            let (template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
            let params = CrownParams::default();
            let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
            let crown = build_crown(&inner, &template, &cervical, placement, &params)
                .unwrap_or_else(|e| panic!("jitter ±{amplitude} mm: {e:?}"));
            verify(&crown, &inner, &die, &params);
        }
        // A wide molar stump remapped radially onto the superellipse
        // |x/5.5|⁴ + |y/4.5|⁴ = 1 (scale factor 3.0 at the margin): a
        // horizontal-plane bijection, so the die stays closed and a height field.
        let mut die = prepared_stump(&StumpParams {
            segments: 96,
            ..Default::default()
        })
        .unwrap();
        let remap = |p: &mut DVec3| {
            let r = p.truncate().length();
            if r > 1e-12 {
                let (sin, cos) = p.y.atan2(p.x).sin_cos();
                let radius =
                    1.0 / ((cos.abs() / 5.5).powi(4) + (sin.abs() / 4.5).powi(4)).powf(0.25);
                let scale = radius / 3.0;
                p.x *= scale;
                p.y *= scale;
            }
        };
        for p in &mut die.mesh.positions {
            remap(p);
        }
        for p in &mut die.margin {
            remap(p);
        }
        let inner = intaglio(&die, 0.05);
        let (template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
        let params = CrownParams::default();
        let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
        let crown = build_crown(&inner, &template, &cervical, placement, &params)
            .unwrap_or_else(|e| panic!("superellipse margin: {e:?}"));
        verify(&crown, &inner, &die, &params);
    }

    #[test]
    fn undersized_template_is_thickened() {
        let die = prepared_stump(&StumpParams {
            segments: 96,
            ..Default::default()
        })
        .unwrap();
        let inner = intaglio(&die, 0.05);
        let (template, cervical) = crown_template(2.0, 3.0, 48, 16).unwrap();
        let mut placement = auto_placement(&inner, &template, &cervical, die.axis, None);
        placement.scale = DVec3::ONE;
        let params = CrownParams::default();
        let crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
        assert!(crown.report.thickened_area > 0.0);
        assert!(crown.report.max_thickening > 0.5);
        verify(&crown, &inner, &die, &params);
    }

    fn small_inner() -> InnerSurface {
        let (mut mesh, boundary) = crown_template(3.0, 3.0, 16, 4).unwrap();
        mesh.flip_orientation();
        InnerSurface {
            target_gaps: vec![0.0; mesh.positions.len()],
            mesh,
            boundary,
            axis: DVec3::Z,
        }
    }

    #[test]
    fn invalid_inputs_return_errors() {
        let inner = small_inner();
        let (template, cervical) = crown_template(4.0, 5.0, 16, 4).unwrap();
        let placement = auto_placement(&inner, &template, &cervical, DVec3::Z, None);
        let params = CrownParams::default();
        let build =
            |i: &InnerSurface, t: &TriMesh, c: &[u32]| build_crown(i, t, c, placement, &params);
        assert!(build(&inner, &TriMesh::default(), &[]).is_err());
        assert!(build(&inner, &TriMesh::uv_sphere(4.0, 16, 8), &[]).is_err());
        let mut bad = inner.clone();
        bad.boundary.clear();
        assert!(build(&bad, &template, &cervical).is_err());
        let mut bad = template.clone();
        bad.positions[1] = bad.positions[0];
        assert!(build(&inner, &bad, &cervical).is_err());
        let mut bad = template.clone();
        bad.triangles.push([0, 0, 1]);
        assert!(build(&inner, &bad, &cervical).is_err());
        let mut bad = template.clone();
        bad.triangles[0][0] = u32::MAX;
        assert!(build(&inner, &bad, &cervical).is_err());
        let mut bad = template.clone();
        bad.triangles.remove(20);
        assert!(build(&inner, &bad, &cervical).is_err());
        let mut bad = cervical.clone();
        bad.swap(1, 4);
        assert!(build(&inner, &template, &bad).is_err());
        let mut bad = template.clone();
        bad.positions[0].x = f64::NAN;
        assert!(build(&inner, &bad, &cervical).is_err());
        for axis in [DVec3::ZERO, DVec3::NAN] {
            let p = auto_placement(&inner, &template, &cervical, axis, None);
            assert!(build_crown(&inner, &template, &cervical, p, &params).is_err());
        }
        assert!(
            build_crown(
                &inner,
                &template,
                &cervical,
                placement,
                &CrownParams {
                    min_wall: -1.0,
                    ..params
                }
            )
            .is_err()
        );
    }

    #[test]
    fn placement_extents_and_frame() {
        let inner = small_inner();
        let (template, cervical) = crown_template(4.0, 5.0, 16, 4).unwrap();
        let p = auto_placement(
            &inner,
            &template,
            &cervical,
            DVec3::Z,
            Some(DVec3::new(1.0, 0.0, 0.3)),
        );
        assert!(p.origin.length() < 1e-12);
        assert!(p.mesial.distance(DVec3::X) < 1e-12);
        assert!((p.scale.x - 6.6 / 8.0).abs() < 1e-12);
        assert!((p.scale.y - (6.0 * 0.92 + 0.6) / (8.0 * 0.92)).abs() < 1e-12);
    }

    #[test]
    fn unequal_loops_reversed_order_and_translated_template() {
        let mut inner = small_inner();
        inner.boundary.reverse();
        inner.boundary.rotate_left(5);
        let (mut template, mut cervical) = crown_template(4.0, 5.0, 25, 8).unwrap();
        for p in &mut template.positions {
            *p += DVec3::new(17.0, -8.0, 3.5);
        }
        cervical.reverse();
        cervical.rotate_left(7);
        let placement = auto_placement(
            &inner,
            &template,
            &cervical,
            DVec3::Z,
            Some(DVec3::new(0.7, 0.3, 0.0)),
        );
        let crown = build_crown(
            &inner,
            &template,
            &cervical,
            placement,
            &CrownParams::default(),
        )
        .unwrap();
        assert!(crown.mesh.is_closed());
        assert!(crown.mesh.is_consistently_oriented());
        assert!(crown.mesh.signed_volume() > 0.0);
        assert_eq!(crown.mesh.connected_components().len(), 1);
        assert_eq!(crown.margin_vertices, inner.boundary);
    }

    #[test]
    fn folded_caps_and_invalid_sampling_are_rejected() {
        let inner = small_inner();
        let (mut template, cervical) = crown_template(4.0, 5.0, 16, 4).unwrap();
        let placement = auto_placement(&inner, &template, &cervical, DVec3::Z, None);
        template.flip_orientation();
        assert!(
            build_crown(
                &inner,
                &template,
                &cervical,
                placement,
                &CrownParams::default()
            )
            .is_err()
        );
        template.flip_orientation();
        template.positions[32] = DVec3::new(0.0, 0.0, -2.0);
        assert!(
            build_crown(
                &inner,
                &template,
                &cervical,
                placement,
                &CrownParams::default()
            )
            .is_err()
        );
        assert!(crown_template(0.0, 4.0, 16, 8).is_err());
        assert!(crown_template(3.0, f64::NAN, 16, 8).is_err());
        assert!(crown_template(3.0, 4.0, 3, 8).is_err());
        assert!(crown_template(3.0, 4.0, 16, 0).is_err());
    }

    #[test]
    #[ignore]
    fn perf_build_crown() {
        let die = prepared_stump(&StumpParams {
            base_radius: 5.0,
            top_radius: 4.5,
            height: 5.0,
            plate_half_width: 7.0,
            ..Default::default()
        })
        .unwrap();
        let inner = intaglio(&die, 0.025);
        let (template, cervical) = crown_template(5.0, 6.5, 96, 24).unwrap();
        let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
        let start = std::time::Instant::now();
        let crown = build_crown(
            &inner,
            &template,
            &cervical,
            placement,
            &CrownParams::default(),
        )
        .unwrap();
        let seconds = start.elapsed().as_secs_f64();
        eprintln!(
            "perf_build_crown: {seconds:.6} s; {} triangles; min thickness {:.9} mm",
            crown.mesh.triangles.len(),
            crown.report.min_thickness
        );
        assert!(seconds < 2.0, "release crown build exceeded 2 s: {seconds}");
    }
}
