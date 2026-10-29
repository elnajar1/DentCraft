//! Small branded widgets: small and command buttons, and SVG icons.

use dentcraft_render::ViewPreset;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, Sense, Stroke,
    TextStyle, Ui, Vec2, WidgetText, vec2,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// Preparation under a cap that follows it at a gap, for the inner surface step.
    Inner,
    /// A molar crown with cusps, for the anatomy step.
    Anatomy,
    /// Arrow rising out of a tray, for writing files out.
    Export,
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
        paint_icon(ui.painter(), icon, rect.shrink(size * 0.22), ink);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Draws `icon` as large as fits in `rect`, tinted `ink`.
pub fn paint_icon(painter: &egui::Painter, icon: Icon, rect: Rect, ink: Color32) {
    crate::icon_svg::paint(painter, icon, rect, ink);
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
