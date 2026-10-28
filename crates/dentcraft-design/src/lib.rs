//! Insertion axes, undercut maps and crown intaglio surfaces. Geometry uses `f64`,
//! distances are millimetres, and angles in the public API are degrees.

pub mod axis;
pub mod error;
pub mod inner;
pub mod synthetic;

mod geometry;
mod polygon;

#[cfg(test)]
mod review_regressions;

pub use axis::{
    AxisResult, UndercutMap, UndercutParams, average_normal, best_insertion_axis,
    best_insertion_axis_with_params, undercut_map, undercut_map_with_params,
};
pub use error::DesignError;
pub use inner::{InnerSurface, InnerSurfaceParams, inner_surface};
pub use synthetic::{PreparedStump, StumpParams, prepared_stump};
