//! Transient scene selection. Object ids survive list reordering; no case geometry is edited.

use std::collections::BTreeSet;

use dentcraft_case::Fdi;
use dentcraft_mesh::{Aabb, Bvh, TriMesh};
use dentcraft_render::{Camera, ClipPlane, Overlay};
use eframe::egui::{Pos2, Rect};
use glam::{DVec3, Vec2};

use crate::design_objects::{DesignObjects, DesignPart, PartGeometry, PartKind};
use crate::objects::SceneObject;

#[derive(Clone, Copy)]
pub struct Click {
    pub pixel: Pos2,
    pub ray: (DVec3, DVec3),
    pub additive: bool,
    pub context: bool,
}

pub fn visible_parts<'a>(
    parts: &'a [DesignPart],
    design: &DesignObjects,
    owned: &[(PartKind, Fdi)],
    tool_visible: Option<(PartKind, Fdi)>,
    hidden_arch: Option<dentcraft_case::Arch>,
) -> Vec<&'a DesignPart> {
    let shown = |kind, tooth: Fdi| {
        design.shown(kind, tooth)
            && hidden_arch != Some(tooth.arch())
            && !owned.contains(&(kind, tooth))
    };
    parts
        .iter()
        .filter(|p| {
            if matches!(&p.geometry, PartGeometry::Line(points) if points.len() < 2) {
                return false;
            }
            let key = (p.kind, p.tooth);
            if owned.contains(&key) {
                return tool_visible == Some(key);
            }
            shown(p.kind, p.tooth)
                && design.opacity(p.kind) > 0.05
                && !(p.kind == PartKind::Inner
                    && parts.iter().any(|c| {
                        c.kind == PartKind::Crown && c.tooth == p.tooth && shown(c.kind, c.tooth)
                    }))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Object {
    Scan(u64),
    Part(PartKind, Fdi),
}

#[derive(Default)]
pub struct Selection {
    pub items: BTreeSet<Object>,
    pub active: Option<Object>,
}

impl Selection {
    pub fn click(&mut self, hit: Option<Object>, additive: bool, context: bool) {
        // Background context menus also keep the group available for Fit / Hide.
        if context && hit.is_none() {
            return;
        }
        // A context click within a selection preserves the group for the menu's actions.
        if context && hit.is_some_and(|h| self.items.contains(&h)) {
            self.active = hit;
            return;
        }
        if !additive {
            self.items.clear();
        }
        if let Some(hit) = hit {
            if additive && !context && !self.items.insert(hit) {
                self.items.remove(&hit);
            } else {
                self.items.insert(hit);
            }
            self.active = self
                .items
                .contains(&hit)
                .then_some(hit)
                .or_else(|| self.items.last().copied());
        } else if !additive {
            self.active = None;
        }
    }

    pub fn retain(&mut self, mut keep: impl FnMut(Object) -> bool) {
        self.items.retain(|item| keep(*item));
        if self.active.is_some_and(|a| !self.items.contains(&a)) {
            self.active = self.items.last().copied();
        }
    }

    pub fn teeth(&self) -> BTreeSet<Fdi> {
        self.items
            .iter()
            .filter_map(|o| match o {
                Object::Part(_, tooth) => Some(*tooth),
                _ => None,
            })
            .collect()
    }
}

/// View model shared by the Expert strip and context menu.
#[derive(Default)]
pub struct Actions {
    pub count: usize,
    pub apply_count: usize,
    pub edit: Option<(usize, Fdi)>,
    pub source: Option<Fdi>,
}

/// A clip plane removes its positive half-space. Start on the kept side so a removed front
/// triangle does not prevent selecting a visible back triangle. All mesh queries stay in f64.
pub fn ray_hit(
    mesh: &TriMesh,
    bvh: &Bvh,
    origin: DVec3,
    dir: DVec3,
    clip: Option<ClipPlane>,
) -> Option<DVec3> {
    let mut start = origin;
    if let Some(plane) = clip {
        let n = plane.normal.as_dvec3();
        let distance = (origin - plane.point.as_dvec3()).dot(n);
        let slope = dir.dot(n);
        if distance > 0.0 {
            if slope >= 0.0 {
                return None;
            }
            start += dir * (-distance / slope);
        }
    }
    let hit = bvh.raycast(mesh, start, dir)?.point;
    if clip.is_some_and(|p| (hit - p.point.as_dvec3()).dot(p.normal.as_dvec3()) > 1e-8) {
        return None;
    }
    Some(hit)
}

pub struct Pick<'a> {
    pub objects: &'a [SceneObject],
    /// Only parts actually drawn, including the active tool's own visible part.
    pub parts: &'a [&'a DesignPart],
    pub design: &'a DesignObjects,
    pub camera: &'a Camera,
    pub viewport: Rect,
    pub pixel: Pos2,
    pub ray: (DVec3, DVec3),
    pub clip: Option<ClipPlane>,
}

impl Pick<'_> {
    pub fn hit(&self) -> Option<Object> {
        let (origin, dir) = self.ray;
        let mut nearest = None;
        let mut distance = f64::INFINITY;
        let mut consider = |object, point: DVec3| {
            let d = point.distance_squared(origin);
            if d < distance {
                nearest = Some(object);
                distance = d;
            }
        };
        for object in self
            .objects
            .iter()
            .filter(|o| o.shown() && o.opacity > 0.05)
        {
            if let Some(loaded) = object.loaded()
                && let Some(hit) = ray_hit(&loaded.mesh, &loaded.bvh, origin, dir, self.clip)
            {
                consider(Object::Scan(object.id.0), hit);
            }
        }
        for part in self.parts {
            if let Some((mesh, bvh)) = self.design.picking_mesh(part.kind, part.tooth)
                && let Some(hit) = ray_hit(mesh, bvh, origin, dir, self.clip)
            {
                consider(Object::Part(part.kind, part.tooth), hit);
            }
        }
        // Margins are screen-space overlays (also visible through a scan). Hit-test their
        // rendered segments rather than only their control vertices.
        let min = Vec2::new(self.viewport.left(), self.viewport.top());
        let size = Vec2::new(self.viewport.width(), self.viewport.height());
        let cursor = Vec2::new(self.pixel.x, self.pixel.y);
        let mut line_distance = 6.0_f32.powi(2);
        for part in self.parts {
            if let PartGeometry::Line(points) = &part.geometry {
                if points.len() < 2 {
                    continue;
                }
                for (a, b) in points
                    .iter()
                    .zip(points.iter().cycle().skip(1))
                    .take(points.len())
                {
                    if let (Some(a), Some(b)) = (
                        self.camera.project_to_pixel(a.as_vec3(), min, size),
                        self.camera.project_to_pixel(b.as_vec3(), min, size),
                    ) {
                        let ab = b - a;
                        let t = if ab.length_squared() > 0.0 {
                            ((cursor - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let d = cursor.distance_squared(a + t * ab);
                        if d < line_distance {
                            line_distance = d;
                            nearest = Some(Object::Part(part.kind, part.tooth));
                        }
                    }
                }
            }
        }
        nearest
    }
}

pub fn bounds(
    object: Object,
    scans: &[SceneObject],
    parts: &[DesignPart],
    design: &DesignObjects,
) -> Option<Aabb> {
    match object {
        Object::Scan(id) => scans.iter().find(|o| o.id.0 == id)?.loaded()?.mesh.bounds(),
        Object::Part(kind, tooth) => {
            let part = parts.iter().find(|p| p.kind == kind && p.tooth == tooth)?;
            match &part.geometry {
                PartGeometry::Mesh(_) => design.source_mesh(kind, tooth)?.bounds(),
                PartGeometry::Line(points) => {
                    let first = *points.first()?;
                    let (min, max) = points
                        .iter()
                        .fold((first, first), |(min, max), p| (min.min(*p), max.max(*p)));
                    Some(Aabb { min, max })
                }
            }
        }
    }
}

pub fn outline(bounds: Aabb, color: [f32; 3]) -> Vec<Overlay> {
    let corners: Vec<_> = (0..8)
        .map(|i| {
            DVec3::new(
                if i & 1 == 0 {
                    bounds.min.x
                } else {
                    bounds.max.x
                },
                if i & 2 == 0 {
                    bounds.min.y
                } else {
                    bounds.max.y
                },
                if i & 4 == 0 {
                    bounds.min.z
                } else {
                    bounds.max.z
                },
            )
        })
        .collect();
    let mut lines = Vec::new();
    for i in 0..8 {
        for axis in [1, 2, 4] {
            if i & axis == 0 {
                lines.push(Overlay::Polyline {
                    points: vec![corners[i].as_vec3(), corners[i | axis].as_vec3()],
                    closed: false,
                    color,
                    width_px: 1.5,
                });
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_visibility_follows_hidden_groups_arches_and_tool_owned_parts() {
        let upper = Fdi::new(16).unwrap();
        let lower = Fdi::new(46).unwrap();
        let parts: Vec<_> = [upper, lower]
            .into_iter()
            .flat_map(|tooth| {
                PartKind::ALL.map(|kind| DesignPart {
                    kind,
                    tooth,
                    geometry: PartGeometry::Line(vec![DVec3::ZERO, DVec3::X]),
                })
            })
            .collect();
        let mut design = DesignObjects::default();
        let visible = visible_parts(
            &parts,
            &design,
            &[],
            None,
            Some(dentcraft_case::Arch::Lower),
        );
        assert_eq!(
            visible
                .iter()
                .map(|p| (p.kind, p.tooth))
                .collect::<Vec<_>>(),
            [(PartKind::Margin, upper), (PartKind::Crown, upper)]
        );
        design.set_group(PartKind::Crown, false);
        let visible = visible_parts(&parts, &design, &[], None, None);
        assert!(visible.iter().all(|p| p.kind != PartKind::Crown));
        assert!(visible.iter().any(|p| p.kind == PartKind::Inner));
        let owned = [(PartKind::Inner, upper), (PartKind::Crown, upper)];
        let visible = visible_parts(
            &parts,
            &design,
            &owned,
            Some((PartKind::Crown, upper)),
            None,
        );
        assert!(
            visible
                .iter()
                .any(|p| p.kind == PartKind::Crown && p.tooth == upper)
        );
        assert!(
            !visible
                .iter()
                .any(|p| p.kind == PartKind::Inner && p.tooth == upper)
        );
    }

    #[test]
    fn margin_picking_hits_segment_interiors_and_handles_empty_and_duplicate_points() {
        let mut camera = Camera::default();
        camera.set_view(dentcraft_render::ViewPreset::Occlusal, None);
        let viewport = Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(960.0, 600.0));
        camera.aspect = viewport.width() / viewport.height();
        let size = Vec2::new(viewport.width(), viewport.height());
        let pixel = camera
            .project_to_pixel(glam::Vec3::ZERO, Vec2::ZERO, size)
            .unwrap();
        let tooth = Fdi::new(16).unwrap();
        let design = DesignObjects::default();
        for points in [
            vec![],
            vec![-DVec3::X * 8.0, DVec3::X * 8.0],
            vec![DVec3::ZERO; 3],
        ] {
            let empty = points.is_empty();
            let part = DesignPart {
                kind: PartKind::Margin,
                tooth,
                geometry: PartGeometry::Line(points),
            };
            let parts = [&part];
            let pick = Pick {
                objects: &[],
                parts: &parts,
                design: &design,
                camera: &camera,
                viewport,
                pixel: Pos2::new(pixel.x, pixel.y),
                ray: (10.0 * DVec3::Z, -DVec3::Z),
                clip: None,
            };
            assert_eq!(
                pick.hit(),
                (!empty).then_some(Object::Part(PartKind::Margin, tooth))
            );
            assert_eq!(
                bounds(
                    Object::Part(PartKind::Margin, tooth),
                    &[],
                    std::slice::from_ref(&part),
                    &design
                )
                .is_none(),
                empty
            );
        }
    }

    #[test]
    fn nearest_visible_scan_wins_and_hidden_or_transparent_scans_are_not_selected() {
        use crate::loader::Loaded;
        use crate::objects::ObjectState;
        use dentcraft_render::{MeshData, MeshId};
        use std::sync::Arc;
        let make = |id, z| {
            let mesh = TriMesh::new(
                vec![
                    DVec3::new(-1.0, -1.0, z),
                    DVec3::new(1.0, -1.0, z),
                    DVec3::new(0.0, 1.0, z),
                ],
                vec![[0, 1, 2]],
            );
            SceneObject {
                id: MeshId(id),
                name: String::new(),
                path: Default::default(),
                role: dentcraft_case::ScanRole::Other,
                color: [255; 3],
                visible: true,
                opacity: 1.0,
                auto_hidden: false,
                state: ObjectState::Ready(Box::new(Loaded {
                    bvh: Bvh::build(&mesh),
                    render: Arc::new(MeshData::from_mesh(&mesh)),
                    mesh,
                })),
            }
        };
        let mut objects = [make(1, 1.0), make(2, 0.0)];
        let camera = Camera::default();
        let design = DesignObjects::default();
        let hit = |objects: &[SceneObject]| {
            Pick {
                objects,
                parts: &[],
                design: &design,
                camera: &camera,
                viewport: Rect::EVERYTHING,
                pixel: Pos2::ZERO,
                ray: (3.0 * DVec3::Z, -DVec3::Z),
                clip: None,
            }
            .hit()
        };
        assert_eq!(hit(&objects), Some(Object::Scan(1)));
        objects[0].auto_hidden = true;
        assert_eq!(hit(&objects), Some(Object::Scan(2)));
        objects[1].opacity = 0.0;
        assert_eq!(hit(&objects), None);
    }

    #[test]
    fn ctrl_toggles_and_context_preserves_the_group_then_prunes_stale_ids() {
        let mut s = Selection::default();
        s.click(Some(Object::Scan(1)), false, false);
        s.click(Some(Object::Scan(2)), true, false);
        s.click(Some(Object::Scan(1)), false, true);
        assert_eq!(s.items.len(), 2);
        s.click(None, false, true);
        assert_eq!(s.items.len(), 2);
        s.click(None, true, false);
        assert_eq!(s.items.len(), 2);
        s.click(Some(Object::Scan(1)), true, false);
        assert_eq!(s.items, BTreeSet::from([Object::Scan(2)]));
        s.retain(|_| false);
        assert!(s.active.is_none());
        s.click(Some(Object::Scan(3)), false, false);
        s.click(None, false, false);
        assert!(s.items.is_empty());
    }

    #[test]
    fn selection_deduplicates_teeth_across_parts() {
        let t = Fdi::new(16).unwrap();
        let mut s = Selection::default();
        for kind in PartKind::ALL {
            s.click(Some(Object::Part(kind, t)), true, false);
        }
        assert_eq!(s.teeth(), BTreeSet::from([t]));
    }

    #[test]
    fn ray_picking_handles_clipped_fronts_and_degenerate_open_meshes() {
        let front = vec![
            DVec3::new(-1.0, -1.0, 1.0),
            DVec3::new(1.0, -1.0, 1.0),
            DVec3::new(0.0, 1.0, 1.0),
        ];
        let mut vertices = front.clone();
        vertices.extend(front.iter().map(|p| *p - 2.0 * DVec3::Z));
        let mesh = TriMesh::new(vertices, vec![[0, 1, 2], [3, 4, 5], [0, 0, 0]]);
        let bvh = Bvh::build(&mesh);
        let eye = 3.0 * DVec3::Z;
        assert_eq!(ray_hit(&mesh, &bvh, eye, -DVec3::Z, None), Some(DVec3::Z));
        let clip = Some(ClipPlane {
            point: glam::Vec3::ZERO,
            normal: glam::Vec3::Z,
        });
        assert_eq!(ray_hit(&mesh, &bvh, eye, -DVec3::Z, clip), Some(-DVec3::Z));
        assert!(ray_hit(&mesh, &bvh, eye, DVec3::Z, clip).is_none());
        for mesh in [
            TriMesh::default(),
            TriMesh::new(vec![DVec3::ZERO; 3], vec![[0, 1, 2]]),
        ] {
            assert!(ray_hit(&mesh, &Bvh::build(&mesh), eye, -DVec3::Z, None).is_none());
        }
    }
}
