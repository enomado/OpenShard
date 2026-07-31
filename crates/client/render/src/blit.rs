//! The world image, scaled onto the viewport.
//!
//! This is where the zoom is, and it is the only place. The three world passes
//! draw at 1:1 into an offscreen texture of [`Camera::render_width`] by
//! [`Camera::render_height`]; this stretches that texture over the rectangle the
//! UI left free. Every quad, every atlas region and every pixel-exact assertion
//! in the world passes therefore keeps meaning what it meant, and what is new is
//! one fullscreen quad and one sampler.
//!
//! Scaling the *geometry* instead would resample five-bit art through a filter
//! at every fractional step, and would put a scale factor inside three passes
//! that currently have none. It is also what ClassicUO does in substance, and it
//! is the only arrangement where an interface drawn at 1:1 stays crisp over a
//! magnified world.
//!
//! [`Camera::render_width`]: crate::camera::Camera::render_width
//! [`Camera::render_height`]: crate::camera::Camera::render_height

use crate::camera::Zoom;

/// Where the world image goes on the surface, in physical pixels.
///
/// Not always the whole window: a docked panel shrinks it, which is the same
/// path a resize already takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ViewportRect {
    /// Pixels from the surface's left edge.
    pub x: u32,
    /// Pixels from its top edge.
    pub y: u32,
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
}

/// Draws one texture over a rectangle of another.
#[derive(Debug)]
pub struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// For magnifying: a texel has to stay a square.
    nearest: wgpu::Sampler,
    /// For minifying: nearest would sample one texel in four and the ground
    /// would shimmer as the camera walks.
    linear: wgpu::Sampler,
}

impl Blit {
    /// Build the pipeline for a target of `format`.
    ///
    /// `format` should be a non-sRGB one, as everywhere else here: this pass
    /// copies the world image through untouched, and an sRGB target would gamma
    /// it on the way out — see the crate docs.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            })
        };

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                // The corners come from the vertex index. WebGL2 has
                // `gl_VertexID`, so this costs nothing there either.
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
            // No depth at all: the world's depth buffer ordered the world, and
            // this draws the result of that as a picture.
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            layout,
            nearest: sampler("blit nearest", wgpu::FilterMode::Nearest),
            linear: sampler("blit linear", wgpu::FilterMode::Linear),
        }
    }

    /// Draw `world` over `rect` of `target`, clearing whatever is outside it.
    ///
    /// The filter follows the direction of the zoom: nearest magnifying, linear
    /// minifying. Two rules rather than one, because pixel art wants its texels
    /// square when they are grown and wants them averaged when four of them have
    /// to become one.
    pub fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        world: &wgpu::TextureView,
        zoom: Zoom,
        rect: ViewportRect,
    ) {
        // A bind group per call rather than per `Blit`: the world texture is
        // recreated on every resize and every zoom step, and a cached group
        // would be a handle to a texture that is no longer being drawn into.
        let magnifying = zoom.numerator() >= zoom.denominator();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(world),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(if magnifying {
                        &self.nearest
                    } else {
                        &self.linear
                    }),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Clears the whole surface, not just the rect: whatever the
                    // UI does not cover and the world does not fill is this
                    // frame's, and leaving the last one there would smear.
                    load: wgpu::LoadOp::Clear(crate::renderer::CLEAR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if rect.width == 0 || rect.height == 0 {
            // A minimised window, or a UI that has taken the whole surface. The
            // clear above still happened, which is the frame.
            return;
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        // The viewport is what puts the quad in the rect: the shader emits clip
        // space corners and this is the rectangle clip space maps onto.
        pass.set_viewport(
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
            0.0,
            1.0,
        );
        pass.draw(0..4, 0..1);
    }
}

/// The format of the texture the world is drawn into.
///
/// Every pipeline that draws into that texture — ground, statics, mobiles —
/// must be built with *this* format and never with the surface's. The two are
/// not the same value: a surface may offer `Rgba16Float` first among its
/// non-sRGB formats (an HDR display does), and a pipeline built for it fails
/// validation at `set_pipeline` against a pass whose attachment is this
/// texture. Only the blit and the HUD, which draw to the surface itself, take
/// the surface's format.
pub const WORLD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Create the texture the world is drawn into, at a camera's render size.
///
/// Here rather than in the caller because the format and the usage are this
/// crate's decision: a texture created without `TEXTURE_BINDING` fails at
/// bind-group time with an error that names neither the blit nor the pass that
/// filled it.
pub fn world_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("world"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // `Rgba8Unorm`, like every other texture here: the world passes write
        // the art's own bytes and this carries them to the surface unconverted.
        format: WORLD_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            // So a test can read the world image back and compare it with what
            // the blit produced, which is the only way to know the blit is a
            // copy at 1:1 rather than merely plausible.
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
