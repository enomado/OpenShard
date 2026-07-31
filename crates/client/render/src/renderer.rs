//! The ground pass, on the GPU.
//!
//! Given a device somebody else created and a view somebody else owns, this
//! uploads an atlas once and then draws one instanced quad per visible tile.
//! It never asks for an adapter and never presents: a surface belongs to the
//! application, and a test has no surface at all.

use crate::atlas::{LandAtlas, StaticAtlas, TexmapAtlas};

use crate::camera::{TILE_HEIGHT, TILE_WIDTH, Z_STEP};
use crate::ground::GroundQuad;
use crate::hue::HueRamp;
use crate::sprite::SpriteQuad;

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

/// The depth format every pass here uses.
///
/// `Depth24Plus` and not `Depth32Float`: 24 bits is what WebGL2 guarantees, and
/// the browser is a target. It resolves about 6e-8, where one step of the
/// ordering in [`crate::depth`] is 1e-6 — a margin of more than ten, which that
/// module asserts rather than assumes.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

/// Where a frame is being drawn: the views, and the size they actually are.
///
/// They travel together because the shader turns viewport pixels into clip
/// space and needs the size to do it. Passing them separately invites a resize
/// that updates one and not the other, which draws a correct frame at the wrong
/// scale — and looks like a projection bug.
#[derive(Clone, Copy, Debug)]
pub struct Target<'a> {
    /// What to draw into.
    pub view: &'a wgpu::TextureView,
    /// The depth buffer both passes share.
    ///
    /// Shared on purpose: it is the only thing that makes the ground pass and
    /// the statics pass agree about what covers what. A pass with its own
    /// buffer would be back to "everything drawn later wins", which puts every
    /// wall in front of every hill.
    pub depth: &'a wgpu::TextureView,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
}

/// Create the depth texture a [`Target`] needs, at a given size.
///
/// Here rather than in the caller because the format is this crate's decision
/// and a texture created with another one fails at pipeline-bind time with an
/// error that names neither side.
pub fn depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

/// The depth-stencil state every pass here shares.
///
/// `Less` and writing: nearer is smaller — see [`crate::depth`] — so an
/// untouched buffer at 1.0 means "nothing drawn yet" and every quad that passes
/// the test also records itself for the next one.
fn depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
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
    /// The two atlas textures, kept rather than only viewed.
    ///
    /// A view is all a bind group needs, and holding the texture as well is what
    /// lets an atlas *grow* into the one already bound — see
    /// [`GroundRenderer::upload_changes`]. Dropping them here meant a new
    /// graphic at the edge of the view had to build a whole new renderer, which
    /// recreates the pipeline behind it for two sprites' worth of pixels.
    land_texture: wgpu::Texture,
    texmap_texture: wgpu::Texture,
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
        texmaps: &TexmapAtlas,
    ) -> Self {
        // Two atlases of the same size, uploaded the same way: the flat art and
        // the textures a slope is stretched over. They stay apart because their
        // grids do — 44 pixels against 64 — and not because the shader could not
        // sample one texture.
        let land_texture = upload(
            device,
            queue,
            "land atlas",
            LandAtlas::side(),
            LandAtlas::side(),
            atlas.pixels(),
        );
        let texmap_texture = upload(
            device,
            queue,
            "texmap atlas",
            TexmapAtlas::side(),
            TexmapAtlas::side(),
            texmaps.pixels(),
        );
        let view = land_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texmap_view = texmap_texture.create_view(&wgpu::TextureViewDescriptor::default());

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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
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
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&texmap_view),
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
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 40,
                                shader_location: 4,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: 56,
                                shader_location: 5,
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
            depth_stencil: Some(depth_state()),
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
            land_texture,
            texmap_texture,
        }
    }

    /// Send whatever the two atlases have grown by since this was last asked.
    ///
    /// The pair travels together because a ground quad asks both the same
    /// question: a land graphic packed into one and not yet uploaded from the
    /// other is a slope textured with somebody else's terrain for a frame.
    ///
    /// Nothing to upload is the ordinary frame — a camera standing still
    /// introduces no graphics — and a growth is a band of rows rather than the
    /// 16MB texture.
    pub fn upload_changes(&self, queue: &wgpu::Queue, land: &mut LandAtlas, texmaps: &mut TexmapAtlas) {
        if let Some(rows) = land.take_dirty() {
            write_rows(queue, &self.land_texture, land.pixels(), rows);
        }
        if let Some(rows) = texmaps.take_dirty() {
            write_rows(queue, &self.texmap_texture, texmaps.pixels(), rows);
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
            // Cleared to the far plane here, because this is the frame's first
            // pass. Whatever runs after it loads what this left behind, which
            // is how one ordering spans two passes.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
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

/// The side of every sprite atlas this pass can bind.
///
/// One number rather than a parameter: all three atlases here are the same
/// square, because the ceiling is WebGL2's guaranteed `MAX_TEXTURE_SIZE` and
/// not a choice any of them makes.
pub const SPRITE_ATLAS_SIDE: u32 = StaticAtlas::side();

/// Draws sprites — statics or mobiles — into whatever the ground pass left
/// behind.
///
/// Its own pipeline rather than a mode of [`GroundRenderer`]: the two share
/// nothing but the projection — different atlas, different quad shape,
/// different instance layout — and the one thing they do have to agree on, the
/// ordering, is a number the CPU computes for both.
#[derive(Debug)]
pub struct SpriteRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    quad: wgpu::Buffer,
    instances: wgpu::Buffer,
    /// Quads the instance buffer can hold before it has to be replaced.
    capacity: u64,
    /// The atlas texture, kept so that it can be grown into rather than
    /// replaced. See [`SpriteRenderer::upload_rows`].
    atlas_texture: wgpu::Texture,
}

impl SpriteRenderer {
    /// Upload an atlas and build the pipeline for a target of `format`.
    ///
    /// `pixels` is any square RGBA8 atlas of [`SPRITE_ATLAS_SIDE`] — the
    /// statics' or the mobiles' — because this pass does not care which: it
    /// draws rectangles of somebody else's picture, and where they go is the
    /// caller's arithmetic. Two atlases therefore mean two of these rather than
    /// one that switches, since a draw call binds one texture either way.
    ///
    /// `format` should be a non-sRGB one, for the reason in the crate docs.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        pixels: &[u8],
        hue_ramp: &HueRamp,
    ) -> Self {
        let atlas_texture = upload(
            device,
            queue,
            "sprite atlas",
            SPRITE_ATLAS_SIDE,
            SPRITE_ATLAS_SIDE,
            pixels,
        );
        let view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ramp_texture = upload(
            device,
            queue,
            "hue ramp",
            HueRamp::width(),
            hue_ramp.height(),
            hue_ramp.pixels(),
        );
        let ramp_view = ramp_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Nearest and clamped, exactly as the ground: UO art is pixel art, and
        // filtering it samples whatever was packed next door. The ramp is looked
        // up the same way — `statics.wgsl` picks one exact texel, and filtering
        // between two hues would blend colours nothing in `hues.mul` asked for.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("static atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("statics viewport"),
            size: STATIC_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("statics"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("statics"),
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
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&ramp_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("statics"),
            source: wgpu::ShaderSource::Wgsl(include_str!("statics.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("statics"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("statics"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }),
                    // One instance per static. The layout is asserted in
                    // `SpriteQuad::write`'s test, which is the only thing
                    // linking this to the shader's `@location`s.
                    Some(wgpu::VertexBufferLayout {
                        array_stride: SpriteQuad::STRIDE,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 1,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 8,
                                shader_location: 2,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 16,
                                shader_location: 3,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: 32,
                                shader_location: 4,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Uint32,
                                offset: 36,
                                shader_location: 5,
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
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(depth_state()),
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

        let instances = new_static_instance_buffer(device, INITIAL_QUADS);

        Self {
            pipeline,
            bind_group,
            uniforms,
            quad,
            instances,
            capacity: INITIAL_QUADS,
            atlas_texture,
        }
    }

    /// Send a band of an atlas that has grown since it was uploaded.
    ///
    /// `pixels` is the whole atlas and `rows` is what
    /// [`StaticAtlas::take_dirty`](crate::atlas::StaticAtlas::take_dirty) — or
    /// the animation atlas's — just handed back. Untyped for the same reason
    /// [`SpriteRenderer::new`] is: this pass draws rectangles of somebody else's
    /// picture and does not care which atlas they came from. The pairing is the
    /// caller's, and taking the band is what makes it hard to get wrong — a
    /// range only exists because something was written.
    pub fn upload_rows(&self, queue: &wgpu::Queue, pixels: &[u8], rows: std::ops::Range<u32>) {
        write_rows(queue, &self.atlas_texture, pixels, rows);
    }

    /// Draw `quads` into `target`, keeping what is already there.
    ///
    /// Loads rather than clears, colour and depth alike: this runs after the
    /// ground pass and both of the ground's results are what it is drawing
    /// against. A pass that cleared here would erase the world and then test
    /// against a depth buffer describing it.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: Target<'_>,
        quads: &[SpriteQuad],
    ) {
        let mut uniform_bytes = Vec::with_capacity(STATIC_UNIFORM_BYTES as usize);
        for value in [target.width as f32, target.height as f32, 0.0, 0.0] {
            uniform_bytes.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &uniform_bytes);

        if quads.len() as u64 > self.capacity {
            self.capacity = (quads.len() as u64).next_power_of_two();
            self.instances = new_static_instance_buffer(device, self.capacity);
        }
        let mut instance_bytes = Vec::with_capacity(quads.len() * SpriteQuad::STRIDE as usize);
        for quad in quads {
            quad.write(&mut instance_bytes);
        }
        if instance_bytes.is_empty() {
            // Nothing to draw and nothing to clear: unlike the ground pass,
            // this one owns no pixels of its own, so an empty pass would only
            // cost a submission.
            return;
        }
        queue.write_buffer(&self.instances, 0, &instance_bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("statics"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        pass.draw(0..4, 0..quads.len() as u32);
    }
}

/// Bytes of the statics uniform block: a `vec2<f32>` and its padding.
const STATIC_UNIFORM_BYTES: u64 = 16;

fn new_static_instance_buffer(device: &wgpu::Device, quads: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("static instances"),
        size: quads * SpriteQuad::STRIDE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Create an RGBA8 texture, fill it, and hand back a view of it.
///
/// Not necessarily square: every sprite atlas here is, but [`HueRamp`] is
/// [`HueRamp::width`] wide by however many hues were read, and giving this its
/// own two dimensions is simpler than a wrapper that pretends it is square.
///
/// `Rgba8Unorm` and never `…Srgb`: the atlas holds the file's own bytes and a
/// gamma conversion here would mean the pixel that went in is not the pixel that
/// comes out — see the crate docs.
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
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
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// Write a band of rows into an atlas texture already on the device.
///
/// `rows` indexes the atlas, and `pixels` is the *whole* atlas — the band is cut
/// out of it here, because `write_texture` wants a slice that starts at the
/// first texel it will write and the caller should not have to do that
/// arithmetic twice.
///
/// An empty or out-of-range band writes nothing rather than failing: the atlas
/// that produced the range and the texture it is written into are the same size
/// by construction, so a range outside it is a bug in this crate and not
/// something a frame should die of.
fn write_rows(queue: &wgpu::Queue, texture: &wgpu::Texture, pixels: &[u8], rows: std::ops::Range<u32>) {
    let width = texture.width();
    let height = texture.height();
    let (top, bottom) = (rows.start.min(height), rows.end.min(height));
    if top >= bottom {
        return;
    }
    let stride = width as usize * 4;
    let from = top as usize * stride;
    let to = bottom as usize * stride;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: top, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &pixels[from..to],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(bottom - top),
        },
        wgpu::Extent3d {
            width,
            height: bottom - top,
            depth_or_array_layers: 1,
        },
    );
}

fn new_instance_buffer(device: &wgpu::Device, quads: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ground instances"),
        size: quads * GroundQuad::STRIDE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
