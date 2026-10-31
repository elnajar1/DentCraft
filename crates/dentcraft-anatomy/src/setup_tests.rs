//! Tests of the set-up import with synthetic arches of open domes.

use super::*;
use crate::{CrownParams, ToothLibrary};
use glam::DMat3;

/// An open dome, `half_md` by `half_bl` at its cervical line, `height` tall, with `cols`
/// columns: cervical line in the plane z = 0 of the local frame, tip on +Z, wound outward.
pub(crate) fn dome(half_md: f64, half_bl: f64, height: f64, cols: usize) -> TriMesh {
    let rows = 10;
    let mut positions = Vec::new();
    for r in 0..rows {
        let phi = std::f64::consts::FRAC_PI_2 * r as f64 / rows as f64;
        for c in 0..cols {
            let th = std::f64::consts::TAU * c as f64 / cols as f64;
            positions.push(DVec3::new(
                half_md * phi.cos() * th.cos(),
                half_bl * phi.cos() * th.sin(),
                height * phi.sin(),
            ));
        }
    }
    let top = positions.len() as u32;
    positions.push(DVec3::new(0.0, 0.0, height));
    let v = |r: usize, c: usize| (r * cols + c % cols) as u32;
    let mut triangles = Vec::new();
    for c in 0..cols {
        for r in 0..rows - 1 {
            triangles.push([v(r, c), v(r, c + 1), v(r + 1, c + 1)]);
            triangles.push([v(r, c), v(r + 1, c + 1), v(r + 1, c)]);
        }
        triangles.push([v(rows - 1, c), v(rows - 1, c + 1), top]);
    }
    TriMesh::new(positions, triangles)
}

/// (FDI, polar angle from the front in degrees (+ towards patient's right = +X), half width).
const UPPER: [(u8, f64, f64); 4] = [
    (11, 20.0, 4.2),
    (12, 55.0, 3.2),
    (21, -20.0, 4.2),
    (22, -55.0, 3.2),
];
const LOWER: [(u8, f64, f64); 4] = [
    (41, 16.6, 2.6),
    (42, 51.0, 2.8),
    (31, -16.6, 2.6),
    (32, -51.0, 2.8),
];

/// A set-up in patient coordinates (+X patient's right, +Y front, +Z up), then turned by `r`
/// and moved. Domes are shuffled; returns the mesh and the FDI of each component in order.
fn setup(skip: &[u8], lower_scale: f64, r: DMat3) -> (TriMesh, Vec<u8>) {
    let mut placed = Vec::new();
    for (arch, radius, level, down) in [(&UPPER, 12.0, 12.0, true), (&LOWER, 9.0, -12.0, false)] {
        for &(fdi, deg, half) in arch.iter() {
            if skip.contains(&fdi) {
                continue;
            }
            let half = if down { half } else { half * lower_scale };
            let a = deg.to_radians();
            let outward = DVec3::new(a.sin(), a.cos(), 0.0);
            let z = if down { -DVec3::Z } else { DVec3::Z };
            let x = outward.cross(z);
            let at = outward * radius + DVec3::Z * level;
            let mut m = dome(half, 3.0, 6.0, 24 + fdi as usize % 10);
            for p in &mut m.positions {
                *p = r * (at + x * p.x + outward * p.y + z * p.z) + DVec3::new(3.0, -50.0, 1.0);
            }
            placed.push((fdi, m));
        }
    }
    placed.sort_by_key(|(fdi, _)| (*fdi as u32 * 7) % 11);
    let mut mesh = TriMesh::default();
    for (_, m) in &placed {
        mesh.merge(m);
    }
    (mesh, placed.iter().map(|(f, _)| *f).collect())
}

fn turn() -> DMat3 {
    DMat3::from_axis_angle(DVec3::new(0.3, -1.0, 0.5).normalize(), 0.9)
}

#[test]
fn a_set_up_splits_into_numbered_teeth() {
    let r = turn();
    let (mesh, order) = setup(&[], 1.0, r);
    let teeth = split_setup(&mesh).unwrap();
    let codes: Vec<u8> = teeth.iter().map(|t| t.fdi.code()).collect();
    assert_eq!(codes, [11, 12, 21, 22, 31, 32, 41, 42]);
    for t in &teeth {
        assert_eq!(
            order[t.component - 1],
            t.fdi.code(),
            "component {}",
            t.component
        );
        let upper = t.fdi.quadrant() <= 2;
        let expect = r * if upper { -DVec3::Z } else { DVec3::Z };
        assert!(t.occlusal.dot(expect) > 0.999, "{} occlusal", t.fdi.code());
        let (deg, _) = UPPER
            .iter()
            .chain(&LOWER)
            .find(|e| e.0 == t.fdi.code())
            .map(|e| (e.1, e.2))
            .unwrap();
        let a: f64 = deg.to_radians();
        let outward = r * DVec3::new(a.sin(), a.cos(), 0.0);
        assert!(t.facial.dot(outward) > 0.9, "{} facial", t.fdi.code());
        assert!(t.facial.dot(t.occlusal).abs() < 1e-9);
        assert!(t.mesh.positions.contains(&t.cervical));
        let c = t.mesh.positions.iter().sum::<DVec3>() / t.mesh.positions.len() as f64;
        assert!(
            (t.cervical - c).dot(t.facial) > 2.5,
            "{} cervical",
            t.fdi.code()
        );
    }
}

#[test]
fn an_imported_set_up_loads_with_each_crown_its_own() {
    let (mesh, _) = setup(&[], 1.0, turn());
    let teeth = split_setup(&mesh).unwrap();
    let dir = std::env::temp_dir().join(format!("dentcraft-setup-{}", std::process::id()));
    write_library(&teeth, &dir).unwrap();
    let lib = ToothLibrary::load(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    for t in &teeth {
        let built = lib.crown(t.fdi, &CrownParams::default()).unwrap().unwrap();
        // Each tooth's own file (dome columns differ per tooth) and its own cervical line.
        assert_eq!(built.cervical.len(), 24 + t.fdi.code() as usize % 10);
        assert_eq!(built.mesh.boundary_loops().len(), 1);
    }
}

#[test]
fn what_the_import_cannot_decide_is_an_error_naming_the_crown() {
    let r = turn();
    // A closed crown.
    let (mut mesh, _) = setup(&[], 1.0, r);
    let mut ball = TriMesh::uv_sphere(2.0, 12, 8);
    for p in &mut ball.positions {
        *p += DVec3::new(0.0, 0.0, 40.0);
    }
    mesh.merge(&ball);
    let e = split_setup(&mesh).unwrap_err().0;
    assert!(
        e.contains("component 9") && e.contains("0 open boundaries"),
        "{e}"
    );
    // A crown lying on its side.
    let (mut mesh, _) = setup(&[], 1.0, r);
    let mut side = dome(3.0, 3.0, 6.0, 24);
    for p in &mut side.positions {
        *p = r * DVec3::new(p.z, p.y, p.x) + DVec3::new(3.0, -50.0, 1.0);
    }
    mesh.merge(&side);
    let e = split_setup(&mesh).unwrap_err().0;
    assert!(
        e.contains("component 9") && e.contains("neither arch"),
        "{e}"
    );
    // A missing central incisor leaves a gap.
    let e = split_setup(&setup(&[21], 1.0, r).0).unwrap_err().0;
    assert!(e.contains("gap") && e.contains("component"), "{e}");
    // Upper and lower centrals the same width.
    let e = split_setup(&setup(&[], 1.6, r).0).unwrap_err().0;
    assert!(e.contains("upper arch from the lower"), "{e}");
    // One arch only.
    let e = split_setup(&setup(&[41, 42, 31, 32], 1.0, r).0)
        .unwrap_err()
        .0;
    assert!(e.contains("crown"), "{e}");
    assert!(split_setup(&TriMesh::default()).is_err());
}

#[test]
fn the_jacobi_solver_diagonalises() {
    let m = [[4.0, 1.0, 0.5], [1.0, 3.0, 0.2], [0.5, 0.2, 1.0]];
    let (values, vectors) = eigen_symmetric(m);
    assert!(values[0] <= values[1] && values[1] <= values[2]);
    for (l, v) in values.iter().zip(vectors) {
        let mv = DVec3::new(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        );
        assert!((mv - v * *l).length() < 1e-9);
    }
}
