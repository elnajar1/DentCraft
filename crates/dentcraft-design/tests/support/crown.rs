// Shared independent geometry verification for synthetic and real library caps.
use super::{Crown, CrownParams, InnerSurface, PreparedStump};
use dentcraft_mesh::{Bvh, TriMesh};
use glam::DVec3;

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

fn face_samples(p: [DVec3; 3]) -> impl Iterator<Item = [f64; 3]> {
    let edge = (0..3)
        .map(|i| p[i].distance(p[(i + 1) % 3]))
        .fold(0.0, f64::max);
    let n = (edge / 0.1).ceil().max(3.0) as usize;
    [
        [0.5, 0.5, 0.0],
        [0.0, 0.5, 0.5],
        [0.5, 0.0, 0.5],
        [1.0 / 3.0; 3],
    ]
    .into_iter()
    .chain((0..=n).flat_map(move |i| {
        (0..=n - i).map(move |j| {
            [
                i as f64 / n as f64,
                j as f64 / n as f64,
                (n - i - j) as f64 / n as f64,
            ]
        })
    }))
}

pub(super) fn verify(
    crown: &Crown,
    inner: &InnerSurface,
    die: &PreparedStump,
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
    let origin: DVec3 = inner
        .boundary
        .iter()
        .map(|&v| inner.mesh.positions[v as usize])
        .sum::<DVec3>()
        / inner.boundary.len() as f64;
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
        for weights in face_samples(p) {
            let point = p[0] * weights[0] + p[1] * weights[1] + p[2] * weights[2];
            let normal = (n[0] * weights[0] + n[1] * weights[1] + n[2] * weights[2]).normalize();
            let hit = bvh.closest_point(&inner.mesh, point).unwrap();
            // Independently compute the documented ramp and orientation blend.
            let mut d = f64::INFINITY;
            for i in 0..die.margin.len() {
                let a = die.margin[i];
                let b = die.margin[(i + 1) % die.margin.len()];
                let edge = b - a;
                d = d.min(hit.point.distance(
                    a + edge * ((hit.point - a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0),
                ));
            }
            let f = (normal.dot(die.axis) * 2.0).clamp(0.0, 1.0);
            let full =
                params.min_wall + (params.min_occlusal - params.min_wall) * f * f * (3.0 - 2.0 * f);
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
