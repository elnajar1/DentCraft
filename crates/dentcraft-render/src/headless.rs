//! Rendering without a window, for tests, documentation images and batch thumbnails.

use image::RgbaImage;

use crate::{Camera, DrawItem, MeshData, MeshId, MeshRenderer, RenderOptions};

/// A GPU device with a [`MeshRenderer`] and no window.
pub struct Headless {
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    renderer: MeshRenderer,
}

impl Headless {
    /// Opens the default GPU adapter, or returns `None` when the machine has none (CI).
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("dentcraft headless"),
            ..Default::default()
        }))
        .ok()?;
        let samples = MeshRenderer::supported_sample_count(&adapter);
        let renderer = MeshRenderer::new(&device, samples, None);
        Some(Self {
            device,
            queue,
            info: adapter.get_info(),
            renderer,
        })
    }

    /// Name and backend of the adapter in use.
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.info
    }

    pub fn upload(&mut self, id: MeshId, data: &MeshData) {
        self.renderer.upload(&self.device, id, data);
    }

    /// Renders the scene and reads it back as an sRGB RGBA image.
    pub fn render_to_image(
        &mut self,
        width: u32,
        height: u32,
        camera: &Camera,
        items: &[DrawItem],
        options: &RenderOptions,
    ) -> RgbaImage {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            [width, height],
            camera,
            items,
            options,
        );
        let (width, height) = (width.max(1), height.max(1));
        let row = width * 4;
        let padded_row =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dentcraft readback"),
            size: u64::from(padded_row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let texture = self
            .renderer
            .scene_texture()
            .expect("render created the target");
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("mapping the readback buffer");
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("waiting for the GPU");
        let data = slice.get_mapped_range().expect("readback buffer is mapped");
        let mut pixels = Vec::with_capacity((row * height) as usize);
        for y in 0..height as usize {
            let start = y * padded_row as usize;
            pixels.extend_from_slice(&data[start..start + row as usize]);
        }
        drop(data);
        buffer.unmap();
        RgbaImage::from_raw(width, height, pixels).expect("buffer matches the image size")
    }
}
