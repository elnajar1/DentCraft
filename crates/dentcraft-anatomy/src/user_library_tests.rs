//! Tests of tooth libraries with a synthetic tooth (crown and root) written by the test.

use super::*;
use glam::DMat3;

/// A closed tooth of revolution (elliptical sections, `a` mesiodistal by `b` facial-lingual)
/// through the profile `(rho, z)` from the root apex to the crown's top, both on the axis.
/// The mesial (+X) half of the crown is up to 40 % fuller towards the top, so mirrors show.
fn revolved(profile: &[(f64, f64)], a: f64, b: f64) -> TriMesh {
    let cols = 48;
    let mut positions = vec![DVec3::new(0.0, 0.0, profile[0].1)];
    for &(rho, z) in &profile[1..profile.len() - 1] {
        for c in 0..cols {
            let th = std::f64::consts::TAU * c as f64 / cols as f64;
            let bulge = 1.0 + 0.4 * th.cos().max(0.0) * (z / 9.0).clamp(0.0, 1.0);
            positions.push(DVec3::new(
                a * rho * th.cos() * bulge,
                b * rho * th.sin(),
                z,
            ));
        }
    }
    let rings = profile.len() - 2;
    let top = positions.len() as u32;
    positions.push(DVec3::new(0.0, 0.0, profile[profile.len() - 1].1));
    let v = |r: usize, c: usize| (1 + r * cols + c % cols) as u32;
    let mut triangles = Vec::new();
    for c in 0..cols {
        triangles.push([0, v(0, c + 1), v(0, c)]);
        for r in 0..rings - 1 {
            triangles.push([v(r, c), v(r, c + 1), v(r + 1, c + 1)]);
            triangles.push([v(r, c), v(r + 1, c + 1), v(r + 1, c)]);
        }
        triangles.push([v(rings - 1, c), v(rings - 1, c + 1), top]);
    }
    TriMesh::new(positions, triangles)
}

/// Crown of height 9 above the CEJ at z = 0, root down to z = -12.5. With `well`, an access
/// cavity: the crown top opens into a 0.8 mm well down to a floor at z = 4.
fn tooth_profile(well: bool) -> Vec<(f64, f64)> {
    let mut p = vec![(0.0, -12.5)];
    for k in 0..=12 {
        let z = -12.0 + k as f64;
        p.push((3.08 * (1.0 + z / 14.0), z));
    }
    for k in 1..20 {
        let z = 9.0 * k as f64 / 20.0;
        let rho = 4.0 * (1.0 - ((z - 3.5) / 5.5).powi(2)).max(0.0).sqrt();
        if well && z > 8.0 {
            break;
        }
        p.push((rho.max(if well { 1.2 } else { 0.0 }), z));
    }
    if well {
        p.extend([(1.0, 8.4), (0.8, 6.0), (0.8, 4.0), (0.0, 4.0)]);
    } else {
        p.push((0.0, 9.0));
    }
    p
}

/// Writes the tooth, turned and moved in file space, with a manifest serving `serves`.
fn library(name: &str, well: bool, seal: bool, serves: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dentcraft-lib-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut mesh = revolved(&tooth_profile(well), 1.0, 0.8);
    // Any rotation: this one takes the tooth's mesial to file -Z, facial to +X, occlusal to -Y.
    let r = DMat3::from_cols(-DVec3::Z, DVec3::X, -DVec3::Y);
    assert!((r.determinant() - 1.0).abs() < 1e-12);
    let to_file = |p: DVec3| r * p + DVec3::new(10.0, -3.0, 7.0);
    for q in &mut mesh.positions {
        *q = to_file(*q);
    }
    std::fs::write(dir.join("tooth.stl"), dentcraft_io::write_stl_binary(&mesh)).unwrap();
    let occlusal = r * DVec3::Z;
    let facial = r * DVec3::Y;
    let cervical = to_file(DVec3::new(0.0, 0.8 * 3.08, 0.0));
    let seal = if seal {
        let c = to_file(DVec3::new(0.0, 0.0, 8.4));
        format!(
            r#", "seal": [{{ "center": {:?}, "radius": 1.6 }}]"#,
            c.to_array()
        )
    } else {
        String::new()
    };
    let json = format!(
        r#"{{ "teeth": [ {{ "file": "tooth.stl", "serves": {serves:?}, "occlusal": {:?},
            "facial": {:?}, "cervical": {:?}{seal} }} ] }}"#,
        occlusal.to_array(),
        facial.to_array(),
        cervical.to_array()
    );
    std::fs::write(dir.join(MANIFEST), json).unwrap();
    dir
}

fn template(lib: &ToothLibrary, code: u8, p: &CrownParams) -> CrownTemplate {
    lib.crown(Fdi::new(code).unwrap(), p).unwrap().unwrap()
}

/// Volume of the crown closed with a fan over its cervical loop: positive iff normals point out.
fn closed_volume(t: &CrownTemplate) -> f64 {
    let mut mesh = t.mesh.clone();
    let c = t
        .cervical
        .iter()
        .map(|&v| mesh.positions[v as usize])
        .sum::<DVec3>()
        / t.cervical.len() as f64;
    let ci = mesh.positions.len() as u32;
    mesh.positions.push(c);
    let n = t.cervical.len();
    for k in 0..n {
        mesh.triangles
            .push([ci, t.cervical[(k + 1) % n], t.cervical[k]]);
    }
    mesh.signed_volume()
}

/// The cervical list runs along the boundary the way the triangles wind it.
fn cervical_follows_the_winding(t: &CrownTemplate) -> bool {
    let edges: std::collections::HashSet<(u32, u32)> = t
        .mesh
        .triangles
        .iter()
        .flat_map(|tr| (0..3).map(move |e| (tr[e], tr[(e + 1) % 3])))
        .collect();
    let n = t.cervical.len();
    (0..n).all(|k| edges.contains(&(t.cervical[k], t.cervical[(k + 1) % n])))
}

#[test]
fn a_library_tooth_becomes_a_sound_canonical_crown() {
    let dir = library("sound", false, false, &[11, 41]);
    let lib = ToothLibrary::load(&dir).unwrap();
    let t = template(&lib, 11, &CrownParams::default());
    std::fs::remove_dir_all(&dir).unwrap();
    let mesh = &t.mesh;
    assert!((15_000..=20_000).contains(&mesh.triangle_count()));
    let loops = mesh.boundary_loops();
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].len(), t.cervical.len());
    assert!(mesh.is_consistently_oriented());
    assert!((0..mesh.triangle_count()).all(|i| mesh.triangle_cross(i).length() > 1e-9));
    assert!(closed_volume(&t) > 0.0, "normals point inward");
    assert!(cervical_follows_the_winding(&t));
    // Canonical frame: mid-facial CEJ at z = 0 on +Y, cervical outline centred.
    let first = mesh.positions[t.cervical[0] as usize];
    assert!(
        first.z.abs() < 1e-9 && first.y > 2.0 && first.x.abs() < 0.3,
        "{first:?}"
    );
    let (lo, hi) = t
        .cervical
        .iter()
        .map(|&v| mesh.positions[v as usize])
        .fold((DVec3::MAX, DVec3::MIN), |(lo, hi), q| {
            (lo.min(q), hi.max(q))
        });
    assert!((lo.x + hi.x).abs() < 1e-9 && (lo.y + hi.y).abs() < 1e-9);
    // The CEJ rises mesially and distally (textbook 3.5 / 2.5 mm for 11).
    assert!(hi.z > 2.0 && lo.z > -1.5, "cut from {} to {}", lo.z, hi.z);
    let b = mesh.bounds().unwrap();
    assert!((b.size().x - 8.5).abs() < 1e-9 && (b.size().y - 7.0).abs() < 1e-9);
    assert!((b.max.z - 10.5).abs() < 1e-9);
    // The fuller mesial half of the top ends up on +X for a quadrant 1 tooth.
    let upper: Vec<f64> = mesh
        .positions
        .iter()
        .filter(|q| q.z > 0.6 * b.max.z)
        .map(|q| q.x)
        .collect();
    let mean_x = upper.iter().sum::<f64>() / upper.len() as f64;
    assert!(mean_x > 0.05, "top centred at x = {mean_x}");
    // Landmarks are vertices on the expected sides.
    let l = &t.landmarks;
    let at = |v: u32| mesh.positions[v as usize];
    assert!(at(l.mesial_contact).x > 3.0 && at(l.distal_contact).x < -3.0);
    assert!(at(l.facial_contour).y > 3.0 && at(l.lingual_contour).y < -3.0);
    assert!(l.incisal_mesial.is_some() && l.cusps.is_empty());
}

#[test]
fn the_other_side_is_a_mirror_wound_outward() {
    let dir = library("mirror", false, false, &[11]);
    let lib = ToothLibrary::load(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let r = template(&lib, 11, &CrownParams::default());
    let l = template(&lib, 21, &CrownParams::default());
    for (a, b) in r.mesh.positions.iter().zip(&l.mesh.positions) {
        assert!((a.x + b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9);
    }
    assert!(closed_volume(&l) > 0.0, "mirrored normals point inward");
    assert!(cervical_follows_the_winding(&l));
    assert_eq!(l.cervical[0], r.cervical[0], "starts mid-facial");
    assert!(l.mesh.is_consistently_oriented());
    assert!(lib.serves(Fdi::new(21).unwrap()));
    assert!(!lib.serves(Fdi::new(31).unwrap()));
    let none = lib.crown(Fdi::new(31).unwrap(), &CrownParams::default());
    assert!(none.unwrap().is_none());
}

#[test]
fn one_tooth_scales_to_each_type_it_serves_and_to_the_params() {
    let dir = library("scale", false, false, &[11, 41]);
    let lib = ToothLibrary::load(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let size = |code: u8, p: CrownParams| {
        let b = template(&lib, code, &p).mesh.bounds().unwrap();
        (b.size().x, b.size().y, b.max.z)
    };
    let (x, y, z) = size(41, CrownParams::default());
    assert!((x - 5.0).abs() < 1e-9 && (y - 6.0).abs() < 1e-9 && (z - 9.0).abs() < 1e-9);
    let p = CrownParams {
        mesiodistal: 1.2,
        buccolingual: 0.9,
        height: 1.1,
        ..CrownParams::default()
    };
    let (x, y, z) = size(31, p);
    assert!((x - 6.0).abs() < 1e-9 && (y - 5.4).abs() < 1e-9 && (z - 9.9).abs() < 1e-9);
}

#[test]
fn a_seal_closes_an_access_cavity() {
    let pole_height = |seal: bool| {
        let dir = library(&format!("seal-{seal}"), true, seal, &[16]);
        let lib = ToothLibrary::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let t = template(&lib, 16, &CrownParams::default());
        let pole = *t.mesh.positions.last().unwrap();
        pole.z / t.mesh.bounds().unwrap().max.z
    };
    let open = pole_height(false);
    let sealed = pole_height(true);
    assert!(open < 0.65, "the well is not in the open crown ({open:.2})");
    assert!(sealed > 0.85, "the seal left a pit ({sealed:.2})");
}

#[test]
fn a_bad_library_reports_a_plain_error() {
    let dir = std::env::temp_dir().join(format!("dentcraft-lib-{}-bad", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = ToothLibrary::load(&dir).unwrap_err();
    assert!(err.0.contains("library.json"), "{err}");
    let entry = |serves: &str, facial: &str| {
        format!(
            r#"{{ "teeth": [ {{ "file": "x.stl", "serves": [{serves}], "occlusal": [0,0,1],
                "facial": [{facial}], "cervical": [0,0,0] }} ] }}"#
        )
    };
    for (json, expect) in [
        (entry("19", "0,1,0"), "19"),
        (entry("11", "0,0,2"), "parallel"),
        (entry("11", "0,1,0"), "x.stl"),
    ] {
        std::fs::write(dir.join(MANIFEST), json).unwrap();
        let err = ToothLibrary::load(&dir).unwrap_err();
        assert!(err.0.contains(expect), "{err}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_open_library_crown_keeps_its_own_surface_and_cervical_line() {
    let dir = std::env::temp_dir().join(format!("dentcraft-lib-{}-open", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A dome with a wavy cervical line (0.8 mm up mesially and distally), turned, inside out.
    let mut source = crate::setup::tests::dome(3.5, 4.5, 7.0, 40);
    for q in &mut source.positions {
        q.z += 0.8 * (2.0 * q.y.atan2(q.x)).cos() * (1.0 - q.z / 7.0);
    }
    let r = DMat3::from_cols(-DVec3::Z, DVec3::X, -DVec3::Y);
    let to_file = |p: DVec3| r * p + DVec3::new(10.0, -3.0, 7.0);
    let mut file = source.clone();
    for q in &mut file.positions {
        *q = to_file(*q);
    }
    file.flip_orientation();
    std::fs::write(dir.join("tooth.stl"), dentcraft_io::write_stl_binary(&file)).unwrap();
    let json = format!(
        r#"{{ "teeth": [ {{ "file": "tooth.stl", "serves": [14], "occlusal": {:?},
            "facial": {:?}, "cervical": {:?} }} ] }}"#,
        (r * DVec3::Z).to_array(),
        (r * DVec3::Y).to_array(),
        to_file(DVec3::new(0.0, 4.5, -0.8)).to_array()
    );
    std::fs::write(dir.join(MANIFEST), json).unwrap();
    let lib = ToothLibrary::load(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    for code in [14, 24] {
        let t = template(&lib, code, &CrownParams::default());
        let mesh = &t.mesh;
        // The library's own vertices and cervical line, not a 160-column resampling.
        assert_eq!(mesh.vertex_count(), source.vertex_count());
        assert_eq!(t.cervical.len(), 40);
        assert_eq!(mesh.boundary_loops().len(), 1);
        assert!(closed_volume(&t) > 0.0, "{code}: normals point inward");
        assert!(cervical_follows_the_winding(&t), "{code}");
        let first = mesh.positions[t.cervical[0] as usize];
        assert!(first.z.abs() < 1e-9 && first.y > 2.0, "{first:?}");
        // Its own CEJ wave (1.6 mm in 7.8 mm of height, scaled to 8.5 mm) survives.
        let zs: Vec<f64> = t
            .cervical
            .iter()
            .map(|&v| mesh.positions[v as usize].z)
            .collect();
        let wave = zs.iter().copied().fold(f64::MIN, f64::max)
            - zs.iter().copied().fold(f64::MAX, f64::min);
        assert!((wave - 1.6 * 8.5 / 7.8).abs() < 0.15, "{code}: wave {wave}");
        assert_eq!(t.landmarks.cusps.len(), 2);
    }
}
