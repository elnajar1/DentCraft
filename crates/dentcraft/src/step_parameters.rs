//! Explicit parameter propagation, recorded as one design edit. Placement stays per tooth.

use std::collections::BTreeSet;

use dentcraft_case::{Case, Fdi};
use dentcraft_workflow::{Session, StepId};

use crate::history::History;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scope {
    Step,
    Selection,
}

pub fn label(step: StepId) -> Option<&'static str> {
    match step {
        StepId::InnerSurface => Some("inner-surface settings"),
        StepId::Anatomy => Some("anatomy shape (keeping each tooth's placement)"),
        StepId::Occlusion => Some("contact targets"),
        _ => None,
    }
}

/// Restrict the operation to other restorations of the current step, never arbitrary teeth.
pub fn targets(
    case: &Case,
    session: &Session,
    source: Option<Fdi>,
    selected: Option<&BTreeSet<Fdi>>,
) -> Vec<Fdi> {
    let Some(step) = session.current() else {
        return Vec::new();
    };
    let Some(source) = source.filter(|t| step.teeth.contains(t) && case.unit(*t).is_some()) else {
        return Vec::new();
    };
    if label(step.id).is_none() {
        return Vec::new();
    }
    step.teeth
        .iter()
        .copied()
        .filter(|t| {
            *t != source && case.unit(*t).is_some() && selected.is_none_or(|set| set.contains(t))
        })
        .collect()
}

/// Returns the first changed tooth in workflow order, so Next reviews all affected teeth.
pub fn apply(
    case: &mut Case,
    history: &mut History,
    session: &Session,
    source: Option<Fdi>,
    selected: Option<&BTreeSet<Fdi>>,
) -> Option<Fdi> {
    let targets = targets(case, session, source, selected);
    if targets.is_empty() {
        return None;
    }
    let step = session
        .current()
        .expect("targets require a current step")
        .id;
    let source = case
        .design
        .teeth
        .get(&source.expect("targets require a source"))
        .cloned()
        .unwrap_or_default();
    let before = case.design.clone();
    let mut first_changed = None;
    for tooth in targets {
        let target = case.design.teeth.entry(tooth).or_default();
        let previous = target.clone();
        match step {
            StepId::InnerSurface => target.inner_surface = source.inner_surface,
            StepId::Anatomy => {
                let rotation = target.anatomy.rotation_deg;
                let offset = target.anatomy.offset;
                target.anatomy = source.anatomy;
                target.anatomy.rotation_deg = rotation;
                target.anatomy.offset = offset;
            }
            StepId::Occlusion => target.contacts = source.contacts,
            _ => unreachable!("targets require a supported step"),
        }
        if *target != previous && first_changed.is_none() {
            first_changed = Some(tooth);
        }
        if *target == Default::default() {
            case.design.teeth.remove(&tooth);
        }
    }
    history.record(
        &format!("Apply {} to teeth", label(step).unwrap()),
        before,
        &case.design,
        None,
    );
    first_changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_case::RestorationKind;
    use glam::DVec3;

    fn setup(step: StepId) -> (Case, Session, [Fdi; 3]) {
        let teeth = [16, 15, 14].map(|t| Fdi::new(t).unwrap());
        let mut case = Case::new("generated");
        for t in teeth {
            case.set_unit(t, RestorationKind::Crown);
        }
        let mut session = Session::for_case(&case);
        while session.current().unwrap().id != step {
            session.complete().unwrap();
        }
        (case, session, teeth)
    }

    #[test]
    fn group_copy_is_one_undo_and_preserves_geometry_and_other_settings() {
        for step in [StepId::InnerSurface, StepId::Anatomy, StepId::Occlusion] {
            let (mut case, session, [a, b, c]) = setup(step);
            let source = case.design.teeth.entry(a).or_default();
            source.inner_surface.cement_gap = Some(0.08);
            source.anatomy.height = 1.2;
            source.anatomy.offset = [2.0, 3.0, 4.0];
            source.anatomy.rotation_deg = 30.0;
            source.contacts.occlusal_clearance = 0.1;
            let target = case.design.teeth.entry(b).or_default();
            target.margin = Some(vec![DVec3::X, DVec3::Y, DVec3::Z]);
            target.insertion_axis = Some(DVec3::Z);
            target.anatomy.offset = [-1.0, 0.0, 0.0];
            target.anatomy.rotation_deg = -15.0;
            let before = case.design.clone();
            let mut history = History::default();
            apply(&mut case, &mut history, &session, Some(a), None);
            let target = &case.design.teeth[&b];
            assert_eq!(target.margin, before.teeth[&b].margin);
            assert_eq!(target.insertion_axis, Some(DVec3::Z));
            assert_eq!(target.anatomy.offset, [-1.0, 0.0, 0.0]);
            assert_eq!(target.anatomy.rotation_deg, -15.0);
            match step {
                StepId::InnerSurface => {
                    assert_eq!(target.inner_surface.cement_gap, Some(0.08));
                    assert_eq!(case.design.teeth[&c].inner_surface.cement_gap, Some(0.08));
                    assert_eq!(target.anatomy, before.teeth[&b].anatomy);
                    assert_eq!(target.contacts, before.teeth[&b].contacts);
                }
                StepId::Anatomy => {
                    assert_eq!(target.anatomy.height, 1.2);
                    assert_eq!(case.design.teeth[&c].anatomy.height, 1.2);
                    assert_eq!(target.inner_surface, before.teeth[&b].inner_surface);
                    assert_eq!(target.contacts, before.teeth[&b].contacts);
                }
                StepId::Occlusion => {
                    assert_eq!(target.contacts.occlusal_clearance, 0.1);
                    assert_eq!(case.design.teeth[&c].contacts.occlusal_clearance, 0.1);
                    assert_eq!(target.inner_surface, before.teeth[&b].inner_surface);
                    assert_eq!(target.anatomy, before.teeth[&b].anatomy);
                }
                _ => unreachable!(),
            }
            let after = case.design.clone();
            history.undo(&mut case.design).unwrap();
            assert_eq!(case.design, before);
            assert!(history.undo_label().is_none());
            history.redo(&mut case.design).unwrap();
            assert_eq!(case.design, after);
            apply(&mut case, &mut history, &session, Some(a), None);
            history.undo(&mut case.design).unwrap();
            assert_eq!(
                case.design, before,
                "reapplying identical values must not add history"
            );
        }
    }

    #[test]
    fn selected_targets_exclude_source_outsiders_and_missing_source() {
        let (mut case, session, [a, b, c]) = setup(StepId::InnerSurface);
        let outside = Fdi::new(31).unwrap();
        let selected = BTreeSet::from([a, b, outside]);
        case.design
            .teeth
            .entry(a)
            .or_default()
            .inner_surface
            .seal_gap = Some(0.02);
        let mut history = History::default();
        apply(&mut case, &mut history, &session, Some(a), Some(&selected));
        assert_eq!(case.design.teeth[&b].inner_surface.seal_gap, Some(0.02));
        assert!(!case.design.teeth.contains_key(&c));
        assert!(!case.design.teeth.contains_key(&outside));
        assert!(targets(&case, &session, None, None).is_empty());
        assert!(targets(&case, &session, Some(outside), None).is_empty());
    }

    #[test]
    fn copying_from_the_last_tooth_returns_to_the_first_changed_tooth_for_review() {
        let (mut case, session, _) = setup(StepId::InnerSurface);
        let teeth = &session.current().unwrap().teeth;
        let last = *teeth.last().unwrap();
        case.design
            .teeth
            .entry(last)
            .or_default()
            .inner_surface
            .cement_gap = Some(0.08);
        let mut history = History::default();
        let first = apply(&mut case, &mut history, &session, Some(last), None).unwrap();
        assert_eq!(first, teeth[0]);
        let mut visited = vec![first];
        while let Some(next) =
            crate::margin_step::neighbour_tooth(teeth, visited.last().copied(), true)
        {
            visited.push(next);
        }
        assert_eq!(visited, *teeth);
        assert_eq!(
            apply(&mut case, &mut history, &session, Some(last), None),
            None
        );
    }
}
