//! The Orientation step. "Auto orient" finds the jaw's occlusal plane and midline on a worker
//! thread and stores them in the case's design with undo. The viewport tool ([`OrientTool`]) only
//! draws the frame (occlusal and forward arrows, a faint plane ring); the camera follows the
//! stored frame through [`camera_frame`].

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_case::{Arch, Design, Fdi, JawFrame, ScanRole};
use dentcraft_design::JawFrame as Found;
use dentcraft_render::{Camera, Overlay};
use eframe::egui;
use glam::{DVec3, Quat, Vec3};

use crate::history::History;
use crate::objects::SceneObject;
use crate::theme::Palette;
use crate::tools::{Tool, rgb};
use crate::workflow_panel::OrientCard;

/// Name of the orientation tool.
pub const ORIENT: &str = "Orient";
/// Undo label of an automatic orientation.
pub const ORIENT_LABEL: &str = "Orient jaw";
/// Undo label of clearing the orientation.
pub const RESET_LABEL: &str = "Reset orientation";
/// Length of the drawn arrows, in millimetres.
const ARROW_MM: f32 = 8.0;
/// Radius of the drawn occlusal-plane ring, in millimetres.
const RING_MM: f32 = 20.0;
const RING_POINTS: usize = 64;

/// Rotation the camera uses for a stored jaw frame: identity without one.
pub fn camera_frame(frame: Option<&JawFrame>) -> Quat {
    let mut camera = Camera::default();
    if let Some(f) = frame {
        let forward = DVec3::from_array(f.forward);
        let occlusal = DVec3::from_array(f.occlusal);
        camera.set_frame(
            forward.cross(occlusal).as_vec3(),
            forward.as_vec3(),
            occlusal.as_vec3(),
        );
    }
    camera.frame
}

/// Whether the case's restorations sit on the upper jaw (the arch of its first unit).
pub fn is_upper_jaw(teeth: &[Fdi]) -> bool {
    teeth.first().is_some_and(|t| t.arch() == Arch::Upper)
}

/// An arrow from `origin` along `dir` as one polyline: shaft to the tip, then two head strokes
/// (each back from the tip and out to one side of `side`). Five points.
pub fn arrow_points(origin: Vec3, dir: Vec3, side: Vec3, length: f32) -> Vec<Vec3> {
    let tip = origin + dir * length;
    let back = tip - dir * (length * 0.28);
    let wing = side * (length * 0.14);
    vec![origin, tip, back + wing, tip, back - wing]
}

/// A closed ring of `n` points of `radius` round `origin` in the plane spanned by `a` and `b`.
pub fn ring_points(origin: Vec3, a: Vec3, b: Vec3, radius: f32, n: usize) -> Vec<Vec3> {
    (0..n)
        .map(|i| {
            let t = std::f32::consts::TAU * i as f32 / n as f32;
            origin + (a * t.cos() + b * t.sin()) * radius
        })
        .collect()
}

/// Overlays of a stored frame: the occlusal arrow (accent), the forward arrow (danger colour,
/// the palette's only second hue) and the dimmed plane ring.
pub fn frame_overlays(frame: &JawFrame, p: &Palette) -> Vec<Overlay> {
    let origin = DVec3::from_array(frame.origin).as_vec3();
    let forward = DVec3::from_array(frame.forward).as_vec3();
    let occlusal = DVec3::from_array(frame.occlusal).as_vec3();
    let right = forward.cross(occlusal);
    vec![
        Overlay::Polyline {
            points: ring_points(origin, right, forward, RING_MM, RING_POINTS),
            closed: true,
            color: rgb(p.text_dim),
            width_px: 1.5,
        },
        Overlay::Polyline {
            points: arrow_points(origin, occlusal, right, ARROW_MM),
            closed: false,
            color: rgb(p.accent),
            width_px: 3.0,
        },
        Overlay::Polyline {
            points: arrow_points(origin, forward, right, ARROW_MM),
            closed: false,
            color: rgb(p.danger),
            width_px: 3.0,
        },
    ]
}

/// The viewport tool of the Orientation step: draws the frame and picks nothing.
pub struct OrientTool {
    frame: Rc<RefCell<Option<JawFrame>>>,
}

impl Tool for OrientTool {
    fn name(&self) -> &'static str {
        ORIENT
    }

    fn picks_surface(&self) -> bool {
        false
    }

    fn overlays(&self, palette: &Palette) -> Vec<Overlay> {
        self.frame
            .borrow()
            .as_ref()
            .map(|f| frame_overlays(f, palette))
            .unwrap_or_default()
    }
}

/// The scan the case's restorations sit on: the jaw scan of the first unit's arch.
fn jaw_scan(objects: &[SceneObject], teeth: &[Fdi]) -> Option<usize> {
    let role = match teeth.first()?.arch() {
        Arch::Upper => ScanRole::Upper,
        Arch::Lower => ScanRole::Lower,
    };
    objects
        .iter()
        .position(|o| o.role == role && o.loaded().is_some())
}

fn to_case(f: &Found) -> JawFrame {
    JawFrame {
        origin: f.origin.to_array(),
        occlusal: f.occlusal.to_array(),
        forward: f.forward.to_array(),
    }
}

/// Sets (or with `None` clears) the jaw frame and records the edit as `label`.
pub fn set_frame(design: &mut Design, history: &mut History, frame: Option<JawFrame>, label: &str) {
    let before = design.clone();
    design.jaw_frame = frame;
    history.record(label, before, design, None);
}

/// The app side of the Orientation step: the worker job and the undoable edits.
pub struct OrientStep {
    shown: Rc<RefCell<Option<JawFrame>>>,
    running: bool,
    tx: Sender<Result<Found, String>>,
    rx: Receiver<Result<Found, String>>,
    error: Option<String>,
    repaint: Option<egui::Context>,
}

impl OrientStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self {
            shown: Rc::default(),
            running: false,
            tx,
            rx,
            error: None,
            repaint,
        }
    }

    /// Forgets the open case's job and error.
    pub fn reset(&mut self) {
        *self = Self::new(self.repaint.take());
    }

    pub fn tool(&self) -> Box<dyn Tool> {
        Box::new(OrientTool {
            frame: self.shown.clone(),
        })
    }

    pub fn busy(&self) -> bool {
        self.running
    }

    pub fn card(&self, has_frame: bool) -> OrientCard {
        OrientCard {
            busy: self.running.then_some("Finding the occlusal plane…"),
            error: self.error.clone(),
            has_frame,
        }
    }

    /// Finds the frame of the jaw scan the `teeth` sit on, on a worker thread.
    pub fn start(&mut self, objects: &[SceneObject], teeth: &[Fdi]) {
        if self.running {
            return;
        }
        let Some(mesh) = jaw_scan(objects, teeth)
            .and_then(|i| objects[i].loaded())
            .map(|l| l.mesh.clone())
        else {
            self.error = Some("No jaw scan to orient".to_owned());
            return;
        };
        self.error = None;
        self.running = true;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let result = dentcraft_design::jaw_frame(&mesh).map_err(|e| e.to_string());
            let _ = tx.send(result);
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }

    /// Clears the orientation.
    pub fn clear(&mut self, design: &mut Design, history: &mut History) {
        self.error = None;
        set_frame(design, history, None, RESET_LABEL);
    }

    /// Runs one frame: takes a finished job's result and shows the stored frame to the tool.
    pub fn update(&mut self, design: &mut Design, history: &mut History) {
        while let Ok(result) = self.rx.try_recv() {
            self.running = false;
            match result {
                Ok(found) => {
                    self.error = None;
                    set_frame(design, history, Some(to_case(&found)), ORIENT_LABEL);
                }
                Err(e) => self.error = Some(capitalize(&e)),
            }
        }
        let mut shown = self.shown.borrow_mut();
        if *shown != design.jaw_frame {
            *shown = design.jaw_frame;
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(origin: [f64; 3], occlusal: [f64; 3], forward: [f64; 3]) -> JawFrame {
        JawFrame {
            origin,
            occlusal,
            forward,
        }
    }

    #[test]
    fn no_frame_and_world_axes_give_identity() {
        assert_eq!(camera_frame(None), Quat::IDENTITY);
        // forward +Y, occlusal +Z: right = Y x Z = +X.
        let world = frame([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        assert!(camera_frame(Some(&world)).angle_between(Quat::IDENTITY) < 1e-6);
    }

    #[test]
    fn frame_quaternion_maps_axes_right_handed() {
        // forward -Y, occlusal -Z: right = (-Y) x (-Z) = +X.
        let upper = frame([0.0; 3], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0]);
        let q = camera_frame(Some(&upper));
        assert!(q.is_normalized());
        let (x, y, z) = (q * Vec3::X, q * Vec3::Y, q * Vec3::Z);
        assert!(x.distance(Vec3::X) < 1e-6, "right {x}");
        assert!(y.distance(Vec3::NEG_Y) < 1e-6, "forward {y}");
        assert!(z.distance(Vec3::NEG_Z) < 1e-6, "occlusal {z}");
        assert!(x.cross(y).distance(z) < 1e-6, "right-handed");
        // A tilted frame stays orthonormal and maps +Y to forward, +Z to occlusal.
        let f = Vec3::new(0.3, 0.8, 0.1).normalize();
        let o = f.cross(Vec3::X).normalize();
        let tilted = frame([0.0; 3], o.as_dvec3().to_array(), f.as_dvec3().to_array());
        let q = camera_frame(Some(&tilted));
        let (x, y, z) = (q * Vec3::X, q * Vec3::Y, q * Vec3::Z);
        assert!(x.cross(y).distance(z) < 1e-5);
        assert!(y.distance(f) < 1e-5 && z.distance(o) < 1e-5);
    }

    #[test]
    fn arrow_has_a_shaft_and_two_head_strokes() {
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let pts = arrow_points(origin, Vec3::Z, Vec3::X, 8.0);
        assert_eq!(pts.len(), 5);
        assert_eq!(pts[0], origin);
        assert!(pts[1].distance(origin + Vec3::Z * 8.0) < 1e-6, "tip");
        assert_eq!(pts[1], pts[3], "both strokes start at the tip");
        assert!(
            pts[2].z < pts[1].z && pts[4].z < pts[1].z,
            "heads sweep back"
        );
        assert!(
            (pts[2].x - origin.x) * (pts[4].x - origin.x) < 0.0,
            "either side"
        );
    }

    #[test]
    fn ring_lies_in_the_plane_at_the_radius() {
        let o = Vec3::new(0.0, 0.0, 5.0);
        let pts = ring_points(o, Vec3::X, Vec3::Y, 20.0, 32);
        assert_eq!(pts.len(), 32);
        assert!(
            pts.iter()
                .all(|p| p.z == 5.0 && (p.distance(o) - 20.0).abs() < 1e-4)
        );
    }

    #[test]
    fn orienting_and_resetting_are_undoable_edits() {
        let mut design = Design::default();
        let mut history = History::new(100);
        let f = frame([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        set_frame(&mut design, &mut history, Some(f), ORIENT_LABEL);
        assert_eq!(design.jaw_frame, Some(f));
        assert_eq!(history.undo_label(), Some(ORIENT_LABEL));
        set_frame(&mut design, &mut history, None, RESET_LABEL);
        assert_eq!(design.jaw_frame, None);
        assert_eq!(history.undo_label(), Some(RESET_LABEL));
        history.undo(&mut design);
        assert_eq!(design.jaw_frame, Some(f));
    }
}
