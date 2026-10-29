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
    /// Whether the frame was computed on the upper jaw. Absent in older files: then the arch of
    /// the case's first unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper: Option<bool>,
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
    /// Shape and placement of the crown's anatomy relative to the automatic fit.
    #[serde(default, skip_serializing_if = "AnatomyDesign::is_default")]
    pub anatomy: AnatomyDesign,
    /// Contact targets against the neighbours and the antagonist.
    #[serde(default, skip_serializing_if = "ContactDesign::is_default")]
    pub contacts: ContactDesign,
}

/// The tooth's contact targets, in millimetres of signed distance (negative = into the other
/// surface): `proximal_contact` against the mesial and distal neighbours, `occlusal_clearance`
/// against the antagonist. The default is a light 0.02 mm proximal contact and occlusal contact
/// without penetration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactDesign {
    pub proximal_contact: f64,
    pub occlusal_clearance: f64,
}

impl Default for ContactDesign {
    fn default() -> Self {
        Self {
            proximal_contact: -0.02,
            occlusal_clearance: 0.0,
        }
    }
}

impl ContactDesign {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The user's adjustments of a tooth's crown anatomy. Shape factors scale the library tooth
/// around its textbook average (1 = average); `wear` runs from 0 (unworn) to 1. `rotation_deg`
/// turns the crown about the insertion axis and `offset` moves it, in millimetres in scan
/// coordinates, after the automatic placement. The default changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnatomyDesign {
    pub mesiodistal: f64,
    pub buccolingual: f64,
    pub height: f64,
    pub wear: f64,
    pub cusp_height: f64,
    pub rotation_deg: f64,
    pub offset: [f64; 3],
}

impl Default for AnatomyDesign {
    fn default() -> Self {
        Self {
            mesiodistal: 1.0,
            buccolingual: 1.0,
            height: 1.0,
            wear: 0.0,
            cusp_height: 1.0,
            rotation_deg: 0.0,
            offset: [0.0; 3],
        }
    }
}

impl AnatomyDesign {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anatomy_defaults_are_left_out_and_old_teeth_load() {
        let tooth = ToothDesign {
            insertion_axis: Some(DVec3::Z),
            ..ToothDesign::default()
        };
        let text = serde_json::to_string(&tooth).unwrap();
        assert!(!text.contains("anatomy"), "{text}");
        // A tooth saved before the Anatomy step loads with the identity anatomy.
        let old: ToothDesign = serde_json::from_str(r#"{"insertion_axis":[0.0,0.0,1.0]}"#).unwrap();
        assert_eq!(old, tooth);
        assert!(old.anatomy.is_default());
    }

    #[test]
    fn contacts_are_left_out_at_their_defaults_and_round_trip() {
        let tooth = ToothDesign {
            insertion_axis: Some(DVec3::Z),
            ..ToothDesign::default()
        };
        let text = serde_json::to_string(&tooth).unwrap();
        assert!(!text.contains("contacts"), "{text}");
        let old: ToothDesign = serde_json::from_str(r#"{"insertion_axis":[0.0,0.0,1.0]}"#).unwrap();
        assert!(old.contacts.is_default());
        let tooth = ToothDesign {
            contacts: ContactDesign {
                proximal_contact: -0.05,
                occlusal_clearance: 0.1,
            },
            ..tooth
        };
        let text = serde_json::to_string(&tooth).unwrap();
        assert!(text.contains("contacts"), "{text}");
        assert_eq!(serde_json::from_str::<ToothDesign>(&text).unwrap(), tooth);
        let partial: ToothDesign =
            serde_json::from_str(r#"{"contacts":{"occlusal_clearance":0.2}}"#).unwrap();
        assert_eq!(
            partial.contacts,
            ContactDesign {
                occlusal_clearance: 0.2,
                ..ContactDesign::default()
            }
        );
    }

    #[test]
    fn anatomy_round_trips_and_missing_fields_keep_their_defaults() {
        let tooth = ToothDesign {
            anatomy: AnatomyDesign {
                mesiodistal: 1.05,
                wear: 0.3,
                rotation_deg: -5.0,
                offset: [0.1, -0.2, 0.0],
                ..AnatomyDesign::default()
            },
            ..ToothDesign::default()
        };
        let text = serde_json::to_string(&tooth).unwrap();
        assert!(text.contains("anatomy"), "{text}");
        assert_eq!(serde_json::from_str::<ToothDesign>(&text).unwrap(), tooth);
        let partial: ToothDesign = serde_json::from_str(r#"{"anatomy":{"height":1.2}}"#).unwrap();
        assert_eq!(
            partial.anatomy,
            AnatomyDesign {
                height: 1.2,
                ..AnatomyDesign::default()
            }
        );
    }
}
