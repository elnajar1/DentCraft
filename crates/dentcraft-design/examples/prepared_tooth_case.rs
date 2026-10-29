//! Writes a synthetic prepared tooth as a binary STL and a case with one crown on 36 that uses it
//! as the lower jaw scan, for trying the margin step on known geometry.
//!
//! ```sh
//! cargo run -p dentcraft-design --example prepared_tooth_case -- [--undercut] [--neighbours] [tooth.stl] [case.dentcraft]
//! ```
//!
//! The defaults are `scratch/demo/prepared-tooth.stl` and `scratch/case/prep.dentcraft`, both
//! gitignored. `--undercut` bulges the facial (+Y) half of the axial wall outward by up to
//! 0.35 mm between heights 9 and 11 mm, which leaves an undercut below the bulge, and writes
//! `scratch/demo/prepared-tooth-undercut.stl` and `scratch/case/prep-undercut.dentcraft`.
//! `--neighbours` merges two box-shaped neighbour teeth 0.3 mm from the crown along ±Y into the
//! lower scan and adds an antagonist dome as an upper scan whose lowest point is 0.2 mm below the
//! crown's highest cusp; it writes `scratch/demo/prepared-tooth-neighbours.stl`,
//! `scratch/demo/antagonist.stl` and `scratch/case/prep-neighbours.dentcraft`. The preparation
//! stands on the origin along +Z; its top is at (0, 0, 12) and the margin is the ring of radius
//! 4 mm at height 8 mm.

use std::path::PathBuf;

use dentcraft_case::{Case, Fdi, JawFrame, RestorationKind, Scan, ScanRole};
use dentcraft_design::synthetic::rounded_box;
use dentcraft_design::{ToothParams, prepared_tooth};
use dentcraft_mesh::TriMesh;
use glam::DVec3;

/// The crown's outer surface reaches about this far along ±Y, mm.
const CROWN_HALF_WIDTH: f64 = 5.3;
/// Gap between the crown and each neighbour, mm.
const NEIGHBOUR_GAP: f64 = 0.3;
/// Height of the crown's highest cusp, mm (measured on the built crown).
const CROWN_TOP: f64 = 16.3;
/// How far the antagonist's lowest point is below the highest cusp, mm.
const ANTAGONIST_DEPTH: f64 = 0.2;

fn translated(mut mesh: TriMesh, by: DVec3) -> TriMesh {
    for p in &mut mesh.positions {
        *p += by;
    }
    mesh
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let mut flag = |name: &str| {
        let found = args.iter().position(|a| a.as_os_str() == name);
        found.map(|i| args.remove(i)).is_some()
    };
    let undercut = flag("--undercut");
    let neighbours = flag("--neighbours");
    let suffix = match (undercut, neighbours) {
        (false, false) => "",
        (true, false) => "-undercut",
        (false, true) => "-neighbours",
        (true, true) => "-undercut-neighbours",
    };
    let mut args = args.into_iter();
    let stl = args
        .next()
        .unwrap_or_else(|| format!("scratch/demo/prepared-tooth{suffix}.stl").into());
    let case_path = args
        .next()
        .unwrap_or_else(|| format!("scratch/case/prep{suffix}.dentcraft").into());
    for path in [&stl, &case_path] {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
    }

    let mut tooth = prepared_tooth(&ToothParams::default())?;
    if undercut {
        for p in &mut tooth.mesh.positions {
            let radius = p.truncate().length();
            if (9.0..=11.0).contains(&p.z) && radius > 2.0 {
                let height = (std::f64::consts::FRAC_PI_2 * (p.z - 10.0)).cos().powi(2);
                let facial = (p.y / radius).max(0.0);
                let scale = 1.0 + 0.35 * height * facial / radius;
                p.x *= scale;
                p.y *= scale;
            }
        }
    }
    let mut antagonist = None;
    if neighbours {
        let centre = CROWN_HALF_WIDTH + NEIGHBOUR_GAP + 5.0;
        for side in [-1.0, 1.0] {
            let neighbour = rounded_box(DVec3::new(5.0, 5.0, 4.0), 1.5, 24)?;
            tooth
                .mesh
                .merge(&translated(neighbour, DVec3::new(0.0, side * centre, 10.0)));
        }
        let radius = 4.0;
        let dome = TriMesh::uv_sphere(radius, 96, 48);
        let lowest = CROWN_TOP - ANTAGONIST_DEPTH;
        antagonist = Some(translated(dome, DVec3::new(0.0, 0.0, lowest + radius)));
    }
    dentcraft_io::save_mesh(&stl, &tooth.mesh)?;

    let mut case = Case::new("Synthetic preparation 36");
    case.set_unit(Fdi::new(36)?, RestorationKind::Crown);
    case.scans.push(Scan {
        role: ScanRole::Lower,
        file: std::path::absolute(&stl)?,
    });
    if let Some(dome) = &antagonist {
        let file = stl.with_file_name("antagonist.stl");
        dentcraft_io::save_mesh(&file, dome)?;
        case.scans.push(Scan {
            role: ScanRole::Upper,
            file: std::path::absolute(&file)?,
        });
    }
    case.design.jaw_frame = Some(JawFrame {
        origin: [0.0; 3],
        occlusal: [0.0, 0.0, 1.0],
        forward: [0.0, 1.0, 0.0],
    });
    case.save(&case_path)?;
    println!(
        "wrote {} ({} triangles) and {}",
        stl.display(),
        tooth.mesh.triangles.len(),
        case_path.display()
    );
    Ok(())
}
