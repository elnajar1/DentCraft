//! Command line of the desktop application.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use dentcraft_render::ViewPreset;

use crate::theme::ThemeKind;

/// Default window size, in logical points.
pub const DEFAULT_SIZE: [u32; 2] = [1600, 1000];

#[derive(Debug, Parser)]
#[command(name = "dentcraft", version, about = "DentCraft — open dental CAD")]
pub struct Args {
    /// Scan files to open (STL, PLY or OBJ).
    pub files: Vec<PathBuf>,

    /// Initial view: occlusal, facial, lingual, left, right or bottom.
    #[arg(long, value_parser = parse_view)]
    pub view: Option<ViewPreset>,

    /// Colour theme (overrides the saved choice).
    #[arg(long, value_enum)]
    pub theme: Option<ThemeArg>,

    /// Render the window once everything has loaded, write it to this PNG and exit.
    #[arg(long, value_name = "OUT.png")]
    pub screenshot: Option<PathBuf>,

    /// Window size in points, e.g. 1600x1000.
    #[arg(long, value_name = "WxH", value_parser = parse_size)]
    pub size: Option<[u32; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ThemeArg {
    Dark,
    Light,
}

impl From<ThemeArg> for ThemeKind {
    fn from(arg: ThemeArg) -> Self {
        match arg {
            ThemeArg::Dark => ThemeKind::Dark,
            ThemeArg::Light => ThemeKind::Light,
        }
    }
}

fn parse_view(text: &str) -> Result<ViewPreset, String> {
    ViewPreset::parse(text).ok_or_else(|| {
        format!("unknown view `{text}` (expected occlusal, facial, lingual, left, right or bottom)")
    })
}

/// Parses `WxH` (also accepts `X` or `×`) into a size of at least 320×240.
pub fn parse_size(text: &str) -> Result<[u32; 2], String> {
    let bad = || format!("invalid size `{text}` (expected WxH, e.g. 1600x1000)");
    let mut parts = text.split(['x', 'X', '×']);
    let (Some(w), Some(h), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(bad());
    };
    let w: u32 = w.trim().parse().map_err(|_| bad())?;
    let h: u32 = h.trim().parse().map_err(|_| bad())?;
    if w < 320 || h < 240 || w > 16384 || h > 16384 {
        return Err(format!(
            "size `{text}` is out of range (320x240 to 16384x16384)"
        ));
    }
    Ok([w, h])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_parses_width_and_height() {
        assert_eq!(parse_size("1600x1000"), Ok([1600, 1000]));
        assert_eq!(parse_size("800X600"), Ok([800, 600]));
    }

    #[test]
    fn size_rejects_malformed_and_tiny_values() {
        assert!(parse_size("1600").is_err());
        assert!(parse_size("1600x").is_err());
        assert!(parse_size("1x2x3").is_err());
        assert!(parse_size("axb").is_err());
        assert!(parse_size("100x100").is_err());
    }

    #[test]
    fn full_command_line_parses() {
        let args = Args::try_parse_from([
            "dentcraft",
            "a.stl",
            "b.ply",
            "--view",
            "facial",
            "--theme",
            "light",
            "--screenshot",
            "out.png",
            "--size",
            "1280x800",
        ])
        .unwrap();
        assert_eq!(args.files.len(), 2);
        assert_eq!(args.view, Some(ViewPreset::Facial));
        assert_eq!(args.theme, Some(ThemeArg::Light));
        assert_eq!(args.screenshot, Some(PathBuf::from("out.png")));
        assert_eq!(args.size, Some([1280, 800]));
    }

    #[test]
    fn unknown_view_is_an_error() {
        assert!(Args::try_parse_from(["dentcraft", "--view", "sideways"]).is_err());
    }
}
