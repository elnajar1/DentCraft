//! Command line of the desktop application.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use dentcraft_case::{Fdi, ScanRole};
use dentcraft_render::ViewPreset;
use dentcraft_workflow::StepId;
use glam::DVec3;

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

    /// Start the Measure tool between two points in millimetres, e.g. "0,0,0;10,0,0". Each point
    /// snaps to the nearest surface of the visible scans once they have loaded.
    #[arg(long, value_name = "X,Y,Z;X,Y,Z", value_parser = parse_measure, allow_hyphen_values = true)]
    pub measure: Option<[DVec3; 2]>,

    /// Run the jaw orientation (occlusal plane and midline) automatically at startup.
    #[arg(long)]
    pub orient: bool,

    /// Margin line click on a tooth: FDI tooth number and a point in millimetres, e.g.
    /// "16@1.5,-3,12". Each point snaps to the nearest surface. Repeat for several clicks.
    #[arg(long, value_name = "FDI@X,Y,Z", value_parser = parse_margin_click, allow_hyphen_values = true)]
    pub margin: Vec<(Fdi, DVec3)>,

    /// Hand-traced margin on a tooth: FDI tooth number and waypoints in millimetres, e.g.
    /// "33@1,2,3;4,5,6;7,8,9". Each point snaps to the nearest surface. Repeat per tooth.
    #[arg(long, value_name = "FDI@X,Y,Z;X,Y,Z", value_parser = parse_margin_points, allow_hyphen_values = true)]
    pub margin_points: Vec<(Fdi, Vec<DVec3>)>,

    /// Design step to open: scans, orient, margin, axis, inner, anatomy, contacts or export.
    #[arg(long, value_enum)]
    pub step: Option<StepArg>,

    /// Tooth (FDI number, 11-48) the design steps work on.
    #[arg(long, value_name = "FDI", value_parser = parse_tooth)]
    pub tooth: Option<Fdi>,

    /// Start the Inner surface step with its section view on.
    #[arg(long)]
    pub section: bool,

    /// Write every built crown as STL into this folder once the startup steps are done.
    #[arg(long, value_name = "DIR")]
    pub export: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum StepArg {
    Scans,
    Orient,
    Margin,
    Axis,
    Inner,
    Anatomy,
    Contacts,
    Export,
}

impl From<StepArg> for StepId {
    fn from(arg: StepArg) -> Self {
        match arg {
            StepArg::Scans => StepId::Scans,
            StepArg::Orient => StepId::Orient,
            StepArg::Margin => StepId::Margin,
            StepArg::Axis => StepId::InsertionAxis,
            StepArg::Inner => StepId::InnerSurface,
            StepArg::Anatomy => StepId::Anatomy,
            StepArg::Contacts => StepId::Occlusion,
            StepArg::Export => StepId::Finish,
        }
    }
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

/// Parses two points `x,y,z;x,y,z` of finite numbers.
pub fn parse_measure(text: &str) -> Result<[DVec3; 2], String> {
    let bad = || format!("invalid points `{text}` (expected x,y,z;x,y,z, e.g. 0,0,0;10,0,0)");
    let point = |part: &str| -> Result<DVec3, String> {
        let v: Vec<f64> = part
            .split(',')
            .map(|n| n.trim().parse::<f64>().map_err(|_| bad()))
            .collect::<Result<_, _>>()?;
        match v[..] {
            [x, y, z] if v.iter().all(|c| c.is_finite()) => Ok(DVec3::new(x, y, z)),
            _ => Err(bad()),
        }
    };
    let mut parts = text.split(';');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(bad());
    };
    Ok([point(a)?, point(b)?])
}

fn parse_tooth(text: &str) -> Result<Fdi, String> {
    let bad = || format!("invalid tooth `{text}` (expected an FDI number from 11 to 48)");
    Fdi::new(text.trim().parse().map_err(|_| bad())?).map_err(|_| bad())
}

/// Parses `FDI@x,y,z`: a tooth and a point of finite numbers.
pub fn parse_margin_click(text: &str) -> Result<(Fdi, DVec3), String> {
    let bad = || format!("invalid margin click `{text}` (expected FDI@x,y,z, e.g. 16@1.5,-3,12)");
    let (tooth, point) = text.split_once('@').ok_or_else(bad)?;
    let tooth = parse_tooth(tooth)?;
    let v: Vec<f64> = point
        .split(',')
        .map(|n| n.trim().parse::<f64>().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    match v[..] {
        [x, y, z] if v.iter().all(|c| c.is_finite()) => Ok((tooth, DVec3::new(x, y, z))),
        _ => Err(bad()),
    }
}

/// Parses `FDI@x,y,z;x,y,z;...`: a tooth and at least three waypoints.
pub fn parse_margin_points(text: &str) -> Result<(Fdi, Vec<DVec3>), String> {
    let bad = || {
        format!(
            "invalid margin points `{text}` (expected FDI@x,y,z;x,y,z;x,y,z, e.g. 33@1,2,3;4,5,6;7,8,9)"
        )
    };
    let (tooth, points) = text.split_once('@').ok_or_else(bad)?;
    let tooth = parse_tooth(tooth)?;
    let points: Vec<DVec3> = points
        .split(';')
        .map(|p| parse_margin_click(&format!("{tooth}@{p}")).map(|(_, v)| v))
        .collect::<Result<_, _>>()
        .map_err(|_| bad())?;
    if points.len() < 3 {
        return Err(bad());
    }
    Ok((tooth, points))
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
    fn measure_parses_two_points() {
        assert_eq!(
            parse_measure("1,2,3;-4.5, 0 ,1e1"),
            Ok([DVec3::new(1.0, 2.0, 3.0), DVec3::new(-4.5, 0.0, 10.0)])
        );
        let args = Args::try_parse_from(["dentcraft", "--measure", "0,0,0;1,1,1"]).unwrap();
        assert_eq!(args.measure, Some([DVec3::ZERO, DVec3::ONE]));
        assert_eq!(Args::try_parse_from(["dentcraft"]).unwrap().measure, None);
        // A first coordinate below zero is a value, not an option.
        let args = Args::try_parse_from(["dentcraft", "--measure", "-1,0,0;1,0,0"]).unwrap();
        assert_eq!(args.measure, Some([-DVec3::X, DVec3::X]));
    }

    #[test]
    fn measure_rejects_malformed_and_non_finite_points() {
        for bad in [
            "",
            "1,2,3",
            "1,2,3;4,5",
            "1,2,3;4,5,6,7",
            "1,2,3;4,5,6;7,8,9",
            "a,b,c;1,2,3",
            "1,2,3;4,,6",
            "NaN,0,0;1,1,1",
            "0,0,0;inf,1,1",
        ] {
            assert!(parse_measure(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[test]
    fn unknown_view_is_an_error() {
        assert!(Args::try_parse_from(["dentcraft", "--view", "sideways"]).is_err());
    }

    #[test]
    fn orient_margin_step_and_tooth_parse() {
        let args = Args::try_parse_from([
            "dentcraft",
            "--orient",
            "--margin",
            "16@1.5,-3,12",
            "--margin",
            "16@-1,0,2",
            "--step",
            "axis",
            "--tooth",
            "16",
        ])
        .unwrap();
        assert!(args.orient);
        let fdi = Fdi::new(16).unwrap();
        assert_eq!(
            args.margin,
            vec![
                (fdi, DVec3::new(1.5, -3.0, 12.0)),
                (fdi, DVec3::new(-1.0, 0.0, 2.0))
            ]
        );
        assert_eq!(args.step.map(StepId::from), Some(StepId::InsertionAxis));
        assert_eq!(args.tooth, Some(fdi));
        let none = Args::try_parse_from(["dentcraft"]).unwrap();
        assert!(!none.orient && none.margin.is_empty() && none.step.is_none());
        assert_eq!(none.tooth, None);
    }

    #[test]
    fn step_contacts_opens_the_occlusion_step() {
        let args = Args::try_parse_from(["dentcraft", "--step", "contacts"]).unwrap();
        assert_eq!(args.step.map(StepId::from), Some(StepId::Occlusion));
    }

    #[test]
    fn margin_points_parse_a_tooth_and_its_waypoints() {
        let fdi = Fdi::new(33).unwrap();
        let args =
            Args::try_parse_from(["dentcraft", "--margin-points", "33@1,2,3;-4.5,5,6; 7,8,9"])
                .unwrap();
        assert_eq!(
            args.margin_points,
            vec![(
                fdi,
                vec![
                    DVec3::new(1.0, 2.0, 3.0),
                    DVec3::new(-4.5, 5.0, 6.0),
                    DVec3::new(7.0, 8.0, 9.0)
                ]
            )]
        );
        for bad in [
            "",
            "33",
            "33@",
            "33@1,2,3;4,5,6",
            "33@1,2,3;4,5,6;",
            "33@1,2,3;4,5;7,8,9",
            "33@1,2,3;4,5,6;x,y,z",
            "33@1,2,3;4,5,6;NaN,0,0",
            "99@1,2,3;4,5,6;7,8,9",
        ] {
            assert!(
                parse_margin_points(bad).is_err(),
                "`{bad}` should be rejected"
            );
        }
    }

    #[test]
    fn margin_rejects_bad_tooth_point_and_nan() {
        for bad in [
            "",
            "16",
            "16@",
            "16@1,2",
            "16@1,2,3,4",
            "16@a,b,c",
            "19@0,0,0",
            "9@0,0,0",
            "x@0,0,0",
            "@0,0,0",
            "16@NaN,0,0",
            "16@0,inf,0",
        ] {
            assert!(
                parse_margin_click(bad).is_err(),
                "`{bad}` should be rejected"
            );
        }
        assert!(Args::try_parse_from(["dentcraft", "--margin", "99@0,0,0"]).is_err());
        assert!(Args::try_parse_from(["dentcraft", "--tooth", "50"]).is_err());
        assert!(Args::try_parse_from(["dentcraft", "--step", "sculpt"]).is_err());
    }
}
