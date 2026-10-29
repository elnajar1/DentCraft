//! Orbit camera with dental view presets.
//!
//! World frame (millimetres, right-handed): +X towards the patient's right, +Y anterior (towards
//! the lips), +Z occlusal (up for a lower jaw scan). Scanners do not agree on a frame, so the
//! presets are only as good as the scan's orientation; the Orientation step will fix that later.

use dentcraft_mesh::Aabb;
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};

/// Standard dental views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewPreset {
    /// Looking down onto the chewing surfaces.
    Occlusal,
    /// From the front (lips / cheeks side).
    Facial,
    /// From the tongue side.
    Lingual,
    /// From the patient's left.
    Left,
    /// From the patient's right.
    Right,
    /// From below.
    Bottom,
}

impl ViewPreset {
    pub const ALL: [ViewPreset; 6] = [
        Self::Occlusal,
        Self::Facial,
        Self::Lingual,
        Self::Left,
        Self::Right,
        Self::Bottom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Occlusal => "Occlusal",
            Self::Facial => "Facial",
            Self::Lingual => "Lingual",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Bottom => "Bottom",
        }
    }

    /// Parses a lower-case or capitalised view name (`occlusal`, `Facial`, `front`, …).
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "occlusal" | "top" => Some(Self::Occlusal),
            "facial" | "front" | "buccal" => Some(Self::Facial),
            "lingual" | "back" => Some(Self::Lingual),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    /// Camera-to-world rotation of the view. The camera looks along its local -Z, local +Y is up
    /// on screen.
    pub fn rotation(self) -> Quat {
        // (direction from the target towards the eye, screen up)
        let (back, up) = match self {
            Self::Occlusal => (Vec3::Z, Vec3::Y),
            Self::Bottom => (Vec3::NEG_Z, Vec3::Y),
            Self::Facial => (Vec3::Y, Vec3::Z),
            Self::Lingual => (Vec3::NEG_Y, Vec3::Z),
            Self::Left => (Vec3::NEG_X, Vec3::Z),
            Self::Right => (Vec3::X, Vec3::Z),
        };
        let right = up.cross(back);
        Quat::from_mat3(&Mat3::from_cols(right, up, back)).normalize()
    }
}

/// Position of the camera around its target.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pose {
    target: Vec3,
    rotation: Quat,
    distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Transition {
    from: Pose,
    to: Pose,
    elapsed: f32,
    duration: f32,
}

/// Turntable orbit camera with a perspective projection.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    /// Point the camera orbits around and looks at.
    pub target: Vec3,
    /// Camera-to-world rotation.
    pub rotation: Quat,
    /// Distance from the eye to `target`, in millimetres.
    pub distance: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Viewport width / height.
    pub aspect: f32,
    /// Bounding sphere of the scene, used for clip planes and zoom limits.
    pub scene_center: Vec3,
    pub scene_radius: f32,
    /// Jaw frame to world rotation: view presets and the orbit axis are relative to it. Identity
    /// means the world axes are the jaw axes.
    pub frame: Quat,
    /// The jaw is an upper arch: the side presets (Facial, Lingual, Left, Right) are rolled 180
    /// degrees about the view direction so the crowns point down, as dentists see an upper arch.
    pub upper: bool,
    transition: Option<Transition>,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            rotation: ViewPreset::Occlusal.rotation(),
            distance: 60.0,
            fov_y: 30f32.to_radians(),
            aspect: 1.0,
            scene_center: Vec3::ZERO,
            scene_radius: 20.0,
            frame: Quat::IDENTITY,
            upper: false,
            transition: None,
        }
    }
}

const ORBIT_RADIANS_PER_PIXEL: f32 = 0.008;

impl Camera {
    /// Sets the jaw frame from its axes in world coordinates: patient's right, anterior and
    /// occlusal (unit, right-handed). Non-finite or degenerate input is ignored.
    pub fn set_frame(&mut self, right: Vec3, forward: Vec3, occlusal: Vec3) {
        if !(right.is_finite() && forward.is_finite() && occlusal.is_finite()) {
            return;
        }
        let frame = Quat::from_mat3(&Mat3::from_cols(right, forward, occlusal)).normalize();
        if frame.is_finite() {
            self.frame = frame;
        }
    }

    /// Camera-to-world rotation of `preset` in the jaw frame, rolled for an upper jaw's sides.
    pub fn preset_rotation(&self, preset: ViewPreset) -> Quat {
        let side = !matches!(preset, ViewPreset::Occlusal | ViewPreset::Bottom);
        let roll = if self.upper && side {
            Quat::from_rotation_z(std::f32::consts::PI)
        } else {
            Quat::IDENTITY
        };
        (self.frame * preset.rotation() * roll).normalize()
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.rotation * Vec3::new(0.0, 0.0, self.distance)
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    pub fn view(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.eye()).inverse()
    }

    /// Near and far clip distances that enclose the scene sphere.
    pub fn clip_planes(&self) -> (f32, f32) {
        let to_center = (self.scene_center - self.eye()).dot(self.forward());
        let radius = self.scene_radius.max(1e-3) * 1.2;
        let far = (to_center + radius).max(self.distance + radius).max(1.0);
        let near = (to_center - radius).max(far * 1e-4).max(0.01);
        (near, far)
    }

    pub fn projection(&self) -> Mat4 {
        let (near, far) = self.clip_planes();
        glam::camera::rh::proj::directx::perspective(self.fov_y, self.aspect.max(1e-3), near, far)
    }

    pub fn view_projection(&self) -> Mat4 {
        self.projection() * self.view()
    }

    /// Half the visible height at the target's depth, in millimetres.
    fn half_height_at_target(&self) -> f32 {
        self.distance * (self.fov_y * 0.5).tan()
    }

    /// Frames a bounding box: the camera keeps its orientation and looks at the box centre from
    /// a distance at which the whole box is visible.
    pub fn fit(&mut self, bounds: &Aabb) {
        self.transition = None;
        let pose = self.fit_pose(bounds, self.rotation);
        self.set_scene(bounds);
        self.apply(pose);
    }

    /// Sets the scene sphere (clip planes, zoom limits) without moving the camera.
    pub fn set_scene(&mut self, bounds: &Aabb) {
        self.scene_center = bounds.center().as_vec3();
        self.scene_radius = (bounds.diagonal() as f32 * 0.5).max(0.5);
    }

    fn fit_pose(&self, bounds: &Aabb, rotation: Quat) -> Pose {
        let center = bounds.center().as_vec3();
        let half = bounds.size().as_vec3() * 0.5;
        // Extent of the box as seen along the view direction.
        let right = rotation * Vec3::X;
        let up = rotation * Vec3::Y;
        let forward = rotation * Vec3::NEG_Z;
        let extent = |axis: Vec3| half.dot(axis.abs());
        let (half_w, half_h, half_d) = (extent(right), extent(up), extent(forward));
        let tan_y = (self.fov_y * 0.5).tan();
        let tan_x = tan_y * self.aspect.max(1e-3);
        let margin = 1.25;
        let distance = (half_h / tan_y).max(half_w / tan_x) * margin + half_d;
        Pose {
            target: center,
            rotation,
            distance: distance.max(1.0),
        }
    }

    fn pose(&self) -> Pose {
        Pose {
            target: self.target,
            rotation: self.rotation,
            distance: self.distance,
        }
    }

    fn apply(&mut self, pose: Pose) {
        self.target = pose.target;
        self.rotation = pose.rotation;
        self.distance = pose.distance;
    }

    /// Jumps to a preset view, framing `bounds` when given.
    pub fn set_view(&mut self, preset: ViewPreset, bounds: Option<&Aabb>) {
        self.transition = None;
        let rotation = self.preset_rotation(preset);
        let pose = match bounds {
            Some(b) => self.fit_pose(b, rotation),
            None => Pose {
                rotation,
                ..self.pose()
            },
        };
        self.apply(pose);
    }

    /// Starts a smooth transition to a preset view, framing `bounds` when given. Drive it with
    /// [`Camera::update`].
    pub fn animate_to(&mut self, preset: ViewPreset, bounds: Option<&Aabb>, duration: f32) {
        let rotation = self.preset_rotation(preset);
        let to = match bounds {
            Some(b) => self.fit_pose(b, rotation),
            None => Pose {
                rotation,
                ..self.pose()
            },
        };
        self.start_transition(to, duration);
    }

    /// Starts a smooth transition that frames `bounds` from the current direction.
    pub fn animate_fit(&mut self, bounds: &Aabb, duration: f32) {
        self.set_scene(bounds);
        let to = self.fit_pose(bounds, self.rotation);
        self.start_transition(to, duration);
    }

    fn start_transition(&mut self, to: Pose, duration: f32) {
        if duration <= 0.0 {
            self.transition = None;
            self.apply(to);
            return;
        }
        self.transition = Some(Transition {
            from: self.pose(),
            to,
            elapsed: 0.0,
            duration,
        });
    }

    pub fn is_animating(&self) -> bool {
        self.transition.is_some()
    }

    /// Advances a running transition by `dt` seconds. Returns true while it is still running.
    pub fn update(&mut self, dt: f32) -> bool {
        let Some(mut t) = self.transition else {
            return false;
        };
        t.elapsed += dt.max(0.0);
        let s = (t.elapsed / t.duration).clamp(0.0, 1.0);
        // Ease in and out (smootherstep).
        let e = s * s * s * (s * (s * 6.0 - 15.0) + 10.0);
        let mut to_rotation = t.to.rotation;
        if t.from.rotation.dot(to_rotation) < 0.0 {
            to_rotation = -to_rotation;
        }
        self.rotation = t.from.rotation.slerp(to_rotation, e).normalize();
        self.target = t.from.target.lerp(t.to.target, e);
        self.distance =
            (t.from.distance.ln() + (t.to.distance.ln() - t.from.distance.ln()) * e).exp();
        if s >= 1.0 {
            self.apply(t.to);
            self.transition = None;
            false
        } else {
            self.transition = Some(t);
            true
        }
    }

    /// Turntable rotation from a mouse drag in pixels: horizontal drags spin around the jaw's
    /// occlusal axis, vertical drags tilt around the screen's horizontal axis.
    pub fn orbit(&mut self, drag_pixels: Vec2) {
        self.transition = None;
        let yaw = Quat::from_axis_angle(
            self.frame * Vec3::Z,
            -drag_pixels.x * ORBIT_RADIANS_PER_PIXEL,
        );
        let pitch = Quat::from_rotation_x(-drag_pixels.y * ORBIT_RADIANS_PER_PIXEL);
        self.rotation = (yaw * self.rotation * pitch).normalize();
    }

    /// Moves the target so that the scene follows the mouse. `viewport_height` is in the same
    /// pixel unit as the drag.
    pub fn pan(&mut self, drag_pixels: Vec2, viewport_height: f32) {
        self.transition = None;
        let mm_per_pixel = 2.0 * self.half_height_at_target() / viewport_height.max(1.0);
        self.target += (-self.right() * drag_pixels.x + self.up() * drag_pixels.y) * mm_per_pixel;
    }

    /// Zooms by `factor` (< 1 moves closer) keeping the point under the cursor fixed.
    /// `cursor_ndc` is in normalised device coordinates (-1..1, +Y up).
    pub fn zoom_at(&mut self, factor: f32, cursor_ndc: Vec2) {
        self.transition = None;
        let min = (self.scene_radius * 0.02).max(0.5);
        let max = self.scene_radius * 40.0 + 10.0;
        let new_distance = (self.distance * factor).clamp(min, max);
        let factor = new_distance / self.distance;
        let half_h = self.half_height_at_target();
        let under_cursor = self.target
            + self.right() * cursor_ndc.x * half_h * self.aspect
            + self.up() * cursor_ndc.y * half_h;
        self.target = under_cursor + (self.target - under_cursor) * factor;
        self.distance = new_distance;
    }

    /// Ray through a point of the viewport in normalised device coordinates: (origin, unit
    /// direction).
    pub fn ray(&self, ndc: Vec2) -> (Vec3, Vec3) {
        let tan = (self.fov_y * 0.5).tan();
        let dir =
            self.forward() + self.right() * ndc.x * tan * self.aspect + self.up() * ndc.y * tan;
        (self.eye(), dir.normalize())
    }

    /// Projects a world point to normalised device coordinates, or `None` behind the camera.
    pub fn project(&self, point: Vec3) -> Option<Vec2> {
        let clip = self.view_projection() * point.extend(1.0);
        (clip.w > 1e-6).then(|| Vec2::new(clip.x / clip.w, clip.y / clip.w))
    }

    /// Normalised device coordinates of a point in a viewport rectangle given by its top-left
    /// corner and size (+Y down, like egui; pixels or points alike).
    pub fn pixel_to_ndc(pixel: Vec2, viewport_min: Vec2, viewport_size: Vec2) -> Vec2 {
        let uv = (pixel - viewport_min) / viewport_size.max(Vec2::splat(1e-6));
        Vec2::new(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0)
    }

    /// Inverse of [`Camera::pixel_to_ndc`].
    pub fn ndc_to_pixel(ndc: Vec2, viewport_min: Vec2, viewport_size: Vec2) -> Vec2 {
        viewport_min + Vec2::new(ndc.x + 1.0, 1.0 - ndc.y) * 0.5 * viewport_size
    }

    /// Ray through a point of a viewport rectangle (see [`Camera::pixel_to_ndc`]). Uses the
    /// rectangle's aspect ratio, as the renderer does, whatever `aspect` holds.
    pub fn pixel_ray(&self, pixel: Vec2, viewport_min: Vec2, viewport_size: Vec2) -> (Vec3, Vec3) {
        self.with_aspect(viewport_size)
            .ray(Self::pixel_to_ndc(pixel, viewport_min, viewport_size))
    }

    /// Projects a world point into a viewport rectangle, or `None` behind the camera. Uses the
    /// rectangle's aspect ratio, as the renderer does.
    pub fn project_to_pixel(
        &self,
        point: Vec3,
        viewport_min: Vec2,
        viewport_size: Vec2,
    ) -> Option<Vec2> {
        self.with_aspect(viewport_size)
            .project(point)
            .map(|ndc| Self::ndc_to_pixel(ndc, viewport_min, viewport_size))
    }

    fn with_aspect(&self, viewport_size: Vec2) -> Self {
        Self {
            aspect: viewport_size.x / viewport_size.y.max(1e-6),
            transition: None,
            ..self.clone()
        }
    }

    /// Screen directions of the world axes for an orientation gizmo: (+X, +Y, +Z), each as a 2D
    /// vector with +Y up and length ≤ 1, plus its depth towards the viewer.
    pub fn axis_gizmo(&self) -> [(Vec2, f32); 3] {
        let inv = self.rotation.inverse();
        [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| {
            let v = inv * axis;
            (Vec2::new(v.x, v.y), v.z)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn cube() -> Aabb {
        Aabb {
            min: DVec3::new(-5.0, -5.0, -5.0),
            max: DVec3::new(5.0, 5.0, 5.0),
        }
    }

    #[test]
    fn presets_are_orthonormal_and_look_at_the_expected_side() {
        for preset in ViewPreset::ALL {
            let r = preset.rotation();
            assert!((r.length() - 1.0).abs() < 1e-5);
            let back = r * Vec3::Z;
            let expected = match preset {
                ViewPreset::Occlusal => Vec3::Z,
                ViewPreset::Bottom => Vec3::NEG_Z,
                ViewPreset::Facial => Vec3::Y,
                ViewPreset::Lingual => Vec3::NEG_Y,
                ViewPreset::Left => Vec3::NEG_X,
                ViewPreset::Right => Vec3::X,
            };
            assert!(back.distance(expected) < 1e-5, "{preset:?}: {back}");
            assert_eq!(ViewPreset::parse(preset.label()), Some(preset));
        }
        // Seen from the front, the patient's left (−X) is on the viewer's right.
        assert!((ViewPreset::Facial.rotation() * Vec3::X).distance(Vec3::NEG_X) < 1e-5);
    }

    #[test]
    fn fit_keeps_the_box_inside_the_view() {
        let mut camera = Camera {
            aspect: 1.6,
            ..Camera::default()
        };
        camera.set_view(ViewPreset::Facial, None);
        camera.fit(&cube());
        assert_eq!(camera.target, Vec3::ZERO);
        for x in [-5.0, 5.0] {
            for y in [-5.0, 5.0] {
                for z in [-5.0, 5.0] {
                    let ndc = camera.project(Vec3::new(x, y, z)).unwrap();
                    assert!(ndc.abs().max_element() < 1.0, "{ndc}");
                }
            }
        }
        let (near, far) = camera.clip_planes();
        assert!(near > 0.0 && near < camera.distance - 5.0 && far > camera.distance + 5.0);
    }

    #[test]
    fn transition_ends_exactly_at_the_preset() {
        let mut camera = Camera::default();
        camera.fit(&cube());
        camera.animate_to(ViewPreset::Left, Some(&cube()), 0.3);
        let mut steps = 0;
        while camera.update(0.05) {
            steps += 1;
            assert!(steps < 100);
        }
        assert!(steps >= 4);
        assert_eq!(camera.rotation, ViewPreset::Left.rotation());
        assert!(!camera.is_animating());
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut camera = Camera {
            aspect: 1.5,
            ..Camera::default()
        };
        camera.fit(&cube());
        let cursor = Vec2::new(0.4, -0.3);
        let (o, d) = camera.ray(cursor);
        // Point where the cursor ray crosses the target plane.
        let t = (camera.target - o).dot(camera.forward()) / d.dot(camera.forward());
        let p = o + d * t;
        camera.zoom_at(0.5, cursor);
        let after = camera.project(p).unwrap();
        assert!(after.distance(cursor) < 1e-4, "{after} vs {cursor}");
    }

    #[test]
    fn pixel_rays_and_projection_round_trip() {
        let (min, size) = (Vec2::new(250.0, 40.0), Vec2::new(900.0, 560.0));
        for preset in ViewPreset::ALL {
            let mut camera = Camera {
                // Deliberately stale: the pixel helpers use the viewport's aspect.
                aspect: 1.0,
                ..Camera::default()
            };
            camera.set_view(preset, Some(&cube()));
            camera.orbit(Vec2::new(37.0, -21.0));
            camera.pan(Vec2::new(15.0, 8.0), size.y);

            // The ray through the middle of the viewport passes through the target.
            let (o, d) = camera.pixel_ray(min + size * 0.5, min, size);
            let off_axis = (camera.target - o).cross(d).length();
            assert!(off_axis < 1e-3, "{preset:?}: {off_axis}");
            let (o2, d2) = camera.ray(Vec2::ZERO);
            assert!(o2.distance(o) < 1e-5 && d2.distance(d) < 1e-6);

            for (x, y) in [
                (0.0, 0.0),
                (0.13, 0.91),
                (0.5, 0.25),
                (0.97, 0.6),
                (1.0, 1.0),
            ] {
                let pixel = min + size * Vec2::new(x, y);
                let (o, d) = camera.pixel_ray(pixel, min, size);
                for t in [
                    camera.distance * 0.7,
                    camera.distance,
                    camera.distance * 1.3,
                ] {
                    let back = camera.project_to_pixel(o + d * t, min, size).unwrap();
                    assert!(back.distance(pixel) < 0.5, "{preset:?}: {pixel} -> {back}");
                }
            }
        }
        let ndc = Vec2::new(-0.3, 0.8);
        let pixel = Camera::ndc_to_pixel(ndc, min, size);
        assert!(Camera::pixel_to_ndc(pixel, min, size).distance(ndc) < 1e-6);
        assert_eq!(Camera::pixel_to_ndc(min, min, size), Vec2::new(-1.0, 1.0));
    }

    #[test]
    fn orbit_and_pan_move_the_view() {
        let mut camera = Camera::default();
        camera.fit(&cube());
        let before = camera.eye();
        camera.orbit(Vec2::new(40.0, 30.0));
        assert!(camera.eye().distance(before) > 1.0);
        assert!((camera.eye() - camera.target).length() - camera.distance < 1e-3);
        let target = camera.target;
        camera.pan(Vec2::new(100.0, 0.0), 500.0);
        assert!(camera.target.distance(target) > 0.1);
    }

    fn legacy_rotation(preset: ViewPreset) -> Quat {
        let (back, up) = match preset {
            ViewPreset::Occlusal => (Vec3::Z, Vec3::Y),
            ViewPreset::Bottom => (Vec3::NEG_Z, Vec3::Y),
            ViewPreset::Facial => (Vec3::Y, Vec3::Z),
            ViewPreset::Lingual => (Vec3::NEG_Y, Vec3::Z),
            ViewPreset::Left => (Vec3::NEG_X, Vec3::Z),
            ViewPreset::Right => (Vec3::X, Vec3::Z),
        };
        Quat::from_mat3(&Mat3::from_cols(up.cross(back), up, back)).normalize()
    }

    /// An upper jaw as scanned: occlusal faces -Z, anterior is -Y.
    fn upper_frame_camera() -> Camera {
        let mut cam = Camera::default();
        cam.set_frame(Vec3::X, Vec3::NEG_Y, Vec3::NEG_Z);
        cam
    }

    #[test]
    fn upper_jaw_side_presets_show_the_crowns_pointing_down() {
        let mut cam = upper_frame_camera();
        let occlusal = Vec3::NEG_Z;
        for preset in ViewPreset::ALL {
            cam.upper = false;
            cam.set_view(preset, None);
            let (plain_up, plain_forward) = (cam.up(), cam.forward());
            cam.upper = true;
            cam.set_view(preset, None);
            assert!(
                cam.forward().distance(plain_forward) < 1e-5,
                "{preset:?} same direction"
            );
            match preset {
                ViewPreset::Occlusal | ViewPreset::Bottom => {
                    assert!(cam.up().distance(plain_up) < 1e-5, "{preset:?} unchanged");
                }
                _ => {
                    // Screen up is away from the biting surface: the crowns point down.
                    assert!(plain_up.dot(occlusal) > 0.99, "{preset:?}");
                    assert!(cam.up().dot(occlusal) < -0.99, "{preset:?}");
                }
            }
        }
    }

    #[test]
    fn identity_frame_leaves_every_preset_unchanged() {
        for preset in ViewPreset::ALL {
            let mut cam = Camera::default();
            assert_eq!(cam.frame, Quat::IDENTITY);
            cam.set_view(preset, None);
            assert!(
                cam.rotation.angle_between(legacy_rotation(preset)) < 1e-5,
                "{preset:?}"
            );
            cam.animate_to(preset, None, 0.0);
            assert!(cam.rotation.angle_between(legacy_rotation(preset)) < 1e-5);
        }
    }

    #[test]
    fn upper_jaw_frame_looks_at_the_biting_surface() {
        let mut cam = upper_frame_camera();
        cam.set_view(ViewPreset::Occlusal, None);
        assert!((cam.forward() - Vec3::Z).length() < 1e-5);
        let mut animated = upper_frame_camera();
        animated.animate_to(ViewPreset::Occlusal, None, 0.0);
        assert!((animated.forward() - Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn presets_follow_the_frame() {
        let mut cam = upper_frame_camera();
        for preset in ViewPreset::ALL {
            cam.set_view(preset, None);
            let expected = cam.frame * (legacy_rotation(preset) * Vec3::NEG_Z);
            assert!((cam.forward() - expected).length() < 1e-5, "{preset:?}");
        }
        // Facial looks from the anterior side towards the back of the jaw: world +Y here.
        cam.set_view(ViewPreset::Facial, None);
        assert!((cam.forward() - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn yaw_orbit_keeps_the_frame_occlusal_axis() {
        let mut cam = upper_frame_camera();
        cam.set_view(ViewPreset::Facial, None);
        let axis = cam.frame * Vec3::Z;
        let before = cam.up().dot(axis);
        cam.orbit(Vec2::new(90.0, 0.0));
        assert!((cam.up().dot(axis) - before).abs() < 1e-5);
        assert!(cam.forward().dot(axis).abs() < 1e-5);
        assert!(cam.forward().dot(Vec3::Y) < 0.99, "the view turned");
    }

    #[test]
    fn set_frame_ignores_non_finite_input() {
        let mut cam = upper_frame_camera();
        let frame = cam.frame;
        cam.set_frame(Vec3::NAN, Vec3::Y, Vec3::Z);
        cam.set_frame(Vec3::X, Vec3::INFINITY, Vec3::Z);
        assert_eq!(cam.frame, frame);
    }
}
