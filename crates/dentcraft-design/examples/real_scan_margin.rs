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
//! For 33, compares the app's sparse-control retracing with the original region
//! barrier, prints sector wall profiles, and builds the inner surface using the
//! exact barrier and the margin-aware insertion axis. Gap extrema sample true
//! 3D distances at vertices, edge midpoints and triangle centroids.
//! Scans are never part of the repository; pass a local path.

use std::path::PathBuf;
use std::sync::Arc;

use dentcraft_design::{
    InnerSurfaceParams, Margin, MarginTracer, best_insertion_axis, best_insertion_axis_with_margin,
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
    let controls = controls(&margin.points);
    let retraced = tracer.trace(&controls)?;
    let app_region = tracer.region(&retraced, seed);
    let app_axis = best_insertion_axis(mesh, &app_region, Some(initial), 30.0)?;
    let obstruction = margin_overhang(mesh, &app_region, &margin.points, app_axis.axis)?;
    println!(
        "app retraced region {} triangles axis {} tilt {:.3}; overhang {:?}",
        app_region.len(),
        fmt(app_axis.axis),
        app_axis.tilt_deg,
        obstruction
    );
    let original = mesh.extract_triangles(&region);
    let original_bvh = Bvh::build(&original);
    println!(
        "app obstruction distance to original region {:.3} mm",
        obstruction.point.distance(
            original_bvh
                .closest_point(&original, obstruction.point)
                .unwrap()
                .point
        )
    );
    let best = best_insertion_axis(mesh, &region, Some(initial), 30.0)?;
    println!(
        "axis {} tilt {:.3} draft {:.3} undercut area {:.3}",
        fmt(best.axis),
        best.tilt_deg,
        best.min_draft_deg,
        best.undercut_area
    );
    println!(
        "legacy overhang {:?}",
        margin_overhang(mesh, &region, &margin.points, best.axis)?
    );
    sectors(mesh, &margin.points, seed, app_axis.axis);
    let best = best_insertion_axis_with_margin(mesh, &region, &raw, Some(initial), 30.0)?;
    println!(
        "margin-aware axis {} tilt {:.3} draft {:.3} area {:.3} overhang {:?}",
        fmt(best.axis),
        best.tilt_deg,
        best.min_draft_deg,
        best.undercut_area,
        best.margin_overhang
    );
    let start = std::time::Instant::now();
    match inner_surface(
        mesh,
        &region,
        &raw,
        best.axis,
        &InnerSurfaceParams::default(),
    ) {
        Ok(inner) => {
            let prep = mesh.extract_triangles(&region);
            let bvh = Bvh::build(&prep);
            let mut lo = f64::INFINITY;
            let mut hi = 0.0_f64;
            let mut minimum_residual = f64::INFINITY;
            let mut worst = DVec3::ZERO;
            let mut worst_kind = 0;
            for t in 0..inner.mesh.triangles.len() {
                let [a, b, c] = inner.mesh.triangle(t);
                let [ga, gb, gc] = inner.mesh.triangles[t].map(|v| inner.target_gaps[v as usize]);
                for (kind, (p, target)) in [
                    (a, ga),
                    (b, gb),
                    (c, gc),
                    ((a + b) * 0.5, (ga + gb) * 0.5),
                    ((b + c) * 0.5, (gb + gc) * 0.5),
                    ((c + a) * 0.5, (gc + ga) * 0.5),
                    ((a + b + c) / 3.0, (ga + gb + gc) / 3.0),
                ]
                .into_iter()
                .enumerate()
                {
                    let hit = bvh.closest_point(&prep, p).unwrap();
                    let normal = prep.triangle_cross(hit.triangle as usize).normalize();
                    let gap = p.distance(hit.point)
                        * if (p - hit.point).dot(normal) < 0.0 {
                            -1.0
                        } else {
                            1.0
                        };
                    lo = lo.min(gap);
                    hi = hi.max(gap);
                    if gap - target < minimum_residual {
                        minimum_residual = gap - target;
                        worst = p;
                        worst_kind = kind;
                    }
                }
            }
            println!(
                "inner OK {} vertices; signed true 3D gap vertices/edges/centroids {lo:.6}..{hi:.6} mm; minimum clearance minus target {minimum_residual:.6} mm",
                inner.mesh.positions.len()
            );
            if minimum_residual < -0.001 {
                let mut support = f64::NEG_INFINITY;
                for t in 0..prep.triangles.len() {
                    let n = prep.triangle_cross(t).normalize();
                    let offset = n.dot(prep.triangle(t)[0]);
                    if prep.positions.iter().all(|p| n.dot(*p) <= offset + 1e-10) {
                        support = support.max(n.dot(worst) - offset);
                    }
                }
                println!(
                    "worst point {} kind {worst_kind} certified supporting plane distance {support:.6}; upward hit {:?}",
                    fmt(worst),
                    bvh.raycast(&prep, worst + best.axis * 1e-7, best.axis)
                );
                return Err("inner clearance verification failed".into());
            }
        }
        Err(e) => println!("inner ERROR {e}"),
    }
    println!("inner elapsed {:.2}s", start.elapsed().as_secs_f64());
    Ok(())
}

fn controls(curve: &[DVec3]) -> Vec<DVec3> {
    let n = curve.len();
    let total: f64 = (0..n).map(|i| curve[i].distance(curve[(i + 1) % n])).sum();
    let count = ((total / 1.5).round() as usize).clamp(16, 24);
    let (mut segment, mut walked) = (0, 0.0);
    (0..count)
        .map(|k| {
            let target = total * k as f64 / count as f64;
            loop {
                let a = curve[segment % n];
                let b = curve[(segment + 1) % n];
                let length = a.distance(b);
                if walked + length >= target - 1e-12 || segment + 1 >= n {
                    return a.lerp(b, ((target - walked) / length).clamp(0.0, 1.0));
                }
                walked += length;
                segment += 1;
            }
        })
        .collect()
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
