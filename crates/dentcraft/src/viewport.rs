//! The 3D viewport: wgpu paint callback, camera interaction, hover picking, tools and overlays.

use std::collections::HashMap;
use std::sync::Arc;

use dentcraft_mesh::Aabb;
use dentcraft_render::{
    Camera, DrawItem, MeshData, MeshId, MeshRenderer, Overlay, RenderOptions, ViewPreset,
};
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Ui,
    vec2,
};
use eframe::egui_wgpu::{self, wgpu};
use glam::{DVec3, Vec2};

use crate::objects::SceneObject;
use crate::theme::{RADIUS_LG, SIZE_BODY};
use crate::tools::{self, HANDLE_RADIUS, Pointer, SurfaceHit, Tools};
use crate::widgets;

/// Seconds of a camera transition to a preset or a fit.
const TRANSITION: f32 = 0.5;

/// GPU state of the viewport, kept in egui-wgpu's callback resources.
struct ViewportResources {
    renderer: MeshRenderer,
    /// The data each mesh was uploaded from; a different allocation (a tool's colours) uploads
    /// again. Holding the `Arc` keeps its address from being reused.
    uploaded: HashMap<MeshId, Arc<MeshData>>,
}

/// Creates the mesh renderer. Call once at startup with eframe's render state.
pub fn init_gpu(render_state: &egui_wgpu::RenderState) {
    let samples = MeshRenderer::supported_sample_count(&render_state.adapter);
    let renderer = MeshRenderer::new(
        &render_state.device,
        samples,
        Some((render_state.target_format, 1)),
    );
    render_state
        .renderer
        .write()
        .callback_resources
        .insert(ViewportResources {
            renderer,
            uploaded: HashMap::new(),
        });
}

/// One frame of the scene, handed to egui-wgpu.
struct SceneCallback {
    camera: Camera,
    items: Vec<(DrawItem, Arc<MeshData>)>,
    /// Line widths and handle radii in points; scaled to pixels when rendering.
    overlays: Vec<Overlay>,
    options: RenderOptions,
    size_points: egui::Vec2,
}

/// `overlay` with its screen sizes multiplied by `scale`.
fn scale_overlay(overlay: &Overlay, scale: f32) -> Overlay {
    let mut o = overlay.clone();
    match &mut o {
        Overlay::Polyline { width_px, .. } => *width_px *= scale,
        Overlay::Points { radius_px, .. } => *radius_px *= scale,
    }
    o
}

impl egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<ViewportResources>() else {
            return Vec::new();
        };
        // Keep the GPU meshes in step with the objects: drop removed ones, upload new or changed
        // ones.
        let stale: Vec<MeshId> = res
            .uploaded
            .keys()
            .filter(|id| !self.items.iter().any(|(i, _)| i.mesh == **id))
            .copied()
            .collect();
        for id in stale {
            res.renderer.remove(id);
            res.uploaded.remove(&id);
        }
        for (item, data) in &self.items {
            if !res
                .uploaded
                .get(&item.mesh)
                .is_some_and(|d| Arc::ptr_eq(d, data))
            {
                res.renderer.upload(device, item.mesh, data);
                res.uploaded.insert(item.mesh, data.clone());
            }
        }
        let draws: Vec<DrawItem> = self.items.iter().map(|(i, _)| *i).collect();
        let ppp = screen.pixels_per_point;
        let overlays: Vec<Overlay> = self
            .overlays
            .iter()
            .map(|o| scale_overlay(o, ppp))
            .collect();
        let size = [
            (self.size_points.x * ppp).round().max(1.0) as u32,
            (self.size_points.y * ppp).round().max(1.0) as u32,
        ];
        res.renderer.render_with_overlays(
            device,
            queue,
            encoder,
            size,
            &self.camera,
            &draws,
            &overlays,
            &self.options,
        );
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(res) = resources.get::<ViewportResources>() {
            res.renderer.blit(render_pass);
        }
    }
}

/// What the user did in the viewport this frame that the app has to act on.
#[derive(Default)]
pub struct ViewportOutput {
    /// The camera was moved by hand.
    pub interacted: bool,
}

pub struct Viewport {
    pub camera: Camera,
    /// Preset the camera was last sent to; cleared once the user orbits.
    pub preset: Option<ViewPreset>,
    /// Surface point under the pointer, in millimetres.
    pub hover: Option<DVec3>,
    /// The scan surface under the pointer, with the object it belongs to.
    hit: Option<SurfaceHit>,
    last_pointer: Option<Pos2>,
    gpu: bool,
}

impl Viewport {
    pub fn new(gpu: bool, preset: ViewPreset) -> Self {
        let mut camera = Camera::default();
        camera.set_view(preset, None);
        Self {
            camera,
            preset: Some(preset),
            hover: None,
            hit: None,
            last_pointer: None,
            gpu,
        }
    }

    /// Forgets the surface point under the pointer, e.g. after the scans changed.
    pub fn clear_hover(&mut self) {
        self.hover = None;
        self.hit = None;
    }

    /// Smoothly turns to `preset`, framing `bounds`.
    pub fn go_to(&mut self, preset: ViewPreset, bounds: Option<&Aabb>) {
        if let Some(b) = bounds {
            self.camera.set_scene(b);
        }
        self.camera.animate_to(preset, bounds, TRANSITION);
        self.preset = Some(preset);
        self.last_pointer = None;
    }

    /// Smoothly frames `bounds` from the current direction.
    pub fn fit(&mut self, bounds: Option<&Aabb>) {
        if let Some(b) = bounds {
            self.camera.animate_fit(b, TRANSITION);
            self.last_pointer = None;
        }
    }

    /// Draws the scene into `rect` and handles mouse interaction, passing tool events to `tools`.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        objects: &[SceneObject],
        bounds: Option<&Aabb>,
        options: RenderOptions,
        tools: &mut Tools,
    ) -> ViewportOutput {
        let mut out = ViewportOutput::default();
        let response = ui.interact(
            rect,
            ui.scope_id().with("viewport"),
            Sense::click_and_drag(),
        );
        self.camera.aspect = rect.width().max(1.0) / rect.height().max(1.0);
        let min = Vec2::new(rect.left(), rect.top());
        let size = Vec2::new(rect.width(), rect.height());
        let px = |p: Pos2| Vec2::new(p.x, p.y);

        // A primary drag that starts on a tool handle moves the handle instead of orbiting.
        let handles = tools.handles();
        let handle_at = |camera: &Camera, p: Pos2| {
            tools::pick_handle(camera, min, size, &handles, px(p), HANDLE_RADIUS)
        };
        if response.drag_started_by(PointerButton::Primary)
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && let Some(handle) = handle_at(&self.camera, origin)
        {
            let pixel = px(origin);
            let hit = tools::surface_hit(objects, &self.camera, pixel, min, size);
            let ray = Some(ray_at(&self.camera, pixel, min, size));
            tools.drag_start(handle, &Pointer { pixel, hit, ray });
        }
        let handle_drag = tools.dragging().is_some();

        let delta = response.drag_delta();
        let drag = Vec2::new(delta.x, delta.y);
        let mut moved = false;
        if response.dragged_by(PointerButton::Primary) && !handle_drag && drag != Vec2::ZERO {
            self.camera.orbit(drag);
            self.preset = None;
            moved = true;
        }
        if (response.dragged_by(PointerButton::Secondary)
            || response.dragged_by(PointerButton::Middle))
            && drag != Vec2::ZERO
        {
            self.camera.pan(drag, rect.height());
            moved = true;
        }
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let mut factor = 1.0;
            if scroll != 0.0 {
                factor *= (-scroll * 0.0015).exp();
            }
            if pinch != 1.0 {
                factor /= pinch;
            }
            if factor != 1.0 {
                let ndc = Camera::pixel_to_ndc(px(pointer), min, size);
                self.camera.zoom_at(factor, ndc);
                moved = true;
            }
        }
        if response.double_clicked() {
            self.fit(bounds);
            moved = true;
        }
        out.interacted = moved;

        let dt = ui.input(|i| i.stable_dt).min(1.0 / 30.0);
        let animating = self.camera.update(dt);
        if animating {
            ui.ctx().request_repaint();
        }

        // Surface picking: only when the pointer or the view changed, and not while orbiting or
        // panning. A dragged handle follows the surface under the pointer.
        let orbiting = response.dragged() && !handle_drag;
        let pointer = if handle_drag {
            response.interact_pointer_pos()
        } else {
            response.hover_pos().filter(|_| !orbiting)
        };
        if pointer != self.last_pointer || moved || animating {
            self.last_pointer = pointer;
            self.hit =
                pointer.and_then(|p| tools::surface_hit(objects, &self.camera, px(p), min, size));
            self.hover = self.hit.map(|h| h.point);
            let now = pointer.map(|p| Pointer {
                pixel: px(p),
                hit: self.hit,
                ray: Some(ray_at(&self.camera, px(p), min, size)),
            });
            match (&now, handle_drag) {
                (Some(p), true) => tools.drag(p),
                (_, false) => tools.hover(now.as_ref()),
                (None, true) => {}
            }
        }
        if handle_drag && !response.dragged() {
            tools.drag_end(&Pointer {
                pixel: pointer.map_or(Vec2::ZERO, px),
                hit: self.hit,
                ray: None,
            });
        } else if !handle_drag
            && response.clicked_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            let pixel = px(p);
            let hit = tools::surface_hit(objects, &self.camera, pixel, min, size);
            let ray = Some(ray_at(&self.camera, pixel, min, size));
            tools.click(&Pointer { pixel, hit, ray });
        }

        // Cursor: a hand over handles, a crosshair while a tool picks surfaces.
        if tools.dragging().is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if let Some(p) = response.hover_pos().filter(|_| !orbiting) {
            if handle_at(&self.camera, p).is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            } else if tools.picks_surface() {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            }
        }

        if self.gpu {
            let colors = tools.vertex_colors();
            let items = objects
                .iter()
                .filter_map(|o| {
                    let l = o.loaded()?;
                    let mut item = o.draw_item();
                    Some(match &colors {
                        Some(c) if c.scan == o.id => {
                            item.vertex_colors = true;
                            (item, c.data.clone())
                        }
                        _ => (item, l.render.clone()),
                    })
                })
                .collect();
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                rect,
                SceneCallback {
                    camera: self.camera.clone(),
                    items,
                    overlays: tools.overlays(&widgets::palette(ui)),
                    options,
                    size_points: rect.size(),
                },
            ));
        } else {
            let p = widgets::palette(ui);
            ui.painter().rect_filled(rect, 0.0, p.viewport_bottom);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "3D view unavailable: no compatible GPU was found",
                egui::FontId::proportional(14.0),
                p.text_dim,
            );
        }

        if !tools.picks_surface() {
            self.paint_hover_marker(ui, rect);
        }
        self.paint_labels(ui, rect, tools);
        paint_axis_gizmo(ui, rect, &self.camera);
        out
    }

    /// Screen position of a world point in `rect`, or `None` behind the camera.
    fn to_screen(&self, rect: Rect, point: DVec3) -> Option<Pos2> {
        let min = Vec2::new(rect.left(), rect.top());
        let size = Vec2::new(rect.width(), rect.height());
        self.camera
            .project_to_pixel(point.as_vec3(), min, size)
            .map(|p| Pos2::new(p.x, p.y))
    }

    /// The active tool's labels, each in a theme pill just above and right of its anchor.
    fn paint_labels(&self, ui: &Ui, rect: Rect, tools: &Tools) {
        let p = widgets::palette(ui);
        let painter = ui.painter_at(rect);
        for label in tools.labels() {
            let Some(at) = self.to_screen(rect, label.anchor) else {
                continue;
            };
            let galley =
                painter.layout_no_wrap(label.text, FontId::proportional(SIZE_BODY), p.text);
            let pill = Rect::from_min_size(
                at + vec2(10.0, -galley.size().y - 18.0),
                galley.size() + vec2(18.0, 8.0),
            );
            painter.rect(
                pill,
                CornerRadius::same(RADIUS_LG),
                p.panel_header,
                Stroke::new(1.0, p.accent),
                egui::StrokeKind::Inside,
            );
            painter.galley(pill.min + vec2(9.0, 4.0), galley, p.text);
        }
    }

    fn paint_hover_marker(&self, ui: &Ui, rect: Rect) {
        let Some(point) = self.hover else { return };
        let Some(pos) = self.to_screen(rect, point) else {
            return;
        };
        let p = widgets::palette(ui);
        let painter = ui.painter_at(rect);
        painter.circle_stroke(pos, 5.0, Stroke::new(1.5, p.accent));
        painter.circle_filled(pos, 1.5, p.accent);
    }
}

/// X/Y/Z arrows in the bottom-left corner, matching the camera orientation.
fn paint_axis_gizmo(ui: &Ui, rect: Rect, camera: &Camera) {
    let center = rect.left_bottom() + vec2(52.0, -52.0);
    let radius = 30.0;
    let painter = ui.painter_at(rect);
    let colors = [
        Color32::from_rgb(0xe0, 0x6c, 0x64),
        Color32::from_rgb(0x8c, 0xc0, 0x7c),
        Color32::from_rgb(0x6c, 0x9e, 0xe0),
    ];
    let mut axes: Vec<(usize, glam::Vec2, f32)> = camera
        .axis_gizmo()
        .into_iter()
        .enumerate()
        .map(|(i, (dir, depth))| (i, dir, depth))
        .collect();
    // Back to front, so nearer axes are drawn on top.
    axes.sort_by(|a, b| a.2.total_cmp(&b.2));
    for (i, dir, depth) in axes {
        let tip = center + vec2(dir.x, -dir.y) * radius;
        let color = if depth < -0.2 {
            colors[i].gamma_multiply(0.55)
        } else {
            colors[i]
        };
        painter.line_segment([center, tip], Stroke::new(2.0, color));
        painter.circle_filled(tip, 8.0, color);
        widgets::small_text(
            &painter,
            tip,
            ["X", "Y", "Z"][i],
            Color32::from_rgb(0x12, 0x12, 0x12),
        );
    }
}

/// The world ray under a viewport pixel, in double precision.
fn ray_at(camera: &Camera, pixel: Vec2, min: Vec2, size: Vec2) -> (DVec3, DVec3) {
    let (origin, dir) = camera.pixel_ray(pixel, min, size);
    (origin.as_dvec3(), dir.as_dvec3())
}
