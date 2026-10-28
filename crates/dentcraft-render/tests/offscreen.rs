//! Renders scenes offscreen and checks pixels. Skips (and passes) without a GPU adapter.
//!
//! Most checks look down the occlusal view, where the camera's right is +X and its up is +Y, so
//! the left of the image is −X.

use dentcraft_mesh::{Aabb, TriMesh};
use dentcraft_render::{
    Camera, ClipPlane, DrawItem, Headless, MeshData, MeshId, Overlay, RenderOptions, ViewPreset,
    colormap,
};
use glam::{DVec3, Vec2, Vec3};
use image::{Rgba, RgbaImage};

const W: u32 = 256;
const H: u32 = 256;
const SPHERE: MeshId = MeshId(1);

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

/// A square in the plane z = `z`, facing +Z (towards the occlusal camera).
fn quad(half: f64, z: f64) -> TriMesh {
    TriMesh::new(
        vec![
            DVec3::new(-half, -half, z),
            DVec3::new(half, -half, z),
            DVec3::new(half, half, z),
            DVec3::new(-half, half, z),
        ],
        vec![[0, 1, 2], [0, 2, 3]],
    )
}

fn gpu() -> Option<Headless> {
    let Some(gpu) = Headless::new() else {
        println!("note: no GPU adapter available; offscreen render test skipped");
        return None;
    };
    let info = gpu.adapter_info();
    println!(
        "rendering with GPU adapter: {} ({:?})",
        info.name, info.backend
    );
    Some(gpu)
}

/// The radius-5 sphere's box, which frames every occlusal scene below.
fn sphere_bounds() -> Aabb {
    Aabb {
        min: DVec3::splat(-5.0),
        max: DVec3::splat(5.0),
    }
}

fn occlusal() -> Camera {
    let mut camera = Camera::default();
    camera.aspect = W as f32 / H as f32;
    camera.set_view(ViewPreset::Occlusal, Some(&sphere_bounds()));
    camera
}

/// A flat background, so any change of a pixel comes from the scene.
fn flat() -> RenderOptions {
    RenderOptions {
        vignette: 0.0,
        background_top: [0.2, 0.2, 0.2],
        background_bottom: [0.2, 0.2, 0.2],
        ..RenderOptions::dark()
    }
}

fn item(mesh: MeshId, color: [f32; 3]) -> DrawItem {
    DrawItem {
        mesh,
        color,
        opacity: 1.0,
        visible: true,
        vertex_colors: false,
        clip: false,
    }
}

/// The image pixel that `point` projects into.
fn pixel_of(camera: &Camera, point: Vec3) -> (u32, u32) {
    let p = camera
        .project_to_pixel(point, Vec2::ZERO, Vec2::new(W as f32, H as f32))
        .expect("point is in front of the camera");
    (p.x as u32, p.y as u32)
}

fn rgb(image: &RgbaImage, (x, y): (u32, u32)) -> [i32; 3] {
    let Rgba([r, g, b, _]) = *image.get_pixel(x, y);
    [r, g, b].map(i32::from)
}

/// Largest per-channel difference.
fn diff(a: [i32; 3], b: [i32; 3]) -> i32 {
    (0..3).map(|i| (a[i] - b[i]).abs()).max().unwrap_or(0)
}

fn srgb_bytes(c: [f32; 3]) -> [i32; 3] {
    c.map(|v| (v * 255.0).round() as i32)
}

#[test]
fn renders_a_sphere_offscreen() {
    let Some(mut gpu) = gpu() else { return };

    let sphere = uv_sphere(5.0, 32, 64);
    let data = MeshData::from_mesh(&sphere);
    let bounds = data.bounds.unwrap();
    gpu.upload(SPHERE, &data);

    let (w, h) = (320, 200);
    let mut camera = Camera::default();
    camera.aspect = w as f32 / h as f32;
    camera.set_view(ViewPreset::Facial, Some(&bounds));
    let item = item(SPHERE, [0.9, 0.85, 0.75]);
    let options = flat();

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

#[test]
fn vertex_colours_paint_the_mesh() {
    let Some(mut gpu) = gpu() else { return };
    let sphere = uv_sphere(5.0, 32, 64);
    let colors: Vec<[f32; 4]> = sphere
        .positions
        .iter()
        .map(|p| {
            if p.x > 1e-9 {
                [1.0, 0.0, 0.0, 1.0]
            } else if p.x < -1e-9 {
                [0.0, 0.0, 1.0, 1.0]
            } else {
                // On the dividing meridian: keep the base colour.
                [0.0, 0.0, 0.0, 0.0]
            }
        })
        .collect();
    gpu.upload(SPHERE, &MeshData::with_colors(&sphere, &colors).unwrap());
    let camera = occlusal();
    let painted = DrawItem {
        vertex_colors: true,
        ..item(SPHERE, [0.9, 0.85, 0.75])
    };
    let image = gpu.render_to_image(W, H, &camera, &[painted], &flat());

    let left = rgb(&image, (W * 3 / 10, H / 2));
    let right = rgb(&image, (W * 7 / 10, H / 2));
    assert!(
        left[2] > left[0] + 60,
        "the −X side shows its blue vertex colours: {left:?}"
    );
    assert!(
        right[0] > right[2] + 60,
        "the +X side shows its red vertex colours: {right:?}"
    );

    // Without the flag the same mesh keeps its base colour on both sides.
    let plain = gpu.render_to_image(
        W,
        H,
        &camera,
        &[DrawItem {
            vertex_colors: false,
            ..painted
        }],
        &flat(),
    );
    for x in [W * 3 / 10, W * 7 / 10] {
        let c = rgb(&plain, (x, H / 2));
        assert!(c[0] > c[2], "base colour without the flag: {c:?}");
    }
}

#[test]
fn polyline_draws_on_an_empty_view() {
    let Some(mut gpu) = gpu() else { return };
    let camera = occlusal();
    let color = [0.1, 0.9, 0.2];
    let line = Overlay::Polyline {
        points: vec![Vec3::new(-6.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0)],
        closed: false,
        color,
        width_px: 4.0,
    };
    let empty = gpu.render_to_image(W, H, &camera, &[], &flat());
    let image = gpu.render_to_image_with_overlays(W, H, &camera, &[], &[line], &flat());
    let expected = srgb_bytes(color);
    for x in [-4.0, -1.5, 0.0, 2.5, 5.0] {
        let on = pixel_of(&camera, Vec3::new(x, 0.0, 0.0));
        assert!(
            diff(rgb(&image, on), expected) <= 3,
            "line colour at x = {x} ({on:?}): {:?}",
            rgb(&image, on)
        );
    }
    // Off the path and beyond its ends the background is untouched.
    for off in [
        pixel_of(&camera, Vec3::new(0.0, 2.0, 0.0)),
        pixel_of(&camera, Vec3::new(7.0, 0.0, 0.0)),
    ] {
        assert_eq!(image.get_pixel(off.0, off.1), empty.get_pixel(off.0, off.1));
    }
}

#[test]
fn lines_behind_geometry_are_dimmed_not_hidden() {
    let Some(mut gpu) = gpu() else { return };
    let plate = MeshId(2);
    gpu.upload(plate, &MeshData::from_mesh(&quad(8.0, 0.0)));
    let camera = occlusal();
    let items = [item(plate, [0.6, 0.6, 0.6])];
    let color = [0.1, 0.9, 0.2];
    let line_at = |z: f32| Overlay::Polyline {
        points: vec![Vec3::new(-6.0, 0.0, z), Vec3::new(6.0, 0.0, z)],
        closed: false,
        color,
        width_px: 4.0,
    };
    let at = (W / 2, H / 2);
    let alone = rgb(&gpu.render_to_image(W, H, &camera, &items, &flat()), at);
    let behind = rgb(
        &gpu.render_to_image_with_overlays(W, H, &camera, &items, &[line_at(-3.0)], &flat()),
        at,
    );
    let in_front = rgb(
        &gpu.render_to_image_with_overlays(W, H, &camera, &items, &[line_at(3.0)], &flat()),
        at,
    );
    // Green over red/blue: 0 for the grey plate, high for the line.
    let green = |c: [i32; 3]| c[1] - (c[0] + c[2]) / 2;
    assert!(
        green(behind) > green(alone) + 20,
        "a line behind the plate still shows: {alone:?} -> {behind:?}"
    );
    assert!(
        green(in_front) > green(behind) + 40,
        "and is dimmer than in front: {behind:?} vs {in_front:?}"
    );
    assert!(diff(in_front, srgb_bytes(color)) <= 3, "{in_front:?}");
}

#[test]
fn hidden_polyline_joints_blend_once() {
    let Some(mut gpu) = gpu() else { return };
    let plate = MeshId(2);
    gpu.upload(plate, &MeshData::from_mesh(&quad(8.0, 0.0)));
    let camera = occlusal();
    let items = [item(plate, [0.6, 0.6, 0.6])];
    // A sharp V behind the plate: the two segments overlap at the joint.
    let joint = Vec3::new(0.0, 4.0, -3.0);
    let line = Overlay::Polyline {
        points: vec![
            Vec3::new(-6.0, -4.0, -3.0),
            joint,
            Vec3::new(6.0, -4.0, -3.0),
        ],
        closed: false,
        color: [0.1, 0.9, 0.2],
        width_px: 6.0,
    };
    let image = gpu.render_to_image_with_overlays(W, H, &camera, &items, &[line], &flat());
    let at_joint = rgb(&image, pixel_of(&camera, joint));
    let mid = rgb(&image, pixel_of(&camera, Vec3::new(-3.0, 0.0, -3.0)));
    assert!(
        diff(at_joint, mid) <= 3,
        "the joint is as dim as the segments: joint {at_joint:?}, segment {mid:?}"
    );
}

#[test]
fn transparent_items_blend_and_sort_back_to_front() {
    let Some(mut gpu) = gpu() else { return };
    gpu.upload(SPHERE, &MeshData::from_mesh(&uv_sphere(5.0, 32, 64)));
    let (glass, near, far) = (MeshId(2), MeshId(3), MeshId(4));
    gpu.upload(glass, &MeshData::from_mesh(&quad(3.0, 7.0)));
    gpu.upload(near, &MeshData::from_mesh(&quad(4.0, 2.0)));
    gpu.upload(far, &MeshData::from_mesh(&quad(4.0, -2.0)));
    let camera = occlusal();
    let at = (W / 2, H / 2);
    let red = item(SPHERE, [0.9, 0.1, 0.1]);
    let blue_glass = DrawItem {
        opacity: 0.5,
        ..item(glass, [0.1, 0.2, 0.9])
    };

    let sphere = rgb(&gpu.render_to_image(W, H, &camera, &[red], &flat()), at);
    let glass_only = rgb(
        &gpu.render_to_image(W, H, &camera, &[blue_glass], &flat()),
        at,
    );
    // Passed first, drawn last: the transparent quad covers the opaque sphere.
    let mixed = rgb(
        &gpu.render_to_image(W, H, &camera, &[blue_glass, red], &flat()),
        at,
    );
    assert!(
        mixed[2] > sphere[2] + 30 && mixed[0] > glass_only[0] + 30,
        "both show: sphere {sphere:?}, glass {glass_only:?}, mixed {mixed:?}"
    );

    // Two transparent quads: the same picture whichever order they are passed in.
    let blue_near = DrawItem {
        opacity: 0.5,
        ..item(near, [0.1, 0.2, 0.9])
    };
    let red_far = DrawItem {
        opacity: 0.5,
        ..item(far, [0.9, 0.1, 0.1])
    };
    let a = gpu.render_to_image(W, H, &camera, &[blue_near, red_far], &flat());
    let b = gpu.render_to_image(W, H, &camera, &[red_far, blue_near], &flat());
    assert!(a == b, "transparent draw order depends on the item order");
    let both = rgb(&a, at);
    assert!(
        both[2] > both[0],
        "the nearer (blue) quad is on top: {both:?}"
    );
}

#[test]
fn section_plane_cuts_and_caps() {
    let Some(mut gpu) = gpu() else { return };
    gpu.upload(SPHERE, &MeshData::from_mesh(&uv_sphere(5.0, 32, 64)));
    let camera = occlusal();
    let cut = DrawItem {
        clip: true,
        ..item(SPHERE, [0.9, 0.85, 0.75])
    };
    let options = flat();
    let center = (W / 2, H / 2);
    let (left, right) = ((W * 3 / 10, H / 2), (W * 7 / 10, H / 2));
    let section = srgb_bytes(options.section);

    let whole = gpu.render_to_image(W, H, &camera, &[cut], &options);
    assert!(diff(rgb(&whole, center), section) > 30, "no plane, no cap");

    // Cutting away the half towards the eye shows the inside: the flat section colour.
    let towards_eye = RenderOptions {
        clip: Some(ClipPlane {
            point: Vec3::ZERO,
            normal: -camera.forward(),
        }),
        ..options
    };
    let capped = gpu.render_to_image(W, H, &camera, &[cut], &towards_eye);
    assert!(
        diff(rgb(&capped, center), section) <= 3,
        "section colour {section:?} at the centre: {:?}",
        rgb(&capped, center)
    );

    // Cutting away the right half shows the background there and leaves the left alone.
    let right_away = RenderOptions {
        clip: Some(ClipPlane {
            point: Vec3::ZERO,
            normal: camera.right(),
        }),
        ..options
    };
    let empty = gpu.render_to_image(W, H, &camera, &[], &options);
    let halved = gpu.render_to_image(W, H, &camera, &[cut], &right_away);
    assert_eq!(
        rgb(&halved, right),
        rgb(&empty, right),
        "the right half is cut away"
    );
    assert_eq!(rgb(&halved, left), rgb(&whole, left), "the left half stays");

    // Items that do not opt in are not cut.
    let kept = gpu.render_to_image(
        W,
        H,
        &camera,
        &[DrawItem { clip: false, ..cut }],
        &right_away,
    );
    assert_eq!(rgb(&kept, right), rgb(&whole, right));
}

#[test]
fn point_overlay_lands_where_the_camera_projects_it() {
    let Some(mut gpu) = gpu() else { return };
    let camera = occlusal();
    let p = Vec3::new(2.5, -1.5, 1.0);
    let color = [0.95, 0.8, 0.1];
    let handle = Overlay::Points {
        points: vec![p],
        color,
        radius_px: 5.0,
        outline: None,
    };
    let empty = gpu.render_to_image(W, H, &camera, &[], &flat());
    let image = gpu.render_to_image_with_overlays(W, H, &camera, &[], &[handle], &flat());
    let (x, y) = pixel_of(&camera, p);
    for (dx, dy) in [(0, 0), (-2, 0), (2, 0), (0, -2), (0, 2)] {
        let at = (x.wrapping_add_signed(dx), y.wrapping_add_signed(dy));
        assert!(
            diff(rgb(&image, at), srgb_bytes(color)) <= 3,
            "handle colour at {at:?}: {:?}",
            rgb(&image, at)
        );
    }
    for at in [(x + 9, y), (x, y + 9), (x - 9, y)] {
        assert_eq!(
            rgb(&image, at),
            rgb(&empty, at),
            "outside the handle at {at:?}"
        );
    }
}

/// Writes `scratch/shots/render-features.png`: every renderer feature in one image.
/// Run with `cargo test -p dentcraft-render --test offscreen -- --ignored demo_render_features`.
#[test]
#[ignore = "writes a demo image"]
fn demo_render_features() {
    let Some(mut gpu) = gpu() else { return };
    let sphere = uv_sphere(5.0, 96, 192);
    // A smooth signed field over the sphere, colour-mapped blue–grey–red.
    let colors: Vec<[f32; 4]> = sphere
        .positions
        .iter()
        .map(|p| {
            let v = (p.z * 0.35 + (p.x * 0.9).sin() * 1.2) as f32;
            colormap::diverging(v, -2.5, 2.5)
        })
        .collect();
    gpu.upload(SPHERE, &MeshData::with_colors(&sphere, &colors).unwrap());
    let shell = MeshId(2);
    gpu.upload(shell, &MeshData::from_mesh(&uv_sphere(7.0, 64, 128)));

    let (w, h) = (960, 720);
    let mut camera = Camera::default();
    camera.aspect = w as f32 / h as f32;
    let bounds = Aabb {
        min: DVec3::splat(-7.0),
        max: DVec3::splat(7.0),
    };
    camera.set_view(ViewPreset::Facial, Some(&bounds));
    camera.set_scene(&bounds);
    camera.orbit(Vec2::new(-110.0, 60.0));

    let ring: Vec<Vec3> = (0..96)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 96.0;
            Vec3::new(5.0 * a.cos(), 5.0 * a.sin(), 0.0)
        })
        .collect();
    let handles: Vec<Vec3> = ring.iter().step_by(16).copied().collect();
    let overlays = [
        Overlay::Polyline {
            points: ring,
            closed: true,
            color: [1.0, 0.85, 0.2],
            width_px: 3.0,
        },
        Overlay::Points {
            points: handles,
            color: [1.0, 1.0, 1.0],
            radius_px: 6.0,
            outline: Some([0.1, 0.35, 0.8]),
        },
    ];
    let items = [
        DrawItem {
            vertex_colors: true,
            clip: true,
            ..item(SPHERE, [0.9, 0.85, 0.75])
        },
        DrawItem {
            opacity: 0.22,
            ..item(shell, [0.75, 0.85, 1.0])
        },
    ];
    let options = RenderOptions {
        clip: Some(ClipPlane {
            point: Vec3::new(0.0, 2.5, 0.0),
            normal: Vec3::new(0.25, 1.0, 0.35),
        }),
        ..RenderOptions::dark()
    };
    let image = gpu.render_to_image_with_overlays(w, h, &camera, &items, &overlays, &options);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch/shots");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("render-features.png");
    image.save(&path).unwrap();
    println!("wrote {}", path.display());
}
