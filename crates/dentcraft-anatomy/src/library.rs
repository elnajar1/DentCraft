//! The library itself: one anatomy spec per tooth (16 right-side teeth; quadrants 2 and 4 are
//! mirrored, see the frame in the crate docs), assembly into a [`CrownTemplate`], and landmark
//! search.
//!
//! Morphology guidance for the occlusal fields (cusp counts, which cusps are tallest, groove
//! patterns, ridge locations) comes from the same public textbook as the dimensions:
//! Nelson & Ash, *Wheeler's Dental Anatomy, Physiology and Occlusion*, 9th ed., 2010, plus
//! standard FDI / ISO 3950 notation. All coordinates below are in the generation frame
//! (+X mesial, +Y facial, +Z occlusal; the in-mouth handedness of quadrants 1 and 3); xy positions are fractions of the half
//! mesiodistal / half buccolingual widths, sizes and heights as documented in `shape`.

use crate::dimensions::dimensions;
use crate::shape::{
    AnteriorField, BlobSpec, BuiltSurface, CuspSpec, DepressionSpec, Field, FieldCtx, GrooveSpec,
    OutlineParams, PosteriorField, RidgeSpec, SurfaceRequest, SurfaceTuning, build_surface,
};
use crate::{CrownLandmarks, CrownParams, CrownTemplate, CuspLandmark};
use dentcraft_case::Fdi;
use glam::DVec3;

pub(crate) const COLUMNS: usize = 160;
/// 60 row budget across the three segments share two rows, giving 58 rows and ~18.4k triangles.
pub(crate) const ROW_BUDGET: usize = 60;
pub(crate) const WALL_WEIGHT: f64 = 0.75;

// ---------------------------------------------------------------------------
// Spec types and shorthands
// ---------------------------------------------------------------------------

pub(crate) struct ToothSpec {
    outline: OutlineParams,
    /// Height of contour as a fraction of crown height, mesial / distal / facial / lingual.
    z_hoc: [f64; 4],
    /// Occlusal rim as a fraction of the height-of-contour radius, mesial / distal / facial /
    /// lingual.
    kappa: [f64; 4],
    /// Lingual CEJ curvature (mm, usually below the mid-facial CEJ datum).
    cej_lingual: f64,
    /// Centre of the occlusal table / incisal field, fractions of the half widths.
    pole: [f64; 2],
    /// Shoulder profile exponents (a, b).
    shoulder: [f64; 2],
    /// Wall depressions: labial developmental depressions, buccal / lingual grooves that cross
    /// onto the shoulder.
    depressions: Vec<DepressionSpec>,
    field: Field,
}

fn outline(n_f: f64, n_l: f64, taper_x: f64, taper_y: f64, shear: f64) -> OutlineParams {
    OutlineParams {
        a_m: 1.0,
        a_d: 1.0,
        b_f: 1.0,
        b_l: 1.0,
        n_f,
        n_l,
        taper_x,
        taper_y,
        shear,
    }
}

fn cusp(
    name: &'static str,
    fx: f64,
    fy: f64,
    z_frac: f64,
    a_long: f64,
    b_short: f64,
    slope: f64,
) -> CuspSpec {
    CuspSpec {
        name,
        fx,
        fy,
        z_frac,
        a_long,
        b_short,
        slope,
    }
}

/// A ridge from `a` to `b`, each `[x, y, z]`: xy as fractions of the half widths, z as a fraction of
/// the crown height.
fn ridge(a: [f64; 3], b: [f64; 3], bump: f64, sigma: f64) -> RidgeSpec {
    RidgeSpec {
        a: [a[0], a[1]],
        b: [b[0], b[1]],
        za: a[2],
        zb: b[2],
        bump,
        sigma,
    }
}

fn groove(ax: f64, ay: f64, bx: f64, by: f64, depth: f64, sigma: f64) -> GrooveSpec {
    GrooveSpec {
        a: [ax, ay],
        b: [bx, by],
        depth,
        sigma,
    }
}

fn blob(at: [f64; 2], height: f64, sx: f64, sy: f64) -> BlobSpec {
    BlobSpec { at, height, sx, sy }
}

fn dep(theta: f64, sigma: f64, depth: f64) -> DepressionSpec {
    DepressionSpec {
        theta,
        sigma,
        depth,
    }
}

/// The incisal crest line of an anterior tooth: `y = y0 - c x^2` (mm), highest at `tip_x`, dropping
/// `slope_m` / `slope_d` mm per mm mesially / distally of the tip.
struct Crest {
    y0: f64,
    c: f64,
    tip_x: f64,
    slope_m: f64,
    slope_d: f64,
}

fn anterior(
    crest: Crest,
    fossa: f64,
    drop: [f64; 3],
    labial_slope: f64,
    has_cusp: bool,
    ridges: Vec<RidgeSpec>,
    blobs: Vec<BlobSpec>,
) -> Field {
    Field::Anterior(AnteriorField {
        crest_y: crest.y0,
        crest_c: crest.c,
        tip_x: crest.tip_x,
        slope_m: crest.slope_m,
        slope_d: crest.slope_d,
        fossa_frac: fossa,
        drop_a: drop[0],
        drop_lambda: drop[1],
        drop_b: drop[2],
        labial_slope,
        has_cusp,
        ridges,
        grooves: Vec::new(),
        blobs,
    })
}

fn posterior(
    fossa: f64,
    cusps: Vec<CuspSpec>,
    ridges: Vec<RidgeSpec>,
    grooves: Vec<GrooveSpec>,
    blobs: Vec<BlobSpec>,
) -> Field {
    Field::Posterior(PosteriorField {
        fossa_frac: fossa,
        cusps,
        ridges,
        grooves,
        blobs,
    })
}

// ---------------------------------------------------------------------------
// One spec per right-side tooth
// ---------------------------------------------------------------------------

/// Anatomy spec of the right-side counterpart of `fdi` (11-18 for uppers, 41-48 for lowers).
pub(crate) fn tooth_spec(fdi: Fdi) -> ToothSpec {
    let upper = fdi.arch() == dentcraft_case::Arch::Upper;
    match (upper, fdi.position()) {
        // ----- upper anteriors -------------------------------------------------
        (true, 1) => ToothSpec {
            outline: outline(2.7, 2.2, 0.08, 0.02, 0.0),
            z_hoc: [0.68, 0.60, 0.24, 0.26],
            kappa: [0.72, 0.66, 0.18, 0.50],
            cej_lingual: -1.35,
            pole: [0.05, -0.17],
            shoulder: [2.3, 1.4],
            depressions: vec![dep(1.121, 0.16, 0.07), dep(2.020, 0.16, 0.07)],
            field: anterior(
                Crest {
                    y0: 0.6,
                    c: 0.035,
                    tip_x: 0.0,
                    slope_m: 0.025,
                    slope_d: 0.045,
                },
                0.24,
                [5.0, 1.8, 0.4],
                0.9,
                false,
                vec![
                    ridge([0.50, -0.30, 0.42], [0.71, 0.10, 0.90], 0.30, 0.55),
                    ridge([-0.44, -0.28, 0.42], [-0.65, 0.10, 0.88], 0.28, 0.55),
                ],
                vec![
                    blob([0.0, -0.15], -0.25, 1.3, 0.8),
                    blob([0.0, -0.36], 0.55, 1.3, 0.75),
                ],
            ),
        },
        (true, 2) => ToothSpec {
            outline: outline(2.6, 2.2, 0.10, 0.02, 0.0),
            z_hoc: [0.65, 0.55, 0.24, 0.25],
            kappa: [0.70, 0.62, 0.18, 0.48],
            cej_lingual: -1.15,
            pole: [0.05, -0.18],
            shoulder: [2.3, 1.4],
            depressions: vec![dep(1.071, 0.15, 0.06), dep(2.071, 0.15, 0.06)],
            field: anterior(
                Crest {
                    y0: 0.45,
                    c: 0.045,
                    tip_x: 0.1,
                    slope_m: 0.04,
                    slope_d: 0.07,
                },
                0.24,
                [4.4, 1.6, 0.4],
                0.9,
                false,
                vec![
                    ridge([0.52, -0.33, 0.40], [0.70, 0.08, 0.88], 0.26, 0.50),
                    ridge([-0.46, -0.30, 0.40], [-0.63, 0.08, 0.86], 0.24, 0.50),
                ],
                vec![
                    blob([0.02, -0.15], -0.22, 1.0, 0.7),
                    blob([0.0, -0.35], 0.45, 1.1, 0.7),
                ],
            ),
        },
        (true, 3) => ToothSpec {
            outline: outline(2.45, 2.2, 0.10, 0.02, 0.0),
            z_hoc: [0.55, 0.46, 0.28, 0.27],
            kappa: [0.62, 0.55, 0.22, 0.46],
            cej_lingual: -0.9,
            pole: [0.09, -0.15],
            shoulder: [2.2, 1.35],
            depressions: vec![dep(1.021, 0.15, 0.10), dep(2.121, 0.15, 0.10)],
            field: anterior(
                Crest {
                    y0: 0.7,
                    c: 0.03,
                    tip_x: 0.35,
                    slope_m: 0.42,
                    slope_d: 0.30,
                },
                0.26,
                [4.6, 1.7, 0.5],
                0.85,
                true,
                vec![ridge([0.09, -0.25, 0.42], [0.09, 0.15, 0.93], 0.40, 0.45)],
                vec![
                    blob([0.08, -0.33], 0.6, 1.3, 0.8),
                    blob([0.37, -0.10], -0.28, 0.8, 0.7),
                    blob([-0.35, -0.09], -0.25, 0.8, 0.7),
                ],
            ),
        },
        // ----- upper premolars -------------------------------------------------
        (true, 4) => ToothSpec {
            outline: outline(2.35, 2.15, 0.14, 0.03, 0.35),
            z_hoc: [0.65, 0.62, 0.24, 0.45],
            kappa: [0.62, 0.58, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.02, -0.01],
            shoulder: [2.4, 1.6],
            depressions: Vec::new(),
            field: posterior(
                0.50,
                vec![
                    cusp("buccal", 0.08, 0.42, 1.00, 2.2, 1.55, 0.55),
                    cusp("lingual", -0.04, -0.44, 0.80, 2.0, 1.45, 0.50),
                ],
                vec![
                    ridge([0.44, 0.16, 0.86], [0.36, -0.14, 0.72], 0.22, 0.50),
                    ridge([-0.44, 0.16, 0.80], [-0.44, -0.14, 0.68], 0.20, 0.50),
                ],
                vec![groove(0.60, 0.0, -0.55, 0.0, 0.32, 0.40)],
                vec![
                    blob([0.42, 0.01], -0.12, 0.6, 0.5),
                    blob([-0.45, 0.01], -0.12, 0.6, 0.5),
                ],
            ),
        },
        (true, 5) => ToothSpec {
            outline: outline(2.35, 2.1, 0.12, 0.03, 0.25),
            z_hoc: [0.65, 0.62, 0.24, 0.45],
            kappa: [0.62, 0.58, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.0, -0.01],
            shoulder: [2.4, 1.6],
            depressions: Vec::new(),
            field: posterior(
                0.52,
                vec![
                    cusp("buccal", 0.05, 0.42, 1.00, 2.1, 1.50, 0.50),
                    cusp("lingual", 0.0, -0.44, 0.86, 2.0, 1.45, 0.45),
                ],
                vec![
                    ridge([0.44, 0.15, 0.86], [0.38, -0.13, 0.76], 0.20, 0.50),
                    ridge([-0.44, 0.14, 0.80], [-0.42, -0.13, 0.72], 0.18, 0.50),
                ],
                vec![groove(0.55, 0.0, -0.50, 0.0, 0.28, 0.40)],
                vec![
                    blob([0.40, 0.0], -0.10, 0.55, 0.5),
                    blob([-0.42, 0.0], -0.10, 0.55, 0.5),
                ],
            ),
        },
        // ----- upper molars ----------------------------------------------------
        (true, 6) => ToothSpec {
            outline: outline(2.4, 2.05, 0.16, 0.06, -0.85),
            z_hoc: [0.62, 0.58, 0.22, 0.46],
            kappa: [0.60, 0.55, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.0, -0.02],
            shoulder: [2.5, 1.7],
            depressions: vec![dep(1.40, 0.10, 0.22)],
            field: posterior(
                0.56,
                vec![
                    cusp("mesiobuccal", 0.42, 0.36, 0.96, 2.5, 1.8, 0.52),
                    cusp("distobuccal", -0.30, 0.42, 0.90, 2.3, 1.7, 0.52),
                    cusp("mesiolingual", 0.44, -0.48, 1.00, 2.9, 1.9, 0.48),
                    cusp("distolingual", -0.48, -0.40, 0.78, 2.3, 1.7, 0.55),
                ],
                vec![
                    // The oblique ridge, mesiolingual to distobuccal (Carabelli's omitted).
                    ridge([0.44, -0.48, 0.96], [-0.30, 0.42, 0.82], 0.40, 0.70),
                    ridge([0.42, 0.36, 0.90], [0.44, -0.48, 0.93], 0.30, 0.55),
                    ridge([-0.30, 0.42, 0.80], [-0.48, -0.40, 0.72], 0.25, 0.50),
                ],
                vec![
                    groove(0.40, 0.02, 0.12, 0.04, 0.28, 0.42),
                    groove(0.12, 0.04, -0.05, 0.62, 0.25, 0.38),
                    groove(-0.10, 0.05, -0.42, 0.06, 0.25, 0.42),
                ],
                vec![
                    blob([0.42, 0.02], -0.18, 0.6, 0.5),
                    blob([0.02, 0.04], -0.20, 0.65, 0.55),
                    blob([-0.44, 0.06], -0.15, 0.5, 0.5),
                ],
            ),
        },
        (true, 7) => ToothSpec {
            outline: outline(2.35, 2.0, 0.19, 0.06, -1.0),
            z_hoc: [0.62, 0.57, 0.22, 0.46],
            kappa: [0.58, 0.54, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [-0.02, -0.02],
            shoulder: [2.5, 1.7],
            depressions: vec![dep(1.53, 0.10, 0.24)],
            field: posterior(
                0.56,
                vec![
                    cusp("mesiobuccal", 0.40, 0.40, 0.94, 2.4, 1.7, 0.55),
                    cusp("distobuccal", -0.38, 0.42, 0.86, 2.2, 1.6, 0.55),
                    cusp("mesiolingual", 0.42, -0.48, 1.00, 2.7, 1.8, 0.50),
                    cusp("distolingual", -0.44, -0.42, 0.74, 2.1, 1.6, 0.55),
                ],
                vec![
                    ridge([0.42, -0.48, 0.95], [-0.38, 0.42, 0.80], 0.42, 0.70),
                    ridge([0.40, 0.40, 0.88], [0.42, -0.48, 0.93], 0.30, 0.55),
                    ridge([-0.38, 0.42, 0.78], [-0.44, -0.42, 0.68], 0.24, 0.50),
                ],
                vec![
                    groove(0.38, 0.02, 0.10, 0.03, 0.26, 0.42),
                    groove(0.10, 0.03, 0.0, 0.62, 0.28, 0.40),
                    groove(-0.12, 0.04, -0.40, 0.05, 0.26, 0.42),
                ],
                vec![
                    blob([0.40, 0.02], -0.16, 0.6, 0.5),
                    blob([0.0, 0.03], -0.18, 0.65, 0.55),
                    blob([-0.42, 0.05], -0.16, 0.5, 0.5),
                ],
            ),
        },
        (true, 8) => ToothSpec {
            // Third molars vary widely; modelled as a rounder, smaller second molar.
            outline: outline(2.2, 2.0, 0.22, 0.07, -1.15),
            z_hoc: [0.60, 0.56, 0.22, 0.44],
            kappa: [0.56, 0.52, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.0, -0.02],
            shoulder: [2.5, 1.7],
            depressions: vec![dep(1.571, 0.10, 0.20)],
            field: posterior(
                0.55,
                vec![
                    cusp("mesiobuccal", 0.38, 0.40, 0.93, 2.2, 1.6, 0.60),
                    cusp("distobuccal", -0.35, 0.40, 0.83, 2.0, 1.5, 0.60),
                    cusp("mesiolingual", 0.40, -0.46, 1.00, 2.5, 1.7, 0.55),
                    cusp("distolingual", -0.40, -0.42, 0.68, 1.9, 1.4, 0.48),
                ],
                vec![
                    ridge([0.40, -0.46, 0.94], [-0.35, 0.40, 0.78], 0.35, 0.70),
                    ridge([0.38, 0.40, 0.86], [0.40, -0.46, 0.92], 0.26, 0.55),
                ],
                vec![
                    groove(0.35, 0.0, -0.35, 0.0, 0.25, 0.45),
                    groove(0.0, 0.03, 0.0, 0.60, 0.22, 0.40),
                ],
                vec![blob([0.0, 0.02], -0.18, 0.6, 0.55)],
            ),
        },
        // ----- lower anteriors -------------------------------------------------
        (false, 1) => ToothSpec {
            outline: outline(2.5, 2.3, 0.06, 0.02, 0.0),
            z_hoc: [0.60, 0.50, 0.22, 0.24],
            kappa: [0.70, 0.62, 0.20, 0.46],
            cej_lingual: -1.15,
            pole: [0.0, -0.15],
            shoulder: [2.3, 1.4],
            depressions: vec![dep(1.171, 0.16, 0.04), dep(1.971, 0.16, 0.04)],
            field: anterior(
                Crest {
                    y0: 0.35,
                    c: 0.04,
                    tip_x: 0.0,
                    slope_m: 0.05,
                    slope_d: 0.07,
                },
                0.22,
                [3.4, 1.5, 0.4],
                0.95,
                false,
                vec![
                    ridge([0.56, -0.33, 0.42], [0.70, 0.10, 0.88], 0.20, 0.45),
                    ridge([-0.52, -0.30, 0.42], [-0.64, 0.10, 0.86], 0.20, 0.45),
                ],
                vec![
                    blob([0.0, -0.13], -0.15, 0.8, 0.6),
                    blob([0.0, -0.33], 0.25, 0.9, 0.6),
                ],
            ),
        },
        (false, 2) => ToothSpec {
            outline: outline(2.5, 2.3, 0.06, 0.02, 0.0),
            z_hoc: [0.62, 0.52, 0.22, 0.24],
            kappa: [0.70, 0.62, 0.20, 0.46],
            cej_lingual: -1.15,
            pole: [0.0, -0.15],
            shoulder: [2.3, 1.4],
            depressions: vec![dep(1.171, 0.16, 0.04), dep(1.971, 0.16, 0.04)],
            field: anterior(
                Crest {
                    y0: 0.40,
                    c: 0.04,
                    tip_x: 0.05,
                    slope_m: 0.05,
                    slope_d: 0.06,
                },
                0.22,
                [3.6, 1.5, 0.4],
                0.95,
                false,
                vec![
                    ridge([0.55, -0.32, 0.42], [0.68, 0.10, 0.88], 0.20, 0.45),
                    ridge([-0.50, -0.30, 0.42], [-0.62, 0.10, 0.86], 0.20, 0.45),
                ],
                vec![
                    blob([0.02, -0.13], -0.16, 0.85, 0.6),
                    blob([0.0, -0.33], 0.30, 0.95, 0.6),
                ],
            ),
        },
        (false, 3) => ToothSpec {
            outline: outline(2.45, 2.25, 0.08, 0.02, 0.0),
            z_hoc: [0.52, 0.44, 0.26, 0.26],
            kappa: [0.62, 0.55, 0.22, 0.45],
            cej_lingual: -0.9,
            pole: [0.06, -0.15],
            shoulder: [2.2, 1.35],
            depressions: vec![dep(1.071, 0.15, 0.09), dep(2.071, 0.15, 0.09)],
            field: anterior(
                Crest {
                    y0: 0.55,
                    c: 0.03,
                    tip_x: 0.20,
                    slope_m: 0.46,
                    slope_d: 0.30,
                },
                0.25,
                [4.4, 1.6, 0.5],
                0.85,
                true,
                vec![ridge([0.06, -0.23, 0.42], [0.06, 0.13, 0.94], 0.35, 0.45)],
                vec![
                    blob([0.04, -0.32], 0.5, 1.2, 0.75),
                    blob([0.37, -0.10], -0.25, 0.8, 0.7),
                    blob([-0.34, -0.09], -0.22, 0.8, 0.7),
                ],
            ),
        },
        // ----- lower premolars -------------------------------------------------
        (false, 4) => ToothSpec {
            // First premolar: large buccal cusp, small lingual cusp shifted mesially.
            outline: outline(2.35, 2.15, 0.12, 0.03, 0.45),
            z_hoc: [0.66, 0.62, 0.24, 0.44],
            kappa: [0.62, 0.58, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.10, -0.02],
            shoulder: [2.4, 1.6],
            depressions: Vec::new(),
            field: posterior(
                0.50,
                vec![
                    cusp("buccal", 0.06, 0.40, 1.00, 2.2, 1.5, 0.55),
                    cusp("lingual", 0.18, -0.42, 0.72, 1.8, 1.3, 0.50),
                ],
                vec![
                    ridge([0.46, 0.18, 0.84], [0.28, -0.14, 0.68], 0.22, 0.50),
                    ridge([-0.46, 0.16, 0.78], [-0.40, -0.12, 0.66], 0.18, 0.50),
                ],
                vec![groove(0.50, 0.02, -0.40, 0.0, 0.30, 0.40)],
                vec![
                    blob([0.40, 0.0], -0.12, 0.55, 0.5),
                    blob([-0.44, 0.01], -0.12, 0.55, 0.5),
                ],
            ),
        },
        (false, 5) => ToothSpec {
            // Three-cusp form with the Y groove pattern, the most common mandibular second
            // premolar variant (Wheeler's, mandibular premolars chapter).
            outline: outline(2.35, 2.1, 0.13, 0.03, 0.20),
            z_hoc: [0.65, 0.62, 0.24, 0.45],
            kappa: [0.62, 0.58, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.02, -0.04],
            shoulder: [2.4, 1.6],
            depressions: vec![dep(4.71, 0.09, 0.12)],
            field: posterior(
                0.52,
                vec![
                    cusp("buccal", 0.04, 0.42, 1.00, 2.2, 1.5, 0.55),
                    cusp("mesiolingual", 0.32, -0.40, 0.84, 1.9, 1.35, 0.50),
                    cusp("distolingual", -0.33, -0.36, 0.78, 1.8, 1.3, 0.50),
                ],
                vec![
                    ridge([0.44, 0.16, 0.86], [0.28, -0.16, 0.74], 0.22, 0.50),
                    ridge([-0.44, 0.14, 0.80], [-0.36, -0.15, 0.70], 0.18, 0.50),
                ],
                vec![
                    groove(0.45, 0.04, -0.35, 0.02, 0.30, 0.40),
                    groove(0.06, -0.08, 0.0, -0.62, 0.25, 0.38),
                ],
                vec![blob([0.0, 0.02], -0.15, 0.6, 0.5)],
            ),
        },
        // ----- lower molars ----------------------------------------------------
        (false, 6) => ToothSpec {
            outline: outline(2.35, 2.1, 0.18, 0.07, 0.30),
            z_hoc: [0.62, 0.58, 0.22, 0.50],
            kappa: [0.62, 0.56, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.0, -0.05],
            shoulder: [2.5, 1.7],
            depressions: vec![dep(1.75, 0.10, 0.22), dep(4.75, 0.09, 0.15)],
            field: posterior(
                0.55,
                vec![
                    cusp("mesiobuccal", 0.56, 0.44, 0.90, 2.5, 1.75, 0.55),
                    cusp("distobuccal", -0.07, 0.48, 0.85, 2.3, 1.7, 0.55),
                    cusp("distal", -0.65, 0.29, 0.78, 2.0, 1.5, 0.60),
                    cusp("mesiolingual", 0.49, -0.55, 1.00, 2.8, 1.85, 0.50),
                    cusp("distolingual", -0.36, -0.55, 0.93, 2.4, 1.7, 0.55),
                ],
                vec![
                    ridge([0.56, 0.44, 0.86], [0.49, -0.55, 0.93], 0.32, 0.55),
                    ridge([-0.65, 0.29, 0.74], [-0.36, -0.55, 0.87], 0.26, 0.50),
                ],
                vec![
                    groove(0.50, 0.03, 0.0, 0.0, 0.32, 0.42),
                    groove(0.0, 0.0, -0.48, 0.04, 0.32, 0.42),
                    groove(0.0, 0.05, -0.10, 0.66, 0.28, 0.40),
                    groove(0.0, -0.05, 0.05, -0.68, 0.22, 0.38),
                ],
                vec![
                    blob([0.42, 0.02], -0.20, 0.6, 0.5),
                    blob([0.0, 0.0], -0.22, 0.6, 0.55),
                    blob([-0.42, 0.05], -0.18, 0.5, 0.5),
                ],
            ),
        },
        (false, 7) => ToothSpec {
            outline: outline(2.35, 2.1, 0.19, 0.06, 0.25),
            z_hoc: [0.62, 0.58, 0.22, 0.50],
            kappa: [0.60, 0.56, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.0, 0.0],
            shoulder: [2.5, 1.7],
            depressions: vec![dep(1.571, 0.10, 0.24), dep(4.712, 0.09, 0.15)],
            field: posterior(
                0.56,
                vec![
                    cusp("mesiobuccal", 0.42, 0.45, 0.90, 2.4, 1.7, 0.55),
                    cusp("distobuccal", -0.42, 0.45, 0.84, 2.2, 1.6, 0.55),
                    cusp("mesiolingual", 0.44, -0.50, 1.00, 2.6, 1.75, 0.50),
                    cusp("distolingual", -0.44, -0.50, 0.92, 2.3, 1.6, 0.55),
                ],
                vec![
                    ridge([0.42, 0.45, 0.86], [0.44, -0.50, 0.93], 0.30, 0.55),
                    ridge([-0.42, 0.45, 0.80], [-0.44, -0.50, 0.86], 0.26, 0.50),
                ],
                vec![
                    groove(0.50, 0.0, -0.50, 0.0, 0.30, 0.45),
                    groove(0.0, 0.04, 0.0, 0.65, 0.28, 0.40),
                    groove(0.0, -0.04, 0.0, -0.65, 0.25, 0.40),
                ],
                vec![blob([0.0, 0.0], -0.22, 0.7, 0.55)],
            ),
        },
        (false, 8) => ToothSpec {
            // Third molar: rounder and less regular than the second (documented simplification).
            outline: outline(2.15, 2.0, 0.22, 0.07, 0.35),
            z_hoc: [0.60, 0.56, 0.22, 0.48],
            kappa: [0.58, 0.54, 0.74, 0.74],
            cej_lingual: -0.25,
            pole: [0.0, -0.02],
            shoulder: [2.5, 1.7],
            depressions: vec![dep(1.571, 0.10, 0.20)],
            field: posterior(
                0.55,
                vec![
                    cusp("mesiobuccal", 0.40, 0.42, 0.92, 2.2, 1.6, 0.60),
                    cusp("distobuccal", -0.38, 0.42, 0.84, 2.0, 1.5, 0.60),
                    cusp("mesiolingual", 0.42, -0.48, 1.00, 2.4, 1.65, 0.55),
                    cusp("distolingual", -0.42, -0.46, 0.88, 2.1, 1.5, 0.55),
                ],
                vec![ridge([0.40, 0.42, 0.86], [0.42, -0.48, 0.92], 0.26, 0.55)],
                vec![
                    groove(0.40, 0.0, -0.40, 0.0, 0.25, 0.45),
                    groove(0.0, 0.03, 0.0, 0.60, 0.22, 0.40),
                ],
                vec![blob([0.0, 0.0], -0.18, 0.6, 0.55)],
            ),
        },
        (_, p) => unreachable!("positions run 1..=8, got {p}"),
    }
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

struct ScaledDims {
    md: f64,
    bl: f64,
    height: f64,
    cervix_md: f64,
    cervix_bl: f64,
}

fn scaled_dims(fdi: Fdi, p: &CrownParams) -> ScaledDims {
    let dim = dimensions(fdi);
    ScaledDims {
        md: dim.mesiodistal * p.mesiodistal,
        bl: dim.buccolingual * p.buccolingual,
        height: dim.crown_length * p.height,
        cervix_md: dim.mesiodistal_cervix * p.mesiodistal,
        cervix_bl: dim.buccolingual_cervix * p.buccolingual,
    }
}

/// Build the raw right-frame surface of `fdi` at the given wear and cusp factor.
fn build_once(fdi: Fdi, p: &CrownParams, wear: f64, cusp_factor: f64) -> BuiltSurface {
    let dim = dimensions(fdi);
    let spec = tooth_spec(fdi);
    let dims = scaled_dims(fdi, p);
    let tuning = SurfaceTuning {
        outline: spec.outline,
        z_hoc: spec.z_hoc,
        kappa: spec.kappa,
        cej: [
            dim.cej_mesial * p.height,
            dim.cej_distal * p.height,
            0.0,
            spec.cej_lingual * p.height,
        ],
        cej_power: 3.0,
        pole: spec.pole,
        shoulder: spec.shoulder,
        depressions: spec.depressions.clone(),
    };
    let request = SurfaceRequest {
        columns: COLUMNS,
        rows: ROW_BUDGET,
        wall_weight: WALL_WEIGHT,
        md: dims.md,
        bl: dims.bl,
        height: dims.height,
        cervix_md: dims.cervix_md,
        cervix_bl: dims.cervix_bl,
        tuning: &tuning,
        field: &spec.field,
        wear,
        cusp_factor,
    };
    build_surface(&request)
}

/// Build the crown template for `fdi` (any quadrant) with sanitised `params`.
pub(crate) fn build_crown(fdi: Fdi, params: &CrownParams) -> CrownTemplate {
    let p = params.sanitised();
    let dims = scaled_dims(fdi, &p);
    let spec = tooth_spec(fdi);

    // Calibration: the reference build (wear 0, cusp factor 1) must measure exactly the
    // textbook crown length, so its sampling shortfall fixes a vertical scale that is applied
    // to every build of this tooth, worn or not.
    let reference = build_once(fdi, &p, 0.0, 1.0);
    let z_ref = reference
        .positions
        .iter()
        .map(|q| q.z)
        .fold(f64::NEG_INFINITY, f64::max);
    let scale = if z_ref > 1e-6 {
        dims.height / z_ref
    } else {
        1.0
    };

    let built = if p.wear == 0.0 && p.cusp_height == 1.0 {
        reference
    } else {
        build_once(fdi, &p, p.wear, p.cusp_height)
    };
    debug_assert_eq!(
        built.positions.len(),
        built.columns * built.rows + 1,
        "pole fan layout"
    );
    debug_assert!(built.shoulder_start < built.table_start && built.table_start < built.rows);
    debug_assert_eq!(built.pole as usize, built.positions.len() - 1);
    let mut positions = built.positions;
    for q in &mut positions {
        q.z *= scale;
    }
    let mut triangles = built.triangles.clone();
    let mut cervical = built.cervical.clone();

    let field_ctx = FieldCtx {
        half_md: dims.md / 2.0,
        half_bl: dims.bl / 2.0,
        height: dims.height,
        pole: (spec.pole[0] * dims.md / 2.0, spec.pole[1] * dims.bl / 2.0),
        wear: p.wear,
        cusp_factor: p.cusp_height,
    };
    let landmarks = find_landmarks(&positions, &spec.field, &field_ctx, fdi.is_anterior());

    // Quadrants 2 and 4: in the mouth +X = facial x occlusal points distally, so the
    // generated tooth (+X mesial) is mirrored to keep the real tooth's handedness.
    if matches!(fdi.quadrant(), 2 | 4) {
        for q in &mut positions {
            q.x = -q.x;
        }
        for t in &mut triangles {
            t.swap(1, 2);
        }
        cervical.reverse();
    }

    CrownTemplate {
        mesh: dentcraft_mesh::TriMesh::new(positions, triangles),
        cervical,
        landmarks,
    }
}

// ---------------------------------------------------------------------------
// Landmarks
// ---------------------------------------------------------------------------

fn arg_extreme(positions: &[DVec3], score: impl Fn(&DVec3) -> f64) -> u32 {
    let mut best = 0usize;
    let mut best_score = f64::NEG_INFINITY;
    for (i, q) in positions.iter().enumerate() {
        let s = score(q);
        if s > best_score {
            best_score = s;
            best = i;
        }
    }
    best as u32
}

/// Highest vertex near each designed cusp tip; nearest vertex as a fallback.
fn cusp_landmark(positions: &[DVec3], tx: f64, ty: f64, name: &str) -> CuspLandmark {
    let mut best = 0usize;
    let mut best_score = f64::NEG_INFINITY;
    for (i, q) in positions.iter().enumerate() {
        let d2 = (q.x - tx) * (q.x - tx) + (q.y - ty) * (q.y - ty);
        let score = if d2 < 1.69 { 1e6 + q.z } else { -d2 };
        if score > best_score {
            best_score = score;
            best = i;
        }
    }
    CuspLandmark {
        name: name.into(),
        vertex: best as u32,
    }
}

fn find_landmarks(
    positions: &[DVec3],
    field: &Field,
    ctx: &FieldCtx,
    anterior: bool,
) -> CrownLandmarks {
    let cusps = field
        .cusp_tips(ctx)
        .iter()
        .map(|(name, tx, ty)| cusp_landmark(positions, *tx, *ty, name))
        .collect();

    let mesial_contact = arg_extreme(positions, |q| q.x);
    let distal_contact = arg_extreme(positions, |q| -q.x);
    let facial_contour = arg_extreme(positions, |q| q.y);
    let lingual_contour = arg_extreme(positions, |q| -q.y);

    let (central_fossa, incisal_mesial, incisal_distal) = if anterior {
        (
            None,
            Some(arg_extreme(positions, |q| q.x + q.z)),
            Some(arg_extreme(positions, |q| -q.x + q.z)),
        )
    } else {
        let (px, py) = ctx.pole;
        (
            Some(arg_extreme(positions, |q| {
                let d2 = (q.x - px) * (q.x - px) + (q.y - py) * (q.y - py);
                if d2 < 3.24 { -1e6 + q.z } else { -d2 }
            })),
            None,
            None,
        )
    };

    CrownLandmarks {
        cusps,
        mesial_contact,
        distal_contact,
        facial_contour,
        lingual_contour,
        central_fossa,
        incisal_mesial,
        incisal_distal,
    }
}

// ---------------------------------------------------------------------------
// Internal surface-quality tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimensions::dimensions;

    fn raw(fdi_code: u8) -> BuiltSurface {
        build_once(
            Fdi::new(fdi_code).unwrap(),
            &CrownParams::default(),
            0.0,
            1.0,
        )
    }

    fn row_vertex(s: &BuiltSurface, column: usize, row: usize) -> DVec3 {
        s.positions[(column % s.columns) * s.rows + row]
    }

    fn triangle_min_row(s: &BuiltSurface, t: [u32; 3]) -> usize {
        t.iter().map(|v| (*v as usize) % s.rows).min().unwrap()
    }

    #[test]
    fn table_and_fan_are_positive_over_xy() {
        for code in [11u8, 13, 16, 36, 45, 46] {
            let s = raw(code);
            let mut checked = 0;
            for t in &s.triangles {
                let rows: Vec<usize> = t.iter().map(|v| (*v as usize) % s.rows).collect();
                let on_table = rows.iter().all(|r| *r >= s.table_start) || t.contains(&s.pole);
                if !on_table {
                    continue;
                }
                let [a, b, c] = t.map(|v| s.positions[v as usize]);
                let cross = (b - a).cross(c - a);
                assert!(
                    cross.z > 1e-12,
                    "code {code}: table triangle {t:?} folded (cross.z = {})",
                    cross.z
                );
                checked += 1;
            }
            assert!(
                checked > 2000,
                "code {code}: checked {checked} table triangles"
            );
        }
    }

    #[test]
    fn wall_normals_face_outward() {
        for code in [11u8, 16, 46] {
            let s = raw(code);
            for t in &s.triangles {
                if t.contains(&s.pole) || triangle_min_row(&s, *t) >= s.shoulder_start {
                    continue;
                }
                let [a, b, c] = t.map(|v| s.positions[v as usize]);
                let centre = (a + b + c) / 3.0;
                let n = (b - a).cross(c - a);
                let radial = DVec3::new(centre.x, centre.y, 0.0);
                // Above the CEJ rise the radial direction is reliable; use its xy part.
                assert!(
                    n.dot(radial) > 0.0,
                    "code {code}: wall triangle {t:?} faces inward (n = {n:?})"
                );
            }
        }
    }

    #[test]
    fn rim_ring_is_star_shaped_around_the_pole() {
        for code in [11u8, 13, 16, 36, 46] {
            let s = raw(code);
            let pole = s.positions[s.pole as usize];
            let mut prev = None;
            for i in 0..s.columns {
                let p = row_vertex(&s, i, s.table_start);
                let ang = (p.y - pole.y).atan2(p.x - pole.x);
                if let Some(p0) = prev {
                    let d = crate::shape::wrap_angle(ang - p0);
                    assert!(
                        d > 0.0 && d < 1.0,
                        "code {code}: rim ring folds at column {i} (delta {d})"
                    );
                }
                prev = Some(ang);
            }
        }
    }

    #[test]
    fn grid_size_is_in_the_documented_range() {
        // The row budget splits by mean arc length, so the exact count varies a little with
        // tooth type (posterior walls are short, incisor shoulders are long).
        for code in [11u8, 13, 16, 36, 46] {
            let s = raw(code);
            assert!((44..=60).contains(&s.rows), "code {code}: {} rows", s.rows);
            let tris = s.triangles.len();
            assert!(
                (13_500..=19_000).contains(&tris),
                "code {code}: {tris} triangles"
            );
            assert_eq!(s.positions.len(), s.columns * s.rows + 1);
        }
    }

    #[test]
    fn cervical_row_starts_at_the_mid_facial_cej() {
        // Column 0 sits exactly at the facial midline, where the CEJ datum is z = 0; the
        // mesial column (theta = 0) is three quarters around.
        let s = raw(11);
        assert!(row_vertex(&s, 0, 0).z.abs() < 1e-12);
        let d = dimensions(Fdi::new(46).unwrap());
        let s46 = raw(46);
        let mesial_rise = row_vertex(&s46, 3 * s46.columns / 4, 0).z;
        assert!(
            mesial_rise > 0.5 && mesial_rise < d.cej_mesial + 0.3,
            "mesial CEJ rise {mesial_rise} vs table {}",
            d.cej_mesial
        );
    }
}
