//! The designed parts shown in the viewport as objects: every tooth's margin, inner surface and
//! crown, whatever step is active, with visibility per group and per tooth and a transparency
//! per group. The active step's tool still draws its own tooth's part (handles, colour maps);
//! that part is left out here. Also the rule that hides the opposite jaw seen from behind.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use dentcraft_case::{Arch, Fdi, ScanRole};
use dentcraft_mesh::{Bvh, TriMesh};
use dentcraft_render::{Camera, DrawItem, MeshData, MeshId, Overlay};
use glam::{DVec3, Vec3};

use crate::theme::Palette;
use crate::tools::rgb;

/// Renderer ids of design parts start here, clear of the scans' and the tools' ids.
const FIRST_ID: u64 = 1 << 48;
/// The opposite jaw hides once the view direction is this close to looking at its back.
const FROM_BEHIND: f32 = 0.5;

/// A kind of designed part, one group in the Objects panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PartKind {
    Margin,
    Inner,
    Crown,
}

impl PartKind {
    pub const ALL: [Self; 3] = [Self::Margin, Self::Inner, Self::Crown];

    /// Name of the group.
    pub fn label(self) -> &'static str {
        match self {
            Self::Margin => "Margins",
            Self::Inner => "Inner surfaces",
            Self::Crown => "Crowns",
        }
    }

    /// Name of one part.
    pub fn item(self, tooth: Fdi) -> String {
        match self {
            Self::Margin => format!("Margin {tooth}"),
            Self::Inner => format!("Inner surface {tooth}"),
            Self::Crown => format!("Crown {tooth}"),
        }
    }

    /// Whether the group has a transparency: margins are lines drawn over the scans.
    pub fn has_opacity(self) -> bool {
        self != Self::Margin
    }

    fn index(self) -> usize {
        self as usize
    }

    fn mesh_id(self, tooth: Fdi) -> MeshId {
        MeshId(FIRST_ID + 256 * self.index() as u64 + u64::from(tooth.code()))
    }
}

/// The geometry of one designed part.
pub enum PartGeometry {
    /// A closed curve on the scan.
    Line(Vec<DVec3>),
    Mesh(Arc<MeshData>),
}

/// One designed part of one tooth.
pub struct DesignPart {
    pub kind: PartKind,
    pub tooth: Fdi,
    pub geometry: PartGeometry,
}

/// Visibility and look of one group.
#[derive(Clone, Debug)]
struct Group {
    on: bool,
    opacity: f32,
    /// Teeth hidden one by one while the group is on.
    hidden: BTreeSet<Fdi>,
}

/// The last mesh built from a part's source, rebuilt when the source changes.
struct Cached<T> {
    source: Arc<T>,
    mesh: Arc<MeshData>,
    /// Built on the first object-selection click, reused until the source changes.
    picking: OnceLock<Bvh>,
}

struct MeshCache<T>(BTreeMap<Fdi, Cached<T>>);

impl<T> Default for MeshCache<T> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<T> MeshCache<T> {
    fn get(
        &mut self,
        tooth: Fdi,
        source: &Arc<T>,
        mesh: impl FnOnce(&T) -> &TriMesh,
    ) -> Arc<MeshData> {
        match self.0.get(&tooth) {
            Some(c) if Arc::ptr_eq(&c.source, source) => c.mesh.clone(),
            _ => {
                let data = Arc::new(MeshData::from_mesh(mesh(source)));
                self.0.insert(
                    tooth,
                    Cached {
                        source: source.clone(),
                        mesh: data.clone(),
                        picking: OnceLock::new(),
                    },
                );
                data
            }
        }
    }
}

/// The user's choices for the design groups, and the meshes drawn for them.
pub struct DesignObjects {
    groups: [Group; 3],
    /// Hide the opposite jaw when the view looks at it from behind.
    pub auto_hide: bool,
    inner: MeshCache<dentcraft_design::InnerSurface>,
    crowns: MeshCache<dentcraft_design::Crown>,
}

impl Default for DesignObjects {
    fn default() -> Self {
        let group = |opacity| Group {
            on: true,
            opacity,
            hidden: BTreeSet::new(),
        };
        Self {
            groups: [group(1.0), group(0.65), group(1.0)],
            auto_hide: false,
            inner: MeshCache::default(),
            crowns: MeshCache::default(),
        }
    }
}

impl DesignObjects {
    /// Original f64 geometry; picking never reconstructs geometry from render vertices.
    pub fn source_mesh(&self, kind: PartKind, tooth: Fdi) -> Option<&TriMesh> {
        match kind {
            PartKind::Inner => self.inner.0.get(&tooth).map(|c| &c.source.mesh),
            PartKind::Crown => self.crowns.0.get(&tooth).map(|c| &c.source.mesh),
            PartKind::Margin => None,
        }
    }

    pub fn picking_mesh(&self, kind: PartKind, tooth: Fdi) -> Option<(&TriMesh, &Bvh)> {
        let (mesh, cache) = match kind {
            PartKind::Inner => {
                let c = self.inner.0.get(&tooth)?;
                (&c.source.mesh, &c.picking)
            }
            PartKind::Crown => {
                let c = self.crowns.0.get(&tooth)?;
                (&c.source.mesh, &c.picking)
            }
            PartKind::Margin => return None,
        };
        Some((mesh, cache.get_or_init(|| Bvh::build(mesh))))
    }

    /// The mesh of `tooth`'s inner surface.
    pub fn inner_mesh(
        &mut self,
        tooth: Fdi,
        s: &Arc<dentcraft_design::InnerSurface>,
    ) -> Arc<MeshData> {
        self.inner.get(tooth, s, |s| &s.mesh)
    }

    /// The mesh of `tooth`'s crown.
    pub fn crown_mesh(&mut self, tooth: Fdi, c: &Arc<dentcraft_design::Crown>) -> Arc<MeshData> {
        self.crowns.get(tooth, c, |c| &c.mesh)
    }

    /// Drops the meshes of parts that are gone.
    pub fn prune(&mut self, parts: &[DesignPart]) {
        let has = |kind, t: &Fdi| parts.iter().any(|p| p.kind == kind && p.tooth == *t);
        self.inner.0.retain(|t, _| has(PartKind::Inner, t));
        self.crowns.0.retain(|t, _| has(PartKind::Crown, t));
    }

    /// Shows or hides the whole group; showing it also shows its teeth hidden one by one.
    pub fn set_group(&mut self, kind: PartKind, on: bool) {
        let g = &mut self.groups[kind.index()];
        g.on = on;
        if on {
            g.hidden.clear();
        }
    }

    /// Whether the user shows `tooth`'s part of `kind`.
    pub fn shown(&self, kind: PartKind, tooth: Fdi) -> bool {
        let g = &self.groups[kind.index()];
        g.on && !g.hidden.contains(&tooth)
    }

    /// Shows or hides one part. Showing a part of a hidden group shows that part alone:
    /// `teeth` are the group's parts, the others stay hidden.
    pub fn set_shown(&mut self, kind: PartKind, tooth: Fdi, teeth: &[Fdi], on: bool) {
        let g = &mut self.groups[kind.index()];
        if on {
            if !g.on {
                g.on = true;
                g.hidden = teeth.iter().copied().filter(|t| *t != tooth).collect();
            }
            g.hidden.remove(&tooth);
        } else {
            g.hidden.insert(tooth);
        }
    }

    pub fn opacity(&self, kind: PartKind) -> f32 {
        self.groups[kind.index()].opacity
    }

    pub fn set_opacity(&mut self, kind: PartKind, opacity: f32) {
        self.groups[kind.index()].opacity = opacity.clamp(0.0, 1.0);
    }

    /// Shows every group and part.
    pub fn show_all(&mut self) {
        for kind in PartKind::ALL {
            self.set_group(kind, true);
        }
    }

    /// Draw items and overlays of `parts`. Parts in `owned` are drawn by the active tool and
    /// left out; parts of `hidden_arch` are hidden for this frame. A tooth's inner surface is
    /// not drawn under its shown crown, whose fitting surface it is.
    pub fn draw(
        &self,
        parts: &[DesignPart],
        palette: &Palette,
        owned: &[(PartKind, Fdi)],
        hidden_arch: Option<Arch>,
    ) -> (Vec<(DrawItem, Arc<MeshData>)>, Vec<Overlay>) {
        let shown = |kind: PartKind, tooth: Fdi| {
            self.shown(kind, tooth)
                && hidden_arch != Some(tooth.arch())
                && !owned.contains(&(kind, tooth))
        };
        let crown_shown = |tooth| {
            parts
                .iter()
                .any(|p| p.kind == PartKind::Crown && p.tooth == tooth)
                && shown(PartKind::Crown, tooth)
        };
        let mut items = Vec::new();
        let mut lines = Vec::new();
        for part in parts {
            if owned.contains(&(part.kind, part.tooth)) {
                continue;
            }
            let visible = shown(part.kind, part.tooth)
                && !(part.kind == PartKind::Inner && crown_shown(part.tooth));
            match &part.geometry {
                PartGeometry::Line(points) => {
                    if visible && points.len() >= 2 {
                        lines.push(Overlay::Polyline {
                            points: points.iter().map(|p| p.as_vec3()).collect(),
                            closed: true,
                            color: rgb(palette.accent),
                            width_px: 2.0,
                        });
                    }
                }
                PartGeometry::Mesh(mesh) => {
                    let color = match part.kind {
                        PartKind::Inner => palette.inner_surface,
                        _ => palette.crown,
                    };
                    let item = DrawItem {
                        mesh: part.kind.mesh_id(part.tooth),
                        color: rgb(color),
                        opacity: self.opacity(part.kind),
                        visible,
                        vertex_colors: false,
                        clip: false,
                    };
                    items.push((item, mesh.clone()));
                }
            }
        }
        (items, lines)
    }
}

/// The arch a scan role belongs to, for jaw scans.
pub fn role_arch(role: ScanRole) -> Option<Arch> {
    match role {
        ScanRole::Upper | ScanRole::UpperPreOp => Some(Arch::Upper),
        ScanRole::Lower | ScanRole::LowerPreOp => Some(Arch::Lower),
        _ => None,
    }
}

/// The jaw to hide for this view: the jaw opposite the one the camera follows, while the view
/// looks at the followed jaw's occlusal side, so it sees the opposite jaw from behind.
pub fn auto_hidden_arch(camera: &Camera) -> Option<Arch> {
    let occlusal = camera.frame * Vec3::Z;
    (camera.forward().dot(occlusal) < -FROM_BEHIND).then_some(if camera.upper {
        Arch::Lower
    } else {
        Arch::Upper
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn picking_uses_the_current_f64_source_and_drops_stale_geometry() {
        let tooth = Fdi::new(16).unwrap();
        let surface = |z| {
            Arc::new(dentcraft_design::InnerSurface {
                mesh: TriMesh::new(
                    vec![
                        DVec3::new(-1.0, -1.0, z),
                        DVec3::new(1.0, -1.0, z),
                        DVec3::new(0.0, 1.0, z),
                    ],
                    vec![[0, 1, 2]],
                ),
                boundary: vec![0, 1, 2],
                target_gaps: vec![0.0; 3],
                axis: DVec3::Z,
            })
        };
        let mut objects = DesignObjects::default();
        for z in [1.0000000001, 0.0000000001] {
            let source = surface(z);
            objects.inner_mesh(tooth, &source);
            let (mesh, bvh) = objects.picking_mesh(PartKind::Inner, tooth).unwrap();
            let hit = bvh.raycast(mesh, 3.0 * DVec3::Z, -DVec3::Z).unwrap();
            assert!((hit.point.z - z).abs() < 1e-14);
        }
        objects.prune(&[]);
        assert!(objects.picking_mesh(PartKind::Inner, tooth).is_none());
    }

    fn fdi(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn parts() -> Vec<DesignPart> {
        let mesh = Arc::new(MeshData::from_mesh(&TriMesh::uv_sphere(1.0, 8, 4)));
        let mut out = Vec::new();
        for t in [11, 21, 36] {
            for kind in [PartKind::Inner, PartKind::Crown] {
                out.push(DesignPart {
                    kind,
                    tooth: fdi(t),
                    geometry: PartGeometry::Mesh(mesh.clone()),
                });
            }
            out.push(DesignPart {
                kind: PartKind::Margin,
                tooth: fdi(t),
                geometry: PartGeometry::Line(vec![DVec3::ZERO, DVec3::X, DVec3::Y]),
            });
        }
        out
    }

    fn visible(items: &[(DrawItem, Arc<MeshData>)], kind: PartKind, tooth: u8) -> bool {
        items
            .iter()
            .find(|(i, _)| i.mesh == kind.mesh_id(fdi(tooth)))
            .is_some_and(|(i, _)| i.visible)
    }

    #[test]
    fn every_tooth_is_drawn_except_the_tools_own_and_inner_surfaces_under_crowns() {
        let d = DesignObjects::default();
        let owned = [(PartKind::Crown, fdi(21))];
        let (items, lines) = d.draw(&parts(), &Palette::DARK, &owned, None);
        assert!(visible(&items, PartKind::Crown, 11) && visible(&items, PartKind::Crown, 36));
        assert!(
            !items
                .iter()
                .any(|(i, _)| i.mesh == PartKind::Crown.mesh_id(fdi(21))),
            "the tool draws its own crown"
        );
        assert!(!visible(&items, PartKind::Inner, 11), "under its crown");
        assert!(
            visible(&items, PartKind::Inner, 21),
            "its crown is the tool's"
        );
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn hiding_the_crowns_group_hides_every_crown() {
        let mut d = DesignObjects::default();
        d.set_group(PartKind::Crown, false);
        let (items, lines) = d.draw(&parts(), &Palette::DARK, &[], None);
        for t in [11, 21, 36] {
            assert!(!visible(&items, PartKind::Crown, t), "crown {t}");
            assert!(
                visible(&items, PartKind::Inner, t),
                "inner {t} shows instead"
            );
        }
        assert_eq!(lines.len(), 3, "margins stay");
        // Showing one crown of the hidden group shows that one alone.
        let teeth = [fdi(11), fdi(21), fdi(36)];
        d.set_shown(PartKind::Crown, fdi(21), &teeth, true);
        let (items, _) = d.draw(&parts(), &Palette::DARK, &[], None);
        assert!(visible(&items, PartKind::Crown, 21));
        assert!(!visible(&items, PartKind::Crown, 11) && !visible(&items, PartKind::Crown, 36));
        d.show_all();
        let (items, _) = d.draw(&parts(), &Palette::DARK, &[], None);
        assert!(visible(&items, PartKind::Crown, 11) && visible(&items, PartKind::Crown, 36));
    }

    #[test]
    fn a_hidden_arch_hides_its_teeths_parts() {
        let d = DesignObjects::default();
        let (items, lines) = d.draw(&parts(), &Palette::DARK, &[], Some(Arch::Lower));
        assert!(!visible(&items, PartKind::Crown, 36));
        assert!(visible(&items, PartKind::Crown, 11));
        assert_eq!(lines.len(), 2);
    }

    /// A camera looking along `forward` in a frame whose occlusal direction is `occlusal`.
    fn camera(upper: bool, occlusal: Vec3, forward: Vec3) -> Camera {
        let mut c = Camera::default();
        c.frame = Quat::from_rotation_arc(Vec3::Z, occlusal);
        c.upper = upper;
        c.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, forward);
        c
    }

    #[test]
    fn the_lower_jaw_hides_when_seen_from_below_the_upper_jaws_occlusal_side() {
        // An upper jaw whose teeth point down (-Z).
        let down = Vec3::NEG_Z;
        assert_eq!(
            auto_hidden_arch(&camera(true, down, Vec3::Z)),
            Some(Arch::Lower),
            "from below, looking up at the upper teeth"
        );
        assert_eq!(
            auto_hidden_arch(&camera(true, down, Vec3::NEG_Z)),
            None,
            "from above"
        );
        assert_eq!(
            auto_hidden_arch(&camera(true, down, Vec3::Y)),
            None,
            "from the side"
        );
        let tilted = Vec3::new(0.0, 0.6, 0.8).normalize();
        assert_eq!(
            auto_hidden_arch(&camera(true, down, tilted)),
            Some(Arch::Lower)
        );
        // A lower jaw seen from above its teeth hides the upper jaw.
        assert_eq!(
            auto_hidden_arch(&camera(false, Vec3::Z, Vec3::NEG_Z)),
            Some(Arch::Upper)
        );
        assert_eq!(auto_hidden_arch(&camera(false, Vec3::Z, Vec3::Z)), None);
    }
}
