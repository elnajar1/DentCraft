use dentcraft_design::{
    AdaptParams, Crown, CrownParams, InnerSurface, InnerSurfaceParams, PreparedStump, StumpParams,
    adapt_to_neighbours, auto_placement, build_crown, inner_surface, prepared_stump,
    signed_distance_map, synthetic::crown_template,
};
use dentcraft_mesh::{Bvh, TriMesh};
use glam::{DQuat, DVec3};

mod verification {
    include!("support/crown.rs");
}

fn plane(center: DVec3, normal: DVec3, half: f64) -> TriMesh {
    let x = normal.any_orthonormal_vector();
    let y = normal.cross(x);
    TriMesh::new(
        vec![
            center - x * half - y * half,
            center + x * half - y * half,
            center + x * half + y * half,
            center - x * half + y * half,
        ],
        vec![[0, 1, 2], [0, 2, 3]],
    )
}

fn tooth(center: DVec3, subdivisions: usize) -> TriMesh {
    let mut mesh =
        dentcraft_design::synthetic::rounded_box(DVec3::new(2.0, 3.0, 4.0), 0.5, subdivisions)
            .unwrap();
    for p in &mut mesh.positions {
        *p += center;
    }
    mesh
}

fn scene() -> (PreparedStump, InnerSurface, Crown, CrownParams) {
    scene_resolution(0.08, 64, 18)
}

fn scene_resolution(
    step: f64,
    segments: usize,
    rings: usize,
) -> (PreparedStump, InnerSurface, Crown, CrownParams) {
    let die = prepared_stump(&StumpParams {
        segments: 96,
        wall_steps: 12,
        ..Default::default()
    })
    .unwrap();
    let inner = inner_surface(
        &die.mesh,
        &die.region,
        &die.margin,
        die.axis,
        &InnerSurfaceParams {
            grid_step: step,
            ..Default::default()
        },
    )
    .unwrap();
    let (template, cervical) = crown_template(4.0, 5.5, segments, rings).unwrap();
    let params = CrownParams::new(0.8, 0.5);
    let mut placement = auto_placement(&inner, &template, &cervical, die.axis, None);
    placement.scale.x *= 1.35;
    placement.scale.y *= 1.35;
    placement.scale.z *= 1.12;
    let crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
    (die, inner, crown, params)
}

#[test]
fn distance_map_sphere_plane_and_invalid_queries() {
    let mut sphere = TriMesh::uv_sphere(1.0, 64, 32);
    let normal = DVec3::new(0.31, -0.47, 0.83).normalize();
    let center = DVec3::new(0.2, -0.1, 0.35);
    for p in &mut sphere.positions {
        *p += center;
    }
    let other = plane(DVec3::ZERO, normal, 10.0);
    let vertices: Vec<_> = (0..sphere.positions.len() as u32).collect();
    let map = signed_distance_map(&sphere, &vertices, &other, &Bvh::build(&other));
    for (&d, p) in map.iter().zip(&sphere.positions) {
        assert!((d - p.dot(normal)).abs() < 0.001);
    }
    assert!(map.iter().any(|&d| d < 0.0) && map.iter().any(|&d| d > 0.0));
    assert!(signed_distance_map(&sphere, &[u32::MAX], &other, &Bvh::build(&other))[0].is_nan());
    assert!(signed_distance_map(&sphere, &[0], &TriMesh::default(), &Bvh::default())[0].is_nan());
    assert!(signed_distance_map(&sphere, &[], &other, &Bvh::build(&other)).is_empty());
}

#[test]
fn scene_contacts_and_grid_thickness() {
    let (die, inner, mut crown, params) = scene();
    let before = crown.mesh.positions.clone();
    let outer = &before[inner.mesh.positions.len()..];
    let right = outer.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let left = outer.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let top = outer.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
    let mesial = tooth(DVec3::new(right + 0.4 + 2.0, 0.0, 3.0), 16);
    let distal = tooth(DVec3::new(left + 0.3 - 2.0, 0.0, 3.0), 16);
    let antagonist = plane(DVec3::Z * (top - 0.2), -DVec3::Z, 20.0);
    let report = adapt_to_neighbours(
        &mut crown,
        &inner,
        &params,
        &AdaptParams::default(),
        Some(&mesial),
        Some(&distal),
        Some(&antagonist),
    )
    .unwrap();
    eprintln!("contact scene: {report:?}");
    assert!((report.mesial_contact.unwrap() + 0.02).abs() <= 0.005);
    assert!((report.distal_contact.unwrap() + 0.02).abs() <= 0.005);
    assert!(report.min_occlusal_distance.unwrap() >= -0.001);
    assert!(report.thickness_ok);
    assert!(report.moved_area > 0.0);
    assert_eq!(
        &before[..inner.mesh.positions.len()],
        &crown.mesh.positions[..inner.mesh.positions.len()]
    );
    assert!(
        before
            .iter()
            .zip(&crown.mesh.positions)
            .all(|(a, b)| a.distance(*b) <= 1.0 + 1e-10)
    );
    verification::verify(&crown, &inner, &die, &params);
}

#[test]
fn growth_has_compact_support() {
    let (die, inner, mut crown, params) = scene();
    let before = crown.mesh.positions.clone();
    let offset = inner.mesh.positions.len();
    let seed = *before[offset..]
        .iter()
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap();
    let mesial = plane(DVec3::X * (seed.x + 0.4), -DVec3::X, 20.0);
    let adapt = AdaptParams::default();
    let report = adapt_to_neighbours(
        &mut crown,
        &inner,
        &params,
        &adapt,
        Some(&mesial),
        None,
        None,
    )
    .unwrap();
    assert!((report.mesial_contact.unwrap() + 0.02).abs() <= 0.005);
    let mut unchanged = 0;
    for (a, b) in before.iter().zip(&crown.mesh.positions) {
        if a.distance(seed) >= adapt.influence_radius {
            assert_eq!(
                a.to_array().map(f64::to_bits),
                b.to_array().map(f64::to_bits)
            );
            unchanged += 1;
        }
    }
    assert!(unchanged > offset);
    verification::verify(&crown, &inner, &die, &params);
}

#[test]
fn impossible_occlusion_preserves_thickness() {
    let (die, inner, _, params) = scene();
    let (mut template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
    // A flat occlusal table gives an exactly controlled 0.8 mm test clearance.
    for p in &mut template.positions {
        p.z = p.z.min(4.0);
    }
    let mut placement = auto_placement(&inner, &template, &cervical, die.axis, None);
    placement.scale.x *= 1.35;
    placement.scale.y *= 1.35;
    let top_inner = inner
        .mesh
        .positions
        .iter()
        .map(|p| p.z)
        .fold(f64::NEG_INFINITY, f64::max);
    placement.scale.z = (top_inner + 0.8 - placement.origin.z) / 4.0;
    let mut crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
    let inner_bvh = Bvh::build(&inner.mesh);
    let normals = crown.mesh.vertex_normals();
    let top_inner = inner
        .mesh
        .positions
        .iter()
        .map(|p| p.z)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut occlusal_thickness = f64::INFINITY;
    for tri in &crown.mesh.triangles {
        if tri
            .iter()
            .any(|&v| (v as usize) < inner.mesh.positions.len())
        {
            continue;
        }
        let p = tri.map(|v| crown.mesh.positions[v as usize]);
        let n = tri.map(|v| normals[v as usize]);
        let steps = ((0..3)
            .map(|i| p[i].distance(p[(i + 1) % 3]))
            .fold(0.0, f64::max)
            / 0.1)
            .ceil()
            .max(3.0) as usize;
        for i in 0..=steps {
            for j in 0..=steps - i {
                let w = [
                    i as f64 / steps as f64,
                    j as f64 / steps as f64,
                    (steps - i - j) as f64 / steps as f64,
                ];
                let point = p[0] * w[0] + p[1] * w[1] + p[2] * w[2];
                let normal = (n[0] * w[0] + n[1] * w[1] + n[2] * w[2]).normalize();
                let hit = inner_bvh.closest_point(&inner.mesh, point).unwrap();
                if normal.dot(inner.axis) > 0.5 && hit.point.z > top_inner - 0.01 {
                    occlusal_thickness = occlusal_thickness.min(hit.distance);
                }
            }
        }
    }
    assert!(
        (occlusal_thickness - 0.8).abs() <= 0.001,
        "occlusal thickness {occlusal_thickness}"
    );
    let top = crown
        .mesh
        .positions
        .iter()
        .map(|p| p.z)
        .fold(f64::NEG_INFINITY, f64::max);
    let antagonist = plane(DVec3::Z * (top - 1.2), -DVec3::Z, 20.0);
    let report = adapt_to_neighbours(
        &mut crown,
        &inner,
        &params,
        &AdaptParams {
            max_move: 2.0,
            ..Default::default()
        },
        None,
        None,
        Some(&antagonist),
    )
    .unwrap();
    eprintln!("blocked occlusion: {report:?}");
    assert!(!report.thickness_ok);
    assert!(report.min_occlusal_distance.unwrap() < -0.001);
    verification::verify(&crown, &inner, &die, &params);
}

#[test]
fn no_scans_and_invalid_inputs_are_transactional() {
    let (_, inner, mut crown, params) = scene();
    let before = crown.mesh.positions.clone();
    let report = adapt_to_neighbours(
        &mut crown,
        &inner,
        &params,
        &AdaptParams::default(),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(before, crown.mesh.positions);
    assert_eq!(report.moved_area, 0.0);
    assert!(report.mesial_contact.is_none() && report.min_occlusal_distance.is_none());
    let empty = TriMesh::default();
    let zero = TriMesh::new(vec![DVec3::ZERO; 3], vec![[0, 1, 2]]);
    let invalid = TriMesh::new(vec![DVec3::ZERO], vec![[0, 1, 4]]);
    for other in [&empty, &zero, &invalid] {
        assert!(
            adapt_to_neighbours(
                &mut crown,
                &inner,
                &params,
                &AdaptParams::default(),
                Some(other),
                None,
                None
            )
            .is_err()
        );
        assert_eq!(before, crown.mesh.positions);
    }
    let mut invalid_crown = crown.clone();
    invalid_crown.mesh = empty;
    assert!(
        adapt_to_neighbours(
            &mut invalid_crown,
            &inner,
            &params,
            &AdaptParams::default(),
            None,
            None,
            None
        )
        .is_err()
    );
}

#[test]
fn rotated_noisy_scene() {
    let (mut die, mut inner, mut crown, params) = scene();
    let q = DQuat::from_axis_angle(DVec3::new(0.37, -0.81, 0.44).normalize(), 0.47);
    let top = crown
        .mesh
        .positions
        .iter()
        .map(|p| p.z)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut antagonist = plane(DVec3::Z * (top - 0.2), -DVec3::Z, 20.0);
    // Scan-like noise is applied to the opposing surface, with an oblique frame.
    antagonist.positions[0].z += 0.006;
    antagonist.positions[2].z -= 0.006;
    for mesh in [
        &mut die.mesh,
        &mut inner.mesh,
        &mut crown.mesh,
        &mut antagonist,
    ] {
        for p in &mut mesh.positions {
            *p = q * *p;
        }
    }
    for p in &mut die.margin {
        *p = q * *p;
    }
    die.axis = q * die.axis;
    inner.axis = q * inner.axis;
    let report = adapt_to_neighbours(
        &mut crown,
        &inner,
        &params,
        &AdaptParams::default(),
        None,
        None,
        Some(&antagonist),
    )
    .unwrap();
    assert!(
        report.min_occlusal_distance.unwrap() >= -0.001,
        "{report:?}"
    );
    verification::verify(&crown, &inner, &die, &params);
}

#[test]
fn rounded_box_is_closed_and_rejects_degenerate_dimensions() {
    let mesh = dentcraft_design::synthetic::rounded_box(DVec3::splat(2.0), 0.5, 16).unwrap();
    assert!(mesh.is_closed() && mesh.is_consistently_oriented() && mesh.signed_volume() > 0.0);
    for (size, radius, subdivisions) in [
        (DVec3::ZERO, 0.5, 16),
        (DVec3::splat(2.0), 2.0, 16),
        (DVec3::splat(2.0), 0.5, 0),
        (DVec3::splat(2.0), f64::NAN, 16),
    ] {
        assert!(dentcraft_design::synthetic::rounded_box(size, radius, subdivisions).is_err());
    }
}

#[test]
#[ignore = "release performance gate: 300k crown, two 307k tooth scans, <1.5 s"]
fn perf_adapt_300k() {
    // Build the synthetic cap at scan resolution: regular angular/radial
    // sampling, with the same clinical scene and assertions as the small test.
    let (_, inner, mut crown, params) = scene_resolution(0.04, 384, 128);
    assert!(
        (270_000..350_000).contains(&crown.mesh.triangles.len()),
        "{} triangles",
        crown.mesh.triangles.len()
    );
    let right = crown
        .mesh
        .positions
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let left = crown
        .mesh
        .positions
        .iter()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min);
    let mesial = tooth(DVec3::new(right + 0.4 + 2.0, 0.0, 3.0), 160);
    let distal = tooth(DVec3::new(left + 0.3 - 2.0, 0.0, 3.0), 160);
    // Include all scan BVH construction, validation and thickness certification.
    let start = std::time::Instant::now();
    let report = adapt_to_neighbours(
        &mut crown,
        &inner,
        &params,
        &AdaptParams::default(),
        Some(&mesial),
        Some(&distal),
        None,
    )
    .unwrap();
    let elapsed = start.elapsed();
    eprintln!(
        "perf_adapt_300k: crown={} mesial={} distal={} elapsed={elapsed:?}; {report:?}",
        crown.mesh.triangles.len(),
        mesial.triangles.len(),
        distal.triangles.len()
    );
    assert!((report.mesial_contact.unwrap() + 0.02).abs() < 0.005);
    assert!((report.distal_contact.unwrap() + 0.02).abs() < 0.005);
    assert!(elapsed.as_secs_f64() < 1.5, "adaptation took {elapsed:?}");
}
