//! The guided workflow panel: progress, the planned steps and the active step's card.

use dentcraft_case::{Case, Fdi};
use dentcraft_workflow::{Session, StepId, StepStatus};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Layout, Margin, Rect, RichText,
    Sense, Shape, Stroke, Ui, Vec2, vec2,
};

use crate::anatomy_step::{AnatomyParam, NUDGE_MM};
use crate::browser::{self, Section, Sections};
use crate::chart::{ON_FILL, kind_color};
use crate::inner_step::InnerParam;
use crate::theme::{Palette, RADIUS_SM, SIZE_SMALL};
use crate::widgets::{self, Icon, paint_icon, small_button};

/// Shown on every step whose design tools are not built yet.
pub const NOT_YET_NOTE: &str = "Design tools for this step arrive in DentCraft 0.2.";
/// Why the Scans step cannot be completed yet.
pub const NEEDS_JAW_SCAN: &str = "Import an upper or lower jaw scan to continue.";

/// What the user asked the session to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepAction {
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
    /// Put the current tooth's crown back to the library shape and automatic placement.
    ResetAnatomy,
}

/// What the Anatomy step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct AnatomyCard {
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
}

/// Hint shown until the tooth has a margin.
pub const MARGIN_HINT: &str = "Click the top of the preparation";

/// Whether the active step may be completed: the Scans step needs a jaw scan.
pub fn can_complete(step: StepId, has_jaw_scan: bool) -> bool {
    step != StepId::Scans || has_jaw_scan
}

pub struct WorkflowView<'a> {
    pub session: &'a Session,
    pub case: &'a Case,
    pub has_jaw_scan: bool,
    pub error: Option<&'a str>,
    /// Height of the step section last frame, so the step list leaves room for it.
    pub card_height: &'a mut f32,
    pub sections: &'a mut Sections,
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
}

impl WorkflowView<'_> {
    pub fn show(self, ui: &mut Ui) -> Option<StepAction> {
        let p = widgets::palette(ui);
        let session = self.session;
        let mut action = None;
        let (done, total) = session.progress();
        ui.spacing_mut().item_spacing.y = 0.0;

        browser::header(
            ui,
            Section::Workflow,
            self.sections.open_mut(Section::Workflow),
            |ui| {
                ui.label(
                    RichText::new(format!("{done} of {total} steps"))
                        .size(SIZE_SMALL)
                        .color(p.text_dim),
                );
            },
        );
        if self.sections.is_open(Section::Workflow) {
            let step_open = self.sections.is_open(Section::Step);
            let card_height = *self.card_height;
            browser::body(ui, |ui| {
                progress_bar(ui, done as f32 / total.max(1) as f32, &p);
                ui.add_space(4.0);
                // Leave room for the step section under the list.
                let reserve = if step_open {
                    card_height
                } else {
                    browser::HEADER_HEIGHT + 1.0
                };
                let list_height =
                    (ui.available_height() - reserve - f32::from(browser::BODY_MARGIN.bottom))
                        .max(80.0);
                egui::ScrollArea::vertical()
                    .max_height(list_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (index, step) in session.steps().iter().enumerate() {
                            if let Some(a) = step_row(
                                ui,
                                index,
                                step.id,
                                &step.teeth,
                                session.status(index),
                                self.case,
                                self.tooth,
                                &p,
                            ) {
                                action = Some(a);
                            }
                        }
                    });
            });
        }

        let top = ui.cursor().top();
        let current = session.current().map(|s| (session.current_index(), s));
        browser::header(
            ui,
            Section::Step,
            self.sections.open_mut(Section::Step),
            |ui| {
                let text = match current {
                    Some((index, _)) => format!("{} of {total}", index + 1),
                    None => "Done".to_owned(),
                };
                ui.label(RichText::new(text).size(SIZE_SMALL).color(p.text_dim));
            },
        );
        if self.sections.is_open(Section::Step) {
            browser::body(ui, |ui| match current {
                Some((index, step)) => {
                    if let Some(a) = active_card(ui, &self, index, step.id, &step.teeth, &p) {
                        action = Some(a);
                    }
                }
                None => finished_card(ui, total, &p),
            });
        }
        *self.card_height = ui.cursor().top() - top;
        action
    }
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
    matches!(id, StepId::Margin | StepId::InsertionAxis)
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

/// One step of the plan. Returns the user's click: a finished step to re-open it, or a tooth.
#[allow(clippy::too_many_arguments)]
fn step_row(
    ui: &mut Ui,
    index: usize,
    id: StepId,
    teeth: &[Fdi],
    status: StepStatus,
    case: &Case,
    current: Option<Fdi>,
    p: &Palette,
) -> Option<StepAction> {
    let reopenable = matches!(status, StepStatus::Done | StepStatus::Skipped);
    let mut picked = None;
    let mut chip = |ui: &mut Ui, tooth: Fdi| {
        let mark = chip_mark(id, status, tooth, case, current);
        if tooth_chip(ui, tooth, case, mark, p) {
            picked = Some(StepAction::Tooth(index, tooth));
        }
    };
    let background = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::new()
        .inner_margin(Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = RichText::new(id.title());
            let title = match status {
                StepStatus::Active => title.color(p.text).strong(),
                StepStatus::Done => title.color(p.text),
                StepStatus::Skipped | StepStatus::Pending => title.color(p.text_dim),
            };
            // Chips sit beside the title when they fit, and on their own line otherwise.
            let title_width = egui::WidgetText::from(title.clone())
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Body,
                )
                .size()
                .x;
            let chips_width: f32 = teeth
                .iter()
                .map(|t| chip_width(ui, *t, chip_mark(id, status, *t, case, current)) + 4.0)
                .sum();
            let inline = 20.0 + 8.0 + title_width + 12.0 + chips_width <= ui.available_width();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                status_icon(ui, index, status, p);
                ui.label(title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let tag = match status {
                        StepStatus::Skipped => Some("skipped"),
                        _ if id.is_optional() && teeth.is_empty() => Some("optional"),
                        _ => None,
                    };
                    if let Some(tag) = tag {
                        ui.label(RichText::new(tag).size(SIZE_SMALL - 1.0).color(p.text_dim));
                    }
                    if inline {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for &tooth in teeth.iter().rev() {
                            chip(ui, tooth);
                        }
                    }
                });
            });
            if !inline && !teeth.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    ui.add_space(26.0);
                    for &tooth in teeth {
                        chip(ui, tooth);
                    }
                });
            }
        });
    if picked.is_some() {
        return picked;
    }
    let rect = inner.response.rect;
    let sense = if reopenable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, ui.scope_id().with(("step", index)), sense);
    let fill = if status == StepStatus::Active {
        p.panel_header
    } else if reopenable && response.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().set(
        background,
        Shape::rect_filled(rect, CornerRadius::same(8), fill),
    );
    let reopen = reopenable
        && response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Re-open this step")
            .clicked();
    reopen.then_some(StepAction::GoTo(index))
}

fn status_icon(ui: &mut Ui, index: usize, status: StepStatus, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    let c = rect.center();
    let painter = ui.painter();
    match status {
        StepStatus::Done => {
            painter.circle_filled(c, 10.0, p.accent.gamma_multiply(0.22));
            paint_icon(painter, Icon::Check, rect.shrink(5.0), p.accent, *p);
        }
        StepStatus::Skipped => {
            painter.circle_stroke(c, 9.5, Stroke::new(1.0, p.border));
            painter.line_segment(
                [c + vec2(-4.0, 0.0), c + vec2(4.0, 0.0)],
                Stroke::new(1.6, p.text_dim),
            );
        }
        StepStatus::Active => {
            painter.circle_stroke(c, 9.0, Stroke::new(1.5, p.accent));
            painter.circle_filled(c, 4.5, p.accent);
        }
        StepStatus::Pending => {
            painter.circle_stroke(c, 9.5, Stroke::new(1.0, p.border));
            painter.text(
                c,
                Align2::CENTER_CENTER,
                (index + 1).to_string(),
                FontId::proportional(SIZE_SMALL - 1.0),
                p.text_dim,
            );
        }
    }
}

fn chip_font() -> FontId {
    FontId::proportional(SIZE_SMALL - 1.0)
}

/// Room for the tick of a done chip.
const CHIP_TICK: f32 = 10.0;

fn chip_width(ui: &Ui, tooth: Fdi, mark: ChipMark) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(tooth.to_string(), chip_font(), Color32::WHITE);
    galley.size().x + 12.0 + if mark.done { CHIP_TICK } else { 0.0 }
}

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
        paint_icon(ui.painter(), Icon::Check, at, ink, *p);
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
    let total = session.steps().len();
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
    let ready = can_complete(id, view.has_jaw_scan);
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
    } else {
        let note = if id == StepId::Scans {
            if ready {
                "Jaw scan loaded. Check each scan's role in the case panel."
            } else {
                NEEDS_JAW_SCAN
            }
        } else {
            NOT_YET_NOTE
        };
        note_box(ui, note, p);
    }
    if let Some(error) = view.error {
        ui.label(
            RichText::new(error)
                .size(SIZE_SMALL)
                .color(p.text_dim)
                .italics(),
        );
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if small_button(ui, "Back", index > 0, false)
            .on_hover_text("Re-open the previous step")
            .clicked()
        {
            action = Some(StepAction::Back);
        }
        if id.is_optional()
            && small_button(ui, "Skip", true, false)
                .on_hover_text("Leave this step out")
                .clicked()
        {
            action = Some(StepAction::Skip);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let last = session.progress().0 + 1 == total;
            let label = if last { "Finish" } else { "Next" };
            let next = small_button(ui, label, ready, true);
            if next.on_disabled_hover_text(NEEDS_JAW_SCAN).clicked() {
                action = Some(StepAction::Next);
            }
        });
    });
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
                            RichText::new(format!("{length:.2} mm"))
                                .color(p.text)
                                .strong(),
                        );
                        ui.label(RichText::new("Length").size(SIZE_SMALL).color(p.text_dim));
                    }
                });
            });
            let hint = if has_margin {
                "Drag a point along the surface. Click the line to add a point; Delete removes one."
            } else {
                MARGIN_HINT
            };
            ui.label(RichText::new(hint).size(SIZE_SMALL).color(p.text_dim));
            if let Some(error) = &card.error {
                ui.label(RichText::new(error).size(SIZE_SMALL).color(p.danger));
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
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
            let degrees = |d: f64| format!("{d:.1}\u{b0}");
            value(ui, "Tilt from occlusal", card.tilt_deg.map(degrees));
            value(
                ui,
                "Undercut area",
                card.undercut_mm2.map(|a| format!("{a:.2} mm\u{b2}")),
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
            for (axis, ((name, minus, plus), &value)) in axes.iter().zip(&card.offset).enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{name} offset"))
                            .size(SIZE_SMALL)
                            .color(p.text_dim),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if small_button(ui, "+", true, false)
                            .on_hover_text(format!("{plus} {NUDGE_MM:.1} mm"))
                            .clicked()
                        {
                            action = Some(StepAction::NudgeAnatomy(axis, NUDGE_MM));
                        }
                        ui.label(
                            RichText::new(format!("{value:+.1} mm"))
                                .size(SIZE_SMALL)
                                .color(p.text),
                        );
                        if small_button(ui, "\u{2212}", true, false)
                            .on_hover_text(format!("{minus} {NUDGE_MM:.1} mm"))
                            .clicked()
                        {
                            action = Some(StepAction::NudgeAnatomy(axis, -NUDGE_MM));
                        }
                    });
                });
            }
            if let Some((thickness, area)) = card.report {
                ui.label(
                    RichText::new(format!(
                        "Min thickness {thickness:.2} mm \u{b7} thickened {area:.1} mm\u{b2}"
                    ))
                    .size(SIZE_SMALL)
                    .color(p.text),
                );
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
            *p,
        );
        ui.label(RichText::new("Design complete").heading().color(p.text));
    });
    ui.label(
        RichText::new(format!(
            "All {total} steps are done or skipped. Click a step to revisit it."
        ))
        .color(p.text_dim),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_scans_step_needs_a_jaw_scan() {
        assert!(!can_complete(StepId::Scans, false));
        assert!(can_complete(StepId::Scans, true));
        for id in StepId::ALL.into_iter().filter(|id| *id != StepId::Scans) {
            assert!(can_complete(id, false), "{id:?}");
        }
    }
}
