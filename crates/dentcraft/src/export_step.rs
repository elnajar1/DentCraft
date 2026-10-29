//! The Finish & export step: every built crown, simplified within 2 µm with its margin locked,
//! written as a binary STL per tooth on a worker.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use dentcraft_case::{Fdi, RestorationKind, Unit};
use dentcraft_design::Crown;
use dentcraft_mesh::{DecimateOptions, decimate};
use eframe::egui;

/// Largest distance simplification may move the crown's surface, in millimetres.
pub const MAX_ERROR_MM: f64 = 0.002;

/// A case name made safe for a file name: letters and digits (any script), `-` and `_` stay;
/// runs of anything else become one `_`. An empty result becomes "case".
pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "case".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// `<case name>_<FDI>_<kind>.stl`, e.g. "Smith_J_36_crown.stl".
pub fn file_name(case_name: &str, tooth: Fdi, kind: RestorationKind) -> String {
    format!(
        "{}_{tooth}_{}.stl",
        sanitize(case_name),
        kind.label().to_lowercase()
    )
}

/// One unit of the case in the export list.
#[derive(Clone, Debug)]
pub struct PlanItem {
    pub tooth: Fdi,
    pub kind: RestorationKind,
    /// The unit's current crown; `None` when it is missing or out of date.
    pub crown: Option<Arc<Crown>>,
}

/// Every unit of the case along the arch, with its crown when one is built and current.
pub fn plan(units: &[Unit], crown: impl Fn(Fdi) -> Option<Arc<Crown>>) -> Vec<PlanItem> {
    let mut units = units.to_vec();
    units.sort_by_key(|u| (u.tooth.arch() as u8, u.tooth.arch_index()));
    units
        .into_iter()
        .map(|u| PlanItem {
            tooth: u.tooth,
            kind: u.kind,
            crown: crown(u.tooth),
        })
        .collect()
}

/// Simplification that keeps every margin vertex exactly where it is.
pub fn simplify_options(crown: &Crown) -> DecimateOptions {
    let mut locked = vec![false; crown.mesh.positions.len()];
    for &v in &crown.margin_vertices {
        if let Some(slot) = locked.get_mut(v as usize) {
            *slot = true;
        }
    }
    DecimateOptions {
        max_error: MAX_ERROR_MM,
        target_triangles: None,
        locked: Some(locked),
        keep_boundary: true,
    }
}

/// A crown written to disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Written {
    pub path: PathBuf,
    pub bytes: u64,
    pub triangles: usize,
}

/// Simplifies `crown` when asked and writes it to `path` as binary STL.
pub fn write_crown(crown: &Crown, path: &Path, simplify: bool) -> Result<Written, String> {
    let simplified;
    let mesh = if simplify {
        simplified = decimate(&crown.mesh, &simplify_options(crown))
            .map_err(|e| format!("Simplify failed: {e:?}"))?
            .0;
        &simplified
    } else {
        &crown.mesh
    };
    dentcraft_io::save_mesh(path, mesh).map_err(|e| format!("Write failed: {e}"))?;
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(Written {
        path: path.to_owned(),
        bytes,
        triangles: mesh.triangle_count(),
    })
}

/// "512 B", "84.1 kB", "1.25 MB".
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1000 {
        format!("{bytes} B")
    } else if bytes < 1_000_000 {
        format!("{:.1} kB", bytes as f64 / 1e3)
    } else {
        format!("{:.2} MB", bytes as f64 / 1e6)
    }
}

/// Opens `dir` in the system's file browser.
pub fn open_folder(dir: &Path) -> Result<(), String> {
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(dir)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Cannot open {}: {e}", dir.display()))
}

enum Message {
    Progress(String),
    Done(Fdi, Result<Written, String>),
    Finished,
}

/// The finished export: where it went and how much it wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub dir: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub failed: usize,
}

impl Summary {
    pub fn line(&self) -> String {
        let s = if self.files == 1 { "" } else { "s" };
        let mut text = format!("Wrote {} file{s}, {}", self.files, format_bytes(self.bytes));
        if self.failed > 0 {
            text.push_str(&format!(" \u{b7} {} failed", self.failed));
        }
        text
    }
}

/// One row of the step card.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRow {
    pub tooth: Fdi,
    pub kind: RestorationKind,
    /// Triangles and minimum thickness (mm) of the built crown.
    pub crown: Option<(usize, f64)>,
    /// Outcome of the last export of this tooth: triangles and bytes written, or the error.
    pub result: Option<Result<(usize, u64), String>>,
    /// Index of the Anatomy step when it works on this tooth.
    pub anatomy_step: Option<usize>,
}

/// What the Finish & export step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportCard {
    pub rows: Vec<ExportRow>,
    pub simplify: bool,
    pub busy: Option<String>,
    pub summary: Option<String>,
    pub folder: Option<PathBuf>,
}

/// The app side of the step: options, the export worker and its results.
pub struct ExportStep {
    pub simplify: bool,
    rx: Option<Receiver<Message>>,
    progress: Option<String>,
    results: BTreeMap<Fdi, Result<Written, String>>,
    summary: Option<Summary>,
    /// Where the running export writes.
    dir: Option<PathBuf>,
    repaint: Option<egui::Context>,
}

impl ExportStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        Self {
            simplify: true,
            rx: None,
            progress: None,
            results: BTreeMap::new(),
            summary: None,
            dir: None,
            repaint,
        }
    }

    /// Forgets the open case's results.
    pub fn reset(&mut self) {
        let simplify = self.simplify;
        *self = Self::new(self.repaint.take());
        self.simplify = simplify;
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Writes every ready crown of `items` into `dir` on a worker.
    pub fn start(&mut self, dir: PathBuf, case_name: &str, items: &[PlanItem]) {
        if self.busy() {
            return;
        }
        let jobs: Vec<(Fdi, PathBuf, Arc<Crown>)> = items
            .iter()
            .filter_map(|i| {
                let path = dir.join(file_name(case_name, i.tooth, i.kind));
                i.crown.clone().map(|c| (i.tooth, path, c))
            })
            .collect();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.results.clear();
        self.summary = None;
        self.progress = Some("Starting export\u{2026}".to_owned());
        self.dir = Some(dir);
        let simplify = self.simplify;
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let total = jobs.len();
            let poke = || {
                if let Some(ctx) = &repaint {
                    ctx.request_repaint();
                }
            };
            for (n, (tooth, path, crown)) in jobs.into_iter().enumerate() {
                let verb = if simplify { "Simplifying" } else { "Writing" };
                let _ = tx.send(Message::Progress(format!(
                    "{verb} {tooth}\u{2026} {}/{total}",
                    n + 1
                )));
                poke();
                let _ = tx.send(Message::Done(tooth, write_crown(&crown, &path, simplify)));
            }
            let _ = tx.send(Message::Finished);
            poke();
        });
    }

    /// Takes the worker's messages; call once a frame. `true` on the frame the export finishes.
    pub fn update(&mut self) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        let mut finished = false;
        while let Ok(message) = rx.try_recv() {
            match message {
                Message::Progress(text) => self.progress = Some(text),
                Message::Done(tooth, result) => {
                    self.results.insert(tooth, result);
                }
                Message::Finished => finished = true,
            }
        }
        if finished {
            self.rx = None;
            self.progress = None;
            let written = self.results.values().filter_map(|r| r.as_ref().ok());
            self.summary = Some(Summary {
                dir: self.dir.take().unwrap_or_default(),
                files: written.clone().count(),
                bytes: written.map(|w| w.bytes).sum(),
                failed: self.results.values().filter(|r| r.is_err()).count(),
            });
            for (tooth, result) in &self.results {
                if let Err(e) = result {
                    eprintln!("dentcraft: export of {tooth}: {e}");
                }
            }
        }
        finished
    }

    /// The last export's summary, once it has finished.
    pub fn summary(&self) -> Option<&Summary> {
        self.summary.as_ref()
    }

    /// The card for `items`; `anatomy_step(tooth)` is the Anatomy step's index for the tooth.
    pub fn card(
        &self,
        items: &[PlanItem],
        anatomy_step: impl Fn(Fdi) -> Option<usize>,
    ) -> ExportCard {
        ExportCard {
            rows: items
                .iter()
                .map(|i| ExportRow {
                    tooth: i.tooth,
                    kind: i.kind,
                    crown: i
                        .crown
                        .as_ref()
                        .map(|c| (c.mesh.triangle_count(), c.report.min_thickness)),
                    result: self.results.get(&i.tooth).map(|r| {
                        r.as_ref()
                            .map(|w| (w.triangles, w.bytes))
                            .map_err(Clone::clone)
                    }),
                    anatomy_step: anatomy_step(i.tooth),
                })
                .collect(),
            simplify: self.simplify,
            busy: self.progress.clone(),
            summary: self.summary.as_ref().map(Summary::line),
            folder: self.summary.as_ref().map(|s| s.dir.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_design::synthetic::crown_template;
    use dentcraft_design::{
        CrownParams, InnerSurfaceParams, StumpParams, auto_placement, build_crown, inner_surface,
        prepared_stump,
    };

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn test_crown() -> Crown {
        let die = prepared_stump(&StumpParams {
            segments: 96,
            ..Default::default()
        })
        .unwrap();
        let inner = inner_surface(
            &die.mesh,
            &die.region,
            &die.margin,
            die.axis,
            &InnerSurfaceParams {
                grid_step: 0.1,
                ..Default::default()
            },
        )
        .unwrap();
        let (template, cervical) = crown_template(4.0, 5.5, 48, 12).unwrap();
        let placement = auto_placement(&inner, &template, &cervical, die.axis, None);
        build_crown(
            &inner,
            &template,
            &cervical,
            placement,
            &CrownParams::default(),
        )
        .unwrap()
    }

    #[test]
    fn file_names_carry_the_case_tooth_and_kind() {
        assert_eq!(
            file_name("Smith, J. (2026)", t(36), RestorationKind::Crown),
            "Smith_J_2026_36_crown.stl"
        );
        assert_eq!(
            file_name("a/b\\c:d*e?f\"g<h>i|j", t(11), RestorationKind::Veneer),
            "a_b_c_d_e_f_g_h_i_j_11_veneer.stl"
        );
        assert_eq!(sanitize("  "), "case");
        assert_eq!(sanitize(""), "case");
        assert_eq!(sanitize("..."), "case");
        assert_eq!(sanitize("x-ray_01"), "x-ray_01");
        assert_eq!(sanitize("حالة ١"), "حالة_١");
    }

    #[test]
    fn the_plan_lists_every_unit_along_the_arch_with_its_crown() {
        let crown = Arc::new(Crown {
            mesh: dentcraft_mesh::TriMesh::default(),
            margin_vertices: vec![],
            report: dentcraft_design::CrownReport {
                min_thickness: 0.6,
                thickened_area: 0.0,
                max_thickening: 0.0,
            },
        });
        let units = [
            Unit {
                tooth: t(46),
                kind: RestorationKind::Crown,
            },
            Unit {
                tooth: t(36),
                kind: RestorationKind::Onlay,
            },
            Unit {
                tooth: t(45),
                kind: RestorationKind::Pontic,
            },
        ];
        let items = plan(&units, |tooth| (tooth == t(36)).then(|| crown.clone()));
        let order: Vec<Fdi> = items.iter().map(|i| i.tooth).collect();
        let mut expected = order.clone();
        expected.sort_by_key(|f| (f.arch() as u8, f.arch_index()));
        assert_eq!(order, expected);
        assert_eq!(items.len(), 3);
        let ready: Vec<Fdi> = items
            .iter()
            .filter(|i| i.crown.is_some())
            .map(|i| i.tooth)
            .collect();
        assert_eq!(ready, vec![t(36)]);
        assert_eq!(
            items.iter().find(|i| i.tooth == t(36)).unwrap().kind,
            RestorationKind::Onlay
        );
        let step = ExportStep::new(None);
        let card = step.card(&items, |tooth| (tooth != t(45)).then_some(7));
        let row = |f: Fdi| card.rows.iter().find(|r| r.tooth == f).unwrap();
        assert_eq!(row(t(36)).crown, Some((0, 0.6)));
        assert_eq!(row(t(46)).crown, None);
        assert_eq!(row(t(46)).anatomy_step, Some(7));
        assert_eq!(row(t(45)).anatomy_step, None);
    }

    #[test]
    fn simplification_locks_exactly_the_margin_vertices() {
        let crown = test_crown();
        let options = simplify_options(&crown);
        let locked = options.locked.as_ref().unwrap();
        assert_eq!(locked.len(), crown.mesh.positions.len());
        for (i, &l) in locked.iter().enumerate() {
            assert_eq!(l, crown.margin_vertices.contains(&(i as u32)), "vertex {i}");
        }
        assert_eq!(options.max_error, MAX_ERROR_MM);
        let (simplified, report) = decimate(&crown.mesh, &options).unwrap();
        assert!(report.triangles_after < report.triangles_before);
        for &v in &crown.margin_vertices {
            let p = crown.mesh.positions[v as usize];
            assert!(
                simplified.positions.contains(&p),
                "margin vertex {v} moved or was removed"
            );
        }
    }

    #[test]
    fn a_crown_round_trips_through_stl() {
        let crown = test_crown();
        assert!(crown.mesh.is_closed());
        let dir = std::env::temp_dir().join(format!("dentcraft-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(file_name("Round trip", t(36), RestorationKind::Crown));
        let written = write_crown(&crown, &path, true).unwrap();
        let loaded = dentcraft_io::load_mesh(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        let loaded = loaded.unwrap();
        assert_eq!(written.bytes, 84 + 50 * written.triangles as u64);
        assert_eq!(loaded.triangle_count(), written.triangles);
        assert!(written.triangles < crown.mesh.triangle_count());
        assert!(loaded.is_closed(), "the written crown is watertight");
        let (a, b) = (crown.mesh.bounds().unwrap(), loaded.bounds().unwrap());
        assert!(a.min.distance(b.min) <= MAX_ERROR_MM, "{a:?} vs {b:?}");
        assert!(a.max.distance(b.max) <= MAX_ERROR_MM, "{a:?} vs {b:?}");
        for &v in &crown.margin_vertices {
            // STL stores single precision.
            let p = crown.mesh.positions[v as usize].as_vec3().as_dvec3();
            assert!(
                loaded.positions.iter().any(|&q| q.distance(p) < 1e-9),
                "margin vertex {v} missing from the file"
            );
        }
    }

    #[test]
    fn a_failed_write_is_reported_per_tooth() {
        let crown = test_crown();
        let missing = std::env::temp_dir()
            .join(format!("dentcraft-export-none-{}", std::process::id()))
            .join("no")
            .join("x.stl");
        let err = write_crown(&crown, &missing, false).unwrap_err();
        assert!(err.starts_with("Write failed"), "{err}");
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(84_100), "84.1 kB");
        assert_eq!(format_bytes(1_250_000), "1.25 MB");
    }
}
