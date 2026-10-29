//! Preview renders of the whole library (ignored by default).
//!
//! Run with:
//! `cargo test -p dentcraft-anatomy --release -- --ignored preview_all_teeth --nocapture`
//!
//! Writes fixed-distance 30 mm tiles (all teeth at the same scale) into `scratch/shots/`
//! inside the worktree: `teeth-occlusal.png` and `teeth-facial.png` (upper row, lower row, FDI
//! order) plus a larger occlusal view of tooth 36. The folder is gitignored; the images are
//! never committed.

use dentcraft_anatomy::{CrownParams, crown};
use dentcraft_case::{Arch, Fdi};
use dentcraft_mesh::Aabb;
use dentcraft_render::{Camera, DrawItem, Headless, MeshData, MeshId, RenderOptions, ViewPreset};
use image::{GenericImage, Rgba, RgbaImage};

const TILE: u32 = 150;
/// Fixed tile camera distance, so every tooth renders at the same scale.
const DISTANCE: f32 = 30.0;
const FOV_Y: f32 = 30f32.to_radians();
const ENAMEL: [f32; 3] = [0.93, 0.90, 0.86];

fn shots_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch/shots")
}

fn camera_for(bounds: &Aabb, rotation: glam::Quat) -> Camera {
    let mut camera = Camera::default();
    camera.rotation = rotation;
    camera.target = bounds.center().as_vec3();
    camera.distance = DISTANCE;
    camera.fov_y = FOV_Y;
    camera.aspect = 1.0;
    camera.set_scene(bounds);
    camera
}

fn tile(
    gpu: &mut Headless,
    item: &DrawItem,
    bounds: &Aabb,
    rotation: glam::Quat,
    size: u32,
) -> RgbaImage {
    let camera = camera_for(bounds, rotation);
    gpu.render_to_image(
        size,
        size,
        &camera,
        std::slice::from_ref(item),
        &RenderOptions::light(),
    )
}

fn compose(tiles: &[RgbaImage], columns: u32, rows: u32, tile: u32) -> RgbaImage {
    let mut canvas = RgbaImage::from_pixel(columns * tile, rows * tile, Rgba::from([0, 0, 0, 0]));
    for (i, t) in tiles.iter().enumerate() {
        let (x, y) = ((i as u32 % columns) * tile, (i as u32 / columns) * tile);
        canvas.copy_from(t, x, y).unwrap();
    }
    canvas
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

    let teeth: Vec<Fdi> = Arch::Upper.teeth().chain(Arch::Lower.teeth()).collect();
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

    for (preset, name) in [
        (ViewPreset::Occlusal, "teeth-occlusal.png"),
        (ViewPreset::Facial, "teeth-facial.png"),
    ] {
        let tiles: Vec<RgbaImage> = templates
            .iter()
            .zip(&items)
            .map(|(t, item)| {
                tile(
                    &mut gpu,
                    item,
                    &t.mesh.bounds().unwrap(),
                    preset.rotation(),
                    TILE,
                )
            })
            .collect();
        let sheet = compose(&tiles, 16, 2, TILE);
        // 16 teeth per row, upper arch then lower arch, patient right to left.
        let path = dir.join(name);
        sheet.save(&path).unwrap();
        println!("wrote {}", path.display());
        assert!(path.is_file());
    }

    // A closer look at the lower left first molar (36), occlusal.
    let index36 = teeth.iter().position(|f| f.code() == 36).unwrap();
    let big = tile(
        &mut gpu,
        &items[index36],
        &templates[index36].mesh.bounds().unwrap(),
        ViewPreset::Occlusal.rotation(),
        512,
    );
    let path36 = dir.join("tooth-36.png");
    big.save(&path36).unwrap();
    println!("wrote {}", path36.display());
    assert!(path36.is_file());
}
