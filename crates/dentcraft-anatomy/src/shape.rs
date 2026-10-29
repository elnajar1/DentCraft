//! Internal surface machinery for the parametric crown library.
//!
//! One crown is a single regular grid: `n` columns (angular, counter-clockwise seen from the
//! occlusal side, column 0 exactly at the facial midline) of `m` rows (cervical to occlusal)
//! plus a pole fan that closes the occlusal side. Each column has three parts:
//!
//! - **Wall**: from the cervical outline to the height-of-contour outline. Both outlines are
//!   ray-cast from the origin, so the wall is a radial graph and cannot fold. The radial blend
//!   `Oc + sin(pi/2 t)(O - Oc)` puts the widest point exactly at the height of contour, where
//!   the silhouette has a vertical tangent.
//! - **Shoulder**: from the height-of-contour outline `O` to the occlusal rim `R`, both on the
//!   same ray. Height `z = z_hoc + phi(D_R - z_hoc)` with `phi = (1 - (1 - lambda)^a)^(1/b)`,
//!   which leaves the wall vertically and arrives at the rim horizontally; a small cubic term
//!   matches the table's radial slope at the rim so no crease runs around the crown.
//! - **Table**: from `R` to the pole. `R` lies on a ray from the pole, so the whole table is a
//!   graph over XY and cannot self-intersect. Its height is the anatomical field `D`, guarded
//!   to stay at least `0.4 mm` above the height of contour (`softplus` clamp).
//!
//! Everything above the height of contour is a graph over XY and the wall/shoulder are radial
//! graphs on strictly shrinking radii, so the surface has no self-intersections by
//! construction.

use glam::DVec3;

pub(crate) const TAU: f64 = std::f64::consts::TAU;

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Smooth maximum (log-sum-exp) with sharpness `k` in 1/mm.
pub(crate) fn softmax(values: &[f64], k: f64) -> f64 {
    let m = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !m.is_finite() {
        return m;
    }
    m + values.iter().map(|v| (k * (v - m)).exp()).sum::<f64>().ln() / k
}

/// `ln(1 + e^{k x}) / k`: near 0 well below zero, near `x` above.
pub(crate) fn softplus(x: f64, k: f64) -> f64 {
    if k * x > 30.0 {
        x
    } else {
        (1.0 + (k * x).exp()).ln() / k
    }
}

/// `|x|` with the corner rounded at scale `eps`.
pub(crate) fn softabs(x: f64, eps: f64) -> f64 {
    (x * x + eps * eps).sqrt() - eps
}

pub(crate) fn wrap_angle(x: f64) -> f64 {
    let r = (x + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
    if r > std::f64::consts::PI - 1e-12 {
        -std::f64::consts::PI
    } else {
        r
    }
}

/// Weights of the mesial, distal, facial and lingual directions at angle `theta`
/// (0 = mesial, pi/2 = facial). The four weights sum to 1 for every angle.
pub(crate) fn side_weights(theta: f64) -> [f64; 4] {
    let (c, s) = (theta.cos(), theta.sin());
    [
        c.max(0.0) * c.max(0.0),
        (-c).max(0.0) * (-c).max(0.0),
        s.max(0.0) * s.max(0.0),
        (-s).max(0.0) * (-s).max(0.0),
    ]
}

// ---------------------------------------------------------------------------
// Outlines
// ---------------------------------------------------------------------------

/// Tuning of one horizontal outline (cervical or height-of-contour).
#[derive(Clone, Copy, Debug)]
pub(crate) struct OutlineParams {
    /// Half mesiodistal extents, mesial and distal of the origin, before exact scaling.
    pub a_m: f64,
    pub a_d: f64,
    /// Half buccolingual extents, facial and lingual.
    pub b_f: f64,
    pub b_l: f64,
    /// Superellipse exponents of the facial and lingual halves (2 = ellipse, more = squarer).
    pub n_f: f64,
    pub n_l: f64,
    /// Mesiodistal narrowing toward the lingual (fraction of the mesiodistal width).
    pub taper_x: f64,
    /// Buccolingual narrowing toward the distal.
    pub taper_y: f64,
    /// Millimetres the lingual half slides along +X (positive = mesial, negative = distal).
    pub shear: f64,
}

impl OutlineParams {
    /// The outline as a dense closed polyline, exactly `target_x` by `target_y` millimetres.
    fn polyline(&self, target_x: f64, target_y: f64, samples: usize) -> Vec<(f64, f64)> {
        let mut pts = Vec::with_capacity(samples);
        for k in 0..samples {
            let t = TAU * k as f64 / samples as f64;
            let (c, s) = (t.cos(), t.sin());
            let n = if s >= 0.0 { self.n_f } else { self.n_l };
            let a = if c >= 0.0 { self.a_m } else { self.a_d };
            let b = if s >= 0.0 { self.b_f } else { self.b_l };
            pts.push((
                a * c.signum() * c.abs().powf(2.0 / n),
                b * s.signum() * s.abs().powf(2.0 / n),
            ));
        }
        // Trapezoid (lingual narrowing), rhomboid shear and distal depth taper.
        let mut shaped = Vec::with_capacity(samples);
        for &(x, y) in &pts {
            let eta = ((y + self.b_l) / (self.b_f + self.b_l)).clamp(0.0, 1.0); // 0 lingual, 1 facial
            let xi = ((x + self.a_d) / (self.a_m + self.a_d)).clamp(0.0, 1.0); // 0 distal, 1 mesial
            let x = x * (1.0 - self.taper_x * (1.0 - eta)) + self.shear * (1.0 - eta);
            let y = y * (1.0 - self.taper_y * (1.0 - xi));
            shaped.push((x, y));
        }
        // Exact overall extents: the outline measures exactly target_x by target_y.
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &(x, y) in &shaped {
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        let (sx, sy) = (target_x / (x1 - x0), target_y / (y1 - y0));
        shaped.iter().map(|&(x, y)| (x * sx, y * sy)).collect()
    }
}

/// A ray-castable outline: radius as a function of the angle, from a dense polyline.
pub(crate) struct Outline {
    angles: Vec<f64>,
    radii: Vec<f64>,
}

impl Outline {
    pub fn new(params: &OutlineParams, target_x: f64, target_y: f64) -> Self {
        let pts = params.polyline(target_x, target_y, 2048);
        let mut angles = Vec::with_capacity(pts.len());
        let mut radii = Vec::with_capacity(pts.len());
        for &(x, y) in &pts {
            angles.push(y.atan2(x).rem_euclid(TAU));
            radii.push((x * x + y * y).sqrt());
        }
        Self { angles, radii }
    }

    /// True when the polyline points wind once around the origin in increasing angle, i.e. the
    /// outline is star-shaped with respect to the origin and ray casting is well defined.
    pub fn is_star_shaped(&self) -> bool {
        self.angles
            .windows(2)
            .all(|w| w[1] > w[0] && (w[1] - w[0]) < 1.0)
    }

    /// Radius of the outline along the ray at `theta`.
    pub fn radial(&self, theta: f64) -> f64 {
        let t = theta.rem_euclid(TAU);
        let n = self.angles.len();
        // Binary search for the bracketing sample.
        let mut lo = 0usize;
        let mut hi = n; // angles[0] <= t (angle 0 exists); wrap case handled below
        if t < self.angles[0] {
            // Between the last sample (angle just under tau) and the first (0).
            let (a0, r0) = (self.angles[n - 1], self.radii[n - 1]);
            let span = TAU - a0 + self.angles[0];
            let f = (t + TAU - a0) / span;
            return r0 + (self.radii[0] - r0) * f;
        }
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.angles[mid] <= t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (a0, r0) = (self.angles[lo], self.radii[lo]);
        if lo + 1 == n {
            return r0;
        }
        let (a1, r1) = (self.angles[lo + 1], self.radii[lo + 1]);
        r0 + (r1 - r0) * ((t - a0) / (a1 - a0)).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Height fields (the anatomical part of the surface)
// ---------------------------------------------------------------------------

/// Everything the fields need to know about the tooth being built. Units: millimetres.
pub(crate) struct FieldCtx {
    pub half_md: f64,
    pub half_bl: f64,
    pub height: f64,
    pub pole: (f64, f64),
    pub wear: f64,
    pub cusp_factor: f64,
}

/// One occlusal cusp: a cone with a superelliptic, anisotropic base.
///
/// `fx`/`fy` place the tip as fractions of the half mesiodistal / half buccolingual widths,
/// `z_frac` is the tip height as a fraction of the crown height, `a_long`/`b_short` are the
/// base radii in millimetres along the triangular ridge (towards the centre of the table) and
/// across it, `slope` the drop in mm per mm.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CuspSpec {
    pub name: &'static str,
    pub fx: f64,
    pub fy: f64,
    pub z_frac: f64,
    pub a_long: f64,
    pub b_short: f64,
    pub slope: f64,
}

/// An elevated segment (marginal ridges, the oblique ridge): the crest runs from
/// (`a` to `b`) (xy as fractions), its height interpolating `za` to `zb` (fractions of the
/// crown height) plus a `bump` (mm) that peaks mid-segment and fades towards the ends.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RidgeSpec {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub za: f64,
    pub zb: f64,
    pub bump: f64,
    pub sigma: f64,
}

/// A groove: a trench of `depth` mm along a segment, fading out at both ends.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GrooveSpec {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub depth: f64,
    pub sigma: f64,
}

/// A local elevation (`height` > 0) or depression (`height` < 0) with gaussian extent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlobSpec {
    pub at: [f64; 2],
    pub height: f64,
    pub sx: f64,
    pub sy: f64,
}

/// A shallow vertical depression of the wall (labial developmental depressions, buccal and
/// lingual grooves crossing onto the shoulder), centred on angle `theta`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DepressionSpec {
    pub theta: f64,
    pub sigma: f64,
    pub depth: f64,
}

/// Posterior occlusal anatomy: a soft-max of cusp cones and ridges, minus grooves and fossae.
#[derive(Clone, Debug)]
pub(crate) struct PosteriorField {
    pub fossa_frac: f64,
    pub cusps: Vec<CuspSpec>,
    pub ridges: Vec<RidgeSpec>,
    pub grooves: Vec<GrooveSpec>,
    pub blobs: Vec<BlobSpec>,
}

/// Anterior anatomy: an incisal crest line with a lingual drop profile, plus ridges, fossae,
/// the cingulum and labial developmental depressions.
#[derive(Clone, Debug)]
pub(crate) struct AnteriorField {
    /// y of the crest line at x = 0 (mm); the crest is `crest_y - crest_c * x^2`.
    pub crest_y: f64,
    pub crest_c: f64,
    /// x of the cusp tip (mm); incisors are nearly flat, canines pointed.
    pub tip_x: f64,
    /// Crest drop per mm of x, mesial and distal of the tip (dimensionless).
    pub slope_m: f64,
    pub slope_d: f64,
    /// Height of the lingual fossa floor as a fraction of the crown height.
    pub fossa_frac: f64,
    /// Lingual drop profile `A (1 - e^{-s/lambda}) + B s` (mm, mm, mm/mm).
    pub drop_a: f64,
    pub drop_lambda: f64,
    pub drop_b: f64,
    /// Drop per mm on the labial side of the crest (the rounded incisal edge).
    pub labial_slope: f64,
    /// Canines carry one cusp; incisors do not.
    pub has_cusp: bool,
    pub ridges: Vec<RidgeSpec>,
    pub grooves: Vec<GrooveSpec>,
    pub blobs: Vec<BlobSpec>,
}

#[derive(Clone, Debug)]
pub(crate) enum Field {
    Posterior(PosteriorField),
    Anterior(AnteriorField),
}

const LSE_K: f64 = 4.5;
const CONE_EXP: f64 = 1.6;
/// Buccolingual spread of the whole posterior occlusal pattern about the pole: the specs place
/// cusp tips at about 45 % of the half width, which leaves an occlusal table far narrower than
/// the 55-65 % of the buccolingual diameter that young teeth show.
const OCCLUSAL_SPREAD: f64 = 1.28;
/// Millimetres a cusp cone drops at its base radius per unit of its spec `slope`: young cusp
/// tips stand 1.5-2.5 mm above the central fossa.
const CUSP_DROP: f64 = 3.6;

/// Adjusted height of a structure that stands `z_frac` crown heights above the fossa floor.
fn relief(fossa_z: f64, span: f64, z_frac: f64, ctx: &FieldCtx) -> f64 {
    let wear = 1.0 - 0.45 * ctx.wear;
    fossa_z + ctx.cusp_factor * wear * (z_frac * span - fossa_z)
}

fn ridge_height(r: &RidgeSpec, x: f64, y: f64, ctx: &FieldCtx, fossa_z: f64) -> f64 {
    let span = ctx.height;
    let a = (r.a[0] * ctx.half_md, r.a[1] * ctx.half_bl);
    let b = (r.b[0] * ctx.half_md, r.b[1] * ctx.half_bl);
    let ab = (b.0 - a.0, b.1 - a.1);
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    if len2 < 1e-12 {
        return f64::NEG_INFINITY;
    }
    let t = (((x - a.0) * ab.0 + (y - a.1) * ab.1) / len2).clamp(0.0, 1.0);
    let px = a.0 + t * ab.0;
    let py = a.1 + t * ab.1;
    let dist = ((x - px) * (x - px) + (y - py) * (y - py)).sqrt();
    let za = relief(fossa_z, span, r.za, ctx);
    let zb = relief(fossa_z, span, r.zb, ctx);
    let bump = r.bump * ctx.cusp_factor * (1.0 - 0.45 * ctx.wear);
    let crest = za + (zb - za) * t + bump * (0.35 + 0.65 * 4.0 * t * (1.0 - t));
    // Falls off quadratically past the crest width; far away it sinks below the cusps, so the
    // soft-max never sees it.
    crest - 1.2 * (dist - r.sigma).max(0.0).powi(2)
}

fn groove_depth(g: &GrooveSpec, x: f64, y: f64, ctx: &FieldCtx) -> f64 {
    let a = (g.a[0] * ctx.half_md, g.a[1] * ctx.half_bl);
    let b = (g.b[0] * ctx.half_md, g.b[1] * ctx.half_bl);
    let ab = (b.0 - a.0, b.1 - a.1);
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    if len2 < 1e-12 {
        return 0.0;
    }
    let t = (((x - a.0) * ab.0 + (y - a.1) * ab.1) / len2).clamp(0.0, 1.0);
    let px = a.0 + t * ab.0;
    let py = a.1 + t * ab.1;
    let dist = ((x - px) * (x - px) + (y - py) * (y - py)).sqrt();
    // Grooves fade with wear like every other sharp feature.
    g.depth
        * (1.0 - 0.5 * ctx.wear)
        * (-(dist / g.sigma).powi(2)).exp()
        * (std::f64::consts::PI * t).sin()
}

fn blob_height(b: &BlobSpec, x: f64, y: f64, ctx: &FieldCtx) -> f64 {
    let bx = b.at[0] * ctx.half_md;
    let by = b.at[1] * ctx.half_bl;
    let scale = if b.height >= 0.0 {
        ctx.cusp_factor * (1.0 - 0.45 * ctx.wear)
    } else {
        1.0 - 0.5 * ctx.wear
    };
    let dx = (x - bx) / b.sx;
    let dy = (y - by) / b.sy;
    b.height * scale * (-(dx * dx + dy * dy)).exp()
}

impl Field {
    /// Height of the crown surface at `(x, y)` (mm), ignoring the shoulder guard.
    pub fn height(&self, x: f64, y: f64, ctx: &FieldCtx) -> f64 {
        match self {
            Field::Posterior(f) => {
                let y = ctx.pole.1 + (y - ctx.pole.1) / OCCLUSAL_SPREAD;
                let fossa_z = f.fossa_frac * ctx.height;
                let mut hs: Vec<f64> = Vec::with_capacity(f.cusps.len() + f.ridges.len());
                for c in &f.cusps {
                    let tip = (c.fx * ctx.half_md, c.fy * ctx.half_bl);
                    let ztip = relief(fossa_z, ctx.height, c.z_frac, ctx);
                    // Long axis towards the centre of the table: the triangular ridge.
                    let mut du = ctx.pole.0 - tip.0;
                    let mut dv = ctx.pole.1 - tip.1;
                    let dl = (du * du + dv * dv).sqrt();
                    if dl < 1e-9 {
                        du = 0.0;
                        dv = 1.0;
                    } else {
                        du /= dl;
                        dv /= dl;
                    }
                    let (rx, ry) = (x - tip.0, y - tip.1);
                    let u = rx * du + ry * dv;
                    let v = -rx * dv + ry * du;
                    let d = ((u.abs() / c.a_long).powf(CONE_EXP)
                        + (v.abs() / c.b_short).powf(CONE_EXP))
                    .powf(1.0 / CONE_EXP);
                    let tip_round = 0.45 * (1.0 + 2.0 * ctx.wear);
                    let slope = CUSP_DROP * c.slope * (1.0 - 0.4 * ctx.wear);
                    hs.push(ztip - slope * ((d * d + tip_round * tip_round).sqrt() - tip_round));
                }
                for r in &f.ridges {
                    hs.push(ridge_height(r, x, y, ctx, fossa_z));
                }
                // The fossa floor: steep cones must not dig below it where few cusps meet.
                hs.push(fossa_z + 0.06 * ctx.height);
                let mut z = softmax(&hs, LSE_K);
                for g in &f.grooves {
                    z -= groove_depth(g, x, y, ctx);
                }
                for b in &f.blobs {
                    z += blob_height(b, x, y, ctx);
                }
                z
            }
            Field::Anterior(f) => {
                let fossa_z = f.fossa_frac * ctx.height;
                let crest_y = f.crest_y - f.crest_c * x * x;
                let crest_top = relief(fossa_z, ctx.height, 1.0, ctx);
                let (dx, eps) = (x - f.tip_x, 0.35 * (1.0 + 2.0 * ctx.wear));
                let slope = if dx >= 0.0 { f.slope_m } else { f.slope_d }
                    * ctx.cusp_factor
                    * (1.0 - 0.45 * ctx.wear);
                let crest = crest_top - slope * softabs(dx, eps);
                let s = y - crest_y;
                // The incisal edge is rounded, not a knife: both sides of the crest are smoothed
                // over about a quarter millimetre (softplus), the crest height kept.
                let lingual =
                    |s: f64| f.drop_a * (1.0 - (-(s / f.drop_lambda)).exp()) + f.drop_b * s;
                const EDGE_K: f64 = 4.0;
                let s0 = std::f64::consts::LN_2 / EDGE_K;
                let drop = lingual(softplus(s, EDGE_K)) + f.labial_slope * softplus(-s, EDGE_K)
                    - lingual(s0)
                    - f.labial_slope * s0;
                let mut z = crest - drop;
                // Ridges join the soft structure of the crest, so use them additively here.
                let mut ridge_z = f64::NEG_INFINITY;
                for r in &f.ridges {
                    ridge_z = ridge_z.max(ridge_height(r, x, y, ctx, fossa_z));
                }
                if ridge_z.is_finite() {
                    z = z.max(ridge_z - 2.0 * softabs(s - 1.2, 1.0));
                }
                for g in &f.grooves {
                    z -= groove_depth(g, x, y, ctx);
                }
                for b in &f.blobs {
                    z += blob_height(b, x, y, ctx);
                }
                z
            }
        }
    }

    /// Designed cusp-tip positions in millimetres, with names, for landmark search.
    pub fn cusp_tips(&self, ctx: &FieldCtx) -> Vec<(&'static str, f64, f64)> {
        match self {
            Field::Posterior(f) => f
                .cusps
                .iter()
                .map(|c| {
                    let y = ctx.pole.1 + (c.fy * ctx.half_bl - ctx.pole.1) * OCCLUSAL_SPREAD;
                    (c.name, c.fx * ctx.half_md, y)
                })
                .collect(),
            Field::Anterior(f) => {
                if !f.has_cusp {
                    vec![]
                } else {
                    vec![("cusp", f.tip_x, f.crest_y - f.crest_c * f.tip_x * f.tip_x)]
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Grid assembly
// ---------------------------------------------------------------------------

/// Tuning that is constant per tooth but not per tooth type.
#[derive(Clone, Debug)]
pub(crate) struct SurfaceTuning {
    pub outline: OutlineParams,
    pub z_hoc: [f64; 4],
    pub kappa: [f64; 4],
    pub cej: [f64; 4],
    pub cej_power: f64,
    pub pole: [f64; 2],
    pub shoulder: [f64; 2],
    pub depressions: Vec<DepressionSpec>,
}

pub(crate) struct SurfaceRequest<'a> {
    pub columns: usize,
    pub rows: usize,
    pub wall_weight: f64,
    pub md: f64,
    pub bl: f64,
    pub height: f64,
    pub cervix_md: f64,
    pub cervix_bl: f64,
    pub tuning: &'a SurfaceTuning,
    pub field: &'a Field,
    pub wear: f64,
    pub cusp_factor: f64,
}

/// A built surface: positions, triangles, the cervical loop, the pole vertex, and the row
/// layout (for tests that check the table and wall separately).
pub(crate) struct BuiltSurface {
    pub positions: Vec<DVec3>,
    pub triangles: Vec<[u32; 3]>,
    pub cervical: Vec<u32>,
    pub pole: u32,
    /// Row index of the first shoulder row (rows below are wall).
    pub shoulder_start: usize,
    /// Row index of the first table row.
    pub table_start: usize,
    pub rows: usize,
    pub columns: usize,
}

/// Resample a polyline at `n` points (endpoints included), equally spaced by chord length.
fn arc_resample(poly: &[DVec3], n: usize) -> Vec<DVec3> {
    assert!(n >= 2, "resampling needs at least two points");
    let mut cum = vec![0.0f64; poly.len()];
    for i in 1..poly.len() {
        cum[i] = cum[i - 1] + (poly[i] - poly[i - 1]).length();
    }
    let total = cum[cum.len() - 1];
    if total <= 1e-12 {
        return vec![poly[0]; n];
    }
    let mut out = Vec::with_capacity(n);
    let mut seg = 1usize;
    for k in 0..n {
        let s = total * k as f64 / (n - 1) as f64;
        while seg + 1 < poly.len() && cum[seg] < s {
            seg += 1;
        }
        let span = cum[seg] - cum[seg - 1];
        let t = if span > 1e-12 {
            ((s - cum[seg - 1]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push(poly[seg - 1] + (poly[seg] - poly[seg - 1]) * t);
    }
    out
}

fn polyline_length(pts: &[DVec3]) -> f64 {
    pts.windows(2).map(|w| (w[1] - w[0]).length()).sum()
}

/// Guard on the table height: stay at least `margin` above the local height of contour.
fn guarded(d: f64, z_hoc: f64, margin: f64) -> f64 {
    z_hoc + margin + softplus(d - z_hoc - margin, 8.0)
}

pub(crate) fn build_surface(req: &SurfaceRequest) -> BuiltSurface {
    let n = req.columns;
    let tuning = req.tuning;
    let ctx = FieldCtx {
        half_md: req.md / 2.0,
        half_bl: req.bl / 2.0,
        height: req.height,
        pole: (tuning.pole[0] * req.md / 2.0, tuning.pole[1] * req.bl / 2.0),
        wear: req.wear,
        cusp_factor: req.cusp_factor,
    };

    // Cervical outline: rounder than the height-of-contour outline, same character.
    let cervix_params = OutlineParams {
        a_m: req.cervix_md / 2.0,
        a_d: req.cervix_md / 2.0,
        b_f: req.cervix_bl / 2.0,
        b_l: req.cervix_bl / 2.0,
        n_f: (tuning.outline.n_f - 0.35).max(1.9),
        n_l: (tuning.outline.n_l - 0.3).max(1.9),
        taper_x: tuning.outline.taper_x * 0.6,
        taper_y: tuning.outline.taper_y * 0.6,
        shear: tuning.outline.shear * 0.5,
    };
    let hoc = Outline::new(&tuning.outline, req.md, req.bl);
    let cervix = Outline::new(&cervix_params, req.cervix_md, req.cervix_bl);
    assert!(
        hoc.is_star_shaped() && cervix.is_star_shaped(),
        "outlines must be star-shaped"
    );

    let (a_sh, b_sh) = (tuning.shoulder[0], tuning.shoulder[1]);

    // Per-column radii, heights and angles.
    let mut thetas = Vec::with_capacity(n);
    let mut r_oc = Vec::with_capacity(n);
    let mut r_o = Vec::with_capacity(n);
    let mut r_r = Vec::with_capacity(n);
    let mut z_cej = Vec::with_capacity(n);
    let mut z_hoc = Vec::with_capacity(n);
    for i in 0..n {
        let theta = std::f64::consts::FRAC_PI_2 + TAU * i as f64 / n as f64;
        let w = side_weights(theta);
        let ro = hoc.radial(theta);
        let rc = cervix.radial(theta).min(0.985 * ro);
        let rr = (w[0] * tuning.kappa[0]
            + w[1] * tuning.kappa[1]
            + w[2] * tuning.kappa[2]
            + w[3] * tuning.kappa[3])
            * ro;
        let rr = rr.clamp(0.30 * ro, 0.97 * ro);
        let zh = (w[0] * tuning.z_hoc[0]
            + w[1] * tuning.z_hoc[1]
            + w[2] * tuning.z_hoc[2]
            + w[3] * tuning.z_hoc[3])
            * req.height;
        let p = tuning.cej_power;
        let zc = tuning.cej[0] * theta.cos().max(0.0).powf(p)
            + tuning.cej[1] * (-theta.cos()).max(0.0).powf(p)
            + tuning.cej[2] * theta.sin().max(0.0).powf(p)
            + tuning.cej[3] * (-theta.sin()).max(0.0).powf(p);
        thetas.push(theta);
        r_oc.push(rc);
        r_o.push(ro);
        r_r.push(rr);
        z_cej.push(zc);
        z_hoc.push(zh);
    }

    // Rim heights and the table slope at the rim (for the shoulder's tangent matching).
    let mut d_rim = vec![0.0f64; n];
    let mut sigma_t = vec![0.0f64; n];
    for i in 0..n {
        let (ct, st) = (thetas[i].cos(), thetas[i].sin());
        let (rx, ry) = (ct * r_r[i], st * r_r[i]);
        let dr = guarded(req.field.height(rx, ry, &ctx), z_hoc[i], 0.4);
        d_rim[i] = dr;
        // Slope of the table along the rim-to-pole direction, one small step inwards.
        let (mut ux, mut uy) = (ctx.pole.0 - rx, ctx.pole.1 - ry);
        let ul = (ux * ux + uy * uy).sqrt().max(1e-9);
        ux /= ul;
        uy /= ul;
        let step = 0.4f64.min(0.4 * ul);
        let d_in = guarded(
            req.field.height(rx + step * ux, ry + step * uy, &ctx),
            z_hoc[i],
            0.4,
        );
        sigma_t[i] = ((d_in - dr) / step).clamp(-3.0, 3.0);
    }

    // Dense samples of each column, and mean arc lengths per segment.
    const WALL_SAMPLES: usize = 48;
    const SHOULDER_SAMPLES: usize = 24;
    const TABLE_SAMPLES: usize = 40;
    let mut wall_polys: Vec<Vec<DVec3>> = Vec::with_capacity(n);
    let mut shoulder_polys: Vec<Vec<DVec3>> = Vec::with_capacity(n);
    let mut table_polys: Vec<Vec<DVec3>> = Vec::with_capacity(n);
    let mut pole_vertex_z = f64::INFINITY;
    let z_floor = z_hoc.iter().copied().fold(f64::INFINITY, f64::min);
    for i in 0..n {
        let (ct, st) = (thetas[i].cos(), thetas[i].sin());
        let (rc, ro, rr) = (r_oc[i], r_o[i], r_r[i]);
        let mut wall = Vec::with_capacity(WALL_SAMPLES);
        for k in 0..WALL_SAMPLES {
            let t = k as f64 / (WALL_SAMPLES - 1) as f64;
            let r = rc + (std::f64::consts::FRAC_PI_2 * t).sin() * (ro - rc);
            wall.push(DVec3::new(
                ct * r,
                st * r,
                z_cej[i] + t * (z_hoc[i] - z_cej[i]),
            ));
        }
        let mut shoulder = Vec::with_capacity(SHOULDER_SAMPLES);
        for k in 0..SHOULDER_SAMPLES {
            let lambda = k as f64 / (SHOULDER_SAMPLES - 1) as f64;
            let r = ro + lambda * (rr - ro);
            let phi = (1.0 - (1.0 - lambda).powf(a_sh)).powf(1.0 / b_sh);
            let mut z = z_hoc[i] + phi * (d_rim[i] - z_hoc[i]);
            // Tangent matching with the table at the rim, plus wall depressions.
            z += sigma_t[i] * (ro - rr) * lambda * lambda * (lambda - 1.0);
            for d in &tuning.depressions {
                let dth = wrap_angle(thetas[i] - d.theta);
                z -= d.depth
                    * (-(dth / d.sigma).powi(2)).exp()
                    * (std::f64::consts::PI * lambda).sin().powi(2);
            }
            shoulder.push(DVec3::new(ct * r, st * r, z));
        }
        let (rx, ry) = (ct * rr, st * rr);
        let dist_pole = ((ctx.pole.0 - rx).hypot(ctx.pole.1 - ry)).max(0.5);
        let offset = 0.3f64.clamp(0.25 * dist_pole, 0.85 * dist_pole);
        let u_end = 1.0 - offset / dist_pole;
        let mut table = Vec::with_capacity(TABLE_SAMPLES);
        for k in 0..TABLE_SAMPLES {
            let u = u_end * k as f64 / (TABLE_SAMPLES - 1) as f64;
            let x = rx + u * (ctx.pole.0 - rx);
            let y = ry + u * (ctx.pole.1 - ry);
            // The guard keeps the rim above this column's height of contour; towards the pole,
            // where all columns meet, it relaxes to one common level so neighbouring columns agree.
            let fade = (u / 0.45).clamp(0.0, 1.0);
            let fade = fade * fade * (3.0 - 2.0 * fade);
            let level = z_hoc[i] + (z_floor - z_hoc[i]) * fade;
            table.push(DVec3::new(
                x,
                y,
                guarded(req.field.height(x, y, &ctx), level, 0.4),
            ));
        }
        pole_vertex_z = pole_vertex_z.min(table[TABLE_SAMPLES - 1].z);
        wall_polys.push(wall);
        shoulder_polys.push(shoulder);
        table_polys.push(table);
    }

    // Row budget: proportional to the mean arc length, the wall weighted down.
    let mean = |polys: &Vec<Vec<DVec3>>| {
        polys.iter().map(|p| polyline_length(p)).sum::<f64>() / polys.len() as f64
    };
    let (lw, ls, lt) = (mean(&wall_polys), mean(&shoulder_polys), mean(&table_polys));
    let budget =
        |w: f64| ((req.rows as f64) * w / (req.wall_weight * lw + ls + lt)).round() as usize;
    let n_wall = budget(req.wall_weight * lw).clamp(20, 34);
    let n_shoulder = budget(ls).clamp(8, 18);
    let n_table = budget(lt).clamp(14, 26);

    // Vertices, row by row, column by column: v(i, j) = i * rows + j.
    let rows = n_wall + n_shoulder + n_table - 2;
    let shoulder_start = n_wall - 1;
    let table_start = n_wall + n_shoulder - 2;
    let mut positions: Vec<DVec3> = Vec::with_capacity(n * rows + 1);
    let mut cervical: Vec<u32> = Vec::with_capacity(n);
    for i in 0..n {
        let wall = arc_resample(&wall_polys[i], n_wall);
        let shoulder = arc_resample(&shoulder_polys[i], n_shoulder);
        let table = arc_resample(&table_polys[i], n_table);
        let mut column: Vec<DVec3> = Vec::with_capacity(rows);
        column.extend_from_slice(&wall);
        column.extend_from_slice(&shoulder[1..]);
        column.extend_from_slice(&table[1..]);
        debug_assert_eq!(column.len(), rows);
        cervical.push((i * rows) as u32);
        positions.extend_from_slice(&column);
    }
    let pole = (n * rows) as u32;
    positions.push(DVec3::new(ctx.pole.0, ctx.pole.1, pole_vertex_z));

    // Triangles: counter-clockwise seen from outside (columns run counter-clockwise, rows
    // rise), plus the pole fan on the last table ring.
    let mut triangles: Vec<[u32; 3]> = Vec::with_capacity(n * (rows - 1) * 2 + n);
    let v = |i: usize, j: usize| -> u32 { ((i % n) * rows + j) as u32 };
    for i in 0..n {
        for j in 0..rows - 1 {
            let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i, j + 1), v(i + 1, j + 1));
            triangles.push([a, b, d]);
            triangles.push([a, d, c]);
        }
        triangles.push([v(i, rows - 1), v(i + 1, rows - 1), pole]);
    }

    BuiltSurface {
        positions,
        triangles,
        cervical,
        pole,
        shoulder_start,
        table_start,
        rows,
        columns: n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_is_close_to_the_maximum() {
        let m = softmax(&[1.0, 0.0, -1.0], LSE_K);
        assert!((1.0..=1.7).contains(&m));
        assert!(softmax(&[], LSE_K).is_infinite());
    }

    #[test]
    fn softplus_stays_near_zero_below_and_identity_above() {
        assert!(softplus(-10.0, 8.0).abs() < 1e-4);
        assert!((softplus(10.0, 8.0) - 10.0).abs() < 1e-9);
        assert!((softplus(0.0, 8.0) - std::f64::consts::LN_2 / 8.0).abs() < 1e-4);
    }

    #[test]
    fn softabs_is_rounded_not_pointed() {
        assert!(softabs(0.0, 0.2).abs() < 1e-12);
        assert!((softabs(5.0, 0.2) - 4.8).abs() < 1e-2);
    }

    #[test]
    fn side_weights_partition_unity() {
        for k in 0..360 {
            let theta = TAU * k as f64 / 360.0;
            let w = side_weights(theta);
            assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-12);
            assert!(w.iter().all(|x| *x >= 0.0));
        }
        let w = side_weights(0.0);
        assert!((w[0] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn outline_is_ray_castable_and_exact() {
        let params = OutlineParams {
            a_m: 5.0,
            a_d: 5.0,
            b_f: 4.0,
            b_l: 4.0,
            n_f: 2.5,
            n_l: 2.1,
            taper_x: 0.18,
            taper_y: 0.06,
            shear: -0.9,
        };
        let o = Outline::new(&params, 10.0, 8.0);
        assert!(o.is_star_shaped());
        // With shear the extreme x moves off the mesial ray, so sweep the angles: the outline
        // must still measure exactly 10 mm wide and 8 mm deep.
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for k in 0..720 {
            let t = TAU * k as f64 / 720.0;
            let r = o.radial(t);
            let (x, y) = (r * t.cos(), r * t.sin());
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        assert!((x1 - x0 - 10.0).abs() < 0.03, "width {}", x1 - x0);
        assert!((y1 - y0 - 8.0).abs() < 0.03, "depth {}", y1 - y0);
        assert!(o.radial(1.0) > 0.0);
    }

    #[test]
    fn arc_resample_keeps_endpoints_and_length() {
        let poly = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(3.0, 4.0, 0.0),
            DVec3::new(3.0, 4.0, 5.0),
        ];
        let out = arc_resample(&poly, 11);
        assert_eq!(out.len(), 11);
        assert_eq!(out[0], poly[0]);
        assert_eq!(out[10], poly[2]);
        // Equal spacing along the first (straight, 5 mm) segment: 10 mm total in 1.0 mm steps.
        assert!(((out[1] - out[0]).length() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn wrap_angle_stays_in_range() {
        assert!(wrap_angle(0.0).abs() < 1e-12);
        assert!(wrap_angle(TAU + 0.25) - 0.25 < 1e-12);
        assert!(wrap_angle(-TAU - 0.25) + 0.25 < 1e-12);
    }
}
