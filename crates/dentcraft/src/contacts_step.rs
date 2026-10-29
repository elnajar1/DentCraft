//! The Occlusion & contacts step: the tooth's crown adapted to its mesial and distal neighbours
//! and to the antagonist, coloured by its distance to them.
//!
//! The neighbours come from the tooth's own jaw scan: the triangles that belong to another tooth
//! beside the crown — near the crown, away from the preparation, above the gum and model base
//! under it, outside the margin polygon grown by [`MARGIN_GROW_MM`] and within a mesial (or
//! distal) band of the crown ([`neighbours`]). A side with no such triangle reports "no
//! neighbour" instead of failing. The antagonist is the opposing
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

/// [`ContactReport`] as one line for logs: each distance in mm, or which side had nothing to
/// adapt to.
pub fn report_line(report: &ContactReport) -> String {
    let mm =
        |v: Option<f64>, none: &str| v.map_or_else(|| none.to_owned(), |v| format!("{v:.3} mm"));
    format!(
        "mesial contact {}, distal contact {}, occlusal {}",
        mm(report.mesial, "none (no neighbour)"),
        mm(report.distal, "none (no neighbour)"),
        mm(report.occlusal, "none (no antagonist)")
    )
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
/// The margin polygon, projected onto the occlusal plane, is grown by this much (mm) when
/// excluding what belongs to the prepared tooth itself: its root or stump wall below the
/// margin and the gum cuff under the crown, which all sit inside the crown's footprint.
/// Tuned on the synthetic case `scratch/case/prep-neighbours.dentcraft`: 0.8 mm takes in a
/// root that flares outward by 3° below the margin while staying clear of a neighbour tooth
/// whose facing wall is 0.3 mm from the crown.
pub const MARGIN_GROW_MM: f64 = 0.8;
/// A neighbour triangle is at least this high, in mm along the tooth's insertion axis above
/// the margin's lowest point minus this much: anything lower is deep gum or the model's base
/// plate under the crown, not another tooth. The 1 mm slack keeps a neighbour whose own
/// gumline dips slightly below the prepared tooth's margin.
pub const BELOW_MARGIN_MM: f64 = 1.0;
/// A neighbour stands beside the crown: along the mesial (or distal) direction it reaches
/// from this far behind the crown's own extreme, in mm. The 1.5 mm slack covers the crown
/// being slightly smaller than the space, and the 0.28 mm an adaptation moves it.
pub const BEHIND_EXTREME_MM: f64 = 1.5;
/// A neighbour reaches at most this far (mm) beyond the crown's mesial (or distal) extreme:
/// a facing wall 0.3 mm away sits at +0.3, and 4 mm still takes in the neighbour's whole
/// facing surface without reaching past it to the tooth after next.
pub const BEYOND_EXTREME_MM: f64 = 4.0;
/// A neighbour also has to sit within the crown's buccolingual extent: at most the crown's
/// half-depth plus this slack (mm), so buccal or lingual walls of the scan that merely pass
/// beside the crown are not mistaken for a proximal neighbour.
pub const BESIDE_CROWN_MM: f64 = 1.0;
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

/// The triangles of `scan` whose centroids lie within `keep` mm of `bounds`, not within
/// [`EXCLUDE_MM`] of `exclude`, sorted by `classify` into the mesial part (0), the distal
/// part (1) or none. Zero-area triangles are dropped.
/// Positions and triangles of one part of a cropped scan.
type Part = (Vec<DVec3>, Vec<[u32; 3]>);

fn crop(
    scan: &TriMesh,
    bounds: &Aabb,
    keep: f64,
    exclude: Option<(&TriMesh, &Bvh)>,
    classify: impl Fn(DVec3) -> Option<usize>,
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
        if let Some(k) = classify(centre) {
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
    }
    parts.map(|(positions, triangles)| {
        (!triangles.is_empty()).then(|| TriMesh::new(positions, triangles))
    })
}

/// Whether the 2-D point `q` lies inside `ring` or within `grow` of its boundary: inside the
/// polygon grown by `grow`, which need not be convex. Fewer than three vertices exclude
/// nothing.
fn inside_grown(ring: &[(f64, f64)], q: (f64, f64), grow: f64) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        if (a.1 > q.1) != (b.1 > q.1) && q.0 < (b.0 - a.0) * (q.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        let ab = (b.0 - a.0, b.1 - a.1);
        let len2 = ab.0 * ab.0 + ab.1 * ab.1;
        if len2 > 0.0 {
            let t = (((q.0 - a.0) * ab.0 + (q.1 - a.1) * ab.1) / len2).clamp(0.0, 1.0);
            let dx = a.0 + t * ab.0 - q.0;
            let dy = a.1 + t * ab.1 - q.1;
            if dx * dx + dy * dy <= grow * grow {
                return true;
            }
        }
    }
    inside
}

/// The mesial and distal neighbours of a preparation in its jaw `scan`: the triangles that
/// belong to another tooth beside the crown, never to the prepared tooth itself or the gum
/// under it. A triangle qualifies when it is near the crown, away from the preparation
/// (`region`, its triangles in `scan`), above deep gum and the model base ([`BELOW_MARGIN_MM`]
/// below the margin's lowest point along the insertion `axis`), outside the `margin` polygon
/// grown by [`MARGIN_GROW_MM`] in the occlusal plane, and within the crown's mesial (or
/// distal) band: from [`BEHIND_EXTREME_MM`] behind to [`BEYOND_EXTREME_MM`] beyond the crown's
/// extreme along `mesial`, and within the crown's half-depth plus [`BESIDE_CROWN_MM`]
/// buccolingually. `crown` is the crown to adapt; a side with no such triangle is `None`.
pub fn neighbours(
    scan: &TriMesh,
    region: &[u32],
    margin: &[DVec3],
    axis: DVec3,
    mesial: DVec3,
    crown: &TriMesh,
) -> [Option<TriMesh>; 2] {
    let (Some(axis), Some(bounds)) = (axis.try_normalize(), crown.bounds()) else {
        return [None, None];
    };
    let mesial = mesial - axis * mesial.dot(axis);
    let Some(mesial) = mesial.try_normalize() else {
        return [None, None];
    };
    let lateral = axis.cross(mesial);
    let flat = |p: DVec3| (p.dot(mesial), p.dot(lateral));
    let ring: Vec<_> = margin.iter().map(|&p| flat(p)).collect();
    let floor = margin
        .iter()
        .map(|p| p.dot(axis))
        .fold(f64::NEG_INFINITY, f64::max)
        - BELOW_MARGIN_MM;
    let extent = |d: DVec3| {
        crown
            .positions
            .iter()
            .map(|p| p.dot(d))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            })
    };
    let (u_min, u_max) = extent(mesial);
    let (v_min, v_max) = extent(lateral);
    let (v_centre, v_half) = ((v_min + v_max) / 2.0, (v_max - v_min) / 2.0);
    let band = |c: DVec3, extreme: f64, sign: f64| {
        let d = (c.dot(mesial) - extreme) * sign;
        (-BEHIND_EXTREME_MM..=BEYOND_EXTREME_MM).contains(&d)
            && (c.dot(lateral) - v_centre).abs() <= v_half + BESIDE_CROWN_MM
    };
    let prep = TriMesh::new(
        scan.positions.clone(),
        region
            .iter()
            .filter_map(|&t| scan.triangles.get(t as usize).copied())
            .collect(),
    );
    let bvh = Bvh::build(&prep);
    let exclude = (!prep.triangles.is_empty()).then_some((&prep, &bvh));
    crop(scan, &bounds, KEEP_MM, exclude, |c| {
        if c.dot(axis) < floor || inside_grown(&ring, flat(c), MARGIN_GROW_MM) {
            None
        } else if band(c, u_max, 1.0) {
            Some(0)
        } else if band(c, u_min, -1.0) {
            Some(1)
        } else {
            None
        }
    })
}

/// The part of the antagonist scan near the crown.
pub fn antagonist_near(scan: &TriMesh, bounds: &Aabb) -> Option<TriMesh> {
    let [near, _] = crop(scan, bounds, KEEP_MM, None, |_| Some(0));
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
    /// The tooth's stored margin polygon and insertion axis, for choosing the neighbours.
    margin: Vec<DVec3>,
    axis: DVec3,
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
        &job.margin,
        job.axis,
        job.mesial,
        &job.shown.crown.mesh,
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
        let tooth_design = f.case.design.teeth.get(&tooth);
        let margin = tooth_design
            .and_then(|t| t.margin.clone())
            .unwrap_or_default();
        let axis = tooth_design
            .and_then(|t| t.insertion_axis)
            .unwrap_or(DVec3::ZERO);
        self.running = Some(adapt.is_some());
        let job = Job {
            shown: wanted,
            adapt,
            jaw,
            antagonist,
            prep,
            mesial,
            margin,
            axis,
        };
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let tooth = job.shown.tooth;
        let shown = job.adapt.is_none().then(|| job.shown.clone());
        crate::crash::spawn_worker(
            "contacts",
            move || run(job),
            move |result| {
                let _ = tx.send(result.and_then(|r| r).map_err(|e| (tooth, shown, e)));
                if let Some(ctx) = repaint {
                    ctx.request_repaint();
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_design::synthetic::{ToothParams, crown_template, prepared_tooth, rounded_box};
    use dentcraft_design::{InnerSurfaceParams, auto_placement, build_crown, inner_surface};
    use std::f64::consts::TAU;

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn moved(mut mesh: TriMesh, by: DVec3) -> TriMesh {
        for p in &mut mesh.positions {
            *p += by;
        }
        mesh
    }

    /// Clear gap between the crown and each neighbour box, mm.
    const GAP: f64 = 0.3;
    /// Half-extents of the neighbour boxes (mesial, buccolingual, occlusal), mm.
    const BOX: DVec3 = DVec3::new(2.0, 3.0, 4.0);
    /// Height of the boxes' centres, mm.
    const BOX_CENTRE_Z: f64 = 10.0;

    /// The scene of the neighbour tests: a preparation on the origin along +Z (margin radius 4
    /// at height 8) with its base plate and root below the margin and a gum band around the
    /// root, a crown on it, and two rounded-box teeth [`GAP`] mm from the crown's mesial and
    /// distal extremes (±X), merged into one jaw scan.
    struct Scene {
        scan: TriMesh,
        region: Vec<u32>,
        margin: Vec<DVec3>,
        axis: DVec3,
        crown: Crown,
        inner: InnerSurface,
        params: BuildParams,
        /// The crown's outer extremes along the mesial axis: [distal, mesial].
        sides: [f64; 2],
    }

    fn scene() -> Scene {
        let die = prepared_tooth(&ToothParams {
            segments: 128,
            wall_steps: 24,
            ..Default::default()
        })
        .unwrap();
        let inner = inner_surface(
            &die.mesh,
            &die.region,
            &die.margin,
            die.axis,
            &InnerSurfaceParams {
                grid_step: 0.08,
                ..Default::default()
            },
        )
        .unwrap();
        let (template, cervical) = crown_template(4.0, 5.5, 64, 18).unwrap();
        let params = BuildParams::new(0.8, 0.5);
        let mut placement = auto_placement(&inner, &template, &cervical, die.axis, None);
        placement.scale.x *= 1.35;
        placement.scale.y *= 1.35;
        placement.scale.z *= 1.12;
        let crown = build_crown(&inner, &template, &cervical, placement, &params).unwrap();
        let outer = &crown.mesh.positions[inner.mesh.positions.len()..];
        let left = outer.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let right = outer.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let mut scan = die.mesh.clone();
        gum_band(&mut scan, 4.5, 6.4, 7.9);
        for centre in [right + GAP + BOX.x, left - GAP - BOX.x] {
            scan.merge(&moved(
                rounded_box(BOX, 0.5, 16).unwrap(),
                DVec3::new(centre, 0.0, BOX_CENTRE_Z),
            ));
        }
        Scene {
            scan,
            region: die.region.clone(),
            margin: die.margin.clone(),
            axis: die.axis,
            crown,
            inner,
            params,
            sides: [left, right],
        }
    }

    /// A gum band (open tube) of `radius` around the root from `z0` up to `z1`, like a gingiva
    /// cuff whose crest sits just below the margin.
    fn gum_band(scan: &mut TriMesh, radius: f64, z0: f64, z1: f64) {
        let n = 128;
        let base = scan.positions.len() as u32;
        for z in [z0, z1] {
            for i in 0..n {
                let a = TAU * i as f64 / n as f64;
                scan.positions
                    .push(DVec3::new(radius * a.cos(), radius * a.sin(), z));
            }
        }
        for i in 0..n as u32 {
            let a = base + i;
            let b = base + (i + 1) % n as u32;
            let (c, d) = (b + n as u32, a + n as u32);
            scan.triangles.extend([[a, b, c], [a, c, d]]);
        }
    }

    #[test]
    fn neighbours_are_only_the_teeth_beside_the_crown() {
        let s = scene();
        let [mesial, distal] = neighbours(
            &s.scan,
            &s.region,
            &s.margin,
            s.axis,
            DVec3::X,
            &s.crown.mesh,
        );
        for (part, facing, [lo, hi]) in [
            (
                mesial.as_ref(),
                s.sides[1] + GAP,
                [s.sides[1] + GAP, s.sides[1] + GAP + 2.0 * BOX.x],
            ),
            (
                distal.as_ref(),
                s.sides[0] - GAP,
                [s.sides[0] - GAP - 2.0 * BOX.x, s.sides[0] - GAP],
            ),
        ] {
            let part = part.expect("the neighbour tooth is found");
            assert!(
                part.positions.iter().any(|p| (p.x - facing).abs() < 0.01),
                "its facing wall, {GAP} mm from the crown"
            );
            for p in &part.positions {
                assert!(
                    p.x >= lo - 0.01
                        && p.x <= hi + 0.01
                        && p.y.abs() <= BOX.y + 0.01
                        && (p.z - BOX_CENTRE_Z).abs() <= BOX.z + 0.01,
                    "{p:?} is not part of the neighbour box"
                );
            }
        }
        // Each side's triangles lie beyond the crown's extreme on that side: nothing of the
        // preparation, its root, the plate or the gum band survived.
        let (m, d) = (mesial.unwrap(), distal.unwrap());
        assert!(m.positions.iter().all(|p| p.x > 0.0));
        assert!(d.positions.iter().all(|p| p.x < 0.0));
    }

    #[test]
    fn the_scene_adapts_to_light_contacts_on_both_sides() {
        let s = scene();
        let [mesial, distal] = neighbours(
            &s.scan,
            &s.region,
            &s.margin,
            s.axis,
            DVec3::X,
            &s.crown.mesh,
        );
        let mut crown = s.crown.clone();
        let report = adapt_to_neighbours(
            &mut crown,
            &s.inner,
            &s.params,
            &AdaptParams::default(),
            mesial.as_ref(),
            distal.as_ref(),
            None,
        )
        .unwrap();
        assert!(
            (report.mesial_contact.unwrap() + 0.02).abs() <= 0.005,
            "{report:?}"
        );
        assert!(
            (report.distal_contact.unwrap() + 0.02).abs() <= 0.005,
            "{report:?}"
        );
    }

    #[test]
    fn a_side_without_a_neighbour_reports_no_neighbour() {
        let s = scene();
        let mut far = s.crown.mesh.clone();
        for p in &mut far.positions {
            p.x += 100.0;
        }
        assert!(
            neighbours(&s.scan, &s.region, &s.margin, s.axis, DVec3::X, &far)
                .iter()
                .all(Option::is_none),
            "nothing near the far crown"
        );
        // A missing neighbour is not an error for the adaptation itself.
        let mut crown = s.crown.clone();
        let report = adapt_to_neighbours(
            &mut crown,
            &s.inner,
            &s.params,
            &AdaptParams::default(),
            None,
            None,
            None,
        )
        .unwrap();
        assert!(report.mesial_contact.is_none() && report.distal_contact.is_none());
        let empty = TriMesh::new(vec![], vec![]);
        let far_bounds = Aabb {
            min: DVec3::splat(100.0),
            max: DVec3::splat(101.0),
        };
        assert!(antagonist_near(&empty, &far_bounds).is_none());
    }

    #[test]
    fn the_grown_margin_takes_the_cuff_and_leaves_the_neighbour() {
        let square = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        assert!(inside_grown(&square, (2.0, 2.0), 0.8), "well inside");
        assert!(inside_grown(&square, (4.5, 2.0), 0.8), "0.5 mm outside");
        assert!(!inside_grown(&square, (5.0, 2.0), 0.8), "1.0 mm outside");
        assert!(inside_grown(&square, (2.0, -0.7), 0.8), "below an edge");
        assert!(!inside_grown(&square, (10.0, 10.0), 0.8), "far away");
        assert!(!inside_grown(&[], (2.0, 2.0), 0.8), "no margin");
        assert!(!inside_grown(&square[..2], (2.0, 2.0), 0.8), "degenerate");
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
