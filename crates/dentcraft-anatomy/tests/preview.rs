//! Preview renders of the whole library (ignored by default).
//!
//! Run with:
//! `cargo test -p dentcraft-anatomy --release -- --ignored preview_all_teeth --nocapture`
//!
//! Writes into `scratch/shots/` inside the worktree (gitignored; the images are never
//! committed), every tile from the same camera distance so all teeth share one scale:
//!
//! - `teeth-occlusal.png`: occlusal views, facial side down, mesial towards the arch midline,
//!   upper row 18…11 21…28 and lower row 48…41 31…38 (patient's right to left). The lower row
//!   is a direct view; the upper row is mirrored like an occlusal mirror photograph.
//! - `teeth-facial.png`: facial views, upper crowns pointing down, lower crowns up, mesial
//!   towards the midline (direct views, as seen facing the patient).
//! - `tooth-36.png`, `tooth-16.png`, `tooth-11.png`, `tooth-13.png`: 512 px mesio-facial
//!   three-quarter close-ups, crown up, dark theme.
//!
//! `preview_tooth_details` writes `detail-NN.png` per generated tooth (see there).

use dentcraft_anatomy::{CrownParams, crown};
use dentcraft_case::{Arch, Fdi};
use dentcraft_mesh::Aabb;
use dentcraft_render::{Camera, DrawItem, Headless, MeshData, MeshId, RenderOptions};
use glam::{Mat3, Quat, Vec3};
use image::{GenericImage, Rgba, RgbaImage, imageops::flip_horizontal};

const TILE: u32 = 150;
/// Fixed tile camera distance, so every tooth renders at the same scale.
const DISTANCE: f32 = 30.0;
/// Fixed close-up camera distance (a three-quarter view needs more room than a tile).
const CLOSE_DISTANCE: f32 = 34.0;
const CLOSE_SIZE: u32 = 512;
const FOV_Y: f32 = 30f32.to_radians();
const ENAMEL: [f32; 3] = [0.93, 0.90, 0.86];

fn shots_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch/shots")
}

/// Camera-to-world rotation looking along `-back` with `up` towards the top of the screen,
/// built like `ViewPreset::rotation`.
fn look(back: Vec3, up: Vec3) -> Quat {
    let back = back.normalize();
    let right = up.cross(back).normalize();
    let up = back.cross(right);
    Quat::from_mat3(&Mat3::from_cols(right, up, back)).normalize()
}

fn render(
    gpu: &mut Headless,
    item: &DrawItem,
    bounds: &Aabb,
    rotation: Quat,
    distance: f32,
    size: u32,
    options: &RenderOptions,
) -> RgbaImage {
    let mut camera = Camera::default();
    camera.rotation = rotation;
    camera.target = bounds.center().as_vec3();
    camera.distance = distance;
    camera.fov_y = FOV_Y;
    camera.aspect = 1.0;
    camera.set_scene(bounds);
    gpu.render_to_image(size, size, &camera, std::slice::from_ref(item), options)
}

fn compose(tiles: &[RgbaImage], columns: u32, rows: u32, tile: u32) -> RgbaImage {
    let mut canvas = RgbaImage::from_pixel(columns * tile, rows * tile, Rgba::from([0, 0, 0, 0]));
    for (i, t) in tiles.iter().enumerate() {
        let (x, y) = ((i as u32 % columns) * tile, (i as u32 / columns) * tile);
        canvas.copy_from(t, x, y).unwrap();
    }
    canvas
}

/// The 32 teeth in sheet order: 18…11 21…28, then 48…41 31…38.
fn sheet_order() -> Vec<Fdi> {
    let quadrant = |q: u8, reverse: bool| {
        let mut v: Vec<Fdi> = (1..=8).map(|p| Fdi::new(q * 10 + p).unwrap()).collect();
        if reverse {
            v.reverse();
        }
        v
    };
    [
        quadrant(1, true),
        quadrant(2, false),
        quadrant(4, true),
        quadrant(3, false),
    ]
    .concat()
}

#[test]
#[ignore = "writes preview images and needs a GPU adapter"]
fn preview_all_teeth() {
    let Some(mut gpu) = Headless::new() else {
        println!("note: no GPU adapter available; preview not rendered");
        return;
    };
    let info = gpu.adapter_info();
    println!(
        "rendering with GPU adapter: {} ({:?})",
        info.name, info.backend
    );

    let teeth = sheet_order();
    let templates: Vec<_> = teeth
        .iter()
        .map(|&fdi| crown(fdi, &CrownParams::default()))
        .collect();
    for (i, t) in templates.iter().enumerate() {
        gpu.upload(MeshId(i as u64 + 1), &MeshData::from_mesh(&t.mesh));
    }
    let items: Vec<DrawItem> = (0..templates.len())
        .map(|i| DrawItem {
            mesh: MeshId(i as u64 + 1),
            color: ENAMEL,
            opacity: 1.0,
            visible: true,
            vertex_colors: false,
            clip: false,
        })
        .collect();

    let dir = shots_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let light = RenderOptions::light();

    // Occlusal: facial down. Screen right is then -X, i.e. mesial for quadrants 2 and 4 and
    // distal for 1 and 3; the left half of each row needs mesial to the right, so quadrants 1
    // and 2 (the upper row) are flipped and the lower row is a direct view.
    let occlusal: Vec<RgbaImage> = teeth
        .iter()
        .zip(templates.iter().zip(&items))
        .map(|(fdi, (t, item))| {
            let rotation = look(Vec3::Z, Vec3::NEG_Y);
            let bounds = t.mesh.bounds().unwrap();
            let img = render(&mut gpu, item, &bounds, rotation, DISTANCE, TILE, &light);
            if fdi.arch() == Arch::Upper {
                flip_horizontal(&img)
            } else {
                img
            }
        })
        .collect();
    // Facial: facial towards the viewer, crowns pointing to the occlusal plane. Screen right is
    // +X for uppers and -X for lowers: mesial towards the midline in every quadrant.
    let facial: Vec<RgbaImage> = teeth
        .iter()
        .zip(templates.iter().zip(&items))
        .map(|(fdi, (t, item))| {
            let up = if fdi.arch() == Arch::Upper {
                Vec3::NEG_Z
            } else {
                Vec3::Z
            };
            let bounds = t.mesh.bounds().unwrap();
            render(
                &mut gpu,
                item,
                &bounds,
                look(Vec3::Y, up),
                DISTANCE,
                TILE,
                &light,
            )
        })
        .collect();
    for (tiles, name) in [
        (occlusal, "teeth-occlusal.png"),
        (facial, "teeth-facial.png"),
    ] {
        let sheet = compose(&tiles, 16, 2, TILE);
        let path = dir.join(name);
        sheet.save(&path).unwrap();
        println!("wrote {}", path.display());
        assert!(path.is_file());
    }

    // Three-quarter close-ups from mesial, facial and occlusal (all four are generated teeth,
    // +X mesial), crown up. The dark theme: the light background washes out the silhouette.
    let theme = RenderOptions::dark();
    for code in [36u8, 16, 11, 13] {
        let i = teeth.iter().position(|f| f.code() == code).unwrap();
        let back = Vec3::new(0.5, 0.8, 0.6);
        let img = render(
            &mut gpu,
            &items[i],
            &templates[i].mesh.bounds().unwrap(),
            look(back, Vec3::Z),
            CLOSE_DISTANCE,
            CLOSE_SIZE,
            &theme,
        );
        let path = dir.join(format!("tooth-{code}.png"));
        img.save(&path).unwrap();
        println!("wrote {}", path.display());
        assert!(path.is_file());
    }
}

/// Per-tooth detail sheets for the 16 generated teeth (quadrants 1 and 3), 4 × 256 px:
/// occlusal (facial down, direct view), occlusal height contours (0.25 mm bands, blue low to
/// red high), facial and mesial (crown up). Written as `detail-NN.png`.
#[test]
#[ignore = "writes preview images and needs a GPU adapter"]
fn preview_tooth_details() {
    let Some(mut gpu) = Headless::new() else {
        println!("note: no GPU adapter available; preview not rendered");
        return;
    };
    let dir = shots_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let theme = RenderOptions::dark();
    let codes = (11u8..=18).chain(31..=38);
    for (k, code) in codes.enumerate() {
        let t = crown(Fdi::new(code).unwrap(), &CrownParams::default());
        let bounds = t.mesh.bounds().unwrap();
        let (z0, z1) = (bounds.max.z - 4.0, bounds.max.z);
        let colors: Vec<[f32; 4]> = t
            .mesh
            .positions
            .iter()
            .map(|p| {
                let f = (((p.z - z0) / (z1 - z0)) as f32).clamp(0.0, 1.0);
                let band = ((p.z / 0.25).floor() as i64).rem_euclid(2) as f32;
                let s = 0.75 + 0.25 * band;
                [s * f, s * (1.0 - (2.0 * f - 1.0).abs()), s * (1.0 - f), 1.0]
            })
            .collect();
        let shaded_id = MeshId(1000 + 2 * k as u64);
        let banded_id = MeshId(1001 + 2 * k as u64);
        gpu.upload(shaded_id, &MeshData::from_mesh(&t.mesh));
        gpu.upload(banded_id, &MeshData::with_colors(&t.mesh, &colors).unwrap());
        let item = |mesh, vertex_colors| DrawItem {
            mesh,
            color: ENAMEL,
            opacity: 1.0,
            visible: true,
            vertex_colors,
            clip: false,
        };
        let (shaded, banded) = (item(shaded_id, false), item(banded_id, true));
        let views = [
            (&shaded, look(Vec3::Z, Vec3::NEG_Y)),
            (&banded, look(Vec3::Z, Vec3::NEG_Y)),
            (&shaded, look(Vec3::Y, Vec3::Z)),
            (&shaded, look(Vec3::X, Vec3::Z)),
        ];
        let tiles: Vec<RgbaImage> = views
            .iter()
            .map(|(it, rot)| render(&mut gpu, it, &bounds, *rot, 22.0, 256, &theme))
            .collect();
        let path = dir.join(format!("detail-{code}.png"));
        compose(&tiles, 4, 1, 256).save(&path).unwrap();
        println!("wrote {}", path.display());
    }
}
