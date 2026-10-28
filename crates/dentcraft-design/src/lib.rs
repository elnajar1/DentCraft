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

pub mod margin;
pub mod orient;
pub use margin::{Margin, MarginTracer};
pub use orient::{JawFrame, jaw_frame};
pub use synthetic::{FinishLine, ToothParams, prepared_tooth};

/// Deterministic pseudo-random inputs for tests, so tolerances are reproducible.
#[cfg(test)]
pub(crate) mod testutil {
    use std::f64::consts::TAU;

    use glam::{DQuat, DVec3};

    /// A 64-bit linear congruential generator; its arithmetic is deterministic
    /// across platforms.
    pub(crate) struct Lcg(u64);

    impl Lcg {
        pub(crate) fn new(seed: u64) -> Self {
            Self(seed)
        }

        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }

        /// Uniform in `[0, 1)`.
        pub(crate) fn unit(&mut self) -> f64 {
            (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
        }

        /// A uniform random unit vector, by rejection sampling in the cube.
        pub(crate) fn unit_vec3(&mut self) -> DVec3 {
            loop {
                let v = DVec3::new(
                    self.unit() * 2.0 - 1.0,
                    self.unit() * 2.0 - 1.0,
                    self.unit() * 2.0 - 1.0,
                );
                if v.length_squared() < 1.0 {
                    return v / v.length();
                }
            }
        }

        /// A uniform random rotation (Shoemake's quaternion parameterization).
        pub(crate) fn rotation(&mut self) -> DQuat {
            let (u1, u2, u3) = (self.unit(), self.unit(), self.unit());
            let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
            let (c2, s2, c3, s3) = (
                (TAU * u2).cos(),
                (TAU * u2).sin(),
                (TAU * u3).cos(),
                (TAU * u3).sin(),
            );
            DQuat::from_xyzw(a * c2, a * s2, b * c3, b * s3)
        }
    }
}
