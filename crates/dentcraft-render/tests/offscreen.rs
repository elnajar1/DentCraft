//! Renders a sphere offscreen and checks the image. Skips (and passes) without a GPU adapter.

use dentcraft_mesh::TriMesh;
use dentcraft_render::{Camera, DrawItem, Headless, MeshData, MeshId, RenderOptions, ViewPreset};
use glam::DVec3;

fn uv_sphere(radius: f64, rings: u32, segments: u32) -> TriMesh {
    let mut positions = vec![DVec3::new(0.0, 0.0, radius)];
    for ring in 1..rings {
        let theta = std::f64::consts::PI * f64::from(ring) / f64::from(rings);
        for seg in 0..segments {
            let phi = std::f64::consts::TAU * f64::from(seg) / f64::from(segments);
            positions.push(DVec3::new(
                radius * theta.sin() * phi.cos(),
                radius * theta.sin() * phi.sin(),
                radius * theta.cos(),
            ));
        }
    }
    positions.push(DVec3::new(0.0, 0.0, -radius));
    let south = positions.len() as u32 - 1;
    let at = |ring: u32, seg: u32| 1 + (ring - 1) * segments + seg % segments;
    let mut triangles = Vec::new();
    for seg in 0..segments {
        triangles.push([0, at(1, seg), at(1, seg + 1)]);
        triangles.push([south, at(rings - 1, seg + 1), at(rings - 1, seg)]);
    }
    for ring in 1..rings - 1 {
        for seg in 0..segments {
            let (a, b) = (at(ring, seg), at(ring, seg + 1));
            let (c, d) = (at(ring + 1, seg), at(ring + 1, seg + 1));
            triangles.push([a, c, d]);
            triangles.push([a, d, b]);
        }
    }
    TriMesh::new(positions, triangles)
}

#[test]
fn renders_a_sphere_offscreen() {
    let Some(mut gpu) = Headless::new() else {
        println!("note: no GPU adapter available; offscreen render test skipped");
        return;
    };
    let info = gpu.adapter_info();
    println!(
        "rendering with GPU adapter: {} ({:?})",
        info.name, info.backend
    );

    let sphere = uv_sphere(5.0, 32, 64);
    let data = MeshData::from_mesh(&sphere);
    let bounds = data.bounds.unwrap();
    gpu.upload(MeshId(1), &data);

    let (w, h) = (320, 200);
    let mut camera = Camera::default();
    camera.aspect = w as f32 / h as f32;
    camera.set_view(ViewPreset::Facial, Some(&bounds));
    let item = DrawItem {
        mesh: MeshId(1),
        color: [0.9, 0.85, 0.75],
        opacity: 1.0,
        visible: true,
    };
    let options = RenderOptions {
        vignette: 0.0,
        background_top: [0.2, 0.2, 0.2],
        background_bottom: [0.2, 0.2, 0.2],
        ..RenderOptions::dark()
    };

    let empty = gpu.render_to_image(w, h, &camera, &[], &options);
    let image = gpu.render_to_image(w, h, &camera, &[item], &options);
    assert_eq!(image.dimensions(), (w, h));

    let center = *image.get_pixel(w / 2, h / 2);
    let corner = *image.get_pixel(2, 2);
    assert_eq!(
        corner,
        *empty.get_pixel(2, 2),
        "corner shows the background"
    );
    assert_ne!(
        center,
        *empty.get_pixel(w / 2, h / 2),
        "the sphere covers the centre"
    );
    assert!(
        center.0[..3].iter().map(|&c| u32::from(c)).sum::<u32>() > 3 * 90,
        "the sphere is lit: {center:?}"
    );

    // A hidden mesh draws nothing.
    let hidden = gpu.render_to_image(
        w,
        h,
        &camera,
        &[DrawItem {
            visible: false,
            ..item
        }],
        &options,
    );
    assert_eq!(
        hidden.get_pixel(w / 2, h / 2),
        empty.get_pixel(w / 2, h / 2)
    );
}
