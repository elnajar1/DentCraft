//! Gate tests for the parametric crown library.
//!
//! Expected dimensions are repeated from the same published table the crate uses
//! (Nelson & Ash, *Wheeler's Dental Anatomy, Physiology and Occlusion*, 9th ed., 2010,
//! "Measurements of the teeth"), independently of the crate's internal copy.

use dentcraft_anatomy::{CrownParams, crown};
use dentcraft_case::{Arch, Fdi};
use dentcraft_mesh::{Bvh, TriMesh};
use glam::DVec3;
use std::collections::HashSet;

/// (crown length, mesiodistal, buccolingual) per right-side tooth, FDI 11..=18 then 41..=48.
const TABLE: [(u8, f64, f64, f64); 16] = [
    (11, 10.5, 8.5, 7.0),
    (12, 9.0, 6.5, 6.0),
    (13, 10.0, 7.5, 8.0),
    (14, 8.5, 7.0, 9.0),
    (15, 8.5, 7.0, 9.0),
    (16, 7.5, 10.0, 11.0),
    (17, 7.0, 9.0, 11.0),
    (18, 6.5, 8.5, 10.0),
    (41, 9.0, 5.0, 6.0),
    (42, 9.5, 5.5, 6.5),
    (43, 11.0, 7.0, 7.5),
    (44, 8.5, 7.0, 7.5),
    (45, 8.0, 7.0, 8.0),
    (46, 7.5, 11.0, 10.5),
    (47, 7.0, 10.5, 10.0),
    (48, 7.0, 10.0, 9.5),
];

fn template(code: u8) -> dentcraft_anatomy::CrownTemplate {
    crown(Fdi::new(code).unwrap(), &CrownParams::default())
}

fn extents(mesh: &TriMesh) -> (f64, f64, f64, f64, f64) {
    let b = mesh.bounds().unwrap();
    (
        b.max.x - b.min.x,
        b.max.y - b.min.y,
        b.max.z,
        b.min.x,
        b.min.y,
    )
}

/// The crown closed with a fan over its cervical loop; volume > 0 iff normals point outward.
fn closed_with_cervical_fan(t: &dentcraft_anatomy::CrownTemplate) -> TriMesh {
    let mut positions = t.mesh.positions.clone();
    let centre = t
        .cervical
        .iter()
        .map(|&v| positions[v as usize])
        .sum::<DVec3>()
        / t.cervical.len() as f64;
    let c = positions.len() as u32;
    positions.push(centre);
    let mut triangles = t.mesh.triangles.clone();
    let n = t.cervical.len();
    for k in 0..n {
        let (a, b) = (t.cervical[k], t.cervical[(k + 1) % n]);
        triangles.push([c, b, a]);
    }
    TriMesh::new(positions, triangles)
}

fn same_loop(a: &[u32], b: &[u32]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let n = a.len();
    if a.iter().copied().collect::<HashSet<_>>() != b.iter().copied().collect::<HashSet<_>>() {
        return false;
    }
    let starts: Vec<usize> = (0..n).filter(|&i| b[i] == a[0]).collect();
    starts
        .iter()
        .any(|&s| (0..n).all(|k| b[(s + k) % n] == a[k]))
        || starts
            .iter()
            .any(|&s| (0..n).all(|k| b[(s + n - k) % n] == a[k]))
}

#[test]
fn every_fdi_tooth_generates_a_sound_surface() {
    for fdi in Fdi::all() {
        let t = crown(fdi, &CrownParams::default());
        let mesh = &t.mesh;
        assert!(!mesh.is_empty(), "{fdi}");
        assert!(
            mesh.triangle_count() >= 8_000 && mesh.triangle_count() <= 20_000,
            "{fdi}: {} triangles",
            mesh.triangle_count()
        );
        let loops = mesh.boundary_loops();
        assert_eq!(loops.len(), 1, "{fdi}: {} boundary loops", loops.len());
        assert!(mesh.is_consistently_oriented(), "{fdi}");
        for i in 0..mesh.triangle_count() {
            assert!(
                mesh.triangle_cross(i).length() > 1e-6,
                "{fdi}: degenerate triangle {i}"
            );
        }
        let closed = closed_with_cervical_fan(&t);
        let volume = closed.signed_volume();
        assert!(
            volume > 0.0,
            "{fdi}: signed volume {volume} after closing the crown"
        );
    }
}

#[test]
fn the_single_boundary_loop_is_the_cervical_line() {
    for code in [11u8, 21, 36, 43] {
        let t = template(code);
        let loops = t.mesh.boundary_loops();
        assert_eq!(loops.len(), 1, "{code}");
        assert!(
            same_loop(&loops[0], &t.cervical),
            "{code}: boundary loop is not `cervical`"
        );
        assert_eq!(t.cervical.len(), 160, "{code}");
    }
}

#[test]
fn mesh_dimensions_match_wheeler_within_two_percent() {
    for (code, length, md, bl) in TABLE {
        let t = template(code);
        let (width, depth, height, _, _) = extents(&t.mesh);
        assert!(
            (width - md).abs() / md < 0.02,
            "{code}: width {width} vs {md}"
        );
        assert!(
            (depth - bl).abs() / bl < 0.02,
            "{code}: depth {depth} vs {bl}"
        );
        assert!(
            (height - length).abs() / length < 0.02,
            "{code}: height {height} vs {length}"
        );
    }
    // Left teeth measure the same as their right counterparts.
    let t26 = template(26);
    assert!((extents(&t26.mesh).0 - 10.0).abs() < 0.2);
}

#[test]
fn cervical_outline_is_narrower_than_the_crown() {
    for (code, _, md, bl) in TABLE {
        let t = template(code);
        // The cervix row measures the cervical table dimensions.
        let mut x0 = f64::MAX;
        let mut x1 = f64::MIN;
        let mut y0 = f64::MAX;
        let mut y1 = f64::MIN;
        for &v in &t.cervical {
            let p = t.mesh.positions[v as usize];
            x0 = x0.min(p.x);
            x1 = x1.max(p.x);
            y0 = y0.min(p.y);
            y1 = y1.max(p.y);
        }
        assert!(
            (x1 - x0) - (extents(&t.mesh).0) < -0.1,
            "{code}: cervix width"
        );
        assert!(
            (y1 - y0) - (extents(&t.mesh).1) < -0.1,
            "{code}: cervix depth"
        );
        assert!((y1 - y0) / bl > 0.5 && (x1 - x0) / md > 0.5, "{code}");
    }
}

#[test]
fn landmarks_lie_on_the_surface_and_name_the_anatomy() {
    let bvh46 = {
        let t = template(46);
        let bvh = Bvh::build(&t.mesh);
        let names: Vec<&str> = t.landmarks.cusps.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "mesiobuccal",
                "distobuccal",
                "distal",
                "mesiolingual",
                "distolingual"
            ]
        );
        assert!(t.landmarks.central_fossa.is_some());
        assert!(t.landmarks.incisal_mesial.is_none());
        bvh
    };
    let _ = bvh46;

    for code in [11u8, 13, 16, 26, 44, 45, 46, 47] {
        let t = template(code);
        let bvh = Bvh::build(&t.mesh);
        let check = |v: u32, what: &str| {
            let p = t.mesh.positions[v as usize];
            let d = bvh.closest_point(&t.mesh, p).unwrap().distance;
            assert!(d < 0.01, "{code}: {what} is {d} mm off the surface");
        };
        for c in &t.landmarks.cusps {
            check(c.vertex, &format!("cusp {}", c.name));
        }
        check(t.landmarks.mesial_contact, "mesial contact");
        check(t.landmarks.distal_contact, "distal contact");
        check(t.landmarks.facial_contour, "facial contour");
        check(t.landmarks.lingual_contour, "lingual contour");
        if let Some(v) = t.landmarks.central_fossa {
            check(v, "central fossa");
        }
        if let Some(v) = t.landmarks.incisal_mesial {
            check(v, "mesial incisal");
        }
        if let Some(v) = t.landmarks.incisal_distal {
            check(v, "distal incisal");
        }
    }

    let t11 = template(11);
    assert!(t11.landmarks.cusps.is_empty());
    assert!(t11.landmarks.incisal_mesial.is_some());
    let t13 = template(13);
    assert_eq!(t13.landmarks.cusps.len(), 1);
    assert_eq!(t13.landmarks.cusps[0].name, "cusp");
}

#[test]
fn the_lingual_cusp_of_a_lower_molar_is_the_highest() {
    let t = template(46);
    let by_name = |n: &str| {
        t.landmarks
            .cusps
            .iter()
            .find(|c| c.name == n)
            .map(|c| t.mesh.positions[c.vertex as usize].z)
            .unwrap()
    };
    assert!(by_name("mesiolingual") > by_name("mesiobuccal"));
    assert!(by_name("distolingual") > by_name("distobuccal"));
    assert!(by_name("distobuccal") > by_name("distal"));
}

/// Quadrants 2 and 4 are the mirror images of their generated counterparts in quadrants 1 and 3
/// (26 of 16, 46 of 36): `x` negated, winding swapped, cervical loop reversed.
#[test]
fn quadrants_two_and_four_are_mirrors_of_one_and_three() {
    for (generated, mirrored) in [(11u8, 21u8), (16, 26), (36, 46), (33, 43)] {
        let r = template(generated);
        let l = template(mirrored);
        assert_eq!(r.mesh.positions.len(), l.mesh.positions.len());
        for (pr, pl) in r.mesh.positions.iter().zip(&l.mesh.positions) {
            assert!((pr.x + pl.x).abs() < 1e-9, "{mirrored}: x not mirrored");
            assert!((pr.y - pl.y).abs() < 1e-9, "{mirrored}: y changed");
            assert!((pr.z - pl.z).abs() < 1e-9, "{mirrored}: z changed");
        }
        assert_eq!(r.mesh.triangles.len(), l.mesh.triangles.len());
        for (tr, tl) in r.mesh.triangles.iter().zip(&l.mesh.triangles) {
            assert_eq!(
                (tr[0], tr[2], tr[1]),
                (tl[0], tl[1], tl[2]),
                "{mirrored}: winding not swapped"
            );
        }
        // The cervical loop runs the other way round, same vertices.
        let n = r.cervical.len();
        assert_eq!(l.cervical.len(), n);
        for k in 0..n {
            assert_eq!(
                l.cervical[k],
                r.cervical[n - 1 - k],
                "{mirrored}: cervical not reversed"
            );
        }
    }
}

/// Every template, placed in the mouth by a rigid transform built from its facial and occlusal
/// directions at its arch position, has its mesial contact towards the midline (anteriors) or
/// anteriorly (posteriors): the template has the real tooth's handedness.
///
/// Mouth frame: `+x` patient's right, `+y` anterior, `+z` superior (right-handed). The arch is
/// the parabola `y = -x^2 / 40` in the occlusal plane; the facial direction is its outward
/// normal, the occlusal direction is down for uppers and up for lowers.
#[test]
fn templates_placed_by_a_rotation_have_the_real_teeth_handedness() {
    use glam::DMat3;
    // Distance of each FDI position (1..=8) from the midline along x, in mm.
    const ARCH_X: [f64; 8] = [4.0, 10.0, 15.0, 19.0, 23.0, 29.0, 36.0, 42.0];
    for fdi in Fdi::all() {
        let t = crown(fdi, &CrownParams::default());
        // Patient's right for quadrants 1 and 4.
        let side = if matches!(fdi.quadrant(), 1 | 4) {
            1.0
        } else {
            -1.0
        };
        let x = side * ARCH_X[fdi.position() as usize - 1];
        let facial = DVec3::new(x / 20.0, 1.0, 0.0).normalize();
        let occlusal = if fdi.arch() == Arch::Upper {
            DVec3::NEG_Z
        } else {
            DVec3::Z
        };
        let rotation = DMat3::from_cols(facial.cross(occlusal), facial, occlusal);
        assert!(
            (rotation.determinant() - 1.0).abs() < 1e-12,
            "{fdi}: placement is not a rotation"
        );
        let mesial = rotation * t.mesh.positions[t.landmarks.mesial_contact as usize];
        let distal = rotation * t.mesh.positions[t.landmarks.distal_contact as usize];
        if fdi.is_anterior() {
            // Towards the midline: -side * x grows.
            assert!(
                -side * mesial.x > 0.0 && -side * (mesial.x - distal.x) > 2.0,
                "{fdi}: mesial contact {mesial:?} does not point towards the midline"
            );
        } else {
            assert!(
                mesial.y > 0.0 && mesial.y - distal.y > 2.0,
                "{fdi}: mesial contact {mesial:?} does not point anteriorly"
            );
        }
        if let (Some(m), Some(d)) = (t.landmarks.incisal_mesial, t.landmarks.incisal_distal) {
            let (m, d) = (
                rotation * t.mesh.positions[m as usize],
                rotation * t.mesh.positions[d as usize],
            );
            assert!(-side * (m.x - d.x) > 1.0, "{fdi}: incisal angles swapped");
        }
    }
}

#[test]
fn params_scale_measured_dimensions_proportionally() {
    let base = template(46);
    let (w0, d0, h0, _, _) = extents(&base.mesh);

    let wide = crown(
        Fdi::new(46).unwrap(),
        &CrownParams {
            mesiodistal: 1.3,
            ..CrownParams::default()
        },
    );
    let (w1, _, _, _, _) = extents(&wide.mesh);
    assert!((w1 / w0 - 1.3).abs() < 0.01);

    let narrow = crown(
        Fdi::new(46).unwrap(),
        &CrownParams {
            buccolingual: 0.8,
            ..CrownParams::default()
        },
    );
    let (_, d1, _, _, _) = extents(&narrow.mesh);
    assert!((d1 / d0 - 0.8).abs() < 0.01);

    let tall = crown(
        Fdi::new(46).unwrap(),
        &CrownParams {
            height: 1.25,
            ..CrownParams::default()
        },
    );
    let (_, _, h1, _, _) = extents(&tall.mesh);
    assert!((h1 / h0 - 1.25).abs() < 0.01);
}

#[test]
fn wear_lowers_cusp_tips() {
    let unworn = template(46);
    let worn = crown(
        Fdi::new(46).unwrap(),
        &CrownParams {
            wear: 1.0,
            ..CrownParams::default()
        },
    );
    for c in &unworn.landmarks.cusps {
        let z0 = unworn.mesh.positions[c.vertex as usize].z;
        let z1 = worn.mesh.positions[c.vertex as usize].z;
        assert!(z1 < z0 - 0.2, "cusp {} not lowered: {z1} vs {z0}", c.name);
    }
    assert!(extents(&worn.mesh).2 < extents(&unworn.mesh).2);
}

#[test]
fn non_finite_and_out_of_range_params_are_sanitised() {
    let default = template(46);
    let nan = crown(
        Fdi::new(46).unwrap(),
        &CrownParams {
            mesiodistal: f64::NAN,
            buccolingual: f64::INFINITY,
            height: f64::NEG_INFINITY,
            wear: f64::NAN,
            cusp_height: f64::NAN,
        },
    );
    assert_eq!(nan.mesh, default.mesh);

    let over = crown(
        Fdi::new(46).unwrap(),
        &CrownParams {
            wear: 5.0,
            ..CrownParams::default()
        },
    );
    let at_limit = crown(
        Fdi::new(46).unwrap(),
        &CrownParams {
            wear: 1.0,
            ..CrownParams::default()
        },
    );
    assert_eq!(over.mesh, at_limit.mesh);
}

#[test]
fn all_32_teeth_are_covered_by_the_arch_iteration() {
    let upper: Vec<u8> = Arch::Upper.teeth().map(|f| f.code()).collect();
    assert_eq!(
        upper,
        vec![
            18, 17, 16, 15, 14, 13, 12, 11, 21, 22, 23, 24, 25, 26, 27, 28
        ]
    );
    let lower: Vec<u8> = Arch::Lower.teeth().map(|f| f.code()).collect();
    assert_eq!(
        lower,
        vec![
            48, 47, 46, 45, 44, 43, 42, 41, 31, 32, 33, 34, 35, 36, 37, 38
        ]
    );
    for code in upper.iter().chain(&lower) {
        let _ = template(*code);
    }
}

/// No triangle faces into the crown, and the surface never folds back on itself.
///
/// For every triangle, a point 0.01 mm off its centroid along its normal must lie outside the
/// crown closed with a cervical fan (an even number of crossings along a ray), and no two
/// triangles sharing an edge may turn their normals more than ~110 degrees apart (a fold-over
/// crease rather than anatomy). One tooth of each mirror pair: mirroring preserves both.
#[test]
fn no_triangle_faces_into_the_crown() {
    let codes = [
        11u8, 12, 13, 14, 15, 16, 17, 18, 41, 42, 43, 44, 45, 46, 47, 48,
    ];
    for code in codes {
        let t = template(code);
        let closed = closed_with_cervical_fan(&t);
        let bvh = Bvh::build(&closed);
        let normal = |tri: &[u32; 3]| {
            let [a, b, c] = tri.map(|v| t.mesh.positions[v as usize]);
            ((a + b + c) / 3.0, (b - a).cross(c - a).normalize_or_zero())
        };
        let mut inward = Vec::new();
        for (k, tri) in t.mesh.triangles.iter().enumerate() {
            let (centre, n) = normal(tri);
            if n == DVec3::ZERO {
                continue;
            }
            // Parity along three slightly skewed rays (a ray that grazes an edge can miscount
            // once; a point truly inside the crown is odd along every ray).
            let odd = [
                DVec3::new(0.0123, -0.0217, 0.0071),
                DVec3::new(-0.0311, 0.0093, 0.0187),
                DVec3::new(0.0057, 0.0349, -0.0241),
            ]
            .iter()
            .filter(|skew| {
                let dir = (n + **skew).normalize();
                let mut p = centre + n * 0.01;
                let mut crossings = 0;
                while let Some(hit) = bvh.raycast(&closed, p, dir) {
                    crossings += 1;
                    p = hit.point + dir * 1e-7;
                    if crossings > 64 {
                        break;
                    }
                }
                crossings % 2 == 1
            })
            .count();
            if odd >= 2 {
                inward.push(k);
            }
        }
        assert!(
            inward.is_empty(),
            "{code}: {} triangles face into the crown, e.g. {:?} at {:?}",
            inward.len(),
            inward.first(),
            inward.first().map(|&k| normal(&t.mesh.triangles[k]).0)
        );

        let mut edges = std::collections::HashMap::new();
        let mut folds = 0;
        let mut first = None;
        for (k, tri) in t.mesh.triangles.iter().enumerate() {
            for e in 0..3 {
                let (a, b) = (tri[e], tri[(e + 1) % 3]);
                let key = (a.min(b), a.max(b));
                if let Some(&other) = edges.get(&key) {
                    let (c, n0) = normal(&t.mesh.triangles[other]);
                    let (_, n1) = normal(tri);
                    if n0.dot(n1) < -0.35 {
                        folds += 1;
                        first.get_or_insert(c);
                    }
                } else {
                    edges.insert(key, k);
                }
            }
        }
        assert_eq!(
            folds, 0,
            "{code}: {folds} fold-over edges, first near {first:?}"
        );
    }
}
