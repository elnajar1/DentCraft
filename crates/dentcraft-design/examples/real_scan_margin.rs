//! Reproduces the margin proposal on a local jaw scan, without the app.
//!
//! ```sh
//! cargo run --release -p dentcraft-design --example real_scan_margin -- <jaw.ply|stl> [count]
//! ```
//!
//! Finds the `count` (default 6) most prominent peaks along the scan's occlusal direction (a
//! prepared stump stands out above its surroundings), then proposes a margin from each peak twice:
//! with a horizontal axis, as the app did when it used a facial camera view, and with the occlusal
//! direction. Prints the peak coordinates and, per proposal, the error or the margin's length,
//! point count and depth/radius ranges relative to the seed. Also tries the six reported manual
//! clicks on the local lower-jaw 33 preparation (these coordinates apply only to that scan).
//! For 33, prints sector wall profiles and then runs the app's chain: the stored curve is
//! re-followed densely (no sparse-control re-trace), the region comes from that barrier, the
//! axis search seats the margin and the inner surface seals onto the curve. Clearance
//! verification samples true 3D distances at vertices, edge midpoints and triangle centroids
//! and reports the worst samples with their height above the margin, sector, target and whether
//! anything lies above them or below them along the axis (a scan hole).
//! Scans are never part of the repository; pass a local path.

use std::path::PathBuf;
use std::sync::Arc;

use dentcraft_design::{
    InnerSurface, InnerSurfaceParams, Margin, MarginTracer, best_insertion_axis_with_margin,
    inner_surface, margin_overhang, occlusal_direction,
};
use dentcraft_mesh::{Bvh, TriMesh};
use glam::{DVec2, DVec3};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: real_scan_margin <scan> [count]")?,
    );
    let count: usize = args
        .next()
        .and_then(|c| c.to_str().and_then(|c| c.parse().ok()))
        .unwrap_or(6);
    let mesh = dentcraft_io::load_mesh(&path)?;
    println!(
        "{}: {} vertices, {} triangles",
        path.display(),
        mesh.positions.len(),
        mesh.triangles.len()
    );
    let occlusal = occlusal_direction(&mesh)?;
    println!("occlusal direction {}", fmt(occlusal));
    let peaks = prominent_peaks(&mesh.positions, occlusal, count);
    let started = std::time::Instant::now();
    let mesh = Arc::new(mesh);
    let tracer = MarginTracer::new(mesh.clone())?;
    println!("tracer built in {:.2} s", started.elapsed().as_secs_f64());
    let horizontal = occlusal.any_orthonormal_vector();
    for (top, prominence) in peaks {
        println!("\npeak {}  prominence {:.2} mm", fmt(top), prominence);
        for (name, axis) in [("horizontal", horizontal), ("occlusal", occlusal)] {
            let started = std::time::Instant::now();
            match tracer.propose(top, axis) {
                Ok(margin) => println!(
                    "  {name:<10} {}  ({:.2} s)",
                    describe(&margin, top, axis),
                    started.elapsed().as_secs_f64()
                ),
                Err(e) => println!("  {name:<10} error: {e}"),
            }
        }
    }
    let seed = DVec3::new(7.989, -14.224, -2.515);
    let margin = tracer.propose(seed, occlusal)?;
    diagnose(&mesh, &tracer, &margin, seed, occlusal)?;
    let clicks = [
        DVec3::new(6.857, -10.748, -8.090),
        DVec3::new(7.358, -12.831, -8.960),
        DVec3::new(8.323, -13.442, -9.527),
        DVec3::new(9.164, -13.592, -9.610),
        DVec3::new(11.401, -13.404, -9.766),
        DVec3::new(8.853, -9.166, -7.818),
    ];
    match tracer.trace(&clicks) {
        Ok(margin) => println!("\n33 manual: {}", describe(&margin, seed, occlusal)),
        Err(e) => println!("\n33 manual error: {e}"),
    }
    Ok(())
}

/// Greedy non-maximum suppression over heights along `up`: each peak is the highest vertex
/// with no higher accepted peak within 5 mm; prominence is its height above the lowest vertex
/// within 4 mm across the axis. Returns the `count` most prominent peaks.
fn prominent_peaks(positions: &[DVec3], up: DVec3, count: usize) -> Vec<(DVec3, f64)> {
    let mut order: Vec<usize> = (0..positions.len()).collect();
    order.sort_by(|&a, &b| positions[b].dot(up).total_cmp(&positions[a].dot(up)));
    let across = |a: DVec3, b: DVec3| (a - b).reject_from_normalized(up).length();
    let mut peaks: Vec<(DVec3, f64)> = Vec::new();
    for &i in order.iter().take(positions.len() / 4) {
        let p = positions[i];
        if peaks.iter().any(|(q, _)| across(p, *q) < 5.0) {
            continue;
        }
        let low = positions
            .iter()
            .filter(|q| across(p, **q) < 4.0)
            .map(|q| q.dot(up))
            .fold(f64::INFINITY, f64::min);
        peaks.push((p, p.dot(up) - low));
        if peaks.len() >= 40 {
            break;
        }
    }
    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    peaks.truncate(count);
    peaks
}

fn describe(margin: &Margin, seed: DVec3, axis: DVec3) -> String {
    let axis = axis.normalize();
    let n = margin.points.len();
    let length: f64 = (0..n)
        .map(|i| margin.points[i].distance(margin.points[(i + 1) % n]))
        .sum();
    let heights = margin.points.iter().map(|p| (seed - *p).dot(axis));
    let (lo, hi) = heights.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| {
        (lo.min(h), hi.max(h))
    });
    let (rlo, rhi) = margin
        .points
        .iter()
        .map(|p| (*p - seed).cross(axis).length())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), r| {
            (lo.min(r), hi.max(r))
        });
    let centroid = margin.points.iter().sum::<DVec3>() / n as f64;
    let off_axis = (centroid - seed).reject_from_normalized(axis).length();
    let below = (seed - centroid).dot(axis);
    format!(
        "length {length:.1} mm, {n} points, {} loop vertices, depth {lo:.2}..{hi:.2} mm, radius {rlo:.2}..{rhi:.2} mm, \
         centroid {:.2} mm off the seed's axis and {below:.2} mm below it",
        margin.vertices.len(),
        off_axis
    )
}

fn fmt(p: DVec3) -> String {
    format!("({:.3}, {:.3}, {:.3})", p.x, p.y, p.z)
}

fn diagnose(
    mesh: &TriMesh,
    tracer: &MarginTracer,
    margin: &Margin,
    seed: DVec3,
    initial: DVec3,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n33 proposed: {}", describe(margin, seed, initial));
    let region = tracer.region(margin, seed);
    println!("region: {} triangles", region.len());
    println!(
        "initial overhang {:?}",
        margin_overhang(mesh, &region, &margin.points, initial)?
    );
    let raw = margin.region_boundary(mesh)?;
    println!(
        "33 exact region boundary: {}",
        describe(
            &Margin {
                points: raw.clone(),
                vertices: margin.vertices.clone()
            },
            seed,
            initial
        )
    );
    println!(
        "raw loop overhang {:?}",
        margin_overhang(mesh, &region, &raw, initial)?
    );
    sectors(mesh, &margin.points, seed, initial);
    // Reference chain: the exact barrier of the proposal, with the margin-aware
    // axis over it. The app chain below instead follows the stored curve.
    let reference = best_insertion_axis_with_margin(mesh, &region, &raw, Some(initial), 30.0)?;
    println!(
        "reference axis {} tilt {:.3} draft {:.3} area {:.3} overhang {:?}",
        fmt(reference.axis),
        reference.tilt_deg,
        reference.min_draft_deg,
        reference.undercut_area,
        reference.margin_overhang
    );
    let start = std::time::Instant::now();
    match inner_surface(
        mesh,
        &region,
        &raw,
        reference.axis,
        &InnerSurfaceParams::default(),
    ) {
        Ok(inner) => verify(&inner, mesh, &region, &raw, reference.axis)?,
        Err(e) => println!("reference inner ERROR {e}"),
    }
    println!(
        "reference inner elapsed {:.2}s",
        start.elapsed().as_secs_f64()
    );
    // The app's chain: the stored curve is followed densely, so the region's
    // barrier stays within about one triangle of the curve the inner surface
    // seals onto; the axis search seats the margin first.
    let stored = &margin.points;
    let followed = tracer.follow(stored)?;
    let app_region = tracer.region(&followed, seed);
    println!(
        "\napp follow: {} barrier vertices, region {} triangles, overhang vs the stored curve {:?}",
        followed.vertices.len(),
        app_region.len(),
        margin_overhang(mesh, &app_region, stored, initial)?
    );
    let best = best_insertion_axis_with_margin(mesh, &app_region, stored, Some(initial), 30.0)?;
    println!(
        "app-chain axis {} tilt {:.3} draft {:.3} area {:.3} overhang {:?}",
        fmt(best.axis),
        best.tilt_deg,
        best.min_draft_deg,
        best.undercut_area,
        best.margin_overhang
    );
    let start = std::time::Instant::now();
    match inner_surface(
        mesh,
        &app_region,
        stored,
        best.axis,
        &InnerSurfaceParams::default(),
    ) {
        Ok(inner) => {
            verify(&inner, mesh, &app_region, stored, best.axis)?;
        }
        Err(e) => println!("inner ERROR {e}"),
    }
    println!("inner elapsed {:.2}s", start.elapsed().as_secs_f64());
    Ok(())
}

/// Signed true 3D distance from `p` to the preparation.
fn signed_gap(prep: &TriMesh, bvh: &Bvh, p: DVec3) -> f64 {
    let hit = bvh.closest_point(prep, p).unwrap();
    let normal = prep.triangle_cross(hit.triangle as usize).normalize();
    p.distance(hit.point)
        * if (p - hit.point).dot(normal) < 0.0 {
            -1.0
        } else {
            1.0
        }
}

/// Check every vertex, edge midpoint and centroid against its interpolated
/// target and print the clearance extrema plus the worst samples. Samples within
/// the margin seal band are re-measured against the whole scan when the region
/// alone reads them deficient: the smoothed curve crosses the barrier's
/// triangles there and its chords dip into the finish-line valley by the input
/// sampling sagitta, which is not an offset error. Fails when a sample beyond
/// the band misses its target by more than 1 µm, or a band sample by more than
/// 10 µm. Worst samples report height above the margin, sector, gap and target,
/// and whether anything lies above or below them along the axis (a scan hole
/// shows as no hit in either direction).
fn verify(
    inner: &InnerSurface,
    mesh: &TriMesh,
    region: &[u32],
    margin: &[DVec3],
    axis: DVec3,
) -> Result<(), Box<dyn std::error::Error>> {
    let prep = mesh.extract_triangles(region);
    let bvh = Bvh::build(&prep);
    let whole = Bvh::build(mesh);
    let nearest_margin = |p: DVec3| {
        let mut nearest = (f64::INFINITY, DVec3::ZERO);
        for i in 0..margin.len() {
            let a = margin[i];
            let b = margin[(i + 1) % margin.len()];
            let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
            let q = a + (b - a) * t;
            let d = p.distance(q);
            if d < nearest.0 {
                nearest = (d, q);
            }
        }
        nearest
    };
    let mut samples: Vec<(f64, DVec3, f64, f64, bool)> = Vec::new();
    let (mut lo, mut hi) = (f64::INFINITY, 0.0_f64);
    for t in 0..inner.mesh.triangles.len() {
        let [a, b, c] = inner.mesh.triangle(t);
        let [ga, gb, gc] = inner.mesh.triangles[t].map(|v| inner.target_gaps[v as usize]);
        for (p, target) in [
            (a, ga),
            (b, gb),
            (c, gc),
            ((a + b) * 0.5, (ga + gb) * 0.5),
            ((b + c) * 0.5, (gb + gc) * 0.5),
            ((c + a) * 0.5, (gc + ga) * 0.5),
            ((a + b + c) / 3.0, (ga + gb + gc) / 3.0),
        ] {
            let gap = signed_gap(&prep, &bvh, p);
            lo = lo.min(gap);
            hi = hi.max(gap);
            let mut residual = gap - target;
            let mut band = false;
            if residual < -0.001
                && (p - nearest_margin(p).1).dot(axis).abs() <= dentcraft_design::SEAL_BAND_MM
            {
                // The margin's own vicinity: the smoothed curve crosses the
                // barrier there and its 0.1 mm chords dip into the finish-line
                // valley by the sampling sagitta, so re-measure against the scan.
                band = true;
                residual = signed_gap(mesh, &whole, p) - target;
            }
            samples.push((residual, p, gap, target, band));
        }
    }
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let center = margin.iter().sum::<DVec3>() / margin.len() as f64;
    let worst_band = samples
        .iter()
        .filter(|s| s.4)
        .map(|s| s.0)
        .fold(0.0_f64, f64::min);
    let worst_core = samples
        .iter()
        .filter(|s| !s.4)
        .map(|s| s.0)
        .fold(f64::INFINITY, f64::min);
    println!(
        "inner OK {} vertices; signed true 3D gap at vertices/edges/centroids {lo:.6}..{hi:.6} mm; minimum clearance minus target {worst_core:.6} mm beyond the seal band, {worst_band:.6} mm within it (margin sampling sagitta)",
        inner.mesh.positions.len(),
    );
    for &(residual, p, gap, target, band) in samples.iter().take(5) {
        let height = (p - nearest_margin(p).1).dot(axis);
        let d = p - center;
        let side = axis.any_orthonormal_vector();
        let sector = d.dot(axis.cross(side)).atan2(d.dot(side)).to_degrees();
        let up = whole.raycast(mesh, p + axis * 1e-7, axis);
        let down = whole.raycast(mesh, p - axis * 1e-7, -axis);
        println!(
            "worst {} {} residual {residual:.6}: gap {gap:.6} target {target:.6}, height above margin {height:.3} mm, sector {sector:.1} deg, above {:?}, below {:?}",
            if band { "seal band" } else { "core" },
            fmt(p),
            up.map(|h| fmt(h.point)),
            down.map(|h| fmt(h.point)),
        );
    }
    if worst_core < -0.001 || worst_band < -0.01 {
        return Err("inner clearance verification failed".into());
    }
    Ok(())
}

fn sectors(mesh: &TriMesh, margin: &[DVec3], seed: DVec3, axis: DVec3) {
    let helper = if axis.x.abs() < 0.8 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let u = (helper - axis * helper.dot(axis)).normalize();
    let v = axis.cross(u);
    let center = margin.iter().sum::<DVec3>() / margin.len() as f64;
    let origin = center + axis * (seed - center).dot(axis);
    let project = |p: DVec3| DVec2::new((p - origin).dot(u), (p - origin).dot(v));
    let bvh = Bvh::build(mesh);
    println!(
        "sectors about margin centroid: degrees from projected +X toward axis cross +X; depth below seed; first-hit wall radius at margin height + [-0.5,0,0.25,0.5,1,2] mm"
    );
    for sector in 0..24 {
        let angle = (sector as f64 + 0.5) * std::f64::consts::TAU / 24.0;
        let direction = DVec2::new(angle.cos(), angle.sin());
        let radial = u * direction.x + v * direction.y;
        let mut intersection: Option<(f64, DVec3)> = None;
        for i in 0..margin.len() {
            let a = margin[i];
            let b = margin[(i + 1) % margin.len()];
            let pa = project(a);
            let edge = project(b) - pa;
            let denominator = edge.perp_dot(direction);
            if denominator.abs() < 1e-14 {
                continue;
            }
            let t = -pa.perp_dot(direction) / denominator;
            let r = (pa + edge * t).dot(direction);
            if (0.0..=1.0).contains(&t) && r > 0.0 && intersection.is_none_or(|(old, _)| r < old) {
                intersection = Some((r, a.lerp(b, t)));
            }
        }
        let Some((radius, p)) = intersection else {
            continue;
        };
        let depth = (seed - p).dot(axis);
        let radii: Vec<_> = [-0.5, 0.0, 0.25, 0.5, 1.0, 2.0]
            .iter()
            .map(|h| {
                bvh.raycast(mesh, origin - axis * (depth - h), radial)
                    .map_or(f64::NAN, |hit| hit.t)
            })
            .collect();
        println!(
            "{:3}..{:3} depth {:.3} margin radius {:.3} wall {:.3?}",
            sector * 15,
            (sector + 1) * 15,
            depth,
            radius,
            radii
        );
    }
}
