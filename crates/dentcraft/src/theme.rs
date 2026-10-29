//! App look: palettes, egui styles and the viewport background, for the dark and light themes.
//!
//! The colour tokens, type scale and radii are adapted from SolidCraft
//! (https://github.com/tangency-labs/solidcraft, Apache-2.0); see NOTICE.

use dentcraft_render::RenderOptions;
use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme,
    Visuals, style::Selection, vec2,
};

/// Corner radii, from SolidCraft's scale (sm 4, md 6, lg 8).
pub const RADIUS_SM: u8 = 4;
pub const RADIUS_MD: u8 = 6;
pub const RADIUS_LG: u8 = 8;

/// Text sizes in points, from SolidCraft's type scale.
pub const SIZE_SMALL: f32 = 11.0;
pub const SIZE_BODY: f32 = 12.5;
pub const SIZE_HEADING: f32 = 14.0;

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

/// Colour tokens of one theme, adapted from SolidCraft's UI palettes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// App bar and the area behind panels.
    pub app_bg: Color32,
    /// Panels, cards and popups.
    pub panel: Color32,
    /// Section title rows, the status bar and secondary buttons.
    pub panel_header: Color32,
    /// Hairlines and borders.
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// Blue accent: active step, selections, focus and bridge connectors.
    pub accent: Color32,
    /// Fill behind selected and toggled-on items.
    pub accent_soft: Color32,
    /// Text and icons on `accent_soft`.
    pub selected_ink: Color32,
    /// Element under the pointer.
    pub hover: Color32,
    /// Element being pressed.
    pub press: Color32,
    /// Text field background.
    pub field: Color32,
    /// Filled primary action button.
    pub primary: Color32,
    pub primary_hover: Color32,
    /// Text on `primary`.
    pub on_primary: Color32,
    /// Text on an `accent` fill, such as the Current badge of the workspace menu.
    pub on_accent: Color32,
    pub danger: Color32,
    /// Viewport background gradient (sRGB).
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
    /// Tint of the restoration's inner surface over the die.
    pub inner_surface: Color32,
    /// The designed crown: a light tooth colour.
    pub crown: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Palette {
    pub const DARK: Palette = Palette {
        app_bg: rgb(31, 35, 43),
        panel: rgb(39, 43, 52),
        panel_header: rgb(47, 52, 62),
        border: rgb(63, 69, 82),
        text: rgb(222, 226, 232),
        // SolidCraft uses 148,155,168; one step lighter so small text on `panel_header` reaches 4.5:1.
        text_dim: rgb(156, 163, 176),
        accent: rgb(58, 136, 238),
        accent_soft: rgb(42, 70, 110),
        selected_ink: rgb(156, 199, 245),
        hover: rgb(58, 65, 80),
        press: rgb(70, 78, 96),
        field: rgb(47, 52, 62),
        primary: rgb(30, 80, 128),
        primary_hover: rgb(46, 99, 149),
        on_primary: Color32::WHITE,
        on_accent: Color32::WHITE,
        danger: rgb(236, 120, 108),
        viewport_top: rgb(46, 52, 64),
        viewport_bottom: rgb(46, 52, 64),
        inner_surface: rgb(72, 200, 176),
        crown: rgb(238, 230, 212),
    };

    pub const LIGHT: Palette = Palette {
        app_bg: rgb(247, 249, 251),
        panel: rgb(250, 251, 252),
        panel_header: rgb(236, 238, 242),
        border: rgb(208, 212, 220),
        text: rgb(38, 42, 50),
        // SolidCraft uses 110,116,128; one step darker so small text on `panel_header` reaches 4.5:1.
        text_dim: rgb(100, 106, 118),
        accent: rgb(38, 120, 218),
        accent_soft: rgb(214, 230, 250),
        selected_ink: rgb(26, 80, 127),
        hover: rgb(226, 234, 246),
        press: rgb(220, 226, 234),
        field: rgb(255, 255, 255),
        primary: rgb(35, 110, 173),
        primary_hover: rgb(43, 118, 180),
        on_primary: Color32::WHITE,
        on_accent: Color32::WHITE,
        danger: rgb(184, 58, 44),
        viewport_top: rgb(240, 243, 247),
        viewport_bottom: rgb(203, 210, 221),
        inner_surface: rgb(18, 150, 128),
        crown: rgb(236, 226, 204),
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
                ThemeKind::Dark => 0.22,
                ThemeKind::Light => 0.06,
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
    let font = |size| FontId::new(size, FontFamily::Proportional);
    let mut style = egui::Style {
        visuals: visuals(kind, &p),
        text_styles: [
            (TextStyle::Small, font(SIZE_SMALL)),
            (TextStyle::Body, font(SIZE_BODY)),
            (TextStyle::Button, font(SIZE_BODY)),
            (TextStyle::Heading, font(SIZE_HEADING)),
            (
                TextStyle::Monospace,
                FontId::new(SIZE_SMALL, FontFamily::Monospace),
            ),
        ]
        .into(),
        ..Default::default()
    };

    let s = &mut style.spacing;
    s.item_spacing = vec2(6.0, 4.0);
    s.button_padding = vec2(8.0, 3.0);
    s.interact_size = vec2(32.0, 22.0);
    s.window_margin = Margin::same(10);
    s.menu_margin = Margin::same(4);
    s.combo_height = 320.0;
    s.slider_rail_height = 4.0;
    s.icon_width = 14.0;

    style
}

fn visuals(kind: ThemeKind, p: &Palette) -> Visuals {
    let mut v = match kind {
        ThemeKind::Dark => Visuals::dark(),
        ThemeKind::Light => Visuals::light(),
    };
    let radius = CornerRadius::same(RADIUS_SM);

    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(RADIUS_LG);
    v.menu_corner_radius = CornerRadius::same(RADIUS_MD);
    v.faint_bg_color = p.panel_header;
    v.extreme_bg_color = p.field;
    v.text_edit_bg_color = Some(p.field);
    v.code_bg_color = p.panel_header;
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.danger;
    v.slider_trailing_fill = true;
    v.striped = false;
    v.indent_has_left_vline = false;
    let shadow_alpha = match kind {
        ThemeKind::Dark => 80,
        ThemeKind::Light => 24,
    };
    let shadow = Shadow {
        offset: [0, 2],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.selection = Selection {
        bg_fill: p.accent_soft,
        stroke: Stroke::new(1.0, p.selected_ink),
    };

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.border;
    w.inactive.weak_bg_fill = p.field;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.text_dim;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.press;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.panel_header;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    v
}

/// WCAG contrast ratio of two opaque colours.
#[cfg(test)]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let lum = |c: Color32| {
        let ch = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    };
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
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
    fn styles_use_the_solidcraft_tokens_and_compact_sizes() {
        let dark = style(ThemeKind::Dark);
        assert!(dark.visuals.dark_mode);
        assert_eq!(dark.visuals.panel_fill, rgb(39, 43, 52));
        assert_eq!(dark.visuals.selection.bg_fill, rgb(42, 70, 110));
        assert_eq!(dark.text_styles[&TextStyle::Body].size, 12.5);
        assert_eq!(dark.text_styles[&TextStyle::Heading].size, 14.0);
        assert_eq!(dark.text_styles[&TextStyle::Small].size, 11.0);

        let light = style(ThemeKind::Light);
        assert!(!light.visuals.dark_mode);
        assert_eq!(light.visuals.panel_fill, rgb(250, 251, 252));
        assert_eq!(
            light.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(4)
        );
    }

    #[test]
    fn text_reads_at_4_5_to_1_on_every_surface() {
        for p in [Palette::DARK, Palette::LIGHT] {
            for bg in [p.app_bg, p.panel, p.panel_header, p.field] {
                for ink in [p.text, p.text_dim] {
                    assert!(contrast(ink, bg) >= 4.5, "{ink:?} on {bg:?}");
                }
            }
            assert!(contrast(p.selected_ink, p.accent_soft) >= 4.5);
            assert!(contrast(p.on_primary, p.primary) >= 4.5);
            assert!(contrast(p.on_primary, p.primary_hover) >= 4.5);
        }
    }

    #[test]
    fn the_palette_has_no_gold() {
        let gold = rgb(0xcb, 0xa3, 0x60);
        for p in [Palette::DARK, Palette::LIGHT] {
            assert_ne!(p.accent, gold);
            assert!(p.accent.b() > p.accent.r(), "accent is blue");
        }
    }
}
