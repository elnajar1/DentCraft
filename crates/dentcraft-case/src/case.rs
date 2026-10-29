//! A design case: which teeth get which restoration, the scans they are designed on, and the
//! material that sets the design limits.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Design, Fdi, Material};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestorationKind {
    /// Full anatomic crown on a prepared tooth.
    Crown,
    /// Reduced framework of even thickness, to be veneered.
    Coping,
    Inlay,
    Onlay,
    Veneer,
    /// Replacement tooth carried by its bridge neighbours.
    Pontic,
}

impl RestorationKind {
    pub const ALL: [RestorationKind; 6] = [
        Self::Crown,
        Self::Coping,
        Self::Inlay,
        Self::Onlay,
        Self::Veneer,
        Self::Pontic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Crown => "Crown",
            Self::Coping => "Coping",
            Self::Inlay => "Inlay",
            Self::Onlay => "Onlay",
            Self::Veneer => "Veneer",
            Self::Pontic => "Pontic",
        }
    }

    /// Whether the unit sits on a prepared tooth (and so needs a margin line).
    pub fn has_preparation(self) -> bool {
        self != Self::Pontic
    }

    /// Whether the unit can be part of a bridge.
    pub fn can_connect(self) -> bool {
        matches!(self, Self::Crown | Self::Coping | Self::Pontic)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    pub tooth: Fdi,
    pub kind: RestorationKind,
}

/// A connector joins two neighbouring units into a bridge. Stored with `a < b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Connector {
    pub a: Fdi,
    pub b: Fdi,
}

impl Connector {
    pub fn new(x: Fdi, y: Fdi) -> Self {
        if x.arch_index() <= y.arch_index() {
            Self { a: x, b: y }
        } else {
            Self { a: y, b: x }
        }
    }

    pub fn touches(&self, tooth: Fdi) -> bool {
        self.a == tooth || self.b == tooth
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanRole {
    /// Upper jaw as it is now (with preparations, if any).
    Upper,
    Lower,
    /// Jaw scans taken before preparation, used as a shape reference.
    UpperPreOp,
    LowerPreOp,
    /// Buccal bite scan that places the two jaws relative to each other.
    Bite,
    Other,
}

impl ScanRole {
    pub const ALL: [ScanRole; 6] = [
        Self::Upper,
        Self::Lower,
        Self::UpperPreOp,
        Self::LowerPreOp,
        Self::Bite,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Upper => "Upper jaw",
            Self::Lower => "Lower jaw",
            Self::UpperPreOp => "Upper jaw (pre-op)",
            Self::LowerPreOp => "Lower jaw (pre-op)",
            Self::Bite => "Bite registration",
            Self::Other => "Other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scan {
    pub role: ScanRole,
    /// Path of the mesh file, relative to the case file when saved next to it.
    pub file: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub units: Vec<Unit>,
    #[serde(default)]
    pub connectors: Vec<Connector>,
    #[serde(default)]
    pub scans: Vec<Scan>,
    #[serde(default)]
    pub material: Material,
    /// The user's design decisions (file version 2 and later).
    #[serde(default, skip_serializing_if = "Design::is_empty")]
    pub design: Design,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaseIssue {
    #[error("tooth {0} has more than one restoration")]
    DuplicateTooth(Fdi),
    #[error("connector {0}–{1} joins teeth that are not neighbours")]
    ConnectorNotAdjacent(Fdi, Fdi),
    #[error("connector {0}–{1} needs a crown, coping or pontic on both teeth")]
    ConnectorWithoutUnits(Fdi, Fdi),
    #[error("pontic {0} is not connected to a neighbouring unit")]
    FloatingPontic(Fdi),
    #[error("no scan for the {0} arch, where tooth {1} is restored")]
    MissingJawScan(&'static str, Fdi),
}

impl Case {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            notes: String::new(),
            units: Vec::new(),
            connectors: Vec::new(),
            scans: Vec::new(),
            material: Material::default(),
            design: Design::default(),
        }
    }

    pub fn unit(&self, tooth: Fdi) -> Option<&Unit> {
        self.units.iter().find(|u| u.tooth == tooth)
    }

    /// Sets or replaces the restoration on a tooth. Connectors that no longer fit are dropped.
    pub fn set_unit(&mut self, tooth: Fdi, kind: RestorationKind) {
        match self.units.iter_mut().find(|u| u.tooth == tooth) {
            Some(unit) => unit.kind = kind,
            None => self.units.push(Unit { tooth, kind }),
        }
        if !kind.can_connect() {
            self.connectors.retain(|c| !c.touches(tooth));
        }
    }

    pub fn remove_unit(&mut self, tooth: Fdi) {
        self.units.retain(|u| u.tooth != tooth);
        self.connectors.retain(|c| !c.touches(tooth));
    }

    pub fn is_connected(&self, x: Fdi, y: Fdi) -> bool {
        self.connectors.contains(&Connector::new(x, y))
    }

    /// Connects or disconnects two units. Returns false when they cannot be connected.
    pub fn set_connected(&mut self, x: Fdi, y: Fdi, connected: bool) -> bool {
        let connector = Connector::new(x, y);
        if !connected {
            self.connectors.retain(|c| *c != connector);
            return true;
        }
        let both_connectable = [x, y]
            .iter()
            .all(|t| self.unit(*t).is_some_and(|u| u.kind.can_connect()));
        if !x.is_adjacent(y) || !both_connectable {
            return false;
        }
        if !self.connectors.contains(&connector) {
            self.connectors.push(connector);
        }
        true
    }

    /// Groups of connected units, each ordered along the arch. Single units are their own group.
    pub fn bridges(&self) -> Vec<Vec<Fdi>> {
        let mut teeth: Vec<Fdi> = self.units.iter().map(|u| u.tooth).collect();
        teeth.sort_by_key(|t| (t.arch() as u8, t.arch_index()));
        let mut groups: Vec<Vec<Fdi>> = Vec::new();
        for tooth in teeth {
            match groups.last_mut() {
                Some(group) if self.is_connected(*group.last().unwrap(), tooth) => {
                    group.push(tooth)
                }
                _ => groups.push(vec![tooth]),
            }
        }
        groups
    }

    pub fn scan(&self, role: ScanRole) -> Option<&Scan> {
        self.scans.iter().find(|s| s.role == role)
    }

    pub fn validate(&self) -> Vec<CaseIssue> {
        let mut issues = Vec::new();
        for (i, unit) in self.units.iter().enumerate() {
            if self.units[..i].iter().any(|u| u.tooth == unit.tooth) {
                issues.push(CaseIssue::DuplicateTooth(unit.tooth));
            }
        }
        for c in &self.connectors {
            if !c.a.is_adjacent(c.b) {
                issues.push(CaseIssue::ConnectorNotAdjacent(c.a, c.b));
            }
            let connectable = |t: Fdi| self.unit(t).is_some_and(|u| u.kind.can_connect());
            if !connectable(c.a) || !connectable(c.b) {
                issues.push(CaseIssue::ConnectorWithoutUnits(c.a, c.b));
            }
        }
        for unit in &self.units {
            if unit.kind == RestorationKind::Pontic
                && !self.connectors.iter().any(|c| c.touches(unit.tooth))
            {
                issues.push(CaseIssue::FloatingPontic(unit.tooth));
            }
        }
        if !self.scans.is_empty() {
            for unit in &self.units {
                let (role, arch) = match unit.tooth.arch() {
                    crate::Arch::Upper => (ScanRole::Upper, "upper"),
                    crate::Arch::Lower => (ScanRole::Lower, "lower"),
                };
                if self.scan(role).is_none() {
                    issues.push(CaseIssue::MissingJawScan(arch, unit.tooth));
                }
            }
        }
        issues
    }
}

/// On-disk form of a case: a versioned JSON document (`.dentcraft`).
#[derive(Serialize, Deserialize)]
struct CaseFile {
    format: String,
    version: u32,
    case: Case,
}

const FORMAT: &str = "dentcraft-case";
/// 2 added the design section; version 1 files load with an empty design.
const VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum CaseFileError {
    #[error("cannot read or write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a valid DentCraft case: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{path} is not a DentCraft case file")]
    WrongFormat { path: PathBuf },
    #[error("{path} was written by a newer DentCraft (case version {version})")]
    TooNew { path: PathBuf, version: u32 },
}

impl Case {
    pub fn to_json(&self) -> String {
        let file = CaseFile {
            format: FORMAT.into(),
            version: VERSION,
            case: self.clone(),
        };
        serde_json::to_string_pretty(&file).expect("a case always serializes")
    }

    pub fn save(&self, path: &Path) -> Result<(), CaseFileError> {
        std::fs::write(path, self.to_json()).map_err(|source| CaseFileError::Io {
            path: path.into(),
            source,
        })
    }

    pub fn load(path: &Path) -> Result<Case, CaseFileError> {
        let text = std::fs::read_to_string(path).map_err(|source| CaseFileError::Io {
            path: path.into(),
            source,
        })?;
        let file: CaseFile =
            serde_json::from_str(&text).map_err(|source| CaseFileError::Parse {
                path: path.into(),
                source,
            })?;
        if file.format != FORMAT {
            return Err(CaseFileError::WrongFormat { path: path.into() });
        }
        if file.version > VERSION {
            return Err(CaseFileError::TooNew {
                path: path.into(),
                version: file.version,
            });
        }
        Ok(file.case)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn three_unit_bridge() -> Case {
        let mut case = Case::new("bridge");
        case.set_unit(t(14), RestorationKind::Crown);
        case.set_unit(t(15), RestorationKind::Pontic);
        case.set_unit(t(16), RestorationKind::Crown);
        assert!(case.set_connected(t(14), t(15), true));
        assert!(case.set_connected(t(16), t(15), true));
        case
    }

    #[test]
    fn bridge_groups_follow_connectors() {
        let mut case = three_unit_bridge();
        case.set_unit(t(21), RestorationKind::Veneer);
        assert_eq!(case.bridges(), vec![vec![t(16), t(15), t(14)], vec![t(21)]]);
        assert!(case.validate().is_empty());
    }

    #[test]
    fn rejects_connectors_that_cannot_exist() {
        let mut case = three_unit_bridge();
        case.set_unit(t(21), RestorationKind::Veneer);
        case.set_unit(t(11), RestorationKind::Crown);
        assert!(
            !case.set_connected(t(11), t(21), true),
            "veneers cannot connect"
        );
        assert!(!case.set_connected(t(14), t(16), true), "not neighbours");
        assert!(!case.set_connected(t(13), t(14), true), "13 has no unit");
    }

    #[test]
    fn changing_a_unit_to_unconnectable_drops_its_connectors() {
        let mut case = three_unit_bridge();
        case.set_unit(t(14), RestorationKind::Inlay);
        assert_eq!(case.connectors, vec![Connector::new(t(15), t(16))]);
        case.remove_unit(t(16));
        assert_eq!(case.validate(), vec![CaseIssue::FloatingPontic(t(15))]);
    }

    #[test]
    fn reports_missing_jaw_scan_only_once_scans_exist() {
        let mut case = three_unit_bridge();
        assert!(case.validate().is_empty());
        case.scans.push(Scan {
            role: ScanRole::Lower,
            file: "lower.stl".into(),
        });
        assert_eq!(case.validate().len(), 3);
        case.scans.push(Scan {
            role: ScanRole::Upper,
            file: "upper.stl".into(),
        });
        assert!(case.validate().is_empty());
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dentcraft-case-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("case.dentcraft")
    }

    fn designed_bridge() -> Case {
        let mut case = three_unit_bridge();
        case.design.jaw_frame = Some(crate::JawFrame {
            origin: [1.0, 2.0, 3.0],
            occlusal: [0.0, 0.0, 1.0],
            forward: [0.0, 1.0, 0.0],
            upper: Some(true),
        });
        let tooth = case.design.teeth.entry(t(14)).or_default();
        tooth.margin = Some(vec![
            glam::DVec3::new(0.5, 0.25, -1.0),
            glam::DVec3::new(1.0 / 3.0, 2.0, 0.0),
            glam::DVec3::new(-4.0, 1e-9, 7.5),
        ]);
        tooth.insertion_axis = Some(glam::DVec3::new(0.0, 0.1, 0.995));
        tooth.inner_surface.cement_gap = Some(0.07);
        tooth.inner_surface.seal_width = Some(0.8);
        tooth.anatomy.cusp_height = 0.8;
        tooth.anatomy.rotation_deg = 7.5;
        tooth.anatomy.offset = [0.1, 0.0, -0.3];
        case.design
            .teeth
            .entry(t(16))
            .or_default()
            .inner_surface
            .seal_gap = Some(0.02);
        case
    }

    #[test]
    fn version_2_round_trips_the_design_exactly() {
        let case = designed_bridge();
        let path = temp_file("v2");
        case.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 2"), "{text}");
        assert_eq!(Case::load(&path).unwrap(), case);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_frames_jaw_is_optional_in_the_file() {
        let path = temp_file("jaw");
        let mut case = designed_bridge();
        case.design.jaw_frame.as_mut().unwrap().upper = Some(false);
        case.save(&path).unwrap();
        assert_eq!(Case::load(&path).unwrap(), case);
        case.design.jaw_frame.as_mut().unwrap().upper = None;
        case.save(&path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("upper"));
        assert_eq!(Case::load(&path).unwrap(), case);
        std::fs::write(
            &path,
            r#"{"format":"dentcraft-case","version":2,"case":{"name":"old","design":
                {"jaw_frame":{"origin":[0,0,0],"occlusal":[0,0,1],"forward":[0,1,0]}}}}"#,
        )
        .unwrap();
        let frame = Case::load(&path).unwrap().design.jaw_frame.unwrap();
        assert_eq!(frame.upper, None);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn version_1_files_load_with_an_empty_design() {
        let path = temp_file("v1");
        std::fs::write(
            &path,
            r#"{"format":"dentcraft-case","version":1,"case":{"name":"old",
                "units":[{"tooth":14,"kind":"crown"}]}}"#,
        )
        .unwrap();
        let case = Case::load(&path).unwrap();
        assert_eq!(case.units.len(), 1);
        assert!(case.design.is_empty());
        // An empty design is left out of the file.
        assert!(!Case::new("x").to_json().contains("design"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn malformed_designs_are_rejected() {
        let path = temp_file("bad");
        for design in [
            r#"{"teeth":{"99":{}}}"#,
            r#"{"teeth":{"14":{"margin":[[1.0,2.0]]}}}"#,
            r#"{"teeth":{"14":{"insertion_axis":"up"}}}"#,
            r#"{"jaw_frame":{"origin":[0,0,0]}}"#,
            r#"{"teeth":{"14":{"inner_surface":{"cement_gap":"wide"}}}}"#,
        ] {
            let text = format!(
                r#"{{"format":"dentcraft-case","version":2,"case":{{"name":"x","design":{design}}}}}"#
            );
            std::fs::write(&path, text).unwrap();
            assert!(
                matches!(Case::load(&path), Err(CaseFileError::Parse { .. })),
                "{design} was accepted"
            );
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn round_trips_through_json() {
        let case = three_unit_bridge();
        let dir = std::env::temp_dir().join(format!("dentcraft-case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("case.dentcraft");
        case.save(&path).unwrap();
        assert_eq!(Case::load(&path).unwrap(), case);
        std::fs::write(
            &path,
            r#"{"format":"dentcraft-case","version":99,"case":{"name":"x"}}"#,
        )
        .unwrap();
        assert!(matches!(
            Case::load(&path),
            Err(CaseFileError::TooNew { version: 99, .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
