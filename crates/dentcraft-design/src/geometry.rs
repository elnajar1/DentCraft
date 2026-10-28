//! Private validation and frame helpers, kept here rather than changing dentcraft-mesh.

use std::collections::BTreeSet;

use dentcraft_mesh::{Bvh, RayHit, TriMesh};
use glam::{DVec2, DVec3};

use crate::DesignError;

pub(crate) fn unit_axis(axis: DVec3) -> Result<DVec3, DesignError> {
    if !axis.is_finite() || axis == DVec3::ZERO {
        return Err(DesignError::InvalidAxis);
    }
    // Scaling first also handles very large and subnormal finite vectors.
    Ok((axis / axis.abs().max_element())
        .normalize()
        .map(|v| if v == 0.0 { 0.0 } else { v }))
}

pub(crate) struct Region {
    pub mesh: TriMesh,
    pub vertices: Vec<u32>,
    pub bvh: Bvh,
    pub tolerance: f64,
    pub min_undercut_depth: f64,
    diagonal: f64,
}

impl Region {
    pub fn new(mesh: &TriMesh, region: &[u32]) -> Result<Self, DesignError> {
        if region.is_empty() {
            return Err(DesignError::EmptyRegion);
        }
        let mut triangles = BTreeSet::new();
        let mut vertices = BTreeSet::new();
        for &index in region {
            let tri = mesh
                .triangles
                .get(index as usize)
                .ok_or(DesignError::InvalidMesh)?;
            for &v in tri {
                let p = mesh
                    .positions
                    .get(v as usize)
                    .ok_or(DesignError::InvalidMesh)?;
                if !p.is_finite() {
                    return Err(DesignError::InvalidMesh);
                }
                vertices.insert(v);
            }
            let area = mesh.triangle_cross(index as usize).length();
            if !area.is_finite() {
                return Err(DesignError::InvalidMesh);
            }
            if area > 0.0 {
                triangles.insert(index);
            }
        }
        if triangles.is_empty() {
            return Err(DesignError::DegenerateRegion);
        }
        let mesh = mesh.extract_triangles(&triangles.into_iter().collect::<Vec<_>>());
        let diagonal = mesh.bounds().unwrap().diagonal();
        let tolerance = (diagonal * 1e-8).max(1e-9);
        let bvh = Bvh::build(&mesh);
        Ok(Self {
            mesh,
            vertices: vertices.into_iter().collect(),
            bvh,
            tolerance,
            min_undercut_depth: 0.03,
            diagonal,
        })
    }

    fn axial_depth(&self, point: DVec3, axis: DVec3) -> f64 {
        // Starting beyond the exclusion interval ignores near self hits without letting
        // them hide a more distant occluder. Distances remain relative to the original point.
        let mut travel = self.tolerance;
        while let Some(hit) = self.bvh.raycast(&self.mesh, point + axis * travel, axis) {
            let normal = self.mesh.triangle_cross(hit.triangle as usize).normalize();
            travel += hit.t;
            if normal.dot(axis).abs() > 1e-9 && self.covers_ray(hit, axis) {
                return travel;
            }
            // A numerically grazing hit on a planar wall is not an undercut.
            travel += self.tolerance;
            if travel > self.diagonal * 2.0 {
                break;
            }
        }
        0.0
    }

    /// Surveyor depth perpendicular to the insertion axis, only for shadowed points.
    /// Follow the horizontal outward normal; for axial normals use the direction
    /// away from the region centre. Bracket in 2 µm increments doubled up to 1 mm,
    /// then bisect to <= 1 µm. Values still shadowed at 1 mm saturate at 1 mm.
    pub fn horizontal_depth(&self, point: DVec3, axis: DVec3, normal: DVec3) -> f64 {
        if self.axial_depth(point, axis) == 0.0 {
            return 0.0;
        }
        let mut outward = normal - axis * normal.dot(axis);
        if outward.length_squared() < 1e-16 {
            let radial = point - self.mesh.bounds().unwrap().center();
            outward = radial - axis * radial.dot(axis);
        }
        let outward = outward
            .try_normalize()
            .unwrap_or_else(|| Frame::new(point, axis).u);
        let mut lo = 0.0;
        let mut hi = 0.002_f64;
        while self.axial_depth(point + outward * hi, axis) > 0.0 {
            if hi >= 1.0 {
                return 1.0;
            }
            lo = hi;
            hi = (hi * 2.0).min(1.0);
        }
        while hi - lo > 0.001 {
            let mid = (lo + hi) * 0.5;
            if self.axial_depth(point + outward * mid, axis) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    }

    pub fn is_undercut(&self, point: DVec3, axis: DVec3, normal: DVec3) -> bool {
        let depth = self.horizontal_depth(point, axis, normal);
        depth > 0.0 && depth >= self.min_undercut_depth
    }

    fn covers_ray(&self, hit: RayHit, axis: DVec3) -> bool {
        if hit.barycentric.min_element() > 1e-7 {
            return true;
        }
        // A ray travelling along a straight wall can touch the chamfer's silhouette
        // edge. Such a touch is not block-out. Check the projected neighbourhood of
        // edge hits; genuine shared interior edges remain covered by adjacent faces.
        let frame = Frame::new(hit.point, axis);
        let epsilon = self.tolerance * 4.0;
        let origin = hit.point - axis * epsilon * 8.0;
        [frame.u, -frame.u, frame.v, -frame.v]
            .iter()
            .all(|&offset| {
                self.bvh
                    .raycast(&self.mesh, origin + offset * epsilon, axis)
                    .is_some_and(|other| other.t <= epsilon * 16.0)
            })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub origin: DVec3,
    pub u: DVec3,
    pub v: DVec3,
    pub w: DVec3,
}

impl Frame {
    pub fn new(origin: DVec3, w: DVec3) -> Self {
        let helper = if w.x.abs() < 0.8 { DVec3::X } else { DVec3::Y };
        let u = (helper - w * helper.dot(w)).normalize();
        Self {
            origin,
            u,
            v: w.cross(u),
            w,
        }
    }

    pub fn project(self, point: DVec3) -> DVec2 {
        let p = point - self.origin;
        DVec2::new(p.dot(self.u), p.dot(self.v))
    }

    pub fn height(self, point: DVec3) -> f64 {
        (point - self.origin).dot(self.w)
    }

    pub fn lift(self, point: DVec2, height: f64) -> DVec3 {
        self.origin + self.u * point.x + self.v * point.y + self.w * height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip_and_extreme_axes() {
        for axis in [
            DVec3::Z,
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::X * 1e300,
            DVec3::Y * 1e-300,
        ] {
            let w = unit_axis(axis).unwrap();
            let frame = Frame::new(DVec3::splat(10.0), w);
            let p = DVec3::new(2.0, 5.0, 9.0);
            assert!(p.distance(frame.lift(frame.project(p), frame.height(p))) < 1e-12);
        }
        assert_eq!(unit_axis(DVec3::ZERO), Err(DesignError::InvalidAxis));
        assert_eq!(unit_axis(DVec3::NAN), Err(DesignError::InvalidAxis));
    }

    #[test]
    fn region_rejects_bad_indices_and_positions() {
        let mut mesh = TriMesh::new(vec![DVec3::ZERO, DVec3::X, DVec3::Y], vec![[0, 1, 2]]);
        assert!(matches!(
            Region::new(&mesh, &[1]),
            Err(DesignError::InvalidMesh)
        ));
        mesh.positions[1] = DVec3::NAN;
        assert!(matches!(
            Region::new(&mesh, &[0]),
            Err(DesignError::InvalidMesh)
        ));
        mesh.triangles[0][1] = 99;
        assert!(matches!(
            Region::new(&mesh, &[0]),
            Err(DesignError::InvalidMesh)
        ));
    }
}
