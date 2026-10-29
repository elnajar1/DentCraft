//! The Occlusion & contacts step: the tooth's crown adapted to its mesial and distal neighbours
//! and to the antagonist, coloured by its distance to them.
//!
//! The neighbours come from the tooth's own jaw scan: everything more than [`EXCLUDE_MM`] from
//! the preparation and within [`KEEP_MM`] of the crown, split by the plane through the
//! preparation's centroid across the crown's mesial direction. The antagonist is the opposing
//! jaw's scan, when one is loaded. The contact targets are an undoable part of the tooth's design
//! ([`ContactDesign`]); "Adapt" (run once automatically for each new crown from the Anatomy step)
//! deforms a copy of the Anatomy step's crown on a worker. The adapted crown is kept with what it
//! was made from: the Anatomy crown, the contact targets and the two scans. While all of them
//! still match, it replaces the Anatomy crown in this step and in the export
//! ([`ContactsStep::adapted`]); otherwise the Anatomy crown is used as is.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use dentcraft_case::{Arch, Case, ContactDesign, Design, Fdi, ScanRole};
use dentcraft_design::{
    AdaptParams, Crown, CrownParams as BuildParams, InnerSurface, adapt_to_neighbours,
    signed_distance_map,
};
use dentcraft_mesh::{Aabb, Bvh, TriMesh};
use dentcraft_render::{DrawItem, MeshData, MeshId, colormap};
use eframe::egui;
use glam::DVec3;

use crate::history::{EditGroup, History};
use crate::inner_step::{Prep, describe};
use crate::objects::SceneObject;
use crate::theme::Palette;
use crate::tools::{Tool, rgb};

/// What the last adaptation of a crown to its neighbours and antagonist reached, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactReport {
    pub mesial: Option<f64>,
    pub distal: Option<f64>,
    pub occlusal: Option<f64>,
    /// Area of the moved outer surface, mm².
    pub moved_area: f64,
    pub thickness_ok: bool,
}

/// What the step's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactsCard {
    pub tooth: Fdi,
    /// The Anatomy step has a current crown for the tooth.
    pub has_crown: bool,
    /// Values in [`ContactParam::ALL`] order, in millimetres.
    pub values: [f64; 2],
    /// The report of the current adapted crown.
    pub report: Option<ContactReport>,
    /// The tooth was adapted, but its crown, targets or scans changed since.
    pub outdated: bool,
    /// An opposing-jaw scan is loaded.
    pub antagonist: bool,
    /// What the step is busy with.
    pub busy: Option<&'static str>,
    pub error: Option<String>,
}

/// Name of the contacts tool.
pub const CONTACTS: &str = "Contacts";
/// Scan within this distance (mm) of the preparation is not a neighbour.
pub const EXCLUDE_MM: f64 = 1.5;
/// Scan farther than this (mm) outside the crown's bounds is ignored.
pub const KEEP_MM: f64 = 6.0;
/// Range of the distance colour map, in mm: penetration red, contact white, gap blue.
pub const MAP_MIN_MM: f64 = -0.1;
pub const MAP_MAX_MM: f64 = 0.3;
/// Shown when the adaptation stopped at the material's minimum thickness.
pub const THIN_WARNING: &str = "Occlusion needs a thinner crown than the material allows \u{2014} reduce the antagonist contact or adjust the prep";
/// Shown when there is no opposing-jaw scan.
pub const NO_ANTAGONIST: &str = "No opposing-jaw scan: occlusion is skipped";
/// Renderer id of the crown mesh, clear of the other steps' ids.
const MESH: MeshId = MeshId(u64::MAX - 3);
/// First edit group of this step's drags, clear of the other steps' groups.
const FIRST_GROUP: u64 = 1 << 41;

/// One parameter of the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactParam {
    Proximal,
    Occlusal,
}

impl ContactParam {
    pub const ALL: [Self; 2] = [Self::Proximal, Self::Occlusal];

    pub fn name(self) -> &'static str {
        match self {
            Self::Proximal => "Proximal contact",
            Self::Occlusal => "Occlusal clearance",
        }
    }

    pub fn range(self) -> std::ops::RangeInclusive<f64> {
        -0.2..=0.3
    }

    fn slot(self, c: &mut ContactDesign) -> &mut f64 {
        match self {
            Self::Proximal => &mut c.proximal_contact,
            Self::Occlusal => &mut c.occlusal_clearance,
        }
    }

    pub fn get(self, c: &ContactDesign) -> f64 {
        *self.slot(&mut c.clone())
    }
}

/// Undo label of setting `param` to `value` mm ("Set proximal contact −0.05 mm").
pub fn edit_label(param: ContactParam, value: f64) -> String {
    format!(
        "Set {} {} mm",
        param.name().to_lowercase(),
        crate::widgets::fmt_signed(value, 2)
    )
}

/// Sets `param` of `tooth` to `value` mm (rounded to 0.01 mm, clamped) and records the edit.
/// Edits in the same open `group` (a drag) merge.
pub fn set_param(
    design: &mut Design,
    history: &mut History,
    tooth: Fdi,
    param: ContactParam,
    value: f64,
    group: Option<EditGroup>,
) {
    let range = param.range();
    let value = ((value * 100.0).round() / 100.0).clamp(*range.start(), *range.end());
    let before = design.clone();
    *param.slot(&mut design.teeth.entry(tooth).or_default().contacts) = value;
    if design
        .teeth
        .get(&tooth)
        .is_some_and(|t| *t == Default::default())
    {
        design.teeth.remove(&tooth);
    }
    history.record(&edit_label(param, value), before, design, group);
}

/// Colour of a signed distance (mm): red at [`MAP_MIN_MM`] and below (penetration), white at
/// contact (0), blue at [`MAP_MAX_MM`] and above (gap). The two sides are scaled separately so
/// that white sits exactly at contact.
pub fn distance_color(d: f64) -> [f32; 4] {
    let t = if d.is_nan() {
        0.0
    } else if d < 0.0 {
        0.5 + 0.5 * (d / MAP_MIN_MM).min(1.0)
    } else {
        0.5 - 0.5 * (d / MAP_MAX_MM).min(1.0)
    };
    colormap::diverging(t as f32, 0.0, 1.0)
}

/// The triangles of `scan` whose centroids lie within `keep` mm of `bounds` and, with `exclude`,
/// at least `EXCLUDE_MM` from that surface, as compact meshes: the first holds the triangles
/// for which `side` is true. Zero-area triangles are dropped.
/// Positions and triangles of one part of a cropped scan.
type Part = (Vec<DVec3>, Vec<[u32; 3]>);

fn crop(
    scan: &TriMesh,
    bounds: &Aabb,
    keep: f64,
    exclude: Option<(&TriMesh, &Bvh)>,
    side: impl Fn(DVec3) -> bool,
) -> [Option<TriMesh>; 2] {
    let (lo, hi) = (
        bounds.min - DVec3::splat(keep),
        bounds.max + DVec3::splat(keep),
    );
    let mut parts: [Part; 2] = Default::default();
    let mut map = [vec![u32::MAX; scan.positions.len()], vec![]];
    map[1] = map[0].clone();
    for (i, tri) in scan.triangles.iter().enumerate() {
        let [a, b, c] = tri.map(|v| scan.positions[v as usize]);
        let centre = (a + b + c) / 3.0;
        if centre.cmplt(lo).any()
            || centre.cmpgt(hi).any()
            || scan.triangle_cross(i).length_squared() < 1e-24
        {
            continue;
        }
        if let Some((mesh, bvh)) = exclude
            && bvh
                .closest_point(mesh, centre)
                .is_some_and(|h| h.distance < EXCLUDE_MM)
        {
            continue;
        }
        let k = usize::from(!side(centre));
        let (positions, triangles) = &mut parts[k];
        let tri = tri.map(|v| {
            let slot = &mut map[k][v as usize];
            if *slot == u32::MAX {
                *slot = positions.len() as u32;
                positions.push(scan.positions[v as usize]);
            }
            *slot
        });
        triangles.push(tri);
    }
    parts.map(|(positions, triangles)| {
        (!triangles.is_empty()).then(|| TriMesh::new(positions, triangles))
    })
}

/// The mesial and distal neighbours of a preparation in its jaw `scan`: the scan away from the
/// preparation (`region`, triangles of `scan`) and near the crown (`bounds`), split by the plane
/// through `centroid` across `mesial`.
pub fn neighbours(
    scan: &TriMesh,
    region: &[u32],
    centroid: DVec3,
    mesial: DVec3,
    bounds: &Aabb,
) -> [Option<TriMesh>; 2] {
    let prep = TriMesh::new(
        scan.positions.clone(),
        region
            .iter()
            .filter_map(|&t| scan.triangles.get(t as usize).copied())
            .collect(),
    );
    let bvh = Bvh::build(&prep);
    let exclude = (!prep.triangles.is_empty()).then_some((&prep, &bvh));
    crop(scan, bounds, KEEP_MM, exclude, |c| {
        (c - centroid).dot(mesial) >= 0.0
    })
}

/// The part of the antagonist scan near the crown.
pub fn antagonist_near(scan: &TriMesh, bounds: &Aabb) -> Option<TriMesh> {
    let [near, _] = crop(scan, bounds, KEEP_MM, None, |_| true);
    near
}

/// What an adapted crown was made from.
#[derive(Clone)]
struct Stamp {
    base: Arc<Crown>,
    contacts: ContactDesign,
    jaw: MeshId,
    antagonist: Option<MeshId>,
}

impl PartialEq for Stamp {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.base, &o.base)
            && self.contacts == o.contacts
            && self.jaw == o.jaw
            && self.antagonist == o.antagonist
    }
}

/// The crown the view shows and the scans its colours were measured against.
#[derive(Clone)]
struct Shown {
    tooth: Fdi,
    crown: Arc<Crown>,
    jaw: MeshId,
    antagonist: Option<MeshId>,
}

impl PartialEq for Shown {
    fn eq(&self, o: &Self) -> bool {
        self.tooth == o.tooth
            && Arc::ptr_eq(&self.crown, &o.crown)
            && self.jaw == o.jaw
            && self.antagonist == o.antagonist
    }
}

struct Adapted {
    stamp: Stamp,
    crown: Arc<Crown>,
    report: ContactReport,
}

struct Job {
    shown: Shown,
    adapt: Option<(Stamp, Arc<InnerSurface>, BuildParams)>,
    jaw: Arc<TriMesh>,
    antagonist: Option<Arc<TriMesh>>,
    prep: Arc<Prep>,
    mesial: DVec3,
}

struct Outcome {
    shown: Shown,
    mesh: Arc<MeshData>,
    adapted: Option<Adapted>,
}

type Message = Result<Outcome, (Fdi, Option<Shown>, String)>;

fn run(job: Job) -> Result<Outcome, String> {
    let bounds = job
        .shown
        .crown
        .mesh
        .bounds()
        .ok_or_else(|| "The crown is empty".to_owned())?;
    let [mesial, distal] = neighbours(
        &job.jaw,
        &job.prep.region,
        job.prep.centroid,
        job.mesial,
        &bounds,
    );
    let antagonist = job
        .antagonist
        .as_deref()
        .and_then(|a| antagonist_near(a, &bounds));
    let mut shown = job.shown;
    let mut adapted = None;
    if let Some((stamp, surface, params)) = job.adapt {
        let mut crown = (*stamp.base).clone();
        let adapt = AdaptParams {
            proximal_contact: stamp.contacts.proximal_contact,
            occlusal_clearance: stamp.contacts.occlusal_clearance,
            ..AdaptParams::default()
        };
        let r = adapt_to_neighbours(
            &mut crown,
            &surface,
            &params,
            &adapt,
            mesial.as_ref(),
            distal.as_ref(),
            antagonist.as_ref(),
        )
        .map_err(describe)?;
        let crown = Arc::new(crown);
        shown.crown = crown.clone();
        adapted = Some(Adapted {
            stamp,
            crown,
            report: ContactReport {
                mesial: r.mesial_contact,
                distal: r.distal_contact,
                occlusal: r.min_occlusal_distance,
                moved_area: r.moved_area,
                thickness_ok: r.thickness_ok,
            },
        });
    }
    let mesh = &shown.crown.mesh;
    let all: Vec<u32> = (0..mesh.positions.len() as u32).collect();
    let mut nearest = vec![f64::NAN; all.len()];
    for other in [&mesial, &distal, &antagonist].into_iter().flatten() {
        let bvh = Bvh::build(other);
        for (n, d) in nearest
            .iter_mut()
            .zip(signed_distance_map(mesh, &all, other, &bvh))
        {
            if n.is_nan() || d < *n {
                *n = d;
            }
        }
    }
    let colors: Vec<[f32; 4]> = nearest.into_iter().map(distance_color).collect();
    let mesh = MeshData::with_colors(mesh, &colors).map_err(|e| e.to_string())?;
    Ok(Outcome {
        shown,
        mesh: Arc::new(mesh),
        adapted,
    })
}

/// The viewport tool of the step: draws the crown coloured by distance.
pub struct ContactsTool {
    mesh: Rc<RefCell<Option<Arc<MeshData>>>>,
}

impl Tool for ContactsTool {
    fn name(&self) -> &'static str {
        CONTACTS
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
            vertex_colors: true,
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
    pub objects: &'a [SceneObject],
    /// Tooth being worked on, when the step is active.
    pub tooth: Option<Fdi>,
    /// The Anatomy step's current crown of the tooth and its mesial direction.
    pub placed: Option<(Arc<Crown>, DVec3)>,
    pub surface: Option<Arc<InnerSurface>>,
    pub prep: Option<Arc<Prep>>,
    /// The tooth's jaw scan, shared with the margin tracer.
    pub jaw: Option<(MeshId, Arc<TriMesh>)>,
}

/// The app side of the step: worker jobs, the adapted crowns and the shown distance map.
pub struct ContactsStep {
    mesh: Rc<RefCell<Option<Arc<MeshData>>>>,
    wanted: Option<Shown>,
    shown: Option<Shown>,
    failed: Option<Shown>,
    /// Kind of the running job: true while adapting.
    running: Option<bool>,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    adapted: BTreeMap<Fdi, Adapted>,
    /// The Anatomy crown each tooth was last adapted from automatically.
    auto: BTreeMap<Fdi, Arc<Crown>>,
    /// "Adapt" was pressed.
    adapt_requested: bool,
    /// The last adaptation of the tooth failed.
    error: Option<(Fdi, String)>,
    /// The opposing jaw's mesh, copied once.
    antagonist: Option<(MeshId, Arc<TriMesh>)>,
    next_group: u64,
    drag: Option<(ContactParam, EditGroup)>,
    repaint: Option<egui::Context>,
}

/// The loaded scan of the jaw opposite `tooth`'s.
fn antagonist_scan(objects: &[SceneObject], tooth: Fdi) -> Option<&SceneObject> {
    let role = match tooth.arch() {
        Arch::Upper => ScanRole::Lower,
        Arch::Lower => ScanRole::Upper,
    };
    objects
        .iter()
        .find(|o| o.role == role && o.loaded().is_some())
}

impl ContactsStep {
    pub fn new(repaint: Option<egui::Context>) -> Self {
        let (tx, rx) = channel();
        Self {
            mesh: Rc::default(),
            wanted: None,
            shown: None,
            failed: None,
            running: None,
            tx,
            rx,
            adapted: BTreeMap::new(),
            auto: BTreeMap::new(),
            adapt_requested: false,
            error: None,
            antagonist: None,
            next_group: FIRST_GROUP,
            drag: None,
            repaint,
        }
    }

    /// Forgets the open case's crowns, jobs and error.
    pub fn reset(&mut self) {
        *self = Self::new(self.repaint.take());
    }

    pub fn tool(&self) -> Box<dyn Tool> {
        Box::new(ContactsTool {
            mesh: self.mesh.clone(),
        })
    }

    /// A job is running or the shown map is out of date.
    pub fn busy(&self) -> bool {
        self.running.is_some()
            || (self.wanted.is_some() && self.wanted != self.shown && self.failed != self.wanted)
    }

    fn stamp(case: &Case, objects: &[SceneObject], tooth: Fdi, base: &Arc<Crown>) -> Option<Stamp> {
        Some(Stamp {
            base: base.clone(),
            contacts: case
                .design
                .teeth
                .get(&tooth)
                .map(|t| t.contacts)
                .unwrap_or_default(),
            jaw: crate::margin_step::arch_scan(objects, tooth)?,
            antagonist: antagonist_scan(objects, tooth).map(|o| o.id),
        })
    }

    /// The adapted crown of `tooth`, while it was made from `base` (the Anatomy step's current
    /// crown), the tooth's current contact targets and the loaded scans.
    pub fn adapted(
        &self,
        case: &Case,
        objects: &[SceneObject],
        tooth: Fdi,
        base: &Arc<Crown>,
    ) -> Option<Arc<Crown>> {
        self.current(case, objects, tooth, base)
            .map(|a| a.crown.clone())
    }

    fn current(
        &self,
        case: &Case,
        objects: &[SceneObject],
        tooth: Fdi,
        base: &Arc<Crown>,
    ) -> Option<&Adapted> {
        let stamp = Self::stamp(case, objects, tooth, base)?;
        self.adapted.get(&tooth).filter(|a| a.stamp == stamp)
    }

    /// Asks for the current tooth's crown to be adapted again.
    pub fn request_adapt(&mut self) {
        self.adapt_requested = true;
    }

    /// What the step panel shows for `tooth`; `base` is the Anatomy step's current crown and
    /// `upstream` what the earlier steps are busy with.
    pub fn card(
        &self,
        case: &Case,
        objects: &[SceneObject],
        tooth: Fdi,
        base: Option<&Arc<Crown>>,
        upstream: Option<&'static str>,
    ) -> ContactsCard {
        let contacts = case
            .design
            .teeth
            .get(&tooth)
            .map(|t| t.contacts)
            .unwrap_or_default();
        let current = base.and_then(|b| self.current(case, objects, tooth, b));
        let busy = upstream.or(match self.running {
            Some(true) => Some("Adapting crown\u{2026}"),
            Some(false) => Some("Measuring distances\u{2026}"),
            None => None,
        });
        ContactsCard {
            tooth,
            has_crown: base.is_some(),
            values: ContactParam::ALL.map(|p| p.get(&contacts)),
            report: current.map(|a| a.report),
            outdated: current.is_none() && self.adapted.contains_key(&tooth),
            antagonist: antagonist_scan(objects, tooth).is_some(),
            busy,
            error: self
                .error
                .as_ref()
                .filter(|(t, _)| *t == tooth)
                .map(|(_, e)| e.clone()),
        }
    }

    /// Sets `param` of `tooth`; `dragging` merges the changes of one drag into one undo step.
    pub fn set_param(
        &mut self,
        design: &mut Design,
        history: &mut History,
        tooth: Fdi,
        param: ContactParam,
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

    /// Runs one frame: adapts the crown once per new Anatomy crown (and on request) and keeps
    /// the distance map of the shown crown up to date.
    pub fn update(&mut self, f: StepFrame<'_>) {
        while let Ok(message) = self.rx.try_recv() {
            self.running = None;
            match message {
                Ok(o) => {
                    if let Some(a) = o.adapted {
                        self.error = self.error.take().filter(|(t, _)| *t != o.shown.tooth);
                        self.adapted.insert(o.shown.tooth, a);
                    }
                    *self.mesh.borrow_mut() = Some(o.mesh);
                    self.shown = Some(o.shown);
                }
                Err((tooth, shown, e)) => {
                    self.error = Some((tooth, e));
                    if shown.is_some() {
                        self.failed = shown;
                    }
                }
            }
        }
        let inputs = f.tooth.zip(f.placed).zip(f.jaw);
        let Some(((tooth, (base, mesial)), (jaw_id, jaw))) = inputs else {
            self.wanted = None;
            self.shown = None;
            *self.mesh.borrow_mut() = None;
            return;
        };
        let Some(stamp) = Self::stamp(f.case, f.objects, tooth, &base) else {
            return;
        };
        let antagonist = match antagonist_scan(f.objects, tooth) {
            Some(o) => {
                if self.antagonist.as_ref().is_none_or(|(id, _)| *id != o.id)
                    && let Some(l) = o.loaded()
                {
                    self.antagonist = Some((o.id, Arc::new(l.mesh.clone())));
                }
                self.antagonist.as_ref().map(|(_, m)| m.clone())
            }
            None => None,
        };
        let crown = self
            .adapted
            .get(&tooth)
            .filter(|a| a.stamp == stamp)
            .map_or(base.clone(), |a| a.crown.clone());
        let wanted = Shown {
            tooth,
            crown,
            jaw: jaw_id,
            antagonist: stamp.antagonist,
        };
        if self.shown.as_ref().is_some_and(|s| s.tooth != tooth) {
            self.shown = None;
            *self.mesh.borrow_mut() = None;
        }
        self.wanted = Some(wanted.clone());
        if self.running.is_some() {
            return;
        }
        let auto = self.auto.get(&tooth).is_none_or(|b| !Arc::ptr_eq(b, &base));
        let adapt = if auto || self.adapt_requested {
            let (Some(surface), Some(_)) = (f.surface.clone(), f.prep.as_ref()) else {
                return; // The inner surface is on its way.
            };
            self.adapt_requested = false;
            self.auto.insert(tooth, base.clone());
            let m = &f.case.material;
            let params = BuildParams::new(m.min_occlusal_thickness, m.min_wall_thickness);
            Some((stamp, surface, params))
        } else if self.shown.as_ref() != Some(&wanted) && self.failed.as_ref() != Some(&wanted) {
            None
        } else {
            return;
        };
        let Some(prep) = f.prep else {
            return;
        };
        self.running = Some(adapt.is_some());
        let job = Job {
            shown: wanted,
            adapt,
            jaw,
            antagonist,
            prep,
            mesial,
        };
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let tooth = job.shown.tooth;
            let shown = job.adapt.is_none().then(|| job.shown.clone());
            let _ = tx.send(run(job).map_err(|e| (tooth, shown, e)));
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_design::synthetic::{ToothParams, prepared_tooth, rounded_box};

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn moved(mut mesh: TriMesh, by: DVec3) -> TriMesh {
        for p in &mut mesh.positions {
            *p += by;
        }
        mesh
    }

    /// A prepared tooth on the origin along +Z (margin radius 4 at height 8) with box teeth
    /// 1 mm beyond the margin along +Y (mesial) and -Y (distal), merged into one scan.
    fn scene() -> (TriMesh, Vec<u32>, Aabb) {
        let tooth = prepared_tooth(&ToothParams::default()).unwrap();
        let mut scan = tooth.mesh.clone();
        let region = tooth.region.clone();
        for y in [8.5, -8.5] {
            let b = moved(
                rounded_box(DVec3::new(4.0, 3.5, 4.0), 1.0, 8).unwrap(),
                DVec3::new(0.0, y, 9.0),
            );
            let offset = scan.positions.len() as u32;
            scan.positions.extend(b.positions);
            scan.triangles
                .extend(b.triangles.iter().map(|t| t.map(|v| v + offset)));
        }
        let crown = Aabb {
            min: DVec3::new(-5.0, -5.0, 7.5),
            max: DVec3::new(5.0, 5.0, 14.0),
        };
        (scan, region, crown)
    }

    #[test]
    fn neighbours_split_by_side_and_leave_out_the_preparation() {
        let (scan, region, crown) = scene();
        let [mesial, distal] =
            neighbours(&scan, &region, DVec3::new(0.0, 0.0, 10.0), DVec3::Y, &crown);
        let (mesial, distal) = (mesial.unwrap(), distal.unwrap());
        assert!(
            mesial.positions.iter().all(|p| p.y >= -0.5),
            "mesial side only"
        );
        assert!(
            distal.positions.iter().all(|p| p.y <= 0.5),
            "distal side only"
        );
        // Both boxes are found: their facing walls at |y| = 5 are within reach.
        assert!(mesial.positions.iter().any(|p| (p.y - 5.0).abs() < 0.01));
        assert!(distal.positions.iter().any(|p| (p.y + 5.0).abs() < 0.01));
        // Nothing within the exclusion distance of the preparation survives.
        let prep = TriMesh::new(
            scan.positions.clone(),
            region.iter().map(|&i| scan.triangles[i as usize]).collect(),
        );
        let bvh = Bvh::build(&prep);
        for part in [&mesial, &distal] {
            for i in 0..part.triangles.len() {
                let [a, b, c] = part.triangle(i);
                let d = bvh
                    .closest_point(&prep, (a + b + c) / 3.0)
                    .unwrap()
                    .distance;
                assert!(d >= EXCLUDE_MM, "triangle {d} mm from the preparation");
            }
        }
        // Far scan (the boxes' far ends beyond the reach of the crown) is left out.
        assert!(mesial.positions.iter().all(|p| p.y <= 5.0 + KEEP_MM + 1.0));
    }

    #[test]
    fn neighbours_of_an_empty_or_far_scan_are_none() {
        let (scan, region, _) = scene();
        let far = Aabb {
            min: DVec3::splat(100.0),
            max: DVec3::splat(101.0),
        };
        assert!(
            neighbours(&scan, &region, DVec3::ZERO, DVec3::Y, &far)
                .iter()
                .all(Option::is_none)
        );
        let empty = TriMesh::new(vec![], vec![]);
        assert!(antagonist_near(&empty, &far).is_none());
    }

    #[test]
    fn the_colour_map_is_white_at_contact_red_inside_and_blue_outside() {
        let white = distance_color(0.0);
        assert!(white[0] > 0.8 && white[1] > 0.8 && white[2] > 0.8);
        let red = distance_color(-0.2);
        assert!(red[0] > red[2]);
        let blue = distance_color(0.5);
        assert!(blue[2] > blue[0]);
        assert_eq!(distance_color(MAP_MIN_MM), red);
    }

    #[test]
    fn the_defaults_match_the_adaptation_defaults() {
        let c = ContactDesign::default();
        let a = AdaptParams::default();
        assert_eq!(c.proximal_contact, a.proximal_contact);
        assert_eq!(c.occlusal_clearance, a.occlusal_clearance);
    }

    #[test]
    fn a_parameter_edit_is_one_labelled_undo_step() {
        let mut design = Design::default();
        let mut history = History::default();
        let mut step = ContactsStep::new(None);
        for v in [-0.03, -0.04, -0.05] {
            step.set_param(
                &mut design,
                &mut history,
                t(36),
                ContactParam::Proximal,
                v,
                true,
            );
        }
        step.end_drag(&mut history);
        assert_eq!(design.teeth[&t(36)].contacts.proximal_contact, -0.05);
        assert_eq!(
            history.undo_label(),
            Some(edit_label(ContactParam::Proximal, -0.05).as_str())
        );
        assert_eq!(
            edit_label(ContactParam::Proximal, -0.05),
            "Set proximal contact -0.05 mm"
        );
        history.undo(&mut design);
        assert!(
            design.teeth.is_empty(),
            "back to the default, the entry goes"
        );
    }
}
