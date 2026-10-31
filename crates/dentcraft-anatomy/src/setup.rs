//! Tooth libraries from a set-up: one mesh of separate anatomic crowns arranged in an upper and a
//! lower arch, each crown open at its cervical line.
//!
//! [`split_setup`] tells the crowns apart and names them:
//!
//! - every connected component is one crown and must have exactly one open boundary, its
//!   cervical line; its axis runs from the centre of that line to the crown's area centroid;
//! - the occlusal plane is the least-squares plane through the crown centroids. Crowns whose
//!   axis points along its normal form one arch, crowns pointing against it the other, and each
//!   arch must lie on the side of the plane facing the other (upper crowns point down at the
//!   lower arch). A crown's `occlusal` direction is that plane normal, signed for its arch;
//! - the crowns of an arch are ordered along the arch by the shortest path through their
//!   centroids (a minimum spanning tree that must be a chain). The front of the arch is the
//!   direction from the line joining its two end crowns to the crown farthest from that line;
//!   the midline runs between the two neighbouring crowns that sit farthest to the front;
//! - the upper arch is the one with the wider central incisors (textbook 8.5 mm against
//!   5.0 mm); the patient's right is `occlusal(upper) × front`, so a mirrored file is numbered
//!   as its mirror image;
//! - teeth are numbered from the midline outward on each side (1 central … 8 third molar), so a
//!   set-up must start at the central incisors and leave no gap: a gap wider than the crowns
//!   beside it is an error;
//! - `facial` is the arch's own outward direction at the crown (perpendicular to the arch in the
//!   occlusal plane, away from the arch centre), and `cervical` is the point of the crown's own
//!   cervical line straight facial of its centre.
//!
//! Anything this cannot decide is an error naming the crown (as the 1-based component number of
//! the input mesh, with its centroid), never a guess.

use std::path::Path;

use dentcraft_case::Fdi;
use dentcraft_mesh::TriMesh;
use glam::DVec3;
use serde::Serialize;

use crate::user_library::{LibraryError, MANIFEST};

/// Most teeth on one side of an arch.
const SIDE: usize = 8;
/// Gap allowed between neighbouring crowns beyond touching (mm).
const GAP: f64 = 2.0;
/// Central incisors of the upper arch must be at least this much wider than the lower ones.
const UPPER_CENTRAL_RATIO: f64 = 1.2;

/// One crown of a set-up, named and with its manifest directions (in the set-up's coordinates).
#[derive(Clone, Debug)]
pub struct SetupTooth {
    /// The tooth the crown is.
    pub fdi: Fdi,
    /// 1-based component number in the input mesh.
    pub component: usize,
    /// The crown alone.
    pub mesh: TriMesh,
    /// Unit occlusal direction (the occlusal plane normal, towards the opposing arch).
    pub occlusal: DVec3,
    /// Unit facial direction, perpendicular to `occlusal`.
    pub facial: DVec3,
    /// Mid-facial point of the crown's own cervical line (a vertex of `mesh`).
    pub cervical: DVec3,
}

struct Crown {
    component: usize,
    mesh: TriMesh,
    centroid: DVec3,
    cervix: Vec<u32>,
    cervix_centre: DVec3,
    axis: DVec3,
}

fn name(c: &Crown) -> String {
    let g = c.centroid;
    format!(
        "component {} (centroid {:.1}, {:.1}, {:.1})",
        c.component, g.x, g.y, g.z
    )
}

fn err(text: String) -> LibraryError {
    LibraryError(text)
}

/// Area-weighted centroid of a surface.
fn area_centroid(mesh: &TriMesh) -> Option<DVec3> {
    let (mut sum, mut area) = (DVec3::ZERO, 0.0);
    for i in 0..mesh.triangle_count() {
        let [a, b, c] = mesh.triangle(i);
        let w = (b - a).cross(c - a).length();
        sum += (a + b + c) * (w / 3.0);
        area += w;
    }
    (area > 1e-12).then(|| sum / area)
}

/// Length-weighted centre of a closed polyline.
fn loop_centre(mesh: &TriMesh, l: &[u32]) -> DVec3 {
    let (mut sum, mut length) = (DVec3::ZERO, 0.0);
    for k in 0..l.len() {
        let (a, b) = (
            mesh.positions[l[k] as usize],
            mesh.positions[l[(k + 1) % l.len()] as usize],
        );
        let w = a.distance(b);
        sum += (a + b) * (w / 2.0);
        length += w;
    }
    if length > 1e-12 {
        sum / length
    } else {
        mesh.positions[l[0] as usize]
    }
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric 3×3 matrix (cyclic Jacobi).
fn eigen_symmetric(mut a: [[f64; 3]; 3]) -> ([f64; 3], [DVec3; 3]) {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..64 {
        let off = a[0][1].powi(2) + a[0][2].powi(2) + a[1][2].powi(2);
        let scale = a[0][0].powi(2) + a[1][1].powi(2) + a[2][2].powi(2);
        if off <= 1e-30 * scale.max(1e-300) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in &mut a {
                let (kp, kq) = (row[p], row[q]);
                row[p] = c * kp - s * kq;
                row[q] = s * kp + c * kq;
            }
            let (row_p, row_q) = (a[p], a[q]);
            for k in 0..3 {
                a[p][k] = c * row_p[k] - s * row_q[k];
                a[q][k] = s * row_p[k] + c * row_q[k];
            }
            for row in &mut v {
                let (kp, kq) = (row[p], row[q]);
                row[p] = c * kp - s * kq;
                row[q] = s * kp + c * kq;
            }
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let values = order.map(|i| a[i][i]);
    let vectors = order.map(|i| DVec3::new(v[0][i], v[1][i], v[2][i]).normalize());
    (values, vectors)
}

fn crowns(mesh: &TriMesh) -> Result<Vec<Crown>, LibraryError> {
    let parts = mesh.connected_components();
    let mut out = Vec::with_capacity(parts.len());
    for (k, part) in parts.iter().enumerate() {
        let sub = mesh.extract_triangles(part);
        let component = k + 1;
        let centroid = area_centroid(&sub).ok_or_else(|| {
            err(format!(
                "component {component} has no area: it is not a crown"
            ))
        })?;
        let mut crown = Crown {
            component,
            mesh: sub,
            centroid,
            cervix: Vec::new(),
            cervix_centre: DVec3::ZERO,
            axis: DVec3::ZERO,
        };
        let loops = crown.mesh.boundary_loops();
        if loops.len() != 1 {
            return Err(err(format!(
                "{} has {} open boundaries; a set-up crown has exactly one, its cervical line",
                name(&crown),
                loops.len()
            )));
        }
        if loops[0].len() < 3 {
            return Err(err(format!(
                "{} has a cervical line of {} vertices",
                name(&crown),
                loops[0].len()
            )));
        }
        crown.cervix_centre = loop_centre(&crown.mesh, &loops[0]);
        crown.cervix = loops.into_iter().next().unwrap_or_default();
        crown.axis = (crown.centroid - crown.cervix_centre)
            .try_normalize()
            .ok_or_else(|| {
                err(format!(
                    "{}: the crown does not rise above its cervical line",
                    name(&crown)
                ))
            })?;
        out.push(crown);
    }
    Ok(out)
}

fn add_outer(m: &mut [[f64; 3]; 3], d: DVec3) {
    let d = d.to_array();
    for (i, row) in m.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell += d[i] * d[j];
        }
    }
}

/// The occlusal plane normal: the least-squares plane through the crown centroids, fitted to
/// both arches at once as two parallel planes (each arch about its own mean), so that the
/// distance between the arches does not tilt it. The arches for that fit are the two senses of
/// the crowns' dominant axis direction.
fn plane_normal(crowns: &[Crown]) -> Result<DVec3, LibraryError> {
    let mut axes = [[0.0; 3]; 3];
    for c in crowns {
        add_outer(&mut axes, c.axis);
    }
    let dominant = eigen_symmetric(axes).1[2];
    let mut m = [[0.0; 3]; 3];
    for sense in [1.0, -1.0] {
        let arch: Vec<DVec3> = crowns
            .iter()
            .filter(|c| c.axis.dot(dominant) * sense > 0.0)
            .map(|c| c.centroid)
            .collect();
        let mean = arch.iter().sum::<DVec3>() / arch.len().max(1) as f64;
        for g in arch {
            add_outer(&mut m, g - mean);
        }
    }
    let (values, vectors) = eigen_symmetric(m);
    if values[1] < 1.5 * values[0].max(0.0) || values[1] <= 1e-12 {
        return Err(err(
            "the crowns do not spread out along an occlusal plane: cannot find the arches".into(),
        ));
    }
    Ok(vectors[0])
}

/// The crowns of one arch (indices into `crowns`) in order along the arch.
fn arch_order(
    crowns: &[Crown],
    members: &[usize],
    u: DVec3,
    v: DVec3,
) -> Result<Vec<usize>, LibraryError> {
    let k = members.len();
    let plan = |i: usize| {
        let g = crowns[members[i]].centroid;
        (g.dot(u), g.dot(v))
    };
    let dist = |i: usize, j: usize| {
        let (a, b) = (plan(i), plan(j));
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    };
    // Prim's minimum spanning tree.
    let mut joined = vec![false; k];
    let mut best = vec![(f64::INFINITY, usize::MAX); k];
    let mut neighbours = vec![Vec::new(); k];
    joined[0] = true;
    for (j, b) in best.iter_mut().enumerate().skip(1) {
        *b = (dist(0, j), 0);
    }
    for _ in 1..k {
        let next = (0..k)
            .filter(|&j| !joined[j])
            .min_by(|&a, &b| best[a].0.total_cmp(&best[b].0))
            .unwrap_or(0);
        joined[next] = true;
        let from = best[next].1;
        neighbours[next].push(from);
        neighbours[from].push(next);
        for j in 0..k {
            if !joined[j] && dist(next, j) < best[j].0 {
                best[j] = (dist(next, j), next);
            }
        }
    }
    if let Some(i) = (0..k).find(|&i| neighbours[i].len() > 2) {
        return Err(err(format!(
            "{} has more than two nearest neighbours: cannot order the crowns along the arch",
            name(&crowns[members[i]])
        )));
    }
    let start = (0..k).find(|&i| neighbours[i].len() <= 1).unwrap_or(0);
    let mut order = vec![start];
    let mut previous = usize::MAX;
    while order.len() < k {
        let current = *order.last().unwrap_or(&start);
        let Some(&next) = neighbours[current].iter().find(|&&n| n != previous) else {
            break;
        };
        previous = current;
        order.push(next);
    }
    Ok(order.into_iter().map(|i| members[i]).collect())
}

/// Extent of a crown along `dir`.
fn width(c: &Crown, dir: DVec3) -> f64 {
    let (lo, hi) = c
        .mesh
        .positions
        .iter()
        .map(|p| p.dot(dir))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
            (lo.min(x), hi.max(x))
        });
    hi - lo
}

/// One arch, ordered, with its front direction and the midline between `order[mid]` and
/// `order[mid + 1]`.
struct ArchLayout {
    order: Vec<usize>,
    tangents: Vec<DVec3>,
    front: DVec3,
    mid: usize,
    centre: DVec3,
    central_width: f64,
}

fn layout(
    crowns: &[Crown],
    members: &[usize],
    n: DVec3,
    which: &str,
) -> Result<ArchLayout, LibraryError> {
    let u = n.any_orthonormal_vector();
    let v = n.cross(u);
    if members.len() < 3 {
        return Err(err(format!(
            "the {which} arch has only {} crown(s): at least three are needed to find its front",
            members.len()
        )));
    }
    let order = arch_order(crowns, members, u, v)?;
    let flat = |i: usize| {
        let g = crowns[order[i]].centroid;
        g - n * g.dot(n)
    };
    let k = order.len();
    let tangents: Vec<DVec3> = (0..k)
        .map(|i| {
            let (a, b) = (flat(i.saturating_sub(1)), flat((i + 1).min(k - 1)));
            (b - a).normalize_or_zero()
        })
        .collect();
    for i in 0..k - 1 {
        let (a, b) = (&crowns[order[i]], &crowns[order[i + 1]]);
        let along = (flat(i + 1) - flat(i)).normalize_or_zero();
        let spacing = flat(i).distance(flat(i + 1));
        let touching = 0.5 * (width(a, along) + width(b, along));
        if spacing > touching + GAP {
            return Err(err(format!(
                "a gap of {:.1} mm between {} and {}: is a tooth missing from the set-up?",
                spacing - touching,
                name(a),
                name(b)
            )));
        }
    }
    let (e0, e1) = (flat(0), flat(k - 1));
    let chord = (e1 - e0).normalize_or_zero();
    let offset = |i: usize| {
        let d = flat(i) - e0;
        d - chord * d.dot(chord)
    };
    let apex = (0..k)
        .max_by(|&a, &b| offset(a).length().total_cmp(&offset(b).length()))
        .unwrap_or(0);
    let depth = offset(apex).length();
    if depth < 0.15 * e0.distance(e1) || depth < 1e-6 {
        return Err(err(format!(
            "the {which} arch from {} to {} is nearly straight: cannot tell its front",
            name(&crowns[order[0]]),
            name(&crowns[order[k - 1]])
        )));
    }
    let front = offset(apex) / depth;
    let mid = (0..k - 1)
        .max_by(|&a, &b| {
            (flat(a) + flat(a + 1))
                .dot(front)
                .total_cmp(&(flat(b) + flat(b + 1)).dot(front))
        })
        .unwrap_or(0);
    let centre = (0..k).map(flat).sum::<DVec3>() / k as f64;
    let central_width = 0.5
        * (width(&crowns[order[mid]], tangents[mid])
            + width(&crowns[order[mid + 1]], tangents[mid + 1]));
    Ok(ArchLayout {
        order,
        tangents,
        front,
        mid,
        centre,
        central_width,
    })
}

/// Splits a set-up of separate open crowns into named library teeth (see the module docs),
/// sorted by FDI number.
pub fn split_setup(mesh: &TriMesh) -> Result<Vec<SetupTooth>, LibraryError> {
    if mesh.is_empty() {
        return Err(err("the set-up mesh is empty".into()));
    }
    let crowns = crowns(mesh)?;
    if crowns.len() < 6 {
        return Err(err(format!(
            "the set-up has {} crown(s): an upper and a lower arch of at least three each are needed",
            crowns.len()
        )));
    }
    let n = plane_normal(&crowns)?;
    let mut along = Vec::new();
    let mut against = Vec::new();
    for (i, c) in crowns.iter().enumerate() {
        let s = c.axis.dot(n);
        if s.abs() < 0.5 {
            return Err(err(format!(
                "{} points {:.0}° away from the occlusal plane normal: it belongs to neither arch",
                name(c),
                s.abs().acos().to_degrees()
            )));
        }
        if s > 0.0 {
            along.push(i);
        } else {
            against.push(i);
        }
    }
    if along.is_empty() || against.is_empty() {
        return Err(err(
            "every crown points the same way: a set-up needs an upper and a lower arch facing each other"
                .into(),
        ));
    }
    // Crowns pointing along +n face the arch above them along n: they must sit below it.
    let level = |set: &[usize]| {
        set.iter().map(|&i| crowns[i].centroid.dot(n)).sum::<f64>() / set.len() as f64
    };
    let middle = 0.5 * (level(&along) + level(&against));
    for &i in &along {
        if crowns[i].centroid.dot(n) >= middle {
            return Err(err(format!(
                "{} points away from the opposing arch",
                name(&crowns[i])
            )));
        }
    }
    for &i in &against {
        if crowns[i].centroid.dot(n) <= middle {
            return Err(err(format!(
                "{} points away from the opposing arch",
                name(&crowns[i])
            )));
        }
    }
    let a = layout(&crowns, &along, n, "first")?;
    let b = layout(&crowns, &against, -n, "second")?;
    let (wide, narrow) = (
        a.central_width.max(b.central_width),
        a.central_width.min(b.central_width),
    );
    if wide < UPPER_CENTRAL_RATIO * narrow {
        return Err(err(format!(
            "cannot tell the upper arch from the lower: their central incisors are {:.1} and {:.1} mm wide",
            a.central_width, b.central_width
        )));
    }
    // (layout, occlusal, upper?)
    let (upper, lower) = if a.central_width > b.central_width {
        ((a, n), (b, -n))
    } else {
        ((b, -n), (a, n))
    };
    let upper_occlusal = upper.1;
    let mut teeth = Vec::with_capacity(crowns.len());
    for ((arch, occlusal), is_upper) in [(upper, true), (lower, false)] {
        let right = upper_occlusal.cross(arch.front);
        let k = arch.order.len();
        // Is the part of the chain after the midline on the patient's right?
        let after_right = (crowns[arch.order[arch.mid + 1]].centroid
            - crowns[arch.order[arch.mid]].centroid)
            .dot(right)
            > 0.0;
        let before: Vec<usize> = (0..=arch.mid).rev().collect();
        let after: Vec<usize> = (arch.mid + 1..k).collect();
        let (right_side, left_side) = if after_right {
            (after, before)
        } else {
            (before, after)
        };
        let quadrants = if is_upper { (1, 2) } else { (4, 3) };
        for (side, quadrant, label) in [
            (right_side, quadrants.0, "right"),
            (left_side, quadrants.1, "left"),
        ] {
            if side.len() > SIDE {
                return Err(err(format!(
                    "{} crowns on the patient's {label} of the {} arch, at most {SIDE} (from {})",
                    side.len(),
                    if is_upper { "upper" } else { "lower" },
                    name(&crowns[arch.order[side[SIDE]]])
                )));
            }
            for (position, &i) in side.iter().enumerate() {
                let c = &crowns[arch.order[i]];
                let fdi =
                    Fdi::new(quadrant * 10 + position as u8 + 1).map_err(|e| err(e.to_string()))?;
                let mut facial = occlusal.cross(arch.tangents[i]).normalize_or_zero();
                let flat = c.centroid - occlusal * c.centroid.dot(occlusal);
                if facial.dot(flat - arch.centre) < 0.0 {
                    facial = -facial;
                }
                if facial == DVec3::ZERO {
                    return Err(err(format!("{}: no facial direction", name(c))));
                }
                teeth.push(SetupTooth {
                    fdi,
                    component: c.component,
                    mesh: c.mesh.clone(),
                    occlusal,
                    facial,
                    cervical: mid_facial(c, occlusal, facial),
                });
            }
        }
    }
    teeth.sort_by_key(|t| t.fdi);
    Ok(teeth)
}

/// The cervical-line vertex straight facial of the line's centre.
fn mid_facial(c: &Crown, occlusal: DVec3, facial: DVec3) -> DVec3 {
    let x = facial.cross(occlusal);
    c.cervix
        .iter()
        .map(|&v| c.mesh.positions[v as usize])
        .min_by(|p, q| {
            let angle = |p: &DVec3| {
                let d = *p - c.cervix_centre;
                d.dot(x).atan2(d.dot(facial)).abs()
            };
            angle(p).total_cmp(&angle(q))
        })
        .unwrap_or(c.cervix_centre)
}

#[derive(Serialize)]
struct ManifestOut<'a> {
    teeth: Vec<EntryOut<'a>>,
}

#[derive(Serialize)]
struct EntryOut<'a> {
    file: &'a str,
    serves: [u8; 1],
    occlusal: [f64; 3],
    facial: [f64; 3],
    cervical: [f64; 3],
    mirror: bool,
}

/// Writes one binary STL per tooth (`NN.stl`) and the `library.json` manifest into `dir`
/// (created if missing; existing files of the same names are replaced). Each tooth serves its
/// own FDI number; left teeth (quadrants 2 and 4) are marked `mirror`.
pub fn write_library(teeth: &[SetupTooth], dir: &Path) -> Result<(), LibraryError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| err(format!("cannot create {}: {e}", dir.display())))?;
    let files: Vec<String> = teeth
        .iter()
        .map(|t| format!("{}.stl", t.fdi.code()))
        .collect();
    for (t, file) in teeth.iter().zip(&files) {
        let path = dir.join(file);
        std::fs::write(&path, dentcraft_io::write_stl_binary(&t.mesh))
            .map_err(|e| err(format!("cannot write {}: {e}", path.display())))?;
    }
    let manifest = ManifestOut {
        teeth: teeth
            .iter()
            .zip(&files)
            .map(|(t, file)| EntryOut {
                file,
                serves: [t.fdi.code()],
                occlusal: t.occlusal.to_array(),
                facial: t.facial.to_array(),
                cervical: t.cervical.to_array(),
                mirror: matches!(t.fdi.quadrant(), 2 | 4),
            })
            .collect(),
    };
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| err(e.to_string()))?;
    let path = dir.join(MANIFEST);
    std::fs::write(&path, text + "\n")
        .map_err(|e| err(format!("cannot write {}: {e}", path.display())))
}

#[cfg(test)]
#[path = "setup_tests.rs"]
pub(crate) mod tests;
