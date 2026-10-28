//! The objects shown in the viewport: loaded scans with their role and look.

use std::path::{Path, PathBuf};

use dentcraft_case::ScanRole;
use dentcraft_mesh::Aabb;
use dentcraft_render::{DrawItem, MeshId};
use glam::DVec3;

use crate::loader::Loaded;

/// Warm stone for jaw scans, close to dental die stone.
pub const STONE: [u8; 3] = [0xe2, 0xd6, 0xc2];

/// Distinct soft colours for further objects, in the order they are handed out.
pub const SOFT_COLORS: [[u8; 3]; 6] = [
    [0xa9, 0xc4, 0xdb], // mist blue
    [0xb7, 0xd0, 0xb0], // sage
    [0xe0, 0xb4, 0xa8], // blush
    [0xc6, 0xbb, 0xdf], // lavender
    [0xe6, 0xcf, 0x9a], // sand gold
    [0x9f, 0xcf, 0xc9], // seafoam
];

pub enum ObjectState {
    Loading,
    Ready(Box<Loaded>),
}

pub struct SceneObject {
    pub id: MeshId,
    pub name: String,
    pub path: PathBuf,
    pub role: ScanRole,
    /// sRGB.
    pub color: [u8; 3],
    pub visible: bool,
    pub opacity: f32,
    pub state: ObjectState,
}

impl SceneObject {
    pub fn loaded(&self) -> Option<&Loaded> {
        match &self.state {
            ObjectState::Ready(l) => Some(l),
            ObjectState::Loading => None,
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.loaded().map_or(0, |l| l.mesh.triangle_count())
    }

    pub fn draw_item(&self) -> DrawItem {
        DrawItem {
            mesh: self.id,
            color: self.color.map(|c| c as f32 / 255.0),
            opacity: self.opacity,
            visible: self.visible,
        }
    }
}

/// Guesses the scan role from a file name: upper/maxillary → upper jaw, lower/mandibular →
/// lower jaw, anything else → other.
pub fn guess_role(path: &Path) -> ScanRole {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.contains("upper") || name.contains("maxill") {
        ScanRole::Upper
    } else if name.contains("lower") || name.contains("mandib") {
        ScanRole::Lower
    } else {
        ScanRole::Other
    }
}

fn is_jaw(role: ScanRole) -> bool {
    matches!(
        role,
        ScanRole::Upper | ScanRole::Lower | ScanRole::UpperPreOp | ScanRole::LowerPreOp
    )
}

/// Default colour for a new object: stone for jaws, otherwise the next soft colour not yet
/// used by `existing` (cycling when all are taken).
pub fn default_color(role: ScanRole, existing: &[[u8; 3]]) -> [u8; 3] {
    if is_jaw(role) {
        return STONE;
    }
    SOFT_COLORS
        .iter()
        .find(|c| !existing.contains(c))
        .copied()
        .unwrap_or_else(|| {
            let others = existing.iter().filter(|c| **c != STONE).count();
            SOFT_COLORS[others % SOFT_COLORS.len()]
        })
}

/// Smallest box containing every given box.
pub fn union_bounds(boxes: impl IntoIterator<Item = Aabb>) -> Option<Aabb> {
    boxes.into_iter().reduce(|a, b| Aabb {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    })
}

/// Display name of a file: its name without directories.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// First hit of a ray against the visible, loaded objects: (object index, point).
pub fn pick(objects: &[SceneObject], origin: DVec3, dir: DVec3) -> Option<(usize, DVec3)> {
    objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.visible && o.opacity > 0.05)
        .filter_map(|(i, o)| {
            let l = o.loaded()?;
            l.bvh.raycast(&l.mesh, origin, dir).map(|hit| (i, hit))
        })
        .min_by(|a, b| a.1.t.total_cmp(&b.1.t))
        .map(|(i, hit)| (i, hit.point))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_mesh::{Bvh, TriMesh};
    use dentcraft_render::MeshData;
    use std::sync::Arc;

    #[test]
    fn role_is_guessed_from_the_file_name() {
        assert_eq!(
            guess_role(Path::new("F:/scans/Upper moler.stl")),
            ScanRole::Upper
        );
        assert_eq!(guess_role(Path::new("maxillary_arch.ply")), ScanRole::Upper);
        assert_eq!(guess_role(Path::new("LOWER.obj")), ScanRole::Lower);
        assert_eq!(guess_role(Path::new("mandibular.stl")), ScanRole::Lower);
        assert_eq!(guess_role(Path::new("central.stl")), ScanRole::Other);
        // Only the file name counts, not the folder.
        assert_eq!(guess_role(Path::new("upper/central.stl")), ScanRole::Other);
    }

    #[test]
    fn jaws_are_stone_and_others_get_distinct_soft_colours() {
        assert_eq!(default_color(ScanRole::Upper, &[]), STONE);
        assert_eq!(
            default_color(ScanRole::LowerPreOp, &[SOFT_COLORS[0]]),
            STONE
        );
        let first = default_color(ScanRole::Other, &[STONE]);
        let second = default_color(ScanRole::Other, &[STONE, first]);
        assert_ne!(first, STONE);
        assert_ne!(first, second);
        // Once every soft colour is taken, they repeat rather than fail.
        let all: Vec<_> = SOFT_COLORS.to_vec();
        assert!(SOFT_COLORS.contains(&default_color(ScanRole::Bite, &all)));
    }

    #[test]
    fn union_of_boxes() {
        let a = Aabb {
            min: DVec3::new(0.0, 0.0, 0.0),
            max: DVec3::new(1.0, 1.0, 1.0),
        };
        let b = Aabb {
            min: DVec3::new(-2.0, 0.5, 0.5),
            max: DVec3::new(0.5, 3.0, 0.75),
        };
        let u = union_bounds([a, b]).unwrap();
        assert_eq!(u.min, DVec3::new(-2.0, 0.0, 0.0));
        assert_eq!(u.max, DVec3::new(1.0, 3.0, 1.0));
        assert!(union_bounds(std::iter::empty()).is_none());
    }

    fn sphere_object(id: u64, center: DVec3, visible: bool) -> SceneObject {
        let mut mesh = TriMesh::uv_sphere(1.0, 16, 8);
        for p in &mut mesh.positions {
            *p += center;
        }
        let loaded = Loaded {
            bvh: Bvh::build(&mesh),
            render: Arc::new(MeshData::from_mesh(&mesh)),
            mesh,
        };
        SceneObject {
            id: MeshId(id),
            name: format!("sphere {id}"),
            path: PathBuf::new(),
            role: ScanRole::Other,
            color: STONE,
            visible,
            opacity: 1.0,
            state: ObjectState::Ready(Box::new(loaded)),
        }
    }

    #[test]
    fn pick_returns_the_nearest_visible_object() {
        let objects = vec![
            sphere_object(1, DVec3::new(0.0, 0.0, 0.0), true),
            sphere_object(2, DVec3::new(0.0, 0.0, 5.0), true),
            sphere_object(3, DVec3::new(0.0, 0.0, 10.0), false),
        ];
        let (index, point) = pick(&objects, DVec3::new(0.0, 0.0, 20.0), DVec3::NEG_Z).unwrap();
        assert_eq!(index, 1);
        assert!((point.z - 6.0).abs() < 0.05, "hit at {point}");
        assert!(pick(&objects, DVec3::new(9.0, 0.0, 20.0), DVec3::NEG_Z).is_none());
    }
}
