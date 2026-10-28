//! The application shell: top bar, case panel, workflow panel, viewport and status bar.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dentcraft_case::{Case, Scan, ScanRole};
use dentcraft_mesh::Aabb;
use dentcraft_render::{MeshId, ViewPreset};
use dentcraft_workflow::StepId;
use eframe::egui::{
    self, Align, Align2, ColorImage, CornerRadius, Key, Layout, Margin, RichText, Stroke,
    TextureHandle, TextureOptions, vec2,
};

use crate::loader::Loader;
use crate::objects::{self, ObjectState, SceneObject};
use crate::theme::{self, Palette, RADIUS_PANEL, SIZE_SMALL, ThemeKind};
use crate::viewport::Viewport;
use crate::widgets::{self, Icon, Pill, icon_button, pill, section_label};

const THEME_KEY: &str = "theme";
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(60);
/// Frames to wait after loading and camera motion have settled before capturing.
const SCREENSHOT_SETTLE_FRAMES: u32 = 6;
const STUB_TOOLTIP: &str = "Arrives with the case panel";

/// How the app starts, from the command line.
pub struct StartOptions {
    pub files: Vec<PathBuf>,
    pub view: Option<ViewPreset>,
    pub theme: Option<ThemeKind>,
    pub screenshot: Option<PathBuf>,
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
        let stored = if start.screenshot.is_none() {
            cc.storage
                .and_then(|s| s.get_string(THEME_KEY))
                .and_then(|k| ThemeKind::from_key(&k))
        } else {
            None
        };
        let theme = start.theme.or(stored).unwrap_or_default();
        theme::install(&cc.egui_ctx, theme);

        let initial_view = start.view.unwrap_or(ViewPreset::Occlusal);
        let mut app = Self {
            theme,
            case: Case::new("Untitled case"),
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
        app.import(start.files);
        app
    }

    fn palette(&self) -> Palette {
        self.theme.palette()
    }

    fn import(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if !self.objects.iter().any(|o| o.loaded().is_some()) {
            self.auto_fit = true;
        }
        for path in paths {
            let id = MeshId(self.next_id);
            self.next_id += 1;
            let role = objects::guess_role(&path);
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
        }
        self.case.scans = self
            .objects
            .iter()
            .filter(|o| o.loaded().is_some())
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
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        ui.horizontal_centered(|ui| {
            ui.add(
                egui::Image::new(egui::load::SizedTexture::new(
                    self.logo_small.id(),
                    [30.0, 30.0],
                ))
                .corner_radius(CornerRadius::same(7)),
            );
            ui.label(RichText::new("DentCraft").size(17.0).strong().color(p.ink));
            ui.add_space(12.0);
            ui.add(egui::Separator::default().vertical().spacing(8.0));
            ui.add_space(8.0);
            for label in ["New case", "Open case…", "Save case…"] {
                pill(ui, label, Pill::Secondary, false).on_disabled_hover_text(STUB_TOOLTIP);
            }
            ui.add_space(4.0);
            if pill(ui, "Import scans…", Pill::Primary, true)
                .on_hover_text("Add STL, PLY or OBJ scans to the case")
                .clicked()
            {
                self.pick_files();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let icon = match self.theme {
                    ThemeKind::Dark => Icon::Sun,
                    ThemeKind::Light => Icon::Moon,
                };
                if icon_button(ui, icon, 32.0)
                    .on_hover_text("Switch between dark and light theme")
                    .clicked()
                {
                    self.theme = self.theme.toggled();
                    ui.ctx().set_theme(self.theme.egui_theme());
                }
                if icon_button(ui, Icon::Fit, 32.0)
                    .on_hover_text("Fit the scene in view  (F, or double-click)")
                    .clicked()
                {
                    self.viewport.fit(self.bounds().as_ref());
                }
                ui.add_space(4.0);
                let options: Vec<(ViewPreset, &str, String)> = ViewPreset::ALL
                    .iter()
                    .enumerate()
                    .map(|(n, v)| (*v, v.label(), format!("{} view  ({})", v.label(), n + 1)))
                    .rev()
                    .collect();
                // Right-to-left layout: pass the options reversed so they read left to right.
                if let Some(preset) = widgets::segmented(ui, &options, self.viewport.preset) {
                    self.viewport.go_to(preset, self.bounds().as_ref());
                }
            });
        });
    }

    fn case_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        ui.add_space(4.0);
        section_label(ui, "Case");
        ui.add(
            egui::TextEdit::singleline(&mut self.case.name)
                .hint_text("Case name")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            section_label(ui, "Objects");
            if !self.objects.is_empty() {
                ui.label(
                    RichText::new(self.objects.len().to_string())
                        .size(SIZE_SMALL)
                        .color(p.muted),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, Icon::Plus, 24.0)
                    .on_hover_text("Import scans…")
                    .clicked()
                {
                    self.pick_files();
                }
            });
        });

        if self.objects.is_empty() {
            ui.label(
                RichText::new("No scans yet. Import or drop STL, PLY or OBJ files.")
                    .size(SIZE_SMALL)
                    .color(p.muted),
            );
            return;
        }

        let mut remove = None;
        let mut hovered = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, obj) in self.objects.iter_mut().enumerate() {
                    let is_hovered = self.hovered_row == Some(obj.id);
                    let frame = egui::Frame::new()
                        .fill(if is_hovered { p.raised_hover } else { p.raised })
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            if object_row(ui, obj, is_hovered, &p) {
                                remove = Some(index);
                            }
                        });
                    if frame.response.contains_pointer() {
                        hovered = Some(obj.id);
                    }
                    ui.add_space(2.0);
                }
            });
        if hovered != self.hovered_row {
            self.hovered_row = hovered;
            ui.ctx().request_repaint();
        }
        if let Some(index) = remove {
            self.objects.remove(index);
            self.viewport.hover = None;
        }
    }

    fn workflow_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        ui.add_space(4.0);
        section_label(ui, "Workflow");
        egui::Frame::new()
            .fill(p.raised)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(CornerRadius::same(RADIUS_PANEL))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("GUIDED DESIGN")
                        .size(SIZE_SMALL)
                        .color(p.gold)
                        .extra_letter_spacing(1.2)
                        .strong(),
                );
                ui.label(
                    RichText::new("Step by step, from scans to export")
                        .heading()
                        .color(p.ink),
                );
                ui.label(
                    RichText::new(
                        "Pick the teeth on the chart and DentCraft plans the steps for each \
                         restoration. The guided workflow is coming next.",
                    )
                    .color(p.muted),
                );
                ui.add_space(8.0);
                let has_scans = self.objects.iter().any(|o| o.loaded().is_some());
                for (n, step) in StepId::ALL.iter().enumerate() {
                    let current = n == 0;
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
                        let done = current && has_scans;
                        if current {
                            ui.painter().circle_filled(rect.center(), 11.0, p.gold);
                        } else {
                            ui.painter().circle_stroke(
                                rect.center(),
                                10.5,
                                Stroke::new(1.0, p.line),
                            );
                        }
                        let ink = if current {
                            egui::Color32::from_rgb(0x12, 0x12, 0x12)
                        } else {
                            p.muted
                        };
                        if done {
                            // Painted tick: the default fonts have no check mark glyph.
                            let c = rect.center();
                            ui.painter().line(
                                vec![
                                    c + vec2(-4.5, 0.5),
                                    c + vec2(-1.5, 3.5),
                                    c + vec2(4.5, -3.5),
                                ],
                                Stroke::new(1.8, ink),
                            );
                        } else {
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                (n + 1).to_string(),
                                egui::FontId::proportional(SIZE_SMALL),
                                ink,
                            );
                        }
                        let title = RichText::new(step.title());
                        ui.label(if current {
                            title.color(p.ink).strong()
                        } else {
                            title.color(p.muted)
                        });
                    });
                }
                ui.add_space(8.0);
                pill(ui, "Start design", Pill::Primary, false)
                    .on_disabled_hover_text("Arrives with the workflow panel");
            });
    }

    fn status_bar(&self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let p = self.palette();
        let small = |t: String| RichText::new(t).size(SIZE_SMALL).color(p.muted);
        ui.horizontal_centered(|ui| {
            match self.viewport.hover {
                Some(v) => ui.label(
                    small(format!(
                        "x {:>8.3}   y {:>8.3}   z {:>8.3}  mm",
                        v.x, v.y, v.z
                    ))
                    .monospace()
                    .color(p.ink),
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
        let options = self.palette().render_options(self.theme);
        let out = self
            .viewport
            .show(ui, rect, &self.objects, bounds.as_ref(), options);
        if out.interacted {
            self.auto_fit = false;
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
                            .color(p.ink),
                    );
                    ui.label(
                        RichText::new("Upper and lower jaws, bite registrations and pre-op scans")
                            .color(p.muted),
                    );
                    ui.add_space(12.0);
                    clicked = pill(ui, "Import scans…", Pill::Primary, true).clicked();
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
                CornerRadius::same(RADIUS_PANEL),
                p.gold.gamma_multiply(0.08),
                Stroke::new(2.0, p.gold),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center_top() + vec2(0.0, 48.0),
                Align2::CENTER_CENTER,
                "Release to import",
                egui::FontId::proportional(16.0),
                p.ink,
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
                    .fill(p.raised)
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
                                        ui.label(RichText::new(&toast.message).color(p.ink));
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
            ui.add(egui::Spinner::new().size(18.0).color(p.gold));
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
            let name = RichText::new(&obj.name).color(if obj.visible { p.ink } else { p.muted });
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(name).truncate())
                    .on_hover_text(obj.path.display().to_string());
            });
        });
    });
    if loading {
        ui.label(RichText::new("Loading…").size(SIZE_SMALL).color(p.muted));
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
                .color(p.muted),
        );
    });
    ui.label(
        RichText::new(format!(
            "{} triangles",
            group_thousands(obj.triangle_count())
        ))
        .size(SIZE_SMALL)
        .color(p.muted),
    );
    remove
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
        let bar = |m: Margin| egui::Frame::new().fill(p.surface).inner_margin(m);

        egui::Panel::top("top_bar")
            .frame(bar(Margin::symmetric(16, 10)))
            .exact_size(56.0)
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status_bar")
            .frame(bar(Margin::symmetric(16, 4)))
            .exact_size(28.0)
            .show(ui, |ui| self.status_bar(ui, frame));
        egui::Panel::left("case_panel")
            .frame(bar(Margin::same(16)))
            .default_size(320.0)
            .min_size(260.0)
            .show(ui, |ui| self.case_panel(ui));
        egui::Panel::right("workflow_panel")
            .frame(bar(Margin::same(16)))
            .default_size(300.0)
            .min_size(240.0)
            .show(ui, |ui| self.workflow_panel(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.central(ui));

        self.drive_screenshot(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_none() {
            storage.set_string(THEME_KEY, self.theme.key().to_owned());
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
    fn bundled_logo_decodes() {
        let logo = logo_image().expect("logo decodes");
        assert_eq!(logo.dimensions(), (512, 512));
    }
}
