//! Writes a synthetic prepared tooth as a binary STL and a case with one crown on 36 that uses it
//! as the lower jaw scan, for trying the margin step on known geometry.
//!
//! ```sh
//! cargo run -p dentcraft-design --example prepared_tooth_case -- [--undercut] [tooth.stl] [case.dentcraft]
//! ```
//!
//! The defaults are `scratch/demo/prepared-tooth.stl` and `scratch/case/prep.dentcraft`, both
//! gitignored. `--undercut` bulges the facial (+Y) half of the axial wall outward by up to
//! 0.35 mm between heights 9 and 11 mm, which leaves an undercut below the bulge, and writes
//! `scratch/demo/prepared-tooth-undercut.stl` and `scratch/case/prep-undercut.dentcraft`. The preparation stands on the origin along +Z; its top is at (0, 0, 12) and the
//! margin is the ring of radius 4 mm at height 8 mm.

use std::path::PathBuf;

use dentcraft_case::{Case, Fdi, JawFrame, RestorationKind, Scan, ScanRole};
use dentcraft_design::{ToothParams, prepared_tooth};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let undercut = args.first().is_some_and(|a| a.as_os_str() == "--undercut");
    if undercut {
        args.remove(0);
    }
    let suffix = if undercut { "-undercut" } else { "" };
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
    dentcraft_io::save_mesh(&stl, &tooth.mesh)?;

    let mut case = Case::new("Synthetic preparation 36");
    case.set_unit(Fdi::new(36)?, RestorationKind::Crown);
    case.scans.push(Scan {
        role: ScanRole::Lower,
        file: std::path::absolute(&stl)?,
    });
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
