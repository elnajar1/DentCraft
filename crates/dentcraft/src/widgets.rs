//! Small branded widgets: pill buttons, segmented control, painted icons and section labels.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, RichText, Sense,
    Shape, Stroke, TextStyle, Ui, Vec2, WidgetText, epaint::PathShape, pos2, vec2,
};

use crate::theme::{Palette, RADIUS_PILL, SIZE_SMALL, ThemeKind};

/// Palette of the theme `ui` is drawn with.
pub fn palette(ui: &Ui) -> Palette {
    if ui.visuals().dark_mode {
        ThemeKind::Dark.palette()
    } else {
        ThemeKind::Light.palette()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pill {
    /// Filled with the accent colour: the main action of an area.
    Primary,
    /// Raised fill with a hairline border.
    Secondary,
}

/// Pill-shaped button.
pub fn pill(ui: &mut Ui, text: &str, kind: Pill, enabled: bool) -> Response {
    let p = palette(ui);
    let galley = WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let size = vec2(galley.size().x + 32.0, 32.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hovered = enabled && response.hovered();
    let (fill, stroke, ink) = match kind {
        Pill::Primary => (
            if hovered { p.accent_hover } else { p.accent },
            Stroke::NONE,
            p.on_accent,
        ),
        Pill::Secondary => (
            if hovered { p.raised_hover } else { p.raised },
            Stroke::new(1.0, p.line),
            p.ink,
        ),
    };
    let alpha = if enabled { 1.0 } else { 0.45 };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(RADIUS_PILL),
            fill.gamma_multiply(alpha),
            stroke,
            egui::StrokeKind::Inside,
        );
        let pos = rect.center() - galley.size() * 0.5;
        painter.galley(pos, galley, ink.gamma_multiply(alpha));
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A row of mutually exclusive options in a pill-shaped track. Returns the clicked option.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    options: &[(T, &str, String)],
    selected: Option<T>,
) -> Option<T> {
    let p = palette(ui);
    let mut clicked = None;
    egui::Frame::new()
        .fill(p.raised)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(RADIUS_PILL))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (value, label, tooltip) in options {
                    let on = selected == Some(*value);
                    let galley = WidgetText::from(*label).into_galley(
                        ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        TextStyle::Button,
                    );
                    let size = vec2(galley.size().x + 24.0, 26.0);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    let fill = if on {
                        if ui.visuals().dark_mode {
                            p.line
                        } else {
                            p.surface
                        }
                    } else if response.hovered() {
                        p.raised_hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    let stroke = if on && !ui.visuals().dark_mode {
                        Stroke::new(1.0, p.line)
                    } else {
                        Stroke::NONE
                    };
                    let painter = ui.painter();
                    painter.rect(
                        rect,
                        CornerRadius::same(RADIUS_PILL),
                        fill,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                    let ink = if on { p.ink } else { p.muted };
                    painter.galley(rect.center() - galley.size() * 0.5, galley, ink);
                    if response
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(tooltip)
                        .clicked()
                    {
                        clicked = Some(*value);
                    }
                }
            });
        });
    clicked
}

/// Small upper-case label that heads a section.
pub fn section_label(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(SIZE_SMALL)
            .color(p.muted)
            .extra_letter_spacing(1.2)
            .strong(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Eye,
    EyeOff,
    Close,
    Sun,
    Moon,
    Fit,
    Plus,
    /// Tick mark, for done or selected items.
    Check,
    /// Warning triangle.
    Warning,
}

/// Round, frameless icon button.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .circle_filled(rect.center(), size * 0.5, p.raised_hover);
        }
        let ink = if response.hovered() { p.ink } else { p.muted };
        paint_icon(ui.painter(), icon, rect.shrink(size * 0.22), ink, p);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Draws `icon` inside `rect`.
pub fn paint_icon(painter: &egui::Painter, icon: Icon, rect: Rect, ink: Color32, p: Palette) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new(1.5, ink);
    match icon {
        Icon::Eye | Icon::EyeOff => {
            // Lens outline from two arcs, and the iris.
            let n = 16;
            let mut points = Vec::with_capacity(2 * n);
            for i in 0..=n {
                let t = i as f32 / n as f32;
                let x = c.x - r + 2.0 * r * t;
                let y = c.y - (std::f32::consts::PI * t).sin() * r * 0.62;
                points.push(pos2(x, y));
            }
            for i in 1..n {
                let t = 1.0 - i as f32 / n as f32;
                let x = c.x - r + 2.0 * r * t;
                let y = c.y + (std::f32::consts::PI * t).sin() * r * 0.62;
                points.push(pos2(x, y));
            }
            painter.add(Shape::Path(PathShape::closed_line(points, stroke)));
            if icon == Icon::Eye {
                painter.circle_filled(c, r * 0.3, ink);
            } else {
                painter.circle_stroke(c, r * 0.3, stroke);
                painter.line_segment(
                    [c + vec2(-r, r * 0.85), c + vec2(r, -r * 0.85)],
                    Stroke::new(1.5, ink),
                );
            }
        }
        Icon::Close => {
            let d = r * 0.7;
            painter.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
            painter.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
        }
        Icon::Plus => {
            let d = r * 0.8;
            painter.line_segment([c + vec2(-d, 0.0), c + vec2(d, 0.0)], stroke);
            painter.line_segment([c + vec2(0.0, -d), c + vec2(0.0, d)], stroke);
        }
        Icon::Sun => {
            painter.circle_stroke(c, r * 0.4, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let dir = vec2(a.cos(), a.sin());
                painter.line_segment([c + dir * r * 0.65, c + dir * r * 0.95], stroke);
            }
        }
        Icon::Moon => {
            // Crescent: a disc with a background-coloured disc bitten out of it.
            painter.circle_filled(c, r * 0.8, ink);
            let bite = if ink == p.ink {
                p.raised_hover
            } else {
                p.surface
            };
            painter.circle_filled(c + vec2(r * 0.42, -r * 0.32), r * 0.66, bite);
        }
        Icon::Fit => {
            let d = r * 0.9;
            let l = r * 0.45;
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let corner = c + vec2(sx * d, sy * d);
                painter.line_segment([corner, corner - vec2(sx * l, 0.0)], stroke);
                painter.line_segment([corner, corner - vec2(0.0, sy * l)], stroke);
            }
            painter.circle_filled(c, r * 0.18, ink);
        }
        Icon::Check => {
            painter.line(
                vec![
                    c + vec2(-r * 0.75, r * 0.05),
                    c + vec2(-r * 0.25, r * 0.55),
                    c + vec2(r * 0.75, -r * 0.5),
                ],
                Stroke::new(1.8, ink),
            );
        }
        Icon::Warning => {
            let top = c + vec2(0.0, -r * 0.9);
            let left = c + vec2(-r, r * 0.8);
            let right = c + vec2(r, r * 0.8);
            painter.add(Shape::Path(PathShape::closed_line(
                vec![top, right, left],
                Stroke::new(1.3, ink),
            )));
            painter.line_segment([c + vec2(0.0, -r * 0.3), c + vec2(0.0, r * 0.25)], stroke);
            painter.circle_filled(c + vec2(0.0, r * 0.52), 0.9, ink);
        }
    }
}

/// Text centred at `pos` in the small font.
pub fn small_text(painter: &egui::Painter, pos: Pos2, text: &str, color: Color32) {
    painter.text(
        pos,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(SIZE_SMALL - 1.0),
        color,
    );
}
