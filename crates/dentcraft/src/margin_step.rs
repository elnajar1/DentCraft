//! The Margin line step. One click on a preparation proposes the margin; handles along it reshape
//! it. The viewport tool ([`MarginTool`]) and the app-side step ([`MarginStep`]) share a
//! [`Shared`] state: the tool queues requests there, the step drains them every frame, runs the
//! tracer on worker threads (latest request wins, a frame never waits) and records the results in
//! the case's design with undo.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_case::{Arch, Design, Fdi, ScanRole, ToothDesign};
use dentcraft_design::{DesignError, Margin, MarginTracer};
use dentcraft_mesh::TriMesh;
use dentcraft_render::{MeshId, Overlay};
use eframe::egui;
use glam::DVec3;

use crate::history::{EditGroup, History};
use crate::margin_edit::{control_points, curve_length, default_control_count, insertion_index};
use crate::objects::SceneObject;
use crate::theme::Palette;
use crate::tools::{Pointer, Tool, rgb};
use crate::workflow_panel::MarginCard;

/// Name of the margin tool.
pub const MARGIN: &str = "Margin";
/// World distance, in millimetres, within which the pointer is on a control point or the curve.
const NEAR: f64 = 0.5;
/// A margin keeps at least this many control points.
const MIN_CONTROLS: usize = 3;

/// An edit of the margin, named for the Undo tooltip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Detect,
    Move,
    Insert,
    Delete,
    Clear,
    /// A margin traced by hand through waypoints.
    Manual,
}

/// How the Margin step finds the margin of the current tooth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Automatic detection from a click on the preparation.
    #[default]
    Auto,
    /// The tooth has a margin; its control points can be edited.
    Edit,
    /// The margin is traced by hand through waypoints.
    Manual,
}

impl Mode {
    /// The mode of a tooth outside manual tracing.
    fn settled(has_margin: bool) -> Self {
        if has_margin { Self::Edit } else { Self::Auto }
    }

    /// Done: manual tracing ends with the margin stored.
    fn done(self) -> Self {
        if self == Self::Manual {
            Self::Edit
        } else {
            self
        }
    }

    /// Escape: manual tracing is dropped, back to what the tooth had. `false` when there was
    /// no manual trace to cancel.
    fn escape(self, has_margin: bool) -> (Self, bool) {
        if self == Self::Manual {
            (Self::settled(has_margin), true)
        } else {
            (self, false)
        }
    }
}

/// The margin of a plausible tooth is this long, in millimetres.
const MARGIN_LENGTH_MM: std::ops::RangeInclusive<f64> = 8.0..=45.0;
/// ... and its centroid is at most this far from the click, in millimetres.
const MAX_SEED_OFFSET_MM: f64 = 8.0;

/// Whether a detected margin looks like the margin of the tooth clicked at `seed`.
fn plausible(margin: &[DVec3], seed: DVec3) -> bool {
    if margin.len() < 3 {
        return false;
    }
    let centroid = margin.iter().sum::<DVec3>() / margin.len() as f64;
    MARGIN_LENGTH_MM.contains(&curve_length(margin))
        && centroid.distance(seed) <= MAX_SEED_OFFSET_MM
}

/// Shown when automatic detection found nothing usable; manual tracing starts at the same time.
const MANUAL_PROMPT: &str = "Couldn't find the margin automatically. Click points along the \
                             margin instead \u{2014} the line follows the edge between them.";

impl Edit {
    pub fn label(self, tooth: Fdi) -> String {
        match self {
            Self::Detect => format!("Detect margin {tooth}"),
            Self::Move => "Move margin point".to_owned(),
            Self::Insert => "Insert margin point".to_owned(),
            Self::Delete => "Delete margin point".to_owned(),
            Self::Clear => format!("Clear margin {tooth}"),
            Self::Manual => format!("Trace margin {tooth}"),
        }
    }
}

/// What the tool asks the step to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Propose a margin around the preparation clicked at `seed` on scene object `object`.
    Propose { object: usize, seed: DVec3 },
    /// Trace the margin through `controls`.
    Trace {
        object: Option<usize>,
        controls: Vec<DVec3>,
        edit: Edit,
    },
    /// A handle drag ended.
    DragEnd,
}

/// State shared by the tool and the step.
#[derive(Debug, Default)]
pub struct Shared {
    tooth: Option<Fdi>,
    controls: Vec<DVec3>,
    curve: Vec<DVec3>,
    hovered: Option<usize>,
    /// Last dragged or clicked control, which Delete removes when none is hovered.
    selected: Option<usize>,
    requests: Vec<Request>,
    mode: Mode,
    /// Manual mode: the clicked points, in loop order.
    waypoints: Vec<DVec3>,
    /// Manual mode: the loop traced through the waypoints, not yet stored.
    manual_curve: Vec<DVec3>,
    /// Done was asked for; the step stores the trace once the worker is idle.
    finish_requested: bool,
}

impl Shared {
    /// The tooth has a stored margin.
    fn has_margin(&self) -> bool {
        self.curve.len() >= 3
    }

    /// Starts a manual trace through `waypoints`.
    fn enter_manual(&mut self, waypoints: Vec<DVec3>) {
        self.leave_manual(Mode::Manual);
        self.waypoints = waypoints;
    }

    /// Sets `mode` and drops the manual trace state.
    fn leave_manual(&mut self, mode: Mode) {
        self.mode = mode;
        self.waypoints.clear();
        self.manual_curve.clear();
        self.finish_requested = false;
        self.selected = None;
        self.hovered = None;
    }

    /// Escape in manual mode: drops the trace in progress. False in any other mode.
    fn cancel_manual(&mut self) -> bool {
        let (mode, cancelled) = self.mode.escape(self.has_margin());
        if cancelled {
            self.leave_manual(mode);
        }
        cancelled
    }

    /// Asks for a trace through the waypoints; fewer than three have no loop yet.
    fn trace_waypoints(&mut self, object: Option<usize>) {
        if self.waypoints.len() >= 3 {
            let controls = self.waypoints.clone();
            self.requests.push(Request::Trace {
                object,
                controls,
                edit: Edit::Manual,
            });
        } else {
            self.manual_curve.clear();
        }
    }

    /// A surface click in manual mode: selects the waypoint under it, else adds one: appended
    /// up to three, then at the best position along the loop.
    fn click_manual(&mut self, object: usize, p: DVec3) {
        if let Some(i) = nearest(&self.waypoints, p, NEAR) {
            self.selected = Some(i);
            return;
        }
        let i = if self.waypoints.len() < 3 {
            self.waypoints.len()
        } else {
            insertion_index(&self.waypoints, p)
        };
        self.waypoints.insert(i, p);
        self.selected = Some(i);
        self.trace_waypoints(Some(object));
    }

    /// Delete in manual mode: the hovered, else the selected, else the last waypoint.
    fn delete_manual(&mut self) {
        let n = self.waypoints.len();
        if n == 0 {
            return;
        }
        let i = self.hovered.or(self.selected).unwrap_or(n - 1).min(n - 1);
        self.waypoints.remove(i);
        self.hovered = None;
        self.selected = None;
        self.trace_waypoints(None);
    }
}

/// Index of the point of `points` nearest to `p` within `radius`.
fn nearest(points: &[DVec3], p: DVec3, radius: f64) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(i, q)| (i, q.distance(p)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The viewport tool of the Margin step.
pub struct MarginTool {
    shared: Rc<RefCell<Shared>>,
}

impl Tool for MarginTool {
    fn name(&self) -> &'static str {
        MARGIN
    }

    fn hover(&mut self, pointer: Option<&Pointer>) {
        let mut s = self.shared.borrow_mut();
        let points = if s.mode == Mode::Manual {
            &s.waypoints
        } else {
            &s.controls
        };
        s.hovered = pointer
            .and_then(|p| p.hit)
            .and_then(|h| nearest(points, h.point, NEAR));
    }

    fn click(&mut self, pointer: &Pointer) {
        let Some(hit) = pointer.hit else { return };
        let mut s = self.shared.borrow_mut();
        if s.tooth.is_none() {
            return;
        }
        if s.mode == Mode::Manual {
            s.click_manual(hit.object, hit.point);
        } else if s.controls.is_empty() {
            s.requests.push(Request::Propose {
                object: hit.object,
                seed: hit.point,
            });
        } else if let Some(i) = nearest(&s.controls, hit.point, NEAR) {
            s.selected = Some(i);
        } else if nearest(&s.curve, hit.point, NEAR).is_some() {
            let i = insertion_index(&s.controls, hit.point);
            s.controls.insert(i, hit.point);
            s.selected = Some(i);
            let controls = s.controls.clone();
            s.requests.push(Request::Trace {
                object: Some(hit.object),
                controls,
                edit: Edit::Insert,
            });
        }
    }

    fn handles(&self) -> Vec<DVec3> {
        let s = self.shared.borrow();
        if s.mode == Mode::Manual {
            s.waypoints.clone()
        } else {
            s.controls.clone()
        }
    }

    fn drag_start(&mut self, handle: usize, _pointer: &Pointer) {
        self.shared.borrow_mut().selected = Some(handle);
    }

    fn drag(&mut self, handle: usize, pointer: &Pointer) {
        let Some(hit) = pointer.hit else { return };
        let mut s = self.shared.borrow_mut();
        if s.mode == Mode::Manual {
            match s.waypoints.get_mut(handle) {
                Some(w) if *w != hit.point => *w = hit.point,
                _ => return,
            }
            s.trace_waypoints(Some(hit.object));
            return;
        }
        let Some(c) = s.controls.get_mut(handle) else {
            return;
        };
        if *c == hit.point {
            return;
        }
        *c = hit.point;
        let controls = s.controls.clone();
        s.requests.push(Request::Trace {
            object: Some(hit.object),
            controls,
            edit: Edit::Move,
        });
    }

    fn drag_end(&mut self, _handle: usize, _pointer: &Pointer) {
        self.shared.borrow_mut().requests.push(Request::DragEnd);
    }

    fn cancel(&mut self) -> bool {
        self.shared.borrow_mut().cancel_manual()
    }

    fn confirm(&mut self) {
        let mut s = self.shared.borrow_mut();
        s.finish_requested = s.mode == Mode::Manual;
    }

    fn delete(&mut self) {
        let mut s = self.shared.borrow_mut();
        if s.mode == Mode::Manual {
            s.delete_manual();
            return;
        }
        let n = s.controls.len();
        if n <= MIN_CONTROLS {
            return;
        }
        let i = s.hovered.or(s.selected).unwrap_or(n - 1).min(n - 1);
        s.controls.remove(i);
        s.hovered = None;
        s.selected = None;
        let controls = s.controls.clone();
        s.requests.push(Request::Trace {
            object: None,
            controls,
            edit: Edit::Delete,
        });
    }

    fn overlays(&self, palette: &Palette) -> Vec<Overlay> {
        let s = self.shared.borrow();
        let accent = rgb(palette.accent);
        let mut out = Vec::new();
        if s.mode == Mode::Manual {
            if s.manual_curve.len() >= 3 {
                out.push(Overlay::Polyline {
                    points: s.manual_curve.iter().map(|p| p.as_vec3()).collect(),
                    closed: true,
                    color: accent,
                    width_px: 2.5,
                });
            }
            if s.waypoints.len() >= 2 {
                out.push(Overlay::Polyline {
                    points: s.waypoints.iter().map(|p| p.as_vec3()).collect(),
                    closed: false,
                    color: [1.0; 3],
                    width_px: 1.0,
                });
            }
            if !s.waypoints.is_empty() {
                out.push(Overlay::Points {
                    points: s.waypoints.iter().map(|p| p.as_vec3()).collect(),
                    color: accent,
                    radius_px: 5.0,
                    outline: Some([1.0; 3]),
                });
            }
            if let Some(p) = s.hovered.or(s.selected).and_then(|i| s.waypoints.get(i)) {
                out.push(Overlay::Points {
                    points: vec![p.as_vec3()],
                    color: [1.0; 3],
                    radius_px: 7.0,
                    outline: Some(accent),
                });
            }
            return out;
        }
        if s.curve.len() >= 2 {
            out.push(Overlay::Polyline {
                points: s.curve.iter().map(|p| p.as_vec3()).collect(),
                closed: true,
                color: accent,
                width_px: 2.5,
            });
        }
        if !s.controls.is_empty() {
            out.push(Overlay::Points {
                points: s.controls.iter().map(|p| p.as_vec3()).collect(),
                color: accent,
                radius_px: 5.0,
                outline: Some([1.0; 3]),
            });
        }
        if let Some(p) = s.hovered.or(s.selected).and_then(|i| s.controls.get(i)) {
            out.push(Overlay::Points {
                points: vec![p.as_vec3()],
                color: [1.0; 3],
                radius_px: 7.0,
                outline: Some(accent),
            });
        }
        out
    }
}

/// Sets (or with `None` clears) the margin of `tooth` and records the edit as `label`. Edits in
/// the same open `group` merge into one undo step.
pub fn set_margin(
    design: &mut Design,
    history: &mut History,
    tooth: Fdi,
    margin: Option<Vec<DVec3>>,
    label: &str,
    group: Option<EditGroup>,
) {
    let before = design.clone();
    match margin {
        Some(points) => design.teeth.entry(tooth).or_default().margin = Some(points),
        None => {
            if let Some(t) = design.teeth.get_mut(&tooth) {
                t.margin = None;
                if *t == ToothDesign::default() {
                    design.teeth.remove(&tooth);
                }
            }
        }
    }
    history.record(label, before, design, group);
}

/// The tooth after (`forward`) or before `current` in a step's `teeth`; `None` when the step is
/// to be left.
pub fn neighbour_tooth(teeth: &[Fdi], current: Option<Fdi>, forward: bool) -> Option<Fdi> {
    let i = teeth.iter().position(|t| Some(*t) == current)?;
    if forward {
        teeth.get(i + 1).copied()
    } else {
        i.checked_sub(1).map(|j| teeth[j])
    }
}

/// The scan a tooth's preparation is on: the jaw scan of its arch.
pub fn arch_scan(objects: &[SceneObject], tooth: Fdi) -> Option<MeshId> {
    let role = match tooth.arch() {
        Arch::Upper => ScanRole::Upper,
        Arch::Lower => ScanRole::Lower,
    };
    objects
        .iter()
        .find(|o| o.role == role && o.loaded().is_some())
        .map(|o| o.id)
}

/// The loaded scan nearest to `p`, and the point on it.
pub fn nearest_scan(objects: &[SceneObject], p: DVec3) -> Option<(MeshId, DVec3)> {
    objects
        .iter()
        .filter_map(|o| {
            let l = o.loaded()?;
            let c = l.bvh.closest_point(&l.mesh, p)?;
            Some((o.id, c.point, c.distance))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(id, point, _)| (id, point))
}

enum Slot {
    Building,
    /// The tracer and the scan's own occlusal direction, when one can be found.
    Ready(Arc<MarginTracer>, Option<DVec3>),
    Failed(String),
}

#[derive(Clone, Debug)]
enum JobKind {
    /// `axis` is the jaw frame's occlusal direction, once the jaw is oriented.
    Propose {
        seed: DVec3,
        axis: Option<DVec3>,
    },
    Trace(Vec<DVec3>),
}

#[derive(Clone, Debug)]
struct Job {
    tooth: Fdi,
    scan: MeshId,
    kind: JobKind,
    edit: Edit,
    group: Option<EditGroup>,
}

enum Message {
    Tracer(MeshId, Result<(MarginTracer, Option<DVec3>), String>),
    Job(Job, Result<Margin, String>),
}

/// What the step needs from the app for one frame.
pub struct StepFrame<'a> {
    pub design: &'a mut Design,
    pub history: &'a mut History,
    pub objects: &'a [SceneObject],
    /// Tooth being worked on.
    pub tooth: Option<Fdi>,
    /// The jaw frame's occlusal direction, once the jaw is oriented.
    pub axis: Option<DVec3>,
}

/// The app side of the Margin step: tracers, worker jobs and undoable edits.
pub struct MarginStep {
    shared: Rc<RefCell<Shared>>,
    tracers: HashMap<MeshId, Slot>,
    waiting: VecDeque<Job>,
    running: bool,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    /// The stored curve (and its tooth) the controls were derived from or last written.
    synced: (Option<Fdi>, Option<Vec<DVec3>>),
    /// Scan the current tooth's margin was last traced on.
    scan: Option<MeshId>,
    seeds: HashMap<Fdi, (MeshId, DVec3)>,
    next_group: u64,
    /// The open drag's group, and whether the drag has ended.
    drag: Option<(EditGroup, bool)>,
    error: Option<String>,
    repaint: Option<egui::Context>,
}

impl MarginStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self {
            shared: Rc::default(),
            tracers: HashMap::new(),
            waiting: VecDeque::new(),
            running: false,
            tx,
            rx,
            synced: (None, None),
            scan: None,
            seeds: HashMap::new(),
            next_group: 1,
            drag: None,
            error: None,
            repaint,
        }
    }

    /// Forgets the open case's tracers, jobs and seeds.
    pub fn reset(&mut self) {
        *self = Self::new(self.repaint.take());
    }

    /// A tool for the viewport that shares this step's state.
    pub fn tool(&self) -> Box<dyn Tool> {
        Box::new(MarginTool {
            shared: self.shared.clone(),
        })
    }

    /// A tracer is being built or a job is waiting or running.
    pub fn busy(&self) -> bool {
        self.running
            || !self.waiting.is_empty()
            || self.tracers.values().any(|s| matches!(s, Slot::Building))
    }

    /// What the step panel shows for `tooth`.
    pub fn card(&self, tooth: Fdi) -> MarginCard {
        let s = self.shared.borrow();
        let building = self.tracers.values().any(|s| matches!(s, Slot::Building));
        MarginCard {
            tooth,
            length_mm: (s.tooth == Some(tooth))
                .then(|| match s.mode {
                    Mode::Manual => &s.manual_curve,
                    _ => &s.curve,
                })
                .filter(|c| c.len() >= 3)
                .map(|c| curve_length(c)),
            waypoints: (s.mode == Mode::Manual).then_some(s.waypoints.len()),
            can_finish: s.mode == Mode::Manual
                && s.manual_curve.len() >= 3
                && !self.busy()
                && s.waypoints.len() >= 3,
            busy: if building {
                Some("Preparing scan…")
            } else if self.busy() {
                Some("Tracing margin…")
            } else {
                None
            },
            error: self.error.clone(),
            can_detect_again: s.mode == Mode::Manual || self.seeds.contains_key(&tooth),
        }
    }

    /// Starts tracing the margin by hand (the "Trace by hand" button).
    pub fn trace_by_hand(&mut self) {
        self.waiting.retain(|j| j.edit != Edit::Manual);
        self.shared.borrow_mut().enter_manual(Vec::new());
        self.error = None;
    }

    /// Starts a manual trace of `tooth` through `points` on `scan` (the `--margin-points` flag).
    pub fn trace_points(&mut self, tooth: Fdi, scan: MeshId, points: Vec<DVec3>) {
        self.trace_by_hand();
        {
            let mut s = self.shared.borrow_mut();
            s.tooth = Some(tooth);
            s.waypoints = points.clone();
        }
        self.synced.0 = Some(tooth);
        self.push(Job {
            tooth,
            scan,
            kind: JobKind::Trace(points),
            edit: Edit::Manual,
            group: None,
        });
    }

    /// Done: the manual trace is stored once the worker has traced the latest waypoints.
    pub fn finish_trace(&mut self) {
        let mut s = self.shared.borrow_mut();
        s.finish_requested = s.mode == Mode::Manual;
    }

    /// Cancel: the manual trace in progress is dropped.
    pub fn cancel_trace(&mut self) {
        self.waiting.retain(|j| j.edit != Edit::Manual);
        self.shared.borrow_mut().cancel_manual();
    }

    /// Stores the finished manual trace as one undoable edit and returns to edit mode.
    fn commit_manual(&mut self, design: &mut Design, history: &mut History) {
        if !self.shared.borrow().finish_requested {
            return;
        }
        if self.running || self.waiting.iter().any(|j| j.edit == Edit::Manual) {
            return;
        }
        let mut s = self.shared.borrow_mut();
        s.finish_requested = false;
        let (Some(tooth), true) = (s.tooth, s.mode == Mode::Manual && s.manual_curve.len() >= 3)
        else {
            return;
        };
        let curve = std::mem::take(&mut s.manual_curve);
        let label = Edit::Manual.label(tooth);
        set_margin(design, history, tooth, Some(curve.clone()), &label, None);
        let mode = s.mode.done();
        s.leave_manual(mode);
        s.controls = control_points(&curve, default_control_count(curve_length(&curve)));
        s.curve = curve.clone();
        self.synced = (Some(tooth), Some(curve));
    }

    /// Proposes a margin for `tooth` from `seed` on `scan`.
    /// `axis` is the jaw frame's occlusal direction; without one the proposal uses the scan's.
    pub fn propose(&mut self, tooth: Fdi, scan: MeshId, seed: DVec3, axis: Option<DVec3>) {
        self.seeds.insert(tooth, (scan, seed));
        self.error = None;
        self.waiting.retain(|j| j.edit != Edit::Manual);
        self.shared.borrow_mut().leave_manual(Mode::Auto);
        self.push(Job {
            tooth,
            scan,
            kind: JobKind::Propose { seed, axis },
            edit: Edit::Detect,
            group: None,
        });
    }

    /// Proposes again from the last seed of `tooth`.
    pub fn detect_again(&mut self, tooth: Fdi, axis: Option<DVec3>) {
        self.waiting.retain(|j| j.edit != Edit::Manual);
        self.shared.borrow_mut().leave_manual(Mode::Auto);
        self.error = None;
        if let Some(&(scan, seed)) = self.seeds.get(&tooth) {
            self.propose(tooth, scan, seed, axis);
        }
    }

    /// Clears the margin of `tooth`.
    pub fn clear(&mut self, design: &mut Design, history: &mut History, tooth: Fdi) {
        self.waiting.retain(|j| j.tooth != tooth);
        set_margin(
            design,
            history,
            tooth,
            None,
            &Edit::Clear.label(tooth),
            None,
        );
        self.error = None;
    }

    /// Latest wins: a new job replaces a waiting one for the same tooth.
    fn push(&mut self, job: Job) {
        self.waiting.retain(|j| j.tooth != job.tooth);
        self.waiting.push_back(job);
    }

    /// Re-derives the controls when the stored curve of the current tooth is not the one they
    /// came from (undo, redo, case load, another tooth). Returns true when it did.
    fn sync(&mut self, design: &Design, tooth: Option<Fdi>) -> bool {
        let stored = tooth
            .and_then(|t| design.teeth.get(&t))
            .and_then(|t| t.margin.clone());
        if self.synced.0 == tooth && self.synced.1 == stored {
            return false;
        }
        let mut s = self.shared.borrow_mut();
        if self.synced.0 != tooth {
            self.error = None;
            self.scan = None;
            s.leave_manual(Mode::settled(stored.is_some()));
        } else if s.mode != Mode::Manual {
            s.mode = Mode::settled(stored.is_some());
        }
        s.tooth = tooth;
        s.curve = stored.clone().unwrap_or_default();
        s.controls = control_points(&s.curve, default_control_count(curve_length(&s.curve)));
        s.hovered = None;
        s.selected = None;
        self.synced = (tooth, stored);
        true
    }

    /// Runs one frame: re-derives the controls, takes the tool's requests, starts and finishes
    /// worker jobs.
    pub fn update(&mut self, f: StepFrame<'_>) {
        self.sync(f.design, f.tooth);
        let requests = std::mem::take(&mut self.shared.borrow_mut().requests);
        for request in requests {
            let Some(tooth) = f.tooth else { continue };
            let object_id = |o: Option<usize>| o.and_then(|i| f.objects.get(i)).map(|o| o.id);
            match request {
                Request::Propose { object, seed } => {
                    if let Some(scan) = object_id(Some(object)) {
                        self.propose(tooth, scan, seed, f.axis);
                    }
                }
                Request::Trace {
                    object,
                    controls,
                    edit,
                } => {
                    let Some(scan) = object_id(object)
                        .or(self.scan)
                        .or_else(|| arch_scan(f.objects, tooth))
                    else {
                        continue;
                    };
                    let group = (edit == Edit::Move).then(|| match self.drag {
                        Some((g, false)) => g,
                        _ => {
                            let g = EditGroup(self.next_group);
                            self.next_group += 1;
                            self.drag = Some((g, false));
                            g
                        }
                    });
                    self.push(Job {
                        tooth,
                        scan,
                        kind: JobKind::Trace(controls),
                        edit,
                        group,
                    });
                }
                Request::DragEnd => {
                    if let Some((_, ended)) = &mut self.drag {
                        *ended = true;
                    }
                }
            }
        }
        if let Some(scan) = f.tooth.and_then(|t| arch_scan(f.objects, t)) {
            self.ensure_tracer(scan, f.objects);
        }
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Tracer(scan, result) => {
                    let slot = match result {
                        Ok((t, occlusal)) => Slot::Ready(Arc::new(t), occlusal),
                        Err(e) => Slot::Failed(e),
                    };
                    self.tracers.insert(scan, slot);
                }
                Message::Job(job, result) => {
                    self.running = false;
                    self.finish(job, result, f.design, f.history);
                }
            }
        }
        self.start_next(f.objects);
        self.commit_manual(f.design, f.history);
        self.close_drag(f.history);
    }

    fn finish(
        &mut self,
        job: Job,
        result: Result<Margin, String>,
        design: &mut Design,
        history: &mut History,
    ) {
        if job.edit == Edit::Manual {
            // A preview of the hand-traced loop; Done stores it.
            let mut s = self.shared.borrow_mut();
            if s.mode != Mode::Manual || s.waypoints.len() < 3 || s.tooth != Some(job.tooth) {
                return;
            }
            match result {
                Ok(m) if m.points.len() >= 3 => {
                    s.manual_curve = m.points;
                    self.scan = Some(job.scan);
                    self.error = None;
                }
                Ok(_) => {
                    s.manual_curve.clear();
                    self.error = Some("No margin found there".to_owned());
                }
                Err(e) => {
                    s.manual_curve.clear();
                    self.error = Some(e);
                }
            }
            return;
        }
        if let JobKind::Propose { seed, .. } = job.kind {
            // A failed or implausible detection hands over to tracing by hand.
            let found = result.as_ref().is_ok_and(|m| plausible(&m.points, seed));
            if !found {
                self.error = Some(match result {
                    Err(e) => e,
                    Ok(_) => MANUAL_PROMPT.to_owned(),
                });
                self.shared.borrow_mut().enter_manual(Vec::new());
                return;
            }
        }
        let margin = match result {
            Ok(m) if m.points.len() >= 3 => m.points,
            Ok(_) => {
                self.error = Some("No margin found there".to_owned());
                return;
            }
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.error = None;
        let label = job.edit.label(job.tooth);
        set_margin(
            design,
            history,
            job.tooth,
            Some(margin.clone()),
            &label,
            job.group,
        );
        if self.synced.0 == Some(job.tooth) {
            self.scan = Some(job.scan);
            let mut s = self.shared.borrow_mut();
            s.curve = margin.clone();
            s.mode = Mode::Edit;
            if matches!(job.kind, JobKind::Propose { .. }) {
                s.controls = control_points(&margin, default_control_count(curve_length(&margin)));
                s.selected = None;
                s.hovered = None;
            }
            self.synced.1 = Some(margin);
        }
    }

    /// Closes the drag's undo group once it has ended and its last trace has been recorded.
    fn close_drag(&mut self, history: &mut History) {
        if let Some((group, true)) = self.drag {
            let pending = self.running || self.waiting.iter().any(|j| j.group == Some(group));
            if !pending {
                history.end_group();
                self.drag = None;
            }
        }
    }

    /// The tracer of `scan`, building it on a worker when there is none: `Ok(None)` while it is
    /// being built.
    pub fn tracer(
        &mut self,
        scan: MeshId,
        objects: &[SceneObject],
    ) -> Result<Option<Arc<MarginTracer>>, String> {
        self.ensure_tracer(scan, objects);
        match self.tracers.get(&scan) {
            Some(Slot::Ready(t, _)) => Ok(Some(t.clone())),
            Some(Slot::Failed(e)) => Err(e.clone()),
            Some(Slot::Building) => Ok(None),
            None => Err("The scan is no longer loaded".to_owned()),
        }
    }

    fn ensure_tracer(&mut self, scan: MeshId, objects: &[SceneObject]) {
        if self.tracers.contains_key(&scan) {
            return;
        }
        let Some(mesh) = objects
            .iter()
            .find(|o| o.id == scan)
            .and_then(|o| o.loaded())
            .map(|l| Arc::new(l.mesh.clone()))
        else {
            return;
        };
        self.tracers.insert(scan, Slot::Building);
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let work = move || {
            // The scan's occlusal direction is computed once per scan, with its tracer.
            let occlusal = dentcraft_design::occlusal_direction(&mesh).ok();
            MarginTracer::new(mesh).map(|t| (t, occlusal)).map_err(|e| {
                eprintln!("dentcraft: margin tracer: {e}");
                "This scan can't be traced: it has no usable surface. Check the scan, or \
                     load it again."
                    .to_owned()
            })
        };
        crate::crash::spawn_worker("margin tracer", work, move |result| {
            let _ = tx.send(Message::Tracer(scan, result.and_then(|r| r)));
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }

    fn start_next(&mut self, objects: &[SceneObject]) {
        if self.running {
            return;
        }
        let Some(job) = self.waiting.front() else {
            return;
        };
        let scan = job.scan;
        self.ensure_tracer(scan, objects);
        let (tracer, occlusal) = match self.tracers.get(&scan) {
            Some(Slot::Ready(t, occlusal)) => (t.clone(), *occlusal),
            Some(Slot::Building) => return,
            Some(Slot::Failed(e)) => {
                self.error = Some(e.clone());
                self.waiting.pop_front();
                return;
            }
            None => {
                self.error = Some("The scan is no longer loaded".to_owned());
                self.waiting.pop_front();
                return;
            }
        };
        let Some(job) = self.waiting.pop_front() else {
            return;
        };
        self.running = true;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let sent = job.clone();
        let work = move || {
            let traced = match &job.kind {
                JobKind::Propose { seed, axis } => {
                    match proposal_axis(tracer.mesh(), *axis, occlusal, *seed) {
                        Some(axis) => tracer.propose(*seed, axis),
                        None => Err(DesignError::InvalidAxis),
                    }
                }
                JobKind::Trace(controls) => tracer.trace(controls),
            };
            traced.map_err(|e| {
                // The technical reason goes to the log; the panel shows what to do next.
                eprintln!("dentcraft: margin {:?} on {}: {e}", job.edit, job.tooth);
                user_message(&job.kind, &e)
            })
        };
        crate::crash::spawn_worker("margin", work, move |result| {
            let _ = tx.send(Message::Job(sent, result.and_then(|r| r)));
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }
}

/// The direction out of the preparation for a proposal: the jaw frame's occlusal direction when
/// the jaw is oriented, else the scan's own (`scan_occlusal`), else the area-weighted mean
/// normal of the surface within 3 mm of the seed. Never the camera's view direction: from a
/// facial view that is horizontal and no margin can be found around it.
pub fn proposal_axis(
    mesh: &TriMesh,
    jaw: Option<DVec3>,
    scan_occlusal: Option<DVec3>,
    seed: DVec3,
) -> Option<DVec3> {
    let valid = |a: DVec3| (a.is_finite() && a.length_squared() > 1e-24).then(|| a.normalize());
    if let Some(axis) = jaw
        .and_then(valid)
        .or_else(|| scan_occlusal.and_then(valid))
    {
        return Some(axis);
    }
    let sum: DVec3 = (0..mesh.triangles.len())
        .filter(|&i| {
            let [a, b, c] = mesh.triangle(i);
            ((a + b + c) / 3.0).distance_squared(seed) <= 3.0 * 3.0
        })
        .map(|i| mesh.triangle_cross(i))
        .sum();
    valid(sum)
}

/// A short sentence for the panel, with the next thing to try. Internal reasons stay in the log.
fn user_message(kind: &JobKind, error: &DesignError) -> String {
    match (kind, error) {
        (
            _,
            DesignError::EmptyRegion | DesignError::DegenerateRegion | DesignError::InvalidMesh,
        ) => {
            "This scan can't be traced: it has no usable surface here. Check the scan, or load it \
             again."
                .to_owned()
        }
        (JobKind::Propose { .. }, _) => MANUAL_PROMPT.to_owned(),
        (JobKind::Trace(_), _) => {
            "Couldn't run the margin through these points. Move the handles onto the finish \
             line, or click Detect again."
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proposal_axis_never_comes_from_the_view() {
        // A quad tilted 45 degrees about X, facing +Y+Z, with a far triangle facing -Z.
        let mesh = TriMesh::new(
            vec![
                DVec3::new(-1.0, -1.0, 1.0),
                DVec3::new(1.0, -1.0, 1.0),
                DVec3::new(1.0, 1.0, -1.0),
                DVec3::new(-1.0, 1.0, -1.0),
                DVec3::new(20.0, 0.0, 0.0),
                DVec3::new(21.0, 1.0, 0.0),
                DVec3::new(21.0, 0.0, 0.0),
            ],
            vec![[0, 1, 2], [0, 2, 3], [4, 5, 6]],
        );
        let seed = DVec3::ZERO;
        let jaw = DVec3::new(0.0, 0.0, 2.0);
        let scan = DVec3::new(0.0, 3.0, 0.0);
        assert_eq!(
            proposal_axis(&mesh, Some(jaw), Some(scan), seed),
            Some(DVec3::Z)
        );
        assert_eq!(proposal_axis(&mesh, None, Some(scan), seed), Some(DVec3::Y));
        // No direction at all: the surface around the seed decides; the far triangle is ignored.
        let patch = proposal_axis(&mesh, None, None, seed).unwrap();
        assert!((patch - DVec3::new(0.0, 1.0, 1.0).normalize()).length() < 1e-12);
        assert_eq!(
            proposal_axis(&mesh, Some(DVec3::ZERO), None, seed),
            Some(patch)
        );
        assert_eq!(proposal_axis(&mesh, None, None, DVec3::splat(50.0)), None);
    }

    #[test]
    fn a_proposal_without_a_jaw_frame_uses_the_scan_direction() {
        // Without a jaw frame the app used to propose along the camera's view direction, which
        // is horizontal in a facial view: the projection degenerated and nothing was found.
        let die =
            dentcraft_design::prepared_tooth(&dentcraft_design::ToothParams::default()).unwrap();
        let occlusal = dentcraft_design::occlusal_direction(&die.mesh).ok();
        let top = die.axis * 12.0;
        let tracer = MarginTracer::new(Arc::new(die.mesh.clone())).unwrap();
        let axis = proposal_axis(&die.mesh, None, occlusal, top).unwrap();
        assert!(axis.dot(die.axis) > 0.9, "axis {axis}");
        let margin = tracer.propose(top, axis).unwrap();
        assert!(margin.points.len() > 100);
        let facial = die.axis.any_orthonormal_vector();
        assert!(tracer.propose(top, facial).is_err());
    }

    #[test]
    fn failures_read_as_a_next_step_not_an_internal_error() {
        let propose = JobKind::Propose {
            seed: DVec3::ZERO,
            axis: None,
        };
        let trace = JobKind::Trace(Vec::new());
        for kind in [&propose, &trace] {
            for error in [
                DesignError::InvalidMargin,
                DesignError::InvalidAxis,
                DesignError::EmptyRegion,
            ] {
                let text = user_message(kind, &error);
                assert!(!text.contains(&error.to_string()), "{text}");
                assert!(!text.contains("polygon"), "{text}");
                assert!(text.ends_with('.'), "{text}");
            }
        }
        assert_eq!(
            user_message(&propose, &DesignError::InvalidMargin),
            MANUAL_PROMPT
        );
    }

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn ring(radius: f64, n: usize) -> Vec<DVec3> {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                DVec3::new(radius * a.cos(), radius * a.sin(), 8.0)
            })
            .collect()
    }

    #[test]
    fn next_and_back_walk_the_teeth_of_a_step() {
        let teeth = [t(24), t(26), t(21)];
        assert_eq!(neighbour_tooth(&teeth, Some(t(24)), true), Some(t(26)));
        assert_eq!(neighbour_tooth(&teeth, Some(t(26)), true), Some(t(21)));
        assert_eq!(neighbour_tooth(&teeth, Some(t(21)), true), None, "leave");
        assert_eq!(neighbour_tooth(&teeth, Some(t(26)), false), Some(t(24)));
        assert_eq!(neighbour_tooth(&teeth, Some(t(24)), false), None, "leave");
        assert_eq!(neighbour_tooth(&teeth, None, true), None);
        assert_eq!(neighbour_tooth(&teeth, Some(t(36)), false), None);
        assert_eq!(neighbour_tooth(&[], Some(t(24)), true), None);
    }

    #[test]
    fn controls_are_rederived_when_the_stored_curve_changes() {
        let mut step = MarginStep::new(None);
        let mut design = Design::default();
        let tooth = t(36);
        // No margin: no controls.
        assert!(step.sync(&design, Some(tooth)));
        assert!(step.shared.borrow().controls.is_empty());
        assert!(!step.sync(&design, Some(tooth)), "unchanged");

        // A stored curve (case load, redo) gives evenly spaced controls on it.
        let curve = ring(4.0, 252);
        design.teeth.entry(tooth).or_default().margin = Some(curve.clone());
        assert!(step.sync(&design, Some(tooth)));
        let n = default_control_count(curve_length(&curve));
        {
            let s = step.shared.borrow();
            assert_eq!(s.controls.len(), n);
            assert_eq!(s.curve, curve);
            let on_ring = |c: &DVec3| (c.truncate().length() - 4.0).abs() < 1e-3 && c.z == 8.0;
            assert!(s.controls.iter().all(on_ring), "controls lie on the curve");
        }
        // The user's own control edits survive frames where the store did not change.
        step.shared.borrow_mut().controls.pop();
        assert!(!step.sync(&design, Some(tooth)));
        assert_eq!(step.shared.borrow().controls.len(), n - 1);

        // Undo back to no margin clears them; another tooth starts from its own store.
        design.teeth.clear();
        assert!(step.sync(&design, Some(tooth)));
        assert!(step.shared.borrow().controls.is_empty());
        design.teeth.entry(t(37)).or_default().margin = Some(curve);
        assert!(step.sync(&design, Some(t(37))));
        assert_eq!(step.shared.borrow().controls.len(), n);
        assert_eq!(step.shared.borrow().tooth, Some(t(37)));
    }

    #[test]
    fn a_drag_is_one_undo_step_and_edits_have_their_labels() {
        let tooth = t(24);
        let mut design = Design::default();
        let mut history = History::default();
        let detected = ring(4.0, 40);
        let label = Edit::Detect.label(tooth);
        set_margin(
            &mut design,
            &mut history,
            tooth,
            Some(detected.clone()),
            &label,
            None,
        );
        assert_eq!(history.undo_label(), Some("Detect margin 24"));

        // Three trace results of one drag, then the release.
        let group = Some(EditGroup(7));
        for r in [4.1, 4.2, 4.3] {
            let label = Edit::Move.label(tooth);
            set_margin(
                &mut design,
                &mut history,
                tooth,
                Some(ring(r, 40)),
                &label,
                group,
            );
        }
        history.end_group();
        assert_eq!(history.undo_label(), Some("Move margin point"));
        let inserted = ring(4.3, 41);
        let label = Edit::Insert.label(tooth);
        set_margin(
            &mut design,
            &mut history,
            tooth,
            Some(inserted),
            &label,
            None,
        );
        assert_eq!(history.undo_label(), Some("Insert margin point"));
        history.undo(&mut design);
        assert_eq!(design.teeth[&tooth].margin, Some(ring(4.3, 40)));
        // One undo takes the whole drag back.
        assert_eq!(
            history.undo(&mut design).as_deref(),
            Some("Move margin point")
        );
        assert_eq!(design.teeth[&tooth].margin, Some(detected));

        // Clearing removes the tooth entry, so the design is empty again.
        set_margin(
            &mut design,
            &mut history,
            tooth,
            None,
            &Edit::Clear.label(tooth),
            None,
        );
        assert_eq!(history.undo_label(), Some("Clear margin 24"));
        assert!(design.is_empty());
        assert_eq!(Edit::Delete.label(tooth), "Delete margin point");
    }

    #[test]
    fn the_tool_proposes_inserts_and_deletes_through_requests() {
        use crate::tools::SurfaceHit;
        let step = MarginStep::new(None);
        let mut tool = step.tool();
        let at = |point| Pointer {
            pixel: glam::Vec2::ZERO,
            hit: Some(SurfaceHit { object: 0, point }),
            ray: None,
        };
        let seed = DVec3::new(0.0, 0.0, 12.0);
        // No tooth: clicks do nothing.
        tool.click(&at(seed));
        assert!(step.shared.borrow().requests.is_empty());
        let curve = ring(4.0, 252);
        {
            let mut s = step.shared.borrow_mut();
            s.tooth = Some(t(36));
        }
        tool.click(&at(seed));
        assert_eq!(
            step.shared.borrow_mut().requests.pop(),
            Some(Request::Propose { object: 0, seed })
        );
        {
            let mut s = step.shared.borrow_mut();
            s.controls = control_points(&curve, 16);
            s.curve = curve.clone();
        }
        // A click on the curve between two controls inserts one there.
        let between = curve[8];
        tool.click(&at(between));
        let s = step.shared.borrow_mut().requests.pop();
        match s {
            Some(Request::Trace { controls, edit, .. }) => {
                assert_eq!(edit, Edit::Insert);
                assert_eq!(controls.len(), 17);
                assert_eq!(controls[1], between);
            }
            other => panic!("{other:?}"),
        }
        // A click away from the curve does nothing.
        tool.click(&at(DVec3::new(0.0, 0.0, 12.0)));
        assert!(step.shared.borrow().requests.is_empty());
        // Delete removes the selected control, down to three.
        tool.delete();
        assert_eq!(step.shared.borrow().controls.len(), 16);
        step.shared.borrow_mut().controls.truncate(3);
        tool.delete();
        assert_eq!(step.shared.borrow().controls.len(), 3);
    }

    fn pointer(point: DVec3) -> Pointer {
        use crate::tools::SurfaceHit;
        Pointer {
            pixel: glam::Vec2::ZERO,
            hit: Some(SurfaceHit { object: 0, point }),
            ray: None,
        }
    }

    fn job(kind: JobKind, edit: Edit) -> Job {
        Job {
            tooth: t(33),
            scan: MeshId(1),
            kind,
            edit,
            group: None,
        }
    }

    fn margin_of(points: Vec<DVec3>) -> Result<Margin, String> {
        Ok(Margin {
            points,
            vertices: Vec::new(),
        })
    }

    fn propose_job() -> Job {
        let seed = DVec3::new(0.0, 0.0, 12.0);
        job(JobKind::Propose { seed, axis: None }, Edit::Detect)
    }

    #[test]
    fn a_failed_or_implausible_detection_switches_to_manual_tracing() {
        let mut design = Design::default();
        let mut history = History::default();
        let mut step = MarginStep::new(None);
        step.sync(&design, Some(t(33)));
        assert_eq!(step.shared.borrow().mode, Mode::Auto);

        // A detection failure: the panel text asks for points, and manual mode starts.
        let kind = JobKind::Propose {
            seed: DVec3::ZERO,
            axis: None,
        };
        let text = user_message(&kind, &DesignError::InvalidMargin);
        step.finish(propose_job(), Err(text), &mut design, &mut history);
        assert_eq!(step.shared.borrow().mode, Mode::Manual);
        assert_eq!(step.error.as_deref(), Some(MANUAL_PROMPT));
        assert!(design.is_empty());

        // Too short (6.3 mm), too long, and far from the click are all implausible.
        let far = ring(4.0, 40).into_iter().map(|p| p + DVec3::X * 30.0);
        for points in [ring(1.0, 40), ring(8.0, 40), far.collect()] {
            step.shared.borrow_mut().leave_manual(Mode::Auto);
            step.finish(propose_job(), margin_of(points), &mut design, &mut history);
            assert_eq!(step.shared.borrow().mode, Mode::Manual);
            assert!(design.is_empty(), "nothing stored");
        }

        // A plausible margin (25 mm, 4 mm from the click) is stored and edited as before.
        step.shared.borrow_mut().leave_manual(Mode::Auto);
        step.finish(
            propose_job(),
            margin_of(ring(4.0, 40)),
            &mut design,
            &mut history,
        );
        assert_eq!(step.shared.borrow().mode, Mode::Edit);
        assert_eq!(history.undo_label(), Some("Detect margin 33"));
    }

    #[test]
    fn done_stores_one_undoable_trace_and_returns_to_editing() {
        let mut design = Design::default();
        let mut history = History::default();
        let mut step = MarginStep::new(None);
        step.sync(&design, Some(t(33)));
        let mut tool = step.tool();
        step.trace_by_hand();
        let points = ring(4.0, 40);
        for p in [points[0], points[10], points[20], points[30]] {
            tool.click(&pointer(p));
        }
        assert_eq!(step.shared.borrow().waypoints.len(), 4);
        // Traces are requested from the third waypoint on.
        assert_eq!(step.shared.borrow().requests.len(), 2);
        step.shared.borrow_mut().requests.clear();
        // Done before any trace arrived stores nothing.
        step.finish_trace();
        step.commit_manual(&mut design, &mut history);
        assert!(design.is_empty());
        assert_eq!(step.shared.borrow().mode, Mode::Manual);

        // The worker's result becomes the preview; Done then stores it.
        let traced = job(JobKind::Trace(points.clone()), Edit::Manual);
        step.finish(traced, margin_of(points.clone()), &mut design, &mut history);
        assert!(design.is_empty(), "the preview is not stored");
        assert_eq!(step.card(t(33)).waypoints, Some(4));
        step.finish_trace();
        step.commit_manual(&mut design, &mut history);
        assert_eq!(design.teeth[&t(33)].margin, Some(points));
        assert_eq!(history.undo_label(), Some("Trace margin 33"));
        {
            let s = step.shared.borrow();
            assert_eq!(s.mode, Mode::Edit);
            assert!(s.waypoints.is_empty() && !s.controls.is_empty());
        }
        assert_eq!(
            history.undo(&mut design).as_deref(),
            Some("Trace margin 33")
        );
        assert!(design.is_empty());
        assert_eq!(Mode::Manual.done(), Mode::Edit);
        assert_eq!(Mode::Auto.done(), Mode::Auto);
    }

    #[test]
    fn detect_again_and_escape_leave_manual_mode() {
        let mut design = Design::default();
        let mut step = MarginStep::new(None);
        step.sync(&design, Some(t(33)));
        let mut tool = step.tool();
        // Detect again returns to automatic even without a stored click.
        step.trace_by_hand();
        assert_eq!(step.shared.borrow().mode, Mode::Manual);
        step.detect_again(t(33), None);
        assert_eq!(step.shared.borrow().mode, Mode::Auto);

        // Escape drops the trace in progress; a second Escape leaves the tool.
        step.trace_by_hand();
        tool.click(&pointer(DVec3::ZERO));
        assert!(tool.cancel());
        {
            let s = step.shared.borrow();
            assert_eq!(s.mode, Mode::Auto);
            assert!(s.waypoints.is_empty());
        }
        assert!(!tool.cancel());

        // With a stored margin, Escape returns to editing it, untouched.
        design.teeth.entry(t(33)).or_default().margin = Some(ring(4.0, 40));
        step.sync(&design, Some(t(33)));
        assert_eq!(step.shared.borrow().mode, Mode::Edit);
        step.trace_by_hand();
        tool.click(&pointer(DVec3::ZERO));
        assert!(tool.cancel());
        assert_eq!(step.shared.borrow().mode, Mode::Edit);
        assert_eq!(step.shared.borrow().curve, ring(4.0, 40));
        assert_eq!(Mode::Manual.escape(false), (Mode::Auto, true));
        assert_eq!(Mode::Edit.escape(true), (Mode::Edit, false));
    }

    #[test]
    fn waypoints_are_appended_then_inserted_along_the_loop() {
        let mut step = MarginStep::new(None);
        step.sync(&Design::default(), Some(t(33)));
        let mut tool = step.tool();
        step.trace_by_hand();
        let p = |x, y| DVec3::new(x, y, 0.0);
        for q in [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)] {
            tool.click(&pointer(q));
        }
        // The first three keep the click order; the fourth lands on the closing side.
        tool.click(&pointer(p(0.0, 10.0)));
        // A point beside the first side goes between its ends.
        tool.click(&pointer(p(5.0, -1.0)));
        let order = step.shared.borrow().waypoints.clone();
        assert_eq!(
            order,
            vec![
                p(0.0, 0.0),
                p(5.0, -1.0),
                p(10.0, 0.0),
                p(10.0, 10.0),
                p(0.0, 10.0)
            ]
        );
        // Clicking on a waypoint selects it, and Delete removes it and re-traces.
        step.shared.borrow_mut().requests.clear();
        tool.click(&pointer(p(10.0, 10.0)));
        assert_eq!(step.shared.borrow().selected, Some(3));
        tool.delete();
        assert_eq!(step.shared.borrow().waypoints.len(), 4);
        assert!(!step.shared.borrow().waypoints.contains(&p(10.0, 10.0)));
        assert_eq!(step.shared.borrow().requests.len(), 1);
        assert_eq!(Edit::Manual.label(t(33)), "Trace margin 33");
    }
}
