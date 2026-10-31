//! The floating panels over the viewport: the main toolbar at the top left (workspace switcher,
//! file commands, the Design and Tools menus) and the view cluster at the bottom right.

use dentcraft_case::ScanRole;
use dentcraft_render::ViewPreset;
use dentcraft_workflow::{Session, StepId};
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Galley, Id, Key, Modifiers, Painter,
    Pos2, Rect, RectAlign, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2,
    text::LayoutJob, vec2,
};

use crate::anatomy_step::ANATOMY;
use crate::axis_step::AXIS;
use crate::contacts_step::CONTACTS;
use crate::inner_step::INNER;
use crate::margin_step::MARGIN;
use crate::orient_step::ORIENT;
use crate::theme::{RADIUS_LG, RADIUS_MD, RADIUS_SM, SIZE_BODY, SIZE_HEADING, SIZE_SMALL};
use crate::tools::MEASURE;
use crate::widgets::{self, Icon, paint_icon};
use crate::workflow_panel::{self, StepAction};
use crate::workspace::{self, Status, Workspace};

/// Side of a toolbar button.
const BUTTON: f32 = 30.0;
/// Side of a view cluster button.
const VIEW_BUTTON: f32 = 28.0;
/// Padding inside a floating panel.
const PAD: f32 = 4.0;
/// Space between neighbouring buttons.
const GAP: f32 = 2.0;
/// Width taken by a divider between groups of buttons.
const DIVIDER: f32 = 9.0;
/// Width of the workspace switcher: its icon and a dropdown arrow.
const SWITCHER_WIDTH: f32 = 44.0;
/// Width of the Design and Tools menu buttons.
const MENU_BUTTON_WIDTH: f32 = 74.0;
/// Distance of the floating panels from the viewport edges.
pub const INSET: f32 = 10.0;
/// Height of the toolbar panel.
pub const HEIGHT: f32 = BUTTON + 2.0 * PAD;
/// Width of the rows of the Design and Tools menus.
const MENU_ROW_WIDTH: f32 = 220.0;
/// Width of the rows of the workspace menu.
const MENU_WIDTH: f32 = 450.0;
/// Names of the toolbar's groups, for the workspace menu.
pub const GROUPS: [&str; 4] = ["File", "Design", "Tools", "Views"];

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

/// What a toolbar, view cluster or app bar command asked the app to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    New,
    Open,
    Save,
    Import,
    /// Open the case setup screen: teeth, material and scans.
    CaseSetup,
    Step(StepAction),
    View(ViewPreset),
    Fit,
    /// Frame the tooth being designed.
    FitTooth,
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
    /// Open the Occlusion & contacts step, or leave its tool.
    Contacts,
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

/// A place in the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Workspace,
    New,
    Open,
    Save,
    Import,
    CaseSetup,
    DesignMenu,
    ToolsMenu,
}

/// The toolbar's slots in order: width, and whether a divider comes before it.
const SLOTS: [(Slot, f32, bool); 8] = [
    (Slot::Workspace, SWITCHER_WIDTH, false),
    (Slot::New, BUTTON, true),
    (Slot::Open, BUTTON, false),
    (Slot::Save, BUTTON, false),
    (Slot::Import, BUTTON, false),
    (Slot::CaseSetup, BUTTON, true),
    (Slot::DesignMenu, MENU_BUTTON_WIDTH, true),
    (Slot::ToolsMenu, MENU_BUTTON_WIDTH, false),
];

/// The toolbar panel in the top-left corner of `viewport`, and the rectangle of each slot.
pub fn toolbar_layout(viewport: Rect) -> (Rect, Vec<(Slot, Rect)>) {
    let top = viewport.top() + INSET;
    let mut x = viewport.left() + INSET + PAD;
    let mut slots = Vec::with_capacity(SLOTS.len());
    for (n, (slot, width, divider)) in SLOTS.into_iter().enumerate() {
        if divider {
            x += DIVIDER - GAP;
        } else if n > 0 {
            x += GAP;
        }
        let rect = Rect::from_min_size(pos2(x, top + PAD), vec2(width, BUTTON));
        slots.push((slot, rect));
        x = rect.right();
    }
    let panel = Rect::from_min_max(
        pos2(viewport.left() + INSET, top),
        pos2(x + PAD, top + HEIGHT),
    );
    (panel, slots)
}

/// The view cluster panel in the bottom-right corner of `viewport`, and a button per view.
pub fn view_cluster_layout(viewport: Rect) -> (Rect, Vec<(ViewPreset, Rect)>) {
    let n = ViewPreset::ALL.len() as f32;
    let size = vec2(
        n * VIEW_BUTTON + (n - 1.0) * GAP + 2.0 * PAD,
        VIEW_BUTTON + 2.0 * PAD,
    );
    let panel = Rect::from_min_size(viewport.max - vec2(INSET, INSET) - size, size);
    let buttons = ViewPreset::ALL
        .into_iter()
        .enumerate()
        .map(|(k, preset)| {
            let x = panel.left() + PAD + k as f32 * (VIEW_BUTTON + GAP);
            let rect = Rect::from_min_size(pos2(x, panel.top() + PAD), Vec2::splat(VIEW_BUTTON));
            (preset, rect)
        })
        .collect();
    (panel, buttons)
}

/// The state the floating panels show, borrowed from the app for one frame.
pub struct ToolbarView<'a> {
    pub workspace_menu: &'a mut WorkspaceMenu,
    pub session: &'a Session,
    pub has_jaw_scan: bool,
    pub has_units: bool,
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

impl ToolbarView<'_> {
    /// Draws the toolbar and the view cluster over `viewport`; returns the command picked.
    pub fn show(self, ui: &mut Ui, viewport: Rect) -> Option<Command> {
        let mut command = None;
        let (panel, slots) = toolbar_layout(viewport);
        floating_panel(ui, panel, ui.scope_id().with("toolbar_panel"));
        let p = widgets::palette(ui);
        for pair in slots.windows(2) {
            let (left, right) = (pair[0].1, pair[1].1);
            if right.left() - left.right() > GAP {
                let x = (left.right() + right.left()) * 0.5;
                ui.painter().vline(
                    x,
                    left.top() + 5.0..=left.bottom() - 5.0,
                    Stroke::new(1.0, p.border),
                );
            }
        }
        for (slot, rect) in slots {
            match slot {
                Slot::Workspace => workspace_switcher(ui, rect, self.workspace_menu),
                Slot::New => {
                    if tool_button(ui, rect, Icon::NewDoc, true, false)
                        .on_hover_text("New case: start a new, empty case")
                        .clicked()
                    {
                        command = Some(Command::New);
                    }
                }
                Slot::Open => {
                    if tool_button(ui, rect, Icon::FolderOpen, true, false)
                        .on_hover_text("Open: a .dentcraft case and its scans")
                        .clicked()
                    {
                        command = Some(Command::Open);
                    }
                }
                Slot::Save => {
                    let tip = match &self.save_path {
                        Some(path) => format!("Save to {path}"),
                        None => "Save the case as a .dentcraft file".to_owned(),
                    };
                    if tool_button(ui, rect, Icon::Save, true, false)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        command = Some(Command::Save);
                    }
                }
                Slot::Import => {
                    if tool_button(ui, rect, Icon::Import, true, false)
                        .on_hover_text("Import scans: add STL, PLY or OBJ files to the case")
                        .clicked()
                    {
                        command = Some(Command::Import);
                    }
                }
                Slot::CaseSetup => {
                    if tool_button(ui, rect, Icon::CaseSetup, true, false)
                        .on_hover_text("Case setup: teeth and restorations, material and scans")
                        .clicked()
                    {
                        command = Some(Command::CaseSetup);
                    }
                }
                Slot::DesignMenu => {
                    let response = menu_button(ui, rect, "Design")
                        .on_hover_text("Workflow steps and design tools");
                    egui::Popup::menu(&response)
                        .align(RectAlign::BOTTOM_START)
                        .gap(4.0)
                        .show(|ui| self.design_menu(ui, &mut command));
                }
                Slot::ToolsMenu => {
                    let response = menu_button(ui, rect, "Tools")
                        .on_hover_text("Measuring, framing and scan visibility");
                    egui::Popup::menu(&response)
                        .align(RectAlign::BOTTOM_START)
                        .gap(4.0)
                        .show(|ui| self.tools_menu(ui, &mut command));
                }
            }
        }

        let (panel, buttons) = view_cluster_layout(viewport);
        floating_panel(ui, panel, ui.scope_id().with("view_cluster_panel"));
        for (n, (preset, rect)) in buttons.into_iter().enumerate() {
            let on = self.preset == Some(preset);
            if tool_button(ui, rect, Icon::View(preset), true, on)
                .on_hover_text(format!("{} view  ({})", preset.label(), n + 1))
                .clicked()
            {
                command = Some(Command::View(preset));
            }
        }
        command
    }

    fn design_menu(&self, ui: &mut Ui, command: &mut Option<Command>) {
        let session = self.session;
        let step = session.current();
        if menu_item(
            ui,
            Icon::Back,
            "Back",
            "",
            session.current_index() > 0,
            false,
        )
        .on_hover_text("Re-open the previous step")
        .clicked()
        {
            *command = Some(Command::Step(StepAction::Back));
        }
        let blocker =
            step.and_then(|s| workflow_panel::blocker(s.id, self.has_jaw_scan, self.has_units));
        let ready = step.is_some() && blocker.is_none();
        let next = menu_item(ui, Icon::Next, next_label(session), "", ready, false);
        let next = match step {
            Some(_) => next
                .on_hover_text("Complete this step")
                .on_disabled_hover_text(blocker.unwrap_or(workflow_panel::NEEDS_JAW_SCAN)),
            None => next.on_disabled_hover_text("Every step is done"),
        };
        if next.clicked() {
            *command = Some(Command::Step(StepAction::Next));
        }
        let optional = step.is_some_and(|s| s.id.is_optional());
        if menu_item(ui, Icon::Skip, "Skip", "", optional, false)
            .on_hover_text("Leave this step out")
            .on_disabled_hover_text("Only optional steps can be skipped")
            .clicked()
        {
            *command = Some(Command::Step(StepAction::Skip));
        }
        ui.separator();
        let planned = |id: StepId| session.steps().iter().any(|s| s.id == id);
        let tools = [
            (
                Command::Orient,
                Icon::Orient,
                "Orient",
                StepId::Orient,
                ORIENT,
                "Find the jaw's occlusal plane and midline",
                "The case has no Orientation step",
            ),
            (
                Command::Margin,
                Icon::Margin,
                "Margin",
                StepId::Margin,
                MARGIN,
                "Trace the margin line on the current tooth",
                "No crown, coping, inlay, onlay or veneer in the case",
            ),
            (
                Command::Axis,
                Icon::Axis,
                "Insertion axis",
                StepId::InsertionAxis,
                AXIS,
                "Set the insertion axis of the current tooth",
                "The case has no Insertion axis step",
            ),
            (
                Command::Inner,
                Icon::Inner,
                "Inner surface",
                StepId::InnerSurface,
                INNER,
                "Build the inner surface of the current tooth",
                "The case has no Inner surface step",
            ),
            (
                Command::Anatomy,
                Icon::Anatomy,
                "Anatomy",
                StepId::Anatomy,
                ANATOMY,
                "Place and shape the crown of the current tooth",
                "The case has no Anatomy step",
            ),
            (
                Command::Contacts,
                Icon::Contacts,
                "Contacts",
                StepId::Occlusion,
                CONTACTS,
                "Adapt the crown to its neighbours and the antagonist",
                "The case has no Occlusion & contacts step",
            ),
        ];
        for (cmd, icon, label, id, tool, tip, missing) in tools {
            let on = self.active_tool == Some(tool);
            if menu_item(ui, icon, label, "Esc leaves", planned(id), on)
                .on_hover_text(tip)
                .on_disabled_hover_text(missing)
                .clicked()
            {
                *command = Some(cmd);
            }
        }
        let on = step.is_some_and(|s| s.id == StepId::Finish);
        if menu_item(ui, Icon::Export, "Export STL", "", true, on)
            .on_hover_text("Write the finished crowns as STL files")
            .clicked()
        {
            *command = Some(Command::Export);
        }
    }

    fn tools_menu(&self, ui: &mut Ui, command: &mut Option<Command>) {
        let on = self.active_tool == Some(MEASURE);
        if menu_item(ui, Icon::Ruler, "Measure", "Esc", true, on)
            .on_hover_text("Measure between two points on the scans  (Esc or Delete clears)")
            .clicked()
        {
            *command = Some(Command::Measure);
        }
        ui.separator();
        if menu_item(ui, Icon::Fit, "Fit all", "F", true, false)
            .on_hover_text("Fit the scene in view  (F outside the tooth steps, or double-click)")
            .clicked()
        {
            *command = Some(Command::Fit);
        }
        let on_tooth = self
            .session
            .current()
            .is_some_and(|s| crate::tooth_view::is_tooth_step(s.id));
        if menu_item(ui, Icon::FitTooth, "Fit tooth", "F", on_tooth, false)
            .on_hover_text("Frame the tooth being designed  (F)")
            .on_disabled_hover_text("Available in the per-tooth steps")
            .clicked()
        {
            *command = Some(Command::FitTooth);
        }
        ui.separator();
        for (role, jaw, icon, visible) in [
            (ScanRole::Upper, "upper", Icon::UpperJaw, self.upper_visible),
            (ScanRole::Lower, "lower", Icon::LowerJaw, self.lower_visible),
        ] {
            let on = visible.unwrap_or(false);
            if menu_item(
                ui,
                icon,
                &format!("Show {jaw} jaw"),
                "",
                visible.is_some(),
                on,
            )
            .on_hover_text(format!("Show or hide the {jaw} jaw scans"))
            .on_disabled_hover_text(format!("No {jaw} jaw scan in the case"))
            .clicked()
            {
                *command = Some(Command::ShowRole(role, !on));
            }
        }
    }
}

/// Background of a floating panel: translucent theme fill, rounded, a light shadow. It takes the
/// pointer inside `rect`, so a click between its buttons does not reach the viewport below.
fn floating_panel(ui: &Ui, rect: Rect, id: Id) {
    let p = widgets::palette(ui);
    ui.interact(rect, id, Sense::click_and_drag());
    let painter = ui.painter();
    let mut shadow = ui.visuals().popup_shadow;
    shadow.color = shadow.color.gamma_multiply(0.7);
    painter.add(shadow.as_shape(rect, CornerRadius::same(RADIUS_LG)));
    painter.rect(
        rect,
        CornerRadius::same(RADIUS_LG),
        p.floating(),
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
}

/// Square icon button at `rect`.
fn tool_button(ui: &mut Ui, rect: Rect, icon: Icon, enabled: bool, selected: bool) -> Response {
    let p = widgets::palette(ui);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, ui.scope_id().with(("tool_button", icon)), sense);
    if ui.is_rect_visible(rect) {
        let (fill, ink) = widgets::button_colors(&p, &response, enabled, selected);
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
        paint_icon(painter, icon, rect.shrink(rect.width() * 0.17), ink);
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// Text button with a dropdown arrow at `rect`, which opens a menu.
fn menu_button(ui: &mut Ui, rect: Rect, label: &str) -> Response {
    let p = widgets::palette(ui);
    let response = ui.interact(
        rect,
        ui.scope_id().with(("menu_button", label)),
        Sense::click(),
    );
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let (fill, ink) = widgets::button_colors(&p, &response, true, open);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
    let galley = painter.layout_no_wrap(label.to_owned(), FontId::proportional(SIZE_BODY), ink);
    let text_pos = pos2(rect.left() + 9.0, rect.center().y - galley.size().y * 0.5);
    painter.galley(text_pos, galley, ink);
    let arrow = Rect::from_center_size(
        pos2(rect.right() - 11.0, rect.center().y),
        Vec2::splat(12.0),
    );
    paint_icon(painter, Icon::ChevronDown, arrow, ink);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A row of a toolbar menu: icon, label and, on the right, its shortcut. A disabled row shows
/// its disabled hover text and cannot be clicked.
fn menu_item(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    shortcut: &str,
    enabled: bool,
    selected: bool,
) -> Response {
    let p = widgets::palette(ui);
    ui.scope(|ui| {
        if !enabled {
            ui.disable();
        }
        let (rect, response) = ui.allocate_exact_size(vec2(MENU_ROW_WIDTH, 26.0), Sense::click());
        if ui.is_rect_visible(rect) {
            let (fill, ink) = widgets::button_colors(&p, &response, enabled, selected);
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
            let icon_rect = Rect::from_center_size(
                pos2(rect.left() + 15.0, rect.center().y),
                Vec2::splat(18.0),
            );
            paint_icon(painter, icon, icon_rect, ink);
            painter.text(
                pos2(rect.left() + 32.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(SIZE_BODY),
                ink,
            );
            painter.text(
                pos2(rect.right() - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                shortcut,
                FontId::proportional(SIZE_SMALL),
                p.text_dim,
            );
        }
        if enabled {
            response.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            response
        }
    })
    .inner
}

/// The workspace switcher at the left end of the toolbar: the current workspace's icon and a
/// dropdown arrow; it opens the workspace menu below it. Adapted from SolidCraft's workspace
/// switcher.
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
    let (fill, ink) = widgets::button_colors(&p, &response, true, menu.open);
    painter.rect_filled(block, CornerRadius::same(RADIUS_SM), fill);
    let icon = Rect::from_center_size(
        pos2(block.left() + BUTTON * 0.5, block.center().y),
        Vec2::splat(BUTTON - 8.0),
    );
    paint_icon(painter, Icon::Workspace(current), icon, ink);
    let arrow = pos2(block.right() - 8.0, block.center().y);
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
        .on_hover_text(format!("Workspace: {}. Change workspace", current.title()))
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
            .gap(PAD + 2.0)
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
    paint_icon(painter, Icon::Workspace(w), tile.shrink(7.0), icon_ink);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::gizmo_rect;
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

    /// Viewport of the default 1600 × 1000 window and of the smallest window (960 × 600 less the
    /// workflow panel at its minimum width).
    fn viewports() -> [Rect; 2] {
        [
            Rect::from_min_size(pos2(0.0, 32.0), vec2(1300.0, 946.0)),
            Rect::from_min_size(pos2(0.0, 32.0), vec2(680.0, 546.0)),
        ]
    }

    #[test]
    fn the_panels_stay_inside_the_viewport_apart_from_each_other_and_the_gizmo() {
        for viewport in viewports() {
            let (toolbar, slots) = toolbar_layout(viewport);
            let (cluster, buttons) = view_cluster_layout(viewport);
            let gizmo = gizmo_rect(viewport);
            for panel in [toolbar, cluster] {
                assert!(viewport.contains_rect(panel), "{panel:?} in {viewport:?}");
                assert!(!panel.intersects(gizmo), "{panel:?} on the gizmo {gizmo:?}");
            }
            assert!(!toolbar.intersects(cluster));
            assert_eq!(toolbar.left_top(), viewport.left_top() + Vec2::splat(INSET));
            assert_eq!(
                cluster.right_bottom(),
                viewport.right_bottom() - Vec2::splat(INSET)
            );
            // Every button lies inside its panel, and none overlaps the next.
            let rects: Vec<Rect> = slots.iter().map(|s| s.1).collect();
            let views: Vec<Rect> = buttons.iter().map(|b| b.1).collect();
            for (panel, rects) in [(toolbar, rects), (cluster, views)] {
                for r in &rects {
                    assert!(panel.shrink(PAD - 0.01).contains_rect(*r));
                }
                for pair in rects.windows(2) {
                    assert!(pair[0].right() < pair[1].left());
                }
            }
        }
    }

    #[test]
    fn the_view_cluster_has_a_button_per_view() {
        let (_, buttons) = view_cluster_layout(viewports()[0]);
        let presets: Vec<ViewPreset> = buttons.iter().map(|b| b.0).collect();
        assert_eq!(presets, ViewPreset::ALL);
    }

    /// What a click did: the toolbar's command, and whether the viewport below got the click.
    struct Clicked {
        command: Option<Command>,
        viewport: bool,
    }

    /// Clicks the point `target` picks in the viewport in a headless frame sequence, with the
    /// viewport's own click area below the panels as in the app.
    fn click_at(target: impl Fn(Rect) -> Pos2) -> Clicked {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0));
        let viewport = viewports()[0];
        let pos = target(viewport);
        let session = Session::for_case(&Case::new("t"));
        let mut menu = WorkspaceMenu::default();
        let mut picked = None;
        let mut viewport_clicked = false;
        let events = [
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        ];
        for events in events {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                // The viewport below the panels, as in the app.
                viewport_clicked |= ui
                    .interact(
                        viewport,
                        ui.scope_id().with("viewport"),
                        Sense::click_and_drag(),
                    )
                    .clicked();
                let command = ToolbarView {
                    workspace_menu: &mut menu,
                    session: &session,
                    has_jaw_scan: false,
                    has_units: false,
                    save_path: None,
                    preset: None,
                    upper_visible: None,
                    lower_visible: None,
                    active_tool: None,
                }
                .show(ui, viewport);
                picked = picked.or(command);
            });
            // No renderer takes the icon textures here.
            output.textures_delta.clear();
        }
        Clicked {
            command: picked,
            viewport: viewport_clicked,
        }
    }

    fn click(target: impl Fn(Rect) -> Pos2) -> Option<Command> {
        let clicked = click_at(target);
        assert!(!clicked.viewport, "a click on a panel reached the viewport");
        clicked.command
    }

    fn slot_centre(slot: Slot) -> impl Fn(Rect) -> Pos2 {
        move |viewport| {
            let (_, slots) = toolbar_layout(viewport);
            slots.iter().find(|s| s.0 == slot).unwrap().1.center()
        }
    }

    #[test]
    fn the_toolbar_buttons_send_their_file_commands() {
        assert_eq!(click(slot_centre(Slot::New)), Some(Command::New));
        assert_eq!(click(slot_centre(Slot::Open)), Some(Command::Open));
        assert_eq!(click(slot_centre(Slot::Save)), Some(Command::Save));
        assert_eq!(click(slot_centre(Slot::Import)), Some(Command::Import));
        assert_eq!(
            click(slot_centre(Slot::CaseSetup)),
            Some(Command::CaseSetup)
        );
    }

    #[test]
    fn a_view_button_sends_its_view() {
        let lingual = |viewport| {
            let (_, buttons) = view_cluster_layout(viewport);
            buttons[2].1.center()
        };
        assert_eq!(click(lingual), Some(Command::View(ViewPreset::Lingual)));
    }

    #[test]
    fn a_click_on_the_panel_between_buttons_or_on_the_viewport_sends_nothing() {
        let gap = |viewport| {
            let (_, slots) = toolbar_layout(viewport);
            pos2(
                (slots[1].1.right() + slots[2].1.left()) * 0.5,
                slots[1].1.center().y,
            )
        };
        assert_eq!(click(gap), None);
        let centre = click_at(|viewport: Rect| viewport.center());
        assert_eq!(centre.command, None);
        assert!(
            centre.viewport,
            "a click outside the panels must reach the viewport"
        );
    }
}
