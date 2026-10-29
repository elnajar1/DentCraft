//! The application shell: top bar, case panel, workflow panel, viewport and status bar.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dentcraft_case::{Case, Fdi, Material, Scan, ScanRole};
use dentcraft_mesh::Aabb;
use dentcraft_render::{MeshId, ViewPreset};
use dentcraft_workflow::{Session, StepId, plan};
use glam::DVec3;

use eframe::egui::{
    self, Align, Align2, ColorImage, CornerRadius, Key, KeyboardShortcut, Layout, Margin,
    Modifiers, RichText, Stroke, TextureHandle, TextureOptions, vec2,
};

use crate::anatomy_step::{self, ANATOMY, AnatomyStep};
use crate::axis_step::{self, AXIS, AxisStep};
use crate::browser::{self, SECTIONS_KEY, Section, Sections};
use crate::chart::{self, ChartEdit};
use crate::contacts_step::{self, CONTACTS, ContactsStep};
use crate::document::{self, Document};
use crate::export_step::{self, ExportStep, PlanItem};
use crate::history::History;
use crate::inner_step::{self, INNER, InnerStep};
use crate::loader::Loader;
use crate::margin_step::{self, MARGIN, MarginStep, StepFrame};
use crate::objects::{self, ObjectState, SceneObject};
use crate::orient_step::{self, ORIENT, OrientStep};
use crate::ribbon::{self, Command, RibbonTab, RibbonView, WorkspaceMenu};
use crate::theme::{self, Palette, RADIUS_LG, SIZE_SMALL, ThemeKind};
use crate::tools::{self, MEASURE, Measure, Tools};
use crate::viewport::Viewport;
use crate::widgets::{Icon, icon_button, paint_icon, small_button};
use crate::workflow_panel::{self, StepAction, WorkflowView};

const THEME_KEY: &str = "theme";
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(60);
/// Frames to wait after loading and camera motion have settled before capturing.
const SCREENSHOT_SETTLE_FRAMES: u32 = 6;
const UNTITLED: &str = "Untitled case";

/// How the app starts, from the command line.
pub struct StartOptions {
    pub files: Vec<PathBuf>,
    /// Case file to open at startup.
    pub case: Option<PathBuf>,
    pub view: Option<ViewPreset>,
    pub theme: Option<ThemeKind>,
    /// Roles of the startup scans to hide.
    pub hide: Vec<ScanRole>,
    /// Ribbon tab to show first.
    pub tab: Option<RibbonTab>,
    /// Show the workspace menu open, for screenshots.
    pub workspace_menu_open: bool,
    pub screenshot: Option<PathBuf>,
    /// Two points to measure between, snapped to the scans once they have loaded.
    pub measure: Option<[DVec3; 2]>,
    /// Run the jaw orientation at startup.
    pub orient: bool,
    /// Margin clicks (tooth, point) to replay at startup.
    pub margin: Vec<(Fdi, DVec3)>,
    /// Design step to open.
    pub step: Option<StepId>,
    /// Tooth the design steps work on.
    pub tooth: Option<Fdi>,
    /// Start the Inner surface step with its section view on.
    pub section: bool,
    /// Folder to export every built crown to once the startup steps are done.
    pub export: Option<PathBuf>,
    /// Exit code of a screenshot run, read by `main` once the window has closed.
    pub exit_code: Arc<Mutex<Option<u8>>>,
}

struct ScreenshotJob {
    path: PathBuf,
    started: Instant,
    settled_frames: u32,
    requested: bool,
    image: Arc<Mutex<Option<Arc<ColorImage>>>>,
    exit_code: Arc<Mutex<Option<u8>>>,
}

struct Toast {
    message: String,
}

/// An action that would throw away unsaved changes, waiting for the user to confirm it.
enum Discard {
    New,
    Open(PathBuf),
}

pub struct DentCraftApp {
    theme: ThemeKind,
    case: Case,
    objects: Vec<SceneObject>,
    next_id: u64,
    loader: Loader,
    viewport: Viewport,
    initial_view: ViewPreset,
    /// Frame the scene as loads finish, until the user takes the camera.
    auto_fit: bool,
    toasts: Vec<Toast>,
    logo_small: TextureHandle,
    logo_large: TextureHandle,
    hovered_row: Option<MeshId>,
    screenshot: Option<ScreenshotJob>,
    session: Session,
    /// Last refusal from the workflow session, shown under the step card.
    workflow_error: Option<String>,
    /// Height of the workflow step card last frame.
    workflow_card_height: f32,
    doc: Document,
    /// Undo and redo of the case's design.
    history: History,
    confirm_discard: Option<Discard>,
    window_title: String,
    ribbon_tab: RibbonTab,
    workspace_menu: WorkspaceMenu,
    /// Open and collapsed sections of the side panels.
    sections: Sections,
    /// The active viewport tool.
    tools: Tools,
    /// Points from `--measure`, waiting for the scans to load.
    pending_measure: Option<[DVec3; 2]>,
    /// The Margin step's tracers, jobs and edits.
    margin: MarginStep,
    /// The Orientation step's job and edits.
    orient: OrientStep,
    /// The Insertion axis step's preparation, jobs and edits.
    axis: AxisStep,
    /// The Inner surface step's jobs, surface and section toggle.
    inner: InnerStep,
    /// The Anatomy step's jobs and crown.
    anatomy: AnatomyStep,
    /// The Occlusion & contacts step's jobs, adapted crowns and distance map.
    contacts: ContactsStep,
    /// `--step inner` or `--step anatomy`: the start went through the Insertion axis step and
    /// moves on to this step once the tooth has an axis.
    pending_inner: Option<StepId>,
    /// The Finish & export step's options, worker and results.
    export: ExportStep,
    /// `--step contacts` or `--step export`: the start went through the Anatomy step and moves on
    /// to this step (Occlusion or Finish & export) once the crown is built (or has failed).
    pending_finish: Option<StepId>,
    /// The Anatomy or Finish step was entered, or a first crown was built: frame the crown
    /// once it is there.
    frame_crown: bool,
    /// `--export DIR`, waiting for the startup chain to finish.
    pending_export: Option<PathBuf>,
    /// `--orient`, waiting for the scans to load.
    pending_orient: bool,
    /// Tooth the per-tooth step works on.
    tooth: Option<Fdi>,
    /// `--step`, `--tooth` and `--margin`, waiting for the scans to load.
    pending_step: Option<StepId>,
    pending_tooth: Option<Fdi>,
    pending_margin: Vec<(Fdi, DVec3)>,
}

/// The logo shipped with the app, decoded to RGBA.
pub fn logo_image() -> Option<image::RgbaImage> {
    let bytes = include_bytes!("../../../docs/assets/logo-512.png");
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()
        .map(|i| i.to_rgba8())
}

fn logo_texture(ctx: &egui::Context, name: &str, px: u32) -> TextureHandle {
    let image = logo_image()
        .map(|i| image::imageops::resize(&i, px, px, image::imageops::FilterType::Lanczos3))
        .map(|i| ColorImage::from_rgba_unmultiplied([px as usize, px as usize], i.as_raw()))
        .unwrap_or_else(|| ColorImage::filled([1, 1], egui::Color32::TRANSPARENT));
    ctx.load_texture(name, image, TextureOptions::LINEAR)
}

impl DentCraftApp {
    pub fn new(cc: &eframe::CreationContext<'_>, start: StartOptions) -> Self {
        let gpu = match cc.wgpu_render_state.as_ref() {
            Some(rs) => {
                crate::viewport::init_gpu(rs);
                true
            }
            None => false,
        };
        // A screenshot run ignores saved state, so it looks the same on every machine.
        let storage = cc.storage.filter(|_| start.screenshot.is_none());
        let stored = storage
            .and_then(|s| s.get_string(THEME_KEY))
            .and_then(|k| ThemeKind::from_key(&k));
        let sections = storage
            .and_then(|s| s.get_string(SECTIONS_KEY))
            .map(|k| Sections::from_key(&k))
            .unwrap_or_default();
        let theme = start.theme.or(stored).unwrap_or_default();
        theme::install(&cc.egui_ctx, theme);

        let initial_view = start.view.unwrap_or(ViewPreset::Occlusal);
        let case = Case::new(UNTITLED);
        let mut app = Self {
            theme,
            session: Session::for_case(&case),
            workflow_error: None,
            workflow_card_height: 260.0,
            doc: Document::untitled(&case),
            history: History::default(),
            confirm_discard: None,
            window_title: String::new(),
            ribbon_tab: start.tab.unwrap_or_default(),
            workspace_menu: WorkspaceMenu::new(start.workspace_menu_open),
            sections,
            tools: Tools::default(),
            pending_measure: start.measure,
            margin: MarginStep::new(Some(cc.egui_ctx.clone())),
            orient: OrientStep::new(Some(cc.egui_ctx.clone())),
            axis: AxisStep::new(Some(cc.egui_ctx.clone())),
            inner: {
                let mut inner = InnerStep::new(Some(cc.egui_ctx.clone()));
                inner.set_section(start.section);
                inner
            },
            anatomy: AnatomyStep::new(Some(cc.egui_ctx.clone())),
            contacts: ContactsStep::new(Some(cc.egui_ctx.clone())),
            pending_inner: None,
            export: ExportStep::new(Some(cc.egui_ctx.clone())),
            pending_finish: None,
            frame_crown: false,
            pending_export: start.export,
            pending_orient: start.orient,
            tooth: None,
            pending_step: start.step,
            pending_tooth: start.tooth,
            pending_margin: start.margin,
            case,
            objects: Vec::new(),
            next_id: 1,
            loader: Loader::new(Some(cc.egui_ctx.clone())),
            viewport: Viewport::new(gpu, initial_view),
            initial_view,
            auto_fit: true,
            toasts: Vec::new(),
            logo_small: logo_texture(&cc.egui_ctx, "logo-small", 96),
            logo_large: logo_texture(&cc.egui_ctx, "logo-large", 256),
            hovered_row: None,
            screenshot: start.screenshot.map(|path| ScreenshotJob {
                path,
                started: Instant::now(),
                settled_frames: 0,
                requested: false,
                image: Arc::new(Mutex::new(None)),
                exit_code: start.exit_code.clone(),
            }),
        };
        if let Some(path) = start.case {
            app.open_case(path);
        }
        app.import(start.files);
        for role in start.hide {
            objects::set_role_visible(&mut app.objects, role, false);
        }
        app
    }

    fn palette(&self) -> Palette {
        self.theme.palette()
    }

    fn import(&mut self, paths: Vec<PathBuf>) {
        let scans = paths
            .into_iter()
            .map(|path| {
                let role = objects::guess_role(&path);
                (path, role)
            })
            .collect();
        self.add_scans(scans);
    }

    /// Loads scan files into the scene with the given roles.
    fn add_scans(&mut self, scans: Vec<(PathBuf, ScanRole)>) {
        if scans.is_empty() {
            return;
        }
        if !self.objects.iter().any(|o| o.loaded().is_some()) {
            self.auto_fit = true;
        }
        for (path, role) in scans {
            let id = MeshId(self.next_id);
            self.next_id += 1;
            let used: Vec<[u8; 3]> = self.objects.iter().map(|o| o.color).collect();
            self.objects.push(SceneObject {
                id,
                name: objects::display_name(&path),
                path: path.clone(),
                role,
                color: objects::default_color(role, &used),
                visible: true,
                opacity: 1.0,
                state: ObjectState::Loading,
            });
            self.loader.spawn(id, path);
        }
    }

    fn pick_files(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Import scans")
            .add_filter("Scans (STL, PLY, OBJ)", &["stl", "ply", "obj"])
            .add_filter("All files", &["*"])
            .pick_files()
        {
            self.import(paths);
        }
    }

    fn bounds(&self) -> Option<Aabb> {
        objects::union_bounds(
            self.objects
                .iter()
                .filter(|o| o.visible)
                .filter_map(|o| o.loaded()?.mesh.bounds()),
        )
    }

    fn receive_loads(&mut self) {
        for msg in self.loader.poll() {
            let Some(index) = self.objects.iter().position(|o| o.id == msg.id) else {
                continue; // Removed while loading.
            };
            match msg.result {
                Ok(loaded) => self.objects[index].state = ObjectState::Ready(Box::new(loaded)),
                Err(err) => {
                    self.objects.remove(index);
                    self.fail(format!("Could not open {}: {err}", msg.path.display()));
                }
            }
            if let Some(bounds) = self.bounds() {
                if self.auto_fit {
                    self.viewport.go_to(self.initial_view, Some(&bounds));
                } else {
                    self.viewport.camera.set_scene(&bounds);
                }
            }
        }
        if self.objects.iter().all(|o| o.loaded().is_some()) && !self.objects.is_empty() {
            self.auto_fit = false;
            if let Some([a, b]) = self.pending_measure.take() {
                let snap = |p| tools::snap_to_surface(&self.objects, p);
                match (snap(a), snap(b)) {
                    (Some(a), Some(b)) => self.tools.activate(Box::new(Measure::between(a, b))),
                    _ => self.fail("--measure: no visible scan to snap the points to".to_owned()),
                }
            }
            self.start_design();
        }
        // Scans still loading belong to the case too, so opening a case does not mark it changed.
        self.case.scans = self
            .objects
            .iter()
            .map(|o| Scan {
                role: o.role,
                file: o.path.clone(),
            })
            .collect();
    }

    fn fail(&mut self, message: String) {
        if let Some(job) = &self.screenshot {
            eprintln!("dentcraft: {message}");
            set_exit(&job.exit_code, 1);
        }
        self.toasts.push(Toast { message });
    }

    fn is_dirty(&self) -> bool {
        self.doc.is_dirty(&self.case)
    }

    /// Starts the automatic orientation, with the camera's up direction and the case's front
    /// teeth as hints for an arch that does not show its front.
    fn start_orient(&mut self) {
        let teeth: Vec<Fdi> = self.case.units.iter().map(|u| u.tooth).collect();
        let up = self.viewport.camera.up();
        let hints = orient_step::FrontHints {
            anterior: orient_step::anterior_centre(&self.case.design, &teeth),
            up: glam::DVec3::new(up.x.into(), up.y.into(), up.z.into()),
        };
        self.orient.start(&self.objects, &teeth, hints);
    }

    /// Whether a jaw scan has loaded, which the Scans step needs.
    fn has_jaw_scan(&self) -> bool {
        self.objects
            .iter()
            .any(|o| o.loaded().is_some() && matches!(o.role, ScanRole::Upper | ScanRole::Lower))
    }

    /// Re-plans the workflow after the units or connectors changed, keeping finished steps.
    fn replan(&mut self) {
        self.session = self.session.replan(plan(&self.case));
        self.workflow_error = None;
    }

    fn apply_chart_edit(&mut self, edit: ChartEdit) {
        match edit {
            ChartEdit::Set(tooth, Some(kind)) => self.case.set_unit(tooth, kind),
            ChartEdit::Set(tooth, None) => self.case.remove_unit(tooth),
            ChartEdit::Connect(a, b, on) => {
                self.case.set_connected(a, b, on);
            }
        }
        self.replan();
    }

    fn apply_step_action(&mut self, action: StepAction) {
        let walks_teeth = self
            .session
            .current()
            .filter(|s| {
                matches!(
                    s.id,
                    StepId::Margin
                        | StepId::InsertionAxis
                        | StepId::InnerSurface
                        | StepId::Anatomy
                        | StepId::Occlusion
                )
            })
            .map(|s| s.teeth.clone());
        let (result, tooth) = match action {
            StepAction::Next | StepAction::Back => {
                let forward = action == StepAction::Next;
                if let Some(t) = walks_teeth
                    .as_deref()
                    .and_then(|teeth| margin_step::neighbour_tooth(teeth, self.tooth, forward))
                {
                    (Ok(()), Some(t))
                } else if forward {
                    match self.session.current() {
                        Some(step)
                            if let Some(why) = workflow_panel::blocker(
                                step.id,
                                self.has_jaw_scan(),
                                !self.case.units.is_empty(),
                            ) =>
                        {
                            self.workflow_error = Some(why.to_owned());
                            return;
                        }
                        _ => (self.session.complete(), None),
                    }
                } else {
                    let result = self.session.back();
                    let last = self.session.current().and_then(|s| s.teeth.last().copied());
                    (result, last)
                }
            }
            StepAction::Skip => (self.session.skip(), None),
            StepAction::GoTo(index) => (self.session.go_to(index), None),
            StepAction::Tooth(index, tooth) => {
                let result = if index == self.session.current_index() {
                    Ok(())
                } else {
                    self.session.go_to(index)
                };
                (result, Some(tooth))
            }
            StepAction::DetectAgain => {
                if let Some(tooth) = self.tooth {
                    let axis = self.margin_axis();
                    self.margin.detect_again(tooth, axis);
                }
                return;
            }
            StepAction::AutoOrient => {
                self.start_orient();
                return;
            }
            StepAction::ResetOrient => {
                self.orient.clear(&mut self.case.design, &mut self.history);
                return;
            }
            StepAction::BestAxis => {
                let occlusal = self.occlusal();
                self.axis.best(occlusal);
                return;
            }
            StepAction::AxisFromView => {
                if let Some(tooth) = self.tooth {
                    let dir = -self.viewport.camera.forward().as_dvec3();
                    self.axis
                        .set_from_view(&mut self.case.design, &mut self.history, tooth, dir);
                }
                return;
            }
            StepAction::SetInner(param, value, dragging) => {
                if let Some(tooth) = self.tooth {
                    self.inner.set_param(
                        &mut self.case.design,
                        &mut self.history,
                        &self.case.material,
                        tooth,
                        param,
                        value,
                        dragging,
                    );
                }
                return;
            }
            StepAction::EndInnerDrag => {
                self.inner.end_drag(&mut self.history);
                return;
            }
            StepAction::ResetInner => {
                if let Some(tooth) = self.tooth {
                    inner_step::reset_params(&mut self.case.design, &mut self.history, tooth);
                }
                return;
            }
            StepAction::SetAnatomy(param, value, dragging) => {
                if let Some(tooth) = self.tooth {
                    self.anatomy.set_param(
                        &mut self.case.design,
                        &mut self.history,
                        tooth,
                        param,
                        value,
                        dragging,
                    );
                }
                return;
            }
            StepAction::EndAnatomyDrag => {
                self.anatomy.end_drag(&mut self.history);
                return;
            }
            StepAction::NudgeAnatomy(axis, delta) => {
                if let Some(tooth) = self.tooth {
                    anatomy_step::nudge(
                        &mut self.case.design,
                        &mut self.history,
                        tooth,
                        axis,
                        delta,
                    );
                }
                return;
            }
            StepAction::ResetAnatomy => {
                if let Some(tooth) = self.tooth {
                    anatomy_step::reset(&mut self.case.design, &mut self.history, tooth);
                }
                return;
            }
            StepAction::SetContact(param, value, dragging) => {
                if let Some(tooth) = self.tooth {
                    self.contacts.set_param(
                        &mut self.case.design,
                        &mut self.history,
                        tooth,
                        param,
                        value,
                        dragging,
                    );
                }
                return;
            }
            StepAction::EndContactDrag => {
                self.contacts.end_drag(&mut self.history);
                return;
            }
            StepAction::AdaptContacts => {
                self.contacts.request_adapt();
                return;
            }
            StepAction::ToggleSection => {
                self.inner.toggle_section();
                return;
            }
            StepAction::SetSimplify(on) => {
                self.export.simplify = on;
                return;
            }
            StepAction::ExportStl => {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title("Export crowns as STL")
                    .pick_folder()
                {
                    self.export.start(dir, &self.case.name, &self.export_plan());
                }
                return;
            }
            StepAction::OpenExportFolder => {
                if let Some(dir) = self.export.summary().map(|s| s.dir.clone())
                    && let Err(e) = export_step::open_folder(&dir)
                {
                    self.fail(e);
                }
                return;
            }
            StepAction::ClearMargin => {
                if let Some(tooth) = self.tooth {
                    self.margin
                        .clear(&mut self.case.design, &mut self.history, tooth);
                }
                return;
            }
        };
        self.workflow_error = result.err().map(|e| capitalize(&e.to_string()));
        self.enter_step(tooth);
    }

    /// Picks the tooth for the active step (`tooth` when it is one of the step's, else the
    /// first) and starts or leaves the margin tool to match.
    fn enter_step(&mut self, tooth: Option<Fdi>) {
        let step = self.session.current();
        self.tooth = step.and_then(|s| {
            tooth
                .filter(|t| s.teeth.contains(t))
                .or_else(|| s.teeth.first().copied())
        });
        self.frame_crown = step
            .is_some_and(|s| matches!(s.id, StepId::Anatomy | StepId::Occlusion | StepId::Finish));
        if step.is_some_and(|s| s.id == StepId::Margin) && self.tooth.is_some() {
            if self.tools.active_name() != Some(MARGIN) {
                self.tools.activate(self.margin.tool());
            }
        } else if step.is_some_and(|s| s.id == StepId::Orient) {
            if self.tools.active_name() != Some(ORIENT) {
                self.tools.activate(self.orient.tool());
            }
        } else if step.is_some_and(|s| s.id == StepId::InsertionAxis) && self.tooth.is_some() {
            if self.tools.active_name() != Some(AXIS) {
                self.tools.activate(self.axis.tool());
            }
        } else if step.is_some_and(|s| s.id == StepId::InnerSurface) && self.tooth.is_some() {
            if self.tools.active_name() != Some(INNER) {
                self.tools.activate(self.inner.tool());
            }
        } else if step.is_some_and(|s| s.id == StepId::Anatomy) && self.tooth.is_some() {
            if self.tools.active_name() != Some(ANATOMY) {
                self.tools.activate(self.anatomy.tool());
            }
        } else if step.is_some_and(|s| s.id == StepId::Occlusion) && self.tooth.is_some() {
            if self.tools.active_name() != Some(CONTACTS) {
                self.tools.activate(self.contacts.tool());
            }
        } else if matches!(
            self.tools.active_name(),
            Some(MARGIN | ORIENT | AXIS | INNER | ANATOMY | CONTACTS)
        ) {
            self.tools.deactivate();
        }
    }

    /// The jaw's occlusal direction, once the jaw is oriented.
    fn occlusal(&self) -> Option<DVec3> {
        self.case
            .design
            .jaw_frame
            .map(|f| DVec3::from_array(f.occlusal))
    }

    /// Direction out of a preparation when the jaw is oriented; without a jaw frame the margin
    /// step falls back to the scan's own occlusal direction (see `margin_step::proposal_axis`).
    fn margin_axis(&self) -> Option<DVec3> {
        self.occlusal()
    }

    /// `--step inner` or `--step anatomy`: once the Insertion axis step has set the tooth's axis,
    /// completes the steps up to the target and opens it on the same tooth.
    fn pending_inner_step(&mut self) {
        let Some(target) = self.pending_inner else {
            return;
        };
        if self.axis.busy() || self.margin.busy() {
            return;
        }
        let Some(tooth) = self.tooth else {
            self.pending_inner = None;
            return;
        };
        let has_axis = self
            .case
            .design
            .teeth
            .get(&tooth)
            .is_some_and(|t| t.insertion_axis.is_some());
        let has_margin = self
            .case
            .design
            .teeth
            .get(&tooth)
            .is_some_and(|t| t.margin.is_some());
        if !has_axis
            && has_margin
            && self
                .axis
                .card(&self.case.design, tooth, None)
                .error
                .is_none()
        {
            return; // The axis search has not started yet.
        }
        self.pending_inner = None;
        while self.session.current().is_some_and(|s| s.id != target) {
            if self.session.complete().is_err() {
                break;
            }
        }
        self.enter_step(Some(tooth));
    }

    /// Every unit of the case with its current crown.
    fn export_plan(&self) -> Vec<PlanItem> {
        export_step::plan(&self.case.units, |tooth| {
            let base = self.anatomy.crown(&self.case, tooth)?;
            Some(
                self.contacts
                    .adapted(&self.case, &self.objects, tooth, &base)
                    .unwrap_or(base),
            )
        })
    }

    /// `--step export`: once the Anatomy step has built (or failed to build) the crown, moves
    /// on to Finish & export.
    fn pending_finish_step(&mut self) {
        let Some(target) = self.pending_finish else {
            return;
        };
        if self.pending_inner.is_some() || self.inner.busy() || self.anatomy.busy() {
            return;
        }
        self.pending_finish = None;
        let tooth = self.tooth;
        while self.session.current().is_some_and(|s| s.id != target) {
            if self.session.complete().is_err() {
                break;
            }
        }
        self.enter_step(tooth);
    }

    /// `--export DIR`: once the startup steps are done, writes every built crown.
    fn pending_export_run(&mut self) {
        let waiting = self.objects.iter().any(|o| o.loaded().is_none())
            || self.pending_step.is_some()
            || !self.pending_margin.is_empty()
            || self.pending_inner.is_some()
            || self.pending_finish.is_some()
            || self.margin.busy()
            || self.axis.busy()
            || self.inner.busy()
            || self.anatomy.busy()
            || self.contacts.busy();
        if waiting {
            return;
        }
        if let Some(dir) = self.pending_export.take() {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                self.fail(format!("--export: cannot create {}: {e}", dir.display()));
                return;
            }
            let plan = self.export_plan();
            self.export.start(dir, &self.case.name, &plan);
        }
    }

    /// Opens `--step` for `--tooth` and proposes the `--margin` seeds, once the scans are in.
    fn start_design(&mut self) {
        if let Some(mut target) = self.pending_step.take() {
            // Occlusion and Finish & export need the crowns: open the Anatomy step first and
            // move on once the crown is built.
            if matches!(target, StepId::Finish | StepId::Occlusion)
                && self.session.steps().iter().any(|s| s.id == StepId::Anatomy)
                && self.session.steps().iter().any(|s| s.id == target)
            {
                self.pending_finish = Some(target);
                target = StepId::Anatomy;
            }
            // The inner surface (and the crown on it) needs the tooth's axis: open the Insertion
            // axis step first, which finds the best axis, and move on once it is set.
            if matches!(target, StepId::InnerSurface | StepId::Anatomy)
                && self.session.steps().iter().any(|s| s.id == target)
                && self
                    .session
                    .steps()
                    .iter()
                    .any(|s| s.id == StepId::InsertionAxis)
            {
                self.pending_inner = Some(target);
                target = StepId::InsertionAxis;
            }
            if self.session.steps().iter().any(|s| s.id == target) {
                while self.session.current().is_some_and(|s| s.id != target) {
                    if self.session.complete().is_err() {
                        break;
                    }
                }
                let tooth = self.pending_tooth.take();
                self.enter_step(tooth);
            } else {
                self.fail(format!("--step: the case has no {} step", target.title()));
            }
        }
        if std::mem::take(&mut self.pending_orient) {
            self.start_orient();
        }
        let axis = self.margin_axis();
        for (tooth, seed) in std::mem::take(&mut self.pending_margin) {
            match margin_step::nearest_scan(&self.objects, seed) {
                Some((scan, point)) => self.margin.propose(tooth, scan, point, axis),
                None => self.fail("--margin: no scan to put the seed on".to_owned()),
            }
        }
    }

    /// Clears the scene and makes `case` the open case.
    fn reset(&mut self, case: Case) {
        self.objects.clear();
        self.viewport.clear_hover();
        self.tools.deactivate();
        self.hovered_row = None;
        self.auto_fit = true;
        self.session = Session::for_case(&case);
        self.workflow_error = None;
        self.history.clear();
        self.margin.reset();
        self.orient.reset();
        self.axis.reset();
        self.inner.reset();
        self.anatomy.reset();
        self.contacts.reset();
        self.export.reset();
        self.tooth = None;
        self.case = case;
    }

    fn undo(&mut self) {
        self.history.undo(&mut self.case.design);
    }

    fn redo(&mut self) {
        self.history.redo(&mut self.case.design);
    }

    fn new_case(&mut self) {
        self.reset(Case::new(UNTITLED));
        self.doc = Document::untitled(&self.case);
    }

    fn request_new(&mut self) {
        if self.is_dirty() {
            self.confirm_discard = Some(Discard::New);
        } else {
            self.new_case();
        }
    }

    fn request_open(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open case")
            .add_filter("DentCraft case", &[document::EXTENSION])
            .pick_file()
        else {
            return;
        };
        if self.is_dirty() {
            self.confirm_discard = Some(Discard::Open(path));
        } else {
            self.open_case(path);
        }
    }

    /// Opens a case file and re-imports its scans with their saved roles.
    fn open_case(&mut self, path: PathBuf) {
        match document::load(&path) {
            Ok(case) => {
                let scans = case
                    .scans
                    .iter()
                    .map(|s| (s.file.clone(), s.role))
                    .collect();
                self.reset(case);
                self.doc = Document::at(path, &self.case);
                self.add_scans(scans);
            }
            Err(err) => self.fail(err.to_string()),
        }
    }

    fn save_case(&mut self) {
        let path = match self.doc.path() {
            Some(path) => path.to_path_buf(),
            None => {
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Save case")
                    .add_filter("DentCraft case", &[document::EXTENSION])
                    .set_file_name(document::suggested_file_name(&self.case))
                    .save_file()
                else {
                    return;
                };
                document::with_case_extension(path)
            }
        };
        match document::save(&self.case, &path) {
            Ok(()) => self.doc.mark_saved(path, &self.case),
            Err(err) => self.fail(err.to_string()),
        }
    }

    fn confirm_discard_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.confirm_discard else {
            return;
        };
        let p = self.palette();
        let action = match pending {
            Discard::New => "starting a new case",
            Discard::Open(_) => "opening another case",
        };
        let name = document::display_name(&self.case).to_owned();
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("discard_changes"))
            .frame(
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(RADIUS_LG))
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(
                    RichText::new("Discard unsaved changes?")
                        .heading()
                        .color(p.text),
                );
                ui.label(
                    RichText::new(format!(
                        "\u{201c}{name}\u{201d} has changes that are not saved. \
                         They will be lost by {action}."
                    ))
                    .color(p.text_dim),
                );
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if small_button(ui, "Discard changes", true, true).clicked() {
                        choice = Some(true);
                    }
                    if small_button(ui, "Keep editing", true, false).clicked() {
                        choice = Some(false);
                    }
                });
            });
        if choice.is_none() && modal.should_close() {
            choice = Some(false);
        }
        match choice {
            Some(true) => match self.confirm_discard.take() {
                Some(Discard::New) => self.new_case(),
                Some(Discard::Open(path)) => self.open_case(path),
                None => {}
            },
            Some(false) => self.confirm_discard = None,
            None => {}
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = document::window_title(&self.case, self.is_dirty());
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        self.import(dropped);

        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let keys = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
        ];
        // Ctrl+Shift+Z first: Ctrl+Z also matches while Shift is held.
        let (undo, redo) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Z,
            )) || i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Y));
            let undo = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z));
            (undo, redo)
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        let (preset, fit) = ctx.input(|i| {
            let preset = keys
                .iter()
                .position(|k| i.key_pressed(*k))
                .map(|n| ViewPreset::ALL[n]);
            (preset, i.key_pressed(Key::F))
        });
        if let Some(preset) = preset {
            self.viewport.go_to(preset, self.bounds().as_ref());
        }
        if fit {
            self.viewport.fit(self.bounds().as_ref());
        }
        let (escape, delete) =
            ctx.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Delete)));
        if escape {
            self.tools.escape();
        }
        if delete {
            self.tools.delete();
        }
    }

    /// Row 1: logo, app name, undo and redo, case name, view preset and theme toggle.
    fn app_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        ui.horizontal_centered(|ui| {
            ui.add(
                egui::Image::new(egui::load::SizedTexture::new(
                    self.logo_small.id(),
                    [20.0, 20.0],
                ))
                .corner_radius(CornerRadius::same(4)),
            );
            ui.label(RichText::new("DentCraft").size(13.0).strong().color(p.text));
            ui.add_space(8.0);
            // Quick-access buttons next to the app name, as in SolidCraft.
            let undo = self
                .history
                .undo_label()
                .map(|l| format!("Undo {l}  (Ctrl+Z)"));
            if history_button(ui, Icon::Undo, undo, "Nothing to undo").clicked() {
                self.undo();
            }
            let redo = self
                .history
                .redo_label()
                .map(|l| format!("Redo {l}  (Ctrl+Y)"));
            if history_button(ui, Icon::Redo, redo, "Nothing to redo").clicked() {
                self.redo();
            }
            ui.add_space(4.0);
            ui.add(egui::Separator::default().vertical().spacing(8.0));
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(RichText::new(document::display_name(&self.case)).color(p.text))
                    .truncate(),
            )
            .on_hover_text(match self.doc.path() {
                Some(path) => path.display().to_string(),
                None => "Not saved yet".to_owned(),
            });
            if self.is_dirty() {
                let (dot, _) = ui.allocate_exact_size(vec2(8.0, 16.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, p.accent);
                ui.label(RichText::new("Unsaved").size(SIZE_SMALL).color(p.text_dim));
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let icon = match self.theme {
                    ThemeKind::Dark => Icon::Sun,
                    ThemeKind::Light => Icon::Moon,
                };
                if icon_button(ui, icon, 24.0)
                    .on_hover_text("Switch between dark and light theme")
                    .clicked()
                {
                    self.theme = self.theme.toggled();
                    ui.ctx().set_theme(self.theme.egui_theme());
                }
                ui.add_space(6.0);
                let mut chosen = None;
                let current = self.viewport.preset.map_or("Custom", |v| v.label());
                egui::ComboBox::from_id_salt("view_preset")
                    .selected_text(current)
                    .width(96.0)
                    .show_ui(ui, |ui| {
                        for (n, preset) in ViewPreset::ALL.into_iter().enumerate() {
                            let on = self.viewport.preset == Some(preset);
                            if ui
                                .selectable_label(on, preset.label())
                                .on_hover_text(format!("{} view  ({})", preset.label(), n + 1))
                                .clicked()
                            {
                                chosen = Some(preset);
                            }
                        }
                    })
                    .response
                    .on_hover_text("Camera view");
                ui.label(RichText::new("View").size(SIZE_SMALL).color(p.text_dim));
                if let Some(preset) = chosen {
                    self.run(Command::View(preset));
                }
            });
        });
    }

    /// Row 2: the ribbon.
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        let has_jaw_scan = self.has_jaw_scan();
        let command = RibbonView {
            tab: &mut self.ribbon_tab,
            workspace_menu: &mut self.workspace_menu,
            session: &self.session,
            has_jaw_scan,
            has_units: !self.case.units.is_empty(),
            save_path: self.doc.path().map(|p| p.display().to_string()),
            preset: self.viewport.preset,
            upper_visible: objects::role_visible(&self.objects, ScanRole::Upper),
            lower_visible: objects::role_visible(&self.objects, ScanRole::Lower),
            active_tool: self.tools.active_name(),
        }
        .show(ui);
        if let Some(command) = command {
            self.run(command);
        }
    }

    /// Carries out a ribbon or app bar command with the app's existing actions.
    fn run(&mut self, command: Command) {
        match command {
            Command::New => self.request_new(),
            Command::Open => self.request_open(),
            Command::Save => self.save_case(),
            Command::Import => self.pick_files(),
            Command::Step(action) => self.apply_step_action(action),
            Command::View(preset) => self.viewport.go_to(preset, self.bounds().as_ref()),
            Command::Fit => self.viewport.fit(self.bounds().as_ref()),
            Command::ShowRole(role, visible) => {
                objects::set_role_visible(&mut self.objects, role, visible);
            }
            Command::Orient => {
                if self.tools.active_name() == Some(ORIENT) {
                    self.tools.deactivate();
                } else if let Some(index) = self
                    .session
                    .steps()
                    .iter()
                    .position(|s| s.id == StepId::Orient)
                {
                    self.apply_step_action(StepAction::GoTo(index));
                }
            }
            Command::Margin => {
                if self.tools.active_name() == Some(MARGIN) {
                    self.tools.deactivate();
                } else {
                    match self
                        .session
                        .steps()
                        .iter()
                        .position(|s| s.id == StepId::Margin)
                    {
                        Some(index) => self.apply_step_action(StepAction::Tooth(
                            index,
                            self.tooth.unwrap_or(self.session.steps()[index].teeth[0]),
                        )),
                        None => self.workflow_error = None,
                    }
                }
            }
            Command::Axis => {
                if self.tools.active_name() == Some(AXIS) {
                    self.tools.deactivate();
                } else if let Some(index) = self
                    .session
                    .steps()
                    .iter()
                    .position(|s| s.id == StepId::InsertionAxis)
                {
                    let teeth = &self.session.steps()[index].teeth;
                    let tooth = self.tooth.filter(|t| teeth.contains(t)).unwrap_or(teeth[0]);
                    self.apply_step_action(StepAction::Tooth(index, tooth));
                }
            }
            Command::Inner => {
                if self.tools.active_name() == Some(INNER) {
                    self.tools.deactivate();
                } else if let Some(index) = self
                    .session
                    .steps()
                    .iter()
                    .position(|s| s.id == StepId::InnerSurface)
                {
                    let teeth = &self.session.steps()[index].teeth;
                    let tooth = self.tooth.filter(|t| teeth.contains(t)).unwrap_or(teeth[0]);
                    self.apply_step_action(StepAction::Tooth(index, tooth));
                }
            }
            Command::Anatomy => {
                if self.tools.active_name() == Some(ANATOMY) {
                    self.tools.deactivate();
                } else if let Some(index) = self
                    .session
                    .steps()
                    .iter()
                    .position(|s| s.id == StepId::Anatomy)
                {
                    let teeth = &self.session.steps()[index].teeth;
                    let tooth = self.tooth.filter(|t| teeth.contains(t)).unwrap_or(teeth[0]);
                    self.apply_step_action(StepAction::Tooth(index, tooth));
                }
            }
            Command::Contacts => {
                if self.tools.active_name() == Some(CONTACTS) {
                    self.tools.deactivate();
                } else if let Some(index) = self
                    .session
                    .steps()
                    .iter()
                    .position(|s| s.id == StepId::Occlusion)
                {
                    let teeth = &self.session.steps()[index].teeth;
                    let tooth = self.tooth.filter(|t| teeth.contains(t)).unwrap_or(teeth[0]);
                    self.apply_step_action(StepAction::Tooth(index, tooth));
                }
            }
            Command::Export => {
                if let Some(index) = self
                    .session
                    .steps()
                    .iter()
                    .position(|s| s.id == StepId::Finish)
                {
                    self.apply_step_action(StepAction::GoTo(index));
                }
            }
            Command::Measure => {
                if self.tools.active_name() == Some(MEASURE) {
                    self.tools.deactivate();
                } else {
                    self.tools.activate(Box::<Measure>::default());
                }
            }
        }
    }

    fn case_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        let small = |text: String| RichText::new(text).size(SIZE_SMALL).color(p.text_dim);
        ui.spacing_mut().item_spacing.y = 0.0;

        browser::header(
            ui,
            Section::Case,
            self.sections.open_mut(Section::Case),
            |_| {},
        );
        if self.sections.is_open(Section::Case) {
            browser::body(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.case.name)
                        .hint_text("Case name")
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(10, 6)),
                );
            });
        }

        let n = self.case.units.len();
        let header_top = ui.cursor().top();
        browser::header(
            ui,
            Section::Teeth,
            self.sections.open_mut(Section::Teeth),
            |ui| {
                if n > 0 {
                    ui.label(small(format!("{n} unit{}", if n == 1 { "" } else { "s" })));
                }
            },
        );
        if n == 0 {
            // Until a unit exists, the section pulses to say where to start.
            let rect = egui::Rect::from_min_max(
                egui::pos2(ui.max_rect().left(), header_top),
                egui::pos2(ui.max_rect().right(), ui.cursor().top()),
            );
            let t = ui.input(|i| i.time) as f32;
            let alpha = 0.5 + 0.5 * (t * 3.0).sin();
            let stroke = egui::Stroke::new(2.0, p.accent.gamma_multiply(0.35 + 0.65 * alpha));
            ui.painter()
                .rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.sections.is_open(Section::Teeth) {
            let edit = browser::body(ui, |ui| {
                let edit = chart::tooth_chart(ui, &self.case);
                chart::legend(ui);
                if self.case.units.is_empty() {
                    ui.label(small("Click a tooth to add a restoration.".to_owned()));
                }
                let issues = self.case.validate();
                if !issues.is_empty() {
                    ui.add_space(4.0);
                    issues_box(ui, &issues, &p);
                }
                edit
            });
            if let Some(edit) = edit {
                self.apply_chart_edit(edit);
            }
        }

        browser::header(
            ui,
            Section::Material,
            self.sections.open_mut(Section::Material),
            |_| {},
        );
        if self.sections.is_open(Section::Material) {
            browser::body(ui, |ui| self.material_picker(ui, &p));
        }

        let count = self.objects.len();
        let mut import = false;
        browser::header(
            ui,
            Section::Scans,
            self.sections.open_mut(Section::Scans),
            |ui| {
                import = icon_button(ui, Icon::Plus, 20.0)
                    .on_hover_text("Import scans…")
                    .clicked();
                if count > 0 {
                    ui.label(small(count.to_string()));
                }
            },
        );
        if import {
            self.pick_files();
        }
        if self.sections.is_open(Section::Scans) {
            browser::body(ui, |ui| self.scan_list(ui, &p));
        }
    }

    fn scan_list(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if self.objects.is_empty() {
            ui.label(
                RichText::new("No scans yet. Import or drop STL, PLY or OBJ files.")
                    .size(SIZE_SMALL)
                    .color(p.text_dim),
            );
            return;
        }

        let mut remove = None;
        let mut hovered = None;
        for (index, obj) in self.objects.iter_mut().enumerate() {
            let is_hovered = self.hovered_row == Some(obj.id);
            let frame = egui::Frame::new()
                .fill(if is_hovered { p.hover } else { p.panel_header })
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    if object_row(ui, obj, is_hovered, p) {
                        remove = Some(index);
                    }
                });
            if frame.response.contains_pointer() {
                hovered = Some(obj.id);
            }
            ui.add_space(2.0);
        }
        if hovered != self.hovered_row {
            self.hovered_row = hovered;
            ui.ctx().request_repaint();
        }
        if let Some(index) = remove {
            self.objects.remove(index);
            self.viewport.clear_hover();
        }
    }

    fn workflow_panel(&mut self, ui: &mut egui::Ui) {
        let has_jaw_scan = self.has_jaw_scan();
        let occlusal = self.occlusal();
        let steps = self.session.steps();
        let anatomy = steps.iter().position(|s| s.id == StepId::Anatomy);
        let export = self
            .session
            .current()
            .filter(|s| s.id == StepId::Finish)
            .map(|_| {
                self.export.card(&self.export_plan(), |tooth| {
                    anatomy.filter(|&i| steps[i].teeth.contains(&tooth))
                })
            });
        let action = WorkflowView {
            session: &self.session,
            case: &self.case,
            has_jaw_scan,
            error: self.workflow_error.as_deref(),
            card_height: &mut self.workflow_card_height,
            sections: &mut self.sections,
            tooth: self.tooth,
            margin: self.tooth.map(|t| self.margin.card(t)),
            orient: Some(self.orient.card(self.case.design.jaw_frame.is_some())),
            axis: self
                .tooth
                .map(|t| self.axis.card(&self.case.design, t, occlusal)),
            inner: self
                .tooth
                .map(|t| self.inner.card(&self.case.design, &self.case.material, t)),
            anatomy: self.tooth.map(|t| {
                let inner = self.inner.card(&self.case.design, &self.case.material, t);
                self.anatomy.card(
                    &self.case,
                    t,
                    inner.busy.is_some(),
                    inner.error.filter(|_| inner.has_margin && inner.has_axis),
                )
            }),
            contacts: self.tooth.map(|t| {
                let inner = self.inner.card(&self.case.design, &self.case.material, t);
                let base = self.anatomy.crown(&self.case, t);
                self.contacts.card(
                    &self.case,
                    &self.objects,
                    t,
                    base.as_ref(),
                    inner
                        .busy
                        .or(self.anatomy.busy().then_some("Building crown\u{2026}")),
                )
            }),
            export,
        }
        .show(ui);
        if let Some(action) = action {
            self.apply_step_action(action);
        }
    }

    fn material_picker(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let mut chosen = None;
        egui::ComboBox::from_id_salt("material")
            .selected_text(&self.case.material.name)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                for preset in Material::presets() {
                    let on = preset == self.case.material;
                    if ui.selectable_label(on, &preset.name).clicked() {
                        chosen = Some(preset);
                    }
                }
            });
        if let Some(material) = chosen {
            self.case.material = material;
        }
        let m = &self.case.material;
        let values = [
            ("Cement gap", format!("{:.2} mm", m.cement_gap)),
            (
                "Min occlusal",
                format!("{:.1} mm", m.min_occlusal_thickness),
            ),
            ("Min wall", format!("{:.1} mm", m.min_wall_thickness)),
            ("Min connector", format!("{:.0} mm²", m.min_connector_area)),
        ];
        ui.add_space(4.0);
        let width = (ui.available_width() - 6.0) * 0.5;
        for row in values.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (label, value) in row {
                    egui::Frame::new()
                        .fill(p.panel_header)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.set_width(width - 20.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(
                                    RichText::new(*label)
                                        .size(SIZE_SMALL - 1.0)
                                        .color(p.text_dim),
                                );
                                ui.label(RichText::new(value).color(p.text).strong());
                            });
                        });
                }
            });
            ui.add_space(-2.0);
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let p = self.palette();
        let small = |t: String| RichText::new(t).size(SIZE_SMALL).color(p.text_dim);
        ui.horizontal_centered(|ui| {
            match self.viewport.hover {
                Some(v) => ui.label(
                    small(format!(
                        "x {:>8.3}   y {:>8.3}   z {:>8.3}  mm",
                        v.x, v.y, v.z
                    ))
                    .monospace()
                    .color(p.text),
                ),
                None => ui.label(small("Hover a scan to measure its position".to_owned())),
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ms = frame.info().cpu_usage.unwrap_or(0.0) * 1000.0;
                ui.label(small(format!("frame {ms:.1} ms")));
                ui.label(small("·".to_owned()));
                let triangles: usize = self.objects.iter().map(|o| o.triangle_count()).sum();
                ui.label(small(format!("{} triangles", group_thousands(triangles))));
                ui.label(small("·".to_owned()));
                let n = self.objects.len();
                ui.label(small(format!(
                    "{n} object{}",
                    if n == 1 { "" } else { "s" }
                )));
            });
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let bounds = self.bounds();
        // The camera follows the jaw frame (undo, redo and case load included); a new frame
        // turns the current preset view to it.
        let frame = orient_step::camera_frame(self.case.design.jaw_frame.as_ref());
        let teeth: Vec<Fdi> = self.case.units.iter().map(|u| u.tooth).collect();
        let upper = orient_step::is_upper_jaw(&teeth);
        if self.viewport.camera.frame != frame || self.viewport.camera.upper != upper {
            self.viewport.camera.frame = frame;
            self.viewport.camera.upper = upper;
            if let Some(preset) = self.viewport.preset {
                self.viewport.go_to(preset, bounds.as_ref());
            }
        }
        let options = self.palette().render_options(self.theme);
        let out = self.viewport.show(
            ui,
            rect,
            &self.objects,
            bounds.as_ref(),
            options,
            &mut self.tools,
        );
        if out.interacted {
            self.auto_fit = false;
        }
        let axis = self.margin_axis();
        let tooth = self
            .session
            .current()
            .filter(|s| s.id == StepId::Margin)
            .and(self.tooth);
        self.margin.update(StepFrame {
            design: &mut self.case.design,
            history: &mut self.history,
            objects: &self.objects,
            tooth,
            axis,
        });
        self.orient.update(&mut self.case.design, &mut self.history);
        let occlusal = self.occlusal();
        let tooth = self
            .session
            .current()
            .filter(|s| s.id == StepId::InsertionAxis)
            .and(self.tooth);
        self.axis.update(axis_step::StepFrame {
            design: &mut self.case.design,
            history: &mut self.history,
            objects: &self.objects,
            margin: &mut self.margin,
            tooth,
            occlusal,
        });
        self.pending_inner_step();
        let tooth = self
            .session
            .current()
            .filter(|s| {
                matches!(
                    s.id,
                    StepId::InnerSurface | StepId::Anatomy | StepId::Occlusion
                )
            })
            .and(self.tooth);
        self.inner.update(inner_step::StepFrame {
            design: &self.case.design,
            material: &self.case.material,
            objects: &self.objects,
            margin: &mut self.margin,
            tooth,
        });
        // The crown is built on the inner surface, which the Inner step keeps computing here.
        let tooth = self
            .session
            .current()
            .filter(|s| matches!(s.id, StepId::Anatomy | StepId::Occlusion))
            .and(self.tooth);
        self.anatomy.update(anatomy_step::StepFrame {
            case: &self.case,
            tooth,
            surface: tooth.and_then(|t| self.inner.surface(t)),
        });
        let tooth = self
            .session
            .current()
            .filter(|s| s.id == StepId::Occlusion)
            .and(self.tooth);
        let jaw = tooth.and_then(|t| {
            let id = margin_step::arch_scan(&self.objects, t)?;
            match self.margin.tracer(id, &self.objects) {
                Ok(Some(tracer)) => Some((id, tracer.mesh().clone())),
                _ => None,
            }
        });
        self.contacts.update(contacts_step::StepFrame {
            case: &self.case,
            objects: &self.objects,
            tooth,
            placed: tooth.and_then(|t| self.anatomy.placed(&self.case, t)),
            surface: tooth.and_then(|t| self.inner.surface(t)),
            prep: tooth.and_then(|t| self.inner.prep(t)),
            jaw,
        });
        self.pending_finish_step();
        self.pending_export_run();
        if self.export.update()
            && let Some(s) = self.export.summary()
        {
            eprintln!("dentcraft: export to {}: {}", s.dir.display(), s.line());
        }
        // Entering the step on a tooth: widen the view to take in the axis and its handle.
        if self.axis.take_new_prep()
            && let Some(mut b) = self.bounds()
        {
            for h in self.tools.handles() {
                b.min = b.min.min(h);
                b.max = b.max.max(h);
            }
            self.viewport.fit(Some(&b));
        }
        // A first crown of a tooth, or entering the Anatomy / Finish step: widen the view to
        // take in the whole crown as well as the scans.
        if self.anatomy.take_new_crown() {
            self.frame_crown = true;
        }
        if self.frame_crown {
            let on_anatomy = self
                .session
                .current()
                .is_some_and(|s| matches!(s.id, StepId::Anatomy | StepId::Occlusion));
            let crowns: Vec<Aabb> = self
                .case
                .units
                .iter()
                .filter(|u| !on_anatomy || Some(u.tooth) == self.tooth)
                .filter_map(|u| self.anatomy.crown(&self.case, u.tooth)?.mesh.bounds())
                .collect();
            if let Some(crowns) = objects::union_bounds(crowns)
                && let Some(scene) = self.bounds()
            {
                self.frame_crown = false;
                let both = objects::union_bounds([scene, crowns]);
                self.viewport.fit(both.as_ref());
            }
        }
        let p = self.palette();
        if self.objects.is_empty() {
            let mut clicked = false;
            let area = egui::Rect::from_center_size(rect.center(), vec2(420.0, 260.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Image::new(egui::load::SizedTexture::new(
                            self.logo_large.id(),
                            [72.0, 72.0],
                        ))
                        .corner_radius(CornerRadius::same(16)),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Drop STL, PLY or OBJ scans here")
                            .heading()
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new("Upper and lower jaws, bite registrations and pre-op scans")
                            .color(p.text_dim),
                    );
                    ui.add_space(12.0);
                    clicked = small_button(ui, "Import scans…", true, true).clicked();
                });
            });
            if clicked {
                self.pick_files();
            }
        }
        if ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) {
            let painter = ui.painter_at(rect);
            painter.rect(
                rect.shrink(12.0),
                CornerRadius::same(RADIUS_LG),
                p.accent.gamma_multiply(0.08),
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center_top() + vec2(0.0, 48.0),
                Align2::CENTER_CENTER,
                "Release to import",
                egui::FontId::proportional(16.0),
                p.text,
            );
        }
        self.show_toasts(ui, rect);
    }

    fn show_toasts(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let p = self.palette();
        let mut dismissed = None;
        let width = 520.0_f32.min(rect.width() - 48.0);
        let area = egui::Rect::from_min_size(
            rect.center_top() + vec2(-width * 0.5, 16.0),
            vec2(width, rect.height() * 0.5),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            for (index, toast) in self.toasts.iter().enumerate() {
                egui::Frame::new()
                    .fill(p.panel_header)
                    .stroke(Stroke::new(1.0, p.danger.gamma_multiply(0.7)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        ui.horizontal(|ui| {
                            let (dot, _) =
                                ui.allocate_exact_size(vec2(10.0, 18.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.0, p.danger);
                            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                                if icon_button(ui, Icon::Close, 22.0)
                                    .on_hover_text("Dismiss")
                                    .clicked()
                                {
                                    dismissed = Some(index);
                                }
                                ui.with_layout(
                                    Layout::left_to_right(Align::TOP).with_main_wrap(true),
                                    |ui| {
                                        ui.label(RichText::new(&toast.message).color(p.text));
                                    },
                                );
                            });
                        });
                    });
                ui.add_space(6.0);
            }
        });
        if let Some(index) = dismissed {
            self.toasts.remove(index);
        }
    }

    fn drive_screenshot(&mut self, ctx: &egui::Context) {
        let busy = self.objects.iter().any(|o| o.loaded().is_none())
            || self.pending_measure.is_some()
            || self.pending_step.is_some()
            || !self.pending_margin.is_empty()
            || self.pending_orient
            || self.orient.busy()
            || self.margin.busy()
            || self.axis.busy()
            || self.pending_inner.is_some()
            || self.inner.busy()
            || self.anatomy.busy()
            || self.contacts.busy()
            || self.pending_finish.is_some()
            || self.pending_export.is_some()
            || self.export.busy()
            || self.viewport.camera.is_animating();
        let Some(job) = &mut self.screenshot else {
            return;
        };
        ctx.request_repaint();
        let done = |code: u8| {
            set_exit(&job.exit_code, code);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        };
        if job.exit_code.lock().is_ok_and(|c| c.is_some()) {
            // A load failed: give up now rather than capture an incomplete scene.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if job.started.elapsed() > SCREENSHOT_TIMEOUT {
            eprintln!(
                "dentcraft: screenshot timed out after {} s",
                SCREENSHOT_TIMEOUT.as_secs()
            );
            return done(2);
        }
        let image = job.image.lock().ok().and_then(|mut slot| slot.take());
        if let Some(image) = image {
            match save_png(&image, &job.path) {
                Ok(()) => done(0),
                Err(err) => {
                    eprintln!("dentcraft: cannot write {}: {err}", job.path.display());
                    done(1);
                }
            }
            return;
        }
        if job.requested {
            return;
        }
        job.settled_frames = if busy { 0 } else { job.settled_frames + 1 };
        if job.settled_frames >= SCREENSHOT_SETTLE_FRAMES {
            job.requested = true;
            let slot = job.image.clone();
            let repaint = ctx.clone();
            ctx.request_screenshot(move |image: Arc<ColorImage>| {
                if let Ok(mut s) = slot.lock() {
                    *s = Some(image);
                }
                repaint.request_repaint();
            });
        }
    }
}

/// One row of the Objects list. Returns true when the user asked to remove the object.
fn object_row(ui: &mut egui::Ui, obj: &mut SceneObject, hovered: bool, p: &Palette) -> bool {
    let mut remove = false;
    let loading = obj.loaded().is_none();
    ui.horizontal(|ui| {
        if loading {
            ui.add(egui::Spinner::new().size(18.0).color(p.accent));
        } else {
            egui::color_picker::color_edit_button_srgb(ui, &mut obj.color).on_hover_text("Colour");
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if hovered
                && icon_button(ui, Icon::Close, 24.0)
                    .on_hover_text("Remove from the scene")
                    .clicked()
            {
                remove = true;
            }
            if !loading {
                let icon = if obj.visible { Icon::Eye } else { Icon::EyeOff };
                let tip = if obj.visible { "Hide" } else { "Show" };
                if icon_button(ui, icon, 24.0).on_hover_text(tip).clicked() {
                    obj.visible = !obj.visible;
                }
            }
            let name =
                RichText::new(&obj.name).color(if obj.visible { p.text } else { p.text_dim });
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(name).truncate())
                    .on_hover_text(obj.path.display().to_string());
            });
        });
    });
    if loading {
        ui.label(RichText::new("Loading…").size(SIZE_SMALL).color(p.text_dim));
        return remove;
    }
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("role", obj.id.0))
            .selected_text(obj.role.label())
            .width(150.0)
            .show_ui(ui, |ui| {
                for role in ScanRole::ALL {
                    ui.selectable_value(&mut obj.role, role, role.label());
                }
            });
        ui.spacing_mut().slider_width = (ui.available_width() - 44.0).max(40.0);
        ui.add(egui::Slider::new(&mut obj.opacity, 0.0..=1.0).show_value(false))
            .on_hover_text("Opacity");
        ui.label(
            RichText::new(format!("{:.0}%", obj.opacity * 100.0))
                .size(SIZE_SMALL)
                .color(p.text_dim),
        );
    });
    ui.label(
        RichText::new(format!(
            "{} triangles",
            group_thousands(obj.triangle_count())
        ))
        .size(SIZE_SMALL)
        .color(p.text_dim),
    );
    remove
}

/// Undo or redo button of the app bar: enabled with `tip` when there is a step to take.
fn history_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tip: Option<String>,
    disabled_tip: &str,
) -> egui::Response {
    let enabled = tip.is_some();
    ui.add_enabled_ui(enabled, |ui| icon_button(ui, icon, 24.0))
        .inner
        .on_hover_text(tip.unwrap_or_default())
        .on_disabled_hover_text(disabled_tip)
}

/// Case issues in a calm warning box.
fn issues_box(ui: &mut egui::Ui, issues: &[dentcraft_case::CaseIssue], p: &Palette) {
    egui::Frame::new()
        .fill(p.danger.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, p.danger.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            for issue in issues {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 16.0), egui::Sense::hover());
                    paint_icon(
                        ui.painter(),
                        Icon::Warning,
                        egui::Rect::from_center_size(rect.center(), vec2(12.0, 12.0)),
                        p.danger,
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(capitalize(&issue.to_string()))
                                .size(SIZE_SMALL)
                                .color(p.text),
                        )
                        .wrap(),
                    );
                });
            }
        });
}

/// "pontic 15 …" → "Pontic 15 …".
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn set_exit(slot: &Arc<Mutex<Option<u8>>>, code: u8) {
    if let Ok(mut c) = slot.lock() {
        c.get_or_insert(code);
    }
}

fn save_png(image: &ColorImage, path: &std::path::Path) -> Result<(), String> {
    let [w, h] = image.size;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    let buffer = image::RgbaImage::from_raw(w as u32, h as u32, bytes)
        .ok_or_else(|| "screenshot has an unexpected size".to_owned())?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    buffer
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| e.to_string())
}

/// 1234567 → "1,234,567".
pub fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

impl eframe::App for DentCraftApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive_loads();
        self.handle_input(&ctx);
        let p = self.palette();
        let bar = |m: Margin| egui::Frame::new().fill(p.panel).inner_margin(m);

        egui::Panel::top("app_bar")
            .frame(bar(Margin::symmetric(10, 0)).fill(p.app_bg))
            .exact_size(32.0)
            .show(ui, |ui| self.app_bar(ui));
        egui::Panel::top("ribbon")
            .frame(bar(Margin::ZERO))
            .exact_size(ribbon::HEIGHT)
            .show(ui, |ui| self.ribbon(ui));
        egui::Panel::bottom("status_bar")
            .frame(bar(Margin::symmetric(10, 0)).fill(p.panel_header))
            .exact_size(22.0)
            .show(ui, |ui| self.status_bar(ui, frame));
        egui::Panel::left("case_panel")
            .frame(bar(Margin::ZERO))
            .default_size(320.0)
            .min_size(300.0)
            .max_size(460.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.case_panel(ui));
            });
        egui::Panel::right("workflow_panel")
            .frame(bar(Margin::ZERO))
            .default_size(300.0)
            .min_size(280.0)
            .show(ui, |ui| self.workflow_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.central(ui));

        self.confirm_discard_modal(&ctx);
        self.update_title(&ctx);
        self.drive_screenshot(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_none() {
            storage.set_string(THEME_KEY, self.theme.key().to_owned());
            storage.set_string(SECTIONS_KEY, self.sections.to_key());
        }
    }

    fn persist_egui_memory(&self) -> bool {
        self.screenshot.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }

    #[test]
    fn issues_are_capitalized() {
        assert_eq!(capitalize("pontic 15 floats"), "Pontic 15 floats");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn bundled_logo_decodes() {
        let logo = logo_image().expect("logo decodes");
        assert_eq!(logo.dimensions(), (512, 512));
    }
}
