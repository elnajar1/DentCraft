//! Checks a tooth library folder: every template builds and is sound.
//!
//! Run with the folder in `DENTCRAFT_TOOTH_LIBRARY` (a library is the user's own data and is
//! never committed):
//!
//! `DENTCRAFT_TOOTH_LIBRARY=<dir> cargo run -p dentcraft-anatomy --release --example check_library`
//!
//! For each of the 32 FDI teeth it builds the crown template with default parameters and checks
//! one cervical boundary loop (the template's own `cervical`) and outward normals (the crown
//! closed with a fan over its cervical loop has a positive volume). It also counts the faces
//! that fail the radial Jacobian test `build_crown` applies to the placed outer surface: a face
//! fails when it is degenerate or when its normal points towards the crown axis, measured from a
//! point 0.01 mm below the centre of the cervical loop. Here the test runs on the template in its
//! own frame (`+Z` occlusal), where the textbook crowns have none; a library tooth that is not
//! star-shaped about its axis (deep embrasures, overhanging cusps) has some, and `build_crown`
//! rejects it until those faces are dealt with. The parametric crown's count is printed beside
//! it for comparison. The exit code is non-zero when a template does not build or is unsound.

use std::path::PathBuf;
use std::process::ExitCode;

use dentcraft_anatomy::{CrownParams, CrownTemplate, ToothLibrary, crown};
use dentcraft_case::Fdi;
use dentcraft_mesh::TriMesh;
use glam::DVec3;

/// Faces failing the radial Jacobian test about the template's cervical centre.
fn radial_failures(t: &CrownTemplate) -> usize {
    let origin = cervical_centre(t) - DVec3::Z * 0.01;
    (0..t.mesh.triangle_count())
        .filter(|&i| {
            let cross = t.mesh.triangle_cross(i);
            let [a, b, c] = t.mesh.triangle(i);
            !cross.is_finite()
                || cross.length_squared() < 1e-24
                || cross.dot((a + b + c) / 3.0 - origin) <= 0.0
        })
        .count()
}

fn cervical_centre(t: &CrownTemplate) -> DVec3 {
    t.cervical
        .iter()
        .map(|&v| t.mesh.positions[v as usize])
        .sum::<DVec3>()
        / t.cervical.len() as f64
}

/// Volume of the crown closed with a fan over its cervical loop (positive iff outward normals).
fn fan_volume(t: &CrownTemplate) -> f64 {
    let mut positions = t.mesh.positions.clone();
    let c = positions.len() as u32;
    positions.push(cervical_centre(t));
    let mut triangles = t.mesh.triangles.clone();
    let n = t.cervical.len();
    for k in 0..n {
        triangles.push([c, t.cervical[(k + 1) % n], t.cervical[k]]);
    }
    TriMesh::new(positions, triangles).signed_volume()
}

/// Problems with a built template, empty when it is sound.
fn problems(t: &CrownTemplate) -> Vec<String> {
    let mut out = Vec::new();
    let loops = t.mesh.boundary_loops();
    if loops.len() != 1 || loops[0].len() != t.cervical.len() {
        out.push(format!("{} boundary loops", loops.len()));
    }
    if !t.mesh.is_consistently_oriented() {
        out.push("inconsistent winding".into());
    }
    if fan_volume(t) <= 0.0 {
        out.push("inward normals".into());
    }
    out
}

fn main() -> ExitCode {
    let Some(dir) = std::env::var_os("DENTCRAFT_TOOTH_LIBRARY").map(PathBuf::from) else {
        eprintln!("set DENTCRAFT_TOOTH_LIBRARY to a tooth library folder");
        return ExitCode::FAILURE;
    };
    let library = match ToothLibrary::load(&dir) {
        Ok(library) => library,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let params = CrownParams::default();
    let mut failed = 0;
    println!("tooth  triangles  radial-fail  (parametric)  problems");
    for fdi in Fdi::all() {
        let parametric = radial_failures(&crown(fdi, &params));
        match library.crown(fdi, &params) {
            Ok(Some(t)) => {
                let problems = problems(&t);
                failed += usize::from(!problems.is_empty());
                println!(
                    "{:<7}{:>9}  {:>11}  {:>12}  {}",
                    fdi.code(),
                    t.mesh.triangle_count(),
                    radial_failures(&t),
                    parametric,
                    if problems.is_empty() {
                        "none".to_string()
                    } else {
                        problems.join(", ")
                    }
                );
            }
            Ok(None) => {
                failed += 1;
                println!("{:<7}not served", fdi.code());
            }
            Err(e) => {
                failed += 1;
                println!("{:<7}does not build: {e}", fdi.code());
            }
        }
    }
    if failed == 0 {
        println!("all 32 templates build and are sound");
        ExitCode::SUCCESS
    } else {
        println!("{failed} tooth/teeth failed");
        ExitCode::FAILURE
    }
}
