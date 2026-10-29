//! The Orientation step. "Auto orient" finds the jaw's occlusal plane and midline on a worker
//! thread and stores them in the case's design with undo. The viewport tool ([`OrientTool`]) only
//! draws the frame (occlusal and forward arrows, a faint plane ring); the camera follows the
//! stored frame through [`camera_frame`].

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_case::{Arch, Design, Fdi, JawFrame, ScanRole};
use dentcraft_design::{DesignError, JawFrame as Found};
use dentcraft_mesh::TriMesh;
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

/// The frame's occlusal direction for `tooth`: the stored direction on the arch the frame was
/// computed on, its opposite on the other arch. Without a tooth, the frame's own direction.
/// Frames from older files (no recorded jaw) belong to the arch of the first of `teeth`.
pub fn occlusal_for(frame: &JawFrame, teeth: &[Fdi], tooth: Option<Fdi>) -> DVec3 {
    let occlusal = DVec3::from_array(frame.occlusal);
    let Some(tooth) = tooth else {
        return occlusal;
    };
    let frame_upper = frame.upper.unwrap_or_else(|| is_upper_jaw(teeth));
    if (tooth.arch() == Arch::Upper) == frame_upper {
        occlusal
    } else {
        -occlusal
    }
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

/// The scan to orient: the jaw scan of the first unit's arch; without units the first Lower
/// jaw scan, else the first Upper, else the largest scan.
fn jaw_scan(objects: &[SceneObject], teeth: &[Fdi]) -> Option<usize> {
    let first = |role: ScanRole| {
        objects
            .iter()
            .position(|o| o.role == role && o.loaded().is_some())
    };
    if let Some(tooth) = teeth.first() {
        return first(match tooth.arch() {
            Arch::Upper => ScanRole::Upper,
            Arch::Lower => ScanRole::Lower,
        });
    }
    first(ScanRole::Lower)
        .or_else(|| first(ScanRole::Upper))
        .or_else(|| {
            (0..objects.len())
                .filter(|&i| objects[i].loaded().is_some())
                .max_by_key(|&i| objects[i].triangle_count())
        })
}

/// Shown when the front of the arch is a guess.
pub const ESTIMATED_NOTE: &str =
    "Front of the arch estimated — check the view and use Reset if it is wrong";

/// What helps place the front of an arch that does not determine it.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrontHints {
    /// Centre of the case's anterior units, when known.
    pub anterior: Option<DVec3>,
    /// The camera's current up direction.
    pub up: DVec3,
}

/// The jaw frame of `mesh`, and whether its front was estimated. A partial arch (or a single
/// stump) keeps its occlusal direction but takes the front from the hints.
fn orient_mesh(mesh: &TriMesh, hints: FrontHints) -> Result<(Found, bool), String> {
    match dentcraft_design::jaw_frame(mesh) {
        Ok(found) => Ok((found, false)),
        Err(DesignError::PartialArch) => estimated_frame(mesh, hints)
            .ok_or_else(|| capitalize(&DesignError::PartialArch.to_string())),
        Err(e) => Err(capitalize(&e.to_string())),
    }
}

/// The partial arch's frame, and whether its front is a guess: the shape of the arch
/// (`arch_front`) comes first, then the anterior units, then the camera's up.
fn estimated_frame(mesh: &TriMesh, hints: FrontHints) -> Option<(Found, bool)> {
    let occlusal = dentcraft_design::occlusal_direction(mesh).ok()?;
    let (centroid, _) = area_centroid(mesh)?;
    let in_plane = |v: DVec3| {
        let flat = v - occlusal * v.dot(occlusal);
        (flat.length() > 1e-6).then(|| flat.normalize())
    };
    let from_shape = dentcraft_design::arch_front(mesh, occlusal);
    let forward = from_shape
        .or_else(|| hints.anterior.and_then(|a| in_plane(a - centroid)))
        .or_else(|| in_plane(hints.up))
        .or_else(|| in_plane(occlusal.any_orthogonal_vector()))?;
    let mut heights: Vec<f64> = mesh.positions.iter().map(|p| p.dot(occlusal)).collect();
    heights.sort_by(f64::total_cmp);
    let cusp = heights[((heights.len() - 1) as f64 * 0.95) as usize];
    let origin = centroid + occlusal * (cusp - centroid.dot(occlusal));
    let found = Found {
        origin,
        occlusal,
        forward,
        right: forward.cross(occlusal),
    };
    Some((found, from_shape.is_none()))
}

/// Area-weighted centroid of the triangles, with the total area.
fn area_centroid(mesh: &TriMesh) -> Option<(DVec3, f64)> {
    let (mut sum, mut total) = (DVec3::ZERO, 0.0);
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
        let area = 0.5 * (b - a).cross(c - a).length();
        sum += area * (a + b + c) / 3.0;
        total += area;
    }
    (total > 0.0 && total.is_finite()).then(|| (sum / total, total))
}

/// Centre of the margins of the anterior units that have one.
pub fn anterior_centre(design: &Design, teeth: &[Fdi]) -> Option<DVec3> {
    let points: Vec<DVec3> = teeth
        .iter()
        .filter(|t| t.is_anterior())
        .filter_map(|t| design.teeth.get(t)?.margin.as_ref())
        .flatten()
        .copied()
        .collect();
    (!points.is_empty()).then(|| points.iter().sum::<DVec3>() / points.len() as f64)
}

fn to_case(f: &Found, upper: bool) -> JawFrame {
    JawFrame {
        upper: Some(upper),
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
    /// Whether the running job orients the upper jaw.
    upper: bool,
    running: bool,
    tx: Sender<Result<(Found, bool), String>>,
    rx: Receiver<Result<(Found, bool), String>>,
    error: Option<String>,
    note: Option<String>,
    repaint: Option<egui::Context>,
}

impl OrientStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self {
            shown: Rc::default(),
            upper: false,
            running: false,
            tx,
            rx,
            error: None,
            note: None,
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
            note: self.note.clone(),
            has_frame,
        }
    }

    /// Finds the frame of the jaw scan the `teeth` sit on, on a worker thread.
    pub fn start(&mut self, objects: &[SceneObject], teeth: &[Fdi], hints: FrontHints) {
        if self.running {
            return;
        }
        let Some(mesh) = jaw_scan(objects, teeth)
            .and_then(|i| objects[i].loaded())
            .map(|l| l.mesh.clone())
        else {
            self.error = Some("No jaw scan to orient".to_owned());
            self.note = None;
            return;
        };
        self.error = None;
        self.note = None;
        self.upper = is_upper_jaw(teeth);
        self.running = true;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        crate::crash::spawn_worker(
            "orient",
            move || orient_mesh(&mesh, hints),
            move |result| {
                let _ = tx.send(result.and_then(|r| r));
                if let Some(ctx) = repaint {
                    ctx.request_repaint();
                }
            },
        );
    }

    /// Clears the orientation.
    pub fn clear(&mut self, design: &mut Design, history: &mut History) {
        self.error = None;
        self.note = None;
        set_frame(design, history, None, RESET_LABEL);
    }

    /// Runs one frame: takes a finished job's result and shows the stored frame to the tool.
    pub fn update(&mut self, design: &mut Design, history: &mut History) {
        while let Ok(result) = self.rx.try_recv() {
            self.running = false;
            match result {
                Ok((found, estimated)) => {
                    self.error = None;
                    self.note = estimated.then(|| ESTIMATED_NOTE.to_owned());
                    set_frame(
                        design,
                        history,
                        Some(to_case(&found, self.upper)),
                        ORIENT_LABEL,
                    );
                }
                Err(e) => {
                    self.note = None;
                    self.error = Some(e);
                }
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
            upper: None,
        }
    }

    #[test]
    fn occlusal_follows_the_arch_of_the_tooth() {
        let (u, l) = (Fdi::new(16).unwrap(), Fdi::new(44).unwrap());
        let mut f = frame([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        f.upper = Some(true);
        let up = DVec3::Z;
        assert_eq!(occlusal_for(&f, &[l], Some(u)), up);
        assert_eq!(occlusal_for(&f, &[l], Some(l)), -up);
        assert_eq!(occlusal_for(&f, &[l], None), up);
        f.upper = Some(false);
        assert_eq!(occlusal_for(&f, &[u], Some(l)), up);
        assert_eq!(occlusal_for(&f, &[u], Some(u)), -up);
        // Legacy frame: the arch of the first unit.
        f.upper = None;
        assert_eq!(occlusal_for(&f, &[l, u], Some(l)), up);
        assert_eq!(occlusal_for(&f, &[l, u], Some(u)), -up);
        assert_eq!(occlusal_for(&f, &[u], Some(u)), up);
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

    fn object(id: u64, role: ScanRole, mesh: TriMesh) -> SceneObject {
        let loaded = crate::loader::Loaded {
            bvh: dentcraft_mesh::Bvh::build(&mesh),
            render: std::sync::Arc::new(dentcraft_render::MeshData::from_mesh(&mesh)),
            mesh,
        };
        SceneObject {
            id: dentcraft_render::MeshId(id),
            name: format!("scan {id}"),
            path: std::path::PathBuf::new(),
            role,
            color: [200, 200, 200],
            visible: true,
            opacity: 1.0,
            state: crate::objects::ObjectState::Ready(Box::new(loaded)),
        }
    }

    #[test]
    fn without_units_the_lower_jaw_scan_is_oriented() {
        let small = TriMesh::uv_sphere(1.0, 8, 4);
        let big = TriMesh::uv_sphere(1.0, 32, 16);
        let objects = vec![
            object(1, ScanRole::Other, big.clone()),
            object(2, ScanRole::Upper, small.clone()),
            object(3, ScanRole::Lower, small.clone()),
            object(4, ScanRole::Lower, big.clone()),
        ];
        assert_eq!(jaw_scan(&objects, &[]), Some(2));
        assert_eq!(jaw_scan(&objects[..2], &[]), Some(1));
        // No jaw role: the largest scan.
        let others = vec![
            object(1, ScanRole::Other, small),
            object(2, ScanRole::Other, big),
        ];
        assert_eq!(jaw_scan(&others, &[]), Some(1));
        assert_eq!(jaw_scan(&[], &[]), None);
        // A unit still picks the scan of its arch.
        let upper_tooth = [Fdi::new(11).unwrap()];
        assert_eq!(jaw_scan(&objects, &upper_tooth), Some(1));
    }

    #[test]
    fn a_partial_arch_keeps_its_occlusal_and_estimates_the_front() {
        let mesh = dentcraft_design::synthetic::open_arch_cut(1.3, 8.0);
        assert_eq!(
            dentcraft_design::jaw_frame(&mesh),
            Err(DesignError::PartialArch)
        );
        let occlusal = dentcraft_design::occlusal_direction(&mesh).unwrap();
        let up = DVec3::new(0.3, 0.8, 0.5);
        let (found, estimated) = orient_mesh(&mesh, FrontHints { anterior: None, up }).unwrap();
        assert!(estimated);
        assert!((found.occlusal - occlusal).length() < 1e-12);
        assert!(found.forward.dot(occlusal).abs() < 1e-9);
        assert!((found.forward.length() - 1.0).abs() < 1e-9);
        let flat_up = up - occlusal * up.dot(occlusal);
        assert!(found.forward.dot(flat_up.normalize()) > 0.999);
        assert!((found.right.cross(found.forward) - found.occlusal).length() < 1e-9);

        // Anterior units pull the front towards themselves instead.
        let (centroid, _) = area_centroid(&mesh).unwrap();
        let towards = centroid + DVec3::new(-1.0, 0.0, 0.0) * 30.0;
        let hints = FrontHints {
            anterior: Some(towards),
            up,
        };
        let (found, _) = orient_mesh(&mesh, hints).unwrap();
        let wanted = towards - centroid;
        let wanted = (wanted - occlusal * wanted.dot(occlusal)).normalize();
        assert!(found.forward.dot(wanted) > 0.999);
    }

    #[test]
    fn a_full_arch_is_not_flagged_as_estimated() {
        let mesh = dentcraft_design::synthetic::arch(true);
        let (_, estimated) = orient_mesh(&mesh, FrontHints::default()).unwrap();
        assert!(!estimated);
    }

    #[test]
    fn real_errors_stay_one_plain_sentence() {
        let empty = TriMesh::new(Vec::new(), Vec::new());
        let error = orient_mesh(&empty, FrontHints::default()).unwrap_err();
        assert!(error.chars().next().unwrap().is_uppercase());
        assert!(!error.contains('\n'));
    }
}
