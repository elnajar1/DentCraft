//! Framing the tooth being designed: the camera bounds and the once-per-entry trigger.

use dentcraft_case::Fdi;
use dentcraft_mesh::Aabb;
use dentcraft_workflow::StepId;
use glam::DVec3;

use crate::objects;

/// Room kept around the margin, in millimetres.
pub const PADDING: f64 = 5.0;

/// Whether `step` designs one tooth at a time.
pub fn is_tooth_step(step: StepId) -> bool {
    matches!(
        step,
        StepId::Margin
            | StepId::InsertionAxis
            | StepId::InnerSurface
            | StepId::Anatomy
            | StepId::Occlusion
    )
}

/// The box to frame for a tooth: the margin grown by [`PADDING`], joined with the crown's
/// bounds when there is one. `None` without a margin.
pub fn frame_bounds(margin: &[DVec3], crown: Option<Aabb>) -> Option<Aabb> {
    let first = *margin.first()?;
    let (min, max) = margin
        .iter()
        .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let grown = Aabb {
        min: min - DVec3::splat(PADDING),
        max: max + DVec3::splat(PADDING),
    };
    objects::union_bounds(std::iter::once(grown).chain(crown))
}

/// Remembers which (step, tooth) still has to be framed, so the camera moves once per entry.
#[derive(Default)]
pub struct FrameTrigger {
    entry: Option<(StepId, Fdi)>,
    pending: bool,
}

impl FrameTrigger {
    /// Call every frame with the current entry (`None` outside a tooth step) and whether the
    /// tooth has a margin. True exactly once per entry, on the first frame with a margin.
    pub fn due(&mut self, entry: Option<(StepId, Fdi)>, has_margin: bool) -> bool {
        if entry != self.entry {
            self.entry = entry;
            self.pending = entry.is_some();
        }
        if self.pending && has_margin {
            self.pending = false;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tooth(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    #[test]
    fn margin_is_grown_and_joined_with_crown() {
        let ring = [DVec3::new(0.0, 0.0, 0.0), DVec3::new(6.0, 4.0, 1.0)];
        let b = frame_bounds(&ring, None).unwrap();
        assert_eq!(b.min, DVec3::new(-5.0, -5.0, -5.0));
        assert_eq!(b.max, DVec3::new(11.0, 9.0, 6.0));
        let crown = Aabb {
            min: DVec3::new(1.0, 1.0, 0.0),
            max: DVec3::new(5.0, 3.0, 12.0),
        };
        let b = frame_bounds(&ring, Some(crown)).unwrap();
        assert_eq!(b.max.z, 12.0);
        assert_eq!(b.min.x, -5.0);
        assert!(frame_bounds(&[], Some(crown)).is_none());
    }

    #[test]
    fn frames_once_per_entry_and_waits_for_the_margin() {
        let mut t = FrameTrigger::default();
        let a = Some((StepId::Margin, tooth(33)));
        assert!(!t.due(a, false));
        assert!(!t.due(a, false));
        assert!(t.due(a, true), "the margin appeared");
        assert!(!t.due(a, true), "not again while the user orbits");
        let b = Some((StepId::Margin, tooth(34)));
        assert!(t.due(b, true), "another tooth");
        assert!(
            t.due(Some((StepId::Anatomy, tooth(34))), true),
            "another step"
        );
        assert!(!t.due(None, true));
        assert!(t.due(b, true), "re-entering counts as a new entry");
    }
}
