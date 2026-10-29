//! Icon renderer, modelled on SolidCraft's (`crates/ui-egui/src/icon_svg.rs`, Apache-2.0): the
//! SVG masters in `assets/icons` are embedded at build time and rasterised at the pixel size they
//! are shown, so they stay crisp at any display scale.
//!
//! Two kinds of master:
//! - Monochrome glyphs (eye, chevrons, close, undo...) draw in `currentColor`. Each is rasterised
//!   once per pixel size as a white coverage mask; drawing it tinted with a colour gives the
//!   theme's text, dim or accent ink without a texture per colour.
//! - Coloured command icons use SolidCraft's isometric palette: grey passive faces, blue active
//!   faces, a dark outline. They are drawn in their own colours, rasterised once per pixel size
//!   and theme: in the dark theme every palette colour is swapped for its dark value, as
//!   SolidCraft's `assets/icons/ui/design/palette.toml` does.

use std::collections::HashMap;

use dentcraft_render::ViewPreset;
use eframe::egui::{self, Color32, ColorImage, Context, Id, Rect, TextureHandle, pos2, vec2};

use crate::widgets::Icon;
use crate::workspace::Workspace;

/// The SVG master of `icon`.
pub fn source(icon: Icon) -> &'static str {
    macro_rules! svg {
        ($name:literal) => {
            include_str!(concat!("../assets/icons/", $name, ".svg"))
        };
    }
    match icon {
        Icon::Eye => svg!("eye"),
        Icon::EyeOff => svg!("eye_off"),
        Icon::Close => svg!("close"),
        Icon::Sun => svg!("sun"),
        Icon::Moon => svg!("moon"),
        Icon::Fit => svg!("fit"),
        Icon::FitTooth => svg!("fit_tooth"),
        Icon::UpperJaw => svg!("jaw_upper"),
        Icon::LowerJaw => svg!("jaw_lower"),
        Icon::Plus => svg!("plus"),
        Icon::Check => svg!("check"),
        Icon::Warning => svg!("warning"),
        Icon::NewDoc => svg!("new"),
        Icon::FolderOpen => svg!("folder"),
        Icon::Save => svg!("save"),
        Icon::Import => svg!("import_tray"),
        Icon::Export => svg!("export_tray"),
        Icon::Back => svg!("back"),
        Icon::Next => svg!("next"),
        Icon::Skip => svg!("skip"),
        Icon::ChevronRight => svg!("chevron_right"),
        Icon::ChevronDown => svg!("chevron_down"),
        Icon::Ruler => svg!("measure"),
        Icon::Margin => svg!("margin"),
        Icon::Orient => svg!("orient"),
        Icon::Axis => svg!("axis"),
        Icon::Inner => svg!("inner"),
        Icon::Anatomy => svg!("anatomy"),
        Icon::Contacts => svg!("contacts"),
        Icon::Undo => svg!("undo"),
        Icon::Redo => svg!("redo"),
        Icon::View(preset) => match preset {
            ViewPreset::Occlusal => svg!("view_occlusal"),
            ViewPreset::Facial => svg!("view_facial"),
            ViewPreset::Lingual => svg!("view_lingual"),
            ViewPreset::Left => svg!("view_left"),
            ViewPreset::Right => svg!("view_right"),
            ViewPreset::Bottom => svg!("view_bottom"),
        },
        Icon::Workspace(w) => match w {
            Workspace::CrownBridge => svg!("ws_crown_bridge"),
            Workspace::ModelBuilder => svg!("ws_model_builder"),
            Workspace::Implant => svg!("ws_implant"),
            Workspace::Splint => svg!("ws_splint"),
            Workspace::Denture => svg!("ws_denture"),
            Workspace::SmileDesign => svg!("ws_smile"),
        },
    }
}

/// Whether `icon` is drawn in its own colours rather than tinted.
pub fn coloured(icon: Icon) -> bool {
    !matches!(
        icon,
        Icon::Eye
            | Icon::EyeOff
            | Icon::Close
            | Icon::Sun
            | Icon::Moon
            | Icon::Plus
            | Icon::Check
            | Icon::Warning
            | Icon::ChevronRight
            | Icon::ChevronDown
            | Icon::Undo
            | Icon::Redo
    )
}

/// The colours of SolidCraft's icon palette (`palette.toml`, Apache-2.0) as the masters write
/// them, with their light and dark values. In the dark theme the faces darken and keep their
/// shading order, outlines over a face turn near-black and line art drawn straight on the
/// background (`currentColor`) turns light.
const PALETTE: [(&str, &str, &str); 9] = [
    ("#34465b", "#34465b", "#1f2021"),      // outline over a face
    ("currentColor", "#34465b", "#dbe4ee"), // line art on the background
    ("#e2e8ef", "#e2e8ef", "#8da3c3"),      // passive, lightest face
    ("#b8c3cf", "#b8c3cf", "#6d81a2"),      // passive, middle face
    ("#8e9faf", "#8e9faf", "#546782"),      // passive, darkest face
    ("#6f879c", "#6f879c", "#8093a4"),      // guide
    ("#b9ddf8", "#b9ddf8", "#4ea1dc"),      // active, palest face
    ("#328bd3", "#328bd3", "#3287d1"),      // active, middle face
    ("#236ead", "#236ead", "#2976b8"),      // active, strongest: faces, arrows, accents
];

/// A coloured master with its palette colours set for the dark or light theme.
pub fn themed(svg: &str, dark: bool) -> String {
    let mut out = svg.to_owned();
    for (master, light, dark_value) in PALETTE {
        out = out.replace(master, if dark { dark_value } else { light });
    }
    out
}

/// `icon` rasterised to a `px` square: a white coverage mask for a monochrome icon, its own
/// colours in the `dark` or light theme for a coloured one.
pub fn rasterise(icon: Icon, px: u32, dark: bool) -> Result<ColorImage, String> {
    let svg = if coloured(icon) {
        themed(source(icon), dark)
    } else {
        source(icon).replace("currentColor", "#ffffff")
    };
    egui_extras::image::load_svg_bytes_with_size(
        svg.as_bytes(),
        egui::SizeHint::Size {
            width: px,
            height: px,
            maintain_aspect_ratio: true,
        },
        &Default::default(),
    )
}

/// Textures already rasterised, by icon, pixel size and theme (dark or not).
#[derive(Clone, Default)]
struct Cache(HashMap<(Icon, u32, bool), TextureHandle>);

/// The texture of `icon` at `px` pixels in the current theme, rasterised on first use.
fn texture(ctx: &Context, icon: Icon, px: u32) -> Option<TextureHandle> {
    // A mask is the same in either theme.
    let dark = coloured(icon) && ctx.theme() == egui::Theme::Dark;
    let key = (icon, px, dark);
    let id = Id::unique("dentcraft-icon-textures");
    if let Some(handle) = ctx.data(|d| d.get_temp::<Cache>(id).and_then(|c| c.0.get(&key).cloned()))
    {
        return Some(handle);
    }
    let image = rasterise(icon, px, dark)
        .inspect_err(|err| eprintln!("icon {icon:?} does not render: {err}"))
        .ok()?;
    let name = format!("icon {icon:?} {px}{}", if dark { " dark" } else { "" });
    let handle = ctx.load_texture(name, image, Default::default());
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Cache>(id)
            .0
            .insert(key, handle.clone());
    });
    Some(handle)
}

/// Draws `icon` as large as fits in `rect`, centred and aligned to the pixel grid: a monochrome
/// icon tinted `ink`, a coloured one in its own colours at the opacity of `ink`, so a disabled
/// command's faded ink fades its icon too.
pub fn paint(painter: &egui::Painter, icon: Icon, rect: Rect, ink: Color32) {
    let ppp = painter.pixels_per_point();
    let px = (rect.width().min(rect.height()) * ppp).round().max(1.0);
    let side = px / ppp;
    let min = rect.center() - vec2(side, side) * 0.5;
    let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
    let tint = if coloured(icon) {
        Color32::WHITE.gamma_multiply(f32::from(ink.a()) / 255.0)
    } else {
        ink
    };
    if let Some(texture) = texture(painter.ctx(), icon, px as u32) {
        painter.image(
            texture.id(),
            Rect::from_min_size(min, vec2(side, side)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            tint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_icon() -> Vec<Icon> {
        let mut icons = vec![
            Icon::Eye,
            Icon::EyeOff,
            Icon::Close,
            Icon::Sun,
            Icon::Moon,
            Icon::Plus,
            Icon::Check,
            Icon::Warning,
            Icon::NewDoc,
            Icon::FolderOpen,
            Icon::Save,
            Icon::Import,
            Icon::Back,
            Icon::Next,
            Icon::Skip,
            Icon::ChevronRight,
            Icon::ChevronDown,
            Icon::Ruler,
            Icon::Margin,
            Icon::Orient,
            Icon::Axis,
            Icon::Inner,
            Icon::Anatomy,
            Icon::Contacts,
            Icon::Export,
            Icon::Undo,
            Icon::Redo,
        ];
        icons.extend(ViewPreset::ALL.map(Icon::View));
        icons.extend(Workspace::ALL.map(Icon::Workspace));
        icons
    }

    fn every_icon_with_extras() -> Vec<Icon> {
        let mut icons = every_icon();
        icons.extend([Icon::Fit, Icon::FitTooth, Icon::UpperJaw, Icon::LowerJaw]);
        icons
    }

    #[test]
    fn every_icon_rasterises_visibly_at_each_size_in_both_themes() {
        for icon in every_icon_with_extras() {
            for px in [16, 24, 32] {
                for dark in [false, true] {
                    let image = rasterise(icon, px, dark)
                        .unwrap_or_else(|err| panic!("{icon:?} at {px} px: {err}"));
                    assert_eq!(image.size, [px as usize; 2], "{icon:?} at {px} px");
                    let covered = image.pixels.iter().filter(|c| c.a() > 0).count();
                    // Something is drawn, and it is not a filled square.
                    assert!(
                        covered > 0 && covered < image.pixels.len(),
                        "{icon:?} at {px} px covers {covered} pixels"
                    );
                    if !coloured(icon) {
                        // White masks only, so a tint gives the ink exactly.
                        assert!(
                            image
                                .pixels
                                .iter()
                                .all(|c| c.r() == c.a() && c.g() == c.a() && c.b() == c.a()),
                            "{icon:?} has colour of its own"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn coloured_icons_keep_more_than_one_hue() {
        for icon in every_icon_with_extras()
            .into_iter()
            .filter(|i| coloured(*i))
        {
            for dark in [false, true] {
                let image = rasterise(icon, 32, dark).expect("renders");
                let opaque = || image.pixels.iter().filter(|c| c.a() == 255);
                // Blue, from the active faces or accents...
                let blue = opaque()
                    .filter(|c| i32::from(c.b()) - i32::from(c.r()) > 60)
                    .count();
                // ...beside the grey-blue faces or outline, which are far less saturated.
                let muted = opaque()
                    .filter(|c| i32::from(c.b()) - i32::from(c.r()) < 45)
                    .count();
                assert!(
                    blue > 4 && muted > 4,
                    "{icon:?} ({}) has {blue} blue and {muted} muted pixels",
                    if dark { "dark" } else { "light" }
                );
            }
        }
    }

    #[test]
    fn coloured_masters_use_only_the_palette_and_swap_it_in_the_dark_theme() {
        for icon in every_icon_with_extras()
            .into_iter()
            .filter(|i| coloured(*i))
        {
            let src = source(icon);
            assert!(
                !src.contains("#ffffff"),
                "{icon:?}: cut gaps with a mask, white would show in the dark theme"
            );
            let hexes = src.match_indices("\"#").map(|(i, _)| &src[i + 1..i + 8]);
            for hex in hexes {
                assert!(
                    PALETTE.iter().any(|(master, _, _)| *master == hex),
                    "{icon:?} uses {hex}, which is not a palette colour"
                );
            }
            let dark = themed(src, true);
            for (master, _, _) in PALETTE {
                assert!(!dark.contains(master), "{icon:?} keeps {master} when dark");
            }
        }
    }

    #[test]
    fn icons_have_distinct_masters() {
        let icons = every_icon_with_extras();
        for (i, a) in icons.iter().enumerate() {
            for b in &icons[i + 1..] {
                assert_ne!(source(*a), source(*b), "{a:?} and {b:?} draw the same");
            }
        }
    }
}
