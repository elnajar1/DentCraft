//! The DentCraft case model: teeth, restorations, scans and materials.

mod case;
mod material;
mod tooth;

pub use case::{Case, CaseFileError, CaseIssue, Connector, RestorationKind, Scan, ScanRole, Unit};
pub use material::Material;
pub use tooth::{Arch, Fdi, InvalidFdi, ToothType};
