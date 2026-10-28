//! Mesh rendering into an offscreen, multisampled target that can be blitted into another render
//! pass (the UI's) or read back as an image.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use dentcraft_mesh::{Aabb, TriMesh};
use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::Camera;

/// Colour format of the offscreen scene target.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SCENE_FORMAT_RAW: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const DRAW_STRIDE: u64 = 256;

const BLIT_SHADER: &str = r"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct Out { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    o.uv = uv;
    return o;
}
@fragment fn fs(o: Out) -> @location(0) vec4<f32> { return textureSample(tex, samp, o.uv); }
";

/// Identifies an uploaded mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(pub u64);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// GPU-ready mesh data. Building it is CPU work (normals, f64 → f32), so do it off the UI thread.
#[derive(Clone, Debug)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub bounds: Option<Aabb>,
}

impl MeshData {
    pub fn from_mesh(mesh: &TriMesh) -> Self {
        let normals = mesh.vertex_normals();
        let vertices = mesh
            .positions
            .iter()
            .zip(&normals)
            .map(|(p, n)| Vertex {
                position: p.as_vec3().to_array(),
                normal: n.as_vec3().to_array(),
            })
            .collect();
        Self {
            vertices,
            indices: mesh.triangles.iter().flatten().copied().collect(),
            bounds: mesh.bounds(),
        }
    }
}

/// One mesh to draw with its look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawItem {
    pub mesh: MeshId,
    /// sRGB colour, 0..1.
    pub color: [f32; 3],
    /// 1 = opaque.
    pub opacity: f32,
    pub visible: bool,
}

/// Scene-wide look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// sRGB gradient colours for the top and bottom of the background.
    pub background_top: [f32; 3],
    pub background_bottom: [f32; 3],
    /// Vignette strength (0 = none).
    pub vignette: f32,
    /// sRGB tint for back faces (the inside of open scans) and how strongly it applies.
    pub backface: [f32; 3],
    pub backface_strength: f32,
}

impl RenderOptions {
    pub fn dark() -> Self {
        Self {
            background_top: [0.205, 0.225, 0.255],
            background_bottom: [0.085, 0.092, 0.105],
            vignette: 0.5,
            backface: [0.40, 0.45, 0.53],
            backface_strength: 0.7,
        }
    }

    pub fn light() -> Self {
        Self {
            background_top: [0.96, 0.97, 0.98],
            background_bottom: [0.78, 0.81, 0.85],
            vignette: 0.25,
            backface: [0.56, 0.61, 0.69],
            backface_strength: 0.7,
        }
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self::dark()
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(srgb_to_linear)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    backface: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawUniform {
    color: [f32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    center: Vec3,
}

struct Target {
    size: [u32; 2],
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    blit_bind_group: Option<wgpu::BindGroup>,
}

struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    raw_view: bool,
}

/// Draws meshes with the DentCraft scan look.
pub struct MeshRenderer {
    sample_count: u32,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    draw_layout: wgpu::BindGroupLayout,
    draw_buffer: wgpu::Buffer,
    draw_bind_group: wgpu::BindGroup,
    draw_capacity: u64,
    background: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    meshes: HashMap<MeshId, GpuMesh>,
    target: Option<Target>,
    blit: Option<Blit>,
    /// Draw order scratch space (item index, view depth), reused across frames.
    order: Vec<(usize, f32)>,
}

impl MeshRenderer {
    /// Highest of 4× / 1× MSAA the adapter supports for the scene target.
    pub fn supported_sample_count(adapter: &wgpu::Adapter) -> u32 {
        let color = adapter.get_texture_format_features(SCENE_FORMAT).flags;
        let depth = adapter.get_texture_format_features(DEPTH_FORMAT).flags;
        if color.sample_count_supported(4) && depth.sample_count_supported(4) {
            4
        } else {
            1
        }
    }

    /// `output`: when the scene will be blitted into another render pass, its colour format and
    /// sample count (that pass must have no depth attachment).
    pub fn new(
        device: &wgpu::Device,
        sample_count: u32,
        output: Option<(wgpu::TextureFormat, u32)>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dentcraft scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let uniform_entry = |dynamic: bool, size: u64| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let frame_size = std::mem::size_of::<FrameUniform>() as u64;
        let draw_size = std::mem::size_of::<DrawUniform>() as u64;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dentcraft frame"),
            entries: &[uniform_entry(false, frame_size)],
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dentcraft draw"),
            entries: &[uniform_entry(true, draw_size)],
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dentcraft frame"),
            size: frame_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dentcraft frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let draw_capacity = 16;
        let (draw_buffer, draw_bind_group) = Self::draw_buffer(device, &draw_layout, draw_capacity);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dentcraft scene"),
            bind_group_layouts: &[Some(&frame_layout), Some(&draw_layout)],
            immediate_size: 0,
        });
        let multisample = wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
        };
        let mesh_pipeline = |label: &str, blend: wgpu::BlendState, depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_mesh"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout.clone())],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_mesh"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: SCENE_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = mesh_pipeline("dentcraft opaque", wgpu::BlendState::REPLACE, true);
        let transparent = mesh_pipeline(
            "dentcraft transparent",
            wgpu::BlendState::ALPHA_BLENDING,
            false,
        );
        let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dentcraft background"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_background"),
                compilation_options: Default::default(),
                targets: &[Some(SCENE_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let blit = output.map(|(format, samples)| Self::create_blit(device, format, samples));
        Self {
            sample_count,
            frame_buffer,
            frame_bind_group,
            draw_layout,
            draw_buffer,
            draw_bind_group,
            draw_capacity,
            background,
            opaque,
            transparent,
            meshes: HashMap::new(),
            target: None,
            blit,
            order: Vec::new(),
        }
    }

    fn draw_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        capacity: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dentcraft draws"),
            size: capacity * DRAW_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dentcraft draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<DrawUniform>() as u64),
                }),
            }],
        });
        (buffer, bind_group)
    }

    fn create_blit(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Blit {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dentcraft blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dentcraft blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dentcraft blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dentcraft blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples.max(1),
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("dentcraft blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Blit {
            pipeline,
            layout,
            sampler,
            // A UI pass that writes gamma-encoded values into a non-sRGB target wants the
            // stored sRGB bytes unchanged.
            raw_view: !format.is_srgb(),
        }
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn upload(&mut self, device: &wgpu::Device, id: MeshId, data: &MeshData) {
        if data.indices.is_empty() {
            self.meshes.remove(&id);
            return;
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dentcraft mesh vertices"),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dentcraft mesh indices"),
            contents: bytemuck::cast_slice(&data.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let center = data
            .bounds
            .map(|b| b.center().as_vec3())
            .unwrap_or(Vec3::ZERO);
        self.meshes.insert(
            id,
            GpuMesh {
                vertices,
                indices,
                index_count: data.indices.len() as u32,
                center,
            },
        );
    }

    pub fn remove(&mut self, id: MeshId) {
        self.meshes.remove(&id);
    }

    pub fn contains(&self, id: MeshId) -> bool {
        self.meshes.contains_key(&id)
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let texture = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let msaa = (self.sample_count > 1).then(|| {
            texture(
                "dentcraft scene msaa",
                SCENE_FORMAT,
                self.sample_count,
                attachment,
                &[],
            )
            .create_view(&Default::default())
        });
        let depth = texture(
            "dentcraft scene depth",
            DEPTH_FORMAT,
            self.sample_count,
            attachment,
            &[],
        )
        .create_view(&Default::default());
        let resolve = texture(
            "dentcraft scene",
            SCENE_FORMAT,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            &[SCENE_FORMAT_RAW],
        );
        let resolve_view = resolve.create_view(&Default::default());
        let blit_bind_group = self.blit.as_ref().map(|blit| {
            let view = resolve.create_view(&wgpu::TextureViewDescriptor {
                format: Some(if blit.raw_view {
                    SCENE_FORMAT_RAW
                } else {
                    SCENE_FORMAT
                }),
                ..Default::default()
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dentcraft blit"),
                layout: &blit.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&blit.sampler),
                    },
                ],
            })
        });
        self.target = Some(Target {
            size,
            msaa,
            depth,
            resolve,
            resolve_view,
            blit_bind_group,
        });
    }

    /// Renders the scene into the offscreen target of `size` pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        camera: &Camera,
        items: &[DrawItem],
        options: &RenderOptions,
    ) {
        let size = [size[0].max(1), size[1].max(1)];
        self.ensure_target(device, size);
        let mut camera = camera.clone();
        camera.aspect = size[0] as f32 / size[1] as f32;
        let bg = |rgb: [f32; 3], w: f32| {
            let [r, g, b] = linear(rgb);
            [r, g, b, w]
        };
        let frame = FrameUniform {
            view_proj: camera.view_projection().to_cols_array_2d(),
            view: camera.view().to_cols_array_2d(),
            bg_top: bg(options.background_top, options.vignette),
            bg_bottom: bg(options.background_bottom, 0.0),
            backface: bg(options.backface, options.backface_strength),
        };
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));

        // Opaque first (front to back), then transparent (back to front).
        let view = camera.view();
        self.order.clear();
        for (i, item) in items.iter().enumerate() {
            if let Some(mesh) = self.meshes.get(&item.mesh)
                && item.visible
                && item.opacity > 0.0
            {
                let depth = -view.transform_point3(mesh.center).z;
                self.order.push((i, depth));
            }
        }
        let is_opaque = |i: usize| items[i].opacity >= 0.999;
        self.order.sort_by(|a, b| {
            let (oa, ob) = (is_opaque(a.0), is_opaque(b.0));
            ob.cmp(&oa).then_with(|| {
                if oa {
                    a.1.total_cmp(&b.1)
                } else {
                    b.1.total_cmp(&a.1)
                }
            })
        });
        let needed = self.order.len().max(1) as u64;
        if needed > self.draw_capacity {
            self.draw_capacity = needed.next_power_of_two();
            (self.draw_buffer, self.draw_bind_group) =
                Self::draw_buffer(device, &self.draw_layout, self.draw_capacity);
        }
        let mut uniforms = vec![0u8; (self.order.len() as u64 * DRAW_STRIDE) as usize];
        for (slot, (i, _)) in self.order.iter().enumerate() {
            let item = &items[*i];
            let [r, g, b] = linear(item.color);
            let draw = DrawUniform {
                color: [r, g, b, item.opacity.clamp(0.0, 1.0)],
            };
            let at = slot * DRAW_STRIDE as usize;
            uniforms[at..at + std::mem::size_of::<DrawUniform>()]
                .copy_from_slice(bytemuck::bytes_of(&draw));
        }
        if !uniforms.is_empty() {
            queue.write_buffer(&self.draw_buffer, 0, &uniforms);
        }

        let target = self.target.as_ref().expect("target was just created");
        let (view, resolve_target) = match &target.msaa {
            Some(msaa) => (msaa, Some(&target.resolve_view)),
            None => (&target.resolve_view, None),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dentcraft scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.frame_bind_group, &[]);
        pass.set_bind_group(1, &self.draw_bind_group, &[0]);
        pass.set_pipeline(&self.background);
        pass.draw(0..3, 0..1);
        let mut transparent = false;
        pass.set_pipeline(&self.opaque);
        for (slot, (i, _)) in self.order.iter().enumerate() {
            let item = &items[*i];
            if !transparent && item.opacity < 0.999 {
                transparent = true;
                pass.set_pipeline(&self.transparent);
            }
            let mesh = &self.meshes[&item.mesh];
            pass.set_bind_group(
                1,
                &self.draw_bind_group,
                &[(slot as u64 * DRAW_STRIDE) as u32],
            );
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }

    /// Draws the last rendered scene into `pass`, filling its viewport. Requires `output` to have
    /// been given to [`MeshRenderer::new`].
    pub fn blit(&self, pass: &mut wgpu::RenderPass<'_>) {
        let (Some(blit), Some(Some(bind_group))) =
            (&self.blit, self.target.as_ref().map(|t| &t.blit_bind_group))
        else {
            return;
        };
        pass.set_pipeline(&blit.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The resolved scene texture of the last `render` (sRGB RGBA8).
    pub fn scene_texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|t| &t.resolve)
    }
}
