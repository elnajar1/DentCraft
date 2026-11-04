//! Tooth libraries from STL files: crown templates cut from the user's own teeth.
//!
//! A library is a folder of mesh files (STL, PLY or OBJ) plus a `library.json` manifest:
//!
//! ```json
//! { "teeth": [ {
//!     "file": "central.stl",
//!     "serves": [11, 12, 31, 32],
//!     "occlusal": [0, 0, 1],
//!     "facial": [1, 0, 0],
//!     "cervical": [-138.0, 4.0, 12.0],
//!     "mirror": false,
//!     "seal": [ { "center": [-139.5, 4.0, 22.0], "radius": 2.5 } ]
//! } ] }
//! ```
//!
//! - `serves`: FDI codes the file is used for. A code also serves its counterpart on the other
//!   side of the same arch (11 serves 21); the first entry naming a tooth wins.
//! - `occlusal`, `facial`: the tooth's occlusal (incisal) and facial directions in file
//!   coordinates. `facial` need not be perpendicular to `occlusal`.
//! - `cervical`: a point on the mid-facial CEJ, in file coordinates. A resampled crown (see
//!   below) is cut off there, with the textbook CEJ curvature of the served tooth type rising
//!   mesially and distally.
//! - `mirror`: `true` when the file is a left tooth (quadrant 2 or 4 handedness), so that
//!   `facial × occlusal` points distally rather than mesially.
//! - `seal`: optional spheres (file coordinates, millimetres) over openings in the crown surface
//!   to close, such as an endodontic access cavity or an engraved mark. The surface under them
//!   is rebuilt as a smooth membrane from its surroundings.
//!
//! A file that already is a clean open crown (one connected, consistently wound, manifold surface
//! with exactly one boundary loop, no duplicate vertex positions once zero-area triangles are
//! dropped) and has no `seal` is kept as it is: its own triangles and its own boundary as the
//! cervical line, such as the teeth [`crate::write_library`] splits from a set-up. Its winding
//! is made outward (the crown closed with a fan over its boundary has a positive volume), the
//! boundary must turn once round the occlusal axis, the cervical loop starts at the boundary
//! vertex straight facial of the loop's centre (the centre of its outline seen from occlusal),
//! and the crown is moved so that this centre is on the axis at the height of that mid-facial
//! vertex. It retains file millimetres, scaled only by explicit [`CrownParams`] factors.
//! Its landmarks (cusp tips included) come from its own surface, and it
//! is mirrored for quadrants 2 and 4. The crown keeps the file's own cervical line: it is not
//! cut at `cervical` or to the textbook CEJ curvature.
//!
//! Any other file is resampled rather than cut from the file's triangles: rays from outside towards
//! a centre inside the crown find its outer surface, so roots, pulp chambers, canals, defects
//! and the file's own winding never reach the template. The result has the parametric crown's
//! layout (160 columns from the mid-facial cervix, rows by arc length, a pole fan), is smoothed,
//! kept in file millimetres with only the explicit [`CrownParams`] width, depth and
//! height factors, and mirrored for quadrants 2 and 4. `wear` and `cusp_height` do not apply to
//! library crowns.

use std::path::{Path, PathBuf};

use dentcraft_case::Fdi;
use dentcraft_mesh::{Bvh, TriMesh};
use glam::DVec3;
use serde::Deserialize;

use crate::dimensions::dimensions;
use crate::library::{COLUMNS, cej_lingual, landmarks_for, landmarks_from_surface};
use crate::{CrownParams, CrownTemplate};

/// Manifest file name inside a library folder.
pub const MANIFEST: &str = "library.json";

/// Ray samples per column before the arc-length resampling.
const FINE_ROWS: usize = 200;
/// Rows of the template, the pole included (the parametric crown has 58).
const ROWS: usize = 58;
/// Taubin smoothing iterations over the resampled crown.
const SMOOTHING: u32 = 30;
/// Smallest cosine, between a face's normal and the ray to it from the radial pole, that the
/// radial repair leaves. `dentcraft_design::build_crown` needs it above 0; the margin survives
/// the crown's placement on a die and its thickening.
const RADIAL_MARGIN: f64 = 0.05;
/// Smoothing rounds the radial repair may take.
const REPAIR_ROUNDS: usize = 400;
/// Largest vertex move (template millimetres) the radial repair may make before it gives up.
const REPAIR_MAX_MOVE: f64 = 1.0;

/// A tooth library that failed to load or could not build a crown.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryError(pub String);

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LibraryError {}

/// What the radial repair did to a library crown: see [`ToothLibrary::crown_with_repair`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RadialRepair {
    /// Faces of the unrepaired crown that failed the radial test.
    pub faces: usize,
    /// Largest distance a vertex moved, template millimetres.
    pub largest_move: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    teeth: Vec<EntrySpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntrySpec {
    file: String,
    serves: Vec<u8>,
    occlusal: [f64; 3],
    facial: [f64; 3],
    cervical: [f64; 3],
    #[serde(default)]
    mirror: bool,
    #[serde(default)]
    seal: Vec<SealSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SealSpec {
    center: [f64; 3],
    radius: f64,
}

/// One library tooth, moved into the template frame (+Z occlusal, +Y facial, +X mesial, the
/// mid-facial CEJ point at the origin).
#[derive(Clone, Debug)]
struct LibraryTooth {
    file: String,
    serves: Vec<Fdi>,
    mesh: TriMesh,
    seals: Vec<(DVec3, f64)>,
}

/// A loaded tooth library.
#[derive(Clone, Debug)]
pub struct ToothLibrary {
    dir: PathBuf,
    teeth: Vec<LibraryTooth>,
}

/// The same tooth on the other side of the arch (11 and 21, 36 and 46).
fn counterpart(fdi: Fdi) -> Fdi {
    let q = fdi.quadrant();
    let other = match q {
        1 => 2,
        2 => 1,
        3 => 4,
        _ => 3,
    };
    Fdi::new(other * 10 + fdi.position()).unwrap_or(fdi)
}

fn vector(v: [f64; 3], what: &str, file: &str) -> Result<DVec3, LibraryError> {
    let v = DVec3::from_array(v);
    if v.is_finite() && v.length() > 1e-9 {
        Ok(v)
    } else {
        Err(LibraryError(format!(
            "{file}: \"{what}\" must be a non-zero direction"
        )))
    }
}

impl ToothLibrary {
    /// Loads the library in `dir`: its `library.json` and every mesh it names.
    pub fn load(dir: &Path) -> Result<Self, LibraryError> {
        let path = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| LibraryError(format!("cannot read {}: {e}", path.display())))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .map_err(|e| LibraryError(format!("{}: {e}", path.display())))?;
        if manifest.teeth.is_empty() {
            return Err(LibraryError(format!("{}: no teeth listed", path.display())));
        }
        let teeth = manifest
            .teeth
            .into_iter()
            .map(|entry| load_tooth(dir, entry))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            dir: dir.into(),
            teeth,
        })
    }

    /// The folder the library was loaded from.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn tooth_for(&self, fdi: Fdi) -> Option<&LibraryTooth> {
        let other = counterpart(fdi);
        self.teeth
            .iter()
            .find(|t| t.serves.contains(&fdi))
            .or_else(|| self.teeth.iter().find(|t| t.serves.contains(&other)))
    }

    /// Whether the library has a tooth for `fdi`.
    pub fn serves(&self, fdi: Fdi) -> bool {
        self.tooth_for(fdi).is_some()
    }

    /// The crown template of `fdi` built from the library tooth serving it, `Ok(None)` when no
    /// library tooth serves `fdi`.
    pub fn crown(
        &self,
        fdi: Fdi,
        params: &CrownParams,
    ) -> Result<Option<CrownTemplate>, LibraryError> {
        Ok(self.crown_with_repair(fdi, params)?.map(|(t, _)| t))
    }

    /// [`Self::crown`] with what its radial repair did.
    ///
    /// `dentcraft_design::build_crown` deforms a crown in polar coordinates about a point just
    /// below the centre of its cervical loop ([`radial_pole`]) and rejects a crown with a face
    /// seen edge-on or from behind from there (a cusp slope or fissure wall, an incisor's thin
    /// incisal edge or lingual fossa). The loader smooths only those faces' neighbourhood (two
    /// rings round them, the cervical line fixed) until every face is seen from the front with
    /// a small margin. A crown that would need more than [`REPAIR_MAX_MOVE`] mm is an error.
    pub fn crown_with_repair(
        &self,
        fdi: Fdi,
        params: &CrownParams,
    ) -> Result<Option<(CrownTemplate, RadialRepair)>, LibraryError> {
        let Some(tooth) = self.tooth_for(fdi) else {
            return Ok(None);
        };
        build(tooth, fdi, params)
            .map(Some)
            .map_err(|e| LibraryError(format!("{}: {e}", tooth.file)))
    }
}

fn load_tooth(dir: &Path, e: EntrySpec) -> Result<LibraryTooth, LibraryError> {
    let file = e.file.clone();
    let serves = e
        .serves
        .iter()
        .map(|&code| {
            Fdi::new(code)
                .map_err(|_| LibraryError(format!("{file}: {code} is not an FDI tooth number")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let z = vector(e.occlusal, "occlusal", &file)?.normalize();
    let facial = vector(e.facial, "facial", &file)?;
    let y = (facial - z * facial.dot(z))
        .try_normalize()
        .ok_or_else(|| LibraryError(format!("{file}: \"facial\" is parallel to \"occlusal\"")))?;
    let x = y.cross(z);
    let origin = DVec3::from_array(e.cervical);
    if !origin.is_finite() {
        return Err(LibraryError(format!("{file}: \"cervical\" is not a point")));
    }
    let sign = if e.mirror { -1.0 } else { 1.0 };
    let frame = |p: DVec3| {
        let d = p - origin;
        DVec3::new(sign * d.dot(x), d.dot(y), d.dot(z))
    };
    let path = dir.join(&e.file);
    let raw = dentcraft_io::load_mesh(&path).map_err(|err| LibraryError(err.to_string()))?;
    if raw.is_empty() {
        return Err(LibraryError(format!("{file}: the mesh is empty")));
    }
    let mesh = TriMesh::new(
        raw.positions.iter().map(|&p| frame(p)).collect(),
        raw.triangles,
    );
    let top = mesh.positions.iter().map(|p| p.z).fold(f64::MIN, f64::max);
    if top < 2.0 {
        return Err(LibraryError(format!(
            "{file}: the crown rises only {top:.2} mm above \"cervical\" along \"occlusal\""
        )));
    }
    let seals = e
        .seal
        .iter()
        .map(|s| {
            let c = DVec3::from_array(s.center);
            if c.is_finite() && s.radius.is_finite() && s.radius > 0.0 {
                Ok((frame(c), s.radius))
            } else {
                Err(LibraryError(format!(
                    "{file}: a seal needs a centre and a radius > 0"
                )))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LibraryTooth {
        file,
        serves,
        mesh,
        seals,
    })
}

/// Height of the cut above the mid-facial CEJ at angle `theta` (0 mesial, pi/2 facial), from
/// the CEJ curvatures mesial / distal / facial / lingual, like the parametric crown's.
fn cut_height(theta: f64, cej: [f64; 4]) -> f64 {
    let (c, s) = (theta.cos(), theta.sin());
    cej[0] * c.max(0.0).powi(3)
        + cej[1] * (-c).max(0.0).powi(3)
        + cej[2] * s.max(0.0).powi(3)
        + cej[3] * (-s).max(0.0).powi(3)
}

/// Whether the segment `a`–`b` passes within `r` of `c`.
fn segment_meets_sphere(a: DVec3, b: DVec3, c: DVec3, r: f64) -> bool {
    let ab = b - a;
    let t = ((c - a).dot(ab) / ab.length_squared().max(1e-30)).clamp(0.0, 1.0);
    (a + ab * t).distance(c) < r
}

/// The tooth as an open crown with its own cervical line, wound outward, when it is one: a
/// single connected, consistently wound surface with exactly one boundary loop, distinct vertex
/// positions and no zero-area triangles once those are dropped.
fn open_crown(mesh: &TriMesh) -> Option<(TriMesh, Vec<u32>)> {
    let mut crown = mesh.clone();
    crown.remove_degenerate_triangles(1e-10);
    crown.remove_unreferenced_vertices();
    if crown.is_empty()
        || crown.connected_components().len() != 1
        || !crown.is_consistently_oriented()
        || crown.non_manifold_edge_count() != 0
    {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    if !crown
        .positions
        .iter()
        .all(|q| seen.insert(q.to_array().map(|x| (x + 0.0).to_bits())))
    {
        return None;
    }
    let mut loops = crown.boundary_loops();
    if loops.len() != 1 {
        return None;
    }
    // Closed with a fan over its cervical line, an outward crown has a positive volume.
    let fan = |crown: &TriMesh, l: &[u32]| {
        let c = l
            .iter()
            .map(|&v| crown.positions[v as usize])
            .sum::<DVec3>()
            / l.len() as f64;
        let mut closed = crown.clone();
        let ci = closed.positions.len() as u32;
        closed.positions.push(c);
        for k in 0..l.len() {
            closed.triangles.push([ci, l[(k + 1) % l.len()], l[k]]);
        }
        closed.signed_volume()
    };
    if fan(&crown, &loops[0]) < 0.0 {
        crown.flip_orientation();
        loops = crown.boundary_loops();
    }
    let l = loops.pop()?;
    Some((crown, l))
}

/// A crown template from a library crown that already is an open crown: its own surface and
/// cervical line, moved and scaled like the resampled crown, with landmarks from its geometry.
fn build_open(
    mut crown: TriMesh,
    mut cervical: Vec<u32>,
    fdi: Fdi,
    p: &CrownParams,
) -> Result<(CrownTemplate, RadialRepair), String> {
    let at = |crown: &TriMesh, v: u32| crown.positions[v as usize];
    let (clo, chi) = cervical
        .iter()
        .map(|&v| at(&crown, v))
        .fold((DVec3::MAX, DVec3::MIN), |(lo, hi), q| {
            (lo.min(q), hi.max(q))
        });
    let (cx, cy) = ((clo.x + chi.x) / 2.0, (clo.y + chi.y) / 2.0);
    let angle = |q: DVec3| (q.x - cx).atan2(q.y - cy);
    // Seen from occlusal the outward crown's cervical line runs counter-clockwise, its angle
    // from +Y (facial) towards +X falling steadily.
    let n = cervical.len();
    let turning: f64 = (0..n)
        .map(|k| {
            let d = angle(at(&crown, cervical[(k + 1) % n])) - angle(at(&crown, cervical[k]));
            (d + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
        })
        .sum();
    if turning > -1.0 {
        return Err(
            "the open crown's boundary is not a cervical line round its occlusal axis".into(),
        );
    }
    let start = (0..n)
        .min_by(|&a, &b| {
            angle(at(&crown, cervical[a]))
                .abs()
                .total_cmp(&angle(at(&crown, cervical[b])).abs())
        })
        .unwrap_or(0);
    cervical.rotate_left(start);
    let shift = DVec3::new(cx, cy, at(&crown, cervical[0]).z);
    for q in &mut crown.positions {
        *q -= shift;
    }
    let b = crown.bounds().ok_or("empty crown")?;
    let size = b.size();
    let height = b.max.z;
    if size.x < 1e-6 || size.y < 1e-6 || height < 1e-6 {
        return Err("the crown has no extent".into());
    }
    let scale = DVec3::new(p.mesiodistal, p.buccolingual, p.height);
    for q in &mut crown.positions {
        *q *= scale;
    }
    let repair = repair_radial(&mut crown, &cervical)?;
    let landmarks = landmarks_from_surface(fdi, &crown.positions, p);
    if matches!(fdi.quadrant(), 2 | 4) {
        for q in &mut crown.positions {
            q.x = -q.x;
        }
        crown.flip_orientation();
        cervical[1..].reverse();
    }
    Ok((
        CrownTemplate {
            mesh: crown,
            cervical,
            landmarks,
        },
        repair,
    ))
}

fn build(
    tooth: &LibraryTooth,
    fdi: Fdi,
    params: &CrownParams,
) -> Result<(CrownTemplate, RadialRepair), String> {
    let p = params.sanitised();
    if tooth.seals.is_empty()
        && let Some((crown, cervical)) = open_crown(&tooth.mesh)
    {
        return build_open(crown, cervical, fdi, &p);
    }
    let dim = dimensions(fdi);
    let mesh = &tooth.mesh;
    let top = mesh.positions.iter().map(|q| q.z).fold(f64::MIN, f64::max);
    // File millimetres per textbook millimetre along the crown, for the CEJ curvature.
    let to_file = top / dim.crown_length;
    let cej = [
        dim.cej_mesial * to_file,
        dim.cej_distal * to_file,
        0.0,
        cej_lingual(fdi) * to_file,
    ];
    let lowest_cut = cej[3].min(0.0);

    // The crown part (and a little root) only: roots flaring past the crown never shadow it.
    let part = TriMesh::new(
        mesh.positions.clone(),
        mesh.triangles
            .iter()
            .copied()
            .filter(|t| {
                t.iter()
                    .any(|&v| mesh.positions[v as usize].z >= lowest_cut - 0.5)
            })
            .collect(),
    );
    let bounds = part.bounds().ok_or("no crown above the cervical point")?;
    let crown_points: Vec<DVec3> = part
        .positions
        .iter()
        .copied()
        .filter(|q| q.z >= 0.0)
        .collect();
    let (lo, hi) = crown_points
        .iter()
        .fold((DVec3::MAX, DVec3::MIN), |(lo, hi), q| {
            (lo.min(*q), hi.max(*q))
        });
    let centre = DVec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, 0.4 * top);
    let far = 4.0 * bounds.diagonal() + 10.0;
    let bvh = Bvh::build(&part);
    // Outermost surface point along the line from `centre` in direction `dir`.
    let outer = |from: DVec3, dir: DVec3| -> Option<(DVec3, DVec3)> {
        let origin = from + dir * far;
        bvh.raycast(&part, origin, -dir).map(|h| (origin, h.point))
    };

    let n = COLUMNS;
    let m = FINE_ROWS;
    let pole_dir = DVec3::Z;
    let (pole_origin, pole_hit) = outer(centre, pole_dir).ok_or("no crown above the centre")?;
    let mut dirs = vec![DVec3::ZERO; n * m];
    let mut radius = vec![0.0; n * m];
    let mut known = vec![true; n * m];
    let mut cervix = Vec::with_capacity(n);
    let sealed = |a: DVec3, b: DVec3| {
        tooth
            .seals
            .iter()
            .any(|&(c, r)| segment_meets_sphere(a, b, c, r))
    };
    let mut phi_step = Vec::with_capacity(n);
    for i in 0..n {
        let theta = std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * i as f64 / n as f64;
        let radial = DVec3::new(theta.cos(), theta.sin(), 0.0);
        let zc = cut_height(theta, cej);
        let axis_point = DVec3::new(centre.x, centre.y, zc);
        let (_, q) = outer(axis_point, radial)
            .ok_or_else(|| format!("no crown wall at the cervical cut, column {i}"))?;
        cervix.push(q);
        let rho = (q - axis_point).length();
        let phi_min = (zc - centre.z).atan2(rho);
        let step = (std::f64::consts::FRAC_PI_2 - phi_min) / (m - 1) as f64;
        phi_step.push(step);
        for j in 0..m {
            let k = i * m + j;
            let phi = phi_min + step * j as f64;
            let dir = if j == 0 {
                (q - centre).normalize()
            } else {
                radial * phi.cos() + DVec3::Z * phi.sin()
            };
            dirs[k] = dir;
            let (origin, hit) = if j == 0 {
                (q + dir * far, q)
            } else if j == m - 1 {
                (pole_origin, pole_hit)
            } else {
                outer(centre, dir).ok_or_else(|| format!("a ray missed the crown, column {i}"))?
            };
            radius[k] = (hit - centre).length();
            known[k] = j == 0 || !sealed(origin, hit);
        }
    }
    fill_sealed(&mut radius, &known, &dirs, &phi_step, n, m);

    // Positions per column, resampled to ROWS rows by arc length (pole last).
    let column = |i: usize| -> Vec<DVec3> {
        (0..m)
            .map(|j| {
                if j == 0 {
                    cervix[i]
                } else {
                    centre + dirs[i * m + j] * radius[i * m + j]
                }
            })
            .collect()
    };
    let rows = ROWS - 1; // rows per column below the pole
    let mut positions = Vec::with_capacity(n * rows + 1);
    let mut pole = DVec3::ZERO;
    for i in 0..n {
        let pts = column(i);
        let mut cum = vec![0.0; m];
        for j in 1..m {
            cum[j] = cum[j - 1] + pts[j].distance(pts[j - 1]);
        }
        let total = cum[m - 1].max(1e-9);
        let mut seg = 0;
        for k in 0..rows {
            let target = total * k as f64 / (ROWS - 1) as f64;
            while seg + 2 < m && cum[seg + 1] < target {
                seg += 1;
            }
            let span = (cum[seg + 1] - cum[seg]).max(1e-12);
            let t = ((target - cum[seg]) / span).clamp(0.0, 1.0);
            positions.push(pts[seg].lerp(pts[seg + 1], t));
        }
        pole += pts[m - 1];
    }
    positions.push(pole / n as f64);
    let index = |i: usize, k: usize| ((i % n) * rows + k) as u32;
    let pole_index = (n * rows) as u32;
    let mut triangles = Vec::with_capacity(n * (2 * rows - 1));
    for i in 0..n {
        for k in 0..rows - 1 {
            let (a, b, c, d) = (
                index(i, k),
                index(i + 1, k),
                index(i + 1, k + 1),
                index(i, k + 1),
            );
            triangles.push([a, b, c]);
            triangles.push([a, c, d]);
        }
        triangles.push([index(i, rows - 1), index(i + 1, rows - 1), pole_index]);
    }
    let cervical: Vec<u32> = (0..n).map(|i| index(i, 0)).collect();

    // Smooth: the cervical line along itself (heights kept), then the surface with it fixed.
    for _ in 0..4 {
        let old: Vec<DVec3> = cervical.iter().map(|&v| positions[v as usize]).collect();
        for (k, &v) in cervical.iter().enumerate() {
            let (a, b) = (old[(k + n - 1) % n], old[(k + 1) % n]);
            let q = &mut positions[v as usize];
            q.x = 0.5 * old[k].x + 0.25 * (a.x + b.x);
            q.y = 0.5 * old[k].y + 0.25 * (a.y + b.y);
        }
    }
    let mut crown = TriMesh::new(positions, triangles);
    crown.taubin_smooth(SMOOTHING, 0.5, -0.53, None, false);

    // Centre the cervical outline, retaining file millimetres unless the user scales it.
    let (clo, chi) = cervical
        .iter()
        .map(|&v| crown.positions[v as usize])
        .fold((DVec3::MAX, DVec3::MIN), |(lo, hi), q| {
            (lo.min(q), hi.max(q))
        });
    let shift = DVec3::new((clo.x + chi.x) / 2.0, (clo.y + chi.y) / 2.0, 0.0);
    let b = crown.bounds().ok_or("empty crown")?;
    let size = b.size();
    let height = b.max.z;
    if size.x < 1e-6 || size.y < 1e-6 || height < 1e-6 {
        return Err("the crown has no extent".into());
    }
    let scale = DVec3::new(p.mesiodistal, p.buccolingual, p.height);
    for q in &mut crown.positions {
        *q = (*q - shift) * scale;
    }
    let repair = repair_radial(&mut crown, &cervical)?;
    let landmarks = landmarks_for(fdi, &crown.positions, &p);

    let mut cervical = cervical;
    if matches!(fdi.quadrant(), 2 | 4) {
        for q in &mut crown.positions {
            q.x = -q.x;
        }
        for t in &mut crown.triangles {
            t.swap(1, 2);
        }
        // Same start (mid-facial), the other way round.
        cervical[1..].reverse();
    }
    Ok((
        CrownTemplate {
            mesh: crown,
            cervical,
            landmarks,
        },
        repair,
    ))
}

/// The point `dentcraft_design::build_crown` sees a crown's faces from: 0.01 mm below the
/// centre (vertex mean) of its cervical loop.
pub fn radial_pole(mesh: &TriMesh, cervical: &[u32]) -> DVec3 {
    cervical
        .iter()
        .map(|&v| mesh.positions[v as usize])
        .sum::<DVec3>()
        / cervical.len().max(1) as f64
        - DVec3::Z * 0.01
}

/// Faces that are degenerate or whose normal makes a cosine of at most `margin` with the ray
/// from `pole` to their centroid (`margin = 0` is `build_crown`'s test).
pub fn radially_failing(mesh: &TriMesh, pole: DVec3, margin: f64) -> Vec<usize> {
    (0..mesh.triangle_count())
        .filter(|&i| {
            let cross = mesh.triangle_cross(i);
            let [a, b, c] = mesh.triangle(i);
            let ray = (a + b + c) / 3.0 - pole;
            !cross.is_finite()
                || cross.length_squared() < 1e-24
                || cross.dot(ray) <= margin * cross.length() * ray.length()
        })
        .collect()
}

/// Smooths the neighbourhood of the faces failing the radial test until none fails with
/// [`RADIAL_MARGIN`]; the cervical line stays fixed. Bounded by [`REPAIR_ROUNDS`] and
/// [`REPAIR_MAX_MOVE`].
fn repair_radial(mesh: &mut TriMesh, cervical: &[u32]) -> Result<RadialRepair, String> {
    let pole = radial_pole(mesh, cervical);
    let faces = radially_failing(mesh, pole, 0.0).len();
    let original = mesh.positions.clone();
    let adjacency = mesh.vertex_adjacency();
    let mut locked = vec![false; original.len()];
    for &v in cervical {
        locked[v as usize] = true;
    }
    let largest = |mesh: &TriMesh| {
        mesh.positions
            .iter()
            .zip(&original)
            .map(|(a, b)| a.distance(*b))
            .fold(0.0, f64::max)
    };
    for _ in 0..REPAIR_ROUNDS {
        let bad = radially_failing(mesh, pole, RADIAL_MARGIN);
        if bad.is_empty() {
            return Ok(RadialRepair {
                faces,
                largest_move: largest(mesh),
            });
        }
        let mut region = vec![false; original.len()];
        for &f in &bad {
            for v in mesh.triangles[f] {
                region[v as usize] = true;
            }
        }
        for _ in 0..2 {
            let ring: Vec<u32> = (0..region.len() as u32)
                .filter(|&v| region[v as usize])
                .collect();
            for v in ring {
                for &w in adjacency.neighbors(v) {
                    region[w as usize] = true;
                }
            }
        }
        let old = mesh.positions.clone();
        for v in 0..old.len() {
            let ring = adjacency.neighbors(v as u32);
            if !region[v] || locked[v] || ring.is_empty() {
                continue;
            }
            let mean = ring.iter().map(|&w| old[w as usize]).sum::<DVec3>() / ring.len() as f64;
            mesh.positions[v] = old[v].lerp(mean, 0.5);
        }
        if largest(mesh) > REPAIR_MAX_MOVE {
            break;
        }
    }
    Err(format!(
        "{faces} faces are seen from behind from the crown's axis; smoothing them away would \
         move the surface more than {REPAIR_MAX_MOVE} mm"
    ))
}

/// Fills the radii of sealed samples with a smooth membrane (a harmonic interpolation by
/// successive over-relaxation) from the known samples around them. The grid is periodic across
/// columns; the last row of every column is the shared pole.
fn fill_sealed(
    radius: &mut [f64],
    known: &[bool],
    dirs: &[DVec3],
    phi_step: &[f64],
    n: usize,
    m: usize,
) {
    let unknown: Vec<usize> = (0..n * m).filter(|&k| !known[k]).collect();
    if unknown.is_empty() {
        return;
    }
    let mean = (0..n * m)
        .filter(|&k| known[k])
        .map(|k| radius[k])
        .sum::<f64>()
        / (0..n * m).filter(|&k| known[k]).count().max(1) as f64;
    for &k in &unknown {
        radius[k] = mean;
    }
    let pole_known = (0..n).all(|i| known[i * m + m - 1]);
    let d_theta = std::f64::consts::TAU / n as f64;
    for _ in 0..20_000 {
        let mut change: f64 = 0.0;
        if !pole_known {
            let avg = (0..n).map(|i| radius[i * m + m - 2]).sum::<f64>() / n as f64;
            for i in 0..n {
                let k = i * m + m - 1;
                change = change.max((avg - radius[k]).abs());
                radius[k] = avg;
            }
        }
        for &k in &unknown {
            let (i, j) = (k / m, k % m);
            if j == m - 1 {
                continue;
            }
            let cos_phi = (1.0 - dirs[k].z * dirs[k].z).sqrt().max(0.05);
            let w_theta = 1.0 / (cos_phi * d_theta).powi(2);
            let w_phi = 1.0 / phi_step[i].powi(2);
            let left = radius[((i + n - 1) % n) * m + j];
            let right = radius[((i + 1) % n) * m + j];
            let below = radius[k - 1];
            let above = radius[k + 1];
            let target =
                (w_theta * (left + right) + w_phi * (below + above)) / (2.0 * (w_theta + w_phi));
            let next = radius[k] + 1.8 * (target - radius[k]);
            change = change.max((next - radius[k]).abs());
            radius[k] = next;
        }
        if change < 1e-7 {
            break;
        }
    }
}

#[cfg(test)]
#[path = "user_library_tests.rs"]
mod tests;
