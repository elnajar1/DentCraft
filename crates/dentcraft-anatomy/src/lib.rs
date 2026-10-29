//! Parametric tooth anatomy: DentCraft's own crown library, generated from public knowledge.
//!
//! Every tooth is built procedurally from published average crown dimensions
//! (Nelson & Ash, *Wheeler's Dental Anatomy, Physiology and Occlusion*, 9th ed., 2010,
//! "Measurements of the teeth") and morphology descriptions from the same textbook. Nothing in
//! this crate is derived from any proprietary dental CAD product. Dimensions are millimetres;
//! geometry is `f64` throughout.
//!
//! # Canonical frame
//!
//! A [`CrownTemplate`] is the anatomic crown from the cervical line (CEJ) to the
//! occlusal / incisal surface, open at the cervical line, wound counter-clockwise seen from
//! outside (outward normals):
//!
//! - origin at the centre of the cervical outline, mid-facial CEJ at `z = 0`;
//! - `+Z` occlusal (or incisal);
//! - `+Y` facial (labial / buccal);
//! - `+X` mesial for the right quadrants (FDI 1 and 4).
//!
//! Teeth of the left quadrants (FDI 2 and 3) are built as their right counterparts and then
//! mirrored: `x` negated, triangle winding swapped, cervical loop reversed, so their mesial
//! direction is `-X` and the meshes remain outward-oriented.
//!
//! Crown height is the `Z` extent from the mid-facial CEJ (`z = 0`) to the highest vertex; the
//! mesiodistal and buccolingual extents of the mesh equal the published averages exactly (up to
//! vertex sampling), scaled by [`CrownParams`].
//!
//! # Parameter sanitisation
//!
//! [`CrownParams`] are multiplicative factors around the textbook average. Non-finite values
//! fall back to the default (1, or 0 for `wear`) and finite values are clamped to:
//!
//! | field | range | default |
//! |---|---|---|
//! | `mesiodistal` | 0.5 – 2.0 | 1.0 |
//! | `buccolingual` | 0.5 – 2.0 | 1.0 |
//! | `height` | 0.5 – 2.0 | 1.0 |
//! | `wear` | 0 – 1 | 0 |
//! | `cusp_height` | 0.3 – 2.0 | 1.0 |

mod dimensions;
mod library;
mod shape;

use dentcraft_case::Fdi;
use dentcraft_mesh::TriMesh;

/// A library tooth: anatomic crown mesh, its open cervical boundary and named landmarks.
#[derive(Clone, Debug)]
pub struct CrownTemplate {
    /// The anatomic crown from the CEJ to the occlusal surface, open at the cervical line,
    /// smooth (~18k triangles), outward normals, no self-intersections by construction.
    pub mesh: TriMesh,
    /// The single boundary loop of `mesh`, ordered along the loop, starting at the mid-facial
    /// cervix (for right teeth: counter-clockwise seen from the occlusal side).
    pub cervical: Vec<u32>,
    /// Named surface points; every landmark is a vertex of `mesh`, so it lies on the surface
    /// exactly.
    pub landmarks: CrownLandmarks,
}

/// Named landmark vertices of a [`CrownTemplate`].
#[derive(Clone, Debug)]
pub struct CrownLandmarks {
    /// Cusp tips (posterior cusps; the canine's single cusp), highest vertices near the
    /// designed tips. Incisors have none.
    pub cusps: Vec<CuspLandmark>,
    /// Widest vertex towards the mesial (the mesial contact area).
    pub mesial_contact: u32,
    /// Widest vertex towards the distal.
    pub distal_contact: u32,
    /// Highest vertex towards the facial / labial (facial height of contour).
    pub facial_contour: u32,
    /// Highest vertex towards the lingual (lingual height of contour).
    pub lingual_contour: u32,
    /// Lowest vertex near the centre of the table (posteriors only).
    pub central_fossa: Option<u32>,
    /// Mesial incisal angle (anteriors only): vertex maximising `x + z`.
    pub incisal_mesial: Option<u32>,
    /// Distal incisal angle (anteriors only): vertex maximising `-x + z`.
    pub incisal_distal: Option<u32>,
}

/// One named cusp tip.
#[derive(Clone, Debug)]
pub struct CuspLandmark {
    /// Anatomic name, e.g. "mesiobuccal", "cusp".
    pub name: String,
    /// Vertex of the template mesh at the cusp tip.
    pub vertex: u32,
}

/// Scaling factors around the textbook average tooth. See the crate docs for the sanitisation
/// rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrownParams {
    /// Mesiodistal width factor.
    pub mesiodistal: f64,
    /// Buccolingual depth factor.
    pub buccolingual: f64,
    /// Crown height factor.
    pub height: f64,
    /// Occlusal wear, 0 (unworn) to 1 (45 % of the cusp relief removed, slopes flattened,
    /// tips and grooves rounded).
    pub wear: f64,
    /// Cusp height factor (scales cusp relief over the fossa floor).
    pub cusp_height: f64,
}

impl Default for CrownParams {
    fn default() -> Self {
        Self {
            mesiodistal: 1.0,
            buccolingual: 1.0,
            height: 1.0,
            wear: 0.0,
            cusp_height: 1.0,
        }
    }
}

impl CrownParams {
    /// Clamps into the documented ranges, replacing non-finite values with the default.
    pub fn sanitised(&self) -> Self {
        fn factor(v: f64, lo: f64, hi: f64) -> f64 {
            if v.is_finite() { v.clamp(lo, hi) } else { 1.0 }
        }
        Self {
            mesiodistal: factor(self.mesiodistal, 0.5, 2.0),
            buccolingual: factor(self.buccolingual, 0.5, 2.0),
            height: factor(self.height, 0.5, 2.0),
            wear: if self.wear.is_finite() {
                self.wear.clamp(0.0, 1.0)
            } else {
                0.0
            },
            cusp_height: factor(self.cusp_height, 0.3, 2.0),
        }
    }
}

/// Builds the crown library tooth `fdi` (any of the 32 permanent teeth) with sanitised `params`.
///
/// The reference build (no wear, full cusps) is vertically calibrated so its highest vertex sits
/// exactly at the (scaled) textbook crown length; worn builds share that calibration.
pub fn crown(fdi: Fdi, params: &CrownParams) -> CrownTemplate {
    library::build_crown(fdi, params)
}
