//! The ground pass, on the GPU.
//!
//! Given a device somebody else created and a view somebody else owns, this
//! uploads an atlas once and then draws one instanced quad per visible tile.
//! It never asks for an adapter and never presents: a surface belongs to the
//! application, and a test has no surface at all.

use crate::atlas::LandAtlas;
use crate::camera::{TILE_HEIGHT, TILE_WIDTH, Z_STEP};
use crate::ground::GroundQuad;

/// What an untouched pixel is left as.
///
/// Fully transparent, and the fragment shader always writes `a = 1.0`. That
/// makes "was anything drawn here" a question about one byte, with no colour a
/// tile could coincidentally match — which is the difference between a frame
/// test that measures coverage and one that hopes.
pub const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// Bytes of the uniform block: two `vec2<f32>`, a height step, and the padding
/// a uniform block's size is rounded up to.
const UNIFORM_BYTES: u64 = 32;

/// The unit quad, as a triangle strip: (0,0) (1,0) (0,1) (1,1).
const QUAD: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];

/// How many quads the instance buffer starts out able to hold.
const INITIAL_QUADS: u64 = 4096;

/// Where a frame is being drawn: a view, and the size it actually is.
///
/// The two travel together because the shader turns viewport pixels into clip
/// space and needs the size to do it. Passing them separately invites a resize
/// that updates one and not the other, which draws a correct frame at the wrong
/// scale — and looks like a projection bug.
#[derive(Clone, Copy, Debug)]
pub struct Target<'a> {
    /// What to draw into.
    pub view: &'a wgpu::TextureView,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
}

/// Draws ground.
#[derive(Debug)]
pub struct GroundRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    quad: wgpu::Buffer,
    instances: wgpu::Buffer,
    /// Quads the instance buffer can hold before it has to be replaced.
    capacity: u64,
}

impl GroundRenderer {
    /// Upload an atlas and build the pipeline for a target of `format`.
    ///
    /// `format` should be a non-sRGB one. The crate's colour rule is that a
    /// pixel in the art is the pixel in the frame, and an sRGB target silently
    /// breaks it — see the crate docs.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        atlas: &LandAtlas,
    ) -> Self {
        let side = LandAtlas::side();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("land atlas"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            atlas.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Nearest, and clamped. UO art is pixel art on an exact grid: filtering
        // it would sample a neighbouring tile across the atlas seam, which shows
        // up as a one-pixel fringe along every diamond.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("land atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ground"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ground"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ground"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ground.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ground"),
            bind_group_layouts: &[Some(&layout)],
            // No immediate data: everything per-frame travels in the uniform
            // block, which WebGL2 supports and push constants do not.
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ground"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    // The unit quad.
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }),
                    // One instance per tile. The layout is asserted in
                    // `GroundQuad::write`'s test, which is the only thing that
                    // links this to the shader's `@location`s.
                    Some(wgpu::VertexBufferLayout {
                        array_stride: GroundQuad::STRIDE,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 1,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 8,
                                shader_location: 2,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 24,
                                shader_location: 3,
                            },
                        ],
                    }),
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // No blending: the shader discards transparent texels, so
                    // every fragment that survives is opaque.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // Quads are emitted in one winding and never seen from behind.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let quad = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("unit quad"),
            size: std::mem::size_of_val(&QUAD) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut quad_bytes = Vec::with_capacity(QUAD.len() * 4);
        for value in QUAD {
            quad_bytes.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&quad, 0, &quad_bytes);

        let instances = new_instance_buffer(device, INITIAL_QUADS);

        Self {
            pipeline,
            bind_group,
            uniforms,
            quad,
            instances,
            capacity: INITIAL_QUADS,
        }
    }

    /// Draw `quads` into `target`, clearing it first.
    ///
    /// The quads carry viewport coordinates, which the shader turns into clip
    /// space using the target's size.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: Target<'_>,
        quads: &[GroundQuad],
    ) {
        let mut uniform_bytes = Vec::with_capacity(UNIFORM_BYTES as usize);
        for value in [
            target.width as f32,
            target.height as f32,
            TILE_WIDTH as f32,
            TILE_HEIGHT as f32,
            Z_STEP as f32,
            0.0,
        ] {
            uniform_bytes.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &uniform_bytes);

        if quads.len() as u64 > self.capacity {
            // Grow in powers of two rather than to the exact size: the camera
            // moves every frame and the count wobbles with it.
            self.capacity = (quads.len() as u64).next_power_of_two();
            self.instances = new_instance_buffer(device, self.capacity);
        }
        let mut instance_bytes = Vec::with_capacity(quads.len() * GroundQuad::STRIDE as usize);
        for quad in quads {
            quad.write(&mut instance_bytes);
        }
        if !instance_bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, &instance_bytes);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ground"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(CLEAR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if quads.is_empty() {
            // The pass still runs: clearing is the frame when there is nothing
            // to draw, and skipping it would leave the previous one on screen.
            return;
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        pass.draw(0..4, 0..quads.len() as u32);
    }
}

fn new_instance_buffer(device: &wgpu::Device, quads: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ground instances"),
        size: quads * GroundQuad::STRIDE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
