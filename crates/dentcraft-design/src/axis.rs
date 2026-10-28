//! Region-only visibility and a draft-centred insertion-axis search.

use std::f64::consts::TAU;

use dentcraft_mesh::TriMesh;
use glam::DVec3;
use rayon::prelude::*;

use crate::{
    DesignError,
    geometry::{Frame, Region, unit_axis},
};

/// Block-out distances for vertices used by the requested region.
#[derive(Clone, Debug)]
pub struct UndercutMap {
    /// Original mesh vertex indices, sorted ascending, including vertices of zero-area faces.
    pub vertices: Vec<u32>,
    /// Distance in mm along the normalized axis to the first non-grazing region hit;
    /// zero for a reachable vertex. Indexed in parallel with `vertices`.
    pub depths: Vec<f64>,
    /// Sum of areas in mm² of nondegenerate region triangles with occluded centroids.
    pub undercut_area: f64,
}

/// The best sampled direction, with metrics recomputed using every region triangle.
#[derive(Clone, Copy, Debug)]
pub struct AxisResult {
    /// Unit insertion direction, pointing away from the preparation.
    pub axis: DVec3,
    /// Occluded centroid area in mm².
    pub undercut_area: f64,
    /// Angle in degrees from the normalized initial axis.
    pub tilt_deg: f64,
    /// Area-weighted fifth percentile of axial-wall draft angles, in degrees.
    /// Zero if there are no walls within 60 degrees of perpendicular to this axis.
    pub min_draft_deg: f64,
}

/// Area-weighted unit normal, or `None` for invalid indices, nonfinite geometry,
/// an empty/zero-area region, or cancelling normals. Duplicate triangle indices count once.
pub fn average_normal(mesh: &TriMesh, region: &[u32]) -> Option<DVec3> {
    let mut indices = region.to_vec();
    indices.sort_unstable();
    indices.dedup();
    let mut sum = DVec3::ZERO;
    let mut total = 0.0;
    for i in indices {
        let t = mesh.triangles.get(i as usize)?;
        let [a, b, c] = t.map(|v| mesh.positions.get(v as usize).copied());
        let n = (b? - a?).cross(c? - a?);
        if !n.is_finite() {
            return None;
        }
        sum += n;
        total += n.length();
    }
    if sum.length() <= total * 1e-12 {
        None
    } else {
        unit_axis(sum).ok()
    }
}

/// Cast along `axis` against only the prep region. Axes are normalized; empty,
/// all-degenerate and invalid regions, and zero/nonfinite axes, return errors.
/// Duplicate faces count once; isolated zero-area faces do not occlude or add area.
/// Hits within `max(1e-9 mm, region_diagonal * 1e-8)` and tangent-wall hits are ignored.
/// Edge hits must cover a four-point projected neighbourhood at four times that
/// tolerance, so a straight wall touching a chamfer silhouette is not classified as blocked.
pub fn undercut_map(
    mesh: &TriMesh,
    region: &[u32],
    axis: DVec3,
) -> Result<UndercutMap, DesignError> {
    let axis = unit_axis(axis)?;
    let prep = Region::new(mesh, region)?;
    let depths = prep
        .vertices
        .par_iter()
        .map(|&i| prep.depth(mesh.positions[i as usize], axis))
        .collect();
    let undercut_area = samples(&prep.mesh)
        .par_iter()
        .filter(|s| prep.depth(s.point, axis) > 0.0)
        .map(|s| s.area)
        .sum();
    Ok(UndercutMap {
        vertices: prep.vertices,
        depths,
        undercut_area,
    })
}

/// Search a cone of `max_tilt_deg` (0 through 90 degrees) around `initial`, or
/// the region's average normal when omitted. Minimize occluded area, then maximize
/// the fifth-percentile wall draft, then minimize tilt. This is a sampled search,
/// not a global-optimum guarantee for arbitrary nonconvex preparations.
///
/// A deterministic equal-area spiral and five local refinements evaluate up to
/// 2048 stratified area-weighted centroid samples in parallel. The returned metrics
/// use every triangle. Area ties allow relative roundoff of 1e-10 of region area;
/// draft ties allow 1e-8 degrees. The supplied initial axis is always considered.
/// Each batch tries its best-draft direction first: if it has zero undercut, it
/// already attains the area lower bound and the other ray casts can be skipped.
pub fn best_insertion_axis(
    mesh: &TriMesh,
    region: &[u32],
    initial: Option<DVec3>,
    max_tilt_deg: f64,
) -> Result<AxisResult, DesignError> {
    if !max_tilt_deg.is_finite() || !(0.0..=90.0).contains(&max_tilt_deg) {
        return Err(DesignError::InvalidParameter(
            "max_tilt_deg must be in [0, 90]",
        ));
    }
    let prep = Region::new(mesh, region)?;
    let average = average_normal(mesh, region);
    let initial = unit_axis(initial.or(average).ok_or(DesignError::InvalidAxis)?)?;
    let all = samples(&prep.mesh);
    let sampled = stratified(&all, 2048);
    let area_tolerance = all.iter().map(|s| s.area).sum::<f64>() * 1e-10;
    let limit = max_tilt_deg.to_radians();
    let frame = Frame::new(DVec3::ZERO, initial);
    let mut directions = vec![initial];
    if let Some(normal) = average.filter(|n| n.dot(initial) >= limit.cos()) {
        directions.push(normal);
    }
    if limit > 0.0 {
        for i in 1..=96 {
            let angle = (1.0 - (1.0 - limit.cos()) * i as f64 / 96.0).acos();
            directions.push(direction(frame, angle, i as f64 * 2.399_963_229_728_653));
        }
    }
    let mut best = choose(&prep, &sampled, &directions, initial, area_tolerance);
    let mut step = limit / 5.0;
    for _ in 0..5 {
        if step == 0.0 {
            break;
        }
        let frame = Frame::new(DVec3::ZERO, best.axis);
        let mut directions = vec![best.axis];
        for radius in [0.5, 1.0] {
            for i in 0..16 {
                let d = direction(frame, step * radius, TAU * i as f64 / 16.0);
                if d.dot(initial) >= limit.cos() - 1e-14 {
                    directions.push(d);
                }
            }
        }
        best = choose(&prep, &sampled, &directions, initial, area_tolerance);
        step *= 0.35;
    }
    Ok(evaluate(&prep, &all, best.axis, initial))
}

#[derive(Clone, Copy)]
struct Sample {
    point: DVec3,
    normal: DVec3,
    area: f64,
}

fn samples(mesh: &TriMesh) -> Vec<Sample> {
    (0..mesh.triangles.len())
        .map(|i| {
            let [a, b, c] = mesh.triangle(i);
            let cross = mesh.triangle_cross(i);
            Sample {
                point: a + ((b - a) + (c - a)) / 3.0,
                normal: cross.normalize(),
                area: cross.length() * 0.5,
            }
        })
        .collect()
}

fn stratified(all: &[Sample], count: usize) -> Vec<Sample> {
    if all.len() <= count {
        return all.to_vec();
    }
    let weight = all.iter().map(|s| s.area).sum::<f64>() / count as f64;
    let mut result: Vec<Sample> = Vec::with_capacity(count);
    let (mut index, mut accumulated) = (0, all[0].area);
    for i in 0..count {
        let target = (i as f64 + 0.5) * weight;
        while accumulated < target && index + 1 < all.len() {
            index += 1;
            accumulated += all[index].area;
        }
        result.push(Sample {
            area: weight,
            ..all[index]
        });
    }
    result
}

fn direction(frame: Frame, angle: f64, azimuth: f64) -> DVec3 {
    (frame.w * angle.cos() + (frame.u * azimuth.cos() + frame.v * azimuth.sin()) * angle.sin())
        .normalize()
}

fn evaluate(prep: &Region, samples: &[Sample], axis: DVec3, initial: DVec3) -> AxisResult {
    let mut result = draft_metrics(samples, axis, initial);
    result.undercut_area = samples
        .par_iter()
        .filter(|s| prep.depth(s.point, axis) > 0.0)
        .map(|s| s.area)
        .sum();
    result
}

fn draft_metrics(samples: &[Sample], axis: DVec3, initial: DVec3) -> AxisResult {
    let mut walls = Vec::with_capacity(samples.len());
    for s in samples {
        let cosine = s.normal.dot(axis).clamp(-1.0, 1.0);
        if cosine.abs() <= 3.0_f64.sqrt() / 2.0 {
            walls.push((cosine, s.area));
        }
    }
    walls.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let target = walls.iter().map(|s| s.1).sum::<f64>() * 0.05;
    let mut accumulated = 0.0;
    let mut draft = 0.0;
    for (cosine, weight) in walls {
        accumulated += weight;
        if accumulated >= target {
            draft = cosine.asin().to_degrees();
            break;
        }
    }
    AxisResult {
        axis,
        undercut_area: 0.0,
        tilt_deg: axis.dot(initial).clamp(-1.0, 1.0).acos().to_degrees(),
        min_draft_deg: draft,
    }
}

fn choose(
    prep: &Region,
    samples: &[Sample],
    directions: &[DVec3],
    initial: DVec3,
    tolerance: f64,
) -> AxisResult {
    let metrics: Vec<_> = directions
        .par_iter()
        .map(|&d| draft_metrics(samples, d, initial))
        .collect();
    let best_draft = metrics
        .iter()
        .copied()
        .reduce(|a, b| prefer(a, b, tolerance))
        .unwrap();
    let baseline = evaluate(prep, samples, best_draft.axis, initial);
    if baseline.undercut_area == 0.0 {
        return baseline;
    }
    // Early rejection is valid because occluded area is a sum of positive weights.
    // Indexed collection plus serial selection keeps approximate ties deterministic.
    metrics
        .par_iter()
        .map(|&metric| {
            if metric.axis == baseline.axis {
                return baseline;
            }
            let mut result = metric;
            for sample in samples {
                if prep.depth(sample.point, metric.axis) > 0.0 {
                    result.undercut_area += sample.area;
                    if result.undercut_area > baseline.undercut_area + tolerance {
                        break;
                    }
                }
            }
            result
        })
        .collect::<Vec<_>>()
        .into_iter()
        .fold(baseline, |best, next| prefer(best, next, tolerance))
}

fn prefer(best: AxisResult, next: AxisResult, tolerance: f64) -> AxisResult {
    if next.undercut_area < best.undercut_area - tolerance
        || ((next.undercut_area - best.undercut_area).abs() <= tolerance
            && (next.min_draft_deg > best.min_draft_deg + 1e-8
                || ((next.min_draft_deg - best.min_draft_deg).abs() <= 1e-8
                    && next.tilt_deg < best.tilt_deg)))
    {
        next
    } else {
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StumpParams, prepared_stump};
    use glam::DQuat;

    #[test]
    fn tapered_stump_is_reachable_and_draft_centred() {
        let die = prepared_stump(&StumpParams::default()).unwrap();
        let map = undercut_map(&die.mesh, &die.region, DVec3::Z).unwrap();
        assert_eq!(map.undercut_area, 0.0);
        assert!(map.depths.iter().all(|&d| d == 0.0));
        assert!(
            average_normal(&die.mesh, &die.region)
                .unwrap()
                .distance(DVec3::Z)
                < 1e-12
        );
        let result = best_insertion_axis(&die.mesh, &die.region, None, 20.0).unwrap();
        assert!(result.axis.distance(DVec3::Z) < 0.01);
        assert_eq!(result.undercut_area, 0.0);
        assert!((result.min_draft_deg - 6.0).abs() < 0.01);
    }

    #[test]
    fn tilted_stump_search_finds_its_axis() {
        let die = prepared_stump(&StumpParams {
            tilt_deg: 15.0,
            ..Default::default()
        })
        .unwrap();
        assert!(
            undercut_map(&die.mesh, &die.region, DVec3::Z)
                .unwrap()
                .undercut_area
                > 0.0
        );
        let result = best_insertion_axis(&die.mesh, &die.region, Some(DVec3::Z), 30.0).unwrap();
        let error = result
            .axis
            .dot(die.axis)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        eprintln!(
            "tilted stump: axis error {error:.6} deg, draft {:.6} deg",
            result.min_draft_deg
        );
        assert!(error < 2.0, "{result:?}, error={error}");
        assert_eq!(result.undercut_area, 0.0);
        assert!((result.tilt_deg - 15.0).abs() < 2.0);
    }

    #[test]
    fn divergent_stump_is_undercut_with_analytic_base_depth() {
        let params = StumpParams {
            top_radius: 3.5,
            ..Default::default()
        };
        let die = prepared_stump(&params).unwrap();
        for angle in [0.0_f64, 2.5, 5.0, 7.5, 10.0] {
            for azimuth in 0..8 {
                let axis = DQuat::from_rotation_z(azimuth as f64 * TAU / 8.0)
                    * DQuat::from_rotation_x(angle.to_radians())
                    * DVec3::Z;
                assert!(
                    undercut_map(&die.mesh, &die.region, axis)
                        .unwrap()
                        .undercut_area
                        > 0.0
                );
            }
        }
        let map = undercut_map(&die.mesh, &die.region, DVec3::Z).unwrap();
        for p in &die.margin {
            let i = map
                .vertices
                .iter()
                .position(|&i| die.mesh.positions[i as usize] == *p)
                .unwrap();
            assert!(
                (map.depths[i] - params.height).abs() < 0.02,
                "{}",
                map.depths[i]
            );
        }
        assert!(
            best_insertion_axis(&die.mesh, &die.region, None, 10.0)
                .unwrap()
                .undercut_area
                > 0.0
        );
    }

    #[test]
    fn nonunit_axes_and_degenerate_open_regions() {
        let die = prepared_stump(&StumpParams::default()).unwrap();
        let a = undercut_map(&die.mesh, &die.region, DVec3::Z).unwrap();
        let b = undercut_map(&die.mesh, &die.region, DVec3::Z * 7.0).unwrap();
        assert_eq!(a.depths, b.depths);
        assert!(matches!(
            undercut_map(&die.mesh, &[], DVec3::Z),
            Err(DesignError::EmptyRegion)
        ));
        assert!(matches!(
            undercut_map(&die.mesh, &die.region, DVec3::ZERO),
            Err(DesignError::InvalidAxis)
        ));
        let mesh = TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::X],
            vec![[0, 1, 2], [0, 1, 3]],
        );
        assert!(average_normal(&mesh, &[1]).is_none());
        assert!(matches!(
            undercut_map(&mesh, &[1], DVec3::Z),
            Err(DesignError::DegenerateRegion)
        ));
        let map = undercut_map(&mesh, &[0, 1, 0], DVec3::Z).unwrap();
        assert_eq!(map.vertices, vec![0, 1, 2, 3]);
        assert_eq!(map.undercut_area, 0.0);
        assert_eq!(average_normal(&mesh, &[0, 0, 1]), Some(DVec3::Z));
        assert!(matches!(
            best_insertion_axis(&mesh, &[0], None, 91.0),
            Err(DesignError::InvalidParameter(_))
        ));
        let result = best_insertion_axis(&mesh, &[0], Some(DVec3::Z * 2.0), 0.0).unwrap();
        assert_eq!(result.axis, DVec3::Z);
        assert_eq!(result.min_draft_deg, 0.0);
    }

    #[test]
    fn straight_walls_do_not_count_as_undercuts() {
        let die = prepared_stump(&StumpParams {
            top_radius: 3.0,
            tilt_deg: 15.0,
            ..Default::default()
        })
        .unwrap();
        let map = undercut_map(&die.mesh, &die.region, die.axis).unwrap();
        assert_eq!(map.undercut_area, 0.0);
        assert!(map.depths.iter().all(|&d| d == 0.0));
    }

    #[test]
    fn only_region_faces_occlude_and_area_counts_once() {
        let mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::Y,
                DVec3::new(-1.0, -1.0, 2.0),
                DVec3::new(2.0, -1.0, 2.0),
                DVec3::new(2.0, 2.0, 2.0),
                DVec3::new(-1.0, 2.0, 2.0),
            ],
            vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        );
        let exposed = undercut_map(&mesh, &[0, 1], DVec3::Z).unwrap();
        assert_eq!(exposed.undercut_area, 0.0);
        assert!(exposed.depths.iter().all(|&d| d == 0.0));
        let blocked = undercut_map(&mesh, &[0, 1, 2, 3, 0], DVec3::Z * 5.0).unwrap();
        assert_eq!(blocked.undercut_area, 1.0);
        for &depth in &blocked.depths[..4] {
            assert!((depth - 2.0).abs() < 1e-12);
        }
        assert!(blocked.depths[4..].iter().all(|&d| d == 0.0));
        assert!(average_normal(&TriMesh::default(), &[]).is_none());
        assert!(matches!(
            undercut_map(&TriMesh::default(), &[], DVec3::Z),
            Err(DesignError::EmptyRegion)
        ));
    }

    #[test]
    fn pruned_selection_matches_exhaustive_area_then_draft_ranking() {
        for top_radius in [2.6, 3.5] {
            let die = prepared_stump(&StumpParams {
                top_radius,
                segments: 96,
                wall_steps: 2,
                ..Default::default()
            })
            .unwrap();
            let prep = Region::new(&die.mesh, &die.region).unwrap();
            let samples = samples(&prep.mesh);
            let directions: Vec<_> = [-12.0_f64, -5.0, 0.0, 3.0, 15.0]
                .iter()
                .map(|a| DQuat::from_rotation_x(a.to_radians()) * DVec3::Z)
                .collect();
            let tolerance = samples.iter().map(|s| s.area).sum::<f64>() * 1e-10;
            let brute = directions
                .iter()
                .map(|&d| evaluate(&prep, &samples, d, DVec3::Z))
                .reduce(|a, b| prefer(a, b, tolerance))
                .unwrap();
            let pruned = choose(&prep, &samples, &directions, DVec3::Z, tolerance);
            assert_eq!(brute.axis, pruned.axis);
            assert!((brute.undercut_area - pruned.undercut_area).abs() < tolerance);
            assert_eq!(brute.min_draft_deg, pruned.min_draft_deg);
        }
    }

    #[test]
    #[ignore]
    fn perf_axis_50k() {
        let die = prepared_stump(&StumpParams {
            segments: 256,
            wall_steps: 96,
            tilt_deg: 15.0,
            ..Default::default()
        })
        .unwrap();
        let start = std::time::Instant::now();
        let result = best_insertion_axis(&die.mesh, &die.region, Some(DVec3::Z), 30.0).unwrap();
        eprintln!("axis {} triangles: {:?}", die.region.len(), start.elapsed());
        assert_eq!(result.undercut_area, 0.0);
        assert!(result.axis.dot(die.axis).acos().to_degrees() < 2.0);
        assert!(start.elapsed().as_secs_f64() < 0.5);
    }
}
