//! DentCraft desktop application.

mod anatomy_step;
mod app;
mod args;
mod axis_step;
mod browser;
mod chart;
mod contacts_step;
mod crash;
mod document;
mod export_step;
mod history;
mod icon_svg;
mod inner_step;
mod loader;
mod margin_edit;
mod margin_step;
mod objects;
mod orient_step;
mod recovery;
mod ribbon;
mod theme;
mod tools;
mod tooth_view;
mod viewport;
mod widgets;
mod workflow_panel;
mod workspace;

use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use clap::Parser;
use eframe::egui;

use crate::app::{DentCraftApp, StartOptions};
use crate::args::{Args, DEFAULT_SIZE, OpenArg};

fn main() -> ExitCode {
    let args = Args::parse();
    let screenshot = args.screenshot.is_some();
    // Crash logs are kept for every run; a screenshot run never touches the user's recovery
    // files.
    let data = crash::data_dir();
    // Nor does an instance that cannot take its lock: other instances would take its file for a
    // crash's.
    let instance = data.as_deref().filter(|_| !screenshot).and_then(|data| {
        match recovery::start(&recovery::recovery_dir(data), std::process::id()) {
            Ok(lock) => Some((data, lock)),
            Err(err) => {
                eprintln!("dentcraft: recovery is off: {err}");
                None
            }
        }
    });
    let recovery = instance
        .as_ref()
        .map(|(data, _)| recovery::recovery_path(data));
    // A recovery file left by another (crashed) instance is offered for restore at start-up.
    let restore = instance.as_ref().and_then(|(data, _)| {
        recovery::leftover(&recovery::recovery_dir(data), std::process::id())
    });
    crash::install_panic_hook(data.as_deref().map(crash::crash_log_dir), recovery.clone());
    let [w, h] = args.size.unwrap_or(DEFAULT_SIZE);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("DentCraft")
        .with_app_id(crash::APP_ID)
        .with_inner_size([w as f32, h as f32])
        .with_min_inner_size([960.0, 600.0])
        .with_drag_and_drop(true);
    if let Some(logo) = app::logo_image() {
        let (width, height) = logo.dimensions();
        viewport = viewport.with_icon(Arc::new(egui::IconData {
            rgba: logo.into_raw(),
            width,
            height,
        }));
    }
    let options = eframe::NativeOptions {
        viewport,
        // The scene renderer brings its own MSAA and depth; egui's pass must have neither.
        multisampling: 0,
        depth_buffer: 0,
        // An explicit size or a screenshot run must not be overridden by the saved window.
        persist_window: !screenshot && args.size.is_none(),
        ..Default::default()
    };

    let exit_code = Arc::new(Mutex::new(None));
    let start = StartOptions {
        files: args.files,
        case: args.case,
        view: args.view,
        theme: args.theme.map(Into::into),
        hide: args.hide,
        tab: args.tab.map(Into::into),
        workspace_menu_open: args.open == Some(OpenArg::WorkspaceMenu),
        screenshot: args.screenshot,
        measure: args.measure,
        orient: args.orient,
        margin: args.margin,
        margin_points: args.margin_points,
        step: args.step.map(Into::into),
        tooth: args.tooth,
        section: args.section,
        export: args.export,
        recovery,
        recovery_lock: instance.map(|(_, lock)| lock),
        restore,
        exit_after: args.exit_after,
        exit_code: exit_code.clone(),
    };
    let result = eframe::run_native(
        "DentCraft",
        options,
        Box::new(move |cc| Ok(Box::new(DentCraftApp::new(cc, start)))),
    );
    if let Err(err) = result {
        eprintln!("dentcraft: {err}");
        return ExitCode::FAILURE;
    }
    if screenshot {
        // Closing the window before the capture counts as a failure.
        let code = exit_code.lock().ok().and_then(|c| *c).unwrap_or(1);
        return ExitCode::from(code);
    }
    ExitCode::SUCCESS
}
