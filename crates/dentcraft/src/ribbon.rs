//! The ribbon under the app bar: tabs, and grouped icon-over-label commands under each tab.

use dentcraft_case::ScanRole;
use dentcraft_render::ViewPreset;
use dentcraft_workflow::Session;
use eframe::egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Rect, Response, Sense, Stroke, Ui, Vec2,
    pos2, vec2,
};

use crate::theme::{RADIUS_SM, SIZE_BODY, SIZE_SMALL};
use crate::widgets::{self, Icon, paint_icon};
use crate::workflow_panel::{self, StepAction};

/// Height of the tab strip.
const TAB_HEIGHT: f32 = 28.0;
/// Minimum size of a command button.
const COMMAND_SIZE: Vec2 = vec2(56.0, 52.0);
/// Height of a group caption.
const CAPTION_HEIGHT: f32 = 15.0;
/// Space above and below the command row.
const GROUP_PAD: f32 = 4.0;
/// Total height of the ribbon: tab strip, hairline, commands and captions.
pub const HEIGHT: f32 = TAB_HEIGHT + 1.0 + 2.0 * GROUP_PAD + COMMAND_SIZE.y + CAPTION_HEIGHT;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RibbonTab {
    #[default]
    Case,
    Design,
    View,
}

impl RibbonTab {
    pub const ALL: [RibbonTab; 3] = [Self::Case, Self::Design, Self::View];

    pub fn label(self) -> &'static str {
        match self {
            Self::Case => "Case",
            Self::Design => "Design",
            Self::View => "View",
        }
    }
}

/// What a ribbon command asked the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    New,
    Open,
    Save,
    Import,
    Step(StepAction),
    View(ViewPreset),
    Fit,
    /// Show or hide every scan with this role.
    ShowRole(ScanRole, bool),
}

/// Label of the command that completes the active step.
pub fn next_label(session: &Session) -> &'static str {
    let (done, total) = session.progress();
    if done + 1 == total {
        "Finish"
    } else if session.current_index() == 0 {
        "Start"
    } else {
        "Next"
    }
}

/// The state the ribbon shows, borrowed from the app for one frame.
pub struct RibbonView<'a> {
    pub tab: &'a mut RibbonTab,
    pub session: &'a Session,
    pub has_jaw_scan: bool,
    /// Where Save writes, if the case has been saved before.
    pub save_path: Option<String>,
    /// The view the camera was last sent to.
    pub preset: Option<ViewPreset>,
    /// Whether upper and lower jaw scans are shown; `None` when there are none.
    pub upper_visible: Option<bool>,
    pub lower_visible: Option<bool>,
}

impl RibbonView<'_> {
    pub fn show(self, ui: &mut Ui) -> Option<Command> {
        let p = widgets::palette(ui);
        let full = ui.max_rect();
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.horizontal(|ui| {
            ui.set_height(TAB_HEIGHT);
            ui.add_space(8.0);
            for tab in RibbonTab::ALL {
                if tab_button(ui, tab.label(), *self.tab == tab).clicked() {
                    *self.tab = tab;
                }
            }
        });
        ui.painter().hline(
            full.x_range(),
            full.top() + TAB_HEIGHT + 0.5,
            Stroke::new(1.0, p.border),
        );
        ui.add_space(1.0 + GROUP_PAD);

        let mut command = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(6.0);
            match *self.tab {
                RibbonTab::Case => self.case_tab(ui, &mut command),
                RibbonTab::Design => self.design_tab(ui, &mut command),
                RibbonTab::View => self.view_tab(ui, &mut command),
            }
        });
        command
    }

    fn case_tab(&self, ui: &mut Ui, command: &mut Option<Command>) {
        group(ui, "File", |ui| {
            if command_button(ui, Icon::NewDoc, "New", true, false)
                .on_hover_text("Start a new, empty case")
                .clicked()
            {
                *command = Some(Command::New);
            }
            if command_button(ui, Icon::FolderOpen, "Open", true, false)
                .on_hover_text("Open a .dentcraft case and its scans")
                .clicked()
            {
                *command = Some(Command::Open);
            }
            let tip = match &self.save_path {
                Some(path) => format!("Save to {path}"),
                None => "Save the case as a .dentcraft file".to_owned(),
            };
            if command_button(ui, Icon::Save, "Save", true, false)
                .on_hover_text(tip)
                .clicked()
            {
                *command = Some(Command::Save);
            }
        });
        divider(ui);
        group(ui, "Scans", |ui| {
            if command_button(ui, Icon::Import, "Import", true, false)
                .on_hover_text("Add STL, PLY or OBJ scans to the case")
                .clicked()
            {
                *command = Some(Command::Import);
            }
        });
    }

    fn design_tab(&self, ui: &mut Ui, command: &mut Option<Command>) {
        let session = self.session;
        let step = session.current();
        group(ui, "Workflow", |ui| {
            if command_button(ui, Icon::Back, "Back", session.current_index() > 0, false)
                .on_hover_text("Re-open the previous step")
                .clicked()
            {
                *command = Some(Command::Step(StepAction::Back));
            }
            let ready = step.is_some_and(|s| workflow_panel::can_complete(s.id, self.has_jaw_scan));
            let next = command_button(ui, Icon::Next, next_label(session), ready, false);
            let next = match step {
                Some(_) => next
                    .on_hover_text("Complete this step")
                    .on_disabled_hover_text(workflow_panel::NEEDS_JAW_SCAN),
                None => next.on_disabled_hover_text("Every step is done"),
            };
            if next.clicked() {
                *command = Some(Command::Step(StepAction::Next));
            }
            let optional = step.is_some_and(|s| s.id.is_optional());
            if command_button(ui, Icon::Skip, "Skip", optional, false)
                .on_hover_text("Leave this step out")
                .on_disabled_hover_text("Only optional steps can be skipped")
                .clicked()
            {
                *command = Some(Command::Step(StepAction::Skip));
            }
        });
        divider(ui);
        group(ui, "Tools", |ui| {
            command_button(ui, Icon::Tooth, "Tools arrive in 0.2", false, false)
                .on_disabled_hover_text(workflow_panel::NOT_YET_NOTE);
        });
    }

    fn view_tab(&self, ui: &mut Ui, command: &mut Option<Command>) {
        group(ui, "Views", |ui| {
            for (n, preset) in ViewPreset::ALL.into_iter().enumerate() {
                let on = self.preset == Some(preset);
                if command_button(ui, Icon::View(preset), preset.label(), true, on)
                    .on_hover_text(format!("{} view  ({})", preset.label(), n + 1))
                    .clicked()
                {
                    *command = Some(Command::View(preset));
                }
            }
            if command_button(ui, Icon::Fit, "Fit", true, false)
                .on_hover_text("Fit the scene in view  (F, or double-click)")
                .clicked()
            {
                *command = Some(Command::Fit);
            }
        });
        divider(ui);
        group(ui, "Scans", |ui| {
            for (role, jaw, visible) in [
                (ScanRole::Upper, "upper", self.upper_visible),
                (ScanRole::Lower, "lower", self.lower_visible),
            ] {
                let on = visible.unwrap_or(false);
                let response =
                    command_button(ui, Icon::Eye, &format!("Show {jaw}"), visible.is_some(), on)
                        .on_hover_text(format!("Show or hide the {jaw} jaw scans"))
                        .on_disabled_hover_text(format!("No {jaw} jaw scan in the case"));
                if response.clicked() {
                    *command = Some(Command::ShowRole(role, !on));
                }
            }
        });
    }
}

/// One tab of the tab strip; the active tab is underlined in the accent.
fn tab_button(ui: &mut Ui, label: &str, active: bool) -> Response {
    let p = widgets::palette(ui);
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(SIZE_BODY),
        Color32::PLACEHOLDER,
    );
    let size = vec2(galley.size().x + 24.0, TAB_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = response.hovered();
        if hovered && !active {
            painter.rect_filled(
                rect.shrink2(vec2(2.0, 4.0)),
                CornerRadius::same(RADIUS_SM),
                p.hover,
            );
        }
        let ink = if active || hovered {
            p.text
        } else {
            p.text_dim
        };
        painter.galley(rect.center() - galley.size() * 0.5, galley, ink);
        if active {
            let bar = Rect::from_min_max(
                pos2(rect.left() + 8.0, rect.bottom() - 2.0),
                pos2(rect.right() - 8.0, rect.bottom()),
            );
            painter.rect_filled(bar, CornerRadius::same(1), p.accent);
        }
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Commands in a row with a caption below them.
fn group(ui: &mut Ui, caption: &str, add: impl FnOnce(&mut Ui)) {
    let p = widgets::palette(ui);
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            add(ui);
        });
        let width = ui.min_rect().width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, CAPTION_HEIGHT), Sense::hover());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            caption,
            FontId::proportional(SIZE_SMALL - 0.5),
            p.text_dim,
        );
    });
}

/// Thin vertical line between groups.
fn divider(ui: &mut Ui) {
    let p = widgets::palette(ui);
    let height = COMMAND_SIZE.y + CAPTION_HEIGHT - 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(13.0, height), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.top() + 3.0..=rect.bottom(),
        Stroke::new(1.0, p.border),
    );
}

/// Ribbon command: an icon above a small label.
fn command_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool, selected: bool) -> Response {
    let p = widgets::palette(ui);
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(SIZE_SMALL),
        Color32::PLACEHOLDER,
    );
    let width = COMMAND_SIZE.x.max(galley.size().x + 12.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, COMMAND_SIZE.y), sense);
    if ui.is_rect_visible(rect) {
        let (fill, ink) = widgets::button_colors(&p, &response, enabled, selected);
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
        let icon_rect =
            Rect::from_center_size(pos2(rect.center().x, rect.top() + 18.0), Vec2::splat(20.0));
        paint_icon(painter, icon, icon_rect, ink, p);
        let text_pos = pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.bottom() - 8.0 - galley.size().y,
        );
        painter.galley(text_pos, galley, ink);
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_case::{Case, Fdi, RestorationKind};

    #[test]
    fn the_next_command_reads_start_next_then_finish() {
        let mut case = Case::new("t");
        case.set_unit(Fdi::new(16).unwrap(), RestorationKind::Crown);
        let mut session = Session::for_case(&case);
        assert!(session.steps().len() > 2);
        assert_eq!(next_label(&session), "Start");
        session.complete().unwrap();
        assert_eq!(next_label(&session), "Next");
        while session.progress().0 + 1 < session.progress().1 {
            session.complete().unwrap();
        }
        assert_eq!(next_label(&session), "Finish");
    }
}
