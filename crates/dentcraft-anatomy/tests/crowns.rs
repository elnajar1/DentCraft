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
        let mut check = |v: u32, what: &str| {
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

#[test]
fn left_teeth_are_mirrors_of_right_teeth() {
    let r = template(11);
    let l = template(21);
    assert_eq!(r.mesh.positions.len(), l.mesh.positions.len());
    for (pr, pl) in r.mesh.positions.iter().zip(&l.mesh.positions) {
        assert!((pr.x + pl.x).abs() < 1e-9, "x not mirrored");
        assert!((pr.y - pl.y).abs() < 1e-9, "y changed");
        assert!((pr.z - pl.z).abs() < 1e-9, "z changed");
    }
    assert_eq!(r.mesh.triangles.len(), l.mesh.triangles.len());
    for (tr, tl) in r.mesh.triangles.iter().zip(&l.mesh.triangles) {
        assert_eq!(
            (tr[0], tr[2], tr[1]),
            (tl[0], tl[1], tl[2]),
            "winding not swapped"
        );
    }
    // The cervical loop runs the other way round, same vertices.
    let n = r.cervical.len();
    assert_eq!(l.cervical.len(), n);
    for k in 0..n {
        assert_eq!(
            l.cervical[k],
            r.cervical[n - 1 - k],
            "cervical not reversed"
        );
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
