//! The Insertion axis step. Entering it finds the preparation inside the tooth's margin and, when
//! the tooth has no axis yet, the best insertion axis; both run on a worker thread. The viewport
//! tool ([`AxisTool`]) draws the axis as an arrow from the preparation's centroid with a handle at
//! its tip and colours the preparation by its undercut depth. Dragging the handle tilts the axis:
//! the tip follows the cursor on a sphere round the centroid (not the scan surface, which behind a
//! tip in the air is the far gum and would flip the axis). The undercut map follows on a worker,
//! latest request wins, and a drag is one undo step.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_case::{Design, Fdi, ToothDesign};
use dentcraft_design::{
    MarginTracer, UndercutParams, best_insertion_axis_with_margin, undercut_map,
};
use dentcraft_mesh::TriMesh;
use dentcraft_render::{MeshData, MeshId, Overlay, colormap};
use eframe::egui;
use glam::DVec3;

use crate::history::{EditGroup, History};
use crate::margin_step::{MarginStep, arch_scan};
use crate::objects::SceneObject;
use crate::orient_step::arrow_points;
use crate::theme::Palette;
use crate::tools::{Pointer, ScanColors, Tool, rgb};
use crate::workflow_panel::AxisCard;

/// Name of the insertion axis tool.
pub const AXIS: &str = "Insertion axis";
/// Undo label of a handle drag.
pub const TILT_LABEL: &str = "Tilt insertion axis";
/// Undercut depth, in millimetres, that gets the full colour of the map.
pub const MAX_UNDERCUT_MM: f32 = 0.3;
/// Length of the drawn axis and radius of the sphere its tip moves on, in millimetres.
const ARROW_MM: f64 = 10.0;
/// Largest tilt the best-axis search tries from its initial direction, in degrees.
const MAX_TILT_DEG: f64 = 30.0;

/// Undo label of setting the axis (on entry, "Best axis", "From view").
pub fn set_label(tooth: Fdi) -> String {
    format!("Set insertion axis {tooth}")
}

/// Vertex colour of an undercut `depth` (mm) over the scan: alpha 0 (the scan's own colour) below
/// the undercut threshold, then the sequential map from 0 to [`MAX_UNDERCUT_MM`], blended in from
/// 40 % so that a shallow undercut still shows, to fully opaque at the maximum.
pub fn undercut_color(depth: f64) -> [f32; 4] {
    if depth.is_nan() || depth < UndercutParams::default().min_undercut_depth {
        return [0.0; 4];
    }
    let t = (depth as f32 / MAX_UNDERCUT_MM).min(1.0);
    let mut c = colormap::sequential(depth as f32, 0.0, MAX_UNDERCUT_MM);
    c[3] = 0.4 + 0.6 * t;
    c
}

/// The axis that puts its tip under the cursor ray (`origin`, `dir`): the direction from `centre`
/// to where the ray meets the sphere of `radius` round it (the near side), or, when the ray
/// passes outside the sphere, to the ray's closest approach (the sphere's silhouette).
pub fn axis_from_ray(centre: DVec3, radius: f64, origin: DVec3, dir: DVec3) -> Option<DVec3> {
    let dir = dir.try_normalize()?;
    let to_centre = centre - origin;
    let along = to_centre.dot(dir);
    let closest = origin + dir * along;
    let off = closest.distance_squared(centre);
    let point = if off <= radius * radius {
        closest - dir * (radius * radius - off).sqrt()
    } else {
        closest
    };
    (point - centre).try_normalize()
}

/// Sets (or with `None` clears) the insertion axis of `tooth` and records the edit as `label`.
/// Edits in the same open `group` merge into one undo step.
pub fn set_axis(
    design: &mut Design,
    history: &mut History,
    tooth: Fdi,
    axis: Option<DVec3>,
    label: &str,
    group: Option<EditGroup>,
) {
    let before = design.clone();
    match axis {
        Some(a) => design.teeth.entry(tooth).or_default().insertion_axis = Some(a),
        None => {
            if let Some(t) = design.teeth.get_mut(&tooth) {
                t.insertion_axis = None;
                if *t == ToothDesign::default() {
                    design.teeth.remove(&tooth);
                }
            }
        }
    }
    history.record(label, before, design, group);
}

/// A point inside the preparation: where a ray from above the margin's centroid, down `axis`,
/// first meets the scan.
pub fn region_seed(
    margin: &[DVec3],
    axis: DVec3,
    scan: &dentcraft_mesh::Bvh,
    mesh: &TriMesh,
) -> Option<DVec3> {
    if margin.is_empty() {
        return None;
    }
    let centroid = margin.iter().sum::<DVec3>() / margin.len() as f64;
    let axis = axis.try_normalize()?;
    scan.raycast(mesh, centroid + axis * 50.0, -axis)
        .map(|h| h.point)
}

/// The preparation's triangles inside the stored margin `curve`, from `seed`.
/// The barrier follows the curve itself (dense snapping, no re-trace through a
/// few control points), so the region stays consistent with the curve the inner
/// surface seals onto.
pub fn prep_region(
    tracer: &MarginTracer,
    curve: &[DVec3],
    seed: DVec3,
) -> Result<Vec<u32>, String> {
    let margin = tracer.follow(curve).map_err(|e| e.to_string())?;
    let region = tracer.region(&margin, seed);
    if region.is_empty() {
        Err("Cannot find the preparation inside the margin".to_owned())
    } else {
        Ok(region)
    }
}

/// Area-weighted centroid of the triangles `region` of `mesh`.
pub(crate) fn region_centroid(mesh: &TriMesh, region: &[u32]) -> DVec3 {
    let (mut sum, mut total) = (DVec3::ZERO, 0.0);
    for &t in region {
        let [a, b, c] = mesh.triangles[t as usize].map(|v| mesh.positions[v as usize]);
        let area = (b - a).cross(c - a).length();
        sum += (a + b + c) / 3.0 * area;
        total += area;
    }
    if total > 0.0 {
        sum / total
    } else {
        DVec3::ZERO
    }
}

/// The preparation found inside a margin.
struct Prep {
    region: Vec<u32>,
    centroid: DVec3,
}

/// What the tool asks the step to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    /// The handle was dragged: the axis is now this.
    Tilt(DVec3),
    /// The handle drag ended.
    DragEnd,
}

/// State shared by the tool and the step.
#[derive(Default)]
pub struct Shared {
    centroid: Option<DVec3>,
    axis: Option<DVec3>,
    colors: Option<ScanColors>,
    requests: Vec<Request>,
}

/// The viewport tool of the Insertion axis step.
pub struct AxisTool {
    shared: Rc<RefCell<Shared>>,
}

impl AxisTool {
    fn tip(&self) -> Option<DVec3> {
        let s = self.shared.borrow();
        Some(s.centroid? + s.axis? * ARROW_MM)
    }
}

impl Tool for AxisTool {
    fn name(&self) -> &'static str {
        AXIS
    }

    fn picks_surface(&self) -> bool {
        false
    }

    fn handles(&self) -> Vec<DVec3> {
        self.tip().into_iter().collect()
    }

    fn drag(&mut self, _handle: usize, pointer: &Pointer) {
        let mut s = self.shared.borrow_mut();
        let (Some(centre), Some((origin, dir))) = (s.centroid, pointer.ray) else {
            return;
        };
        if let Some(axis) = axis_from_ray(centre, ARROW_MM, origin, dir)
            && s.axis != Some(axis)
        {
            s.axis = Some(axis);
            s.requests.push(Request::Tilt(axis));
        }
    }

    fn drag_end(&mut self, _handle: usize, _pointer: &Pointer) {
        self.shared.borrow_mut().requests.push(Request::DragEnd);
    }

    fn overlays(&self, palette: &Palette) -> Vec<Overlay> {
        let s = self.shared.borrow();
        let (Some(c), Some(axis)) = (s.centroid, s.axis) else {
            return Vec::new();
        };
        let accent = rgb(palette.accent);
        let side = axis.any_orthonormal_vector();
        let other = axis.cross(side);
        let (c, a) = (c.as_vec3(), axis.as_vec3());
        let len = ARROW_MM as f32;
        let mut cross_head = arrow_points(c, a, other.as_vec3(), len);
        cross_head.drain(..1);
        vec![
            Overlay::Polyline {
                points: arrow_points(c, a, side.as_vec3(), len),
                closed: false,
                color: accent,
                width_px: 3.0,
            },
            Overlay::Polyline {
                points: cross_head,
                closed: false,
                color: accent,
                width_px: 3.0,
            },
            Overlay::Points {
                points: vec![c + a * len],
                color: accent,
                radius_px: 6.0,
                outline: Some([1.0; 3]),
            },
        ]
    }

    fn vertex_colors(&self) -> Option<ScanColors> {
        self.shared.borrow().colors.clone()
    }
}

/// What the worker should do with the axis.
#[derive(Clone, Copy, Debug)]
enum Want {
    /// Search the best axis around this direction (`None`: the region's average normal).
    Best(Option<DVec3>),
    /// Map the undercuts of this axis.
    Fixed(DVec3),
}

/// Identifies the preparation: the tooth and its stored margin curve.
type Key = (Fdi, Vec<DVec3>);

struct Job {
    key: Key,
    scan: MeshId,
    mesh: Arc<TriMesh>,
    tracer: Arc<MarginTracer>,
    seed: DVec3,
    prep: Option<Arc<Prep>>,
    want: Want,
}

struct Outcome {
    key: Key,
    scan: MeshId,
    prep: Arc<Prep>,
    axis: DVec3,
    best: bool,
    undercut_mm2: f64,
    min_draft_deg: f64,
    colors: Arc<MeshData>,
}

fn run(job: Job) -> Result<Outcome, String> {
    let mesh = &job.mesh;
    let prep = match job.prep {
        Some(p) => p,
        None => {
            let region = prep_region(&job.tracer, &job.key.1, job.seed)?;
            let centroid = region_centroid(mesh, &region);
            Arc::new(Prep { region, centroid })
        }
    };
    let err = |e: dentcraft_design::DesignError| capitalize(&e.to_string());
    // Seating the margin comes first: the search minimises the margin overhang
    // (beyond the seal band), then occluded area, draft and tilt.
    let (result, best) = match job.want {
        Want::Best(initial) => (
            best_insertion_axis_with_margin(mesh, &prep.region, &job.key.1, initial, MAX_TILT_DEG)
                .map_err(err)?,
            true,
        ),
        Want::Fixed(axis) => (
            best_insertion_axis_with_margin(mesh, &prep.region, &job.key.1, Some(axis), 0.0)
                .map_err(err)?,
            false,
        ),
    };
    let map = undercut_map(mesh, &prep.region, result.axis).map_err(err)?;
    let mut colors = vec![[0.0; 4]; mesh.positions.len()];
    for (&v, &d) in map.vertices.iter().zip(&map.horizontal_depths) {
        colors[v as usize] = undercut_color(d);
    }
    let colors = MeshData::with_colors(mesh, &colors).map_err(|e| e.to_string())?;
    Ok(Outcome {
        key: job.key,
        scan: job.scan,
        prep,
        axis: result.axis,
        best,
        undercut_mm2: map.undercut_area,
        min_draft_deg: result.min_draft_deg,
        colors: Arc::new(colors),
    })
}

/// What the step needs from the app for one frame.
pub struct StepFrame<'a> {
    pub design: &'a mut Design,
    pub history: &'a mut History,
    pub objects: &'a [SceneObject],
    /// The Margin step, whose tracers find the preparation.
    pub margin: &'a mut MarginStep,
    /// Tooth being worked on, when the step is active.
    pub tooth: Option<Fdi>,
    /// The jaw's occlusal direction, once it is oriented.
    pub occlusal: Option<DVec3>,
}

/// The app side of the Insertion axis step: the preparation, worker jobs and undoable edits.
pub struct AxisStep {
    shared: Rc<RefCell<Shared>>,
    key: Option<Key>,
    prep: Option<Arc<Prep>>,
    /// Axis, area and draft of the shown map.
    shown: Option<(DVec3, f64, f64)>,
    /// The tooth's preparation has been searched for its best axis this visit.
    searched: bool,
    /// The last job for the current key failed: wait for an explicit action.
    failed: bool,
    /// A preparation was found since the app last asked: the view should take in the axis.
    new_prep: bool,
    waiting: Option<Want>,
    running: bool,
    tx: Sender<Result<Outcome, (Key, String)>>,
    rx: Receiver<Result<Outcome, (Key, String)>>,
    next_group: u64,
    drag: Option<EditGroup>,
    error: Option<String>,
    repaint: Option<egui::Context>,
}

impl AxisStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self {
            shared: Rc::default(),
            key: None,
            prep: None,
            shown: None,
            searched: false,
            failed: false,
            new_prep: false,
            waiting: None,
            running: false,
            tx,
            rx,
            next_group: 1,
            drag: None,
            error: None,
            repaint,
        }
    }

    /// Forgets the open case's preparation, jobs and error.
    pub fn reset(&mut self) {
        *self = Self::new(self.repaint.take());
    }

    pub fn tool(&self) -> Box<dyn Tool> {
        Box::new(AxisTool {
            shared: self.shared.clone(),
        })
    }

    /// A job is waiting or running.
    pub fn busy(&self) -> bool {
        self.running || self.waiting.is_some()
    }

    /// What the step panel shows for `tooth`.
    pub fn card(&self, design: &Design, tooth: Fdi, occlusal: Option<DVec3>) -> AxisCard {
        let stored = design.teeth.get(&tooth);
        let axis = stored.and_then(|t| t.insertion_axis);
        let current = self.key.as_ref().is_some_and(|k| k.0 == tooth);
        let metrics = self
            .shown
            .filter(|(a, ..)| current && Some(*a) == axis)
            .map(|(_, area, draft)| (area, draft));
        AxisCard {
            tooth,
            has_margin: stored.is_some_and(|t| t.margin.is_some()),
            tilt_deg: axis
                .zip(occlusal)
                .map(|(a, o)| a.angle_between(o).to_degrees()),
            undercut_mm2: metrics.map(|m| m.0),
            min_draft_deg: metrics.map(|m| m.1),
            busy: if self.busy() && self.prep.is_none() {
                Some("Finding the preparation…")
            } else if self.busy() {
                Some("Mapping undercuts…")
            } else {
                None
            },
            error: self.error.clone(),
        }
    }

    /// Whether a preparation was found since the last call, so the view can take in its axis.
    pub fn take_new_prep(&mut self) -> bool {
        std::mem::take(&mut self.new_prep)
    }

    /// Searches the best axis again.
    pub fn best(&mut self, occlusal: Option<DVec3>) {
        self.failed = false;
        self.error = None;
        self.waiting = Some(Want::Best(occlusal));
    }

    /// Sets the axis of `tooth` to `dir` (the reversed view direction).
    pub fn set_from_view(
        &mut self,
        design: &mut Design,
        history: &mut History,
        tooth: Fdi,
        dir: DVec3,
    ) {
        if let Some(axis) = dir.try_normalize() {
            self.failed = false;
            set_axis(design, history, tooth, Some(axis), &set_label(tooth), None);
        }
    }

    /// Runs one frame: records the tool's tilts, follows the stored margin and axis, starts and
    /// finishes worker jobs.
    pub fn update(&mut self, f: StepFrame<'_>) {
        let requests = std::mem::take(&mut self.shared.borrow_mut().requests);
        for request in requests {
            let Some(tooth) = f.tooth else { continue };
            match request {
                Request::Tilt(axis) => {
                    let group = *self.drag.get_or_insert_with(|| {
                        self.next_group += 1;
                        EditGroup(self.next_group)
                    });
                    self.failed = false;
                    set_axis(
                        f.design,
                        f.history,
                        tooth,
                        Some(axis),
                        TILT_LABEL,
                        Some(group),
                    );
                }
                Request::DragEnd => {
                    if self.drag.take().is_some() {
                        f.history.end_group();
                    }
                }
            }
        }
        while let Ok(message) = self.rx.try_recv() {
            self.running = false;
            self.finish(message, f.design, f.history);
        }
        let stored = f.tooth.and_then(|t| f.design.teeth.get(&t));
        let key = f.tooth.zip(stored.and_then(|t| t.margin.clone()));
        if key != self.key {
            self.key = key.clone();
            self.prep = None;
            self.shown = None;
            self.searched = false;
            self.failed = false;
            self.waiting = None;
            self.error = None;
            let mut s = self.shared.borrow_mut();
            s.colors = None;
            s.centroid = None;
        }
        let axis = stored.and_then(|t| t.insertion_axis);
        {
            let mut s = self.shared.borrow_mut();
            if let Some(p) = &self.prep {
                s.centroid = Some(p.centroid);
            }
            if self.drag.is_none() {
                s.axis = axis;
            }
        }
        let Some(key) = key else { return };
        if !self.busy() && !self.failed {
            if !self.searched && axis.is_none() {
                self.waiting = Some(Want::Best(f.occlusal));
            } else if let Some(a) = axis
                && self.shown.map(|s| s.0) != Some(a)
            {
                self.waiting = Some(Want::Fixed(a));
            }
        }
        self.searched = true;
        if self.running || self.waiting.is_none() {
            return;
        }
        let Some(scan) = arch_scan(f.objects, key.0) else {
            self.error = Some("No jaw scan for this tooth".to_owned());
            self.waiting = None;
            return;
        };
        let tracer = match f.margin.tracer(scan, f.objects) {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => {
                self.error = Some(format!("Cannot use this scan: {e}"));
                self.waiting = None;
                return;
            }
        };
        let Some(loaded) = f
            .objects
            .iter()
            .find(|o| o.id == scan)
            .and_then(|o| o.loaded())
        else {
            return;
        };
        let up = f.occlusal.or(axis).unwrap_or(DVec3::Z);
        let Some(seed) = region_seed(&key.1, up, &loaded.bvh, &loaded.mesh) else {
            self.error = Some("Cannot find the preparation inside the margin".to_owned());
            self.waiting = None;
            self.failed = true;
            return;
        };
        let Some(want) = self.waiting.take() else {
            return;
        };
        let job = Job {
            key,
            scan,
            mesh: tracer.mesh().clone(),
            tracer,
            seed,
            prep: self.prep.clone(),
            want,
        };
        self.running = true;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let key = job.key.clone();
        crate::crash::spawn_worker(
            "axis",
            move || run(job),
            move |result| {
                let _ = tx.send(result.and_then(|r| r).map_err(|e| (key, e)));
                if let Some(ctx) = repaint {
                    ctx.request_repaint();
                }
            },
        );
    }

    fn finish(
        &mut self,
        message: Result<Outcome, (Key, String)>,
        design: &mut Design,
        history: &mut History,
    ) {
        let outcome = match message {
            Ok(o) => o,
            Err((key, e)) => {
                if self.key.as_ref() == Some(&key) {
                    self.error = Some(e);
                    self.failed = true;
                }
                return;
            }
        };
        if self.key.as_ref() != Some(&outcome.key) {
            return;
        }
        self.error = None;
        self.new_prep |= self.prep.is_none();
        self.prep = Some(outcome.prep);
        if outcome.best {
            let tooth = outcome.key.0;
            set_axis(
                design,
                history,
                tooth,
                Some(outcome.axis),
                &set_label(tooth),
                None,
            );
        }
        self.shown = Some((outcome.axis, outcome.undercut_mm2, outcome.min_draft_deg));
        self.shared.borrow_mut().colors = Some(ScanColors {
            scan: outcome.scan,
            data: outcome.colors,
        });
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
    use dentcraft_design::{ToothParams, prepared_tooth};

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn ring(radius: f64, z: f64, n: usize) -> Vec<DVec3> {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                DVec3::new(radius * a.cos(), radius * a.sin(), z)
            })
            .collect()
    }

    #[test]
    fn region_from_the_margin_and_an_axis_on_a_synthetic_prep() {
        let tooth = prepared_tooth(&ToothParams::default()).unwrap();
        let mesh = Arc::new(tooth.mesh);
        let bvh = dentcraft_mesh::Bvh::build(&mesh);
        let margin = ring(4.0, 8.0, 252);
        let seed = region_seed(&margin, DVec3::Z, &bvh, &mesh).unwrap();
        assert!(
            (seed.z - 12.0).abs() < 0.05 && seed.truncate().length() < 0.1,
            "{seed}"
        );
        let tracer = MarginTracer::new(mesh.clone()).unwrap();
        let region = prep_region(&tracer, &margin, seed).unwrap();
        assert!(!region.is_empty());
        // Nothing of the root or the plate: every region vertex is at or above the margin.
        for &tri in &region {
            for v in mesh.triangles[tri as usize] {
                assert!(mesh.positions[v as usize].z > 7.9);
            }
        }
        let c = region_centroid(&mesh, &region);
        assert!(
            c.truncate().length() < 0.05 && c.z > 8.0 && c.z < 12.0,
            "{c}"
        );
        let best =
            best_insertion_axis_with_margin(&mesh, &region, &margin, Some(DVec3::Z), MAX_TILT_DEG)
                .unwrap();
        assert!(
            best.axis.angle_between(DVec3::Z).to_degrees() < 1.0,
            "{}",
            best.axis
        );
        assert!(best.undercut_area < 1e-6);
        assert!(best.margin_overhang.unwrap().depth < 1e-9);
    }

    #[test]
    fn the_handle_tip_follows_the_cursor_on_a_sphere() {
        let c = DVec3::new(1.0, 2.0, 3.0);
        // Looking straight down at the centre: the axis points up at the camera.
        let a = axis_from_ray(c, 10.0, c + DVec3::Z * 50.0, -DVec3::Z).unwrap();
        assert!(a.distance(DVec3::Z) < 1e-9);
        // A ray 6 mm off the centre meets the near side at 8 mm height: 36.87 degrees of tilt.
        let a = axis_from_ray(c, 10.0, c + DVec3::new(6.0, 0.0, 50.0), -DVec3::Z).unwrap();
        assert!(a.distance(DVec3::new(0.6, 0.0, 0.8)) < 1e-9, "{a}");
        // Outside the sphere: the silhouette, perpendicular to the view.
        let a = axis_from_ray(c, 10.0, c + DVec3::new(0.0, 15.0, 50.0), -DVec3::Z).unwrap();
        assert!(a.distance(DVec3::Y) < 1e-9, "{a}");
        assert!(axis_from_ray(c, 10.0, c, DVec3::ZERO).is_none());
    }

    #[test]
    fn colours_leave_shallow_areas_untouched_and_saturate_at_the_maximum() {
        assert_eq!(undercut_color(0.0), [0.0; 4]);
        assert_eq!(undercut_color(0.02)[3], 0.0, "below the threshold");
        assert_eq!(undercut_color(f64::NAN)[3], 0.0);
        let full = colormap::sequential(1.0, 0.0, 1.0);
        for d in [0.3, 0.5, 1.0] {
            assert_eq!(undercut_color(d), full, "{d}");
        }
        let mid = undercut_color(0.15);
        assert!(mid[3] > 0.4 && mid[3] < 1.0);
    }

    #[test]
    fn a_tilt_drag_is_one_undo_step() {
        let tooth = t(24);
        let step = AxisStep::new(None);
        let mut tool = step.tool();
        step.shared.borrow_mut().centroid = Some(DVec3::ZERO);
        step.shared.borrow_mut().axis = Some(DVec3::Z);
        assert_eq!(tool.handles(), vec![DVec3::Z * ARROW_MM]);
        let mut design = Design::default();
        let mut history = History::default();
        set_axis(
            &mut design,
            &mut history,
            tooth,
            Some(DVec3::Z),
            &set_label(tooth),
            None,
        );
        assert_eq!(history.undo_label(), Some("Set insertion axis 24"));

        let mut step = step;
        let objects = [];
        let mut margin = MarginStep::new(None);
        for x in [1.0, 2.0, 3.0] {
            let pointer = Pointer {
                pixel: glam::Vec2::ZERO,
                hit: None,
                ray: Some((DVec3::new(x, 0.0, 50.0), -DVec3::Z)),
            };
            tool.drag(0, &pointer);
            step.update(StepFrame {
                design: &mut design,
                history: &mut history,
                objects: &objects,
                margin: &mut margin,
                tooth: Some(tooth),
                occlusal: None,
            });
        }
        let pointer = Pointer {
            pixel: glam::Vec2::ZERO,
            hit: None,
            ray: None,
        };
        tool.drag_end(0, &pointer);
        step.update(StepFrame {
            design: &mut design,
            history: &mut history,
            objects: &objects,
            margin: &mut margin,
            tooth: Some(tooth),
            occlusal: None,
        });
        let tilted = design.teeth[&tooth].insertion_axis.unwrap();
        assert!(tilted.x > 0.29 && tilted.x < 0.31, "{tilted}");
        assert_eq!(history.undo_label(), Some(TILT_LABEL));
        assert_eq!(history.undo(&mut design).as_deref(), Some(TILT_LABEL));
        assert_eq!(design.teeth[&tooth].insertion_axis, Some(DVec3::Z));
    }
}
