//! The Objects panel floating at the left of the viewport: the case's scans grouped by role. A
//! checkbox per group shows or hides it, an expander lists its objects one by one, and a
//! right-click on a group sets the transparency of the whole group. "Show all" brings every
//! object back; the "Hidden" expander lists what is hidden.

use dentcraft_case::ScanRole;
use eframe::egui::{
    self, CornerRadius, Id, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::objects::{self, SceneObject};
use crate::theme::{RADIUS_LG, RADIUS_SM, SIZE_SMALL};
use crate::toolbar::{HEIGHT, INSET};
use crate::widgets::{self, Icon, paint_icon};

/// Width of the panel.
pub const WIDTH: f32 = 230.0;
/// Space between the toolbar and the panel.
const GAP: f32 = 8.0;
/// Side of a checkbox.
const CHECK: f32 = 16.0;

/// Where the panel's top-left corner goes in `viewport`: under the toolbar, at its left edge.
pub fn origin(viewport: Rect) -> egui::Pos2 {
    pos2(
        viewport.left() + INSET,
        viewport.top() + INSET + HEIGHT + GAP,
    )
}

/// Id of the checkbox that shows or hides the scans with `role`.
pub fn group_check_id(role: ScanRole) -> Id {
    Id::unique(("objects_group", role))
}

/// Id of the checkbox of the object `index`.
fn object_check_id(index: usize, id: u64) -> Id {
    Id::unique(("objects_item", index, id))
}

/// Draws the panel over `viewport`, above `bottom` (the top of what sits in the bottom-left
/// corner), and applies the user's changes to `objects`.
pub fn show(ui: &Ui, viewport: Rect, bottom: f32, objects: &mut [SceneObject]) {
    let p = widgets::palette(ui);
    let top = origin(viewport);
    let max_height = (bottom - top.y - GAP).max(80.0);
    egui::Area::new(Id::unique("objects_panel"))
        .fixed_pos(top)
        .order(egui::Order::Middle)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            let mut shadow = ui.visuals().popup_shadow;
            shadow.color = shadow.color.gamma_multiply(0.7);
            egui::Frame::new()
                .fill(p.floating())
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(RADIUS_LG))
                .shadow(shadow)
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(WIDTH - 16.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    egui::ScrollArea::vertical()
                        .max_height(max_height - 12.0)
                        .min_scrolled_height(max_height - 12.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| contents(ui, objects));
                });
        });
}

fn contents(ui: &mut Ui, objects: &mut [SceneObject]) {
    let p = widgets::palette(ui);
    let small = |t: String| RichText::new(t).size(SIZE_SMALL).color(p.text_dim);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Objects").strong().color(p.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let any_hidden = objects.iter().any(|o| !o.visible);
            if ui
                .add_enabled(any_hidden, egui::Button::new(small("Show all".to_owned())))
                .on_hover_text("Show every object")
                .clicked()
            {
                for obj in objects.iter_mut() {
                    obj.visible = true;
                }
            }
        });
    });
    if objects.is_empty() {
        ui.label(small(
            "No scans yet. Import them or drop STL, PLY or OBJ files here.".to_owned(),
        ));
        return;
    }
    ui.label(small("Scans".to_owned()));
    for role in ScanRole::ALL {
        let count = objects.iter().filter(|o| o.role == role).count();
        if count > 0 {
            group(ui, role, count, objects);
        }
    }
    let hidden: Vec<usize> = (0..objects.len())
        .filter(|&i| !objects[i].visible)
        .collect();
    if !hidden.is_empty() {
        ui.add_space(4.0);
        egui::CollapsingHeader::new(small(format!("Hidden ({})", hidden.len())))
            .id_salt("objects_hidden")
            .default_open(false)
            .show(ui, |ui| {
                for i in hidden {
                    let obj = &mut objects[i];
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(&obj.name).color(p.text)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Show").clicked() {
                                obj.visible = true;
                            }
                        });
                    });
                }
            });
    }
}

/// One role's group: checkbox, name and count; the expander lists its objects.
fn group(ui: &mut Ui, role: ScanRole, count: usize, objects: &mut [SceneObject]) {
    let p = widgets::palette(ui);
    let id = ui.make_persistent_id(("objects_group_open", role));
    let state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let visible = objects::role_visible(objects, role).unwrap_or(false);
    let (_, header, _) = state
        .show_header(ui, |ui| {
            if check_box(ui, group_check_id(role), visible)
                .on_hover_text(if visible {
                    "Hide the group"
                } else {
                    "Show the group"
                })
                .clicked()
            {
                objects::set_role_visible(objects, role, !visible);
            }
            let label = ui
                .add(
                    egui::Label::new(RichText::new(role.label()).color(if visible {
                        p.text
                    } else {
                        p.text_dim
                    }))
                    .sense(Sense::click()),
                )
                .on_hover_text("Right-click for the group's transparency");
            ui.label(
                RichText::new(count.to_string())
                    .size(SIZE_SMALL)
                    .color(p.text_dim),
            );
            label
        })
        .body(|ui| {
            for (index, obj) in objects.iter_mut().enumerate() {
                if obj.role != role {
                    continue;
                }
                ui.horizontal(|ui| {
                    if check_box(ui, object_check_id(index, obj.id.0), obj.visible)
                        .on_hover_text(if obj.visible { "Hide" } else { "Show" })
                        .clicked()
                    {
                        obj.visible = !obj.visible;
                    }
                    let ink = if obj.visible { p.text } else { p.text_dim };
                    ui.add(egui::Label::new(RichText::new(&obj.name).color(ink)).truncate())
                        .on_hover_text(obj.path.display().to_string());
                });
            }
        });
    header.inner.context_menu(|ui| {
        let with_role = || objects.iter().filter(|o| o.role == role);
        let mut opacity = with_role().map(|o| o.opacity).sum::<f32>() / count as f32;
        let mut transparency = 1.0 - opacity;
        ui.label(RichText::new(format!("{} transparency", role.label())).color(p.text));
        if ui
            .add(
                egui::Slider::new(&mut transparency, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            )
            .changed()
        {
            opacity = 1.0 - transparency;
            for obj in objects.iter_mut().filter(|o| o.role == role) {
                obj.opacity = opacity;
            }
        }
    });
}

/// A checkbox with a known `id`, drawn in the theme's colours.
fn check_box(ui: &mut Ui, id: Id, on: bool) -> egui::Response {
    let p = widgets::palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(CHECK, CHECK), Sense::hover());
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if on { p.accent } else { p.panel };
    let stroke = if response.hovered() || on {
        p.accent
    } else {
        p.border
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(RADIUS_SM),
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    if on {
        paint_icon(painter, Icon::Check, rect.shrink(2.0), egui::Color32::WHITE);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{ObjectState, STONE};
    use dentcraft_render::MeshId;
    use eframe::egui::{Modifiers, Pos2};
    use std::path::PathBuf;

    fn scan(id: u64, role: ScanRole) -> SceneObject {
        SceneObject {
            id: MeshId(id),
            name: format!("scan {id}"),
            path: PathBuf::new(),
            role,
            color: STONE,
            visible: true,
            opacity: 1.0,
            state: ObjectState::Loading,
        }
    }

    /// Runs the panel for a frame with `events`, as in the app's viewport.
    fn frame(ctx: &egui::Context, objects: &mut [SceneObject], events: Vec<egui::Event>) {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let viewport = Rect::from_min_size(pos2(0.0, 32.0), vec2(1300.0, 946.0));
            show(ui, viewport, viewport.bottom() - 100.0, objects);
        });
        // No renderer takes the icon textures here.
        output.textures_delta.clear();
    }

    fn click(ctx: &egui::Context, objects: &mut [SceneObject], id: Id) {
        let pos = ctx
            .read_response(id)
            .expect("the checkbox is drawn")
            .rect
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, objects, vec![egui::Event::PointerMoved(pos)]);
        frame(ctx, objects, vec![button(true)]);
        frame(ctx, objects, vec![button(false)]);
    }

    #[test]
    fn the_group_checkbox_hides_and_shows_the_groups_scans() {
        let ctx = egui::Context::default();
        let mut objects = vec![
            scan(1, ScanRole::Upper),
            scan(2, ScanRole::Lower),
            scan(3, ScanRole::Upper),
        ];
        frame(&ctx, &mut objects, Vec::new());
        assert!(
            ctx.read_response(group_check_id(ScanRole::Bite)).is_none(),
            "a role without scans has no group"
        );
        click(&ctx, &mut objects, group_check_id(ScanRole::Upper));
        assert!(!objects[0].visible && !objects[2].visible);
        assert!(objects[1].visible, "the other group stays shown");
        click(&ctx, &mut objects, group_check_id(ScanRole::Upper));
        assert!(objects.iter().all(|o| o.visible));
    }
}
