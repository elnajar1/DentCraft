//! The 3D viewport: wgpu paint callback, camera interaction, hover picking and overlays.

use std::collections::HashSet;
use std::sync::Arc;

use dentcraft_mesh::Aabb;
use dentcraft_render::{
    Camera, DrawItem, MeshData, MeshId, MeshRenderer, RenderOptions, ViewPreset,
};
use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, Sense, Stroke, Ui, vec2};
use eframe::egui_wgpu::{self, wgpu};
use glam::{DVec3, Vec2};

use crate::objects::{SceneObject, pick};
use crate::widgets;

/// Seconds of a camera transition to a preset or a fit.
const TRANSITION: f32 = 0.5;

/// GPU state of the viewport, kept in egui-wgpu's callback resources.
struct ViewportResources {
    renderer: MeshRenderer,
    uploaded: HashSet<MeshId>,
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
            uploaded: HashSet::new(),
        });
}

/// One frame of the scene, handed to egui-wgpu.
struct SceneCallback {
    camera: Camera,
    items: Vec<(DrawItem, Arc<MeshData>)>,
    options: RenderOptions,
    size_points: egui::Vec2,
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
        // Keep the GPU meshes in step with the objects: drop removed ones, upload new ones.
        let wanted: HashSet<MeshId> = self.items.iter().map(|(i, _)| i.mesh).collect();
        let stale: Vec<MeshId> = res.uploaded.difference(&wanted).copied().collect();
        for id in stale {
            res.renderer.remove(id);
            res.uploaded.remove(&id);
        }
        for (item, data) in &self.items {
            if res.uploaded.insert(item.mesh) {
                res.renderer.upload(device, item.mesh, data);
            }
        }
        let draws: Vec<DrawItem> = self.items.iter().map(|(i, _)| *i).collect();
        let ppp = screen.pixels_per_point;
        let size = [
            (self.size_points.x * ppp).round().max(1.0) as u32,
            (self.size_points.y * ppp).round().max(1.0) as u32,
        ];
        res.renderer.render(
            device,
            queue,
            encoder,
            size,
            &self.camera,
            &draws,
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
            last_pointer: None,
            gpu,
        }
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

    /// Draws the scene into `rect` and handles mouse interaction.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        objects: &[SceneObject],
        bounds: Option<&Aabb>,
        options: RenderOptions,
    ) -> ViewportOutput {
        let mut out = ViewportOutput::default();
        let response = ui.interact(
            rect,
            ui.scope_id().with("viewport"),
            Sense::click_and_drag(),
        );
        self.camera.aspect = rect.width().max(1.0) / rect.height().max(1.0);

        let ndc = |p: Pos2| {
            Vec2::new(
                (p.x - rect.left()) / rect.width() * 2.0 - 1.0,
                1.0 - (p.y - rect.top()) / rect.height() * 2.0,
            )
        };
        let delta = response.drag_delta();
        let drag = Vec2::new(delta.x, delta.y);
        let mut moved = false;
        if response.dragged_by(PointerButton::Primary) && drag != Vec2::ZERO {
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
                self.camera.zoom_at(factor, ndc(pointer));
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

        // Hover picking: only when the pointer or the view changed.
        let dragging = response.dragged();
        let pointer = response.hover_pos().filter(|_| !dragging);
        if pointer != self.last_pointer || moved || animating {
            self.last_pointer = pointer;
            self.hover = pointer.and_then(|p| {
                let (origin, dir) = self.camera.ray(ndc(p));
                pick(objects, origin.as_dvec3(), dir.as_dvec3()).map(|(_, point)| point)
            });
        }

        if self.gpu {
            let items = objects
                .iter()
                .filter_map(|o| o.loaded().map(|l| (o.draw_item(), l.render.clone())))
                .collect();
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                rect,
                SceneCallback {
                    camera: self.camera.clone(),
                    items,
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

        self.paint_hover_marker(ui, rect);
        paint_axis_gizmo(ui, rect, &self.camera);
        out
    }

    fn paint_hover_marker(&self, ui: &Ui, rect: Rect) {
        let Some(point) = self.hover else { return };
        let Some(ndc) = self.camera.project(point.as_vec3()) else {
            return;
        };
        let pos = Pos2::new(
            rect.left() + (ndc.x + 1.0) * 0.5 * rect.width(),
            rect.top() + (1.0 - ndc.y) * 0.5 * rect.height(),
        );
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
