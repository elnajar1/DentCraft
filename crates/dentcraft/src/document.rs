//! The open case file: where it lives, whether it has unsaved changes, and how scan paths are
//! stored relative to it.

use std::path::{Component, Path, PathBuf};

use dentcraft_case::{Case, CaseFileError};

/// File extension of DentCraft cases.
pub const EXTENSION: &str = "dentcraft";

/// The case file behind the open case. The case is dirty when it differs from the state it
/// was last opened, saved or created in.
#[derive(Clone, Debug)]
pub struct Document {
    path: Option<PathBuf>,
    saved: Case,
    /// Restored from the recovery file: unsaved until the user saves it somewhere.
    recovered: bool,
}

impl Document {
    /// A new, never saved case.
    pub fn untitled(case: &Case) -> Self {
        Self {
            path: None,
            saved: case.clone(),
            recovered: false,
        }
    }

    /// A case restored from the recovery file: never saved, and dirty from the start.
    pub fn recovered(case: &Case) -> Self {
        Self {
            recovered: true,
            ..Self::untitled(case)
        }
    }

    /// A case just opened from or saved to `path`.
    pub fn at(path: PathBuf, case: &Case) -> Self {
        Self {
            path: Some(path),
            saved: case.clone(),
            recovered: false,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_dirty(&self, case: &Case) -> bool {
        self.recovered || *case != self.saved
    }

    /// Records that `case` was written to `path`.
    pub fn mark_saved(&mut self, path: PathBuf, case: &Case) {
        *self = Self::at(path, case);
    }
}

/// Window title for a case: its name, an unsaved mark, then the app.
pub fn window_title(case: &Case, dirty: bool) -> String {
    let name = display_name(case);
    let mark = if dirty { " •" } else { "" };
    format!("{name}{mark} — DentCraft")
}

pub fn display_name(case: &Case) -> &str {
    let name = case.name.trim();
    if name.is_empty() {
        "Untitled case"
    } else {
        name
    }
}

/// A file name for saving `case`: its name without characters file systems reject.
pub fn suggested_file_name(case: &Case) -> String {
    let stem: String = display_name(case)
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    format!("{}.{EXTENSION}", stem.trim_end_matches(['.', ' ']))
}

/// Gives `path` the case extension when it has none or another one.
pub fn with_case_extension(mut path: PathBuf) -> PathBuf {
    let has = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION));
    if !has {
        path.set_extension(EXTENSION);
    }
    path
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The folder a case file lives in, as an absolute path.
fn case_dir(case_file: &Path) -> PathBuf {
    absolute(case_file)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// The case as it is written to `case_file`: scans inside the case file's folder are stored
/// relative to it, with `/` separators so the file opens on any system; others stay absolute.
pub fn for_disk(case: &Case, case_file: &Path) -> Case {
    let dir = case_dir(case_file);
    let mut out = case.clone();
    for scan in &mut out.scans {
        let file = absolute(&scan.file);
        if let Ok(rel) = file.strip_prefix(&dir) {
            let parts: Vec<String> = rel
                .components()
                .filter_map(|c| match c {
                    Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            scan.file = PathBuf::from(parts.join("/"));
        } else {
            scan.file = file;
        }
    }
    out
}

/// The case as read from `case_file`, with relative scan paths resolved against its folder.
pub fn from_disk(mut case: Case, case_file: &Path) -> Case {
    let dir = case_dir(case_file);
    for scan in &mut case.scans {
        if scan.file.is_relative() {
            scan.file = dir.join(&scan.file);
        }
    }
    case
}

pub fn save(case: &Case, path: &Path) -> Result<(), CaseFileError> {
    for_disk(case, path).save(path)
}

pub fn load(path: &Path) -> Result<Case, CaseFileError> {
    Case::load(path).map(|case| from_disk(case, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_case::{Fdi, RestorationKind, Scan, ScanRole};

    /// A fresh directory under the system temp folder, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("dentcraft-doc-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn bridge_case(scans: Vec<Scan>) -> Case {
        let mut case = Case::new("Bridge 24–26");
        let t = |c| Fdi::new(c).unwrap();
        case.set_unit(t(24), RestorationKind::Crown);
        case.set_unit(t(25), RestorationKind::Pontic);
        case.set_unit(t(26), RestorationKind::Crown);
        assert!(case.set_connected(t(24), t(25), true));
        assert!(case.set_connected(t(25), t(26), true));
        case.scans = scans;
        case
    }

    #[test]
    fn scans_next_to_the_case_are_stored_relative_and_resolved_on_load() {
        let tmp = TempDir::new("roundtrip");
        let case_file = tmp.0.join("bridge.dentcraft");
        let inside = tmp.0.join("scans").join("upper.ply");
        let outside = std::env::temp_dir().join("elsewhere").join("lower.ply");
        let case = bridge_case(vec![
            Scan {
                role: ScanRole::Upper,
                file: inside.clone(),
            },
            Scan {
                role: ScanRole::Lower,
                file: outside.clone(),
            },
        ]);

        save(&case, &case_file).unwrap();
        let on_disk = Case::load(&case_file).unwrap();
        assert_eq!(on_disk.scans[0].file, PathBuf::from("scans/upper.ply"));
        assert_eq!(on_disk.scans[1].file, outside);

        let loaded = load(&case_file).unwrap();
        assert_eq!(loaded.scans[0].file, inside);
        assert_eq!(loaded.scans[1].file, outside);
        assert_eq!(loaded, case);
    }

    #[test]
    fn relative_case_paths_resolve_against_the_case_folder() {
        let tmp = TempDir::new("relative");
        let case_file = tmp.0.join("a.dentcraft");
        let mut case = bridge_case(vec![Scan {
            role: ScanRole::Upper,
            file: PathBuf::from("upper.ply"),
        }]);
        case.save(&case_file).unwrap();
        let loaded = load(&case_file).unwrap();
        assert_eq!(loaded.scans[0].file, tmp.0.join("upper.ply"));
        case.scans[0].file = tmp.0.join("upper.ply");
        assert_eq!(loaded, case);
    }

    #[test]
    fn dirty_flag_follows_edits_saves_and_reverts() {
        let mut case = Case::new("Untitled case");
        let mut doc = Document::untitled(&case);
        assert!(!doc.is_dirty(&case), "a new case is clean");
        assert_eq!(doc.path(), None);

        let t = Fdi::new(16).unwrap();
        case.set_unit(t, RestorationKind::Crown);
        assert!(doc.is_dirty(&case), "adding a unit dirties the case");

        case.remove_unit(t);
        assert!(
            !doc.is_dirty(&case),
            "undoing the edit makes it clean again"
        );

        case.name = "Smith".into();
        assert!(doc.is_dirty(&case), "renaming dirties the case");

        doc.mark_saved(PathBuf::from("smith.dentcraft"), &case);
        assert!(!doc.is_dirty(&case), "saving cleans it");
        assert_eq!(doc.path(), Some(Path::new("smith.dentcraft")));

        case.material = dentcraft_case::Material::pmma();
        assert!(doc.is_dirty(&case), "changing the material dirties it");

        let reopened = Document::at(PathBuf::from("smith.dentcraft"), &case);
        assert!(!reopened.is_dirty(&case), "an opened case is clean");
    }

    #[test]
    fn titles_and_file_names() {
        let mut case = Case::new("  ");
        assert_eq!(window_title(&case, false), "Untitled case — DentCraft");
        case.name = "Smith: 24/26".into();
        assert_eq!(window_title(&case, true), "Smith: 24/26 • — DentCraft");
        assert_eq!(suggested_file_name(&case), "Smith- 24-26.dentcraft");
        assert_eq!(
            with_case_extension(PathBuf::from("a/b")),
            PathBuf::from("a/b.dentcraft")
        );
        assert_eq!(
            with_case_extension(PathBuf::from("b.DentCraft")),
            PathBuf::from("b.DentCraft")
        );
    }
}
