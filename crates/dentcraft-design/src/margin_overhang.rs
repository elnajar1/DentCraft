//! One margin seating metric shared by axis ranking and inner-surface rejection.

use dentcraft_mesh::TriMesh;
use glam::DVec3;

use crate::{
    DesignError,
    geometry::{Frame, Region, unit_axis},
    polygon::Polygon,
};

/// Worst sampled horizontal extension beyond the projected margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarginOverhang {
    /// Horizontal distance to the margin polygon, in mm (zero when contained).
    pub depth: f64,
    /// Azimuth around the margin centroid in degrees [0, 360). Zero is projected
    /// +X (or +Y when |axis.x| >= 0.8), increasing toward axis cross that vector.
    /// Location fields are meaningful only when `depth` is positive.
    pub sector_deg: f64,
    /// Region point responsible for the maximum.
    pub point: DVec3,
    /// Closest point on the projected margin, interpolated in 3D.
    pub margin_point: DVec3,
    /// Axial height of `point` above `margin_point`, in mm.
    pub height_above_margin: f64,
}

/// Measure the same seating obstruction as `inner_surface`, including region
/// vertices, triangle edge midpoints and centroids. Samples within
/// [`SEAL_BAND_MM`] of the margin line axially are ignored: that is the
/// margin's own vicinity (the crossing zone of the barrier and the smooth
/// curve), not an undercut. This is a sampling metric, not a continuous
/// maximum guarantee for arbitrary concave outlines.
pub fn margin_overhang(
    mesh: &TriMesh,
    region: &[u32],
    margin: &[DVec3],
    axis: DVec3,
) -> Result<MarginOverhang, DesignError> {
    let axis = unit_axis(axis)?;
    let prep = Region::new(mesh, region)?;
    measure(&prep, margin, axis)
}

pub(crate) fn measure(
    prep: &Region,
    margin: &[DVec3],
    axis: DVec3,
) -> Result<MarginOverhang, DesignError> {
    let frame = Frame::new(*margin.first().ok_or(DesignError::InvalidMargin)?, axis);
    let polygon = Polygon::new(margin, frame)?;
    Ok(measure_polygon(prep, &polygon, frame))
}

/// Axial height relative to the margin within which seating samples count as
/// the margin's own vicinity, not an undercut: the region barrier and the
/// smooth margin curve cross each other within about one triangle on either
/// side of the margin line, so a sample in this band is that crossing, not
/// geometry the crown would have to ride over. Further below the margin a
/// cervical skirt still obstructs seating and is measured.
pub const SEAL_BAND_MM: f64 = 0.2;

pub(crate) fn measure_polygon(prep: &Region, polygon: &Polygon, frame: Frame) -> MarginOverhang {
    let center = polygon.margin.iter().sum::<DVec3>() / polygon.margin.len() as f64;
    let floor = polygon
        .margin
        .iter()
        .map(|&p| frame.height(p))
        .fold(f64::INFINITY, f64::min);
    let mut worst = MarginOverhang {
        depth: 0.0,
        sector_deg: 0.0,
        point: center,
        margin_point: center,
        height_above_margin: 0.0,
    };
    let mut visit = |p: DVec3| {
        let q = frame.project(p);
        if frame.height(p) <= floor + prep.tolerance || polygon.contains(q) {
            return;
        }
        let mut distance = f64::INFINITY;
        let mut nearest = center;
        for (i, (a, b)) in polygon.edges().enumerate() {
            let t = ((q - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
            let d = q.distance_squared(a + (b - a) * t);
            if d < distance {
                distance = d;
                nearest = polygon.margin[i].lerp(polygon.margin[(i + 1) % polygon.margin.len()], t);
            }
        }
        let depth = distance.sqrt();
        // The seal band around the margin line (± SEAL_BAND_MM axially) is the
        // margin's own vicinity; a skirt further below or an overhang above the
        // band still counts as a seating obstruction.
        if depth > worst.depth
            && depth > prep.tolerance
            && (p - nearest).dot(frame.w).abs() > SEAL_BAND_MM
        {
            let d = p - center;
            worst = MarginOverhang {
                depth,
                sector_deg: d
                    .dot(frame.v)
                    .atan2(d.dot(frame.u))
                    .to_degrees()
                    .rem_euclid(360.0),
                point: p,
                margin_point: nearest,
                height_above_margin: (p - nearest).dot(frame.w),
            };
        }
    };
    for &p in &prep.mesh.positions {
        visit(p);
    }
    for i in 0..prep.mesh.triangles.len() {
        let [a, b, c] = prep.mesh.triangle(i);
        for p in [
            (a + b) * 0.5,
            (b + c) * 0.5,
            (c + a) * 0.5,
            (a + b + c) / 3.0,
        ] {
            visit(p);
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InnerSurfaceParams, StumpParams, best_insertion_axis_with_margin, inner_surface,
        prepared_stump,
    };
    use glam::DQuat;

    #[test]
    fn seating_precedes_draft_and_tilt_even_when_all_rays_are_visible() {
        let mesh = TriMesh {
            positions: vec![
                DVec3::new(2.7, -0.2, 2.),
                DVec3::new(3.3, 0., 2.),
                DVec3::new(2.7, 0.2, 2.),
            ],
            triangles: vec![[0, 1, 2]],
        };
        let margin = [
            DVec3::new(-3., -3., 0.),
            DVec3::new(3., -3., 0.),
            DVec3::new(3., 3., 0.),
            DVec3::new(-3., 3., 0.),
        ];
        let legacy = crate::best_insertion_axis(&mesh, &[0], Some(DVec3::Z), 30.).unwrap();
        assert_eq!(legacy.tilt_deg, 0.0);
        assert_eq!(legacy.undercut_area, 0.0);
        assert!(
            margin_overhang(&mesh, &[0], &margin, legacy.axis)
                .unwrap()
                .depth
                > 0.29
        );
        let best =
            best_insertion_axis_with_margin(&mesh, &[0], &margin, Some(DVec3::Z), 30.).unwrap();
        assert_eq!(best.undercut_area, 0.0);
        assert_eq!(best.margin_overhang.unwrap().depth, 0.0);
        assert!(best.tilt_deg > 8.0 && best.tilt_deg < 10.0);
        assert!(best_insertion_axis_with_margin(&mesh, &[0], &[], Some(DVec3::Z), 30.).is_err());
        assert!(best_insertion_axis_with_margin(&mesh, &[], &margin, Some(DVec3::Z), 30.).is_err());
        assert!(
            best_insertion_axis_with_margin(&mesh, &[0], &margin, Some(DVec3::Z), f64::NAN)
                .is_err()
        );
    }

    #[test]
    fn a_margin_overhang_within_the_cement_gap_is_scan_noise() {
        // One triangle reaching `depth` past the right side of a 3 mm square margin.
        let undercut = |depth: f64| {
            let margin =
                [(0., 0.), (3., 0.), (3., 3.), (0., 3.)].map(|(x, y)| DVec3::new(x, y, 0.));
            let mesh = TriMesh {
                positions: vec![
                    DVec3::new(0.5, 0.5, 1.),
                    DVec3::new(3. + depth, 1.5, 1.),
                    DVec3::new(0.5, 2.5, 1.),
                ],
                triangles: vec![[0, 1, 2]],
            };
            let params = InnerSurfaceParams::default();
            matches!(
                inner_surface(&mesh, &[0], &margin, DVec3::Z, &params),
                Err(DesignError::MarginUndercut { .. })
            )
        };
        assert!(!undercut(0.04));
        assert!(undercut(0.06));
    }

    #[test]
    fn triangle_interiors_cross_a_concave_margin() {
        // Every triangle vertex lies inside this U; its upper edge crosses the
        // missing notch. A vertices-only overhang check incorrectly reports zero.
        let margin = [
            (0., 0.),
            (3., 0.),
            (3., 3.),
            (2., 3.),
            (2., 1.),
            (1., 1.),
            (1., 3.),
            (0., 3.),
        ]
        .map(|(x, y)| DVec3::new(x, y, 0.));
        let mesh = TriMesh {
            positions: vec![
                DVec3::new(0.5, 2.5, 1.),
                DVec3::new(2.5, 2.5, 1.),
                DVec3::new(1.5, 0.5, 1.),
            ],
            triangles: vec![[0, 1, 2]],
        };
        let result = margin_overhang(&mesh, &[0], &margin, DVec3::Z).unwrap();
        assert!((result.depth - 0.5).abs() < 1e-12);
        assert!((result.height_above_margin - 1.0).abs() < 1e-12);
        assert!(
            matches!(inner_surface(&mesh,&[0],&margin,DVec3::Z,&InnerSurfaceParams::default()), Err(DesignError::MarginUndercut{depth,..}) if (depth-0.5).abs()<1e-12)
        );
    }

    #[test]
    fn divergent_margin_stays_actionable_after_cone_search() {
        let die = prepared_stump(&StumpParams {
            segments: 96,
            wall_steps: 2,
            top_radius: 3.5,
            top_chamfer: 0.01,
            ..Default::default()
        })
        .unwrap();
        let best = best_insertion_axis_with_margin(
            &die.mesh,
            &die.region,
            &die.margin,
            Some(die.axis),
            30.,
        )
        .unwrap();
        let m = best.margin_overhang.unwrap();
        assert!(m.depth > 0.48 && m.depth < 0.5);
        assert!(best.tilt_deg < 0.1);
        assert!((0.0..360.0).contains(&m.sector_deg));
        assert!(m.height_above_margin > 3.9);
        assert_eq!(
            inner_surface(
                &die.mesh,
                &die.region,
                &die.margin,
                best.axis,
                &InnerSurfaceParams::default()
            )
            .unwrap_err(),
            DesignError::MarginUndercut {
                depth: m.depth,
                sector_deg: m.sector_deg
            }
        );
    }

    #[test]
    fn margin_search_handles_rotated_noisy_open_region_and_degeneracy() {
        let mut die = prepared_stump(&StumpParams {
            segments: 96,
            wall_steps: 4,
            tilt_deg: 19.,
            ..Default::default()
        })
        .unwrap();
        let rotation = DQuat::from_axis_angle(DVec3::new(1., 2., 3.).normalize(), 0.37);
        for (i, p) in die.mesh.positions.iter_mut().enumerate() {
            // Include scan noise while keeping the exact cervical edge unchanged.
            if !die.margin.contains(p) {
                *p += DVec3::new(
                    (i as f64 * 1.7).sin(),
                    (i as f64 * 2.3).cos(),
                    (i as f64 * 0.9).sin(),
                ) * 0.002;
            }
            *p = rotation * *p;
        }
        for p in &mut die.margin {
            *p = rotation * *p;
        }
        die.axis = rotation * die.axis;
        let duplicate = die.mesh.positions.len() as u32;
        die.mesh.positions.push(die.mesh.positions[0]);
        die.region.push(die.mesh.triangles.len() as u32);
        die.mesh.triangles.push([duplicate; 3]);
        die.region.push(die.region[0]);
        let initial = rotation * DVec3::Z;
        let best = best_insertion_axis_with_margin(
            &die.mesh,
            &die.region,
            &die.margin,
            Some(initial),
            30.,
        )
        .unwrap();
        assert!(best.tilt_deg <= 30.0 + 1e-10);
        assert_eq!(best.margin_overhang.unwrap().depth, 0.0);
        let inner = inner_surface(
            &die.mesh,
            &die.region,
            &die.margin,
            best.axis,
            &InnerSurfaceParams {
                grid_step: 0.08,
                ..Default::default()
            },
        )
        .unwrap();
        let prep = Region::new(&die.mesh, &die.region).unwrap();
        for i in 0..inner.mesh.triangles.len() {
            let ids = inner.mesh.triangles[i].map(|v| v as usize);
            for weights in [
                [1., 0., 0.],
                [0., 1., 0.],
                [0., 0., 1.],
                [0.5, 0.5, 0.],
                [0., 0.5, 0.5],
                [0.5, 0., 0.5],
                [1. / 3.; 3],
            ] {
                let p = (0..3)
                    .map(|j| inner.mesh.positions[ids[j]] * weights[j])
                    .sum::<DVec3>();
                let target = (0..3)
                    .map(|j| inner.target_gaps[ids[j]] * weights[j])
                    .sum::<f64>();
                let hit = prep.bvh.closest_point(&prep.mesh, p).unwrap();
                let normal = prep.mesh.triangle_cross(hit.triangle as usize).normalize();
                let signed = p.distance(hit.point)
                    * if (p - hit.point).dot(normal) < 0. {
                        -1.
                    } else {
                        1.
                    };
                assert!(signed >= target - 0.001, "{signed} < {target}");
            }
        }
        assert_eq!(
            margin_overhang(&die.mesh, &[], &die.margin, die.axis),
            Err(DesignError::EmptyRegion)
        );
        assert_eq!(
            margin_overhang(
                &die.mesh,
                &[die.region[die.region.len() - 2]],
                &die.margin,
                die.axis
            ),
            Err(DesignError::DegenerateRegion)
        );
        assert_eq!(
            margin_overhang(&die.mesh, &die.region, &[], die.axis),
            Err(DesignError::InvalidMargin)
        );
        assert_eq!(
            margin_overhang(&die.mesh, &die.region, &die.margin, DVec3::ZERO),
            Err(DesignError::InvalidAxis)
        );
    }
}
