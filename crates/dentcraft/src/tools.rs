//! Viewport tools: the active tool gets pointer events with the surface point under the cursor,
//! shows handles the user can drag, and draws overlays, labels and scan colours.
//!
//! A design step plugs in by implementing [`Tool`] and handing it to [`Tools::activate`]. The
//! viewport does the picking: it casts the ray under the cursor, finds the surface hit across the
//! visible scans and the handle under the cursor, and calls the tool. Orbit, pan and zoom stay with
//! the viewport, except that a primary drag starting on a handle goes to the tool.

use std::sync::Arc;

use dentcraft_render::{Camera, ClipPlane, DrawItem, MeshData, MeshId, Overlay};
use eframe::egui::Color32;
use glam::{DVec3, Vec2};

use crate::objects::{SceneObject, pick};
use crate::theme::Palette;

/// Screen distance, in points, within which the pointer grabs a handle.
pub const HANDLE_RADIUS: f32 = 8.0;

/// Name of the Measure tool.
pub const MEASURE: &str = "Measure";

/// A point on a scan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceHit {
    /// Index of the object in the scene list.
    pub object: usize,
    /// In millimetres.
    pub point: DVec3,
}

/// Where the pointer is: its position in the viewport (points, egui coordinates) and the scan
/// surface under it, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub pixel: Vec2,
    pub hit: Option<SurfaceHit>,
    /// The world ray under the pointer (origin, unit direction), when the viewport knows it.
    pub ray: Option<(DVec3, DVec3)>,
}

/// Text drawn in a pill next to a world point.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldLabel {
    pub anchor: DVec3,
    pub text: String,
}

/// Replacement vertex data, with colours, for one scan. The viewport re-uploads the scan when
/// `data` is a different allocation than last frame, so build it once and keep the `Arc`.
#[derive(Clone, Debug)]
pub struct ScanColors {
    pub scan: MeshId,
    pub data: Arc<MeshData>,
}

/// An interactive viewport tool. Every method has a do-nothing default.
pub trait Tool {
    /// Shown in the ribbon and the status bar.
    fn name(&self) -> &'static str;
    /// Whether the tool picks points on scans; the cursor is then a crosshair.
    fn picks_surface(&self) -> bool {
        true
    }
    /// The pointer moved over the viewport (or left it: `None`).
    fn hover(&mut self, _pointer: Option<&Pointer>) {}
    /// A primary click that was not a drag.
    fn click(&mut self, _pointer: &Pointer) {}
    /// World points the user can grab, in the order the drag methods number them.
    fn handles(&self) -> Vec<DVec3> {
        Vec::new()
    }
    /// A primary drag started on handle `handle`.
    fn drag_start(&mut self, _handle: usize, _pointer: &Pointer) {}
    /// The pointer moved while dragging `handle`.
    fn drag(&mut self, _handle: usize, _pointer: &Pointer) {}
    /// The drag of `handle` ended.
    fn drag_end(&mut self, _handle: usize, _pointer: &Pointer) {}
    /// Escape: drop work in progress and return true, or return false to be deactivated.
    fn cancel(&mut self) -> bool {
        false
    }
    /// Enter: accept the work in progress.
    fn confirm(&mut self) {}
    /// Delete: remove what the tool made.
    fn delete(&mut self) {}
    /// Curves and handles to draw over the scans.
    fn overlays(&self, _palette: &Palette) -> Vec<Overlay> {
        Vec::new()
    }
    /// Labels to draw next to world points.
    fn labels(&self) -> Vec<WorldLabel> {
        Vec::new()
    }
    /// Per-vertex colours for a scan.
    fn vertex_colors(&self) -> Option<ScanColors> {
        None
    }
    /// Meshes of the tool's own to draw with the scans.
    fn meshes(&self, _palette: &Palette) -> Vec<(DrawItem, Arc<MeshData>)> {
        Vec::new()
    }
    /// A section plane that cuts the scans and the tool's meshes, for this camera.
    fn clip_plane(&self, _camera: &Camera) -> Option<ClipPlane> {
        None
    }
}

/// The dispatcher: holds the active tool and the handle being dragged.
#[derive(Default)]
pub struct Tools {
    active: Option<Box<dyn Tool>>,
    dragging: Option<usize>,
}

impl Tools {
    pub fn activate(&mut self, tool: Box<dyn Tool>) {
        self.active = Some(tool);
        self.dragging = None;
    }

    pub fn deactivate(&mut self) {
        self.active = None;
        self.dragging = None;
    }

    /// Name of the active tool.
    pub fn active_name(&self) -> Option<&'static str> {
        self.active.as_ref().map(|t| t.name())
    }

    /// Handle being dragged.
    pub fn dragging(&self) -> Option<usize> {
        self.dragging
    }

    pub fn picks_surface(&self) -> bool {
        self.active.as_ref().is_some_and(|t| t.picks_surface())
    }

    pub fn handles(&self) -> Vec<DVec3> {
        self.active
            .as_ref()
            .map(|t| t.handles())
            .unwrap_or_default()
    }

    pub fn hover(&mut self, pointer: Option<&Pointer>) {
        if let Some(t) = &mut self.active {
            t.hover(pointer);
        }
    }

    pub fn click(&mut self, pointer: &Pointer) {
        if let Some(t) = &mut self.active {
            t.click(pointer);
        }
    }

    pub fn drag_start(&mut self, handle: usize, pointer: &Pointer) {
        if let Some(t) = &mut self.active {
            self.dragging = Some(handle);
            t.drag_start(handle, pointer);
        }
    }

    pub fn drag(&mut self, pointer: &Pointer) {
        if let (Some(t), Some(h)) = (&mut self.active, self.dragging) {
            t.drag(h, pointer);
        }
    }

    pub fn drag_end(&mut self, pointer: &Pointer) {
        if let (Some(t), Some(h)) = (&mut self.active, self.dragging.take()) {
            t.drag_end(h, pointer);
        }
    }

    /// Escape: the tool drops its work in progress, or is deactivated when it has none.
    pub fn escape(&mut self) {
        self.dragging = None;
        if let Some(t) = &mut self.active
            && !t.cancel()
        {
            self.deactivate();
        }
    }

    pub fn delete(&mut self) {
        if let Some(t) = &mut self.active {
            t.delete();
        }
    }

    /// Enter: the tool accepts its work in progress.
    pub fn confirm(&mut self) {
        if let Some(t) = &mut self.active {
            t.confirm();
        }
    }

    pub fn overlays(&self, palette: &Palette) -> Vec<Overlay> {
        self.active
            .as_ref()
            .map(|t| t.overlays(palette))
            .unwrap_or_default()
    }

    pub fn labels(&self) -> Vec<WorldLabel> {
        self.active.as_ref().map(|t| t.labels()).unwrap_or_default()
    }

    pub fn vertex_colors(&self) -> Option<ScanColors> {
        self.active.as_ref().and_then(|t| t.vertex_colors())
    }

    pub fn meshes(&self, palette: &Palette) -> Vec<(DrawItem, Arc<MeshData>)> {
        self.active
            .as_ref()
            .map(|t| t.meshes(palette))
            .unwrap_or_default()
    }

    pub fn clip_plane(&self, camera: &Camera) -> Option<ClipPlane> {
        self.active.as_ref().and_then(|t| t.clip_plane(camera))
    }
}

/// The scan surface under a viewport pixel (`viewport_min`/`viewport_size` in the same units).
pub fn surface_hit(
    objects: &[SceneObject],
    camera: &Camera,
    pixel: Vec2,
    viewport_min: Vec2,
    viewport_size: Vec2,
) -> Option<SurfaceHit> {
    let (origin, dir) = camera.pixel_ray(pixel, viewport_min, viewport_size);
    pick(objects, origin.as_dvec3(), dir.as_dvec3())
        .map(|(object, point)| SurfaceHit { object, point })
}

/// The handle nearest to `pixel` on screen within `radius`, ignoring handles behind the camera.
pub fn pick_handle(
    camera: &Camera,
    viewport_min: Vec2,
    viewport_size: Vec2,
    handles: &[DVec3],
    pixel: Vec2,
    radius: f32,
) -> Option<usize> {
    handles
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let at = camera.project_to_pixel(h.as_vec3(), viewport_min, viewport_size)?;
            let d = at.distance(pixel);
            (d <= radius).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The nearest point on any visible, loaded scan to `p`.
pub fn snap_to_surface(objects: &[SceneObject], p: DVec3) -> Option<DVec3> {
    objects
        .iter()
        .filter(|o| o.visible)
        .filter_map(|o| {
            let l = o.loaded()?;
            l.bvh.closest_point(&l.mesh, p)
        })
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .map(|c| c.point)
}

/// sRGB colour as the renderer takes it.
pub fn rgb(c: Color32) -> [f32; 3] {
    [c.r(), c.g(), c.b()].map(|v| v as f32 / 255.0)
}

/// Distance between two surface points: click two points, drag either end along the surface.
#[derive(Debug, Default)]
pub struct Measure {
    points: Vec<DVec3>,
    /// Surface point under the pointer, for the line to it while the second point is chosen.
    hover: Option<DVec3>,
}

impl Measure {
    /// A finished measurement between `a` and `b`.
    pub fn between(a: DVec3, b: DVec3) -> Self {
        Self {
            points: vec![a, b],
            hover: None,
        }
    }

    /// The segment shown: both ends, or the first end and the point under the pointer.
    fn segment(&self) -> Option<(DVec3, DVec3)> {
        match self.points[..] {
            [a, b] => Some((a, b)),
            [a] => self.hover.map(|h| (a, h)),
            _ => None,
        }
    }
}

impl Tool for Measure {
    fn name(&self) -> &'static str {
        MEASURE
    }

    fn hover(&mut self, pointer: Option<&Pointer>) {
        self.hover = pointer.and_then(|p| p.hit).map(|h| h.point);
    }

    fn click(&mut self, pointer: &Pointer) {
        let Some(hit) = pointer.hit else { return };
        if self.points.len() >= 2 {
            self.points.clear();
        }
        self.points.push(hit.point);
    }

    fn handles(&self) -> Vec<DVec3> {
        self.points.clone()
    }

    fn drag(&mut self, handle: usize, pointer: &Pointer) {
        if let (Some(hit), Some(p)) = (pointer.hit, self.points.get_mut(handle)) {
            *p = hit.point;
        }
    }

    fn cancel(&mut self) -> bool {
        let had = !self.points.is_empty();
        self.points.clear();
        had
    }

    fn delete(&mut self) {
        self.points.clear();
    }

    fn overlays(&self, palette: &Palette) -> Vec<Overlay> {
        let mut out = Vec::new();
        if let Some((a, b)) = self.segment() {
            out.push(Overlay::Polyline {
                points: vec![a.as_vec3(), b.as_vec3()],
                closed: false,
                color: rgb(palette.accent),
                width_px: 2.0,
            });
        }
        if !self.points.is_empty() {
            out.push(Overlay::Points {
                points: self.points.iter().map(|p| p.as_vec3()).collect(),
                color: rgb(palette.accent),
                radius_px: 5.0,
                outline: Some([1.0; 3]),
            });
        }
        out
    }

    fn labels(&self) -> Vec<WorldLabel> {
        self.segment()
            .map(|(a, b)| WorldLabel {
                anchor: (a + b) * 0.5,
                text: format!("{:.2} mm", a.distance(b)),
            })
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::Loaded;
    use crate::objects::{ObjectState, STONE};
    use dentcraft_case::ScanRole;
    use dentcraft_mesh::{Bvh, TriMesh};
    use std::path::PathBuf;

    const MIN: Vec2 = Vec2::new(100.0, 50.0);
    const SIZE: Vec2 = Vec2::new(800.0, 600.0);

    fn sphere(radius: f64) -> SceneObject {
        let mesh = TriMesh::uv_sphere(radius, 48, 24);
        SceneObject {
            id: MeshId(1),
            name: "sphere".into(),
            path: PathBuf::new(),
            role: ScanRole::Other,
            color: STONE,
            visible: true,
            opacity: 1.0,
            state: ObjectState::Ready(Box::new(Loaded {
                bvh: Bvh::build(&mesh),
                render: Arc::new(MeshData::from_mesh(&mesh)),
                mesh,
            })),
        }
    }

    fn pointer_at(point: DVec3) -> Pointer {
        Pointer {
            pixel: Vec2::ZERO,
            hit: Some(SurfaceHit { object: 0, point }),
            ray: None,
        }
    }

    #[test]
    fn the_pixel_ray_hits_the_surface_under_the_cursor() {
        let objects = vec![sphere(5.0)];
        let camera = Camera::default();
        let center = MIN + SIZE * 0.5;
        let hit = surface_hit(&objects, &camera, center, MIN, SIZE).expect("centre hits");
        let expected = camera.eye().as_dvec3().normalize() * 5.0;
        assert!(hit.point.distance(expected) < 0.05, "hit {}", hit.point);
        // A point on the sphere projects back to the pixel the ray went through.
        let off = DVec3::new(3.0, 0.0, 4.0);
        let pixel = camera
            .project_to_pixel(off.as_vec3(), MIN, SIZE)
            .expect("in front");
        let hit = surface_hit(&objects, &camera, pixel, MIN, SIZE).unwrap();
        let back = camera
            .project_to_pixel(hit.point.as_vec3(), MIN, SIZE)
            .unwrap();
        assert!(back.distance(pixel) < 0.5, "{back} vs {pixel}");
        assert!(surface_hit(&objects, &camera, MIN + Vec2::splat(2.0), MIN, SIZE).is_none());
    }

    #[test]
    fn handle_picking_prefers_the_nearest_within_the_radius() {
        let camera = Camera::default();
        let center = MIN + SIZE * 0.5;
        let near = DVec3::ZERO;
        // A world point a few pixels right of the centre.
        let right = (camera.right() * 0.4).as_dvec3();
        let px = camera.project_to_pixel(right.as_vec3(), MIN, SIZE).unwrap();
        assert!((3.0..HANDLE_RADIUS).contains(&px.distance(center)));
        let handles = [right, near];
        let pick = |pixel| pick_handle(&camera, MIN, SIZE, &handles, pixel, HANDLE_RADIUS);
        assert_eq!(pick(center), Some(1));
        assert_eq!(pick(px), Some(0));
        // Nothing within the radius.
        assert_eq!(pick(center + Vec2::new(0.0, 40.0)), None);
    }

    #[test]
    fn handles_behind_the_camera_are_ignored() {
        let camera = Camera::default();
        let center = MIN + SIZE * 0.5;
        let behind = (camera.eye() - camera.forward() * 10.0).as_dvec3();
        assert_eq!(
            pick_handle(&camera, MIN, SIZE, &[behind], center, HANDLE_RADIUS),
            None
        );
    }

    #[test]
    fn measure_takes_two_clicks_drags_ends_and_clears() {
        let a = DVec3::new(0.0, 0.0, 0.0);
        let b = DVec3::new(3.0, 4.0, 0.0);
        let mut tools = Tools::default();
        tools.activate(Box::new(Measure::default()));
        tools.click(&pointer_at(a));
        tools.hover(Some(&pointer_at(b)));
        assert_eq!(
            tools.labels()[0].text,
            "5.00 mm",
            "live distance to the pointer"
        );
        tools.click(&pointer_at(b));
        assert_eq!(tools.handles(), vec![a, b]);
        let label = &tools.labels()[0];
        assert_eq!(label.text, "5.00 mm");
        assert_eq!(label.anchor, DVec3::new(1.5, 2.0, 0.0));

        // Drag the second end; a frame with no surface under the pointer leaves it in place.
        tools.drag_start(1, &pointer_at(b));
        tools.drag(&pointer_at(DVec3::new(0.0, 0.0, 2.5)));
        tools.drag(&Pointer {
            pixel: Vec2::ZERO,
            hit: None,
            ray: None,
        });
        tools.drag_end(&pointer_at(DVec3::new(0.0, 0.0, 2.5)));
        assert_eq!(tools.dragging(), None);
        assert_eq!(tools.labels()[0].text, "2.50 mm");

        // A third click starts a new measurement.
        tools.click(&pointer_at(b));
        assert_eq!(tools.handles(), vec![b]);

        // Escape clears, a second Escape leaves the tool.
        tools.escape();
        assert!(tools.handles().is_empty());
        assert_eq!(tools.active_name(), Some("Measure"));
        tools.escape();
        assert!(tools.active_name().is_none());
    }

    #[test]
    fn measure_ignores_clicks_off_the_scans_and_delete_clears() {
        let mut m = Measure::between(DVec3::ZERO, DVec3::X);
        m.click(&Pointer {
            pixel: Vec2::ZERO,
            hit: None,
            ray: None,
        });
        assert_eq!(m.labels()[0].text, "1.00 mm");
        let p = Palette::DARK;
        assert_eq!(m.overlays(&p).len(), 2);
        m.delete();
        assert!(m.overlays(&p).is_empty() && m.labels().is_empty());
    }
}
