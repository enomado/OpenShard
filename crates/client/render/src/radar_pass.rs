//! The radar's own texture, and its own draw.
//!
//! # Why this is not a `GumpArt` variant
//!
//! [`GumpArt`](crate::gump::GumpArt) is a closed enum whose two arms both **name
//! a picture in a client file**, and [`GumpAtlas`](crate::gump::GumpAtlas) is a
//! shelf packer for art that never changes once it is packed. A radar is a
//! bitmap this client *generates*, and it is rewritten every time the player
//! takes a step.
//!
//! The deciding property is mutability rather than identity. Shelf-packing
//! something that is rewritten per step either fragments the atlas or forces the
//! whole-atlas rebuild `docs/client.md` names as the tightest resource in the
//! client — and a mutable entry in a structure whose entries are immutable by
//! construction is the same shape of mistake `docs/boats.md` refused when it
//! kept a moving hull out of `Obstructions`.
//!
//! So the radar gets a texture the size of its own bitmap and a pass that draws
//! one quad. That is also *cheaper* than the alternative: the gump atlas is
//! 2048 square, and reserving a corner of it for a 256-tile radar would carry
//! sixteen megabytes to draw a sixty-fourth of it.
//!
//! # What it does not have
//!
//! **No hue.** `hues.mul` tints art, and there is no ramp for a colour that was
//! never in a client file — `radarcol.mul`'s entries *are* the colours.
//!
//! **No instances.** There is one quad, so its place lives in the uniform block
//! rather than in a vertex buffer with one element in it.
//!
//! **No blending, and nothing to discard.** A radar tile with no colour is
//! [`radar::UNKNOWN`](crate::radar::UNKNOWN) rather than transparent, so the
//! bitmap is opaque everywhere by construction.

use openshard_uofiles::color::Color16;

use crate::gump::Frame;
use crate::renderer::{QUAD, upload};

/// Bytes of the radar pass's uniform block: the target, the quad's place and
/// size, and the scale — rounded up to the sixteen a uniform block is sized in.
const RADAR_UNIFORM_BYTES: u64 = 32;

/// Where the radar is drawn and how big it is, in gump pixels.
///
/// The same coordinate space every window in this client is placed in, so a
/// radar beside a skill sheet is placed the way the skill sheet is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placement {
    /// The top-left corner.
    pub origin: (f32, f32),
    /// The size drawn. Not the bitmap's size: a 256-tile radar shown in a
    /// 128-pixel window is drawn at half a pixel a tile, and the sampler is
    /// `Nearest` so what that drops is dropped rather than smeared.
    pub extent: (f32, f32),
}

/// The radar's texture and the pass that puts it on the frame.
#[derive(Debug)]
pub struct RadarRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    quad: wgpu::Buffer,
    texture: wgpu::Texture,
    /// The bitmap's size in tiles, so an upload of the wrong length is refused
    /// rather than written past the end of a row.
    size: (u32, u32),
}

impl RadarRenderer {
    /// Build the pass for a `width` × `height` bitmap, drawn onto a target of
    /// `format`.
    ///
    /// `format` is the **surface's**, for the gump pass's reason: this draws
    /// onto the finished frame rather than into the world image.
    ///
    /// The texture starts black. Nothing is drawn until [`upload`](Self::upload)
    /// has put a map in it, and the caller that owns the window is the one that
    /// knows when that is.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let blank = vec![0u8; (width as usize) * (height as usize) * 4];
        let texture = upload(device, queue, "radar", width, height, &blank);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Nearest, for the gump pass's reason: this is pixel art at one pixel a
        // tile, and any filtering at all turns a coastline into a grey smear.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("radar sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radar place"),
            size: RADAR_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("radar"),
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
            label: Some("radar"),
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
            label: Some("radar"),
            source: wgpu::ShaderSource::Wgsl(include_str!("radar.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("radar"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("radar"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            // None, for the gump pass's reason: the world's depth buffer ordered
            // the world, and the interface is drawn on the result in the order it
            // is listed.
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Opaque everywhere — see the module header.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let quad = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radar quad"),
            size: std::mem::size_of_val(&QUAD) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut quad_bytes = Vec::with_capacity(QUAD.len() * 4);
        for value in QUAD {
            quad_bytes.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&quad, 0, &quad_bytes);

        Self {
            pipeline,
            bind_group,
            uniforms,
            quad,
            texture,
            size: (width, height),
        }
    }

    /// The bitmap's size in tiles — what [`upload`](Self::upload) expects.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Replace the whole bitmap.
    ///
    /// `pixels` is [`radar::fill`](crate::radar::fill)'s output, row-major and
    /// `width * height` long. A slice of any other length is **ignored**: a
    /// short one would be written as a full texture with whatever followed it in
    /// memory, and a long one would silently draw a rotated map. Refusing is the
    /// only answer that is visible as a stale radar rather than as garbage.
    ///
    /// The whole texture every time rather than a dirty band. A radar is at most
    /// a few hundred tiles square — a quarter of a megabyte — and it changes
    /// everywhere the moment the player steps, because the window scrolls with
    /// them. There is no band to be dirty.
    pub fn upload(&self, queue: &wgpu::Queue, pixels: &[Color16]) {
        let (width, height) = self.size;
        if pixels.len() != (width as usize) * (height as usize) {
            return;
        }
        let mut bytes = Vec::with_capacity(pixels.len() * 4);
        for colour in pixels {
            let rgb = colour.rgb8();
            bytes.extend_from_slice(&[rgb.red, rgb.green, rgb.blue, 0xFF]);
        }
        crate::renderer::write_rows(queue, &self.texture, &bytes, 0..height);
    }

    /// Draw the radar over what is already on the target.
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: Frame<'_>,
        at: Placement,
    ) {
        let mut uniform_bytes = Vec::with_capacity(RADAR_UNIFORM_BYTES as usize);
        for value in [
            frame.width as f32,
            frame.height as f32,
            at.origin.0,
            at.origin.1,
            at.extent.0,
            at.extent.1,
            frame.scale,
            0.0,
        ] {
            uniform_bytes.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.uniforms, 0, &uniform_bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("radar"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Loaded: the world and whatever interface came before this
                    // are already on the surface.
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad.slice(..));
        pass.draw(0..4, 0..1);
    }
}
