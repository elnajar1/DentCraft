//! FDI tooth chart: the arch layout, restoration colours and the interactive chart widget.

use dentcraft_case::{Arch, Case, Fdi, RestorationKind, ToothType};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, RichText, Sense, Shape,
    Stroke, Ui, Vec2, emath::Rot2, pos2, vec2,
};

use crate::theme::{Palette, SIZE_SMALL};
use crate::widgets::{self, Icon, paint_icon};

/// Space between neighbouring teeth, in points.
const GAP: f32 = 2.0;
/// Space between a central incisor and the midline.
const MIDLINE_HALF_GAP: f32 = 2.5;
/// Height of the connector lane on the occlusal side of each arch.
const LANE: f32 = 12.0;
/// Space between the two connector lanes at the back of the arches.
const BAND: f32 = 4.0;
/// Margin around the chart.
const PAD: f32 = 3.0;
/// How far the arches curve, in tooth-width units.
const SAG: f32 = 0.9;
/// Narrowest tooth-width unit: below it the numbers no longer fit.
const MIN_UNIT: f32 = 8.0;

/// Width of a tooth type relative to a premolar.
fn relative_width(t: ToothType) -> f32 {
    match t {
        ToothType::Incisor => 0.85,
        ToothType::Canine => 0.95,
        ToothType::Premolar => 1.0,
        ToothType::Molar => 1.3,
    }
}

/// Crown height of a tooth type relative to a premolar's width.
fn relative_height(t: ToothType) -> f32 {
    match t {
        ToothType::Incisor => 1.5,
        ToothType::Canine => 1.6,
        ToothType::Premolar => 1.4,
        ToothType::Molar => 1.35,
    }
}

/// Where one tooth sits in the chart, relative to the chart's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToothSlot {
    pub tooth: Fdi,
    pub center: Pos2,
    pub size: Vec2,
    /// Rotation that follows the arch curve, in radians.
    pub angle: f32,
    /// Point in the connector lane on the tooth's occlusal side.
    pub lane: Pos2,
}

impl ToothSlot {
    /// Unrotated bounds; the click target of the tooth.
    pub fn rect(&self) -> Rect {
        Rect::from_center_size(self.center, self.size)
    }
}

/// Positions of all 32 teeth: the upper arch on top, 18…11 | 21…28, and the lower arch below,
/// 48…41 | 31…38, each on a gentle curve. The patient's right is on the viewer's left.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartLayout {
    pub slots: Vec<ToothSlot>,
    pub size: Vec2,
    pub midline_x: f32,
    /// Vertical extent of the space between the arches at the midline.
    pub band: (f32, f32),
}

impl ChartLayout {
    pub fn new(width: f32) -> Self {
        let half_units: f32 = (1..=8)
            .map(|p| relative_width(Fdi::new(10 + p).expect("valid FDI").tooth_type()))
            .sum();
        let unit = ((width * 0.5 - PAD - MIDLINE_HALF_GAP - 7.0 * GAP) / half_units).max(MIN_UNIT);
        let half_width = PAD + MIDLINE_HALF_GAP + 7.0 * GAP + half_units * unit;
        let width = 2.0 * half_width;
        let mid = half_width;
        let tallest = relative_height(ToothType::Canine) * unit;
        let sag = SAG * unit;
        let upper_edge = PAD + tallest;
        let lower_edge = upper_edge + 2.0 * sag + 2.0 * LANE + BAND;
        let height = lower_edge + tallest + PAD;

        // Distance of each position from the midline, on the patient's left side.
        let mut offsets = [0.0_f32; 9];
        let mut x = MIDLINE_HALF_GAP;
        for position in 1..=8 {
            let w = relative_width(Fdi::new(20 + position).expect("valid FDI").tooth_type()) * unit;
            offsets[position as usize] = x + w * 0.5;
            x += w + GAP;
        }

        let slots = Fdi::all()
            .map(|tooth| {
                let t = tooth.tooth_type();
                let size = vec2(relative_width(t), relative_height(t)) * unit;
                let side = match tooth.quadrant() {
                    1 | 4 => -1.0,
                    _ => 1.0,
                };
                let dx = side * offsets[tooth.position() as usize];
                let xn = dx / (half_width - PAD);
                let x = mid + dx;
                let curve = sag * xn * xn;
                let slope = 2.0 * sag * xn / (half_width - PAD);
                match tooth.arch() {
                    Arch::Upper => {
                        let edge = upper_edge + curve;
                        ToothSlot {
                            tooth,
                            center: pos2(x, edge - size.y * 0.5),
                            size,
                            angle: slope.atan(),
                            lane: pos2(x, edge + LANE * 0.5),
                        }
                    }
                    Arch::Lower => {
                        let edge = lower_edge - curve;
                        ToothSlot {
                            tooth,
                            center: pos2(x, edge + size.y * 0.5),
                            size,
                            angle: (-slope).atan(),
                            lane: pos2(x, edge - LANE * 0.5),
                        }
                    }
                }
            })
            .collect();
        Self {
            slots,
            size: vec2(width, height),
            midline_x: mid,
            band: (upper_edge + LANE, lower_edge - LANE),
        }
    }

    pub fn slot(&self, tooth: Fdi) -> &ToothSlot {
        self.slots
            .iter()
            .find(|s| s.tooth == tooth)
            .expect("the layout has every tooth")
    }
}

/// Fill colour of a restoration kind on the chart, in the legend and on step chips.
pub fn kind_color(kind: RestorationKind) -> Color32 {
    match kind {
        RestorationKind::Crown => Color32::from_rgb(0xcb, 0xa3, 0x60),
        RestorationKind::Coping => Color32::from_rgb(0xa8, 0x9e, 0x90),
        RestorationKind::Inlay => Color32::from_rgb(0x9a, 0xb5, 0x93),
        RestorationKind::Onlay => Color32::from_rgb(0x72, 0xa0, 0x95),
        RestorationKind::Veneer => Color32::from_rgb(0x93, 0xac, 0xcc),
        RestorationKind::Pontic => Color32::from_rgb(0xcf, 0x96, 0x96),
    }
}

/// Ink for text drawn on a restoration fill.
const ON_FILL: Color32 = Color32::from_rgb(0x1d, 0x1c, 0x1a);

/// A change the user asked for on the chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartEdit {
    /// Set the tooth's restoration, or remove it with `None`.
    Set(Fdi, Option<RestorationKind>),
    /// Connect or disconnect two neighbouring units.
    Connect(Fdi, Fdi, bool),
}

/// "Upper right molar" and the like.
pub fn describe(tooth: Fdi) -> String {
    let place = match tooth.quadrant() {
        1 => "Upper right",
        2 => "Upper left",
        3 => "Lower left",
        _ => "Lower right",
    };
    let kind = match tooth.tooth_type() {
        ToothType::Incisor => "incisor",
        ToothType::Canine => "canine",
        ToothType::Premolar => "premolar",
        ToothType::Molar => "molar",
    };
    format!("{place} {kind}")
}

/// Crown silhouette of a tooth in local coordinates, occlusal edge towards +y, clockwise.
fn outline(t: ToothType, size: Vec2) -> Vec<Vec2> {
    let (exponent, taper) = match t {
        ToothType::Molar => (3.4, 0.10),
        ToothType::Premolar => (3.0, 0.16),
        ToothType::Canine => (2.6, 0.22),
        ToothType::Incisor => (2.8, 0.20),
    };
    let (a, b) = (size.x * 0.5, size.y * 0.5);
    let n = 32;
    (0..n)
        .map(|i| {
            let (s, c) = (i as f32 / n as f32 * std::f32::consts::TAU).sin_cos();
            let x = a * c.signum() * c.abs().powf(2.0 / exponent);
            let y = b * s.signum() * s.abs().powf(2.0 / exponent);
            // The cervical (root) end is narrower than the occlusal one.
            let k = if y < 0.0 { 1.0 - taper * (-y / b) } else { 1.0 };
            vec2(x * k, y)
        })
        .collect()
}

fn tooth_shape(slot: &ToothSlot, origin: Vec2, fill: Color32, stroke: Stroke) -> Shape {
    let rot = Rot2::from_angle(slot.angle);
    let center = slot.center + origin;
    let mut points: Vec<Pos2> = outline(slot.tooth.tooth_type(), slot.size)
        .into_iter()
        .map(|v| match slot.tooth.arch() {
            Arch::Upper => center + rot * v,
            Arch::Lower => center + rot * vec2(v.x, -v.y),
        })
        .collect();
    if slot.tooth.arch() == Arch::Lower {
        // Mirroring reversed the winding.
        points.reverse();
    }
    Shape::convex_polygon(points, fill, stroke)
}

/// The interactive chart. Returns the edit the user made this frame, if any.
pub fn tooth_chart(ui: &mut Ui, case: &Case) -> Option<ChartEdit> {
    let p = widgets::palette(ui);
    let layout = ChartLayout::new(ui.available_width().min(420.0));
    let mut edit = None;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), layout.size.y), Sense::hover());
    let origin = vec2(rect.center().x - layout.size.x * 0.5, rect.min.y);
    let painter = ui.painter_at(rect.expand(2.0));

    // Midline, and which side is the patient's right.
    let mid = origin.x + layout.midline_x;
    let dash = Stroke::new(1.0, p.line);
    painter.extend(Shape::dashed_line(
        &[pos2(mid, rect.top() + 1.0), pos2(mid, rect.bottom() - 1.0)],
        dash,
        3.0,
        3.0,
    ));
    let band_y = origin.y + (layout.band.0 + layout.band.1) * 0.5;
    let side_font = FontId::proportional(SIZE_SMALL - 2.0);
    painter.text(
        pos2(mid - 9.0, band_y),
        Align2::CENTER_CENTER,
        "R",
        side_font.clone(),
        p.muted,
    );
    painter.text(
        pos2(mid + 9.0, band_y),
        Align2::CENTER_CENTER,
        "L",
        side_font,
        p.muted,
    );

    let number_font = FontId::proportional((layout.slot(fdi(11)).size.x * 0.78).clamp(8.5, 11.0));
    for slot in &layout.slots {
        let tooth = slot.tooth;
        let kind = case.unit(tooth).map(|u| u.kind);
        let id = ui.scope_id().with(("tooth", tooth.code()));
        let response = ui
            .interact(slot.rect().translate(origin), id, Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let menu = egui::Popup::menu(&response)
            .width(184.0)
            .show(|ui| tooth_menu(ui, tooth, kind, &p));
        // The popup is shown exactly while it is open: that tooth is the selected one.
        let selected = menu.is_some();
        if let Some(e) = menu.and_then(|r| r.inner) {
            edit = Some(e);
        }
        let hovered = response.hovered();
        let (fill, stroke, ink) = match kind {
            Some(k) => {
                let base = kind_color(k);
                let fill = if hovered {
                    base.lerp_to_gamma(Color32::WHITE, 0.18)
                } else {
                    base
                };
                (
                    fill,
                    Stroke::new(1.0, base.lerp_to_gamma(Color32::BLACK, 0.3)),
                    ON_FILL,
                )
            }
            None => (
                if hovered { p.raised_hover } else { p.raised },
                Stroke::new(1.0, if hovered { p.muted } else { p.line }),
                if hovered { p.ink } else { p.muted },
            ),
        };
        if selected {
            let halo = ToothSlot {
                size: slot.size + Vec2::splat(5.0),
                ..*slot
            };
            painter.add(tooth_shape(
                &halo,
                origin,
                Color32::TRANSPARENT,
                Stroke::new(1.5, p.ink),
            ));
        }
        painter.add(tooth_shape(slot, origin, fill, stroke));
        painter.text(
            slot.center + origin,
            Align2::CENTER_CENTER,
            tooth.to_string(),
            number_font.clone(),
            ink,
        );
        if !selected {
            let label = kind.map_or("No restoration", |k| k.label());
            response.on_hover_text(format!("{tooth} · {label}"));
        }
    }

    // Connector toggles between neighbouring units that can form a bridge.
    for arch in [Arch::Upper, Arch::Lower] {
        let teeth: Vec<Fdi> = arch.teeth().collect();
        for pair in teeth.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let connectable = |t: Fdi| case.unit(t).is_some_and(|u| u.kind.can_connect());
            if !connectable(a) || !connectable(b) {
                continue;
            }
            let pa = layout.slot(a).lane + origin;
            let pb = layout.slot(b).lane + origin;
            let centre = pa + (pb - pa) * 0.5;
            let hit = Rect::from_center_size(centre, vec2((pb.x - pa.x).abs() * 0.8, LANE));
            let id = ui.scope_id().with(("connector", a.code(), b.code()));
            let response = ui
                .interact(hit, id, Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let connected = case.is_connected(a, b);
            let hovered = response.hovered();
            if connected {
                let color = if hovered {
                    p.gold.lerp_to_gamma(Color32::WHITE, 0.25)
                } else {
                    p.gold
                };
                painter.line_segment([pa, pb], Stroke::new(3.5, color));
                painter.circle_filled(pa, 2.6, color);
                painter.circle_filled(pb, 2.6, color);
            } else {
                let pill = Rect::from_center_size(centre, vec2(11.0, 6.0));
                let (fill, stroke) = if hovered {
                    (p.gold.gamma_multiply(0.3), Stroke::new(1.2, p.gold))
                } else {
                    (Color32::TRANSPARENT, Stroke::new(1.2, p.muted))
                };
                painter.rect(
                    pill,
                    CornerRadius::same(3),
                    fill,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
            let tip = if connected {
                format!("Disconnect {a}–{b}")
            } else {
                format!("Connect {a}–{b} into a bridge")
            };
            if response.on_hover_text(tip).clicked() {
                edit = Some(ChartEdit::Connect(a, b, !connected));
            }
        }
    }
    edit
}

fn fdi(code: u8) -> Fdi {
    Fdi::new(code).expect("valid FDI")
}

/// The restoration menu of one tooth. Returns the chosen edit.
fn tooth_menu(
    ui: &mut Ui,
    tooth: Fdi,
    current: Option<RestorationKind>,
    p: &Palette,
) -> Option<ChartEdit> {
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(
        RichText::new(format!("Tooth {tooth}"))
            .strong()
            .color(p.ink),
    );
    ui.label(
        RichText::new(describe(tooth))
            .size(SIZE_SMALL)
            .color(p.muted),
    );
    ui.add_space(4.0);
    let mut edit = None;
    for kind in RestorationKind::ALL {
        if menu_row(
            ui,
            Some(kind_color(kind)),
            kind.label(),
            current == Some(kind),
            p,
        )
        .clicked()
        {
            edit = Some(ChartEdit::Set(tooth, Some(kind)));
        }
    }
    ui.separator();
    if menu_row(ui, None, "No restoration", current.is_none(), p).clicked() {
        edit = Some(ChartEdit::Set(tooth, None));
    }
    if edit.is_some() {
        ui.close();
    }
    edit
}

fn menu_row(
    ui: &mut Ui,
    swatch: Option<Color32>,
    label: &str,
    checked: bool,
    p: &Palette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), p.raised_hover);
    }
    let dot = pos2(rect.left() + 14.0, rect.center().y);
    match swatch {
        Some(color) => painter.circle_filled(dot, 5.0, color),
        None => painter.circle_stroke(dot, 4.5, Stroke::new(1.2, p.muted)),
    };
    painter.text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        p.ink,
    );
    if checked {
        let r = Rect::from_center_size(
            pos2(rect.right() - 14.0, rect.center().y),
            Vec2::splat(12.0),
        );
        paint_icon(painter, Icon::Check, r, p.gold, *p);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Compact key to the restoration colours and the bridge connector.
pub fn legend(ui: &mut Ui) {
    let p = widgets::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
        for kind in RestorationKind::ALL {
            legend_item(ui, kind.label(), &p, |painter, r| {
                painter.rect_filled(r, CornerRadius::same(3), kind_color(kind));
            });
        }
        legend_item(ui, "Bridge", &p, |painter, r| {
            let y = r.center().y;
            painter.line_segment(
                [pos2(r.left() - 2.0, y), pos2(r.right() + 2.0, y)],
                Stroke::new(3.0, p.gold),
            );
        });
    });
}

fn legend_item(ui: &mut Ui, label: &str, p: &Palette, swatch: impl FnOnce(&egui::Painter, Rect)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
        swatch(
            ui.painter(),
            Rect::from_center_size(r.center(), Vec2::splat(9.0)),
        );
        ui.label(RichText::new(label).size(SIZE_SMALL - 1.0).color(p.muted));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirror(tooth: Fdi) -> Fdi {
        // 1x ↔ 2x and 4x ↔ 3x.
        let quadrant = match tooth.quadrant() {
            1 => 2,
            2 => 1,
            3 => 4,
            _ => 3,
        };
        fdi(quadrant * 10 + tooth.position())
    }

    #[test]
    fn layout_has_all_32_teeth_once() {
        let layout = ChartLayout::new(268.0);
        assert_eq!(layout.slots.len(), 32);
        let mut codes: Vec<u8> = layout.slots.iter().map(|s| s.tooth.code()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), 32);
    }

    #[test]
    fn teeth_do_not_overlap() {
        for width in [268.0, 300.0, 420.0] {
            let layout = ChartLayout::new(width);
            for (i, a) in layout.slots.iter().enumerate() {
                for b in &layout.slots[i + 1..] {
                    assert!(
                        !a.rect().intersects(b.rect()),
                        "{} and {} overlap at width {width}",
                        a.tooth,
                        b.tooth
                    );
                }
            }
        }
    }

    #[test]
    fn layout_fits_the_panel_width() {
        let layout = ChartLayout::new(268.0);
        assert!(layout.size.x <= 268.0 + 0.01, "{}", layout.size.x);
        for slot in &layout.slots {
            let r = slot.rect();
            assert!(
                r.left() >= 0.0 && r.right() <= layout.size.x,
                "{}",
                slot.tooth
            );
            assert!(
                r.top() >= 0.0 && r.bottom() <= layout.size.y,
                "{}",
                slot.tooth
            );
        }
    }

    #[test]
    fn layout_is_symmetric_about_the_midline() {
        let layout = ChartLayout::new(300.0);
        for slot in &layout.slots {
            let other = layout.slot(mirror(slot.tooth));
            let mid = layout.midline_x;
            assert!((slot.center.x - mid + (other.center.x - mid)).abs() < 1e-3);
            assert!((slot.center.y - other.center.y).abs() < 1e-3);
            assert_eq!(slot.size, other.size);
            assert!((slot.angle + other.angle).abs() < 1e-5);
        }
    }

    #[test]
    fn arches_read_right_to_left_and_upper_above_lower() {
        let layout = ChartLayout::new(300.0);
        for arch in [Arch::Upper, Arch::Lower] {
            let xs: Vec<f32> = arch.teeth().map(|t| layout.slot(t).center.x).collect();
            assert!(
                xs.windows(2).all(|w| w[0] < w[1]),
                "{arch:?} is not left to right"
            );
        }
        let upper_bottom = Arch::Upper
            .teeth()
            .map(|t| layout.slot(t).rect().bottom())
            .fold(f32::MIN, f32::max);
        let lower_top = Arch::Lower
            .teeth()
            .map(|t| layout.slot(t).rect().top())
            .fold(f32::MAX, f32::min);
        assert!(upper_bottom < lower_top);
    }

    #[test]
    fn molars_are_wider_than_incisors_and_arches_curve() {
        let layout = ChartLayout::new(300.0);
        assert!(layout.slot(fdi(16)).size.x > layout.slot(fdi(11)).size.x);
        // Upper arch: incisors higher than molars; lower arch: the reverse.
        assert!(layout.slot(fdi(11)).rect().bottom() < layout.slot(fdi(18)).rect().bottom());
        assert!(layout.slot(fdi(41)).rect().top() > layout.slot(fdi(48)).rect().top());
    }
}
