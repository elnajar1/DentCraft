//! Tooth identification with the FDI two-digit notation (ISO 3950).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A permanent tooth in FDI notation: quadrant 1–4, position 1–8 (`11` … `48`).
///
/// Quadrants follow the patient: 1 upper right, 2 upper left, 3 lower left, 4 lower right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Fdi(u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    Upper,
    Lower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToothType {
    Incisor,
    Canine,
    Premolar,
    Molar,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0} is not a permanent tooth in FDI notation (11–48)")]
pub struct InvalidFdi(pub u8);

impl Fdi {
    pub fn new(code: u8) -> Result<Self, InvalidFdi> {
        let (quadrant, position) = (code / 10, code % 10);
        if (1..=4).contains(&quadrant) && (1..=8).contains(&position) {
            Ok(Self(code))
        } else {
            Err(InvalidFdi(code))
        }
    }

    /// Every permanent tooth, upper arch then lower arch, each from patient right to left.
    pub fn all() -> impl Iterator<Item = Fdi> {
        Arch::Upper.teeth().chain(Arch::Lower.teeth())
    }

    pub fn code(self) -> u8 {
        self.0
    }

    pub fn quadrant(self) -> u8 {
        self.0 / 10
    }

    /// 1 (central incisor) to 8 (third molar).
    pub fn position(self) -> u8 {
        self.0 % 10
    }

    pub fn arch(self) -> Arch {
        if self.quadrant() <= 2 {
            Arch::Upper
        } else {
            Arch::Lower
        }
    }

    pub fn tooth_type(self) -> ToothType {
        match self.position() {
            1 | 2 => ToothType::Incisor,
            3 => ToothType::Canine,
            4 | 5 => ToothType::Premolar,
            _ => ToothType::Molar,
        }
    }

    pub fn is_anterior(self) -> bool {
        self.position() <= 3
    }

    /// Index along the arch from the patient's right third molar (0) to the left one (15).
    pub fn arch_index(self) -> u8 {
        match self.quadrant() {
            // Patient's right side: 18 → 0 … 11 → 7, and 48 → 0 … 41 → 7.
            1 | 4 => 8 - self.position(),
            // Patient's left side: 21 → 8 … 28 → 15, and 31 → 8 … 38 → 15.
            _ => 7 + self.position(),
        }
    }

    /// True when the two teeth touch along the same arch, including across the midline.
    pub fn is_adjacent(self, other: Fdi) -> bool {
        self.arch() == other.arch() && self.arch_index().abs_diff(other.arch_index()) == 1
    }

    /// The tooth in the opposing arch at the same place (16 ↔ 46, 21 ↔ 31).
    pub fn antagonist(self) -> Fdi {
        let quadrant = match self.quadrant() {
            1 => 4,
            2 => 3,
            3 => 2,
            _ => 1,
        };
        Fdi(quadrant * 10 + self.position())
    }
}

impl Arch {
    /// The teeth of this arch from the patient's right to left.
    pub fn teeth(self) -> impl Iterator<Item = Fdi> {
        let (right, left) = match self {
            Arch::Upper => (1, 2),
            Arch::Lower => (4, 3),
        };
        (1..=8)
            .rev()
            .map(move |p| Fdi(right * 10 + p))
            .chain((1..=8).map(move |p| Fdi(left * 10 + p)))
    }
}

impl TryFrom<u8> for Fdi {
    type Error = InvalidFdi;
    fn try_from(code: u8) -> Result<Self, Self::Error> {
        Fdi::new(code)
    }
}

impl From<Fdi> for u8 {
    fn from(tooth: Fdi) -> u8 {
        tooth.0
    }
}

impl fmt::Display for Fdi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    #[test]
    fn rejects_invalid_codes() {
        for code in [0, 9, 10, 19, 50, 51, 99] {
            assert_eq!(Fdi::new(code), Err(InvalidFdi(code)));
        }
    }

    #[test]
    fn there_are_32_permanent_teeth() {
        assert_eq!(Fdi::all().count(), 32);
    }

    #[test]
    fn arch_index_runs_right_to_left() {
        let upper: Vec<u8> = Arch::Upper.teeth().map(Fdi::arch_index).collect();
        assert_eq!(upper, (0..16).collect::<Vec<_>>());
        let lower: Vec<u8> = Arch::Lower.teeth().map(Fdi::arch_index).collect();
        assert_eq!(lower, (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn adjacency_crosses_the_midline_but_not_the_arches() {
        assert!(t(11).is_adjacent(t(21)));
        assert!(t(41).is_adjacent(t(31)));
        assert!(t(14).is_adjacent(t(15)));
        assert!(!t(14).is_adjacent(t(16)));
        assert!(!t(11).is_adjacent(t(41)));
        assert!(!t(11).is_adjacent(t(11)));
    }

    #[test]
    fn antagonists_and_types() {
        assert_eq!(t(16).antagonist(), t(46));
        assert_eq!(t(21).antagonist(), t(31));
        assert_eq!(t(36).antagonist().antagonist(), t(36));
        assert_eq!(t(13).tooth_type(), ToothType::Canine);
        assert_eq!(t(45).tooth_type(), ToothType::Premolar);
        assert!(t(23).is_anterior() && !t(24).is_anterior());
    }

    #[test]
    fn serializes_as_a_number() {
        assert_eq!(serde_json::to_string(&t(36)).unwrap(), "36");
        assert!(serde_json::from_str::<Fdi>("19").is_err());
    }
}
