//! Headless DentCraft: mesh info, mesh conversion and case plans.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, error::ErrorKind};
use dentcraft_case::{Case, Connector, Fdi, RestorationKind, Unit};
use dentcraft_io::{load_mesh, save_mesh};
use dentcraft_mesh::TriMesh;
use dentcraft_workflow::plan;
use serde::Serialize;

#[derive(Parser)]
#[command(name = "dentcraft", version, about = "Headless DentCraft dental CAD")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print size, area, volume and topology of a mesh (.stl, .ply or .obj).
    Info {
        mesh: PathBuf,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Convert a mesh between .stl, .ply and .obj.
    Convert {
        input: PathBuf,
        output: PathBuf,
        /// Merge vertices closer than this distance (mm) before writing.
        #[arg(long, value_name = "MM")]
        weld: Option<f64>,
    },
    /// Print the design steps for a case.
    Plan {
        /// A restoration as TOOTH=KIND, e.g. 14=crown. Kinds: crown, coping, inlay, onlay,
        /// veneer, pontic.
        #[arg(long = "unit", value_name = "TOOTH=KIND")]
        units: Vec<String>,
        /// A connector between two teeth, e.g. 14-15.
        #[arg(long = "connect", value_name = "A-B")]
        connectors: Vec<String>,
    },
}

#[derive(Serialize)]
struct Info {
    vertices: usize,
    triangles: usize,
    bounds_min_mm: Option<[f64; 3]>,
    bounds_max_mm: Option<[f64; 3]>,
    size_mm: Option<[f64; 3]>,
    surface_area_mm2: f64,
    closed: bool,
    volume_mm3: Option<f64>,
    components: usize,
    boundary_loops: usize,
    non_manifold_edges: usize,
}

fn info(mesh: &TriMesh) -> Info {
    let bounds = mesh.bounds();
    let closed = mesh.is_closed();
    Info {
        vertices: mesh.vertex_count(),
        triangles: mesh.triangle_count(),
        bounds_min_mm: bounds.map(|b| b.min.to_array()),
        bounds_max_mm: bounds.map(|b| b.max.to_array()),
        size_mm: bounds.map(|b| b.size().to_array()),
        surface_area_mm2: mesh.surface_area(),
        closed,
        volume_mm3: closed.then(|| mesh.signed_volume()),
        components: mesh.connected_components().len(),
        boundary_loops: mesh.boundary_loops().len(),
        non_manifold_edges: mesh.non_manifold_edge_count(),
    }
}

fn info_text(i: &Info) -> String {
    let xyz = |v: [f64; 3]| format!("{:.3}  {:.3}  {:.3}", v[0], v[1], v[2]);
    let mut rows: Vec<(&str, String)> = vec![
        ("vertices", i.vertices.to_string()),
        ("triangles", i.triangles.to_string()),
    ];
    for (label, value) in [
        ("bounds min (mm)", i.bounds_min_mm),
        ("bounds max (mm)", i.bounds_max_mm),
        ("size (mm)", i.size_mm),
    ] {
        rows.push((label, value.map_or("none".into(), xyz)));
    }
    rows.push(("surface area (mm2)", format!("{:.3}", i.surface_area_mm2)));
    rows.push(("closed", if i.closed { "yes" } else { "no" }.into()));
    if let Some(volume) = i.volume_mm3 {
        rows.push(("volume (mm3)", format!("{volume:.3}")));
    }
    rows.push(("components", i.components.to_string()));
    rows.push(("boundary loops", i.boundary_loops.to_string()));
    rows.push(("non-manifold edges", i.non_manifold_edges.to_string()));
    rows.iter()
        .map(|(label, value)| format!("{label:<20}{value}\n"))
        .collect()
}

fn parse_tooth(text: &str) -> Result<Fdi, String> {
    let code: u8 = text
        .trim()
        .parse()
        .map_err(|_| format!("'{text}' is not a tooth code"))?;
    Fdi::new(code).map_err(|e| e.to_string())
}

fn parse_kind(text: &str) -> Result<RestorationKind, String> {
    let lower = text.trim().to_ascii_lowercase();
    RestorationKind::ALL
        .into_iter()
        .find(|k| k.label().to_ascii_lowercase() == lower)
        .ok_or_else(|| {
            format!("unknown restoration kind '{text}' (expected crown, coping, inlay, onlay, veneer or pontic)")
        })
}

fn build_case(units: &[String], connectors: &[String]) -> Result<Case, String> {
    let mut case = Case::new("cli");
    for spec in units {
        let (tooth, kind) = spec
            .split_once('=')
            .ok_or_else(|| format!("--unit '{spec}': expected TOOTH=KIND, e.g. 14=crown"))?;
        case.units.push(Unit {
            tooth: parse_tooth(tooth)?,
            kind: parse_kind(kind)?,
        });
    }
    for spec in connectors {
        let (a, b) = spec
            .split_once('-')
            .ok_or_else(|| format!("--connect '{spec}': expected A-B, e.g. 14-15"))?;
        case.connectors
            .push(Connector::new(parse_tooth(a)?, parse_tooth(b)?));
    }
    Ok(case)
}

fn plan_text(case: &Case) -> String {
    let mut out = String::new();
    let issues = case.validate();
    if !issues.is_empty() {
        out.push_str("Issues:\n");
        for issue in &issues {
            out.push_str(&format!("  - {issue}\n"));
        }
        out.push('\n');
    }
    out.push_str("Plan:\n");
    for (n, step) in plan(case).iter().enumerate() {
        out.push_str(&format!("  {}. {}", n + 1, step.id.title()));
        if !step.teeth.is_empty() {
            let teeth: Vec<String> = step.teeth.iter().map(|t| t.to_string()).collect();
            out.push_str(&format!(" ({})", teeth.join(", ")));
        }
        if step.id.is_optional() {
            out.push_str(" (optional)");
        }
        out.push('\n');
    }
    out
}

fn run(command: Command) -> Result<String, String> {
    match command {
        Command::Info { mesh, json } => {
            let info = info(&load_mesh(&mesh).map_err(|e| e.to_string())?);
            if json {
                let text = serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?;
                Ok(text + "\n")
            } else {
                Ok(info_text(&info))
            }
        }
        Command::Convert {
            input,
            output,
            weld,
        } => {
            let mut mesh = load_mesh(&input).map_err(|e| e.to_string())?;
            if let Some(tolerance) = weld {
                if !tolerance.is_finite() || tolerance < 0.0 {
                    return Err("--weld must be a non-negative number of millimetres".into());
                }
                mesh.weld(tolerance);
            }
            save_mesh(&output, &mesh).map_err(|e| e.to_string())?;
            Ok(format!(
                "wrote {}: {} vertices, {} triangles\n",
                output.display(),
                mesh.vertex_count(),
                mesh.triangle_count()
            ))
        }
        Command::Plan { units, connectors } => Ok(plan_text(&build_case(&units, &connectors)?)),
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let ok = matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
            let text = e.to_string();
            let _ = if ok {
                std::io::stdout().write_all(text.as_bytes())
            } else {
                std::io::stderr().write_all(text.as_bytes())
            };
            return if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    match run(cli.command) {
        Ok(text) => {
            // A closed pipe is not worth a panic or a failure.
            let _ = std::io::stdout().write_all(text.as_bytes());
            ExitCode::SUCCESS
        }
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "error: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn tetrahedron() -> TriMesh {
        TriMesh::new(
            vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z],
            vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
        )
    }

    #[test]
    fn info_reports_closed_volume() {
        let text = info_text(&info(&tetrahedron()));
        assert!(text.contains("closed              yes"), "{text}");
        assert!(text.contains("volume (mm3)        0.167"), "{text}");
        let open = TriMesh::new(tetrahedron().positions, vec![[0, 2, 1]]);
        let text = info_text(&info(&open));
        assert!(text.contains("closed              no"));
        assert!(!text.contains("volume"));
    }

    #[test]
    fn case_arguments_are_parsed_and_checked() {
        let units = ["14=crown".to_string(), "15=Pontic".to_string()];
        let case = build_case(&units, &["15-14".to_string()]).unwrap();
        assert_eq!(case.units.len(), 2);
        assert_eq!(case.connectors.len(), 1);
        for bad in ["19=crown", "14", "14=bridge", "x=crown"] {
            assert!(build_case(&[bad.to_string()], &[]).is_err(), "{bad}");
        }
        assert!(build_case(&[], &["14".to_string()]).is_err());
    }

    #[test]
    fn plan_lists_issues_for_duplicates_and_floating_pontics() {
        let units = ["15=pontic".to_string(), "15=crown".to_string()];
        let text = plan_text(&build_case(&units, &[]).unwrap());
        assert!(text.starts_with("Issues:"), "{text}");
        assert!(text.contains("more than one restoration"));
        assert!(text.contains("not connected"));
    }
}
