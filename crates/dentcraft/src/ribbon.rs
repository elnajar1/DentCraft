//! The ribbon under the app bar: tabs, and grouped icon-over-label commands under each tab.

use dentcraft_case::ScanRole;
use dentcraft_render::ViewPreset;
use dentcraft_workflow::{Session, StepId};
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Galley, Key, Modifiers, Painter, Pos2,
    Rect, RectAlign, Response, RichText, Sense, Shape, Stroke, Ui, UiBuilder, Vec2, pos2,
    text::LayoutJob, vec2,
};

use crate::anatomy_step::ANATOMY;
use crate::axis_step::AXIS;
use crate::inner_step::INNER;
use crate::margin_step::MARGIN;
use crate::orient_step::ORIENT;
use crate::theme::{RADIUS_LG, RADIUS_MD, RADIUS_SM, SIZE_BODY, SIZE_HEADING, SIZE_SMALL};
use crate::tools::MEASURE;
use crate::widgets::{self, Icon, paint_icon};
use crate::workflow_panel::{self, StepAction};
use crate::workspace::{self, Status, Workspace};

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
/// Width of the workspace switcher at the left end of the ribbon.
const SWITCHER_WIDTH: f32 = 88.0;
/// Width of the rows of the workspace menu.
const MENU_WIDTH: f32 = 450.0;

/// Open state and keyboard highlight of the workspace menu, kept by the app between frames.
#[derive(Debug, Default)]
pub struct WorkspaceMenu {
    pub open: bool,
    /// Row picked out with Up and Down.
    key_row: Option<usize>,
}

impl WorkspaceMenu {
    pub fn new(open: bool) -> Self {
        Self {
            open,
            key_row: None,
        }
    }
}

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
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Start or leave the Measure tool.
    Measure,
    /// Open the Orientation step, or leave its tool.
    Orient,
    /// Open the Margin step, or leave its tool.
    Margin,
    /// Open the Insertion axis step, or leave its tool.
    Axis,
    /// Open the Inner surface step, or leave its tool.
    Inner,
    /// Open the Anatomy step, or leave its tool.
    Anatomy,
    /// Open the Finish & export step.
    Export,
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
    pub workspace_menu: &'a mut WorkspaceMenu,
    pub session: &'a Session,
    pub has_jaw_scan: bool,
    /// Where Save writes, if the case has been saved before.
    pub save_path: Option<String>,
    /// The view the camera was last sent to.
    pub preset: Option<ViewPreset>,
    /// Whether upper and lower jaw scans are shown; `None` when there are none.
    pub upper_visible: Option<bool>,
    pub lower_visible: Option<bool>,
    /// Name of the active viewport tool.
    pub active_tool: Option<&'static str>,
}

impl RibbonView<'_> {
    pub fn show(self, ui: &mut Ui) -> Option<Command> {
        let full = ui.max_rect();
        let block = Rect::from_min_size(full.min, vec2(SWITCHER_WIDTH, full.height()));
        workspace_switcher(ui, block, self.workspace_menu);
        let rest = Rect::from_min_max(pos2(block.right(), full.top()), full.max);
        ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| {
            self.tabs_and_commands(ui)
        })
        .inner
    }

    /// The tab strip and the commands of the active tab, right of the workspace switcher.
    fn tabs_and_commands(self, ui: &mut Ui) -> Option<Command> {
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
            let orient_planned = session.steps().iter().any(|s| s.id == StepId::Orient);
            let orient_on = self.active_tool == Some(ORIENT);
            if command_button(ui, Icon::Orient, "Orient", orient_planned, orient_on)
                .on_hover_text("Find the jaw's occlusal plane and midline  (Esc leaves)")
                .on_disabled_hover_text("The case has no Orientation step")
                .clicked()
            {
                *command = Some(Command::Orient);
            }
            let planned = session.steps().iter().any(|s| s.id == StepId::Margin);
            let on = self.active_tool == Some(MARGIN);
            if command_button(ui, Icon::Margin, "Margin", planned, on)
                .on_hover_text("Trace the margin line on the current tooth  (Esc leaves)")
                .on_disabled_hover_text("No crown, coping, inlay, onlay or veneer in the case")
                .clicked()
            {
                *command = Some(Command::Margin);
            }
            let planned = session
                .steps()
                .iter()
                .any(|s| s.id == StepId::InsertionAxis);
            let on = self.active_tool == Some(AXIS);
            if command_button(ui, Icon::Axis, "Axis", planned, on)
                .on_hover_text("Set the insertion axis of the current tooth  (Esc leaves)")
                .on_disabled_hover_text("The case has no Insertion axis step")
                .clicked()
            {
                *command = Some(Command::Axis);
            }
            let planned = session.steps().iter().any(|s| s.id == StepId::InnerSurface);
            let on = self.active_tool == Some(INNER);
            if command_button(ui, Icon::Inner, "Inner", planned, on)
                .on_hover_text("Build the inner surface of the current tooth  (Esc leaves)")
                .on_disabled_hover_text("The case has no Inner surface step")
                .clicked()
            {
                *command = Some(Command::Inner);
            }
            let planned = session.steps().iter().any(|s| s.id == StepId::Anatomy);
            let on = self.active_tool == Some(ANATOMY);
            if command_button(ui, Icon::Anatomy, "Anatomy", planned, on)
                .on_hover_text("Place and shape the crown of the current tooth  (Esc leaves)")
                .on_disabled_hover_text("The case has no Anatomy step")
                .clicked()
            {
                *command = Some(Command::Anatomy);
            }
            let on = session.current().is_some_and(|s| s.id == StepId::Finish);
            if command_button(ui, Icon::Export, "Export", true, on)
                .on_hover_text("Write the finished crowns as STL files")
                .clicked()
            {
                *command = Some(Command::Export);
            }
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
        divider(ui);
        group(ui, "Inspect", |ui| {
            let on = self.active_tool == Some(MEASURE);
            if command_button(ui, Icon::Ruler, "Measure", true, on)
                .on_hover_text("Measure between two points on the scans  (Esc or Delete clears)")
                .clicked()
            {
                *command = Some(Command::Measure);
            }
        });
    }
}

/// The block at the left end of the ribbon that shows the current workspace and opens the
/// workspace menu below it. Adapted from SolidCraft's workspace switcher.
fn workspace_switcher(ui: &mut Ui, block: Rect, menu: &mut WorkspaceMenu) {
    let p = widgets::palette(ui);
    let current = Workspace::current();
    let response = ui.interact(
        block,
        ui.scope_id().with("workspace_switcher"),
        Sense::click(),
    );
    if response.clicked() {
        menu.open = !menu.open;
    }
    let painter = ui.painter();
    if response.hovered() || menu.open {
        painter.rect_filled(block, CornerRadius::ZERO, p.hover);
    }
    painter.rect_filled(
        Rect::from_min_size(block.min, vec2(block.width(), 3.0)),
        CornerRadius::ZERO,
        current.accent(&p),
    );
    painter.vline(
        block.right() - 0.5,
        block.y_range(),
        Stroke::new(1.0, p.border),
    );
    let x = block.center().x;
    let icon = Rect::from_center_size(pos2(x, block.top() + 34.0), Vec2::splat(30.0));
    paint_icon(painter, Icon::Workspace(current), icon, p.text, p);
    let name = painter.layout_no_wrap(
        current.short_name().to_owned(),
        FontId::proportional(SIZE_BODY),
        p.text,
    );
    let name_pos = pos2(x, block.top() + 64.0) - name.size() * 0.5;
    semibold(painter, name_pos, name, p.text);
    // The label and its dropdown arrow are centred together.
    let label = painter.layout_no_wrap(
        "Workspace".to_owned(),
        FontId::proportional(SIZE_SMALL),
        p.text_dim,
    );
    let label_pos = pos2(x - 6.0, block.top() + 84.0);
    let arrow = pos2(label_pos.x + label.size().x * 0.5 + 7.0, label_pos.y + 0.5);
    painter.galley(label_pos - label.size() * 0.5, label, p.text_dim);
    painter.add(Shape::convex_polygon(
        vec![
            arrow + vec2(-3.5, -1.75),
            arrow + vec2(3.5, -1.75),
            arrow + vec2(0.0, 2.25),
        ],
        p.text_dim,
        Stroke::NONE,
    ));
    let response = response
        .on_hover_text("Change workspace")
        .on_hover_cursor(CursorIcon::PointingHand);

    // Up and Down move a highlight over the rows that can be picked; Enter picks.
    let mut picked = None;
    if menu.open {
        let enabled = Workspace::ALL.map(Workspace::is_enabled);
        let (down, up, enter) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if down || up {
            menu.key_row = workspace::step_highlight(&enabled, menu.key_row, down);
        }
        if enter {
            picked = menu.key_row.map(|k| Workspace::ALL[k]);
        }
    }
    let key_row = menu.key_row;
    let max_height = (ui.ctx().content_rect().bottom() - block.bottom() - 116.0).max(60.0);
    if picked.is_none() {
        picked = egui::Popup::from_response(&response)
            .open_bool(&mut menu.open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .align(RectAlign::BOTTOM_START)
            .align_alternatives(&[])
            .frame(egui::Frame::popup(ui.style()).inner_margin(9))
            .show(|ui| workspace_menu(ui, key_row, max_height))
            .and_then(|r| r.inner);
    }
    // Picking the current workspace just closes the menu. Switching to another arrives with the
    // second workspace that ships; until then no other row can be picked.
    if picked.is_some() {
        menu.open = false;
    }
    if !menu.open {
        menu.key_row = None;
    }
}

/// The workspace menu: a header, a row per workspace and a footer note. Returns the workspace
/// clicked. Adapted from SolidCraft's workspace menu.
fn workspace_menu(ui: &mut Ui, key_row: Option<usize>, max_height: f32) -> Option<Workspace> {
    let p = widgets::palette(ui);
    ui.set_width(MENU_WIDTH);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(4.0);
    ui.label(
        RichText::new("WORKSPACE")
            .size(SIZE_SMALL)
            .color(p.text_dim),
    );
    ui.add_space(8.0);
    let mut picked = None;
    egui::ScrollArea::vertical()
        .id_salt("workspace_menu_rows")
        .max_height(max_height)
        .show(ui, |ui| {
            for (k, w) in Workspace::ALL.into_iter().enumerate() {
                if workspace_row(ui, w, key_row == Some(k)) {
                    picked = Some(w);
                }
            }
        });
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            RichText::new(workspace::FOOTER)
                .size(SIZE_SMALL)
                .color(p.text_dim),
        )
        .wrap(),
    );
    ui.add_space(4.0);
    picked
}

/// One row of the workspace menu: accent stripe, icon tile, title with a status badge, the
/// description and, for a workspace that ships, its tabs. Returns true when it was clicked.
fn workspace_row(ui: &mut Ui, w: Workspace, key_highlight: bool) -> bool {
    let p = widgets::palette(ui);
    let status = w.status();
    let planned = status == Status::Planned;
    let current = status == Status::Current;
    let enabled = w.is_enabled();
    let accent = w.accent(&p);
    let soft = p.panel.lerp_to_gamma(accent, 0.10);
    let height = if planned { 60.0 } else { 72.0 };
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (r, response) = ui.allocate_exact_size(vec2(MENU_WIDTH, height), sense);
    let painter = ui.painter();
    if current || (enabled && (response.hovered() || key_highlight)) {
        painter.rect_filled(r, CornerRadius::same(RADIUS_MD - 1), soft);
    }
    painter.rect_filled(
        Rect::from_min_max(
            r.min + vec2(2.0, 7.0),
            pos2(r.left() + 6.0, r.bottom() - 7.0),
        ),
        CornerRadius::same(RADIUS_SM / 2),
        accent,
    );
    let tile = Rect::from_center_size(pos2(r.left() + 40.0, r.center().y), Vec2::splat(44.0));
    let tile_fill = if planned { p.panel_header } else { soft };
    painter.rect_filled(tile, CornerRadius::same(RADIUS_LG + 1), tile_fill);
    let icon_ink = if enabled {
        p.text
    } else {
        p.text.gamma_multiply(0.35)
    };
    paint_icon(painter, Icon::Workspace(w), tile.shrink(7.0), icon_ink, p);

    let text = if enabled { p.text } else { p.text_dim };
    let x = r.left() + 74.0;
    let y = r.top() + 17.0;
    let title = painter.layout_no_wrap(
        w.title().to_owned(),
        FontId::proportional(SIZE_HEADING),
        text,
    );
    let badge_x = x + title.size().x + 9.0;
    semibold(painter, pos2(x, y - title.size().y * 0.5), title, text);
    let badge = painter.layout_no_wrap(
        status.label().to_owned(),
        FontId::proportional(SIZE_SMALL),
        if current { p.on_accent } else { accent },
    );
    let badge_rect = Rect::from_min_size(pos2(badge_x, y - 8.0), badge.size() + vec2(12.0, 3.0));
    let badge_fill = if current { accent } else { soft };
    painter.rect_filled(badge_rect, CornerRadius::same(RADIUS_LG), badge_fill);
    painter.galley(badge_rect.min + vec2(6.0, 1.5), badge, accent);
    let line = |content: String, y: f32| {
        let mut job = LayoutJob::simple_singleline(content, FontId::proportional(SIZE_BODY), text);
        job.wrap.max_width = r.right() - x - 6.0;
        job.wrap.max_rows = 1;
        painter.galley(pos2(x, y), painter.layout_job(job), text);
    };
    line(w.description().to_owned(), r.top() + 29.0);
    if let Some(tabs) = w.tabs() {
        line(tabs, r.top() + 47.0);
    }
    enabled && response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// Paints `galley` again half a point to the right for a semibold look; the bundled fonts have
/// no bold face.
fn semibold(painter: &Painter, origin: Pos2, galley: Arc<Galley>, ink: Color32) {
    let ppp = painter.pixels_per_point();
    let origin = ((origin.to_vec2() * ppp).round() / ppp).to_pos2();
    painter.galley(origin + vec2(0.5, 0.0), galley.clone(), ink);
    painter.galley(origin, galley, ink);
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
