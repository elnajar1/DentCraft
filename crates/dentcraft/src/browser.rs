//! Browser panels: collapsible sections with a title row, as in the side panels of a CAD app.

use eframe::egui::{
    self, Align, CursorIcon, FontId, Layout, Margin, Rect, Sense, TextStyle, Ui, UiBuilder, vec2,
};

use crate::widgets::{self, Icon, paint_icon};

/// Storage key of the collapsed sections.
pub const SECTIONS_KEY: &str = "browser_sections";
/// Height of a section's title row.
pub const HEADER_HEIGHT: f32 = 24.0;
/// Padding around a section's content.
pub const BODY_MARGIN: Margin = Margin {
    left: 10,
    right: 10,
    top: 8,
    bottom: 10,
};

/// A collapsible section of the case or workflow panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Case,
    Teeth,
    Material,
    Scans,
    Workflow,
    Step,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Self::Case,
        Self::Teeth,
        Self::Material,
        Self::Scans,
        Self::Workflow,
        Self::Step,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Case => "Case",
            Self::Teeth => "Teeth",
            Self::Material => "Material",
            Self::Scans => "Scans",
            Self::Workflow => "Workflow",
            Self::Step => "Step",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Case => "case",
            Self::Teeth => "teeth",
            Self::Material => "material",
            Self::Scans => "scans",
            Self::Workflow => "workflow",
            Self::Step => "step",
        }
    }
}

/// Which sections are open. Every section starts open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sections {
    open: [bool; Section::ALL.len()],
}

impl Default for Sections {
    fn default() -> Self {
        Self {
            open: [true; Section::ALL.len()],
        }
    }
}

impl Sections {
    pub fn is_open(&self, section: Section) -> bool {
        self.open[section as usize]
    }

    pub fn open_mut(&mut self, section: Section) -> &mut bool {
        &mut self.open[section as usize]
    }

    /// The collapsed sections as a comma-separated list, for storage. Storing the collapsed
    /// ones keeps sections added later open.
    pub fn to_key(self) -> String {
        Section::ALL
            .iter()
            .filter(|s| !self.is_open(**s))
            .map(|s| s.key())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Reads [`Sections::to_key`]; unknown names are ignored.
    pub fn from_key(text: &str) -> Self {
        let mut sections = Self::default();
        for name in text.split(',').map(str::trim) {
            if let Some(section) = Section::ALL.into_iter().find(|s| s.key() == name) {
                *sections.open_mut(section) = false;
            }
        }
        sections
    }
}

/// Title row of a section, spanning the full width of `ui`: a chevron and the title on the
/// header fill, `extra` laid out right to left at its end, then a hairline. A click on the
/// row outside `extra`'s widgets opens or closes the section.
///
/// Expects `ui` to have no vertical item spacing, so rows and hairlines butt together.
pub fn header(ui: &mut Ui, section: Section, open: &mut bool, extra: impl FnOnce(&mut Ui)) {
    let p = widgets::palette(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEADER_HEIGHT), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() {
            p.hover
        } else {
            p.panel_header
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        let chevron = if *open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let icon = Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(10.0, 10.0));
        paint_icon(ui.painter(), chevron, icon, p.text_dim);
        let font: FontId = TextStyle::Body.resolve(ui.style());
        ui.painter().text(
            rect.left_center() + vec2(26.0, 0.0),
            egui::Align2::LEFT_CENTER,
            section.title(),
            font,
            p.text,
        );
    }
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(6.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    extra(&mut row);
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        *open = !*open;
    }
    let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, p.border);
}

/// Content of an open section, padded by [`BODY_MARGIN`] and with the usual item spacing.
pub fn body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .inner_margin(BODY_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = ui.ctx().global_style().spacing.item_spacing;
            add(ui)
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_starts_open() {
        let sections = Sections::default();
        assert!(Section::ALL.iter().all(|s| sections.is_open(*s)));
        assert_eq!(sections.to_key(), "");
        assert_eq!(Sections::from_key(""), sections);
    }

    #[test]
    fn collapsed_sections_survive_a_round_trip() {
        let mut sections = Sections::default();
        *sections.open_mut(Section::Teeth) = false;
        *sections.open_mut(Section::Step) = false;
        assert_eq!(sections.to_key(), "teeth,step");
        assert_eq!(Sections::from_key(&sections.to_key()), sections);
    }

    #[test]
    fn unknown_section_names_are_ignored() {
        let sections = Sections::from_key("scans, gallery,,");
        assert!(!sections.is_open(Section::Scans));
        assert!(
            Section::ALL
                .iter()
                .filter(|s| **s != Section::Scans)
                .all(|s| sections.is_open(*s))
        );
    }
}
