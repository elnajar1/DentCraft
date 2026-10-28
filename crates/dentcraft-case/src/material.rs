//! Restoration materials and the design limits they impose.

use serde::{Deserialize, Serialize};

/// Design limits for one material, in millimetres (area in mm²).
///
/// The presets are conservative starting points from the dental literature. Labs must check them
/// against the instructions of the material they actually use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// Space left for cement between the preparation and the inner surface.
    pub cement_gap: f64,
    /// Distance from the margin over which the cement gap is not applied, for a tight seal.
    pub margin_seal_width: f64,
    pub min_occlusal_thickness: f64,
    pub min_wall_thickness: f64,
    /// Minimum connector cross-section between bridge units.
    pub min_connector_area: f64,
}

impl Material {
    pub fn zirconia() -> Self {
        Self {
            name: "Zirconia (monolithic)".into(),
            cement_gap: 0.03,
            margin_seal_width: 0.8,
            min_occlusal_thickness: 0.7,
            min_wall_thickness: 0.5,
            min_connector_area: 9.0,
        }
    }

    pub fn lithium_disilicate() -> Self {
        Self {
            name: "Lithium disilicate".into(),
            cement_gap: 0.04,
            margin_seal_width: 0.8,
            min_occlusal_thickness: 1.5,
            min_wall_thickness: 1.0,
            min_connector_area: 16.0,
        }
    }

    pub fn pmma() -> Self {
        Self {
            name: "PMMA (provisional)".into(),
            cement_gap: 0.06,
            margin_seal_width: 0.6,
            min_occlusal_thickness: 1.0,
            min_wall_thickness: 0.8,
            min_connector_area: 12.0,
        }
    }

    pub fn wax() -> Self {
        Self {
            name: "Wax / burnout resin".into(),
            cement_gap: 0.05,
            margin_seal_width: 0.8,
            min_occlusal_thickness: 0.5,
            min_wall_thickness: 0.4,
            min_connector_area: 9.0,
        }
    }

    pub fn presets() -> Vec<Material> {
        vec![
            Self::zirconia(),
            Self::lithium_disilicate(),
            Self::pmma(),
            Self::wax(),
        ]
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::zirconia()
    }
}
