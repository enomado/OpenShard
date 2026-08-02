//! A hard edge drawn round a highlighted sprite's silhouette.
//!
//! The second way of saying "the cursor is on this". The first is
//! [`items::HIGHLIGHT_HUE`](crate::items::HIGHLIGHT_HUE), which replaces the
//! art's colour; this draws a ring round its shape instead, and the two compose
//! because they happen in different passes over different pixels.
//!
//! # Two passes and one texture
//!
//! 1. [`SpriteRenderer::render_mask`](crate::renderer::SpriteRenderer::render_mask)
//!    draws the sprites to be outlined into an `R8Uint` mask the size of the
//!    world image, each in its own id — `silhouette.wgsl`. It tests the world's
//!    depth buffer, so the mask holds what is *visible*.
//! 2. [`Outline::render`] runs after the blit, over the same rectangle of the
//!    surface, and turns "this texel borders a different id" into a coloured
//!    ring — `outline.wgsl`.
//!
//! Nothing is precomputed and nothing is stored per graphic. The cost is one
//! small draw plus one pass over the viewport, and it is the same whether the
//! atlas holds ten sprites or ten thousand — which is the argument for doing it
//! here rather than baking an edge into the art, where the work would scale with
//! the art instead of with what is lit.
//!
//! # Why the thickness is in virtual pixels
//!
//! The mask is the world image's size, so its texels are the world's *virtual*
//! pixels and the ring is magnified by the blit along with the art it surrounds.
//! That is on purpose. A one-*screen*-pixel hairline round a sprite drawn at 4×
//! is a line finer than any edge in the picture it is tracing, and it reads as a
//! rendering artefact rather than as a highlight; a ring one art pixel thick
//! grows with the sprite and stays part of the same picture.
//!
//! It also costs nothing extra: the ring is found where the mask already is, and
//! the blit's nearest sampler carries it up whole.
//!
//! `docs/outline.md` is the plan this is built against.

/// The format of the mask the silhouette pass writes and this one reads.
///
/// `R8Uint` and not `R8Unorm`: what is stored is an *identity*, not a coverage,
/// and comparing two ids for equality through a normalised float would be
/// comparing `n / 255.0` values — right for 255 objects and wrong-looking in
/// every code review that follows. Uint also means no sampler: the ring reads
/// exact texels with `textureLoad`, which is the only correct thing to do to a
/// mask of numbers.
pub const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Uint;

/// How many sprites can be outlined at once, each with its own ring.
///
/// The mask holds one byte and zero means "nothing here", so this is what is
/// left. The client lights one item at a time today; the ceiling exists so the
/// silhouette pass can say what happens past it rather than wrapping ids round
/// and ringing two objects as one.
pub const MAX_OUTLINED: usize = 255;

/// Bytes of `outline.wgsl`'s uniform block: two `vec4`s.
const RING_BYTES: u64 = 32;

/// Create the mask the silhouette pass draws into, at the world image's size.
///
/// Here rather than in the caller for the reason
/// [`blit::world_texture`](crate::blit::world_texture) is: the format and the
/// usage are this module's decision, and a texture created without
/// `TEXTURE_BINDING` fails at bind-group time with an error naming neither pass.
///
/// **The size is the world image's, not the surface's.** The silhouette pass
/// shares the world's depth buffer, and a depth attachment must match its colour
/// attachment exactly.
pub fn mask_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("outline mask"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: MASK_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// What one ring looks like.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ring {
    /// Its colour, straight through to the surface — no hue ramp and no
    /// lighting. A highlight that dimmed at night would be a highlight that
    /// stops working exactly when the picture is hardest to read, which is why
    /// this pass runs *after* the blit rather than into the world image.
    pub color: [f32; 4],
    /// Its half-width, in the mask's texels — that is, in virtual pixels. One
    /// is a one-pixel edge round the silhouette, drawn at the art's own scale.
    ///
    /// Above one the neighbourhood is searched densely, so the cost is
    /// `(2n+1)²` taps a fragment. Two is already a heavy ring on 44-pixel art.
    pub width: u32,
}

impl Ring {
    /// A white one-pixel edge.
    ///
    /// White rather than a colour of its own: the ring's job is to be *not the
    /// picture*, and UO art has no white in its silhouettes worth speaking of.
    /// A caller wanting the reference's yellow-green highlight has the field.
    pub const DEFAULT: Self = Self {
        color: [1.0, 1.0, 1.0, 1.0],
        width: 1,
    };
}

/// Where one ring pass draws, and from what.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// What to draw onto — the surface, after the blit has put the world there.
    pub target: &'a wgpu::TextureView,
    /// The mask the silhouette pass filled.
    pub mask: &'a wgpu::TextureView,
    /// Its size in texels. Carried beside the view for the reason
    /// [`Target`](crate::renderer::Target) carries a size: a view does not know
    /// its own extent, and a mask sampled at the wrong one is a ring drawn in
    /// the wrong place — which looks like a projection bug and is not one.
    pub mask_size: (u32, u32),
    /// The rectangle of `target` the world was blitted into. The same
    /// [`ViewportRect`](crate::blit::ViewportRect) the blit was given, or the
    /// ring lands somewhere the world is not.
    pub rect: crate::blit::ViewportRect,
}

/// Turns a silhouette mask into a ring on the surface.
#[derive(Debug)]
pub struct Outline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
}

impl Outline {
    /// Build the pipeline for a target of `format` — the surface's, not
    /// [`WORLD_FORMAT`](crate::blit::WORLD_FORMAT): this pass draws over the
    /// blit's output.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("outline"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ring"),
            size: RING_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("outline"),
            source: wgpu::ShaderSource::Wgsl(include_str!("outline.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("outline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Alpha blending, unlike every other pass here, because this
                    // one draws *onto* a finished picture rather than into an
                    // empty target: a ring at less than full alpha is a ring the
                    // world shows through, which is what a soft highlight is
                    // made of and what the glow will need untouched.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            // No depth: the world's depth buffer already decided what is
            // visible, and the mask is the record of that decision.
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            layout,
            uniforms,
        }
    }

    /// Draw the ring over `frame.rect`, keeping everything else.
    ///
    /// Loads rather than clears: the blit has just written the world there, and
    /// this adds an edge to it.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: Frame<'_>,
        ring: Ring,
    ) {
        let mut bytes = Vec::with_capacity(RING_BYTES as usize);
        for value in [
            frame.mask_size.0 as f32,
            frame.mask_size.1 as f32,
            ring.width.max(1) as f32,
            0.0,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for channel in ring.color {
            bytes.extend_from_slice(&channel.to_le_bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &bytes);

        // A bind group per call, as the blit does: the mask is recreated on
        // every resize and every zoom step, and a cached group would point at a
        // texture nothing is drawing into any more.
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("outline"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(frame.mask),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("outline"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if frame.rect.width == 0 || frame.rect.height == 0 {
            return;
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_viewport(
            frame.rect.x as f32,
            frame.rect.y as f32,
            frame.rect.width as f32,
            frame.rect.height as f32,
            0.0,
            1.0,
        );
        pass.draw(0..4, 0..1);
    }
}
