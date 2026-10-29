//! The Anatomy step: a library crown placed over the tooth's inner surface and thickened to the
//! material's minimums. The user scales and wears its shape, turns it about the insertion axis and
//! nudges it; every change is an undoable edit of the tooth's [`AnatomyDesign`]. The crown is
//! built on a worker whenever the inner surface or the anatomy changes (latest request wins) and
//! drawn as its own opaque mesh; the inner surface is hidden in this step.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_anatomy::{CrownParams as ShapeParams, crown};
use dentcraft_case::{
    AnatomyDesign, Arch, Case, Design, Fdi, JawFrame, Material, RestorationKind, ToothDesign,
    ToothType,
};
use dentcraft_design::{
    Crown, CrownParams as BuildParams, CrownPlacement, CrownReport, InnerSurface, auto_placement,
    build_crown,
};
use dentcraft_render::{DrawItem, MeshData, MeshId};
use eframe::egui;
use glam::DVec3;

use crate::history::{EditGroup, History};
use crate::inner_step::describe;
use crate::theme::Palette;
use crate::tools::{Tool, rgb};
use crate::workflow_panel::AnatomyCard;

/// Name of the anatomy tool.
pub const ANATOMY: &str = "Anatomy";
/// Undo label of "Reset".
pub const RESET_LABEL: &str = "Reset crown anatomy";
/// Distance one offset nudge moves the crown, in millimetres.
pub const NUDGE_MM: f64 = 0.1;
/// Renderer id of the crown mesh, clear of the scans' and the inner surface's ids.
const MESH: MeshId = MeshId(u64::MAX - 2);
/// First edit group of this step's drags, clear of the other steps' groups.
const FIRST_GROUP: u64 = 1 << 40;

/// One slider of the Anatomy step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnatomyParam {
    Width,
    Depth,
    Height,
    CuspHeight,
    Wear,
    Rotation,
}

impl AnatomyParam {
    pub const ALL: [Self; 6] = [
        Self::Width,
        Self::Depth,
        Self::Height,
        Self::CuspHeight,
        Self::Wear,
        Self::Rotation,
    ];

    /// Panel label.
    pub fn name(self) -> &'static str {
        match self {
            Self::Width => "Width (MD)",
            Self::Depth => "Depth (BL)",
            Self::Height => "Height",
            Self::CuspHeight => "Cusp height",
            Self::Wear => "Wear",
            Self::Rotation => "Rotation",
        }
    }

    /// Unit shown after the value.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Rotation => "°",
            _ => " %",
        }
    }

    /// Values the panel accepts, in shown units (percent or degrees).
    pub fn range(self) -> RangeInclusive<f64> {
        match self {
            Self::Width | Self::Depth | Self::Height => 50.0..=200.0,
            Self::CuspHeight => 30.0..=200.0,
            Self::Wear => 0.0..=100.0,
            Self::Rotation => -90.0..=90.0,
        }
    }

    /// Shown units per stored unit: factors show as percent, the rotation as degrees.
    fn scale(self) -> f64 {
        match self {
            Self::Rotation => 1.0,
            _ => 100.0,
        }
    }

    fn slot(self, a: &mut AnatomyDesign) -> &mut f64 {
        match self {
            Self::Width => &mut a.mesiodistal,
            Self::Depth => &mut a.buccolingual,
            Self::Height => &mut a.height,
            Self::CuspHeight => &mut a.cusp_height,
            Self::Wear => &mut a.wear,
            Self::Rotation => &mut a.rotation_deg,
        }
    }

    /// The parameter's value in `a`, in shown units.
    pub fn get(self, a: &AnatomyDesign) -> f64 {
        *self.slot(&mut a.clone()) * self.scale()
    }

    /// `value` (shown units) rounded to whole units and clamped to the range.
    fn round(self, value: f64) -> f64 {
        let r = self.range();
        value.round().clamp(*r.start(), *r.end())
    }
}

/// Undo label of setting `param` to `value` in shown units ("Set crown width 105 %").
pub fn edit_label(param: AnatomyParam, value: f64) -> String {
    let what = match param {
        AnatomyParam::Width => "crown width",
        AnatomyParam::Depth => "crown depth",
        AnatomyParam::Height => "crown height",
        AnatomyParam::CuspHeight => "cusp height",
        AnatomyParam::Wear => "wear",
        AnatomyParam::Rotation => {
            return format!("Rotate crown {}°", crate::widgets::fmt_mm(value, 0));
        }
    };
    format!("Set {what} {value:.0} %")
}

/// The library template's shape factors for an anatomy design.
pub fn shape_params(a: &AnatomyDesign) -> ShapeParams {
    ShapeParams {
        mesiodistal: a.mesiodistal,
        buccolingual: a.buccolingual,
        height: a.height,
        wear: a.wear,
        cusp_height: a.cusp_height,
    }
}

/// +1 where the template's +X is the tooth's mesial direction (quadrants 1 and 3), -1 where the
/// template is mirrored and its +X is distal (quadrants 2 and 4).
fn template_sign(tooth: Fdi) -> f64 {
    if tooth.quadrant() % 2 == 0 { -1.0 } else { 1.0 }
}

/// The direction the template's +X should take (the `mesial` of a [`CrownPlacement`]), from the
/// jaw's forward direction and the tooth's insertion axis.
///
/// Up (towards the skull) is the axis for a lower tooth and its reverse for an upper one, and
/// the patient's right is `forward × up`. The tooth's mesial direction is towards the midline
/// (left for a right-side tooth, right for a left-side one) for incisors, forward for premolars
/// and molars, and halfway between for canines, which sit at the corner of the arch. Teeth of
/// quadrants 2 and 4 use mirrored templates, so their template +X is the distal direction.
/// `None` when the directions are degenerate (forward along the axis).
pub fn template_x_hint(tooth: Fdi, axis: DVec3, forward: DVec3) -> Option<DVec3> {
    let forward = forward.try_normalize()?;
    let up = if tooth.arch() == Arch::Upper {
        -axis
    } else {
        axis
    };
    let right = forward.cross(up).try_normalize()?;
    let right_side = matches!(tooth.quadrant(), 1 | 4 | 5 | 8);
    let midline = if right_side { -right } else { right };
    let mesial = match tooth.tooth_type() {
        ToothType::Incisor => midline,
        ToothType::Canine => midline + forward,
        ToothType::Premolar | ToothType::Molar => forward,
    };
    (mesial * template_sign(tooth)).try_normalize()
}

/// `v` turned by `angle` radians about the unit `axis` (right-handed).
fn rotate(v: DVec3, axis: DVec3, angle: f64) -> DVec3 {
    let (s, c) = angle.sin_cos();
    v * c + axis.cross(v) * s + axis * axis.dot(v) * (1.0 - c)
}

/// The user's rotation and offset applied to an automatic placement. The crown turns by
/// `rotation_deg` about the placement's occlusal axis through its origin (counter-clockwise seen
/// from the occlusal side), then moves by `offset` along the tooth's mesial, facial and occlusal
/// directions. The default anatomy returns `p` unchanged.
pub fn adjust(p: CrownPlacement, tooth: Fdi, a: &AnatomyDesign) -> CrownPlacement {
    let z = p.occlusal.normalize_or_zero();
    let mesial = rotate(p.mesial, z, a.rotation_deg.to_radians());
    let x = (mesial - z * mesial.dot(z)).normalize_or_zero();
    let [m, f, o] = a.offset;
    let origin = p.origin + x * (m * template_sign(tooth)) + z.cross(x) * f + z * o;
    CrownPlacement {
        origin,
        mesial,
        ..p
    }
}

/// Drops `tooth`'s entry when nothing is left in it.
fn prune(design: &mut Design, tooth: Fdi) {
    if design
        .teeth
        .get(&tooth)
        .is_some_and(|t| *t == Default::default())
    {
        design.teeth.remove(&tooth);
    }
}

/// Sets `param` of `tooth` to `value` (shown units, rounded and clamped) and records the edit.
/// Edits in the same open `group` (a drag) merge.
pub fn set_param(
    design: &mut Design,
    history: &mut History,
    tooth: Fdi,
    param: AnatomyParam,
    value: f64,
    group: Option<EditGroup>,
) {
    let value = param.round(value);
    let before = design.clone();
    *param.slot(&mut design.teeth.entry(tooth).or_default().anatomy) = value / param.scale();
    prune(design, tooth);
    history.record(&edit_label(param, value), before, design, group);
}

/// Undo label of moving the crown by `delta` mm along offset axis `axis` (0 mesial, 1 facial,
/// 2 occlusal).
pub fn nudge_label(axis: usize, delta: f64) -> String {
    let names = [
        ["mesially", "distally"],
        ["facially", "lingually"],
        ["occlusally", "cervically"],
    ];
    let name = names[axis.min(2)][usize::from(delta < 0.0)];
    format!("Move crown {name} {:.1} mm", delta.abs())
}

/// Moves the crown of `tooth` by `delta` mm along offset axis `axis` and records the edit.
pub fn nudge(design: &mut Design, history: &mut History, tooth: Fdi, axis: usize, delta: f64) {
    let before = design.clone();
    let offset = &mut design.teeth.entry(tooth).or_default().anatomy.offset[axis.min(2)];
    *offset = ((*offset + delta) * 1000.0).round() / 1000.0;
    prune(design, tooth);
    history.record(&nudge_label(axis, delta), before, design, None);
}

/// Puts the crown of `tooth` back to the library shape and the automatic placement.
pub fn reset(design: &mut Design, history: &mut History, tooth: Fdi) {
    let before = design.clone();
    if let Some(t) = design.teeth.get_mut(&tooth) {
        t.anatomy = AnatomyDesign::default();
    }
    prune(design, tooth);
    history.record(RESET_LABEL, before, design, None);
}

/// Everything the crown depends on.
#[derive(Clone)]
struct Key {
    tooth: Fdi,
    surface: Arc<InnerSurface>,
    anatomy: AnatomyDesign,
    forward: Option<DVec3>,
    min_occlusal: f64,
    min_wall: f64,
}

impl PartialEq for Key {
    fn eq(&self, o: &Self) -> bool {
        self.tooth == o.tooth
            && Arc::ptr_eq(&self.surface, &o.surface)
            && self.anatomy == o.anatomy
            && self.forward == o.forward
            && self.min_occlusal == o.min_occlusal
            && self.min_wall == o.min_wall
    }
}

/// What a crown was built from in the case: the tooth's design, the material and the jaw frame.
/// A crown whose stamp no longer matches the case is out of date.
#[derive(Clone, Debug, PartialEq)]
struct Stamp {
    tooth: Option<ToothDesign>,
    material: Material,
    jaw: Option<JawFrame>,
}

impl Stamp {
    fn of(case: &Case, tooth: Fdi) -> Self {
        Self {
            tooth: case.design.teeth.get(&tooth).cloned(),
            material: case.material.clone(),
            jaw: case.design.jaw_frame,
        }
    }
}

struct Outcome {
    key: Key,
    stamp: Stamp,
    mesh: Arc<MeshData>,
    crown: Arc<Crown>,
}

fn run(key: Key, stamp: Stamp) -> Result<Outcome, String> {
    let template = crown(key.tooth, &shape_params(&key.anatomy));
    let inner = &*key.surface;
    let hint = key
        .forward
        .and_then(|f| template_x_hint(key.tooth, inner.axis, f));
    let placement = auto_placement(inner, &template.mesh, &template.cervical, inner.axis, hint);
    let placement = adjust(placement, key.tooth, &key.anatomy);
    let built = build_crown(
        inner,
        &template.mesh,
        &template.cervical,
        placement,
        &BuildParams::new(key.min_occlusal, key.min_wall),
    )
    .map_err(describe)?;
    Ok(Outcome {
        mesh: Arc::new(MeshData::from_mesh(&built.mesh)),
        crown: Arc::new(built),
        stamp,
        key,
    })
}

/// The viewport tool of the Anatomy step: draws the crown.
pub struct AnatomyTool {
    mesh: Rc<RefCell<Option<Arc<MeshData>>>>,
}

impl Tool for AnatomyTool {
    fn name(&self) -> &'static str {
        ANATOMY
    }

    fn picks_surface(&self) -> bool {
        false
    }

    fn meshes(&self, palette: &Palette) -> Vec<(DrawItem, Arc<MeshData>)> {
        let item = DrawItem {
            mesh: MESH,
            color: rgb(palette.crown),
            opacity: 1.0,
            visible: true,
            vertex_colors: false,
            clip: false,
        };
        self.mesh
            .borrow()
            .clone()
            .map(|m| (item, m))
            .into_iter()
            .collect()
    }
}

/// What the step needs from the app for one frame.
pub struct StepFrame<'a> {
    pub case: &'a Case,
    /// Tooth being worked on, when the step is active.
    pub tooth: Option<Fdi>,
    /// The tooth's current inner surface, once computed.
    pub surface: Option<Arc<InnerSurface>>,
}

/// The app side of the Anatomy step: worker jobs, the shown crown and its report.
pub struct AnatomyStep {
    mesh: Rc<RefCell<Option<Arc<MeshData>>>>,
    wanted: Option<Key>,
    shown: Option<Key>,
    /// The last job for this key failed: do not retry it.
    failed: Option<Key>,
    running: bool,
    report: Option<CrownReport>,
    tx: Sender<Result<Outcome, (Key, String)>>,
    rx: Receiver<Result<Outcome, (Key, String)>>,
    next_group: u64,
    /// The parameter being dragged and its edit group.
    drag: Option<(AnatomyParam, EditGroup)>,
    error: Option<String>,
    repaint: Option<egui::Context>,
    /// The latest crown built for each tooth and what it was built from, for the export.
    built: BTreeMap<Fdi, (Stamp, Arc<Crown>)>,
    /// Teeth whose first crown was built, so the view has framed it once.
    framed: BTreeSet<Fdi>,
    /// A tooth's first crown was built since the app last asked: the view should take it in.
    new_crown: bool,
}

impl AnatomyStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self {
            mesh: Rc::default(),
            wanted: None,
            shown: None,
            failed: None,
            running: false,
            report: None,
            tx,
            rx,
            next_group: FIRST_GROUP,
            drag: None,
            error: None,
            repaint,
            built: BTreeMap::new(),
            framed: BTreeSet::new(),
            new_crown: false,
        }
    }

    /// Whether a tooth's first crown was built since the last call, so the view can take it in.
    pub fn take_new_crown(&mut self) -> bool {
        std::mem::take(&mut self.new_crown)
    }

    /// The latest crown built for `tooth`, while the case still matches what it was built from.
    pub fn crown(&self, case: &Case, tooth: Fdi) -> Option<Arc<Crown>> {
        let (stamp, crown) = self.built.get(&tooth)?;
        (*stamp == Stamp::of(case, tooth)).then(|| crown.clone())
    }

    /// Forgets the open case's crown, jobs and error.
    pub fn reset(&mut self) {
        *self = Self::new(self.repaint.take());
    }

    pub fn tool(&self) -> Box<dyn Tool> {
        Box::new(AnatomyTool {
            mesh: self.mesh.clone(),
        })
    }

    /// A job is running or the shown crown is out of date.
    pub fn busy(&self) -> bool {
        self.running || (self.wanted.is_some() && self.wanted != self.shown && !self.stuck())
    }

    fn stuck(&self) -> bool {
        self.wanted.is_some() && self.failed == self.wanted
    }

    /// What the step panel shows for `tooth`. `inner_busy` and `inner_error` are the Inner
    /// surface step's state for the same tooth.
    pub fn card(
        &self,
        case: &Case,
        tooth: Fdi,
        inner_busy: bool,
        inner_error: Option<String>,
    ) -> AnatomyCard {
        let stored = case.design.teeth.get(&tooth);
        let anatomy = stored.map(|t| t.anatomy).unwrap_or_default();
        let current = self.wanted.as_ref().is_some_and(|k| k.tooth == tooth);
        let shown = self.shown.as_ref().is_some_and(|k| k.tooth == tooth);
        let busy = if inner_busy {
            Some("Computing inner surface…")
        } else if current && self.busy() {
            Some("Building crown…")
        } else {
            None
        };
        AnatomyCard {
            tooth,
            pontic: case
                .units
                .iter()
                .any(|u| u.tooth == tooth && u.kind == RestorationKind::Pontic),
            has_margin: stored.is_some_and(|t| t.margin.is_some()),
            has_axis: stored.is_some_and(|t| t.insertion_axis.is_some()),
            inner_failed: inner_error.is_some(),
            values: AnatomyParam::ALL.map(|p| p.get(&anatomy)),
            offset: anatomy.offset,
            adjusted: !anatomy.is_default(),
            report: self
                .report
                .filter(|_| shown)
                .map(|r| (r.min_thickness, r.thickened_area)),
            busy,
            error: inner_error.or_else(|| self.error.clone().filter(|_| current)),
        }
    }

    /// Sets `param` of `tooth`; `dragging` merges the changes of one drag into one undo step.
    pub fn set_param(
        &mut self,
        design: &mut Design,
        history: &mut History,
        tooth: Fdi,
        param: AnatomyParam,
        value: f64,
        dragging: bool,
    ) {
        let group = if dragging {
            match self.drag {
                Some((p, g)) if p == param => Some(g),
                _ => {
                    self.end_drag(history);
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
        set_param(design, history, tooth, param, value, group);
    }

    /// A slider drag ended.
    pub fn end_drag(&mut self, history: &mut History) {
        if self.drag.take().is_some() {
            history.end_group();
        }
    }

    /// Runs one frame: follows the inner surface and the anatomy, starts and finishes jobs.
    pub fn update(&mut self, f: StepFrame<'_>) {
        while let Ok(message) = self.rx.try_recv() {
            self.running = false;
            match message {
                Ok(o) => {
                    // Kept even when no longer wanted: the stamp tells the export if it is current.
                    self.built.insert(o.key.tooth, (o.stamp, o.crown.clone()));
                    if Some(&o.key) == self.wanted.as_ref() {
                        self.new_crown |= self.framed.insert(o.key.tooth);
                        self.error = None;
                        *self.mesh.borrow_mut() = Some(o.mesh);
                        self.report = Some(o.crown.report);
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
        let design = &f.case.design;
        let material = &f.case.material;
        let wanted = f.tooth.zip(f.surface).map(|(tooth, surface)| Key {
            tooth,
            surface,
            anatomy: design
                .teeth
                .get(&tooth)
                .map(|t| t.anatomy)
                .unwrap_or_default(),
            forward: design.jaw_frame.map(|j| DVec3::from_array(j.forward)),
            min_occlusal: material.min_occlusal_thickness,
            min_wall: material.min_wall_thickness,
        });
        if wanted.as_ref().map(|k| k.tooth) != self.shown.as_ref().map(|k| k.tooth) {
            // Another tooth (or no surface): drop the other crown now.
            self.shown = None;
            self.report = None;
            *self.mesh.borrow_mut() = None;
        }
        if wanted != self.wanted {
            self.error = None;
            self.wanted = wanted;
        }
        let Some(key) = self.wanted.clone() else {
            return;
        };
        if self.running || self.shown.as_ref() == Some(&key) || self.stuck() {
            return;
        }
        self.running = true;
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        // The surface in the key was computed from the case as it is this frame.
        let stamp = Stamp::of(f.case, key.tooth);
        std::thread::spawn(move || {
            let k = key.clone();
            let _ = tx.send(run(key, stamp).map_err(|e| (k, e)));
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

    fn close(a: DVec3, b: DVec3) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn a_built_crown_is_offered_only_while_the_case_matches_it() {
        let mut case = Case::new("x");
        let tooth = t(36);
        let crown = Arc::new(Crown {
            mesh: dentcraft_mesh::TriMesh::default(),
            margin_vertices: vec![],
            report: CrownReport {
                min_thickness: 0.5,
                thickened_area: 0.0,
                max_thickening: 0.0,
            },
        });
        let mut step = AnatomyStep::new(None);
        step.built
            .insert(tooth, (Stamp::of(&case, tooth), crown.clone()));
        assert!(step.crown(&case, tooth).is_some());
        assert!(step.crown(&case, t(46)).is_none());
        case.design
            .teeth
            .entry(tooth)
            .or_default()
            .anatomy
            .mesiodistal = 1.05;
        assert!(step.crown(&case, tooth).is_none(), "anatomy changed");
        step.built.insert(tooth, (Stamp::of(&case, tooth), crown));
        assert!(step.crown(&case, tooth).is_some());
        case.material.min_wall_thickness += 0.1;
        assert!(step.crown(&case, tooth).is_none(), "material changed");
    }

    #[test]
    fn the_design_maps_onto_the_template_parameters() {
        assert_eq!(
            shape_params(&AnatomyDesign::default()),
            ShapeParams::default()
        );
        let a = AnatomyDesign {
            mesiodistal: 1.05,
            buccolingual: 0.9,
            height: 1.1,
            wear: 0.3,
            cusp_height: 0.8,
            rotation_deg: 12.0,
            offset: [0.1, 0.2, 0.3],
        };
        assert_eq!(
            shape_params(&a),
            ShapeParams {
                mesiodistal: 1.05,
                buccolingual: 0.9,
                height: 1.1,
                wear: 0.3,
                cusp_height: 0.8,
            }
        );
        // Shown values: factors as percent, the rotation in degrees.
        assert_eq!(
            AnatomyParam::ALL.map(|p| (p.get(&a) * 1e6).round() / 1e6),
            [105.0, 90.0, 110.0, 80.0, 30.0, 12.0]
        );
    }

    #[test]
    fn rotation_and_offset_move_the_placement() {
        let p = CrownPlacement {
            origin: DVec3::new(1.0, 2.0, 3.0),
            occlusal: DVec3::Z,
            mesial: DVec3::X,
            scale: DVec3::new(1.1, 0.9, 1.2),
        };
        let same = adjust(p, t(36), &AnatomyDesign::default());
        assert_eq!(
            (same.origin, same.mesial, same.occlusal, same.scale),
            (p.origin, p.mesial, p.occlusal, p.scale)
        );
        let a = AnatomyDesign {
            rotation_deg: 90.0,
            offset: [0.5, 0.25, -1.0],
            ..AnatomyDesign::default()
        };
        // Quadrant 3: template +X is mesial. Turned 90° counter-clockwise from above, mesial is
        // +Y and facial (occlusal × mesial) is -X.
        let q = adjust(p, t(36), &a);
        assert!(close(q.mesial, DVec3::Y), "{:?}", q.mesial);
        assert_eq!((q.occlusal, q.scale), (p.occlusal, p.scale));
        let expected = p.origin + DVec3::Y * 0.5 - DVec3::X * 0.25 - DVec3::Z;
        assert!(close(q.origin, expected), "{:?}", q.origin);
        // Quadrant 4: the template is mirrored, its +X is distal, so mesial is the other way.
        let q = adjust(p, t(46), &a);
        let expected = p.origin - DVec3::Y * 0.5 - DVec3::X * 0.25 - DVec3::Z;
        assert!(close(q.origin, expected), "{:?}", q.origin);
    }

    #[test]
    fn the_mesial_hint_follows_the_quadrant_and_the_jaw_frame() {
        // Patient faces +Y with the skull at +Z, so the patient's right is +X. The hint is the
        // template's +X; occlusal × hint must be the tooth's facial direction.
        let forward = DVec3::Y;
        let cases = [
            // (tooth, insertion axis, facial)
            (36, DVec3::Z, -DVec3::X),
            (46, DVec3::Z, DVec3::X),
            (16, -DVec3::Z, DVec3::X),
            (26, -DVec3::Z, -DVec3::X),
            (11, -DVec3::Z, DVec3::Y),
            (21, -DVec3::Z, DVec3::Y),
            (31, DVec3::Z, DVec3::Y),
            (41, DVec3::Z, DVec3::Y),
        ];
        for (code, axis, facial) in cases {
            let hint = template_x_hint(t(code), axis, forward).unwrap();
            assert!(close(axis.cross(hint), facial), "{code}: {hint:?}");
        }
        // Posteriors point forward (mesially) in quadrants 1 and 3.
        assert!(close(
            template_x_hint(t(36), DVec3::Z, forward).unwrap(),
            DVec3::Y
        ));
        // A canine sits at the corner: halfway between forward and the midline.
        let h = template_x_hint(t(33), DVec3::Z, forward).unwrap();
        assert!(close(h, DVec3::new(1.0, 1.0, 0.0).normalize()), "{h:?}");
        assert!(template_x_hint(t(36), DVec3::Z, DVec3::Z).is_none());
        assert!(template_x_hint(t(36), DVec3::Z, DVec3::ZERO).is_none());
    }

    #[test]
    fn a_slider_drag_is_one_undo_step() {
        let tooth = t(36);
        let mut design = Design::default();
        let mut history = History::default();
        let mut step = AnatomyStep::new(None);
        for v in [101.0, 103.4, 105.0] {
            step.set_param(
                &mut design,
                &mut history,
                tooth,
                AnatomyParam::Width,
                v,
                true,
            );
        }
        step.end_drag(&mut history);
        assert_eq!(history.undo_label(), Some("Set crown width 105 %"));
        assert_eq!(design.teeth[&tooth].anatomy.mesiodistal, 1.05);
        step.set_param(
            &mut design,
            &mut history,
            tooth,
            AnatomyParam::Rotation,
            5.2,
            false,
        );
        assert_eq!(history.undo_label(), Some("Rotate crown 5°"));
        assert_eq!(design.teeth[&tooth].anatomy.rotation_deg, 5.0);
        history.undo(&mut design);
        assert_eq!(
            history.undo(&mut design).as_deref(),
            Some("Set crown width 105 %")
        );
        assert!(
            design.teeth.is_empty(),
            "back to the identity: no entry left"
        );
        // Clamped to the range; a drag of another slider is its own step.
        step.set_param(
            &mut design,
            &mut history,
            tooth,
            AnatomyParam::CuspHeight,
            10.0,
            true,
        );
        step.set_param(
            &mut design,
            &mut history,
            tooth,
            AnatomyParam::Wear,
            40.0,
            true,
        );
        step.end_drag(&mut history);
        assert_eq!(history.undo_label(), Some("Set wear 40 %"));
        history.undo(&mut design);
        assert_eq!(history.undo_label(), Some("Set cusp height 30 %"));
        // Nudges and reset.
        nudge(&mut design, &mut history, tooth, 0, -NUDGE_MM);
        nudge(&mut design, &mut history, tooth, 0, -NUDGE_MM);
        nudge(&mut design, &mut history, tooth, 0, -NUDGE_MM);
        assert_eq!(history.undo_label(), Some("Move crown distally 0.1 mm"));
        assert_eq!(design.teeth[&tooth].anatomy.offset, [-0.3, 0.0, 0.0]);
        reset(&mut design, &mut history, tooth);
        assert_eq!(history.undo_label(), Some(RESET_LABEL));
        assert!(design.teeth.is_empty());
        history.undo(&mut design);
        assert_eq!(design.teeth[&tooth].anatomy.cusp_height, 0.3);
    }
}
