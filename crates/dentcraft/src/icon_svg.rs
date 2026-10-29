//! Icon renderer, modelled on SolidCraft's (`crates/ui-egui/src/icon_svg.rs`, Apache-2.0): the
//! SVG masters in `assets/icons` are embedded at build time and rasterised at the pixel size they
//! are shown, so they stay crisp at any display scale.
//!
//! The masters are monochrome and draw in `currentColor`. Each is rasterised once per pixel size
//! as a white coverage mask and cached as an egui texture; drawing it tinted with a colour gives
//! the theme's text, dim or accent ink without a texture per colour.

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

/// `svg` rasterised to a `px` square white coverage mask.
pub fn rasterise(svg: &str, px: u32) -> Result<ColorImage, String> {
    let white = svg.replace("currentColor", "#ffffff");
    egui_extras::image::load_svg_bytes_with_size(
        white.as_bytes(),
        egui::SizeHint::Size {
            width: px,
            height: px,
            maintain_aspect_ratio: true,
        },
        &Default::default(),
    )
}

/// Textures already rasterised, by icon and pixel size.
#[derive(Clone, Default)]
struct Cache(HashMap<(Icon, u32), TextureHandle>);

/// The texture of `icon` at `px` pixels, rasterised on first use.
fn texture(ctx: &Context, icon: Icon, px: u32) -> Option<TextureHandle> {
    let key = (icon, px);
    let id = Id::unique("dentcraft-icon-textures");
    if let Some(handle) = ctx.data(|d| d.get_temp::<Cache>(id).and_then(|c| c.0.get(&key).cloned()))
    {
        return Some(handle);
    }
    let image = rasterise(source(icon), px)
        .inspect_err(|err| eprintln!("icon {icon:?} does not render: {err}"))
        .ok()?;
    let handle = ctx.load_texture(format!("icon {icon:?} {px}"), image, Default::default());
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Cache>(id)
            .0
            .insert(key, handle.clone());
    });
    Some(handle)
}

/// Draws `icon` as large as fits in `rect`, centred, tinted `ink` and aligned to the pixel grid.
pub fn paint(painter: &egui::Painter, icon: Icon, rect: Rect, ink: Color32) {
    let ppp = painter.pixels_per_point();
    let px = (rect.width().min(rect.height()) * ppp).round().max(1.0);
    let side = px / ppp;
    let min = rect.center() - vec2(side, side) * 0.5;
    let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
    if let Some(texture) = texture(painter.ctx(), icon, px as u32) {
        painter.image(
            texture.id(),
            Rect::from_min_size(min, vec2(side, side)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            ink,
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
            Icon::Fit,
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

    #[test]
    fn every_icon_rasterises_to_a_visible_mask_at_each_size() {
        for icon in every_icon() {
            for px in [16, 24, 32] {
                let image = rasterise(source(icon), px)
                    .unwrap_or_else(|err| panic!("{icon:?} at {px} px: {err}"));
                assert_eq!(image.size, [px as usize; 2], "{icon:?} at {px} px");
                let covered = image.pixels.iter().filter(|c| c.a() > 0).count();
                // Something is drawn, and it is not a filled square.
                assert!(
                    covered > 0 && covered < image.pixels.len(),
                    "{icon:?} at {px} px covers {covered} pixels"
                );
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

    #[test]
    fn icons_have_distinct_masters() {
        let icons = every_icon();
        for (i, a) in icons.iter().enumerate() {
            for b in &icons[i + 1..] {
                assert_ne!(source(*a), source(*b), "{a:?} and {b:?} draw the same");
            }
        }
    }
}
