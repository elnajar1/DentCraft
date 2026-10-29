//! Small branded widgets: small and command buttons, and painted line icons.

use dentcraft_render::ViewPreset;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, Sense, Shape,
    Stroke, TextStyle, Ui, Vec2, WidgetText, epaint::PathShape, pos2, vec2,
};

use crate::theme::{Palette, RADIUS_SM, SIZE_SMALL, ThemeKind};
use crate::workspace::Workspace;

/// Palette of the theme `ui` is drawn with.
pub fn palette(ui: &Ui) -> Palette {
    if ui.visuals().dark_mode {
        ThemeKind::Dark.palette()
    } else {
        ThemeKind::Light.palette()
    }
}

/// Small flat button with a hairline border. `primary` marks the main action of an area with
/// the selected fill and an accent border.
pub fn small_button(ui: &mut Ui, text: &str, enabled: bool, primary: bool) -> Response {
    let p = palette(ui);
    let galley = WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let size = vec2((galley.size().x + 20.0).max(56.0), 24.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let (mut fill, ink) = button_colors(&p, &response, enabled, primary);
        if fill == Color32::TRANSPARENT {
            fill = p.panel_header;
        }
        let border = if primary && enabled {
            p.accent
        } else {
            p.border
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(RADIUS_SM),
            fill,
            Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() * 0.5, galley, ink);
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
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
    /// Blank page with a plus.
    NewDoc,
    FolderOpen,
    /// Floppy disk.
    Save,
    /// Arrow down into a tray.
    Import,
    /// Arrow pointing left.
    Back,
    /// Arrow pointing right.
    Next,
    /// Two chevrons and a bar.
    Skip,
    /// Chevron pointing right: a closed section.
    ChevronRight,
    /// Chevron pointing down: an open section.
    ChevronDown,
    /// Ruler with tick marks, for measuring.
    Ruler,
    /// Preparation with its margin ring and handles, for the margin tool.
    Margin,
    /// Occlusal plane with the occlusal and forward arrows, for the orientation step.
    Orient,
    /// Preparation with a tilted arrow rising out of it, for the insertion axis step.
    Axis,
    /// Arrow curving back to the left.
    Undo,
    /// Arrow curving on to the right.
    Redo,
    /// Cube with the face the view looks at filled in.
    View(ViewPreset),
    /// Picture of a workspace for the switcher and its menu.
    Workspace(Workspace),
}

/// Fill and ink of a flat command button in its current state.
pub fn button_colors(
    p: &Palette,
    response: &Response,
    enabled: bool,
    selected: bool,
) -> (Color32, Color32) {
    if !enabled {
        let fill = if selected {
            p.accent_soft.gamma_multiply(0.5)
        } else {
            Color32::TRANSPARENT
        };
        return (fill, p.text_dim.gamma_multiply(0.55));
    }
    if response.is_pointer_button_down_on() {
        (p.press, p.text)
    } else if selected {
        (p.accent_soft, p.selected_ink)
    } else if response.hovered() {
        (p.hover, p.text)
    } else {
        (Color32::TRANSPARENT, p.text)
    }
}

/// Round, frameless icon button.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .circle_filled(rect.center(), size * 0.5, p.hover);
        }
        let ink = if response.hovered() {
            p.text
        } else {
            p.text_dim
        };
        paint_icon(ui.painter(), icon, rect.shrink(size * 0.22), ink, p);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Draws `icon` inside `rect`.
pub fn paint_icon(painter: &egui::Painter, icon: Icon, rect: Rect, ink: Color32, p: Palette) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let stroke = Stroke::new(1.5, ink);
    // Line paths in units of the icon's half size, centred on `c`.
    let points = |pts: &[(f32, f32)]| -> Vec<Pos2> {
        pts.iter().map(|&(x, y)| c + vec2(x, y) * r).collect()
    };
    let open = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(points(pts), stroke));
    };
    let closed = |pts: &[(f32, f32)]| {
        painter.add(Shape::Path(PathShape::closed_line(points(pts), stroke)));
    };
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
            let bite = if ink == p.text { p.hover } else { p.app_bg };
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
        Icon::NewDoc => {
            closed(&[
                (-0.6, -0.9),
                (0.25, -0.9),
                (0.65, -0.5),
                (0.65, 0.9),
                (-0.6, 0.9),
            ]);
            open(&[(0.25, -0.9), (0.25, -0.5), (0.65, -0.5)]);
            open(&[(-0.3, 0.3), (0.3, 0.3)]);
            open(&[(0.0, 0.0), (0.0, 0.6)]);
        }
        Icon::FolderOpen => {
            open(&[
                (0.65, -0.1),
                (0.65, -0.45),
                (-0.2, -0.45),
                (-0.35, -0.7),
                (-0.9, -0.7),
                (-0.9, 0.75),
            ]);
            closed(&[(-0.9, 0.75), (-0.6, -0.1), (0.95, -0.1), (0.65, 0.75)]);
        }
        Icon::Save => {
            closed(&[
                (-0.85, -0.85),
                (0.5, -0.85),
                (0.85, -0.5),
                (0.85, 0.85),
                (-0.85, 0.85),
            ]);
            open(&[(-0.45, -0.85), (-0.45, -0.35), (0.3, -0.35), (0.3, -0.85)]);
            open(&[(-0.5, 0.85), (-0.5, 0.2), (0.5, 0.2), (0.5, 0.85)]);
        }
        Icon::Import => {
            open(&[(0.0, -0.9), (0.0, 0.35)]);
            open(&[(-0.4, -0.05), (0.0, 0.35), (0.4, -0.05)]);
            open(&[(-0.85, 0.2), (-0.85, 0.85), (0.85, 0.85), (0.85, 0.2)]);
        }
        Icon::Back | Icon::Next => {
            let s = if icon == Icon::Back { 1.0 } else { -1.0 };
            open(&[(0.8 * s, 0.0), (-0.8 * s, 0.0)]);
            open(&[(-0.25 * s, -0.55), (-0.8 * s, 0.0), (-0.25 * s, 0.55)]);
        }
        Icon::Skip => {
            open(&[(-0.85, -0.6), (-0.25, 0.0), (-0.85, 0.6)]);
            open(&[(-0.2, -0.6), (0.4, 0.0), (-0.2, 0.6)]);
            open(&[(0.75, -0.65), (0.75, 0.65)]);
        }
        Icon::ChevronRight => open(&[(-0.3, -0.7), (0.4, 0.0), (-0.3, 0.7)]),
        Icon::ChevronDown => open(&[(-0.7, -0.3), (0.0, 0.4), (0.7, -0.3)]),
        Icon::Ruler => {
            // A ruler lying at 45 degrees, ticks of two lengths along its upper edge.
            closed(&[(-0.95, 0.45), (0.45, -0.95), (0.95, -0.45), (-0.45, 0.95)]);
            for (i, len) in [0.3, 0.18, 0.3, 0.18, 0.3].into_iter().enumerate() {
                let t = -0.5 + i as f32 * 0.25;
                let (x, y) = (t - 0.25, -t - 0.25);
                open(&[(x, y), (x + len * 0.7, y + len * 0.7)]);
            }
        }
        Icon::Margin => {
            // A tapered preparation, the margin ring round its base and two handles on it.
            open(&[
                (-0.55, 0.45),
                (-0.4, -0.55),
                (-0.2, -0.8),
                (0.2, -0.8),
                (0.4, -0.55),
                (0.55, 0.45),
            ]);
            let ring: Vec<(f32, f32)> = (0..20)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 20.0;
                    (0.9 * a.cos(), 0.5 + 0.32 * a.sin())
                })
                .collect();
            closed(&ring);
            for (x, y) in [(-0.9, 0.5), (0.64, 0.73)] {
                painter.circle_filled(c + vec2(x, y) * r, r * 0.17, ink);
            }
        }
        Icon::Orient => {
            // A flat plane ring with an arrow rising from its centre and one along it.
            let ring: Vec<(f32, f32)> = (0..20)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 20.0;
                    (0.9 * a.cos(), 0.5 + 0.32 * a.sin())
                })
                .collect();
            closed(&ring);
            open(&[(0.0, 0.5), (0.0, -0.85)]);
            open(&[(-0.3, -0.5), (0.0, -0.85), (0.3, -0.5)]);
            open(&[(0.0, 0.5), (0.7, 0.8)]);
        }
        Icon::Axis => {
            // A tapered preparation and a tilted arrow rising from inside it.
            open(&[(-0.6, 0.9), (-0.45, 0.0), (0.15, 0.0), (0.3, 0.9)]);
            open(&[(-0.15, 0.45), (0.6, -0.85)]);
            open(&[(0.2, -0.75), (0.6, -0.85), (0.65, -0.45)]);
        }
        Icon::Undo | Icon::Redo => {
            // A hook: down the right side of an arc, then back along the top to the arrow tip.
            let s = if icon == Icon::Undo { 1.0 } else { -1.0 };
            let mut hook = vec![(-0.35 * s, 0.75)];
            for i in 0..=8 {
                let a = std::f32::consts::FRAC_PI_2 - i as f32 * std::f32::consts::PI / 8.0;
                hook.push(((0.15 + 0.6 * a.cos()) * s, 0.15 + 0.6 * a.sin()));
            }
            hook.push((-0.75 * s, -0.45));
            open(&hook);
            open(&[(-0.4 * s, -0.8), (-0.75 * s, -0.45), (-0.4 * s, -0.1)]);
        }
        Icon::View(preset) => paint_view_cube(painter, preset, c, r, ink),
        Icon::Workspace(w) => paint_workspace(painter, w, c, r, stroke),
    }
}

/// Workspace pictures, drawn as smooth curves through points in units of the half size.
fn paint_workspace(painter: &egui::Painter, w: Workspace, c: Pos2, r: f32, stroke: Stroke) {
    let points = |pts: &[(f32, f32)]| -> Vec<Pos2> {
        pts.iter().map(|&(x, y)| c + vec2(x, y) * r).collect()
    };
    let smooth_closed = |pts: &[(f32, f32)]| {
        let path = spline(&points(pts), true);
        painter.add(Shape::Path(PathShape::closed_line(path, stroke)));
    };
    let smooth_open = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(spline(&points(pts), false), stroke));
    };
    let polygon = |pts: &[(f32, f32)]| {
        painter.add(Shape::Path(PathShape::closed_line(points(pts), stroke)));
    };
    let segment = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([c + vec2(a.0, a.1) * r, c + vec2(b.0, b.1) * r], stroke);
    };
    // Points on an arch around (0, cy), `t` in degrees from its front (top of the icon).
    let arch = |rx: f32, ry: f32, cy: f32, t: f32| {
        let t = t.to_radians();
        (rx * t.sin(), cy - ry * t.cos())
    };
    match w {
        // Molar crown: three cusps on top, a rounded margin at the bottom.
        Workspace::CrownBridge => smooth_closed(&[
            (-0.6, 0.72),
            (-0.82, 0.15),
            (-0.8, -0.45),
            (-0.55, -0.82),
            (-0.27, -0.55),
            (0.0, -0.78),
            (0.27, -0.55),
            (0.55, -0.82),
            (0.8, -0.45),
            (0.82, 0.15),
            (0.6, 0.72),
            (0.0, 0.84),
        ]),
        // Dental arch standing on a plinth seen slightly from above.
        Workspace::ModelBuilder => {
            polygon(&[
                (-0.92, 0.5),
                (-0.72, 0.3),
                (0.72, 0.3),
                (0.92, 0.5),
                (0.92, 0.88),
                (-0.92, 0.88),
            ]);
            segment((-0.92, 0.5), (0.92, 0.5));
            smooth_open(&[
                (-0.66, 0.3),
                (-0.72, -0.3),
                (-0.42, -0.74),
                (0.0, -0.88),
                (0.42, -0.74),
                (0.72, -0.3),
                (0.66, 0.3),
            ]);
            smooth_open(&[
                (-0.28, 0.3),
                (-0.32, -0.12),
                (0.0, -0.4),
                (0.32, -0.12),
                (0.28, 0.3),
            ]);
        }
        // Crown on an abutment over a threaded, tapered screw.
        Workspace::Implant => {
            smooth_closed(&[
                (-0.44, -0.3),
                (-0.52, -0.66),
                (-0.26, -0.92),
                (0.0, -0.76),
                (0.26, -0.92),
                (0.52, -0.66),
                (0.44, -0.3),
                (0.0, -0.24),
            ]);
            segment((-0.16, -0.25), (-0.16, -0.08));
            segment((0.16, -0.25), (0.16, -0.08));
            polygon(&[
                (-0.32, -0.08),
                (0.32, -0.08),
                (0.2, 0.74),
                (0.0, 0.92),
                (-0.2, 0.74),
            ]);
            for y in [0.12, 0.34, 0.56] {
                let half = 0.32 - (y + 0.08) * 0.14;
                segment((-half, y + 0.07), (half, y - 0.07));
            }
        }
        // Horseshoe shell over the arch, open at the back.
        Workspace::Splint => smooth_closed(&[
            (-0.6, 0.86),
            (-0.86, 0.3),
            (-0.78, -0.42),
            (-0.4, -0.82),
            (0.0, -0.9),
            (0.4, -0.82),
            (0.78, -0.42),
            (0.86, 0.3),
            (0.6, 0.86),
            (0.44, 0.62),
            (0.48, 0.05),
            (0.3, -0.36),
            (0.0, -0.48),
            (-0.3, -0.36),
            (-0.48, 0.05),
            (-0.44, 0.62),
        ]),
        // Gum flange with a row of teeth around the arch.
        Workspace::Denture => {
            let curve = |rx: f32, ry: f32| {
                (-11..=11)
                    .map(|i| arch(rx, ry, 0.12, i as f32 * 11.0))
                    .collect::<Vec<_>>()
            };
            smooth_open(&curve(0.96, 1.02));
            smooth_open(&curve(0.34, 0.42));
            for i in -3..=3 {
                let (x, y) = arch(0.66, 0.72, 0.12, i as f32 * 33.0);
                painter.circle_stroke(c + vec2(x, y) * r, 0.15 * r, stroke);
            }
        }
        // Lips around a smile with the upper teeth showing.
        Workspace::SmileDesign => {
            smooth_closed(&[
                (-0.96, -0.12),
                (-0.56, -0.5),
                (-0.18, -0.6),
                (0.0, -0.48),
                (0.18, -0.6),
                (0.56, -0.5),
                (0.96, -0.12),
                (0.56, 0.52),
                (0.0, 0.74),
                (-0.56, 0.52),
            ]);
            smooth_closed(&[
                (-0.74, -0.1),
                (-0.36, -0.22),
                (0.0, -0.2),
                (0.36, -0.22),
                (0.74, -0.1),
                (0.36, 0.3),
                (0.0, 0.38),
                (-0.36, 0.3),
            ]);
            smooth_open(&[
                (-0.62, 0.0),
                (-0.3, 0.12),
                (0.0, 0.16),
                (0.3, 0.12),
                (0.62, 0.0),
            ]);
            for x in [-0.42, -0.14, 0.14, 0.42_f32] {
                let top = -0.21 + x.abs() * 0.05;
                let bottom = 0.16 - x * x * 0.4;
                segment((x, top), (x, bottom));
            }
        }
    }
}

/// Catmull-Rom curve through `pts`, sampled into a polyline.
fn spline(pts: &[Pos2], closed: bool) -> Vec<Pos2> {
    const STEPS: usize = 6;
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let at = |i: isize| -> Pos2 {
        if closed {
            pts[i.rem_euclid(n as isize) as usize]
        } else {
            pts[i.clamp(0, n as isize - 1) as usize]
        }
    };
    let segments = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(segments * STEPS + 1);
    for s in 0..segments as isize {
        let (p0, p1, p2, p3) = (at(s - 1), at(s), at(s + 1), at(s + 2));
        for k in 0..STEPS {
            let t = k as f32 / STEPS as f32;
            let (t2, t3) = (t * t, t * t * t);
            let v = (p1.to_vec2() * 2.0
                + (p2 - p0) * t
                + (p0.to_vec2() * 2.0 - p1.to_vec2() * 5.0 + p2.to_vec2() * 4.0 - p3.to_vec2())
                    * t2
                + (p3.to_vec2() - p0.to_vec2() + p1.to_vec2() * 3.0 - p2.to_vec2() * 3.0) * t3)
                * 0.5;
            out.push(v.to_pos2());
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    out
}

/// Oblique cube with the front face at the lower left and the face `preset` looks at filled.
fn paint_view_cube(painter: &egui::Painter, preset: ViewPreset, c: Pos2, r: f32, ink: Color32) {
    let at = |x: f32, y: f32| c + vec2(x, y) * r;
    // Front square, then the back square offset up and to the right.
    let ftl = at(-0.85, -0.45);
    let ftr = at(0.45, -0.45);
    let fbr = at(0.45, 0.85);
    let fbl = at(-0.85, 0.85);
    let btl = at(-0.45, -0.85);
    let btr = at(0.85, -0.85);
    let bbr = at(0.85, 0.45);
    let bbl = at(-0.45, 0.45);
    let face = match preset {
        ViewPreset::Occlusal => [ftl, ftr, btr, btl],
        ViewPreset::Bottom => [fbl, fbr, bbr, bbl],
        ViewPreset::Facial => [ftl, ftr, fbr, fbl],
        ViewPreset::Lingual => [btl, btr, bbr, bbl],
        ViewPreset::Left => [ftl, btl, bbl, fbl],
        ViewPreset::Right => [ftr, btr, bbr, fbr],
    };
    painter.add(Shape::convex_polygon(
        face.to_vec(),
        ink.gamma_multiply(0.45),
        Stroke::NONE,
    ));
    let hidden = Stroke::new(1.0, ink.gamma_multiply(0.4));
    for end in [btl, bbr, fbl] {
        painter.line_segment([bbl, end], hidden);
    }
    let stroke = Stroke::new(1.5, ink);
    painter.add(Shape::Path(PathShape::closed_line(
        vec![ftl, ftr, fbr, fbl],
        stroke,
    )));
    painter.add(Shape::line(vec![ftl, btl, btr, bbr, fbr], stroke));
    painter.line_segment([ftr, btr], stroke);
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
