//! The user's design decisions for a case. Only what the user chose is stored here; meshes
//! derived from it (inner surfaces, crowns) are regenerated, never saved.

use std::collections::BTreeMap;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::Fdi;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Design {
    /// Frame of the jaw the restorations are designed in, once the user has set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jaw_frame: Option<JawFrame>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub teeth: BTreeMap<Fdi, ToothDesign>,
}

impl Design {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Orientation of the jaw: an origin, the occlusal direction (out of the biting surfaces) and
/// the forward direction (towards the incisors). Millimetres, scan coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JawFrame {
    pub origin: [f64; 3],
    pub occlusal: [f64; 3],
    pub forward: [f64; 3],
}

/// Decisions for one tooth.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToothDesign {
    /// Margin line on the preparation, as an ordered closed loop of surface points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<Vec<DVec3>>,
    /// Direction the restoration is seated along, pointing out of the preparation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insertion_axis: Option<DVec3>,
    /// Inner-surface parameters that differ from the material's defaults.
    #[serde(default, skip_serializing_if = "InnerSurfaceOverrides::is_empty")]
    pub inner_surface: InnerSurfaceOverrides,
}

/// Per-tooth overrides of the inner-surface parameters, in millimetres. `None` keeps the
/// default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InnerSurfaceOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cement_gap: Option<f64>,
    /// Gap along the seal band just above the margin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal_gap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal_width: Option<f64>,
    /// Width of the blend from the seal gap to the cement gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_width: Option<f64>,
}

impl InnerSurfaceOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}
