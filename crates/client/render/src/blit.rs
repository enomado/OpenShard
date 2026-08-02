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
use crate::light::Lighting;

/// How many lights the uniform block holds. `blit.wgsl`'s `MAX_LIGHTS`, and the
/// two are one number: the array's length is fixed at shader compile time, so a
/// buffer written to a different one is rejected by wgpu rather than drawn
/// wrongly.
const MAX_LIGHTS: usize = Lighting::MAX;

/// The uniform block's size: two header `vec4`s — the ambient with the light
/// count, and the occlusion grid's rectangle — then two per light.
const LIGHTING_BYTES: u64 = (2 + 2 * MAX_LIGHTS as u64) * 16;

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

/// What one blit draws, and where.
///
/// The four values that describe *this frame's* picture, grouped for the reason
/// [`Target`](crate::renderer::Target) is: they always travel together, and a
/// `render` that took them one by one alongside a device, a queue, an encoder
/// and the lighting would be a call whose arguments are told apart by position
/// alone.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// Where the picture goes — the surface.
    pub target: &'a wgpu::TextureView,
    /// The world image it comes from.
    pub world: &'a wgpu::TextureView,
    /// Which tile each of that image's pixels came from — see [`crate::place`].
    /// What the lighting is computed against, and the reason this pass can tell
    /// a wall's lit face from the shadow behind it.
    pub place: &'a wgpu::TextureView,
    /// Which way the scaling goes, and so which sampler is right.
    pub zoom: Zoom,
    /// The rectangle of `target` the world gets.
    pub rect: ViewportRect,
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
    /// The frame's lights, rewritten every frame — see [`crate::light`].
    lighting: wgpu::Buffer,
    /// What stands in their way, as one texel a tile — see
    /// [`crate::occlusion`]. Recreated when the frame's grid changes size, which
    /// is a zoom step or a resize and not an ordinary frame; rewritten every
    /// frame, because the camera moves and the grid is relative to it.
    occluders: wgpu::Texture,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The place attachment and the occlusion grid, both integer
                // textures and therefore both unfilterable: there is no sampler
                // for either, and the shader reads exact texels.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        // Zeroed at creation, which is *not* the identity — a zero ambient is
        // black — so the first frame writes it before drawing. Every frame
        // does; this only has to be a buffer of the right size to bind.
        let lighting = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting"),
            size: LIGHTING_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            lighting,
            // One texel, which is a grid of one open tile: a daylit frame binds
            // it and never reads it, and the first lit frame replaces it. A
            // texture of no size is not a thing wgpu will make.
            occluders: occluder_texture(device, 1, 1),
        }
    }

    /// Draw `world` over `rect` of `target`, clearing whatever is outside it.
    ///
    /// The filter follows the direction of the zoom: nearest magnifying, linear
    /// minifying. Two rules rather than one, because pixel art wants its texels
    /// square when they are grown and wants them averaged when four of them have
    /// to become one.
    ///
    /// `lighting` is what the frame's flames do to the image on the way past —
    /// see [`crate::light`]. [`Lighting::NONE`] leaves it a copy, which is what
    /// a daylit frame and every frame test pass.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: Frame<'_>,
        lighting: &Lighting,
    ) {
        let Frame {
            target,
            world,
            place,
            zoom,
            rect,
        } = frame;
        queue.write_buffer(&self.lighting, 0, &lighting_bytes(lighting));
        self.upload_occluders(device, queue, lighting);
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.lighting.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(place),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        &self
                            .occluders
                            .create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
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

/// The uniform block `blit.wgsl` reads, laid out by hand.
///
/// Written as bytes rather than through a `#[repr(C)]` struct for the reason
/// every other pass here does it: the layout is a contract with text the Rust
/// compiler never sees, and a field order stated once in the shader and once in
/// a struct is two statements that can disagree. This way the writing order is
/// the shader's declaration order, in one place.
///
/// Lights past [`Lighting::MAX`] are dropped rather than wrapping the array —
/// [`crate::light::collect`] already keeps only the nearest that many, so this
/// is the second half of one rule and not a policy of its own.
fn lighting_bytes(lighting: &Lighting) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(LIGHTING_BYTES as usize);
    let count = lighting.lights.len().min(MAX_LIGHTS);

    // `ambient`, with the light count in the fourth channel.
    for channel in lighting.ambient {
        bytes.extend_from_slice(&channel.to_le_bytes());
    }
    bytes.extend_from_slice(&(count as f32).to_le_bytes());

    // `grid`: where the occlusion texture's corner is on the map, and how big it
    // is. Signed integers, which is what the shader declares that field as —
    // a rectangle may start at a negative tile for a camera near the map's
    // corner, and the walk has to be able to say so.
    let bounds = lighting.occlusion.bounds();
    for value in [bounds.min_x, bounds.min_y, bounds.width(), bounds.height()] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    for light in &lighting.lights[..count] {
        for value in [
            light.at.x,
            light.at.y,
            light.z,
            light.radius,
            light.color[0],
            light.color[1],
            light.color[2],
            light.intensity,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    // The tail of the array is never read — the shader stops at the count — but
    // the buffer is bound whole, and a short write leaves whatever the last
    // frame put there. Zeroed, so a partial upload cannot be mistaken for one.
    bytes.resize(LIGHTING_BYTES as usize, 0);
    bytes
}

/// The occlusion grid as a texture: one texel a tile. See [`crate::occlusion`]
/// for what the four channels hold.
///
/// `Rgba8Uint` and not four floats, because every one of them is a byte in the
/// grid already, and because an integer texture cannot be filtered — a wall
/// averaged with the open ground beside it would be a half-wall standing on
/// neither tile.
fn occluder_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("occluders"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

impl Blit {
    /// Put this frame's occluders on the GPU, growing the texture if the grid's
    /// rectangle has changed size.
    ///
    /// The *size* changes on a zoom step or a resize and on nothing else: the
    /// grid is the visible tiles grown by a fixed margin, so a camera walking
    /// keeps its dimensions and only its contents move. Recreating a texture per
    /// frame would be a hundred kilobytes of allocation on every one of them.
    fn upload_occluders(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, lighting: &Lighting) {
        let bounds = lighting.occlusion.bounds();
        let (width, height) = (bounds.width().max(1) as u32, bounds.height().max(1) as u32);
        if self.occluders.width() != width || self.occluders.height() != height {
            self.occluders = occluder_texture(device, width, height);
        }
        let bytes = lighting.occlusion.bytes();
        if bytes.is_empty() {
            // A daylit frame, or one with no grid at all: the texture keeps
            // whatever it held and the shader never reads it, because there are
            // no lights to walk a ray for.
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.occluders,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
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
