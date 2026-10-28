//! Workspaces: the kinds of restoration work DentCraft is organised into. Each has its own
//! ribbon tabs over the same case; the switcher at the left end of the ribbon lists them.

use eframe::egui::Color32;

use crate::ribbon::RibbonTab;
use crate::theme::Palette;

/// Footer note under the workspace list.
pub const FOOTER: &str = "Every workspace works on the same case, scans and teeth; only the \
    ribbon changes. Planned workspaces are on the roadmap.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The workspace the ribbon shows now.
    Current,
    /// Ships and can be switched to.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no workspace is available until a second one ships"
        )
    )]
    Available,
    /// On the roadmap; listed but not selectable.
    Planned,
}

impl Status {
    /// Text of the badge next to the workspace title.
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "Current",
            Self::Available => "Available",
            Self::Planned => "Planned",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    CrownBridge,
    ModelBuilder,
    Implant,
    Splint,
    Denture,
    SmileDesign,
}

impl Workspace {
    /// Every workspace, in menu order.
    pub const ALL: [Workspace; 6] = [
        Self::CrownBridge,
        Self::ModelBuilder,
        Self::Implant,
        Self::Splint,
        Self::Denture,
        Self::SmileDesign,
    ];

    /// The workspace the ribbon shows.
    pub fn current() -> Self {
        Self::ALL
            .into_iter()
            .find(|w| w.status() == Status::Current)
            .unwrap_or(Self::CrownBridge)
    }

    /// Title in the workspace menu.
    pub fn title(self) -> &'static str {
        match self {
            Self::CrownBridge => "Crown & Bridge",
            Self::ModelBuilder => "Model Builder",
            Self::Implant => "Implant",
            Self::Splint => "Splint",
            Self::Denture => "Denture",
            Self::SmileDesign => "Smile Design",
        }
    }

    /// Name under the icon of the switcher, which is only 88 px wide.
    pub fn short_name(self) -> &'static str {
        match self {
            Self::CrownBridge => "Crowns",
            Self::ModelBuilder => "Models",
            Self::Implant => "Implant",
            Self::Splint => "Splint",
            Self::Denture => "Denture",
            Self::SmileDesign => "Smile",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::CrownBridge => "Crowns, bridges, inlays, onlays and veneers on prepared teeth",
            Self::ModelBuilder => "Printable models with bases, dies and labels",
            Self::Implant => "Custom abutments, screw-retained crowns and bars",
            Self::Splint => "Night guards and occlusal splints",
            Self::Denture => "Full and partial dentures",
            Self::SmileDesign => "Smile previews and mock-ups from photos",
        }
    }

    pub fn status(self) -> Status {
        match self {
            Self::CrownBridge => Status::Current,
            _ => Status::Planned,
        }
    }

    /// Whether the menu lets the user pick this workspace.
    pub fn is_enabled(self) -> bool {
        self.status() != Status::Planned
    }

    /// The ribbon tabs of a workspace that ships, joined for the menu; planned ones have none.
    pub fn tabs(self) -> Option<String> {
        match self {
            Self::CrownBridge => Some(RibbonTab::ALL.map(RibbonTab::label).join(" \u{b7} ")),
            _ => None,
        }
    }

    /// Colour of the row stripe, badge and highlight. A workspace that ships later gets its own
    /// colour here, as SolidCraft's Render workspace is pink.
    pub fn accent(self, p: &Palette) -> Color32 {
        match self.status() {
            Status::Planned => p.text_dim,
            Status::Current | Status::Available => p.accent,
        }
    }
}

/// The row a keyboard highlight moves to from `from`, skipping rows that are not `enabled` and
/// wrapping at the ends. With no highlight yet, Down starts at the first row and Up at the last.
pub fn step_highlight(enabled: &[bool], from: Option<usize>, forward: bool) -> Option<usize> {
    let n = enabled.len();
    if n == 0 {
        return None;
    }
    // A virtual start just outside the list, so the first step lands on an end row.
    let start = match (from, forward) {
        (Some(k), _) => k.min(n - 1) as isize,
        (None, true) => -1,
        (None, false) => n as isize,
    };
    let step = if forward { 1 } else { -1 };
    (1..=n as isize)
        .map(|i| (start + step * i).rem_euclid(n as isize) as usize)
        .find(|&k| enabled[k])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_workspace_is_current() {
        let current: Vec<_> = Workspace::ALL
            .into_iter()
            .filter(|w| w.status() == Status::Current)
            .collect();
        assert_eq!(current, vec![Workspace::CrownBridge]);
        assert_eq!(Workspace::current(), Workspace::CrownBridge);
    }

    #[test]
    fn planned_workspaces_have_no_tabs_and_cannot_be_picked() {
        for w in Workspace::ALL {
            let planned = w.status() == Status::Planned;
            assert_eq!(w.tabs().is_none(), planned, "{w:?}");
            assert_eq!(w.is_enabled(), !planned, "{w:?}");
        }
        assert_eq!(
            Workspace::CrownBridge.tabs().as_deref(),
            Some("Case \u{b7} Design \u{b7} View")
        );
    }

    #[test]
    fn every_workspace_has_a_title_short_name_and_description() {
        for w in Workspace::ALL {
            assert!(!w.title().trim().is_empty(), "{w:?}");
            assert!(!w.short_name().trim().is_empty(), "{w:?}");
            assert!(!w.description().trim().is_empty(), "{w:?}");
        }
    }

    #[test]
    fn every_status_has_a_badge() {
        let labels = [Status::Current, Status::Available, Status::Planned].map(Status::label);
        assert_eq!(labels, ["Current", "Available", "Planned"]);
    }

    #[test]
    fn planned_rows_take_the_dim_colour() {
        let p = Palette::DARK;
        assert_eq!(Workspace::CrownBridge.accent(&p), p.accent);
        assert_eq!(Workspace::Denture.accent(&p), p.text_dim);
    }

    #[test]
    fn the_highlight_skips_disabled_rows_and_wraps() {
        let rows = [false, true, false, true, false];
        assert_eq!(step_highlight(&rows, None, true), Some(1));
        assert_eq!(step_highlight(&rows, Some(1), true), Some(3));
        assert_eq!(step_highlight(&rows, Some(3), true), Some(1));
        assert_eq!(step_highlight(&rows, None, false), Some(3));
        assert_eq!(step_highlight(&rows, Some(3), false), Some(1));
        assert_eq!(step_highlight(&rows, Some(1), false), Some(3));
        // From a disabled row, the next enabled one in that direction.
        assert_eq!(step_highlight(&rows, Some(2), true), Some(3));
        assert_eq!(step_highlight(&rows, Some(2), false), Some(1));
    }

    #[test]
    fn a_single_enabled_row_keeps_the_highlight() {
        let rows = Workspace::ALL.map(Workspace::is_enabled);
        assert_eq!(step_highlight(&rows, None, true), Some(0));
        assert_eq!(step_highlight(&rows, Some(0), true), Some(0));
        assert_eq!(step_highlight(&rows, Some(0), false), Some(0));
    }

    #[test]
    fn no_enabled_rows_means_no_highlight() {
        assert_eq!(step_highlight(&[], None, true), None);
        assert_eq!(step_highlight(&[false, false], None, true), None);
        assert_eq!(step_highlight(&[false, false], Some(1), false), None);
    }
}
