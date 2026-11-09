//! Conservative triangle overlap predicate for geometric diagnostics.
//!
//! Uses separating axes for two triangles, including in-plane axes for coplanar
//! triangles. Touching and gaps within the numerical tolerance are candidates,
//! not proof of an invalid mesh. See D. Eberly, *Intersection of Convex Objects:
//! The Method of Separating Axes*, section on triangle-triangle intersection.

use crate::{Bvh, TriMesh};
use glam::DVec3;

/// Result of a conservative diagnostic. A candidate can be a contact or a
/// numerical near-contact. `NoneFound` is not a self-intersection certificate:
/// triangle pairs sharing a vertex are deliberately excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonAdjacentOverlap {
    NoneFound,
    Candidate(u32, u32),
    Invalid,
}

/// Find an unambiguous edge-through-face crossing between triangles that do
/// not share a vertex. This deliberately misses coplanar overlaps, touching,
/// and crossings involving adjacent faces; `NoneFound` is not certification.
/// `min_plane_distance` is a positive threshold in the mesh's units.
pub fn find_strict_nonadjacent_crossing(
    mesh: &TriMesh,
    min_plane_distance: f64,
) -> NonAdjacentOverlap {
    if !min_plane_distance.is_finite() || min_plane_distance <= 0.0 || !valid_mesh(mesh) {
        return NonAdjacentOverlap::Invalid;
    }
    let tree = Bvh::build(mesh);
    let pair = tree.find_overlapping_pair(mesh, |i, j| {
        let (a, b) = (mesh.triangles[i as usize], mesh.triangles[j as usize]);
        if a.iter().any(|v| b.contains(v)) {
            return false;
        }
        let (a, b) = (mesh.triangle(i as usize), mesh.triangle(j as usize));
        strict_edge_crosses(a, b, min_plane_distance)
            || strict_edge_crosses(b, a, min_plane_distance)
    });
    pair.map_or(NonAdjacentOverlap::NoneFound, |(a, b)| {
        NonAdjacentOverlap::Candidate(a, b)
    })
}

fn valid_mesh(mesh: &TriMesh) -> bool {
    mesh.triangles.iter().all(|triangle| {
        let [Some(a), Some(b), Some(c)] = triangle.map(|i| mesh.positions.get(i as usize)) else {
            return false;
        };
        triangles_may_overlap([*a, *b, *c], [*a, *b, *c]).is_some()
    })
}

fn strict_edge_crosses(source: [DVec3; 3], face: [DVec3; 3], epsilon: f64) -> bool {
    let (u, v) = (face[1] - face[0], face[2] - face[0]);
    let Some(normal) = u.cross(v).try_normalize() else {
        return false;
    };
    let distances = source.map(|p| (p - face[0]).dot(normal));
    let (uu, uv, vv) = (u.dot(u), u.dot(v), v.dot(v));
    let denominator = uu * vv - uv * uv;
    if !denominator.is_finite() || denominator <= 0.0 {
        return false;
    }
    (0..3).any(|i| {
        let j = (i + 1) % 3;
        let (a, b) = (distances[i], distances[j]);
        if !((a > epsilon && b < -epsilon) || (a < -epsilon && b > epsilon)) {
            return false;
        }
        let point = source[i] + (source[j] - source[i]) * (a / (a - b));
        let relative = point - face[0];
        let (pu, pv) = (relative.dot(u), relative.dot(v));
        let x = (vv * pu - uv * pv) / denominator;
        let y = (uu * pv - uv * pu) / denominator;
        let margin = 1e-10;
        x > margin && y > margin && x + y < 1.0 - margin
    })
}

/// Find a possible overlap between distinct triangles that share no indexed
/// vertex. The input is checked before building the acceleration structure;
/// invalid indices, non-finite coordinates and degenerate triangles yield
/// `Invalid`. Open surfaces can be inspected, but adjacency crossings are not
/// covered by this diagnostic.
pub fn find_nonadjacent_overlap(mesh: &TriMesh) -> NonAdjacentOverlap {
    if !valid_mesh(mesh) {
        return NonAdjacentOverlap::Invalid;
    }
    let bvh = Bvh::build(mesh);
    let mut invalid = false;
    let pair = bvh.find_overlapping_pair(mesh, |a, b| {
        let (x, y) = (mesh.triangles[a as usize], mesh.triangles[b as usize]);
        if x.iter().any(|v| y.contains(v)) {
            return false;
        }
        match triangles_may_overlap(mesh.triangle(a as usize), mesh.triangle(b as usize)) {
            Some(value) => value,
            None => {
                invalid = true;
                true
            }
        }
    });
    if invalid {
        NonAdjacentOverlap::Invalid
    } else if let Some((a, b)) = pair {
        NonAdjacentOverlap::Candidate(a, b)
    } else {
        NonAdjacentOverlap::NoneFound
    }
}

/// Returns `None` for non-finite or degenerate triangles; otherwise returns
/// whether two closed triangles may overlap, including touching. This is a
/// floating-point diagnostic, not an exact-predicate certification.
pub fn triangles_may_overlap(a: [DVec3; 3], b: [DVec3; 3]) -> Option<bool> {
    if a.iter().chain(&b).any(|p| !p.is_finite()) {
        return None;
    }
    // Recenter before projecting to reduce loss of precision for translated scans.
    let origin = a[0];
    let a = a.map(|p| p - origin);
    let b = b.map(|p| p - origin);
    if a.iter().chain(&b).any(|p| !p.is_finite()) {
        return None;
    }
    let edges = |t: [DVec3; 3]| [t[1] - t[0], t[2] - t[1], t[0] - t[2]];
    let (ea, eb) = (edges(a), edges(b));
    let (na, nb) = (ea[0].cross(ea[1]), eb[0].cross(eb[1]));
    let size = |edges: &[DVec3; 3]| edges.iter().map(|e| e.length()).fold(0.0_f64, f64::max);
    let (size_a, size_b) = (size(&ea), size(&eb));
    let scale = size_a.max(size_b);
    if !scale.is_finite()
        || size_a == 0.0
        || size_b == 0.0
        || na.length() <= 1e-14 * size_a * size_a
        || nb.length() <= 1e-14 * size_b * size_b
    {
        return None;
    }
    let tolerance = scale * 1e-10;
    let separated = |axis: DVec3| -> bool {
        let Some(axis) = axis.try_normalize() else {
            return false;
        };
        let bounds = |triangle: [DVec3; 3]| {
            let p = triangle.map(|p| p.dot(axis));
            (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]))
        };
        let (amin, amax) = bounds(a);
        let (bmin, bmax) = bounds(b);
        amax + tolerance < bmin || bmax + tolerance < amin
    };
    if separated(na) || separated(nb) {
        return Some(false);
    }
    for x in ea {
        for y in eb {
            if separated(x.cross(y)) {
                return Some(false);
            }
        }
    }
    // Cross-products of normals and edges separate coplanar triangles.
    for edge in ea.into_iter().chain(eb) {
        if separated(na.cross(edge)) || separated(nb.cross(edge)) {
            return Some(false);
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;

    #[test]
    fn strict_crossing_finds_interior_crossings_but_not_coplanar_contacts() {
        let a = [DVec3::ZERO, DVec3::X * 2.0, DVec3::Y * 2.0];
        let b = [
            DVec3::new(0.5, 0.5, -1.0),
            DVec3::new(0.5, 0.5, 1.0),
            DVec3::new(1.0, 0.7, 0.3),
        ];
        let touching = [
            DVec3::ZERO,
            DVec3::new(-1.0, 0.0, 0.0),
            DVec3::new(0.0, -1.0, 0.0),
        ];
        for rotation in [
            DQuat::IDENTITY,
            DQuat::from_axis_angle(DVec3::new(1.0, 3.0, -2.0).normalize(), 0.8),
        ] {
            let transform = |t: [DVec3; 3]| t.map(|p| rotation * p + DVec3::new(-42.0, 21.0, 83.0));
            let (a, b, touching) = (transform(a), transform(b), transform(touching));
            let mesh = TriMesh::new(a.into_iter().chain(b).collect(), vec![[0, 1, 2], [3, 4, 5]]);
            assert_eq!(
                find_strict_nonadjacent_crossing(&mesh, 0.0001),
                NonAdjacentOverlap::Candidate(0, 1)
            );
            let touching = TriMesh::new(
                a.into_iter().chain(touching).collect(),
                vec![[0, 1, 2], [3, 4, 5]],
            );
            assert_eq!(
                find_strict_nonadjacent_crossing(&touching, 0.0001),
                NonAdjacentOverlap::NoneFound
            );
        }
        assert_eq!(
            find_strict_nonadjacent_crossing(&TriMesh::default(), 0.0),
            NonAdjacentOverlap::Invalid
        );
        let invalid = TriMesh::new(vec![DVec3::ZERO], vec![[0, 0, 9]]);
        assert_eq!(
            find_strict_nonadjacent_crossing(&invalid, 0.001),
            NonAdjacentOverlap::Invalid
        );
    }

    #[test]
    fn mesh_diagnostic_reports_unrelated_crossings_and_invalid_input() {
        let cube = crate::tests::cube(2.0);
        assert_eq!(
            find_nonadjacent_overlap(&cube),
            NonAdjacentOverlap::NoneFound
        );
        assert_eq!(
            find_nonadjacent_overlap(&TriMesh::default()),
            NonAdjacentOverlap::NoneFound
        );
        let mesh = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X * 2.0,
                DVec3::Y * 2.0,
                DVec3::new(0.5, 0.5, -1.0),
                DVec3::new(0.5, 0.5, 1.0),
                DVec3::new(1.0, 0.5, 0.0),
            ],
            vec![[0, 1, 2], [3, 4, 5]],
        );
        assert_eq!(
            find_nonadjacent_overlap(&mesh),
            NonAdjacentOverlap::Candidate(0, 1)
        );
        let mut duplicated = TriMesh::new(
            vec![
                DVec3::ZERO,
                DVec3::X,
                DVec3::Y,
                DVec3::ZERO,
                DVec3::X,
                DVec3::Y,
            ],
            vec![[0, 1, 2], [3, 4, 5]],
        );
        assert_eq!(
            find_nonadjacent_overlap(&duplicated),
            NonAdjacentOverlap::Candidate(0, 1)
        );
        // The diagnostic deliberately excludes even overlapping triangles that share an index.
        duplicated.triangles[1] = [0, 4, 5];
        assert_eq!(
            find_nonadjacent_overlap(&duplicated),
            NonAdjacentOverlap::NoneFound
        );
        let mut invalid = mesh.clone();
        invalid.triangles[1][2] = 99;
        assert_eq!(
            find_nonadjacent_overlap(&invalid),
            NonAdjacentOverlap::Invalid
        );
        invalid = mesh.clone();
        invalid.positions[3] = DVec3::NAN;
        assert_eq!(
            find_nonadjacent_overlap(&invalid),
            NonAdjacentOverlap::Invalid
        );
        invalid = mesh;
        invalid.triangles[1] = [3, 3, 4];
        assert_eq!(
            find_nonadjacent_overlap(&invalid),
            NonAdjacentOverlap::Invalid
        );
    }

    #[test]
    fn overlap_handles_skew_coplanar_touching_and_degenerate_triangles() {
        let a = [DVec3::ZERO, DVec3::X * 2.0, DVec3::Y * 2.0];
        let crossing = [
            DVec3::new(0.5, 0.5, -1.0),
            DVec3::new(0.5, 0.5, 1.0),
            DVec3::new(1.0, 0.5, 0.0),
        ];
        let coplanar_overlap = [
            DVec3::new(0.5, 0.5, 0.0),
            DVec3::new(1.5, 0.5, 0.0),
            DVec3::new(0.5, 1.5, 0.0),
        ];
        // AABBs overlap, but the triangles do not.
        let coplanar_separate = [
            DVec3::new(1.2, 1.2, 0.0),
            DVec3::new(2.0, 1.2, 0.0),
            DVec3::new(1.2, 2.0, 0.0),
        ];
        let touching = [
            DVec3::X * 2.0,
            DVec3::new(3.0, 0.0, 0.0),
            DVec3::new(2.0, -1.0, 0.0),
        ];
        for rotation in [
            DQuat::IDENTITY,
            DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 0.73),
        ] {
            let transform = |t: [DVec3; 3]| t.map(|p| rotation * p + DVec3::new(71.0, -29.0, 13.0));
            for (other, expected) in [
                (crossing, true),
                (coplanar_overlap, true),
                (coplanar_separate, false),
                (touching, true),
            ] {
                assert_eq!(
                    triangles_may_overlap(transform(a), transform(other)),
                    Some(expected)
                );
                assert_eq!(
                    triangles_may_overlap(transform(other), transform(a)),
                    Some(expected)
                );
            }
        }
        assert_eq!(triangles_may_overlap(a, [DVec3::ZERO; 3]), None);
        assert_eq!(
            triangles_may_overlap(a, [DVec3::NAN, DVec3::X, DVec3::Y]),
            None
        );
    }
}
