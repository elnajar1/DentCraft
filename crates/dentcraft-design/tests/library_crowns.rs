//! Exercise the app's anatomy-to-crown path using the procedural tooth library.
use dentcraft_design::{
    Crown, CrownParams, InnerSurface, InnerSurfaceParams, MarginTracer, PreparedStump, ToothParams,
    auto_placement, build_crown, inner_surface, prepared_tooth,
};
use glam::DVec3;
#[path = "support/crown.rs"]
mod verification;
use verification::verify;

fn intaglio(die: &PreparedStump, grid_step: f64) -> InnerSurface {
    inner_surface(
        &die.mesh,
        &die.region,
        &die.margin,
        die.axis,
        &InnerSurfaceParams {
            grid_step,
            ..Default::default()
        },
    )
    .unwrap()
}

fn library_tooth(code: u8, scale: DVec3) {
    let mut die = prepared_tooth(&ToothParams::default()).unwrap();
    for p in &mut die.mesh.positions {
        *p *= scale;
    }
    let tracer = MarginTracer::new(std::sync::Arc::new(die.mesh.clone())).unwrap();
    let seed = DVec3::new(0.0, 0.0, 12.0) * scale;
    let margin = tracer.propose(seed, die.axis).unwrap();
    die.region = tracer.region(&margin, seed);
    die.margin = margin.points;
    let inner = intaglio(&die, 0.05);
    let tooth =
        dentcraft_anatomy::crown(dentcraft_case::Fdi::new(code).unwrap(), &Default::default());
    let placement = auto_placement(&inner, &tooth.mesh, &tooth.cervical, die.axis, None);
    let params = CrownParams::default();
    let built = build_crown(&inner, &tooth.mesh, &tooth.cervical, placement, &params)
        .unwrap_or_else(|e| panic!("library {code}: {e:?}"));
    eprintln!("library {code}");
    verify(&built, &inner, &die, &params);
}

#[test]
fn library_36() {
    library_tooth(36, DVec3::ONE);
}
#[test]
fn library_46() {
    library_tooth(46, DVec3::ONE);
}
#[test]
fn library_16() {
    library_tooth(16, DVec3::new(1.0, 1.1, 1.0));
}
#[test]
fn library_14() {
    library_tooth(14, DVec3::new(0.7, 0.85, 1.0));
}
#[test]
fn library_11() {
    library_tooth(11, DVec3::new(0.85, 0.65, 1.3));
}
#[test]
fn library_13() {
    library_tooth(13, DVec3::new(0.75, 0.75, 1.4));
}
