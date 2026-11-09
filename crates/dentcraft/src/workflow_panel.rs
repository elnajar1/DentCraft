//! The floating current-step wizard, with scrolling parameters and fixed navigation.

use dentcraft_case::{Case, Fdi};
use dentcraft_workflow::{Session, StepId, StepStatus};
use eframe::egui::{
    self, Align, CornerRadius, CursorIcon, FontId, Layout, Margin, Rect, RichText, Sense, Stroke,
    Ui, Vec2, vec2,
};

use crate::anatomy_step::{AnatomyParam, FINE_NUDGE_MM, NUDGE_MM};
use crate::app::group_thousands;
use crate::chart::{ON_FILL, kind_color};
use crate::contacts_step::{
    ContactParam, ContactsCard, MAP_MAX_MM, MAP_MIN_MM, NO_ANTAGONIST, THIN_WARNING, distance_color,
};
use crate::export_step::{ExportCard, format_bytes};
use crate::inner_step::InnerParam;
use crate::theme::{Palette, RADIUS_SM, SIZE_SMALL};
use crate::widgets::{self, Icon, paint_icon, small_button};

/// Shown on every step whose design tools are not built yet.
pub const NOT_YET_NOTE: &str = "Design tools for this step arrive in DentCraft 0.2.";
/// Why the Scans step cannot be completed yet.
pub const NEEDS_RESTORATION: &str = "Add a restoration in the tooth chart to continue.";
pub const NEEDS_JAW_SCAN: &str = "Import an upper or lower jaw scan to continue.";

/// What the user asked the session to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepAction {
    /// Copy current step parameters, with one undo, to its other or selected teeth.
    ApplyParameters(crate::step_parameters::Scope),
    Next,
    Back,
    Skip,
    GoTo(usize),
    /// Work on this tooth of step `usize` (a chip click).
    Tooth(usize, Fdi),
    /// Propose the margin again from the last click.
    DetectAgain,
    /// Remove the current tooth's margin.
    ClearMargin,
    /// Trace the current tooth's margin by hand, point by point.
    TraceByHand,
    /// Store the hand-traced margin.
    FinishTrace,
    /// Drop the hand-traced margin in progress.
    CancelTrace,
    /// Find the jaw's occlusal plane and midline.
    AutoOrient,
    /// Remove the jaw's orientation.
    ResetOrient,
    /// Search the current tooth's best insertion axis again.
    BestAxis,
    /// Set the current tooth's insertion axis to the reversed view direction.
    AxisFromView,
    /// Set an inner-surface parameter of the current tooth; `true` while its value is dragged.
    SetInner(InnerParam, f64, bool),
    /// A drag of an inner-surface parameter ended.
    EndInnerDrag,
    /// Remove the current tooth's inner-surface overrides.
    ResetInner,
    /// Turn the section view on or off.
    ToggleSection,
    /// Set an anatomy slider of the current tooth (shown units); `true` while it is dragged.
    SetAnatomy(AnatomyParam, f64, bool),
    /// A drag of an anatomy slider ended.
    EndAnatomyDrag,
    /// Move the current tooth's crown along offset axis `usize` (mesial, facial, occlusal) by
    /// the millimetres given.
    NudgeAnatomy(usize, f64),
    /// Reset position and rotation while retaining the selected model and shape.
    ResetAnatomyPlacement,
    /// Put the current tooth's crown back to the library shape and automatic placement.
    ResetAnatomy,
    ChooseLibraryTooth(Fdi),
    /// Set a contact target of the current tooth in millimetres; `true` while it is dragged.
    SetContact(ContactParam, f64, bool),
    /// A drag of a contact slider ended.
    EndContactDrag,
    /// Adapt the current tooth's crown to its neighbours and antagonist.
    AdaptContacts,
    MapReference(crate::contacts_step::MapReference),
    /// Simplify the crowns before writing them (or not).
    SetSimplify(bool),
    /// Pick a folder and write every ready crown into it as STL.
    ExportStl,
    /// Open the last export's folder in the system's file browser.
    OpenExportFolder,
}

/// What the Anatomy step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct AnatomyCard {
    pub model: Fdi,
    pub models: Vec<(Fdi, bool)>,
    pub tooth: Fdi,
    pub pontic: bool,
    pub has_margin: bool,
    pub has_axis: bool,
    /// The tooth's inner surface could not be computed.
    pub inner_failed: bool,
    /// Slider values in [`AnatomyParam::ALL`] order, in shown units.
    pub values: [f64; 6],
    /// Offset along mesial, facial and occlusal, in millimetres.
    pub offset: [f64; 3],
    /// The anatomy differs from the library shape and automatic placement.
    pub adjusted: bool,
    /// Minimum thickness (mm) and thickened area (mm²) of the shown crown.
    pub report: Option<(f64, f64)>,
    /// What the step is busy with.
    pub busy: Option<&'static str>,
    /// Where the crowns come from: the tooth library's folder name or "DentCraft parametric".
    pub library: String,
    /// The tooth library could not be loaded.
    pub library_error: Option<String>,
    /// The tooth's crown uses the parametric tooth although the library serves it, and why.
    pub library_note: Option<String>,
    pub error: Option<String>,
}

/// Shown for a pontic in the Anatomy step.
pub const PONTIC_NOTE: &str = "Pontics arrive with bridges";

/// What the Inner surface step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct InnerCard {
    pub tooth: Fdi,
    pub has_margin: bool,
    pub has_axis: bool,
    /// Parameter values in [`InnerParam::ALL`] order, in millimetres.
    pub values: [f64; 4],
    /// The tooth overrides the material.
    pub overridden: bool,
    pub section: bool,
    /// What the step is busy with.
    pub busy: Option<&'static str>,
    pub error: Option<String>,
}

/// Shown for a tooth without an insertion axis in the Inner surface step.
pub const INNER_NEEDS_AXIS: &str = "Set the insertion axis first";

/// What the Insertion axis step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisCard {
    pub tooth: Fdi,
    pub has_margin: bool,
    /// Angle between the axis and the jaw's occlusal direction.
    pub tilt_deg: Option<f64>,
    /// Undercut area and minimum draft of the shown axis.
    pub undercut_mm2: Option<f64>,
    pub min_draft_deg: Option<f64>,
    /// What the step is busy with.
    pub busy: Option<&'static str>,
    pub error: Option<String>,
}

/// Shown for a tooth without a margin in the Insertion axis step.
pub const AXIS_NEEDS_MARGIN: &str = "Trace the margin first";

/// What the Orientation step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct OrientCard {
    /// What the step is busy with.
    pub busy: Option<&'static str>,
    pub error: Option<String>,
    /// A gentle note, such as an estimated front of the arch.
    pub note: Option<String>,
    /// The case has an orientation.
    pub has_frame: bool,
}

/// What the Margin step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct MarginCard {
    pub tooth: Fdi,
    /// Length of the margin, once there is one.
    pub length_mm: Option<f64>,
    /// What the step is busy with.
    pub busy: Option<&'static str>,
    pub error: Option<String>,
    pub can_detect_again: bool,
    /// Waypoints placed so far, while the margin is traced by hand.
    pub waypoints: Option<usize>,
    /// A hand-traced margin is ready to be stored.
    pub can_finish: bool,
}

/// Hint shown while the margin is traced by hand.
pub const MANUAL_HINT: &str = "Click 4\u{2013}8 points along the finish line around the tooth. The line follows the edge between them.";

/// Hint shown until the tooth has a margin.
pub const MARGIN_HINT: &str = "Click the top of the preparation";

/// Call to action shown while the case has no restoration.
pub const ADD_RESTORATION: &str = "Add a restoration in Case setup from the toolbar.";

/// Why the active step cannot be completed yet: the Scans step needs a jaw scan and a
/// restoration to design.
pub fn blocker(step: StepId, has_jaw_scan: bool, has_units: bool) -> Option<&'static str> {
    if step != StepId::Scans {
        None
    } else if !has_jaw_scan {
        Some(NEEDS_JAW_SCAN)
    } else if !has_units {
        Some(NEEDS_RESTORATION)
    } else {
        None
    }
}

pub struct WorkflowView<'a> {
    pub session: &'a Session,
    pub case: &'a Case,
    pub has_jaw_scan: bool,
    pub error: Option<&'a str>,
    /// Tooth the per-tooth step works on.
    pub tooth: Option<Fdi>,
    /// The Margin step's card, when that step is active.
    pub margin: Option<MarginCard>,
    /// The Orientation step's card, when that step is active.
    pub orient: Option<OrientCard>,
    /// The Insertion axis step's card, when that step is active.
    pub axis: Option<AxisCard>,
    /// The Inner surface step's card, when that step is active.
    pub inner: Option<InnerCard>,
    /// The Anatomy step's card, when that step is active.
    pub anatomy: Option<AnatomyCard>,
    /// The Occlusion & contacts step's card, when that step is active.
    pub contacts: Option<ContactsCard>,
    /// The Finish & export step's card, when that step is active.
    pub export: Option<ExportCard>,
}

/// Fixed rectangles keep scrolling content from moving navigation or leaking clicks.
pub fn wizard_rect(viewport: Rect) -> Rect {
    let top = viewport.top() + crate::toolbar::INSET + crate::toolbar::HEIGHT + 10.0;
    Rect::from_min_max(
        egui::pos2(viewport.right() - 320.0, top),
        egui::pos2(viewport.right() - 10.0, viewport.bottom() - 64.0),
    )
}

impl WorkflowView<'_> {
    pub fn show(self, ui: &mut Ui, viewport: Rect) -> Option<StepAction> {
        let panel = wizard_rect(viewport);
        crate::toolbar::floating_panel(ui, panel, ui.scope_id().with("wizard_panel"));
        let p = widgets::palette(ui);
        let content = panel.shrink(12.0);
        let mut action = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(content);
            let (done, total) = self.session.progress();
            let label = if self.session.is_finished() {
                format!("{total} of {total} steps complete")
            } else {
                format!("Step {} of {total}", self.session.current_index() + 1)
            };
            ui.label(RichText::new(label).size(SIZE_SMALL).color(p.text_dim));
            progress_bar(ui, done as f32 / total.max(1) as f32, &p);
            ui.add_space(8.0);
            let body_height = (content.bottom() - 42.0 - ui.cursor().top()).max(1.0);
            egui::ScrollArea::vertical()
                .id_salt("wizard_body")
                .max_height(body_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(step) = self.session.current() {
                        action = active_card(
                            ui,
                            &self,
                            self.session.current_index(),
                            step.id,
                            &step.teeth,
                            &p,
                        );
                    } else {
                        finished_card(ui, total, &p);
                    }
                });
        });
        let footer = Rect::from_min_max(
            egui::pos2(content.left(), content.bottom() - 32.0),
            content.max,
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(footer), |ui| {
            ui.set_clip_rect(footer);
            if let Some(a) = navigation(
                ui,
                self.session,
                self.has_jaw_scan,
                !self.case.units.is_empty(),
            ) {
                action = Some(a);
            }
        });
        action
    }
}

/// Shared by the wizard and context menu so both obey the same workflow gates.
pub fn navigation(
    ui: &mut Ui,
    session: &Session,
    has_jaw_scan: bool,
    has_units: bool,
) -> Option<StepAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if small_button(ui, "Back", session.current_index() > 0, false).clicked() {
            action = Some(StepAction::Back);
        }
        let step = session.current();
        if step.is_some_and(|s| s.id.is_optional())
            && small_button(ui, "Skip", true, false).clicked()
        {
            action = Some(StepAction::Skip);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let why = step.and_then(|s| blocker(s.id, has_jaw_scan, has_units));
            if small_button(
                ui,
                crate::toolbar::next_label(session),
                step.is_some() && why.is_none(),
                true,
            )
            .on_disabled_hover_text(why.unwrap_or("Every step is done"))
            .clicked()
            {
                action = Some(StepAction::Next);
            }
        });
    });
    action
}

pub fn reachable(session: &Session, index: usize) -> bool {
    session.status(index) == StepStatus::Active || session.clone().go_to(index).is_ok()
}

fn progress_bar(ui: &mut Ui, fraction: f32, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let radius = CornerRadius::same(3);
    ui.painter().rect_filled(rect, radius, p.border);
    if fraction > 0.0 {
        let mut fill = rect;
        fill.set_width((rect.width() * fraction.clamp(0.0, 1.0)).max(6.0));
        ui.painter().rect_filled(fill, radius, p.accent);
    }
}

/// How a tooth chip is drawn.
#[derive(Clone, Copy, Debug, Default)]
struct ChipMark {
    /// The tooth the active step works on.
    current: bool,
    /// The step's work on this tooth is done.
    done: bool,
    /// A click picks the tooth.
    clickable: bool,
}

/// Whether the step has per-tooth tools, so its chips pick the tooth to work on.
fn has_tooth_tools(id: StepId) -> bool {
    matches!(
        id,
        StepId::Margin
            | StepId::InsertionAxis
            | StepId::InnerSurface
            | StepId::Anatomy
            | StepId::Occlusion
    )
}

fn chip_mark(
    id: StepId,
    status: StepStatus,
    tooth: Fdi,
    case: &Case,
    current: Option<Fdi>,
) -> ChipMark {
    let tools = has_tooth_tools(id);
    ChipMark {
        current: tools && status == StepStatus::Active && current == Some(tooth),
        done: case.design.teeth.get(&tooth).is_some_and(|d| match id {
            StepId::Margin => d.margin.is_some(),
            StepId::InsertionAxis => d.insertion_axis.is_some(),
            _ => false,
        }),
        clickable: tools && status != StepStatus::Pending,
    }
}

fn chip_font() -> FontId {
    FontId::proportional(SIZE_SMALL - 1.0)
}

/// Room for the tick of a done chip.
const CHIP_TICK: f32 = 10.0;

/// A small chip with a tooth number, tinted with its restoration colour, ringed when it is the
/// current tooth and ticked when its work is done. Returns true when a clickable chip was clicked.
fn tooth_chip(ui: &mut Ui, tooth: Fdi, case: &Case, mark: ChipMark, p: &Palette) -> bool {
    // Solid restoration fills with dark numbers read at 4.5:1 or better in both themes.
    let (tint, ink) = case
        .unit(tooth)
        .map_or((p.panel_header, p.text), |u| (kind_color(u.kind), ON_FILL));
    let galley = ui
        .painter()
        .layout_no_wrap(tooth.to_string(), chip_font(), ink);
    let tick = if mark.done { CHIP_TICK } else { 0.0 };
    let size = vec2(galley.size().x + 10.0 + tick, 16.0);
    let sense = if mark.clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, mut response) = ui.allocate_exact_size(size, sense);
    let radius = CornerRadius::same(RADIUS_SM);
    ui.painter().rect_filled(rect, radius, tint);
    if mark.current {
        ui.painter().rect_stroke(
            rect.expand(1.5),
            radius,
            Stroke::new(1.5, p.accent),
            egui::StrokeKind::Outside,
        );
    }
    let text_at = rect.center() - galley.size() * 0.5 - vec2(tick * 0.5, 0.0);
    ui.painter().galley(text_at, galley, ink);
    if mark.done {
        let at = Rect::from_center_size(
            egui::pos2(rect.right() - 3.0 - CHIP_TICK * 0.5, rect.center().y),
            Vec2::splat(9.0),
        );
        paint_icon(ui.painter(), Icon::Check, at, ink);
    }
    if let Some(unit) = case.unit(tooth) {
        let done = if mark.done { " · margin done" } else { "" };
        response = response.on_hover_text(format!("{tooth} · {}{done}", unit.kind.label()));
    }
    mark.clickable && response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

fn active_card(
    ui: &mut Ui,
    view: &WorkflowView<'_>,
    index: usize,
    id: StepId,
    teeth: &[Fdi],
    p: &Palette,
) -> Option<StepAction> {
    let session = view.session;
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(id.title()).heading().color(p.text));
        if id.is_optional() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("Optional").size(SIZE_SMALL).color(p.text_dim));
            });
        }
    });
    ui.label(RichText::new(id.summary()).color(p.text_dim));
    if !teeth.is_empty() {
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.label(RichText::new("Teeth").size(SIZE_SMALL).color(p.text_dim));
            ui.add_space(2.0);
            for &tooth in teeth {
                let mark = chip_mark(id, StepStatus::Active, tooth, view.case, view.tooth);
                if tooth_chip(ui, tooth, view.case, mark, p) {
                    action = Some(StepAction::Tooth(index, tooth));
                }
            }
        });
    }
    ui.add_space(4.0);
    let has_units = !view.case.units.is_empty();
    if !has_units {
        ui.label(RichText::new(ADD_RESTORATION).color(p.accent).strong());
        ui.add_space(4.0);
    }
    if let Some(card) = view.margin.as_ref().filter(|_| id == StepId::Margin) {
        if let Some(a) = margin_section(ui, card, view.case, p) {
            action = Some(a);
        }
    } else if let Some(card) = view.orient.as_ref().filter(|_| id == StepId::Orient) {
        if let Some(a) = orient_section(ui, card, p) {
            action = Some(a);
        }
    } else if let Some(card) = view.axis.as_ref().filter(|_| id == StepId::InsertionAxis) {
        let margin_step = session
            .steps()
            .iter()
            .position(|s| s.id == StepId::Margin && s.teeth.contains(&card.tooth));
        if let Some(a) = axis_section(ui, card, margin_step, p) {
            action = Some(a);
        }
    } else if let Some(card) = view.inner.as_ref().filter(|_| id == StepId::InnerSurface) {
        let step = |want: StepId| {
            session
                .steps()
                .iter()
                .position(|s| s.id == want && s.teeth.contains(&card.tooth))
        };
        let (margin_step, axis_step) = (step(StepId::Margin), step(StepId::InsertionAxis));
        if let Some(a) = inner_section(ui, card, margin_step, axis_step, p) {
            action = Some(a);
        }
    } else if let Some(card) = view.anatomy.as_ref().filter(|_| id == StepId::Anatomy) {
        let step = |want: StepId| {
            session
                .steps()
                .iter()
                .position(|s| s.id == want && s.teeth.contains(&card.tooth))
        };
        let steps = [
            step(StepId::Margin),
            step(StepId::InsertionAxis),
            step(StepId::InnerSurface),
        ];
        if let Some(a) = anatomy_section(ui, card, steps, p) {
            action = Some(a);
        }
    } else if let Some(card) = view.contacts.as_ref().filter(|_| id == StepId::Occlusion) {
        let anatomy_step = session
            .steps()
            .iter()
            .position(|s| s.id == StepId::Anatomy && s.teeth.contains(&card.tooth));
        if let Some(a) = contacts_section(ui, card, anatomy_step, p) {
            action = Some(a);
        }
    } else if let Some(card) = view.export.as_ref().filter(|_| id == StepId::Finish) {
        if let Some(a) = export_section(ui, card, p) {
            action = Some(a);
        }
    } else {
        let note = if id == StepId::Scans {
            if view.has_jaw_scan {
                "Jaw scan loaded. Check each scan's role in Case setup (toolbar)."
            } else {
                NEEDS_JAW_SCAN
            }
        } else {
            NOT_YET_NOTE
        };
        note_box(ui, note, p);
    }
    let targets = crate::step_parameters::targets(view.case, session, view.tooth, None);
    if !targets.is_empty() {
        ui.add_space(6.0);
        if small_button(
            ui,
            &format!("Apply to {} other teeth", targets.len()),
            true,
            false,
        )
        .on_hover_text(format!(
                "Copy {} from the current tooth, then review changed teeth with Next. Undo reverts the whole group.",
            crate::step_parameters::label(id).unwrap_or("settings")
        ))
        .clicked()
        {
            action = Some(StepAction::ApplyParameters(
                crate::step_parameters::Scope::Step,
            ));
        }
    }
    if let Some(error) = view.error {
        ui.label(
            RichText::new(error)
                .size(SIZE_SMALL)
                .color(p.text_dim)
                .italics(),
        );
    }
    action
}

/// The Margin step's tooth, hint, length, busy state, error and commands.
fn margin_section(ui: &mut Ui, card: &MarginCard, case: &Case, p: &Palette) -> Option<StepAction> {
    let mut action = None;
    let has_margin = case
        .design
        .teeth
        .get(&card.tooth)
        .is_some_and(|d| d.margin.is_some());
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Tooth {}", card.tooth))
                        .strong()
                        .color(p.text),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(busy) = card.busy {
                        ui.label(RichText::new(busy).size(SIZE_SMALL).color(p.text_dim));
                        ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    } else if let Some(length) = card.length_mm {
                        ui.label(
                            RichText::new(format!("{} mm", widgets::fmt_mm(length, 2)))
                                .color(p.text)
                                .strong(),
                        );
                        ui.label(RichText::new("Length").size(SIZE_SMALL).color(p.text_dim));
                    }
                });
            });
            let hint = if card.waypoints.is_some() {
                MANUAL_HINT
            } else if has_margin {
                "Drag a point along the surface. Click the line to add a point; Delete removes one."
            } else {
                MARGIN_HINT
            };
            ui.label(RichText::new(hint).size(SIZE_SMALL).color(p.text_dim));
            if let Some(error) = &card.error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
            }
            if let Some(count) = card.waypoints {
                let noun = if count == 1 { "waypoint" } else { "waypoints" };
                ui.label(
                    RichText::new(format!("{count} {noun}"))
                        .size(SIZE_SMALL)
                        .color(p.text),
                );
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if card.waypoints.is_some() {
                    if small_button(ui, "Done", card.can_finish, true)
                        .on_hover_text("Store the traced margin (Enter)")
                        .clicked()
                    {
                        action = Some(StepAction::FinishTrace);
                    }
                    if small_button(ui, "Cancel", true, false)
                        .on_hover_text("Drop the trace (Escape)")
                        .clicked()
                    {
                        action = Some(StepAction::CancelTrace);
                    }
                } else if small_button(ui, "Trace by hand", true, false)
                    .on_hover_text("Click points along the finish line yourself")
                    .clicked()
                {
                    action = Some(StepAction::TraceByHand);
                }
                if small_button(ui, "Detect again", card.can_detect_again, false)
                    .on_hover_text("Propose the margin again from the last click")
                    .on_disabled_hover_text(MARGIN_HINT)
                    .clicked()
                {
                    action = Some(StepAction::DetectAgain);
                }
                if small_button(ui, "Clear", has_margin, false)
                    .on_hover_text("Remove this tooth's margin")
                    .clicked()
                {
                    action = Some(StepAction::ClearMargin);
                }
            });
        });
    action
}

/// The Insertion axis step's tooth, values, undercut legend, busy state, error and commands.
fn axis_section(
    ui: &mut Ui,
    card: &AxisCard,
    margin_step: Option<usize>,
    p: &Palette,
) -> Option<StepAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Tooth {}", card.tooth))
                        .strong()
                        .color(p.text),
                );
                if let Some(busy) = card.busy {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(busy).size(SIZE_SMALL).color(p.text_dim));
                        ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    });
                }
            });
            if !card.has_margin {
                ui.label(
                    RichText::new(AXIS_NEEDS_MARGIN)
                        .size(SIZE_SMALL)
                        .color(p.text_dim),
                );
                if let Some(index) = margin_step
                    && small_button(ui, "Margin step", true, true)
                        .on_hover_text("Open the Margin step on this tooth")
                        .clicked()
                {
                    action = Some(StepAction::Tooth(index, card.tooth));
                }
                return;
            }
            let value = |ui: &mut Ui, name: &str, text: Option<String>| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).size(SIZE_SMALL).color(p.text_dim));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let text = text.unwrap_or_else(|| "\u{2013}".to_owned());
                        ui.label(RichText::new(text).color(p.text).strong());
                    });
                });
            };
            let degrees = |d: f64| format!("{}\u{b0}", widgets::fmt_mm(d, 1));
            value(ui, "Tilt from occlusal", card.tilt_deg.map(degrees));
            value(
                ui,
                "Undercut area",
                card.undercut_mm2
                    .map(|a| format!("{} mm\u{b2}", widgets::fmt_mm(a, 2))),
            );
            value(ui, "Min draft", card.min_draft_deg.map(degrees));
            undercut_legend(ui, p);
            ui.label(
                RichText::new("Drag the arrow tip to tilt the axis.")
                    .size(SIZE_SMALL)
                    .color(p.text_dim),
            );
            if let Some(error) = &card.error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if small_button(ui, "Best axis", card.busy.is_none(), true)
                    .on_hover_text("Find the axis with the least undercut and the most draft")
                    .clicked()
                {
                    action = Some(StepAction::BestAxis);
                }
                if small_button(ui, "From view", true, false)
                    .on_hover_text("Insert along the direction you are looking from")
                    .clicked()
                {
                    action = Some(StepAction::AxisFromView);
                }
            });
        });
    action
}

/// The Inner surface step's tooth, parameters, section toggle, busy state, error and commands.
fn inner_section(
    ui: &mut Ui,
    card: &InnerCard,
    margin_step: Option<usize>,
    axis_step: Option<usize>,
    p: &Palette,
) -> Option<StepAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Tooth {}", card.tooth))
                        .strong()
                        .color(p.text),
                );
                if let Some(busy) = card.busy {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(busy).size(SIZE_SMALL).color(p.text_dim));
                        ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    });
                }
            });
            let missing = if !card.has_margin {
                Some((AXIS_NEEDS_MARGIN, "Margin step", margin_step))
            } else if !card.has_axis {
                Some((INNER_NEEDS_AXIS, "Insertion axis step", axis_step))
            } else {
                None
            };
            if let Some((hint, button, index)) = missing {
                ui.label(RichText::new(hint).size(SIZE_SMALL).color(p.text_dim));
                if let Some(index) = index
                    && small_button(ui, button, true, true).clicked()
                {
                    action = Some(StepAction::Tooth(index, card.tooth));
                }
                return;
            }
            for (param, &value) in InnerParam::ALL.into_iter().zip(&card.values) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(param.name())
                            .size(SIZE_SMALL)
                            .color(p.text_dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut v = value;
                        let step = 10f64.powi(-(param.decimals() as i32));
                        let r = ui.add(
                            egui::DragValue::new(&mut v)
                                .range(param.range())
                                .speed(step)
                                .fixed_decimals(param.decimals())
                                .suffix(" mm"),
                        );
                        if r.changed() {
                            action = Some(StepAction::SetInner(param, v, r.dragged()));
                        } else if r.drag_stopped() {
                            action = Some(StepAction::EndInnerDrag);
                        }
                    });
                });
            }
            if let Some(error) = &card.error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
                if let Some(index) = axis_step
                    && small_button(ui, "Insertion axis step", true, false)
                        .on_hover_text("Open the Insertion axis step on this tooth")
                        .clicked()
                {
                    action = Some(StepAction::Tooth(index, card.tooth));
                }
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if small_button(ui, "Reset to material", card.overridden, false)
                    .on_hover_text("Use the material's cement gap and seal")
                    .clicked()
                {
                    action = Some(StepAction::ResetInner);
                }
                let mut on = card.section;
                if ui
                    .checkbox(&mut on, "Section")
                    .on_hover_text("Cut the view through the preparation along the axis")
                    .changed()
                {
                    action = Some(StepAction::ToggleSection);
                }
            });
        });
    action
}

/// The Anatomy step's card. `steps` are the indices of the Margin, Insertion axis and Inner
/// surface steps for the tooth, for the buttons to a missing step.
fn anatomy_section(
    ui: &mut Ui,
    card: &AnatomyCard,
    steps: [Option<usize>; 3],
    p: &Palette,
) -> Option<StepAction> {
    let mut action = None;
    let [margin_step, axis_step, inner_step] = steps;
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Tooth {}", card.tooth))
                        .strong()
                        .color(p.text),
                );
                if let Some(busy) = card.busy.filter(|_| !card.pontic) {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(busy).size(SIZE_SMALL).color(p.text_dim));
                        ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    });
                }
            });
            ui.label(
                RichText::new(format!("Crowns from: {}", card.library))
                    .size(SIZE_SMALL)
                    .color(p.text_dim),
            );
            if let Some(error) = &card.library_error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
            }
            egui::ComboBox::from_id_salt("anatomy-library-model")
                .selected_text(format!("Model {}", card.model))
                .show_ui(ui, |ui| {
                    for &(model, from_library) in &card.models {
                        let source = if from_library {
                            "Library"
                        } else {
                            "Parametric"
                        };
                        if ui
                            .selectable_label(model == card.model, format!("{model} · {source}"))
                            .clicked()
                        {
                            action = Some(StepAction::ChooseLibraryTooth(model));
                        }
                    }
                });
            if let Some(note) = &card.library_note {
                ui.label(RichText::new(note).size(SIZE_SMALL).color(p.accent));
            }
            if card.pontic {
                ui.label(
                    RichText::new(PONTIC_NOTE)
                        .size(SIZE_SMALL)
                        .color(p.text_dim),
                );
                return;
            }
            let missing = if !card.has_margin {
                Some((AXIS_NEEDS_MARGIN, "Margin step", margin_step))
            } else if !card.has_axis {
                Some((INNER_NEEDS_AXIS, "Insertion axis step", axis_step))
            } else {
                None
            };
            if let Some((hint, button, index)) = missing {
                ui.label(RichText::new(hint).size(SIZE_SMALL).color(p.text_dim));
                if let Some(index) = index
                    && small_button(ui, button, true, true).clicked()
                {
                    action = Some(StepAction::Tooth(index, card.tooth));
                }
                return;
            }
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(60.0);
            for (param, &value) in AnatomyParam::ALL.into_iter().zip(&card.values) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(param.name())
                            .size(SIZE_SMALL)
                            .color(p.text_dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut v = value;
                        let r = ui.add(
                            egui::Slider::new(&mut v, param.range())
                                .step_by(1.0)
                                .fixed_decimals(0)
                                .suffix(param.unit()),
                        );
                        if r.changed() {
                            action = Some(StepAction::SetAnatomy(param, v, r.dragged()));
                        } else if r.drag_stopped() {
                            action = Some(StepAction::EndAnatomyDrag);
                        }
                    });
                });
            }
            let axes = [
                ("Mesial", "Distal", "Mesial"),
                ("Facial", "Lingual", "Facial"),
                ("Occlusal", "Cervical", "Occlusal"),
            ];
            ui.label(
                RichText::new("Drag M / F / O handles to move the crown; it rebuilds on release.")
                    .size(SIZE_SMALL)
                    .color(p.text_dim),
            );
            let nudge = if ui.input(|i| i.modifiers.shift) {
                FINE_NUDGE_MM
            } else {
                NUDGE_MM
            };
            ui.label(
                RichText::new("Move: 0.1 mm · Shift: 0.01 mm")
                    .size(SIZE_SMALL)
                    .color(p.text_dim),
            );
            for (axis, ((name, minus, plus), &value)) in axes.iter().zip(&card.offset).enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{name} offset"))
                            .size(SIZE_SMALL)
                            .color(p.text_dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if small_button(ui, "+", true, false)
                            .on_hover_text(format!("{plus} {nudge:.2} mm"))
                            .clicked()
                        {
                            action = Some(StepAction::NudgeAnatomy(axis, nudge));
                        }
                        ui.label(
                            RichText::new(format!("{} mm", widgets::fmt_signed(value, 2)))
                                .size(SIZE_SMALL)
                                .color(p.text),
                        );
                        if small_button(ui, "\u{2212}", true, false)
                            .on_hover_text(format!("{minus} {nudge:.2} mm"))
                            .clicked()
                        {
                            action = Some(StepAction::NudgeAnatomy(axis, -nudge));
                        }
                    });
                });
            }
            if small_button(
                ui,
                "Reset placement",
                card.offset.iter().any(|&offset| offset != 0.0) || card.values[5] != 0.0,
                false,
            )
            .on_hover_text("Restore automatic position and rotation; keep the model and shape")
            .clicked()
            {
                action = Some(StepAction::ResetAnatomyPlacement);
            }
            if let Some((thickness, area)) = card.report {
                let rows = [
                    (
                        "Min thickness",
                        format!("{} mm", widgets::fmt_mm(thickness, 3)),
                    ),
                    (
                        "Thickened",
                        format!("{} mm\u{b2}", widgets::fmt_mm(area, 1)),
                    ),
                ];
                for (name, text) in rows {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).size(SIZE_SMALL).color(p.text_dim));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(text).color(p.text).strong());
                        });
                    });
                }
            }
            if let Some(error) = &card.error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
                if card.inner_failed
                    && let Some(index) = inner_step
                    && small_button(ui, "Inner surface step", true, false)
                        .on_hover_text("Open the Inner surface step on this tooth")
                        .clicked()
                {
                    action = Some(StepAction::Tooth(index, card.tooth));
                }
            }
            if small_button(ui, "Reset", card.adjusted, false)
                .on_hover_text("Back to the library shape and the automatic placement")
                .clicked()
            {
                action = Some(StepAction::ResetAnatomy);
            }
        });
    action
}

/// The Occlusion & contacts step's card. `anatomy_step` is the Anatomy step's index for the
/// tooth, for the button to a missing crown.
fn contacts_section(
    ui: &mut Ui,
    card: &ContactsCard,
    anatomy_step: Option<usize>,
    p: &Palette,
) -> Option<StepAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Tooth {}", card.tooth))
                        .strong()
                        .color(p.text),
                );
                if let Some(busy) = card.busy {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(busy).size(SIZE_SMALL).color(p.text_dim));
                        ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    });
                }
            });
            if !card.has_crown {
                ui.label(
                    RichText::new("This tooth has no crown yet.")
                        .size(SIZE_SMALL)
                        .color(p.text_dim),
                );
                if let Some(index) = anatomy_step
                    && small_button(ui, "Anatomy step", true, true).clicked()
                {
                    action = Some(StepAction::Tooth(index, card.tooth));
                }
                return;
            }
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(60.0);
            for (param, &value) in ContactParam::ALL.into_iter().zip(&card.values) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(param.name())
                            .size(SIZE_SMALL)
                            .color(p.text_dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut v = value;
                        let r = ui.add(
                            egui::Slider::new(&mut v, param.range())
                                .step_by(0.01)
                                .fixed_decimals(2)
                                .suffix(" mm"),
                        );
                        if r.changed() {
                            action = Some(StepAction::SetContact(param, v, r.dragged()));
                        } else if r.drag_stopped() {
                            action = Some(StepAction::EndContactDrag);
                        }
                    });
                });
            }
            if !card.antagonist {
                ui.label(
                    RichText::new(NO_ANTAGONIST)
                        .size(SIZE_SMALL)
                        .color(p.text_dim),
                );
            }
            if small_button(ui, "Adapt", card.busy.is_none(), true)
                .on_hover_text("Adapt the crown to its neighbours and the antagonist")
                .clicked()
            {
                action = Some(StepAction::AdaptContacts);
            }
            if card.outdated {
                ui.label(
                    RichText::new("Outdated \u{2014} press Adapt")
                        .size(SIZE_SMALL)
                        .color(p.accent),
                );
            }
            if let Some(report) = &card.report {
                let mm = |v: Option<f64>, none: &str| {
                    v.map_or_else(
                        || none.to_owned(),
                        |v| format!("{} mm", widgets::fmt_mm(v, 3)),
                    )
                };
                let rows = [
                    ("Mesial contact", mm(report.mesial, "unmeasured"), report.mesial.is_some_and(|v| ContactParam::Proximal.needs_review(v, card.values[0]))),
                    ("Distal contact", mm(report.distal, "unmeasured"), report.distal.is_some_and(|v| ContactParam::Proximal.needs_review(v, card.values[0]))),
                    (
                        "Min occlusal distance",
                        mm(report.occlusal, "unmeasured"),
                        report.occlusal.is_some_and(|v| ContactParam::Occlusal.needs_review(v, card.values[1])),
                    ),
                    (
                        "Moved area",
                        format!("{} mm\u{b2}", widgets::fmt_mm(report.moved_area, 1)),
                        false,
                    ),
                ];
                for (name, text, review) in rows {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).size(SIZE_SMALL).color(p.text_dim));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if review {
                                ui.label(RichText::new(format!("Review · {text}")).color(p.danger).strong())
                                    .on_hover_text("The measured distance is outside the selected target. Inspect this surface in the distance map.");
                            } else {
                                ui.label(RichText::new(text).color(p.text).strong());
                            }
                        });
                    });
                }
                if !report.thickness_ok {
                    ui.label(RichText::new(THIN_WARNING).size(SIZE_SMALL).color(p.danger));
                }
            }
            egui::ComboBox::from_label("Map reference")
                .selected_text(card.reference.label())
                .show_ui(ui, |ui| {
                    for reference in crate::contacts_step::MapReference::OPTIONS {
                        if ui
                            .selectable_label(reference == card.reference, reference.label())
                            .clicked()
                        {
                            action = Some(StepAction::MapReference(reference));
                        }
                    }
                });
            match card.map_available {
                Some(true) => {
                    distance_legend(ui, p);
                    ui.weak("Gray areas are outside measurable scan coverage");
                }
                Some(false) => {
                    ui.weak("No measurable reference surface near this crown");
                }
                None => {}
            }
            if let Some(error) = &card.error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
            }
        });
    action
}

/// The Finish & export step's card: every unit's crown, the options and the export.
fn export_section(ui: &mut Ui, card: &ExportCard, p: &Palette) -> Option<StepAction> {
    let mut action = None;
    let busy = card.busy.is_some();
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            if card.rows.is_empty() {
                ui.label(
                    RichText::new("The case has no restorations.")
                        .size(SIZE_SMALL)
                        .color(p.text_dim),
                );
            }
            for row in &card.rows {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} {}", row.tooth, row.kind.label()))
                            .strong()
                            .color(p.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (text, color) = match (&row.result, row.crown) {
                            (Some(Ok(_)), _) => ("Written", p.accent),
                            (Some(Err(_)), _) => ("Failed", p.danger),
                            (None, Some(_)) if row.changed_since_export => ("Changed", p.danger),
                            (None, Some(_)) => ("Ready", p.text),
                            (None, None) => ("Missing crown", p.text_dim),
                        };
                        ui.label(RichText::new(text).size(SIZE_SMALL).color(color));
                    });
                });
                if row.changed_since_export {
                    ui.label(
                        RichText::new(
                            "Changed since last export; export again to update the file.",
                        )
                        .size(SIZE_SMALL)
                        .color(p.text_dim),
                    );
                }
                match row.crown {
                    Some((triangles, thickness)) => {
                        let mut text = format!(
                            "{} triangles \u{b7} min {} mm",
                            group_thousands(triangles),
                            widgets::fmt_mm(thickness, 2)
                        );
                        if let Some(Ok(written)) = &row.result {
                            text = format!(
                                "{} triangles written \u{b7} {}",
                                group_thousands(written.triangles),
                                format_bytes(written.bytes)
                            );
                        }
                        ui.label(RichText::new(text).size(SIZE_SMALL).color(p.text_dim));
                    }
                    None => {
                        if let Some(index) = row.anatomy_step
                            && small_button(ui, "Anatomy step", true, false)
                                .on_hover_text("Build this tooth's crown in the Anatomy step")
                                .clicked()
                        {
                            action = Some(StepAction::Tooth(index, row.tooth));
                        }
                    }
                }
                if let Some(Err(error)) = &row.result {
                    ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
                }
                if let Some(Ok(written)) = &row.result
                    && let Some(warning) = &written.warning
                {
                    ui.label(RichText::new(warning).size(SIZE_SMALL).color(p.danger));
                }
            }
            ui.add_space(2.0);
            let mut simplify = card.simplify;
            if ui
                .add_enabled(
                    !busy,
                    egui::Checkbox::new(&mut simplify, "Simplify (max error 0.002 mm)"),
                )
                .on_hover_text("Fewer triangles; the margin stays exactly where it is")
                .changed()
            {
                action = Some(StepAction::SetSimplify(simplify));
            }
            let ready = card.rows.iter().any(|r| r.crown.is_some());
            ui.horizontal(|ui| {
                if small_button(ui, "Export STL\u{2026}", ready && !busy, true)
                    .on_hover_text("Choose a folder and write one STL per crown")
                    .on_disabled_hover_text("No crown is built yet")
                    .clicked()
                {
                    action = Some(StepAction::ExportStl);
                }
                if let Some(text) = &card.busy {
                    ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    ui.label(RichText::new(text).size(SIZE_SMALL).color(p.text_dim));
                }
            });
            if let Some(summary) = &card.summary {
                ui.label(RichText::new(summary).size(SIZE_SMALL).color(p.text));
                if card.folder.is_some()
                    && small_button(ui, "Open folder", true, false)
                        .on_hover_text("Show the files in the file browser")
                        .clicked()
                {
                    action = Some(StepAction::OpenExportFolder);
                }
            }
        });
    action
}

/// A gradient bar of the undercut colour map with its range.
fn undercut_legend(ui: &mut Ui, p: &Palette) {
    use crate::axis_step::MAX_UNDERCUT_MM;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("Undercut 0 \u{2013} {MAX_UNDERCUT_MM} mm"))
                .size(SIZE_SMALL)
                .color(p.text_dim),
        );
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
        let steps = 24;
        for i in 0..steps {
            let t = i as f32 / (steps - 1) as f32;
            let [r, g, b, _] = dentcraft_render::colormap::sequential(t, 0.0, 1.0);
            let byte = |c: f32| (c * 255.0).round() as u8;
            let x0 = rect.left() + rect.width() * i as f32 / steps as f32;
            let x1 = rect.left() + rect.width() * (i + 1) as f32 / steps as f32;
            ui.painter().rect_filled(
                Rect::from_min_max(
                    egui::pos2(x0, rect.top()),
                    egui::pos2(x1 + 0.5, rect.bottom()),
                ),
                0.0,
                egui::Color32::from_rgb(byte(r), byte(g), byte(b)),
            );
        }
    });
}

/// A colour bar for the contacts step's distance map, [`MAP_MIN_MM`] to [`MAP_MAX_MM`].
fn distance_legend(ui: &mut Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "Distance {} \u{2013} {} mm",
                widgets::fmt_signed(MAP_MIN_MM, 1),
                widgets::fmt_signed(MAP_MAX_MM, 1)
            ))
            .size(SIZE_SMALL)
            .color(p.text_dim),
        );
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
        let steps = 24;
        for i in 0..steps {
            let t = (i as f64 + 0.5) / steps as f64;
            let [r, g, b, _] = distance_color(MAP_MIN_MM + t * (MAP_MAX_MM - MAP_MIN_MM));
            let byte = |c: f32| (c * 255.0).round() as u8;
            let x0 = rect.left() + rect.width() * i as f32 / steps as f32;
            let x1 = rect.left() + rect.width() * (i + 1) as f32 / steps as f32;
            ui.painter().rect_filled(
                Rect::from_min_max(
                    egui::pos2(x0, rect.top()),
                    egui::pos2(x1 + 0.5, rect.bottom()),
                ),
                0.0,
                egui::Color32::from_rgb(byte(r), byte(g), byte(b)),
            );
        }
    });
}

/// The Orientation step's hint, busy state, error and commands.
fn orient_section(ui: &mut Ui, card: &OrientCard, p: &Palette) -> Option<StepAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            if let Some(busy) = card.busy {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                    ui.label(RichText::new(busy).size(SIZE_SMALL).color(p.text_dim));
                });
            } else {
                let hint = if card.has_frame {
                    "Blue arrow: biting direction. Red arrow: towards the front teeth."
                } else {
                    "Find the occlusal plane and midline of the jaw so every view lines up."
                };
                ui.label(RichText::new(hint).size(SIZE_SMALL).color(p.text_dim));
            }
            if let Some(error) = &card.error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
            }
            if let Some(note) = &card.note {
                ui.label(RichText::new(note).size(SIZE_SMALL).color(p.text_dim));
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if small_button(ui, "Auto orient", card.busy.is_none(), true)
                    .on_hover_text("Find the occlusal plane and midline from the jaw scan")
                    .clicked()
                {
                    action = Some(StepAction::AutoOrient);
                }
                if small_button(ui, "Reset", card.has_frame, false)
                    .on_hover_text("Remove the orientation")
                    .clicked()
                {
                    action = Some(StepAction::ResetOrient);
                }
            });
        });
    action
}

fn note_box(ui: &mut Ui, text: &str, p: &Palette) {
    egui::Frame::new()
        .fill(p.panel)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(SIZE_SMALL).color(p.text_dim));
        });
}

fn finished_card(ui: &mut Ui, total: usize, p: &Palette) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 14.0, p.accent);
        paint_icon(
            ui.painter(),
            Icon::Check,
            Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
            p.on_primary,
        );
        ui.label(RichText::new("Design complete").heading().color(p.text));
    });
    ui.label(
        RichText::new(format!(
            "All {total} steps are done or skipped. Use Expert mode to revisit a step."
        ))
        .color(p.text_dim),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_and_expert_fit_small_and_large_windows() {
        for size in [vec2(960.0, 546.0), vec2(1600.0, 946.0)] {
            let viewport = Rect::from_min_size(egui::pos2(0.0, 32.0), size);
            let wizard = wizard_rect(viewport);
            let expert = crate::toolbar::expert_rect(viewport);
            let toolbar = crate::toolbar::toolbar_layout(viewport).0;
            let views = crate::toolbar::view_cluster_layout(viewport).0;
            for panel in [wizard, expert] {
                assert!(viewport.contains_rect(panel));
                assert!(!panel.intersects(toolbar));
                assert!(!panel.intersects(views));
                assert!(!panel.intersects(crate::viewport::gizmo_rect(viewport)));
            }
            assert!(!wizard.intersects(expert));
        }
    }

    #[test]
    fn expert_navigation_preserves_step_dependencies_after_going_back() {
        let mut case = Case::new("test");
        case.set_unit(
            Fdi::new(16).unwrap(),
            dentcraft_case::RestorationKind::Crown,
        );
        let mut session = Session::for_case(&case);
        assert!(reachable(&session, 0));
        assert!(!reachable(&session, 1));
        session.complete().unwrap();
        session.complete().unwrap();
        session.back().unwrap();
        assert!(reachable(&session, 0));
        assert!(reachable(&session, 1));
        assert!(!reachable(&session, 2));
        session.complete().unwrap();
        assert!(reachable(&session, 2));
        assert!(!reachable(&session, 3));
    }

    #[test]
    fn long_step_content_keeps_navigation_clickable_and_blocks_the_scene() {
        let viewport = Rect::from_min_size(egui::pos2(0.0, 32.0), vec2(960.0, 546.0));
        let panel = wizard_rect(viewport);
        for (pos, expect_next, expect_scene) in [
            (panel.right_bottom() - vec2(35.0, 29.0), true, false),
            (panel.left_top() + vec2(3.0, 3.0), false, false),
            (viewport.center() - vec2(100.0, 0.0), false, true),
        ] {
            let ctx = egui::Context::default();
            let mut case = Case::new("test");
            case.set_unit(
                Fdi::new(16).unwrap(),
                dentcraft_case::RestorationKind::Crown,
            );
            let session = Session::for_case(&case);
            let error = "A long step message that must scroll. ".repeat(100);
            let mut action = None;
            let mut scene_clicked = false;
            for events in [
                vec![egui::Event::PointerMoved(pos)],
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ] {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        scene_clicked |= ui
                            .interact(
                                viewport,
                                ui.scope_id().with("scene"),
                                Sense::click_and_drag(),
                            )
                            .clicked();
                        action = action.or(WorkflowView {
                            session: &session,
                            case: &case,
                            has_jaw_scan: true,
                            error: Some(&error),
                            tooth: None,
                            margin: None,
                            orient: None,
                            axis: None,
                            inner: None,
                            anatomy: None,
                            contacts: None,
                            export: None,
                        }
                        .show(ui, viewport));
                    },
                );
                output.textures_delta.clear();
            }
            assert_eq!(action == Some(StepAction::Next), expect_next);
            assert_eq!(scene_clicked, expect_scene);
        }
    }

    #[test]
    fn only_the_scans_step_needs_a_jaw_scan() {
        assert!(blocker(StepId::Scans, false, true).is_some());
        assert!(blocker(StepId::Scans, true, false).is_some());
        assert!(blocker(StepId::Scans, true, true).is_none());
        assert_eq!(blocker(StepId::Scans, true, false), Some(NEEDS_RESTORATION));
        for id in StepId::ALL.into_iter().filter(|id| *id != StepId::Scans) {
            assert!(blocker(id, false, false).is_none(), "{id:?}");
        }
    }
}
