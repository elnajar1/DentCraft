//! Regressions for the inner-surface and undercut review findings. Each failed on
//! b0bc7c3 (seal band over the walls, sampled-ball under-offset, faces inside the
//! die, accepted margin undercut, noise-sensitive undercut classification).

use dentcraft_mesh::Bvh;
use glam::DVec3;

use crate::{
    DesignError, InnerSurfaceParams, StumpParams, best_insertion_axis, geometry::Region,
    inner_surface, prepared_stump, undercut_map,
};

fn margin_distance(point: DVec3, margin: &[DVec3]) -> f64 {
    margin
        .iter()
        .zip(margin.iter().cycle().skip(1))
        .map(|(&a, &b)| {
            let t = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
            point.distance(a + t * (b - a))
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn review_wall_gap_0049() {
    let die = prepared_stump(&StumpParams::default()).unwrap();
    let params = InnerSurfaceParams {
        cement_gap: 0.049,
        margin_seal_width: 0.0,
        transition_width: 0.0,
        grid_step: 0.025,
        ..Default::default()
    };
    let surface = inner_surface(&die.mesh, &die.region, &die.margin, die.axis, &params).unwrap();
    let prep = die.mesh.extract_triangles(&die.region);
    let bvh = Bvh::build(&prep);
    let gaps: Vec<_> = surface.mesh.positions[die.margin.len()..]
        .iter()
        .map(|&p| bvh.closest_point(&prep, p).unwrap())
        .filter(|hit| hit.point.z > 1.0 && hit.point.z < 4.85 - 1e-8)
        .map(|hit| hit.distance)
        .collect();
    assert!(gaps.len() > 100);
    let min = gaps.iter().copied().fold(f64::INFINITY, f64::min);
    let avg = gaps.iter().sum::<f64>() / gaps.len() as f64;
    eprintln!("review wall gap: min={min:.9}, avg={avg:.9} mm");
    assert!(min >= 0.045, "wall minimum gap {min:.9} < 0.045 mm");
}

#[test]
fn review_default_face_clearance() {
    let die = prepared_stump(&StumpParams::default()).unwrap();
    let params = InnerSurfaceParams::default();
    let surface = inner_surface(&die.mesh, &die.region, &die.margin, die.axis, &params).unwrap();
    let bvh = Bvh::build(&die.mesh);
    let mut max_penetration = 0.0_f64;
    let mut max_deficit = 0.0_f64;
    for i in 0..surface.mesh.triangles.len() {
        let [a, b, c] = surface.mesh.triangle(i);
        let [ga, gb, gc] = surface.mesh.triangles[i].map(|v| surface.target_gaps[v as usize]);
        for (point, g) in [
            ((a + b) * 0.5, (ga + gb) * 0.5),
            ((b + c) * 0.5, (gb + gc) * 0.5),
            ((c + a) * 0.5, (gc + ga) * 0.5),
            ((a + b + c) / 3.0, (ga + gb + gc) / 3.0),
        ] {
            let hit = bvh.closest_point(&die.mesh, point).unwrap();
            // The preparation above this planar margin is convex. The outward
            // closest-face normal therefore supplies the sign of the distance.
            let normal = die.mesh.triangle_cross(hit.triangle as usize);
            let signed = if (point - hit.point).dot(normal) < 0.0 {
                -hit.distance
            } else {
                hit.distance
            };
            max_penetration = max_penetration.max(-signed);
            // Decision 3: clearance targets interpolate the vertex targets,
            // including an exact zero at the margin with positive seal gaps.
            max_deficit = max_deficit.max(g - signed);
        }
    }
    eprintln!("review faces: penetration={max_penetration:.9}, gap deficit={max_deficit:.9} mm");
    assert!(
        max_penetration <= 0.001,
        "face penetration {max_penetration:.9} > 0.001 mm"
    );
    assert!(
        max_deficit <= 0.001,
        "face gap deficit {max_deficit:.9} > 0.001 mm"
    );
}

#[test]
fn review_surface_distance_bands() {
    let die = prepared_stump(&StumpParams::default()).unwrap();
    let params = InnerSurfaceParams::default();
    let surface = inner_surface(&die.mesh, &die.region, &die.margin, die.axis, &params).unwrap();
    let prep = die.mesh.extract_triangles(&die.region);
    let bvh = Bvh::build(&prep);
    let mut full_count = 0;
    let mut seal_count = 0;
    let mut full_error = 0.0_f64;
    let mut seal_error = 0.0_f64;
    for &point in &surface.mesh.positions[die.margin.len()..] {
        let hit = bvh.closest_point(&prep, point).unwrap();
        if hit.point.z >= 4.85 - 1e-8 {
            continue;
        }
        let distance = margin_distance(hit.point, &die.margin);
        if distance >= 1.4 {
            full_count += 1;
            full_error = full_error.max((hit.distance - params.cement_gap).abs());
        } else if distance <= 0.8 {
            seal_count += 1;
            seal_error = seal_error.max((hit.distance - params.seal_gap).abs());
        }
    }
    eprintln!("review bands: full error={full_error:.9}, seal error={seal_error:.9} mm");
    assert!(full_count > 100 && seal_count > 100);
    assert!(
        full_error <= 0.005,
        "wall full-gap error {full_error:.9} > 0.005 mm"
    );
    assert!(
        seal_error <= 0.005,
        "wall seal-gap error {seal_error:.9} > 0.005 mm"
    );
}

#[test]
fn review_margin_undercut_rejected() {
    let die = prepared_stump(&StumpParams {
        top_radius: 3.5,
        ..Default::default()
    })
    .unwrap();
    let result = inner_surface(
        &die.mesh,
        &die.region,
        &die.margin,
        die.axis,
        &InnerSurfaceParams::default(),
    );
    assert!(
        matches!(result, Err(DesignError::MarginUndercut { .. })),
        "expected MarginUndercut, got {result:?}"
    );
}

#[test]
fn review_noisy_stump_area_and_axis() {
    let mut die = prepared_stump(&StumpParams {
        segments: 384,
        wall_steps: 77,
        ..Default::default()
    })
    .unwrap();
    // Approximately 0.05 mm tessellation; reproducible independent uniform
    // radial noise in [-0.010, 0.010] mm on wall vertices above the margin.
    let mut state = 0x5eed_cafe_u64;
    for p in &mut die.mesh.positions {
        if p.z > 1.0 && p.z < 4.85 + 1e-8 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let noise = ((state >> 11) as f64 / ((1_u64 << 53) as f64) * 2.0 - 1.0) * 0.010;
            let radial = DVec3::new(p.x, p.y, 0.0).normalize();
            *p += radial * noise;
        }
    }
    let prep = Region::new(&die.mesh, &die.region).unwrap();
    let mut total = 0.0;
    for i in 0..prep.mesh.triangles.len() {
        let area = prep.mesh.triangle_cross(i).length() * 0.5;
        total += area;
    }
    let map = undercut_map(&die.mesh, &die.region, die.axis).unwrap();
    let best = best_insertion_axis(&die.mesh, &die.region, Some(die.axis), 20.0).unwrap();
    let error = best.axis.dot(die.axis).clamp(-1.0, 1.0).acos().to_degrees();
    eprintln!(
        "review noise (default horizontal threshold 0.030 mm): area={:.9}/{total:.9} mm² ({:.6}%); axis error={error:.6} deg",
        map.undercut_area,
        map.undercut_area / total * 100.0
    );
    assert!(
        map.undercut_area < total * 0.01,
        "noise undercut area {:.9} >= 1% of {total:.9} mm²",
        map.undercut_area
    );
    assert!(error < 2.0, "noisy axis error {error:.6} >= 2 degrees");
}
