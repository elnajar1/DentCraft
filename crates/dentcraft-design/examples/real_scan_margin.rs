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
//! Scans are never part of the repository; pass a local path.

use std::path::PathBuf;
use std::sync::Arc;

use dentcraft_design::{Margin, MarginTracer, occlusal_direction};
use glam::DVec3;

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
    let tracer = MarginTracer::new(Arc::new(mesh))?;
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
