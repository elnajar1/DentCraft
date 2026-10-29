//! The Inner surface step: the restoration's fitting surface over the preparation, offset by the
//! cement gap and sealed at the margin. Its parameters are the material's defaults overridden per
//! tooth; every change is an undoable edit of the overrides. The surface is computed on a worker
//! whenever the margin, the axis or a parameter changes (latest request wins) and drawn as its own
//! translucent mesh. The Section toggle cuts the view through the preparation so the gap shows.

use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_case::{Design, Fdi, InnerSurfaceOverrides, Material, ToothDesign};
use dentcraft_design::{
    DesignError, InnerSurface, InnerSurfaceParams, MarginTracer, inner_surface,
};
use dentcraft_mesh::TriMesh;
use dentcraft_render::{Camera, ClipPlane, DrawItem, MeshData, MeshId};
use eframe::egui;
use glam::DVec3;

use crate::axis_step::{prep_region, region_centroid, region_seed};
use crate::history::{EditGroup, History};
use crate::margin_step::{MarginStep, arch_scan};
use crate::objects::SceneObject;
use crate::theme::Palette;
use crate::tools::{Tool, rgb};
use crate::workflow_panel::InnerCard;

/// Name of the inner surface tool.
pub const INNER: &str = "Inner surface";
/// Undo label of "Reset to material".
pub const RESET_LABEL: &str = "Reset inner surface to material";
/// Renderer id of the inner surface mesh, clear of the scans' ids.
const MESH: MeshId = MeshId(u64::MAX - 1);
/// Opacity of the drawn inner surface.
const OPACITY: f32 = 0.65;

/// One editable inner-surface parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InnerParam {
    CementGap,
    SealGap,
    SealWidth,
    TransitionWidth,
}

impl InnerParam {
    pub const ALL: [Self; 4] = [
        Self::CementGap,
        Self::SealGap,
        Self::SealWidth,
        Self::TransitionWidth,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::CementGap => "Cement gap",
            Self::SealGap => "Seal gap",
            Self::SealWidth => "Seal width",
            Self::TransitionWidth => "Transition width",
        }
    }

    /// Decimals shown and kept: micrometres for gaps, hundredths for widths.
    pub fn decimals(self) -> usize {
        match self {
            Self::CementGap | Self::SealGap => 3,
            Self::SealWidth | Self::TransitionWidth => 2,
        }
    }

    /// Values the panel accepts, in millimetres.
    pub fn range(self) -> RangeInclusive<f64> {
        match self {
            Self::CementGap => 0.0..=0.2,
            Self::SealGap => 0.0..=0.1,
            Self::SealWidth => 0.0..=2.0,
            Self::TransitionWidth => 0.0..=2.0,
        }
    }

    /// The parameter's value in `params`.
    pub fn get(self, params: &InnerSurfaceParams) -> f64 {
        match self {
            Self::CementGap => params.cement_gap,
            Self::SealGap => params.seal_gap,
            Self::SealWidth => params.margin_seal_width,
            Self::TransitionWidth => params.transition_width,
        }
    }

    fn slot(self, o: &mut InnerSurfaceOverrides) -> &mut Option<f64> {
        match self {
            Self::CementGap => &mut o.cement_gap,
            Self::SealGap => &mut o.seal_gap,
            Self::SealWidth => &mut o.seal_width,
            Self::TransitionWidth => &mut o.transition_width,
        }
    }

    /// `value` rounded to the shown decimals and clamped to the range.
    fn round(self, value: f64) -> f64 {
        let scale = 10f64.powi(self.decimals() as i32);
        let r = self.range();
        ((value * scale).round() / scale).clamp(*r.start(), *r.end())
    }
}

/// The inner-surface parameters of a tooth: the material's cement gap and seal width (the
/// algorithm's defaults for the rest), overridden by the tooth's own values.
pub fn resolve(material: &Material, overrides: &InnerSurfaceOverrides) -> InnerSurfaceParams {
    let base = InnerSurfaceParams::default();
    InnerSurfaceParams {
        cement_gap: overrides.cement_gap.unwrap_or(material.cement_gap),
        seal_gap: overrides.seal_gap.unwrap_or(base.seal_gap),
        margin_seal_width: overrides.seal_width.unwrap_or(material.margin_seal_width),
        transition_width: overrides.transition_width.unwrap_or(base.transition_width),
        ..base
    }
}

/// Undo label of setting `param` to `value` ("Set cement gap 0.040 mm").
pub fn edit_label(param: InnerParam, value: f64) -> String {
    format!(
        "Set {} {:.*} mm",
        param.name().to_lowercase(),
        param.decimals(),
        value
    )
}

/// Drops `tooth`'s entry when nothing is left in it.
fn prune(design: &mut Design, tooth: Fdi) {
    if design.teeth.get(&tooth) == Some(&ToothDesign::default()) {
        design.teeth.remove(&tooth);
    }
}

/// Sets `param` of `tooth` to `value` (rounded and clamped) and records the edit. A value equal to
/// the material's default removes the override. Edits in the same open `group` (a drag) merge.
pub fn set_param(
    design: &mut Design,
    history: &mut History,
    material: &Material,
    tooth: Fdi,
    param: InnerParam,
    value: f64,
    group: Option<EditGroup>,
) {
    let value = param.round(value);
    let default = param.get(&resolve(material, &InnerSurfaceOverrides::default()));
    let before = design.clone();
    let overrides = &mut design.teeth.entry(tooth).or_default().inner_surface;
    *param.slot(overrides) = ((value - default).abs() > 1e-9).then_some(value);
    prune(design, tooth);
    history.record(&edit_label(param, value), before, design, group);
}

/// Removes every override of `tooth`, back to the material's values.
pub fn reset_params(design: &mut Design, history: &mut History, tooth: Fdi) {
    let before = design.clone();
    if let Some(t) = design.teeth.get_mut(&tooth) {
        t.inner_surface = InnerSurfaceOverrides::default();
    }
    prune(design, tooth);
    history.record(RESET_LABEL, before, design, None);
}

/// The section plane through `centre` that contains `axis` and faces the camera: its normal is the
/// direction towards the camera (`toward_camera`) made perpendicular to the axis, so the half
/// nearer the camera is cut away. Looking along the axis, `up` (the camera's up) stands in.
pub fn section_plane(
    centre: DVec3,
    axis: DVec3,
    toward_camera: DVec3,
    up: DVec3,
) -> Option<ClipPlane> {
    let axis = axis.try_normalize()?;
    let flat = |v: DVec3| (v - axis * v.dot(axis)).try_normalize();
    let normal = flat(toward_camera)
        .filter(|n| n.dot(toward_camera.normalize_or_zero()) > 1e-3)
        .or_else(|| flat(-up))?;
    Some(ClipPlane {
        point: centre.as_vec3(),
        normal: normal.as_vec3(),
    })
}

/// Everything the surface depends on: the tooth, its stored margin, its axis and parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    tooth: Fdi,
    margin: Vec<DVec3>,
    axis: DVec3,
    params: [f64; 4],
}

impl Key {
    /// The key of `tooth` in `design`, once it has a margin and an axis.
    pub fn of(design: &Design, material: &Material, tooth: Fdi) -> Option<Key> {
        let t = design.teeth.get(&tooth)?;
        let params = resolve(material, &t.inner_surface);
        Some(Key {
            tooth,
            margin: t.margin.clone()?,
            axis: t.insertion_axis?,
            params: InnerParam::ALL.map(|p| p.get(&params)),
        })
    }

    fn params(&self) -> InnerSurfaceParams {
        let [cement_gap, seal_gap, margin_seal_width, transition_width] = self.params;
        InnerSurfaceParams {
            cement_gap,
            seal_gap,
            margin_seal_width,
            transition_width,
            ..InnerSurfaceParams::default()
        }
    }
}

/// The preparation found inside a margin.
pub(crate) struct Prep {
    tooth: Fdi,
    margin: Vec<DVec3>,
    /// Triangles of the jaw scan inside the margin.
    pub(crate) region: Vec<u32>,
    pub(crate) centroid: DVec3,
}

/// State shared by the tool and the step.
#[derive(Default)]
struct Shared {
    mesh: Option<Arc<MeshData>>,
    /// The surface the mesh shows, for the Anatomy step.
    surface: Option<Arc<InnerSurface>>,
    /// Centre and axis of the section, while the Section toggle is on.
    section: Option<(DVec3, DVec3)>,
}

/// The viewport tool of the Inner surface step: draws the surface and cuts the view.
pub struct InnerTool {
    shared: Rc<RefCell<Shared>>,
}

impl Tool for InnerTool {
    fn name(&self) -> &'static str {
        INNER
    }

    fn picks_surface(&self) -> bool {
        false
    }

    fn meshes(&self, palette: &Palette) -> Vec<(DrawItem, Arc<MeshData>)> {
        let item = DrawItem {
            mesh: MESH,
            color: rgb(palette.inner_surface),
            opacity: OPACITY,
            visible: true,
            vertex_colors: false,
            clip: true,
        };
        self.shared
            .borrow()
            .mesh
            .clone()
            .map(|m| (item, m))
            .into_iter()
            .collect()
    }

    fn clip_plane(&self, camera: &Camera) -> Option<ClipPlane> {
        let (centre, axis) = self.shared.borrow().section?;
        let toward = -camera.forward().as_dvec3();
        section_plane(centre, axis, toward, camera.up().as_dvec3())
    }
}

struct Job {
    key: Key,
    mesh: Arc<TriMesh>,
    tracer: Arc<MarginTracer>,
    seed: DVec3,
    prep: Option<Arc<Prep>>,
}

struct Outcome {
    key: Key,
    prep: Arc<Prep>,
    mesh: Arc<MeshData>,
    surface: Arc<InnerSurface>,
}

/// One line for a failed computation.
pub fn describe(e: DesignError) -> String {
    match e {
        DesignError::MarginUndercut { depth } => {
            format!(
                "The margin is undercut by {} mm along this axis",
                crate::widgets::fmt_mm(depth, 3)
            )
        }
        e => {
            let text = e.to_string();
            let mut chars = text.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

fn run(job: Job) -> Result<Outcome, String> {
    let prep = match job.prep {
        Some(p) => p,
        None => {
            let region = prep_region(&job.tracer, &job.key.margin, job.seed)?;
            let centroid = region_centroid(&job.mesh, &region);
            Arc::new(Prep {
                tooth: job.key.tooth,
                margin: job.key.margin.clone(),
                region,
                centroid,
            })
        }
    };
    let surface = inner_surface(
        &job.mesh,
        &prep.region,
        &job.key.margin,
        job.key.axis,
        &job.key.params(),
    )
    .map_err(describe)?;
    Ok(Outcome {
        key: job.key,
        prep,
        mesh: Arc::new(MeshData::from_mesh(&surface.mesh)),
        surface: Arc::new(surface),
    })
}

/// What the step needs from the app for one frame.
pub struct StepFrame<'a> {
    pub design: &'a Design,
    pub material: &'a Material,
    pub objects: &'a [SceneObject],
    /// The Margin step, whose tracers find the preparation.
    pub margin: &'a mut MarginStep,
    /// Tooth being worked on, when the step is active.
    pub tooth: Option<Fdi>,
}

/// The app side of the Inner surface step: worker jobs, the shown surface and the section toggle.
pub struct InnerStep {
    shared: Rc<RefCell<Shared>>,
    /// What the surface should be computed for now.
    wanted: Option<Key>,
    /// What the shown surface was computed for.
    shown: Option<Key>,
    /// The last job for this key failed: do not retry it.
    failed: Option<Key>,
    running: bool,
    prep: Option<Arc<Prep>>,
    section: bool,
    tx: Sender<Result<Outcome, (Key, String)>>,
    rx: Receiver<Result<Outcome, (Key, String)>>,
    next_group: u64,
    /// The parameter being dragged and its edit group.
    drag: Option<(InnerParam, EditGroup)>,
    error: Option<String>,
    repaint: Option<egui::Context>,
}

impl InnerStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self {
            shared: Rc::default(),
            wanted: None,
            shown: None,
            failed: None,
            running: false,
            prep: None,
            section: false,
            tx,
            rx,
            next_group: 1,
            drag: None,
            error: None,
            repaint,
        }
    }

    /// Forgets the open case's surface, jobs and error; keeps the Section toggle.
    pub fn reset(&mut self) {
        let section = self.section;
        *self = Self::new(self.repaint.take());
        self.section = section;
    }

    pub fn tool(&self) -> Box<dyn Tool> {
        Box::new(InnerTool {
            shared: self.shared.clone(),
        })
    }

    /// A job is running or the shown surface is out of date.
    pub fn busy(&self) -> bool {
        self.running || (self.wanted.is_some() && self.wanted != self.shown && !self.stuck())
    }

    /// The up-to-date inner surface of `tooth`, once computed.
    pub fn surface(&self, tooth: Fdi) -> Option<Arc<InnerSurface>> {
        let current = self.shown.as_ref().is_some_and(|k| k.tooth == tooth);
        if !current || self.wanted != self.shown {
            return None;
        }
        self.shared.borrow().surface.clone()
    }

    /// The preparation the up-to-date inner surface of `tooth` was computed on.
    pub(crate) fn prep(&self, tooth: Fdi) -> Option<Arc<Prep>> {
        self.surface(tooth)?;
        self.prep.clone().filter(|p| p.tooth == tooth)
    }

    /// The wanted surface failed and waits for a change.
    fn stuck(&self) -> bool {
        self.wanted.is_some() && self.failed == self.wanted
    }

    pub fn set_section(&mut self, on: bool) {
        self.section = on;
    }

    pub fn toggle_section(&mut self) {
        self.section = !self.section;
    }

    /// What the step panel shows for `tooth`.
    pub fn card(&self, design: &Design, material: &Material, tooth: Fdi) -> InnerCard {
        let stored = design.teeth.get(&tooth);
        let params = resolve(
            material,
            &stored.map(|t| t.inner_surface).unwrap_or_default(),
        );
        let current = self.wanted.as_ref().is_some_and(|k| k.tooth == tooth);
        InnerCard {
            tooth,
            has_margin: stored.is_some_and(|t| t.margin.is_some()),
            has_axis: stored.is_some_and(|t| t.insertion_axis.is_some()),
            values: InnerParam::ALL.map(|p| p.get(&params)),
            overridden: stored.is_some_and(|t| !t.inner_surface.is_empty()),
            section: self.section,
            busy: (current && self.busy()).then_some("Computing inner surface…"),
            error: self.error.clone().filter(|_| current),
        }
    }

    /// Sets `param` of `tooth`; `dragging` merges the changes of one drag into one undo step.
    #[allow(clippy::too_many_arguments)]
    pub fn set_param(
        &mut self,
        design: &mut Design,
        history: &mut History,
        material: &Material,
        tooth: Fdi,
        param: InnerParam,
        value: f64,
        dragging: bool,
    ) {
        let group = if dragging {
            match self.drag {
                Some((p, g)) if p == param => Some(g),
                _ => {
                    self.next_group += 1;
                    let g = EditGroup(self.next_group);
                    self.drag = Some((param, g));
                    Some(g)
                }
            }
        } else {
            self.end_drag(history);
            None
        };
        set_param(design, history, material, tooth, param, value, group);
    }

    /// A parameter drag ended.
    pub fn end_drag(&mut self, history: &mut History) {
        if self.drag.take().is_some() {
            history.end_group();
        }
    }

    /// Runs one frame: follows the stored margin, axis and parameters, starts and finishes jobs.
    pub fn update(&mut self, f: StepFrame<'_>) {
        while let Ok(message) = self.rx.try_recv() {
            self.running = false;
            match message {
                Ok(o) => {
                    self.prep = Some(o.prep);
                    if Some(&o.key) == self.wanted.as_ref() {
                        self.error = None;
                        let mut shared = self.shared.borrow_mut();
                        shared.mesh = Some(o.mesh);
                        shared.surface = Some(o.surface);
                        drop(shared);
                        self.shown = Some(o.key);
                    }
                }
                Err((key, e)) => {
                    if Some(&key) == self.wanted.as_ref() {
                        self.error = Some(e);
                        self.failed = Some(key);
                    }
                }
            }
        }
        let wanted = f.tooth.and_then(|t| Key::of(f.design, f.material, t));
        if wanted.as_ref().map(|k| k.tooth) != self.shown.as_ref().map(|k| k.tooth) {
            // Another tooth (or none): drop the other tooth's surface now.
            self.shown = None;
            let mut shared = self.shared.borrow_mut();
            shared.mesh = None;
            shared.surface = None;
        }
        if wanted != self.wanted {
            self.error = None;
            self.wanted = wanted;
        }
        if self
            .prep
            .as_ref()
            .zip(self.wanted.as_ref())
            .is_some_and(|(p, k)| p.tooth != k.tooth || p.margin != k.margin)
        {
            self.prep = None;
        }
        self.shared.borrow_mut().section = self
            .section
            .then(|| self.prep.as_ref().zip(self.wanted.as_ref()))
            .flatten()
            .map(|(p, k)| (p.centroid, k.axis));
        let Some(key) = self.wanted.clone() else {
            return;
        };
        if self.running || self.shown.as_ref() == Some(&key) || self.stuck() {
            return;
        }
        let Some(scan) = arch_scan(f.objects, key.tooth) else {
            self.error = Some("No jaw scan for this tooth".to_owned());
            self.failed = Some(key);
            return;
        };
        let tracer = match f.margin.tracer(scan, f.objects) {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => {
                self.error = Some(format!("Cannot use this scan: {e}"));
                self.failed = Some(key);
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
        let Some(seed) = region_seed(&key.margin, key.axis, &loaded.bvh, &loaded.mesh) else {
            self.error = Some("Cannot find the preparation inside the margin".to_owned());
            self.failed = Some(key);
            return;
        };
        let job = Job {
            key,
            mesh: tracer.mesh().clone(),
            tracer,
            seed,
            prep: self.prep.clone(),
        };
        self.running = true;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let key = job.key.clone();
            let _ = tx.send(run(job).map_err(|e| (key, e)));
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn design_with(tooth: Fdi) -> Design {
        let mut design = Design::default();
        let d = design.teeth.entry(tooth).or_default();
        d.margin = Some(vec![DVec3::X, DVec3::Y, -DVec3::X]);
        d.insertion_axis = Some(DVec3::Z);
        design
    }

    #[test]
    fn parameters_are_the_material_overridden_by_the_tooth() {
        let material = Material::zirconia();
        let base = resolve(&material, &InnerSurfaceOverrides::default());
        assert_eq!(base.cement_gap, material.cement_gap);
        assert_eq!(base.margin_seal_width, material.margin_seal_width);
        assert_eq!(base.seal_gap, InnerSurfaceParams::default().seal_gap);
        assert_eq!(
            base.transition_width,
            InnerSurfaceParams::default().transition_width
        );
        assert_eq!(base.grid_step, InnerSurfaceParams::default().grid_step);
        let over = InnerSurfaceOverrides {
            cement_gap: Some(0.07),
            seal_gap: Some(0.01),
            seal_width: None,
            transition_width: Some(1.2),
        };
        let p = resolve(&material, &over);
        assert_eq!(
            InnerParam::ALL.map(|q| q.get(&p)),
            [0.07, 0.01, material.margin_seal_width, 1.2]
        );
    }

    #[test]
    fn a_parameter_edit_stores_an_override_and_undoes() {
        let tooth = t(36);
        let material = Material::zirconia();
        let mut design = design_with(tooth);
        let mut history = History::default();
        let mut step = InnerStep::new(None);
        step.set_param(
            &mut design,
            &mut history,
            &material,
            tooth,
            InnerParam::CementGap,
            0.04,
            false,
        );
        assert_eq!(history.undo_label(), Some("Set cement gap 0.040 mm"));
        assert_eq!(design.teeth[&tooth].inner_surface.cement_gap, Some(0.04));
        assert_eq!(material.cement_gap, 0.03, "the material is untouched");
        // A drag is one undo step, named for its last value.
        for v in [0.81, 0.86, 0.9] {
            step.set_param(
                &mut design,
                &mut history,
                &material,
                tooth,
                InnerParam::SealWidth,
                v,
                true,
            );
        }
        step.end_drag(&mut history);
        assert_eq!(history.undo_label(), Some("Set seal width 0.90 mm"));
        assert_eq!(
            history.undo(&mut design).as_deref(),
            Some("Set seal width 0.90 mm")
        );
        assert_eq!(design.teeth[&tooth].inner_surface.seal_width, None);
        // Setting the material's own value removes the override.
        set_param(
            &mut design,
            &mut history,
            &material,
            tooth,
            InnerParam::CementGap,
            material.cement_gap,
            None,
        );
        assert!(design.teeth[&tooth].inner_surface.is_empty());
        history.undo(&mut design);
        reset_params(&mut design, &mut history, tooth);
        assert_eq!(history.undo_label(), Some(RESET_LABEL));
        assert!(design.teeth[&tooth].inner_surface.is_empty());
        history.undo(&mut design);
        assert_eq!(design.teeth[&tooth].inner_surface.cement_gap, Some(0.04));
        // Values are rounded to the shown decimals and clamped to the range.
        set_param(
            &mut design,
            &mut history,
            &material,
            tooth,
            InnerParam::TransitionWidth,
            9.0,
            None,
        );
        assert_eq!(history.undo_label(), Some("Set transition width 2.00 mm"));
    }

    #[test]
    fn the_section_plane_contains_the_axis_and_faces_the_camera() {
        let centre = DVec3::new(1.0, 2.0, 3.0);
        let axis = DVec3::new(0.0, 0.3, 1.0).normalize();
        let toward = DVec3::new(0.2, -1.0, 0.4).normalize();
        let plane = section_plane(centre, axis, toward, DVec3::Z).unwrap();
        let n = plane.normal.as_dvec3();
        assert!((n.length() - 1.0).abs() < 1e-6);
        assert!(n.dot(axis).abs() < 1e-6, "the axis lies in the plane");
        assert!(n.dot(toward) > 0.5, "the cut-away side faces the camera");
        assert!(plane.point.as_dvec3().distance(centre) < 1e-6);
        // Looking straight down the axis: the camera's up stands in.
        let plane = section_plane(centre, axis, axis, DVec3::X).unwrap();
        let n = plane.normal.as_dvec3();
        assert!(n.dot(axis).abs() < 1e-6 && n.dot(DVec3::X) < -0.99);
        assert!(section_plane(centre, DVec3::ZERO, toward, DVec3::Z).is_none());
    }

    #[test]
    fn the_surface_is_recomputed_when_margin_axis_or_parameters_change() {
        let tooth = t(36);
        let material = Material::zirconia();
        let mut design = design_with(tooth);
        let key = Key::of(&design, &material, tooth).unwrap();
        let mut step = InnerStep::new(None);
        let mut margin = MarginStep::new(None);
        let frame = |d: &Design, m: &mut MarginStep, s: &mut InnerStep| {
            s.update(StepFrame {
                design: d,
                material: &material,
                objects: &[],
                margin: m,
                tooth: Some(tooth),
            });
        };
        frame(&design, &mut margin, &mut step);
        assert_eq!(step.wanted.as_ref(), Some(&key));
        // No scan to compute on: the key fails once and is not retried.
        assert!(step.error.is_some() && !step.busy());
        // Pretend the surface was computed for this key.
        step.failed = None;
        step.error = None;
        step.shown = Some(key.clone());
        frame(&design, &mut margin, &mut step);
        assert!(!step.busy(), "nothing changed");
        // Another tooth's edit changes nothing.
        design.teeth.entry(t(37)).or_default().insertion_axis = Some(DVec3::X);
        frame(&design, &mut margin, &mut step);
        assert!(!step.busy() && step.error.is_none());
        let edits: [fn(&mut ToothDesign); 3] = [
            |d| d.margin.as_mut().unwrap().push(DVec3::NEG_Y),
            |d| d.insertion_axis = Some(DVec3::new(0.0, 0.1, 1.0)),
            |d| d.inner_surface.cement_gap = Some(0.08),
        ];
        for edit in edits {
            let mut changed = design.clone();
            edit(changed.teeth.get_mut(&tooth).unwrap());
            frame(&changed, &mut margin, &mut step);
            assert_ne!(step.wanted.as_ref(), Some(&key));
            // No scan: the new key is tried (and fails) instead of keeping the old surface.
            assert_eq!(step.failed, step.wanted);
            step.failed = None;
            step.shown = Some(key.clone());
            frame(&design, &mut margin, &mut step);
            assert!(!step.busy());
        }
        // No margin or no axis: nothing to compute.
        design.teeth.get_mut(&tooth).unwrap().insertion_axis = None;
        frame(&design, &mut margin, &mut step);
        assert!(step.wanted.is_none() && !step.busy());
    }
}
