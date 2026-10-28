//! Command line of the desktop application.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use dentcraft_case::ScanRole;
use dentcraft_render::ViewPreset;

use crate::ribbon::RibbonTab;
use crate::theme::ThemeKind;

/// Default window size, in logical points.
pub const DEFAULT_SIZE: [u32; 2] = [1600, 1000];

#[derive(Debug, Parser)]
#[command(name = "dentcraft", version, about = "DentCraft — open dental CAD")]
pub struct Args {
    /// Scan files to open (STL, PLY or OBJ).
    pub files: Vec<PathBuf>,

    /// Case file to open at startup (.dentcraft); its scans load with their saved roles.
    #[arg(long, value_name = "FILE.dentcraft")]
    pub case: Option<PathBuf>,

    /// Initial view: occlusal, facial, lingual, left, right or bottom.
    #[arg(long, value_parser = parse_view)]
    pub view: Option<ViewPreset>,

    /// Colour theme (overrides the saved choice).
    #[arg(long, value_enum)]
    pub theme: Option<ThemeArg>,

    /// Hide scans with this role once loaded: upper, lower, upper-pre-op, lower-pre-op, bite
    /// or other. Repeat to hide several.
    #[arg(long, value_name = "ROLE", value_parser = parse_role)]
    pub hide: Vec<ScanRole>,

    /// Ribbon tab to show.
    #[arg(long, value_enum)]
    pub tab: Option<TabArg>,

    /// Popup to show open at startup, e.g. for a screenshot: workspace-menu.
    #[arg(long, value_enum, value_name = "POPUP")]
    pub open: Option<OpenArg>,

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TabArg {
    Case,
    Design,
    View,
}

impl From<TabArg> for RibbonTab {
    fn from(arg: TabArg) -> Self {
        match arg {
            TabArg::Case => RibbonTab::Case,
            TabArg::Design => RibbonTab::Design,
            TabArg::View => RibbonTab::View,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OpenArg {
    /// The workspace list under the switcher at the left end of the ribbon.
    WorkspaceMenu,
}

/// Parses a scan role as written on the command line.
pub fn parse_role(text: &str) -> Result<ScanRole, String> {
    Ok(match text {
        "upper" => ScanRole::Upper,
        "lower" => ScanRole::Lower,
        "upper-pre-op" => ScanRole::UpperPreOp,
        "lower-pre-op" => ScanRole::LowerPreOp,
        "bite" => ScanRole::Bite,
        "other" => ScanRole::Other,
        _ => {
            return Err(format!(
                "unknown role `{text}` (expected upper, lower, upper-pre-op, lower-pre-op, bite or other)"
            ));
        }
    })
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
            "--case",
            "c.dentcraft",
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
        assert_eq!(args.case, Some(PathBuf::from("c.dentcraft")));
        assert_eq!(args.view, Some(ViewPreset::Facial));
        assert_eq!(args.theme, Some(ThemeArg::Light));
        assert_eq!(args.screenshot, Some(PathBuf::from("out.png")));
        assert_eq!(args.size, Some([1280, 800]));
    }

    #[test]
    fn hide_repeats_and_tab_selects_a_ribbon_tab() {
        let args = Args::try_parse_from([
            "dentcraft",
            "--hide",
            "lower",
            "--hide",
            "upper-pre-op",
            "--tab",
            "design",
        ])
        .unwrap();
        assert_eq!(args.hide, vec![ScanRole::Lower, ScanRole::UpperPreOp]);
        assert_eq!(args.tab.map(RibbonTab::from), Some(RibbonTab::Design));
        let none = Args::try_parse_from(["dentcraft"]).unwrap();
        assert!(none.hide.is_empty());
        assert_eq!(none.tab, None);
    }

    #[test]
    fn open_shows_the_workspace_menu() {
        let args = Args::try_parse_from(["dentcraft", "--open", "workspace-menu"]).unwrap();
        assert_eq!(args.open, Some(OpenArg::WorkspaceMenu));
        assert_eq!(Args::try_parse_from(["dentcraft"]).unwrap().open, None);
        assert!(Args::try_parse_from(["dentcraft", "--open", "settings"]).is_err());
    }

    #[test]
    fn every_role_parses() {
        let names = [
            "upper",
            "lower",
            "upper-pre-op",
            "lower-pre-op",
            "bite",
            "other",
        ];
        let roles: Vec<_> = names.iter().map(|n| parse_role(n).unwrap()).collect();
        assert_eq!(roles, ScanRole::ALL);
    }

    #[test]
    fn unknown_role_and_tab_are_errors() {
        assert!(Args::try_parse_from(["dentcraft", "--hide", "maxilla"]).is_err());
        assert!(Args::try_parse_from(["dentcraft", "--tab", "sculpt"]).is_err());
    }

    #[test]
    fn unknown_view_is_an_error() {
        assert!(Args::try_parse_from(["dentcraft", "--view", "sideways"]).is_err());
    }
}
