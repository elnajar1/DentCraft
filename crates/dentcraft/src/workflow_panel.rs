//! The guided workflow panel: progress, the planned steps and the active step's card.

use dentcraft_case::{Case, Fdi};
use dentcraft_workflow::{Session, StepId, StepStatus};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Layout, Margin, Rect, RichText,
    Sense, Shape, Stroke, Ui, Vec2, vec2,
};

use crate::browser::{self, Section, Sections};
use crate::chart::{ON_FILL, kind_color};
use crate::theme::{Palette, RADIUS_SM, SIZE_SMALL};
use crate::widgets::{self, Icon, paint_icon, small_button};

/// Shown on every step whose design tools are not built yet.
pub const NOT_YET_NOTE: &str = "Design tools for this step arrive in DentCraft 0.2.";
/// Why the Scans step cannot be completed yet.
pub const NEEDS_JAW_SCAN: &str = "Import an upper or lower jaw scan to continue.";

/// What the user asked the session to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepAction {
    Next,
    Back,
    Skip,
    GoTo(usize),
}

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
                            if step_row(
                                ui,
                                index,
                                step.id,
                                &step.teeth,
                                session.status(index),
                                self.case,
                                &p,
                            ) {
                                action = Some(StepAction::GoTo(index));
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

/// One step of the plan. Returns true when the user clicked a finished step to re-open it.
fn step_row(
    ui: &mut Ui,
    index: usize,
    id: StepId,
    teeth: &[Fdi],
    status: StepStatus,
    case: &Case,
    p: &Palette,
) -> bool {
    let reopenable = matches!(status, StepStatus::Done | StepStatus::Skipped);
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
            let chips_width: f32 = teeth.iter().map(|t| chip_width(ui, *t) + 4.0).sum();
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
                            tooth_chip(ui, tooth, case, p);
                        }
                    }
                });
            });
            if !inline && !teeth.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    ui.add_space(26.0);
                    for &tooth in teeth {
                        tooth_chip(ui, tooth, case, p);
                    }
                });
            }
        });
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
    if reopenable {
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Re-open this step")
            .clicked()
    } else {
        false
    }
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

fn chip_width(ui: &Ui, tooth: Fdi) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(tooth.to_string(), chip_font(), Color32::WHITE);
    galley.size().x + 12.0
}

/// A small chip with a tooth number, tinted with its restoration colour.
fn tooth_chip(ui: &mut Ui, tooth: Fdi, case: &Case, p: &Palette) {
    // Solid restoration fills with dark numbers read at 4.5:1 or better in both themes.
    let (tint, ink) = case
        .unit(tooth)
        .map_or((p.panel_header, p.text), |u| (kind_color(u.kind), ON_FILL));
    let galley = ui
        .painter()
        .layout_no_wrap(tooth.to_string(), chip_font(), ink);
    let size = vec2(galley.size().x + 10.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS_SM), tint);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, ink);
    if let Some(unit) = case.unit(tooth) {
        response.on_hover_text(format!("{tooth} · {}", unit.kind.label()));
    }
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
                tooth_chip(ui, tooth, view.case, p);
            }
        });
    }
    ui.add_space(4.0);
    let ready = can_complete(id, view.has_jaw_scan);
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
