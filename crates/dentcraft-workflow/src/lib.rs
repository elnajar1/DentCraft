//! The design wizard: which steps a case goes through, in which order, and where the user is.
//!
//! A case's plan is the ordered union of the steps each of its units needs, framed by the steps
//! every case shares (import scans first, finish and export last). Each planned step lists the
//! teeth it applies to, so a step such as the margin line is done tooth by tooth.

use dentcraft_case::{Case, Fdi, RestorationKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepId {
    Scans,
    Orient,
    Margin,
    InsertionAxis,
    InnerSurface,
    Anatomy,
    PonticBase,
    Occlusion,
    Sculpt,
    Connectors,
    Thickness,
    Finish,
}

impl StepId {
    /// Every step in the order a plan runs them.
    pub const ALL: [StepId; 12] = [
        Self::Scans,
        Self::Orient,
        Self::Margin,
        Self::InsertionAxis,
        Self::InnerSurface,
        Self::Anatomy,
        Self::PonticBase,
        Self::Occlusion,
        Self::Sculpt,
        Self::Connectors,
        Self::Thickness,
        Self::Finish,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Scans => "Scans",
            Self::Orient => "Orientation",
            Self::Margin => "Margin line",
            Self::InsertionAxis => "Insertion axis",
            Self::InnerSurface => "Inner surface",
            Self::Anatomy => "Anatomy",
            Self::PonticBase => "Pontic base",
            Self::Occlusion => "Occlusion & contacts",
            Self::Sculpt => "Sculpt",
            Self::Connectors => "Connectors",
            Self::Thickness => "Minimum thickness",
            Self::Finish => "Finish & export",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::Scans => "Load the jaw, antagonist and bite scans and give each its role.",
            Self::Orient => "Set the occlusal plane and midline so every view lines up.",
            Self::Margin => "Trace the preparation margin on each die.",
            Self::InsertionAxis => "Pick the path of insertion and check for undercuts.",
            Self::InnerSurface => "Build the fitting surface with the cement gap and margin seal.",
            Self::Anatomy => "Place a tooth shape and fit it to the gap and neighbours.",
            Self::PonticBase => "Shape the pontic where it meets the gingiva.",
            Self::Occlusion => "Adjust contacts against the antagonist and neighbours.",
            Self::Sculpt => "Refine the shape with freeform tools.",
            Self::Connectors => "Size and place the connectors between bridge units.",
            Self::Thickness => "Check and fix the material's minimum thickness.",
            Self::Finish => "Merge, check the result is watertight and export it.",
        }
    }

    /// Steps the user may skip; the others produce something later steps need.
    pub fn is_optional(self) -> bool {
        matches!(self, Self::Orient | Self::Occlusion | Self::Sculpt)
    }

    /// Steps that run once per tooth rather than once for the whole case.
    pub fn is_per_tooth(self) -> bool {
        !matches!(
            self,
            Self::Scans | Self::Orient | Self::Connectors | Self::Finish
        )
    }

    /// The tooth-level steps for one restoration type, in plan order.
    pub fn for_restoration(kind: RestorationKind) -> &'static [StepId] {
        use StepId::*;
        match kind {
            RestorationKind::Crown | RestorationKind::Inlay | RestorationKind::Onlay => &[
                Margin,
                InsertionAxis,
                InnerSurface,
                Anatomy,
                Occlusion,
                Sculpt,
                Thickness,
            ],
            RestorationKind::Coping => &[Margin, InsertionAxis, InnerSurface, Sculpt, Thickness],
            RestorationKind::Veneer => &[
                Margin,
                InsertionAxis,
                InnerSurface,
                Anatomy,
                Sculpt,
                Thickness,
            ],
            RestorationKind::Pontic => &[Anatomy, PonticBase, Occlusion, Sculpt, Thickness],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedStep {
    pub id: StepId,
    /// Teeth the step applies to, along the arch; empty for case-wide steps.
    pub teeth: Vec<Fdi>,
}

/// The steps a case goes through, in order.
pub fn plan(case: &Case) -> Vec<PlannedStep> {
    let mut units = case.units.clone();
    units.sort_by_key(|u| (u.tooth.arch() as u8, u.tooth.arch_index()));
    StepId::ALL
        .iter()
        .filter_map(|&id| {
            let teeth: Vec<Fdi> = units
                .iter()
                .filter(|u| StepId::for_restoration(u.kind).contains(&id))
                .map(|u| u.tooth)
                .collect();
            let included = match id {
                StepId::Scans | StepId::Orient | StepId::Finish => true,
                StepId::Connectors => !case.connectors.is_empty(),
                _ => !teeth.is_empty(),
            };
            included.then(|| PlannedStep {
                id,
                teeth: if id.is_per_tooth() { teeth } else { vec![] },
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Active,
    Done,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkflowError {
    #[error("the workflow is already finished")]
    Finished,
    #[error("{0} cannot be skipped")]
    NotOptional(&'static str),
    #[error("step {0} is not reachable yet; finish the earlier steps first")]
    NotReachable(usize),
}

/// Where the user is in a case's plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    steps: Vec<PlannedStep>,
    status: Vec<StepStatus>,
    current: usize,
}

impl Session {
    pub fn new(steps: Vec<PlannedStep>) -> Self {
        let mut status = vec![StepStatus::Pending; steps.len()];
        if let Some(first) = status.first_mut() {
            *first = StepStatus::Active;
        }
        Self {
            steps,
            status,
            current: 0,
        }
    }

    pub fn for_case(case: &Case) -> Self {
        Self::new(plan(case))
    }

    pub fn steps(&self) -> &[PlannedStep] {
        &self.steps
    }

    pub fn status(&self, index: usize) -> StepStatus {
        self.status[index]
    }

    pub fn current_index(&self) -> usize {
        self.current
    }

    /// The active step, or `None` once every step is done or skipped.
    pub fn current(&self) -> Option<&PlannedStep> {
        (self.status.get(self.current) == Some(&StepStatus::Active))
            .then(|| &self.steps[self.current])
    }

    pub fn is_finished(&self) -> bool {
        self.status
            .iter()
            .all(|s| matches!(s, StepStatus::Done | StepStatus::Skipped))
    }

    /// (steps done or skipped, total steps)
    pub fn progress(&self) -> (usize, usize) {
        let done = self
            .status
            .iter()
            .filter(|s| matches!(s, StepStatus::Done | StepStatus::Skipped))
            .count();
        (done, self.steps.len())
    }

    /// Marks the active step done and moves to the next unfinished one.
    pub fn complete(&mut self) -> Result<(), WorkflowError> {
        self.finish_current(StepStatus::Done)
    }

    /// Skips the active step, if it is optional.
    pub fn skip(&mut self) -> Result<(), WorkflowError> {
        let step = self.current().ok_or(WorkflowError::Finished)?;
        if !step.id.is_optional() {
            return Err(WorkflowError::NotOptional(step.id.title()));
        }
        self.finish_current(StepStatus::Skipped)
    }

    /// Re-opens a finished step, or moves to the first unfinished one. Later finished steps keep
    /// their status; the user walks forward through them again with `complete`.
    pub fn go_to(&mut self, index: usize) -> Result<(), WorkflowError> {
        let first_open = self.first_open();
        let reachable = index < self.steps.len()
            && (matches!(self.status[index], StepStatus::Done | StepStatus::Skipped)
                || Some(index) == first_open);
        if !reachable {
            return Err(WorkflowError::NotReachable(index));
        }
        if self.status[self.current] == StepStatus::Active {
            self.status[self.current] = StepStatus::Pending;
        }
        self.current = index;
        self.status[index] = StepStatus::Active;
        Ok(())
    }

    /// Re-opens the step before the current one.
    pub fn back(&mut self) -> Result<(), WorkflowError> {
        match self.current.checked_sub(1) {
            Some(previous) => self.go_to(previous),
            None => Err(WorkflowError::NotReachable(0)),
        }
    }

    fn first_open(&self) -> Option<usize> {
        self.status
            .iter()
            .position(|s| matches!(s, StepStatus::Pending | StepStatus::Active))
    }

    fn finish_current(&mut self, status: StepStatus) -> Result<(), WorkflowError> {
        if self.current().is_none() {
            return Err(WorkflowError::Finished);
        }
        self.status[self.current] = status;
        if let Some(next) = self.first_open() {
            self.current = next;
            self.status[next] = StepStatus::Active;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dentcraft_case::{Case, Fdi, RestorationKind};

    fn t(code: u8) -> Fdi {
        Fdi::new(code).unwrap()
    }

    fn ids(steps: &[PlannedStep]) -> Vec<StepId> {
        steps.iter().map(|s| s.id).collect()
    }

    #[test]
    fn empty_case_still_imports_orients_and_finishes() {
        assert_eq!(
            ids(&plan(&Case::new("empty"))),
            vec![StepId::Scans, StepId::Orient, StepId::Finish]
        );
    }

    #[test]
    fn single_crown_plan() {
        let mut case = Case::new("crown");
        case.set_unit(t(36), RestorationKind::Crown);
        let steps = plan(&case);
        use StepId::*;
        assert_eq!(
            ids(&steps),
            vec![
                Scans,
                Orient,
                Margin,
                InsertionAxis,
                InnerSurface,
                Anatomy,
                Occlusion,
                Sculpt,
                Thickness,
                Finish
            ]
        );
        assert_eq!(steps[2].teeth, vec![t(36)]);
        assert!(steps[0].teeth.is_empty());
    }

    #[test]
    fn bridge_plan_has_pontic_base_and_connectors_with_per_step_teeth() {
        let mut case = Case::new("bridge");
        case.set_unit(t(14), RestorationKind::Crown);
        case.set_unit(t(15), RestorationKind::Pontic);
        case.set_unit(t(16), RestorationKind::Crown);
        case.set_connected(t(14), t(15), true);
        case.set_connected(t(15), t(16), true);
        let steps = plan(&case);
        let step = |id| steps.iter().find(|s| s.id == id).unwrap();
        assert_eq!(step(StepId::Margin).teeth, vec![t(16), t(14)]);
        assert_eq!(step(StepId::Anatomy).teeth, vec![t(16), t(15), t(14)]);
        assert_eq!(step(StepId::PonticBase).teeth, vec![t(15)]);
        assert!(step(StepId::Connectors).teeth.is_empty());
        let order = ids(&steps);
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "plan follows step order: {order:?}"
        );
    }

    #[test]
    fn session_walks_skips_and_goes_back() {
        let mut case = Case::new("crown");
        case.set_unit(t(11), RestorationKind::Veneer);
        let mut session = Session::for_case(&case);
        assert_eq!(session.current().unwrap().id, StepId::Scans);
        assert_eq!(session.skip(), Err(WorkflowError::NotOptional("Scans")));
        session.complete().unwrap();
        session.skip().unwrap(); // Orientation
        assert_eq!(session.status(1), StepStatus::Skipped);
        assert_eq!(session.current().unwrap().id, StepId::Margin);
        assert_eq!(session.go_to(4), Err(WorkflowError::NotReachable(4)));
        session.back().unwrap();
        assert_eq!(session.current().unwrap().id, StepId::Orient);
        assert_eq!(session.status(2), StepStatus::Pending);
        session.complete().unwrap();
        assert_eq!(session.current().unwrap().id, StepId::Margin);
        while session.current().is_some() {
            session.complete().unwrap();
        }
        assert!(session.is_finished());
        assert_eq!(session.complete(), Err(WorkflowError::Finished));
        let (done, total) = session.progress();
        assert_eq!(done, total);
    }

    #[test]
    fn reopening_a_step_keeps_later_work() {
        let mut session = Session::for_case(&Case::new("x"));
        session.complete().unwrap();
        session.complete().unwrap();
        session.go_to(0).unwrap();
        assert_eq!(session.status(1), StepStatus::Done);
        session.complete().unwrap();
        assert_eq!(session.current().unwrap().id, StepId::Finish);
    }
}
