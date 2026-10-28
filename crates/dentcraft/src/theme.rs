//! Brand look: palettes, egui styles and the viewport background, for the dark and light themes.

use dentcraft_render::RenderOptions;
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme,
    Visuals, style::Selection, vec2,
};

/// Corner radius of inputs (text fields, combo boxes, swatches).
pub const RADIUS_INPUT: u8 = 6;
/// Corner radius of panels and cards.
pub const RADIUS_PANEL: u8 = 12;
/// Pill-shaped buttons.
pub const RADIUS_PILL: u8 = 255;

/// Text sizes in points.
pub const SIZE_BODY: f32 = 14.0;
pub const SIZE_HEADING: f32 = 18.0;
pub const SIZE_SMALL: f32 = 12.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeKind {
    #[default]
    Dark,
    Light,
}

impl ThemeKind {
    /// Value stored in the app's persistent storage.
    pub fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    pub fn egui_theme(self) -> Theme {
        match self {
            Self::Dark => Theme::Dark,
            Self::Light => Theme::Light,
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette::DARK,
            Self::Light => Palette::LIGHT,
        }
    }
}

/// Colour tokens of one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Window and panel background.
    pub surface: Color32,
    /// Cards, buttons and other raised elements.
    pub raised: Color32,
    /// Raised element under the pointer.
    pub raised_hover: Color32,
    /// Hairlines and borders.
    pub line: Color32,
    pub ink: Color32,
    pub muted: Color32,
    /// Primary action fill.
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text on `accent`.
    pub on_accent: Color32,
    /// Brand gold for small marks (selection, active step, hover point).
    pub gold: Color32,
    /// Text field background.
    pub field: Color32,
    pub danger: Color32,
    /// Viewport background gradient (sRGB).
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Palette {
    pub const DARK: Palette = Palette {
        surface: hex(0x151514),
        raised: hex(0x292929),
        raised_hover: hex(0x333331),
        line: hex(0x3b3a37),
        ink: hex(0xf4f2ee),
        muted: hex(0xb3ada3),
        accent: hex(0xcba360),
        accent_hover: hex(0xd9b878),
        on_accent: hex(0x121212),
        gold: hex(0xcba360),
        field: hex(0x1e1e1d),
        danger: hex(0xe07a6b),
        viewport_top: hex(0x3a3833),
        viewport_bottom: hex(0x161615),
    };

    pub const LIGHT: Palette = Palette {
        surface: hex(0xffffff),
        raised: hex(0xf6f4f0),
        raised_hover: hex(0xeeebe4),
        line: hex(0xe3dfd6),
        ink: hex(0x222222),
        muted: hex(0x5f5a51),
        accent: hex(0x121212),
        accent_hover: hex(0x3a3936),
        on_accent: hex(0xffffff),
        gold: hex(0xcba360),
        field: hex(0xffffff),
        danger: hex(0xb3412f),
        viewport_top: hex(0xfbfaf8),
        viewport_bottom: hex(0xd9d4ca),
    };

    /// Background and back-face look of the 3D viewport.
    pub fn render_options(&self, kind: ThemeKind) -> RenderOptions {
        let rgb = |c: Color32| [c.r(), c.g(), c.b()].map(|v| v as f32 / 255.0);
        let base = match kind {
            ThemeKind::Dark => RenderOptions::dark(),
            ThemeKind::Light => RenderOptions::light(),
        };
        RenderOptions {
            background_top: rgb(self.viewport_top),
            background_bottom: rgb(self.viewport_bottom),
            vignette: match kind {
                ThemeKind::Dark => 0.45,
                ThemeKind::Light => 0.18,
            },
            ..base
        }
    }
}

/// Installs both themes' styles and selects `kind`.
pub fn install(ctx: &egui::Context, kind: ThemeKind) {
    for k in [ThemeKind::Dark, ThemeKind::Light] {
        ctx.set_style_of(k.egui_theme(), style(k));
    }
    ctx.set_theme(kind.egui_theme());
}

/// The complete egui style of a theme.
pub fn style(kind: ThemeKind) -> egui::Style {
    let p = kind.palette();
    let mut style = egui::Style {
        visuals: visuals(kind, &p),
        text_styles: [
            (
                TextStyle::Small,
                FontId::new(SIZE_SMALL, FontFamily::Proportional),
            ),
            (
                TextStyle::Body,
                FontId::new(SIZE_BODY, FontFamily::Proportional),
            ),
            (
                TextStyle::Button,
                FontId::new(SIZE_BODY, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(SIZE_HEADING, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into(),
        ..Default::default()
    };

    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 8.0);
    s.button_padding = vec2(12.0, 4.0);
    s.interact_size = vec2(40.0, 28.0);
    s.window_margin = Margin::same(12);
    s.menu_margin = Margin::same(8);
    s.combo_height = 320.0;
    s.slider_rail_height = 4.0;
    s.icon_width = 16.0;

    style
}

fn visuals(kind: ThemeKind, p: &Palette) -> Visuals {
    let mut v = match kind {
        ThemeKind::Dark => Visuals::dark(),
        ThemeKind::Light => Visuals::light(),
    };
    let input = CornerRadius::same(RADIUS_INPUT);

    v.panel_fill = p.surface;
    v.window_fill = p.raised;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.window_corner_radius = CornerRadius::same(RADIUS_PANEL);
    v.menu_corner_radius = CornerRadius::same(10);
    v.faint_bg_color = p.raised;
    v.extreme_bg_color = p.field;
    v.text_edit_bg_color = Some(p.field);
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.gold;
    v.error_fg_color = p.danger;
    v.slider_trailing_fill = true;
    v.striped = false;
    v.indent_has_left_vline = false;
    let shadow_alpha = match kind {
        ThemeKind::Dark => 90,
        ThemeKind::Light => 28,
    };
    let shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.selection = Selection {
        bg_fill: p.gold.gamma_multiply(match kind {
            ThemeKind::Dark => 0.45,
            ThemeKind::Light => 0.35,
        }),
        stroke: Stroke::new(1.0, p.gold),
    };

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.ink);
    w.noninteractive.corner_radius = input;

    w.inactive.bg_fill = p.line;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.line);
    w.inactive.fg_stroke = Stroke::new(1.0, p.ink);
    w.inactive.corner_radius = input;

    w.hovered.bg_fill = p.muted;
    w.hovered.weak_bg_fill = p.raised_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.muted.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.5, p.ink);
    w.hovered.corner_radius = input;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.gold;
    w.active.weak_bg_fill = p.raised_hover;
    w.active.bg_stroke = Stroke::new(1.0, p.gold);
    w.active.fg_stroke = Stroke::new(1.5, p.ink);
    w.active.corner_radius = input;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised_hover;
    w.open.bg_stroke = Stroke::new(1.0, p.line);
    w.open.fg_stroke = Stroke::new(1.0, p.ink);
    w.open.corner_radius = input;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_key_round_trips() {
        for kind in [ThemeKind::Dark, ThemeKind::Light] {
            assert_eq!(ThemeKind::from_key(kind.key()), Some(kind));
            assert_eq!(kind.toggled().toggled(), kind);
        }
        assert_eq!(ThemeKind::from_key("sepia"), None);
    }

    #[test]
    fn styles_use_the_brand_tokens_and_readable_sizes() {
        let dark = style(ThemeKind::Dark);
        assert!(dark.visuals.dark_mode);
        assert_eq!(dark.visuals.panel_fill, hex(0x151514));
        assert_eq!(dark.text_styles[&TextStyle::Body].size, 14.0);
        assert_eq!(dark.text_styles[&TextStyle::Heading].size, 18.0);
        assert_eq!(dark.text_styles[&TextStyle::Small].size, 12.0);

        let light = style(ThemeKind::Light);
        assert!(!light.visuals.dark_mode);
        assert_eq!(light.visuals.panel_fill, Color32::WHITE);
        assert_eq!(
            light.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(6)
        );
    }
}
