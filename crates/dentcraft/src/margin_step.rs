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
use dentcraft_design::{Margin, MarginTracer};
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
}

impl Edit {
    pub fn label(self, tooth: Fdi) -> String {
        match self {
            Self::Detect => format!("Detect margin {tooth}"),
            Self::Move => "Move margin point".to_owned(),
            Self::Insert => "Insert margin point".to_owned(),
            Self::Delete => "Delete margin point".to_owned(),
            Self::Clear => format!("Clear margin {tooth}"),
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
        s.hovered = pointer
            .and_then(|p| p.hit)
            .and_then(|h| nearest(&s.controls, h.point, NEAR));
    }

    fn click(&mut self, pointer: &Pointer) {
        let Some(hit) = pointer.hit else { return };
        let mut s = self.shared.borrow_mut();
        if s.tooth.is_none() {
            return;
        }
        if s.controls.is_empty() {
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
        self.shared.borrow().controls.clone()
    }

    fn drag_start(&mut self, handle: usize, _pointer: &Pointer) {
        self.shared.borrow_mut().selected = Some(handle);
    }

    fn drag(&mut self, handle: usize, pointer: &Pointer) {
        let Some(hit) = pointer.hit else { return };
        let mut s = self.shared.borrow_mut();
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

    fn delete(&mut self) {
        let mut s = self.shared.borrow_mut();
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
    Ready(Arc<MarginTracer>),
    Failed(String),
}

#[derive(Clone, Debug)]
enum JobKind {
    Propose { seed: DVec3, axis: DVec3 },
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
    Tracer(MeshId, Result<MarginTracer, String>),
    Job(Job, Result<Margin, String>),
}

/// What the step needs from the app for one frame.
pub struct StepFrame<'a> {
    pub design: &'a mut Design,
    pub history: &'a mut History,
    pub objects: &'a [SceneObject],
    /// Tooth being worked on.
    pub tooth: Option<Fdi>,
    /// Direction out of the preparation for a proposal.
    pub axis: DVec3,
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
            length_mm: (s.tooth == Some(tooth) && s.curve.len() >= 3)
                .then(|| curve_length(&s.curve)),
            busy: if building {
                Some("Preparing scan…")
            } else if self.busy() {
                Some("Tracing margin…")
            } else {
                None
            },
            error: self.error.clone(),
            can_detect_again: self.seeds.contains_key(&tooth),
        }
    }

    /// Proposes a margin for `tooth` from `seed` on `scan`.
    pub fn propose(&mut self, tooth: Fdi, scan: MeshId, seed: DVec3, axis: DVec3) {
        self.seeds.insert(tooth, (scan, seed));
        self.error = None;
        self.push(Job {
            tooth,
            scan,
            kind: JobKind::Propose { seed, axis },
            edit: Edit::Detect,
            group: None,
        });
    }

    /// Proposes again from the last seed of `tooth`.
    pub fn detect_again(&mut self, tooth: Fdi, axis: DVec3) {
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
                        Ok(t) => Slot::Ready(Arc::new(t)),
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
        self.close_drag(f.history);
    }

    fn finish(
        &mut self,
        job: Job,
        result: Result<Margin, String>,
        design: &mut Design,
        history: &mut History,
    ) {
        let margin = match result {
            Ok(m) if m.points.len() >= 3 => m.points,
            Ok(_) => {
                self.error = Some("No margin found there".to_owned());
                return;
            }
            Err(e) => {
                self.error = Some(capitalize(&e));
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
            Some(Slot::Ready(t)) => Ok(Some(t.clone())),
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
        std::thread::spawn(move || {
            let result = MarginTracer::new(mesh).map_err(|e| e.to_string());
            let _ = tx.send(Message::Tracer(scan, result));
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
        let tracer = match self.tracers.get(&scan) {
            Some(Slot::Ready(t)) => t.clone(),
            Some(Slot::Building) => return,
            Some(Slot::Failed(e)) => {
                self.error = Some(format!("Cannot trace on this scan: {e}"));
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
        std::thread::spawn(move || {
            let result = match &job.kind {
                JobKind::Propose { seed, axis } => tracer.propose(*seed, *axis),
                JobKind::Trace(controls) => tracer.trace(controls),
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(Message::Job(job, result));
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
