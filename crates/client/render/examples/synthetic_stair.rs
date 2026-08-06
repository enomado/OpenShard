//! A climbable static, alone: no client files, no map, no art — just one
//! [`facing::Prism`] on one tile, its own [`occlusion::Occlusion`], and one
//! flame, rendered through the real GPU pipeline
//! (`GroundRenderer`/`MeshFaceRenderer`/`Blit`) and dumped as a picture.
//!
//! `isolated_scene`'s minimal-scene idea without the client dependency: where
//! `isolated_scene` reads a real static's real art and tiledata to find out
//! whether it is climbable, this builds a `Prism` by hand and hands it
//! straight to [`occlusion::Shape::solid`] — the same construction
//! `light.rs`'s own `a_treads_top_is_not_shadowed_by_its_own_riser` test
//! uses. What that buys is a scene with nothing in it to misread: a lamppost,
//! a texture, a second static, all gone, so a shape seen in the picture is a
//! shape this file's own few lines of geometry produced.
//!
//! - `OPENSHARD_STAIR_UP=north|east|south|west` — which side the climb faces.
//!   Default `north`.
//! - `OPENSHARD_STAIR_TREADS=h1,h2,...` — [`facing::Prism::new`]'s own height
//!   profile, each above the static's own base at `z 0`. Default `1,3,5`,
//!   the same modest three-step rise `light.rs`'s own fixtures climb —
//!   `11,13,15` looks like a real staircase only because the real one it was
//!   copied from stands on a `z 10` base; used as absolute heights from `z 0`
//!   here, it renders five times too tall.
//! - `OPENSHARD_LIGHT_AT=dx,dy` — the flame's position, offset from the
//!   static's own tile. Default `2.5,1.0`, which lights the nearer tread and
//!   leaves the far one in its own riser's shadow — a light held level with
//!   any one tread instead reads *nothing* in the way at all
//!   (`Surface::shadowed_by_own_tile`'s exemption, decision 32), which is the
//!   wrong shape for looking at a shadow.
//! - `OPENSHARD_LIGHT_Z` / `OPENSHARD_LIGHT_RADIUS` — default `2` and `6`.
//! - `OPENSHARD_FRAME_VIEW=n` — an index into `debug::View::ALL`; `7` is
//!   `Shadow`. Default `0`, `Lit` — mostly uninformative here, since this
//!   scene draws no billboard under the mesh, but the same index every other
//!   tool in this crate uses.
//! - `OPENSHARD_SCENE_ZOOM=n` — notches of `Zoom::scale_up`. Default `3`,
//!   already the ladder's own maximum (`4:1`) from `Zoom::ONE`.
//!
//! The picture gets one more mark that is not the shader's own output: a lime
//! crosshair on a black backing plate at the flame's own projected position,
//! because "is the light behind the stair or in front of it" is faster to
//! answer by looking than by reading `OPENSHARD_LIGHT_AT` back.
//!
//! ```sh
//! OPENSHARD_FRAME_VIEW=7 OPENSHARD_FRAME_DUMP=/tmp/stair.ppm \
//!     cargo run --release -p openshard-client-render --example synthetic_stair
//! ```

use std::path::PathBuf;

use openshard_client_render::blit::{Blit, ViewportRect};
use openshard_client_render::camera::{Camera, Zoom, project_exact};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::debug::View;
use openshard_client_render::facing::{Face, Prism};
use openshard_client_render::light::{Light, Lighting};
use openshard_client_render::mesh_face::{MeshFaceRow, MeshFaceVertex};
use openshard_client_render::occlusion::{Builder, Shape};
use openshard_client_render::place::Stance;
use openshard_client_render::renderer::{self, GroundRenderer, MeshFaceRenderer, Target};
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::tiledata::{StaticTile, TileFlags};

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn env_or(name: &str, default: &str) -> String {
    env_opt(name).unwrap_or_else(|| default.to_string())
}

fn parse_face(spec: &str) -> Face {
    match spec.trim().to_ascii_lowercase().as_str() {
        "north" => Face::North,
        "east" => Face::East,
        "south" => Face::South,
        "west" => Face::West,
        _ => panic!("OPENSHARD_STAIR_UP wants north/east/south/west, got {spec:?}"),
    }
}

fn parse_treads(spec: &str) -> Vec<u8> {
    spec.split(',')
        .map(|s| s.trim().parse().unwrap_or_else(|_| panic!("tread height: {s:?}")))
        .collect()
}

fn parse_pair(spec: &str) -> (f32, f32) {
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

fn main() {
    let (device, queue) = gpu().expect("an adapter");

    let up = parse_face(&env_or("OPENSHARD_STAIR_UP", "north"));
    let treads = parse_treads(&env_or("OPENSHARD_STAIR_TREADS", "1,3,5"));
    let prism = Prism::new(up, &treads).expect("1..=MAX_TREADS heights");
    let at = Point::new(100, 100, 0);

    let stair = StaticTile {
        flags: TileFlags::new(TileFlags::NO_SHOOT | TileFlags::CLIMBABLE),
        height: 20,
        ..StaticTile::default()
    };
    let bounds = openshard_client_render::camera::TileBounds {
        min_x: 95,
        max_x: 105,
        min_y: 95,
        max_y: 105,
    };
    let mut builder = Builder::new(bounds);
    builder.add(at.x, at.y, at.z, Graphic(0x0736), &stair, Shape::solid(prism));
    let occlusion = builder.finish(&Cutaway::OPEN);

    for solid in occlusion.solids_at(i32::from(at.x), i32::from(at.y)) {
        eprintln!(
            "solid: x {:.3}..{:.3}, y {:.3}..{:.3}, z {:.1}..{:.1}, edges {:#06b}",
            solid.space.min.x,
            solid.space.max.x,
            solid.space.min.y,
            solid.space.max.y,
            solid.space.min.z,
            solid.space.max.z,
            solid.edges,
        );
    }

    let (width, height): (u32, u32) = (512, 512);
    let zoom_notches: u32 = env_or("OPENSHARD_SCENE_ZOOM", "3").parse().expect("a number");
    let mut camera = Camera::new(at, width, height);
    let mut zoom = Zoom::ONE;
    for _ in 0..zoom_notches {
        zoom = zoom.scale_up();
    }
    camera.zoom_about((width / 2) as i32, (height / 2) as i32, zoom);

    const DEPTH: f32 = 0.5;
    let mesh = prism.mesh(i32::from(at.x), i32::from(at.y), i32::from(at.z));
    let mut vertices: Vec<MeshFaceVertex> = Vec::new();
    let mut rows: Vec<MeshFaceRow> = Vec::new();
    for face in mesh.faces() {
        let id = rows.len() as u32;
        rows.push(MeshFaceRow {
            tile: (at.x, at.y),
            stance: Stance::of_normal(face.normal).expect("a stair's own normals are all recognized"),
        });
        for corner in face.fan() {
            let screen = camera.to_view_exact(project_exact(corner));
            vertices.push(MeshFaceVertex {
                screen,
                world: [corner.x as f32, corner.y as f32, corner.z as f32],
                depth: DEPTH,
                id,
                tile: [f32::from(at.x), f32::from(at.y)],
            });
        }
    }
    eprintln!("{} faces, {} vertices", rows.len(), vertices.len());
    for (id, row) in rows.iter().enumerate() {
        let corners: Vec<&MeshFaceVertex> = vertices.iter().filter(|v| v.id == id as u32).collect();
        let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for c in &corners {
            minx = minx.min(c.screen.x);
            maxx = maxx.max(c.screen.x);
            miny = miny.min(c.screen.y);
            maxy = maxy.max(c.screen.y);
        }
        eprintln!(
            "face {id}: stance {:?}, screen x {minx:.1}..{maxx:.1}, y {miny:.1}..{maxy:.1}",
            row.stance,
        );
    }

    let format = openshard_client_render::blit::WORLD_FORMAT;
    let world = openshard_client_render::blit::world_texture(&device, width, height);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(&device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_tex = renderer::depth_texture(&device, width, height);
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let land = openshard_client_render::atlas::LandAtlas::pack([]).expect("nothing always fits");
    let texmaps = openshard_client_render::atlas::TexmapAtlas::pack([]).expect("nothing always fits");
    let mut ground_pass = GroundRenderer::new(&device, &queue, format, &land, &texmaps);
    let mut mesh_pass = MeshFaceRenderer::new(&device);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = Target {
        place: &place_view,
        view: &world_view,
        depth: &depth_view,
        width,
        height,
        projection: camera.projection(),
    };
    ground_pass.render(&device, &queue, &mut encoder, target, &[]);
    mesh_pass.render(&device, &queue, &mut encoder, target, &vertices, &rows);
    queue.submit([encoder.finish()]);

    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
        size: wgpu::Extent3d {
            width,
            height,
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
    let mut blit = Blit::new(&device, format);

    let (ldx, ldy) = parse_pair(&env_or("OPENSHARD_LIGHT_AT", "2.5,1.0"));
    let light_z: f32 = env_or("OPENSHARD_LIGHT_Z", "2").parse().expect("a number");
    let light_radius: f32 = env_or("OPENSHARD_LIGHT_RADIUS", "6").parse().expect("a number");
    eprintln!("light: at ({ldx:+}, {ldy:+}) of the tile, z {light_z}, radius {light_radius}");
    let lighting = Lighting {
        ambient: openshard_client_render::light::NIGHT,
        lights: vec![Light {
            at: openshard_client_render::geometry::Vec2::new(f32::from(at.x) + ldx, f32::from(at.y) + ldy),
            z: light_z,
            radius: light_radius,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            beam: None,
        }],
        occlusion,
        sun: None,
        view: View::ALL[env_or("OPENSHARD_FRAME_VIEW", "0")
            .parse::<usize>()
            .expect("an index")],
    };
    let dummy_instances = openshard_client_render::blit::dummy_instances(&device);
    let dummy_ground_instances = openshard_client_render::blit::dummy_ground_instances(&device);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blit.render(
        &device,
        &queue,
        &mut encoder,
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            place: &place_view,
            face_instances: &dummy_instances,
            mobile_instances: &dummy_instances,
            mesh_instances: mesh_pass.rows_buffer(),
            ground_instances: &dummy_ground_instances,
            zoom: Zoom::ONE,
            rect: ViewportRect {
                x: 0,
                y: 0,
                width,
                height,
            },
        },
        &lighting,
    );
    queue.submit([encoder.finish()]);

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(width) * u64::from(height) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &surface,
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
        result.expect("mapping a buffer this example just wrote");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting on our own submission");
    let mut pixels = slice
        .get_mapped_range()
        .expect("the map completed above")
        .to_vec();

    // Where the flame itself projects to, marked directly on the picture: a
    // number in a log line does not answer "is the light behind the stair or
    // in front of it" nearly as fast as a mark on the frame does.
    let projection = camera.projection();
    let light_screen = camera.to_view_exact(project_exact(openshard_client_render::camera::WorldSpot {
        x: f64::from(at.x) + f64::from(ldx),
        y: f64::from(at.y) + f64::from(ldy),
        z: f64::from(light_z),
    }));
    let light_pixel = (
        (light_screen.x - projection.origin.x) * projection.scale + width as f32 * 0.5,
        (light_screen.y - projection.origin.y) * projection.scale + height as f32 * 0.5,
    );
    eprintln!("light pixel: {light_pixel:?}");
    let (lpx, lpy) = (light_pixel.0.round() as i32, light_pixel.1.round() as i32);
    let mut mark = |dx: i32, dy: i32, colour: [u8; 3]| {
        let (px, py) = (lpx + dx, lpy + dy);
        if px < 0 || py < 0 || px as u32 >= width || py as u32 >= height {
            return;
        }
        let at_pixel = ((py as u32 * width + px as u32) * 4) as usize;
        pixels[at_pixel] = colour[0];
        pixels[at_pixel + 1] = colour[1];
        pixels[at_pixel + 2] = colour[2];
        pixels[at_pixel + 3] = 255;
    };
    // A big lime crosshair on a black backing plate — thick enough (a 41-pixel
    // arm on a 512-pixel frame) to find at a glance, and the black backing is
    // what keeps it visible crossing a already-white wall pixel, not just a
    // black one.
    for dy in -20..=20i32 {
        for dx in -20..=20i32 {
            if dx.abs() > 2 && dy.abs() > 2 {
                continue;
            }
            mark(dx, dy, [0, 0, 0]);
        }
    }
    for dy in -18..=18i32 {
        for dx in -18..=18i32 {
            if dx.abs() > 1 && dy.abs() > 1 {
                continue;
            }
            mark(dx, dy, [80, 255, 0]);
        }
    }

    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    let path = env_opt("OPENSHARD_FRAME_DUMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("synthetic_stair.ppm"));
    std::fs::write(&path, ppm).expect("writing the frame");
    eprintln!("wrote {}", path.display());
}
