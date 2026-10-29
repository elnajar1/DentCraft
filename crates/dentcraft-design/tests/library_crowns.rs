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

#[test]
fn canine_33_on_sloping_noisy_margin() {
    let stump = dentcraft_design::StumpParams {
        height: 6.0,
        top_radius: 2.0,
        segments: 192,
        ..Default::default()
    };
    let mut die = dentcraft_design::prepared_stump(&stump).unwrap();
    let rotation = glam::DQuat::from_rotation_y(0.37) * glam::DQuat::from_rotation_z(0.29);
    let warp = |p: DVec3| {
        let angle = p.y.atan2(p.x);
        let r = (p.truncate().length() / stump.base_radius).min(1.0);
        let fade = 1.0 - ((p.z - stump.margin_height) / stump.height).clamp(0.0, 1.0);
        rotation
            * (p + DVec3::Z * (2.4 * angle.cos() * r + 0.01 * (19.0 * angle).sin() * r * r) * fade)
    };
    for p in &mut die.mesh.positions {
        *p = warp(*p);
    }
    for p in &mut die.margin {
        *p = warp(*p);
    }
    die.axis = rotation * DVec3::Z;
    let inner = intaglio(&die, 0.05);
    let tooth =
        dentcraft_anatomy::crown(dentcraft_case::Fdi::new(33).unwrap(), &Default::default());
    let placement = auto_placement(&inner, &tooth.mesh, &tooth.cervical, die.axis, None);
    let params = CrownParams::new(0.7, 0.5);
    let built = build_crown(&inner, &tooth.mesh, &tooth.cervical, placement, &params).unwrap();
    verify(&built, &inner, &die, &params);
}
