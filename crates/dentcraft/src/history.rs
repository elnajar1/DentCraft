//! Undo and redo of design edits: a stack of design snapshots taken before and after each edit.
//!
//! The case's dirty flag needs no bookkeeping here: [`crate::document::Document`] compares the
//! whole case, design included, with the saved one, so undoing back to the saved design makes
//! the case clean again.

use dentcraft_case::Design;

/// Edits kept by default; the oldest are dropped beyond this.
pub const DEFAULT_CAP: usize = 200;

/// Identifies one continuous edit, such as a handle drag. Successive records with the same
/// group merge into one undo step until [`History::end_group`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditGroup(pub u64);

#[derive(Clone, Debug)]
struct Entry {
    label: String,
    before: Design,
    after: Design,
    /// Still open for coalescing.
    group: Option<EditGroup>,
}

#[derive(Clone, Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    cap: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cap: cap.max(1),
        }
    }

    /// Records an edit that changed the design from `before` to `after`, named for the Undo
    /// tooltip ("Move margin point"). An edit in the same open `group` as the last one extends
    /// it instead of adding a step, and takes its name. Any edit clears the redo stack; an edit that changed
    /// nothing is ignored.
    pub fn record(
        &mut self,
        label: &str,
        before: Design,
        after: &Design,
        group: Option<EditGroup>,
    ) {
        if group.is_some()
            && let Some(last) = self.undo.last_mut()
            && last.group == group
        {
            last.after = after.clone();
            label.clone_into(&mut last.label);
            self.redo.clear();
            return;
        }
        if before == *after {
            return;
        }
        self.end_group();
        self.redo.clear();
        self.undo.push(Entry {
            label: label.to_owned(),
            before,
            after: after.clone(),
            group,
        });
        if self.undo.len() > self.cap {
            let excess = self.undo.len() - self.cap;
            self.undo.drain(..excess);
        }
    }

    /// Closes the open group, so the next edit is a step of its own (call on drag release).
    pub fn end_group(&mut self) {
        if let Some(last) = self.undo.last_mut() {
            last.group = None;
        }
    }

    /// Restores the design from before the last edit. Returns the undone edit's name.
    pub fn undo(&mut self, design: &mut Design) -> Option<String> {
        let mut entry = self.undo.pop()?;
        entry.group = None;
        *design = entry.before.clone();
        let label = entry.label.clone();
        self.redo.push(entry);
        Some(label)
    }

    /// Re-applies the last undone edit. Returns its name.
    pub fn redo(&mut self, design: &mut Design) -> Option<String> {
        let entry = self.redo.pop()?;
        *design = entry.after.clone();
        let label = entry.label.clone();
        self.undo.push(entry);
        Some(label)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use dentcraft_case::{Case, Fdi};
    use glam::DVec3;

    /// A design whose tooth 14 margin is the given points along x.
    fn margin(xs: &[f64]) -> Design {
        let mut d = Design::default();
        d.teeth.entry(Fdi::new(14).unwrap()).or_default().margin =
            Some(xs.iter().map(|&x| DVec3::new(x, 0.0, 0.0)).collect());
        d
    }

    /// Applies an edit to `design` and records it.
    fn edit(h: &mut History, design: &mut Design, to: Design, group: Option<EditGroup>) {
        let before = std::mem::replace(design, to);
        h.record("Move margin point", before, design, group);
    }

    #[test]
    fn undo_and_redo_walk_the_snapshots() {
        let mut h = History::default();
        let mut d = Design::default();
        assert_eq!(h.undo(&mut d), None, "nothing to undo yet");
        edit(&mut h, &mut d, margin(&[1.0]), None);
        edit(&mut h, &mut d, margin(&[1.0, 2.0]), None);
        assert_eq!(h.undo_label(), Some("Move margin point"));
        assert_eq!(h.redo_label(), None);

        assert_eq!(h.undo(&mut d).as_deref(), Some("Move margin point"));
        assert_eq!(d, margin(&[1.0]));
        h.undo(&mut d);
        assert_eq!(d, Design::default());
        assert_eq!(h.undo(&mut d), None);

        h.redo(&mut d);
        assert_eq!(d, margin(&[1.0]));
        h.redo(&mut d);
        assert_eq!(d, margin(&[1.0, 2.0]));
        assert_eq!(h.redo(&mut d), None);
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut h = History::default();
        let mut d = Design::default();
        edit(&mut h, &mut d, margin(&[1.0]), None);
        h.undo(&mut d);
        assert!(h.redo_label().is_some());
        edit(&mut h, &mut d, margin(&[5.0]), None);
        assert_eq!(h.redo_label(), None);
        assert_eq!(h.redo(&mut d), None);
        assert_eq!(d, margin(&[5.0]));
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut h = History::default();
        let mut d = margin(&[0.0]);
        let drag = Some(EditGroup(1));
        for x in [0.1, 0.2, 0.3, 0.4] {
            edit(&mut h, &mut d, margin(&[x]), drag);
        }
        h.end_group();
        // The next drag of the same handle is a new step.
        edit(&mut h, &mut d, margin(&[0.9]), drag);
        h.undo(&mut d);
        assert_eq!(d, margin(&[0.4]));
        h.undo(&mut d);
        assert_eq!(d, margin(&[0.0]), "the whole first drag is undone at once");
        assert_eq!(h.undo_label(), None);
    }

    #[test]
    fn a_different_group_or_ungrouped_edit_starts_a_new_step() {
        let mut h = History::default();
        let mut d = Design::default();
        edit(&mut h, &mut d, margin(&[1.0]), Some(EditGroup(1)));
        edit(&mut h, &mut d, margin(&[2.0]), Some(EditGroup(2)));
        edit(&mut h, &mut d, margin(&[3.0]), None);
        let mut steps = 0;
        while h.undo(&mut d).is_some() {
            steps += 1;
        }
        assert_eq!(steps, 3);
        assert_eq!(d, Design::default());
    }

    #[test]
    fn edits_that_change_nothing_are_not_recorded() {
        let mut h = History::default();
        let mut d = margin(&[1.0]);
        edit(&mut h, &mut d, margin(&[1.0]), None);
        assert_eq!(h.undo_label(), None);
    }

    #[test]
    fn the_oldest_edits_fall_off_beyond_the_cap() {
        let mut h = History::new(3);
        let mut d = Design::default();
        for x in 1..=5 {
            edit(&mut h, &mut d, margin(&[x as f64]), None);
        }
        let mut steps = 0;
        while h.undo(&mut d).is_some() {
            steps += 1;
        }
        assert_eq!(steps, 3);
        assert_eq!(d, margin(&[2.0]), "edits 1 and 2 can no longer be undone");
    }

    #[test]
    fn undoing_back_to_the_saved_design_makes_the_case_clean() {
        let mut case = Case::new("bridge");
        let doc = Document::untitled(&case);
        let mut h = History::default();
        edit(&mut h, &mut case.design, margin(&[1.0]), None);
        assert!(doc.is_dirty(&case), "a design edit dirties the case");
        h.undo(&mut case.design);
        assert!(!doc.is_dirty(&case), "undoing it cleans the case again");
        h.redo(&mut case.design);
        assert!(doc.is_dirty(&case), "redoing dirties it again");
    }
}
