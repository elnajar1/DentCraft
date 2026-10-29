/// Outer cap, re-indexed. Template vertex i is outer vertex i.
fn outer_cap(crown: &Crown, inner: &InnerSurface) -> TriMesh {
    let offset = inner.mesh.positions.len() as u32;
    TriMesh::new(
        crown.mesh.positions[offset as usize..].to_vec(),
        crown.mesh.triangles[inner.mesh.triangles.len()..]
            .iter()
            .filter(|t| t.iter().all(|v| *v >= offset))
            .map(|t| t.map(|v| v - offset))
            .collect(),
    )
}

/// Finding 5: a correctly sized auto-placed crown must not overbuild the
/// apex: where the template already met the occlusal requirement the final
/// clearance may exceed it by at most 0.05 mm.
#[test]
fn correct_size_does_not_overbuild() {
    let die = prepared_stump(&StumpParams::default()).unwrap();
    let inner = intaglio(&die, 0.05);
    let (template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
    let params = CrownParams::default();
    let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
    let crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
    verify(&crown, &inner, &die, &params);
    let apex = crown.mesh.positions[inner.mesh.positions.len() + template.positions.len() - 1];
    let clearance = Bvh::build(&inner.mesh)
        .closest_point(&inner.mesh, apex)
        .unwrap()
        .distance;
    eprintln!("apex rise: {clearance:.6} mm over the inner top");
    assert!(
        clearance <= params.min_occlusal + 0.05,
        "apex clearance {clearance}"
    );
}

/// Finding 5: thickening must stay local. A verified crown cap, reused as
/// its own template with a 0.12 mm dent over a small patch, must come out
/// with every moved vertex within two rings of a dented vertex — the apex
/// and everything farther stay bit-identical.
#[test]
fn thickening_stays_local() {
    let die = prepared_stump(&StumpParams::default()).unwrap();
    let inner = intaglio(&die, 0.05);
    let (template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
    let params = CrownParams::default();
    let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
    let built = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
    verify(&built, &inner, &die, &params);
    // Reuse the verified cap as a correctly sized template; identity frame.
    let cap = outer_cap(&built, &inner);
    let cervical: Vec<u32> = cap.boundary_loops().remove(0);
    let origin = centroid(&inner.mesh, &inner.boundary).unwrap();
    let identity = CrownPlacement {
        origin,
        occlusal: die.axis,
        mesial: DVec3::X,
        scale: DVec3::ONE,
    };
    // Dent a φ band on one side radially inward by up to 0.12 mm.
    let mut dented = cap.clone();
    let mut dented_vertices = Vec::new();
    for (i, p) in dented.positions.iter_mut().enumerate() {
        let local = *p - origin;
        let phi = local.z.atan2(local.truncate().length());
        let theta = local.y.atan2(local.x);
        let window = (1.0 - smoothstep(((phi.to_degrees() - 45.0).abs() / 15.0).clamp(0.0, 1.0)))
            * (1.0 - smoothstep((theta.abs() / 0.7).clamp(0.0, 1.0)));
        if window == 0.0 {
            continue;
        }
        let radial = DVec3::new(local.x, local.y, 0.0).normalize();
        *p -= radial * (0.12 * window);
        dented_vertices.push(i as u32);
    }
    assert!(!dented_vertices.is_empty());
    let crown = build_crown(&inner, &dented, &cervical, identity, &params).unwrap();
    verify(&crown, &inner, &die, &params);
    // Control build with requirements the dented cap already satisfies:
    // identical deformation stages, zero thickening, so the difference
    // between the two crowns is exactly the thickness correction.
    let loose = CrownParams::new(0.3, 0.28);
    let control = build_crown(&inner, &dented, &cervical, identity, &loose).unwrap();
    verify(&control, &inner, &die, &loose);
    // Two rings around the dent, by graph distance on the template.
    let adjacency = dented.vertex_adjacency();
    let mut near = dented_vertices.clone();
    for _ in 0..2 {
        near = near
            .iter()
            .flat_map(|&v| {
                let mut ring = adjacency.neighbors(v).to_vec();
                ring.push(v);
                ring
            })
            .collect();
        near.sort_unstable();
        near.dedup();
    }
    let near: HashSet<u32> = near.into_iter().collect();
    let offset = inner.mesh.positions.len();
    let mut moved = 0;
    for i in 0..dented.positions.len() {
        if !near.contains(&(i as u32)) {
            assert_eq!(
                crown.mesh.positions[offset + i]
                    .to_array()
                    .map(f64::to_bits),
                control.mesh.positions[offset + i]
                    .to_array()
                    .map(f64::to_bits),
                "vertex {i} changed outside the dent's two rings"
            );
        }
        let correction = crown.mesh.positions[offset + i] - control.mesh.positions[offset + i];
        if correction == DVec3::ZERO {
            continue;
        }
        moved += 1;
        assert!(
            near.contains(&(i as u32)),
            "vertex {i} was thickened outside the dent's two rings"
        );
    }
    assert!(moved > 0, "the dent was not corrected");
    let apex = offset + dented.positions.len() - 1;
    assert_eq!(
        crown.mesh.positions[apex].to_array().map(f64::to_bits),
        control.mesh.positions[apex].to_array().map(f64::to_bits),
        "apex was thickened"
    );
}

/// Finding 6: thickness must be dense-sampled. A coarse 16×4 template
/// (edges over 1 mm) must pass an independent 16×16 barycentric resample
/// of every outer face with no deficit beyond 1 µm.
#[test]
fn coarse_template_passes_dense_resample() {
    let die = prepared_stump(&StumpParams::default()).unwrap();
    let inner = intaglio(&die, 0.05);
    let (template, cervical) = crown_template(4.0, 5.5, 16, 4).unwrap();
    let params = CrownParams::default();
    let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
    let crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
    verify(&crown, &inner, &die, &params);
    let outer = outer_cap(&crown, &inner);
    let bvh = Bvh::build(&inner.mesh);
    let normals = outer.vertex_normals();
    let mut worst = 0.0_f64;
    for tri in &outer.triangles {
        let p = tri.map(|v| outer.positions[v as usize]);
        let n = tri.map(|v| normals[v as usize]);
        for i in 0..=16usize {
            for j in 0..=(16 - i) {
                let w = [i as f64 / 16.0, j as f64 / 16.0, (16 - i - j) as f64 / 16.0];
                let point = p[0] * w[0] + p[1] * w[1] + p[2] * w[2];
                let normal = (n[0] * w[0] + n[1] * w[1] + n[2] * w[2]).normalize();
                let hit = bvh.closest_point(&inner.mesh, point).unwrap();
                let required = params.required_thickness(hit.point, normal, die.axis, &die.margin);
                worst = worst.max(required - hit.distance);
            }
        }
    }
    eprintln!("independent 16x16 worst deficit {worst:.9} mm");
    assert!(worst <= 0.001, "dense resample deficit {worst:.6} mm");
}

#[test]
fn thin_tapered_incisors_build() {
    for ratio in [0.15, 0.3] {
        let mut die = prepared_stump(&StumpParams::default()).unwrap();
        let taper = |p: &mut DVec3| {
            p.y *= 1.0 - (1.0 - ratio) * (p.z / 4.0).clamp(0.0, 1.0);
        };
        for p in &mut die.mesh.positions {
            taper(p);
        }
        for p in &mut die.margin {
            taper(p);
        }
        let inner = intaglio(&die, 0.05);
        let (mut template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
        for p in &mut template.positions {
            p.y *= 1.0 - (1.0 - ratio) * (p.z / 5.5).clamp(0.0, 1.0);
        }
        let params = CrownParams::new(0.7, 0.5);
        let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
        let crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
        verify(&crown, &inner, &die, &params);
    }
}

#[test]
fn uniform_thickening_does_not_amplify() {
    let mut mesh = TriMesh::new(
        vec![
            DVec3::new(-20.0, -20.0, 0.0),
            DVec3::new(20.0, -20.0, 0.0),
            DVec3::new(0.0, 20.0, 0.0),
        ],
        vec![[0, 2, 1]],
    );
    let margin = mesh.positions.clone();
    let inner = InnerSurface {
        mesh: std::mem::take(&mut mesh),
        boundary: vec![0, 2, 1],
        axis: DVec3::Z,
        target_gaps: vec![0.0; 3],
    };
    let mut outer = TriMesh::new(
        (0..3)
            .map(|i| {
                let a = TAU * i as f64 / 3.0;
                DVec3::new(0.1 * a.cos(), 0.1 * a.sin(), 0.5)
            })
            .collect(),
        vec![[0, 1, 2]],
    );
    let polar = Polar {
        origin: DVec3::ZERO,
        x: DVec3::X,
        y: DVec3::Y,
        z: DVec3::Z,
    };
    let reference = vec![outer.triangle_cross(0)];
    let report = thicken(
        &mut outer,
        &[false; 3],
        &inner,
        &Bvh::build(&inner.mesh),
        &margin,
        (polar, &reference),
        &CrownParams::new(0.7, 0.7),
    )
    .unwrap();
    for p in &outer.positions {
        assert!(
            p.z >= 0.699 && p.z <= 0.701,
            "uniform correction overbuilt to {} mm",
            p.z
        );
    }
    eprintln!("uniform thickness {:.9} mm", report.min_thickness);
}

#[test]
fn cervical_refinement_handles_corner_triangles() {
    let mut mesh = TriMesh::new(
        (0..3)
            .map(|i| {
                let a = TAU * i as f64 / 3.0;
                DVec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect(),
        vec![[0, 1, 2]],
    );
    let original = mesh.clone();
    let polar = Polar {
        origin: DVec3::ZERO,
        x: DVec3::X,
        y: DVec3::Y,
        z: DVec3::Z,
    };
    let mut boundary = polar.ordered(&mesh, &[0, 1, 2]).unwrap();
    let angles: Vec<_> = (0..3).map(|i| (TAU * (i as f64 + 0.5) / 3.0, i)).collect();
    refine_cervical(&mut mesh, &mut boundary, &angles, polar);
    let indices: Vec<_> = boundary.iter().map(|v| v.1).collect();
    validate_surface(&mesh, &indices).unwrap();
    assert_eq!(mesh.positions.len(), 6);
    assert_eq!(mesh.triangles.len(), 4);
    assert_eq!(&mesh.positions[..3], &original.positions);
    let area: f64 = (0..mesh.triangles.len())
        .map(|i| mesh.triangle_cross(i).z)
        .sum();
    assert!((area - original.triangle_cross(0).z).abs() < 1e-12);
    assert!((0..mesh.triangles.len()).all(|i| mesh.triangle_cross(i).z > 0.0));
}
