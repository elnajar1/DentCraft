//! The Objects panel floating at the left of the viewport: the case's scans grouped by role. A
//! checkbox per group shows or hides it, an expander lists its objects one by one, and a
//! right-click on a group sets the transparency of the whole group. "Show all" brings every
//! object back; the "Hidden" expander lists what is hidden.

use dentcraft_case::{Fdi, ScanRole};
use eframe::egui::{
    self, CornerRadius, Id, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::design_objects::{DesignObjects, PartKind};
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
pub fn show(
    ui: &Ui,
    viewport: Rect,
    bottom: f32,
    objects: &mut [SceneObject],
    mut design: Design<'_>,
) {
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
                        .show(ui, |ui| contents(ui, objects, &mut design));
                });
        });
}

/// The designed parts the panel lists, and the user's choices for them.
pub struct Design<'a> {
    pub objects: &'a mut DesignObjects,
    /// Every designed part there is, in the order listed.
    pub parts: &'a [(PartKind, Fdi)],
}

/// Id of the checkbox that shows or hides the design group `kind`.
pub fn part_group_check_id(kind: PartKind) -> Id {
    Id::unique(("objects_part_group", kind))
}

/// Id of the checkbox of `tooth`'s part of `kind`.
pub fn part_check_id(kind: PartKind, tooth: Fdi) -> Id {
    Id::unique(("objects_part", kind, tooth))
}

/// Id of the checkbox of the opposite-jaw toggle.
pub fn auto_hide_check_id() -> Id {
    Id::unique("objects_auto_hide")
}

/// One hidden item of the Hidden list.
enum HiddenItem {
    Scan(usize),
    Part(PartKind, Fdi),
}

fn contents(ui: &mut Ui, objects: &mut [SceneObject], design: &mut Design<'_>) {
    let p = widgets::palette(ui);
    let small = |t: String| RichText::new(t).size(SIZE_SMALL).color(p.text_dim);
    let mut hidden: Vec<HiddenItem> = (0..objects.len())
        .filter(|&i| !objects[i].visible)
        .map(HiddenItem::Scan)
        .collect();
    hidden.extend(
        design
            .parts
            .iter()
            .filter(|(kind, tooth)| !design.objects.shown(*kind, *tooth))
            .map(|&(kind, tooth)| HiddenItem::Part(kind, tooth)),
    );
    ui.horizontal(|ui| {
        ui.label(RichText::new("Objects").strong().color(p.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    !hidden.is_empty(),
                    egui::Button::new(small("Show all".to_owned())),
                )
                .on_hover_text("Show every object")
                .clicked()
            {
                for obj in objects.iter_mut() {
                    obj.visible = true;
                }
                design.objects.show_all();
            }
        });
    });
    ui.horizontal(|ui| {
        let on = design.objects.auto_hide;
        if check_box(ui, auto_hide_check_id(), on)
            .on_hover_text("Hide the opposite jaw while the view looks at it from behind")
            .clicked()
        {
            design.objects.auto_hide = !on;
        }
        ui.label(small("Hide opposite jaw from behind".to_owned()));
    });
    if objects.is_empty() && design.parts.is_empty() {
        ui.label(small(
            "No scans yet. Import them or drop STL, PLY or OBJ files here.".to_owned(),
        ));
        return;
    }
    if !objects.is_empty() {
        ui.label(small("Scans".to_owned()));
    }
    for role in ScanRole::ALL {
        let count = objects.iter().filter(|o| o.role == role).count();
        if count > 0 {
            group(ui, role, count, objects);
        }
    }
    if !design.parts.is_empty() {
        ui.add_space(4.0);
        ui.label(small("Design".to_owned()));
        for kind in PartKind::ALL {
            let teeth: Vec<Fdi> = design
                .parts
                .iter()
                .filter(|(k, _)| *k == kind)
                .map(|(_, t)| *t)
                .collect();
            if !teeth.is_empty() {
                part_group(ui, kind, &teeth, design.objects);
            }
        }
    }
    if !hidden.is_empty() {
        ui.add_space(4.0);
        egui::CollapsingHeader::new(small(format!("Hidden ({})", hidden.len())))
            .id_salt("objects_hidden")
            .default_open(false)
            .show(ui, |ui| {
                for item in hidden {
                    let name = match item {
                        HiddenItem::Scan(i) => objects[i].name.clone(),
                        HiddenItem::Part(kind, tooth) => kind.item(tooth),
                    };
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(name).color(p.text)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Show").clicked() {
                                match item {
                                    HiddenItem::Scan(i) => objects[i].visible = true,
                                    HiddenItem::Part(kind, tooth) => {
                                        let teeth: Vec<Fdi> = design
                                            .parts
                                            .iter()
                                            .filter(|(k, _)| *k == kind)
                                            .map(|(_, t)| *t)
                                            .collect();
                                        design.objects.set_shown(kind, tooth, &teeth, true);
                                    }
                                }
                            }
                        });
                    });
                }
            });
    }
}

/// One design group: checkbox, name and count; the expander lists its teeth.
fn part_group(ui: &mut Ui, kind: PartKind, teeth: &[Fdi], design: &mut DesignObjects) {
    let p = widgets::palette(ui);
    let id = ui.make_persistent_id(("objects_part_group_open", kind));
    let state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    let visible = teeth.iter().any(|t| design.shown(kind, *t));
    let (_, header, _) = state
        .show_header(ui, |ui| {
            if check_box(ui, part_group_check_id(kind), visible)
                .on_hover_text(if visible {
                    "Hide the group"
                } else {
                    "Show the group"
                })
                .clicked()
            {
                design.set_group(kind, !visible);
            }
            let label = ui.add(
                egui::Label::new(RichText::new(kind.label()).color(if visible {
                    p.text
                } else {
                    p.text_dim
                }))
                .sense(Sense::click()),
            );
            let label = if kind.has_opacity() {
                label.on_hover_text("Right-click for the group's transparency")
            } else {
                label
            };
            ui.label(
                RichText::new(teeth.len().to_string())
                    .size(SIZE_SMALL)
                    .color(p.text_dim),
            );
            label
        })
        .body(|ui| {
            for &tooth in teeth {
                ui.horizontal(|ui| {
                    let shown = design.shown(kind, tooth);
                    if check_box(ui, part_check_id(kind, tooth), shown)
                        .on_hover_text(if shown { "Hide" } else { "Show" })
                        .clicked()
                    {
                        design.set_shown(kind, tooth, teeth, !shown);
                    }
                    let ink = if shown { p.text } else { p.text_dim };
                    ui.label(RichText::new(format!("Tooth {tooth}")).color(ink));
                });
            }
        });
    if kind.has_opacity() {
        header.inner.context_menu(|ui| {
            let mut transparency = 1.0 - design.opacity(kind);
            ui.label(RichText::new(format!("{} transparency", kind.label())).color(p.text));
            if ui
                .add(
                    egui::Slider::new(&mut transparency, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                )
                .changed()
            {
                design.set_opacity(kind, 1.0 - transparency);
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
            auto_hidden: false,
            state: ObjectState::Loading,
        }
    }

    /// What the panel shows in a test.
    #[derive(Default)]
    struct Scene {
        objects: Vec<SceneObject>,
        design: DesignObjects,
        parts: Vec<(PartKind, Fdi)>,
    }

    /// Runs the panel for a frame with `events`, as in the app's viewport.
    fn frame(ctx: &egui::Context, scene: &mut Scene, events: Vec<egui::Event>) {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let viewport = Rect::from_min_size(pos2(0.0, 32.0), vec2(1300.0, 946.0));
            let design = Design {
                objects: &mut scene.design,
                parts: &scene.parts,
            };
            show(
                ui,
                viewport,
                viewport.bottom() - 100.0,
                &mut scene.objects,
                design,
            );
        });
        // No renderer takes the icon textures here.
        output.textures_delta.clear();
    }

    fn click(ctx: &egui::Context, scene: &mut Scene, id: Id) {
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
        frame(ctx, scene, vec![egui::Event::PointerMoved(pos)]);
        frame(ctx, scene, vec![button(true)]);
        frame(ctx, scene, vec![button(false)]);
    }

    #[test]
    fn the_crowns_checkbox_hides_and_shows_every_crown() {
        use crate::design_objects::{DesignPart, PartGeometry};
        use crate::theme::Palette;
        use dentcraft_render::MeshData;
        use std::sync::Arc;
        let ctx = egui::Context::default();
        let teeth: Vec<Fdi> = [11, 12, 21].map(|t| Fdi::new(t).unwrap()).to_vec();
        let mesh = Arc::new(MeshData::from_mesh(&dentcraft_mesh::TriMesh::uv_sphere(
            1.0, 8, 4,
        )));
        let parts: Vec<DesignPart> = teeth
            .iter()
            .map(|&tooth| DesignPart {
                kind: PartKind::Crown,
                tooth,
                geometry: PartGeometry::Mesh(mesh.clone()),
            })
            .collect();
        let mut scene = Scene {
            parts: parts.iter().map(|p| (p.kind, p.tooth)).collect(),
            ..Scene::default()
        };
        frame(&ctx, &mut scene, Vec::new());
        assert!(
            ctx.read_response(part_group_check_id(PartKind::Inner))
                .is_none(),
            "no inner surfaces, no group"
        );
        click(&ctx, &mut scene, part_group_check_id(PartKind::Crown));
        let (items, _) = scene.design.draw(&parts, &Palette::DARK, &[], None);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|(i, _)| !i.visible), "every crown hidden");
        click(&ctx, &mut scene, part_group_check_id(PartKind::Crown));
        let (items, _) = scene.design.draw(&parts, &Palette::DARK, &[], None);
        assert!(items.iter().all(|(i, _)| i.visible));
    }

    #[test]
    fn the_opposite_jaw_toggle_switches_the_rule_on_and_off() {
        let ctx = egui::Context::default();
        let mut scene = Scene::default();
        frame(&ctx, &mut scene, Vec::new());
        click(&ctx, &mut scene, auto_hide_check_id());
        assert!(scene.design.auto_hide);
        click(&ctx, &mut scene, auto_hide_check_id());
        assert!(!scene.design.auto_hide);
    }

    #[test]
    fn the_group_checkbox_hides_and_shows_the_groups_scans() {
        let ctx = egui::Context::default();
        let mut scene = Scene {
            objects: vec![
                scan(1, ScanRole::Upper),
                scan(2, ScanRole::Lower),
                scan(3, ScanRole::Upper),
            ],
            ..Scene::default()
        };
        frame(&ctx, &mut scene, Vec::new());
        assert!(
            ctx.read_response(group_check_id(ScanRole::Bite)).is_none(),
            "a role without scans has no group"
        );
        click(&ctx, &mut scene, group_check_id(ScanRole::Upper));
        let objects = &scene.objects;
        assert!(!objects[0].visible && !objects[2].visible);
        assert!(objects[1].visible, "the other group stays shown");
        click(&ctx, &mut scene, group_check_id(ScanRole::Upper));
        assert!(scene.objects.iter().all(|o| o.visible));
    }
}
