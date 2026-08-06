//! Two literal unit cubes, nothing else: no client files, no map, no art —
//! just two [`occlusion::Solid`]s built by hand and drawn through
//! [`solids::SolidsRenderer`], the same pass the live F5 overlay and
//! `isolated_scene`'s `OPENSHARD_SCENE_SOLIDS` mode use.
//!
//! Built to answer one question directly: does a nearer solid's face
//! genuinely hide a farther solid's face behind it, or does the farther one
//! show through? `solids.rs`'s own `Style` doc already says the answer is
//! "it depends" — `opaque: false` (the live overlay's own default) blends on
//! purpose, `opaque: true` is the painter's-algorithm overwrite. What decides
//! either one is *draw order alone*: `SolidsRenderer::render`'s pipeline has
//! `depth_stencil: None`, no hardware depth test at all, so correctness rests
//! entirely on the caller handing solids to it back-to-front. This tool draws
//! the same two cubes three ways — the order `solid::standing` itself would
//! produce, and that order reversed by hand — so a wrong sort is visible by
//! comparison rather than by trusting the sort is right.
//!
//! - `OPENSHARD_CUBE_OFFSET=dx,dy` — where the second cube stands, relative to
//!   the first. Default `1,1`: south-east, the direction this camera's own
//!   "toward it" faces (`East`/`South`, `solid.rs`'s own doc) point along, so
//!   the second cube is nearer the camera and should hide part of the first.
//! - `OPENSHARD_CUBE_HEIGHT=n` — each cube's height in `z` units. Default
//!   `11` (`light::Z_PER_TILE`), so a cube whose footprint is one tile is
//!   also one tile tall on screen rather than a flat slab or a tower.
//! - `OPENSHARD_SCENE_ZOOM=n` — notches of `Zoom::scale_up`. Default `3`.
//! - `OPENSHARD_FRAME_DUMP=/tmp/x` — base path; this tool writes
//!   `<path>_forward.ppm`, `<path>_forward_opaque.ppm` and
//!   `<path>_reversed.ppm` beside it.
//!
//! ```sh
//! OPENSHARD_FRAME_DUMP=/tmp/two_cubes \
//!     cargo run --release -p openshard-client-render --example two_cubes
//! ```

use std::path::PathBuf;

use openshard_client_render::camera::{Camera, TileBounds, Zoom};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::occlusion::{Builder, Shape};
use openshard_client_render::solids::{Frame, SolidsRenderer, Style};
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::tiledata::{StaticTile, TileFlags};

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn env_or(name: &str, default: &str) -> String {
    env_opt(name).unwrap_or_else(|| default.to_string())
}

fn parse_pair(spec: &str) -> (i32, i32) {
    let (a, b) = spec
        .split_once(',')
        .unwrap_or_else(|| panic!("wanted `a,b`, got {spec:?}"));
    (
        a.trim().parse().unwrap_or_else(|_| panic!("{a:?}")),
        b.trim().parse().unwrap_or_else(|_| panic!("{b:?}")),
    )
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn dump(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &wgpu::Texture,
    width: u32,
    height: u32,
    path: &PathBuf,
) {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(width) * u64::from(height) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: surface,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("mapping a buffer this example just wrote")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting on our own submission");
    let pixels = slice
        .get_mapped_range()
        .expect("the map completed above")
        .to_vec();
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(path, ppm).expect("writing the frame");
    eprintln!("wrote {}", path.display());
}

fn main() {
    let (device, queue) = gpu().expect("an adapter");

    let (dx, dy) = parse_pair(&env_or("OPENSHARD_CUBE_OFFSET", "1,1"));
    let height: u8 = env_or("OPENSHARD_CUBE_HEIGHT", "11").parse().expect("a number");

    let a = Point::new(100, 100, 0);
    let b = Point::new((100 + dx) as u16, (100 + dy) as u16, 0);

    // NO_SHOOT so it occludes light at all (a crate's own flags do not:
    // `occlusion::opacity`'s own doc). Not climbable and not FLOOR, and
    // `Shape::UNREAD` says the art named no edge — together that is the
    // `0 | EDGE_ANY` arm of `Builder::add`, one whole-tile body rather than a
    // wall's single named-edge panel.
    let cube_tile = StaticTile {
        flags: TileFlags::new(TileFlags::NO_SHOOT),
        height,
        ..StaticTile::default()
    };
    let bounds = TileBounds {
        min_x: 90,
        max_x: 110,
        min_y: 90,
        max_y: 110,
    };
    let mut builder = Builder::new(bounds);
    builder.add(a.x, a.y, a.z, Graphic(1), &cube_tile, Shape::UNREAD);
    builder.add(b.x, b.y, b.z, Graphic(1), &cube_tile, Shape::UNREAD);
    let occlusion = builder.finish(&Cutaway::OPEN);

    let solid_a = *occlusion
        .solids_at(i32::from(a.x), i32::from(a.y))
        .next()
        .expect("cube A");
    let solid_b = *occlusion
        .solids_at(i32::from(b.x), i32::from(b.y))
        .next()
        .expect("cube B");
    eprintln!(
        "A: x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1}",
        solid_a.space.min.x,
        solid_a.space.max.x,
        solid_a.space.min.y,
        solid_a.space.max.y,
        solid_a.space.min.z,
        solid_a.space.max.z
    );
    eprintln!(
        "B: x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1}",
        solid_b.space.min.x,
        solid_b.space.max.x,
        solid_b.space.min.y,
        solid_b.space.max.y,
        solid_b.space.min.z,
        solid_b.space.max.z
    );

    const RED: [f32; 3] = [255.0, 40.0, 40.0];
    const BLUE: [f32; 3] = [40.0, 120.0, 255.0];

    let forward = vec![(solid_a.space, RED), (solid_b.space, BLUE)];
    let mut reversed = forward.clone();
    reversed.reverse();

    let (width, height_px): (u32, u32) = (512, 512);
    let zoom_notches: u32 = env_or("OPENSHARD_SCENE_ZOOM", "3").parse().expect("a number");
    let mut camera = Camera::new(Point::new((a.x + b.x) / 2, (a.y + b.y) / 2, 0), width, height_px);
    let mut zoom = Zoom::ONE;
    for _ in 0..zoom_notches {
        zoom = zoom.scale_up();
    }
    camera.zoom_about((width / 2) as i32, (height_px / 2) as i32, zoom);

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let base = env_opt("OPENSHARD_FRAME_DUMP").unwrap_or_else(|| "two_cubes".to_string());

    for (suffix, order, style) in [
        (
            "_forward.ppm",
            &forward,
            Style {
                edges: true,
                opaque: false,
            },
        ),
        (
            "_forward_opaque.ppm",
            &forward,
            Style {
                edges: true,
                opaque: true,
            },
        ),
        (
            "_reversed.ppm",
            &reversed,
            Style {
                edges: true,
                opaque: false,
            },
        ),
        (
            "_reversed_opaque.ppm",
            &reversed,
            Style {
                edges: true,
                opaque: true,
            },
        ),
    ] {
        let surface = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("surface"),
            size: wgpu::Extent3d {
                width,
                height: height_px,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let surface_view = surface.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &surface_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut solids_pass = SolidsRenderer::new(&device, format);
        solids_pass.render(
            &device,
            &queue,
            &mut encoder,
            Frame {
                target: &surface_view,
                size: (width, height_px),
                rect: openshard_client_render::blit::ViewportRect {
                    x: 0,
                    y: 0,
                    width,
                    height: height_px,
                },
            },
            &camera,
            order,
            style,
        );
        queue.submit([encoder.finish()]);
        dump(
            &device,
            &queue,
            &surface,
            width,
            height_px,
            &PathBuf::from(format!("{base}{suffix}")),
        );
    }
}
