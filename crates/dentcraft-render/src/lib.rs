//! DentCraft's 3D view: an orbit camera with dental view presets and a wgpu mesh renderer with a
//! soft "dental stone" look.
//!
//! The renderer draws into its own multisampled offscreen target. An app then either blits that
//! target into its UI render pass ([`MeshRenderer::blit`]) or reads it back
//! ([`Headless::render_to_image`]).

mod camera;
pub mod colormap;
mod headless;
mod renderer;

pub use camera::{Camera, ViewPreset};
pub use headless::Headless;
pub use renderer::{
    ClipPlane, ColorCountError, DrawItem, MeshData, MeshId, MeshRenderer, Overlay, RenderOptions,
    SCENE_FORMAT, Vertex,
};
pub use wgpu;
