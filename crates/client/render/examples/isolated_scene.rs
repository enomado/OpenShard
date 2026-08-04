//! A picture of one real place, and only what the caller asks to keep in it.
//!
//! The tool `docs/lighting.md`'s backlog wanted while chasing a corner where a
//! staircase's occlusion box met a lamp's soft pool: `OPENSHARD_FRAME_AT`
//! (`tests/cost.rs`) repoints the camera at a real place but still draws the
//! whole neighbourhood around it, and a house standing beside the thing under
//! test is a second variable in every picture. This draws a **synthetic**
//! [`openshard_uofiles::map::Map`] instead — `Map::from_blocks` never carries
//! statics (see `crate::scene`'s own doc) — and puts back only what is asked
//! for: the real map's statics within a stated radius of a stated point,
//! optionally filtered to a list of tile IDs, the real ground under them or
//! none at all, and any hand-named extra items (a live-shard decoration from
//! `openshard.db`, say — that table is not something this reads itself; see
//! `docs/lighting.md`'s DB-lookup recipe for pulling one out by hand). Turn
//! every knob down and what is left is one tile.
//!
//! Real coordinates translate onto a fixed synthetic anchor
//! ([`SYN_ANCHOR`]) the same way `crate::scene::CENTRE` does it for the
//! built-in scenes — far from the synthetic map's own edge, so a camera never
//! runs off it.
//!
//! # Knobs, all environment variables so no edit-run-revert is ever needed
//!
//! - `OPENSHARD_CLIENT` — the client's files. Required, like every tool here.
//! - `OPENSHARD_SCENE_AT=x,y,z` — the real place. Required: it is both the
//!   centre of the pulled radius and, unless `OPENSHARD_SCENE_LOOK` says
//!   otherwise, where the camera looks.
//! - `OPENSHARD_SCENE_LOOK=x,y,z` — where the camera looks, if not `_AT`.
//! - `OPENSHARD_SCENE_RADIUS=n` — pull real statics from the `(2n+1)^2` tiles
//!   around `_AT`. `0` (the default) is the one tile `_AT` stands on.
//! - `OPENSHARD_SCENE_STATICS=0` — skip the real map's statics entirely
//!   (default: pull them).
//! - `OPENSHARD_SCENE_TILES=0x0739,0x0738` — keep only these tile IDs among
//!   whatever the radius pulled. Unset keeps everything pulled.
//! - `OPENSHARD_SCENE_GROUND=0` — draw no land at all: an empty atlas, so the
//!   ground pass still runs (it always clears) but paints nothing. Default is
//!   the real land, read live off the same facet.
//! - `OPENSHARD_SCENE_EXTRA=x,y,z,graphic[,hue];...` — items to add by hand,
//!   semicolon-separated. What a DB-pulled decoration (or anything else not on
//!   the map) comes in as.
//! - `OPENSHARD_SCENE_VIEWPORT=960x720` — must keep `width * 4` a multiple of
//!   256 (`wgpu`'s row-copy alignment) or the readback panics; the default is
//!   already aligned.
//! - `OPENSHARD_FRAME_DUMP=/tmp/x.ppm` — where to write the picture. Required;
//!   this tool has nothing else to do with a frame once it is drawn.
//! - `OPENSHARD_FRAME_VIEW=n` — index into `debug::View::ALL` (`0` `Lit`, `4`
//!   `Occluders`, `5` `Light`, …). Default `Lit`.
//!
//! # Example: the one tile that made the corner in the user's screenshot
//!
//! ```sh
//! OPENSHARD_CLIENT=… \
//!     OPENSHARD_SCENE_AT=1497,1626,10 OPENSHARD_SCENE_TILES=0x0739,0x0738 \
//!     OPENSHARD_SCENE_GROUND=0 \
//!     OPENSHARD_SCENE_EXTRA=1498,1626,10,2852 \
//!     OPENSHARD_FRAME_DUMP=/tmp/corner.ppm OPENSHARD_FRAME_VIEW=5 \
//!     cargo run --release -p openshard-client-render --example isolated_scene
//! ```

use std::path::PathBuf;

use openshard_client_render::animate::StaticAnimations;
use openshard_client_render::atlas::{LandAtlas, StaticAtlas, TexmapAtlas};
use openshard_client_render::blit::{Blit, ViewportRect};
use openshard_client_render::camera::{Camera, Zoom};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::ground;
use openshard_client_render::items::{self, GroundItem};
use openshard_client_render::renderer::{GroundRenderer, SpriteRenderer, Target};
use openshard_client_render::{light, renderer};
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::Point;
use openshard_uofiles::art::Art;
use openshard_uofiles::map::{LandCell, Map};
use openshard_uofiles::texmaps::TexMaps;
use openshard_uofiles::tiledata::TileData;

/// Where every scene's real anchor lands in the synthetic map — far from its
/// edge, the same convention `crate::scene::CENTRE` uses.
const SYN_ANCHOR: (u16, u16) = (100, 100);

/// A required environment variable, or a clear panic naming which one.
fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn env_flag(name: &str, default: bool) -> bool {
    match env_opt(name).as_deref() {
        None => default,
        Some("0") => false,
        Some("1") => true,
        Some(other) => panic!("{name} wants `0` or `1`, got {other:?}"),
    }
}

/// `x,y,z`, the same shape `tests/cost.rs`'s `OPENSHARD_FRAME_AT` reads.
fn parse_point(spec: &str) -> Point {
    let coords: Vec<&str> = spec.split(',').collect();
    let [x, y, z] = coords[..] else {
        panic!("wanted `x,y,z`, got {spec:?}");
    };
    Point::new(
        x.trim().parse().unwrap_or_else(|_| panic!("x: {x:?}")),
        y.trim().parse().unwrap_or_else(|_| panic!("y: {y:?}")),
        z.trim().parse().unwrap_or_else(|_| panic!("z: {z:?}")),
    )
}

/// A tile ID, decimal or `0x`-prefixed hex — the two shapes `tiledata` dumps
/// and Sphere-flavoured docs both use.
fn parse_tile_id(spec: &str) -> u16 {
    let spec = spec.trim();
    match spec.strip_prefix("0x").or_else(|| spec.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("tile id: {spec:?}")),
        None => spec.parse().unwrap_or_else(|_| panic!("tile id: {spec:?}")),
    }
}

/// `x,y,z,graphic[,hue]`, one item.
fn parse_extra_item(spec: &str) -> GroundItem {
    let parts: Vec<&str> = spec.split(',').collect();
    let (x, y, z, graphic, hue) = match parts[..] {
        [x, y, z, graphic] => (x, y, z, graphic, "0"),
        [x, y, z, graphic, hue] => (x, y, z, graphic, hue),
        _ => panic!("OPENSHARD_SCENE_EXTRA item wants `x,y,z,graphic[,hue]`, got {spec:?}"),
    };
    GroundItem {
        at: Point::new(
            x.trim().parse().unwrap_or_else(|_| panic!("x: {x:?}")),
            y.trim().parse().unwrap_or_else(|_| panic!("y: {y:?}")),
            z.trim().parse().unwrap_or_else(|_| panic!("z: {z:?}")),
        ),
        graphic: Graphic(parse_tile_id(graphic)),
        hue: Hue(hue.trim().parse().unwrap_or_else(|_| panic!("hue: {hue:?}"))),
    }
}

fn shift(anchor: (u16, u16), real: (u16, u16)) -> (u16, u16) {
    (
        (real.0 as i32 - anchor.0 as i32 + SYN_ANCHOR.0 as i32) as u16,
        (real.1 as i32 - anchor.1 as i32 + SYN_ANCHOR.1 as i32) as u16,
    )
}

fn unshift(anchor: (u16, u16), syn: (u16, u16)) -> (u16, u16) {
    (
        (syn.0 as i32 - SYN_ANCHOR.0 as i32 + anchor.0 as i32) as u16,
        (syn.1 as i32 - SYN_ANCHOR.1 as i32 + anchor.1 as i32) as u16,
    )
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// `960x720`-shaped, defaulting to a size whose row is already 256-byte
/// aligned — `wgpu`'s `COPY_BYTES_PER_ROW_ALIGNMENT`, which the readback at
/// the bottom of this file needs and does not itself enforce.
fn viewport() -> (u32, u32) {
    let Some(spec) = env_opt("OPENSHARD_SCENE_VIEWPORT") else {
        return (960, 720);
    };
    let (w, h) = spec
        .split_once('x')
        .unwrap_or_else(|| panic!("OPENSHARD_SCENE_VIEWPORT: {spec:?}"));
    let (w, h): (u32, u32) = (
        w.trim().parse().unwrap_or_else(|_| panic!("width: {w:?}")),
        h.trim().parse().unwrap_or_else(|_| panic!("height: {h:?}")),
    );
    assert!(
        w * 4 % 256 == 0,
        "OPENSHARD_SCENE_VIEWPORT width {w} isn't 256-byte-row aligned (wants a multiple of 64)"
    );
    (w, h)
}

fn main() {
    let dir = PathBuf::from(env("OPENSHARD_CLIENT"));
    let (device, queue) = gpu().expect("an adapter");

    let real_map = Map::load_facet(&dir, 0).expect("Felucca");
    let art = Art::open(&dir).expect("artLegacyMUL.uop");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");

    let at = parse_point(&env("OPENSHARD_SCENE_AT"));
    let anchor = (at.x, at.y);
    let look = env_opt("OPENSHARD_SCENE_LOOK")
        .map(|s| parse_point(&s))
        .unwrap_or(at);

    let radius: u16 = env_opt("OPENSHARD_SCENE_RADIUS")
        .map(|s| {
            s.parse()
                .unwrap_or_else(|_| panic!("OPENSHARD_SCENE_RADIUS: {s:?}"))
        })
        .unwrap_or(0);
    let want_statics = env_flag("OPENSHARD_SCENE_STATICS", true);
    let tile_filter: Option<Vec<u16>> =
        env_opt("OPENSHARD_SCENE_TILES").map(|s| s.split(',').map(parse_tile_id).collect());
    let want_ground = env_flag("OPENSHARD_SCENE_GROUND", true);

    // Land, borrowed live from the real facet at every synthetic cell's real
    // coordinate — or nothing, `_GROUND=0`'s whole point. `Map::from_blocks`
    // never carries statics regardless, so a house never comes along by
    // accident; keeping this a live read rather than a stored blob is what
    // lets `_RADIUS` and `_AT` move the scene to any corner of Britain with no
    // second file to keep in step.
    let synthetic = Map::from_blocks(16, 16, |sx, sy| {
        if !want_ground {
            return LandCell { tile: 0, z: at.z };
        }
        let (rx, ry) = unshift(anchor, (sx, sy));
        real_map.land(rx, ry).unwrap_or(LandCell { tile: 0, z: at.z })
    });

    // The real map's own statics within the radius, translated onto the
    // synthetic anchor and filtered to `_TILES` if it named any. This is what
    // makes the tool general: the caller does not hand-transcribe a tile's
    // graphic and `z` the way this file's first draft did — it is read here,
    // the same way `examples/dump_statics.rs` (this session's throwaway) read
    // it by hand.
    let mut items: Vec<GroundItem> = Vec::new();
    if want_statics {
        for x in at.x.saturating_sub(radius)..=at.x.saturating_add(radius) {
            for y in at.y.saturating_sub(radius)..=at.y.saturating_add(radius) {
                for s in real_map.statics_at(x, y) {
                    if tile_filter
                        .as_ref()
                        .is_some_and(|allowed| !allowed.contains(&s.tile))
                    {
                        continue;
                    }
                    let (sx, sy) = shift(anchor, (x, y));
                    items.push(GroundItem {
                        at: Point::new(sx, sy, s.z),
                        graphic: Graphic(s.tile),
                        hue: Hue(s.hue),
                    });
                }
            }
        }
    }

    // Hand-named extras — typically a live-shard decoration or dropped item
    // pulled from `openshard.db` by the recipe in `docs/lighting.md`, since
    // nothing on the map can see those and nothing here reads the DB itself.
    if let Some(spec) = env_opt("OPENSHARD_SCENE_EXTRA") {
        for entry in spec.split(';').filter(|s| !s.trim().is_empty()) {
            let mut item = parse_extra_item(entry);
            let (sx, sy) = shift(anchor, (item.at.x, item.at.y));
            item.at = Point::new(sx, sy, item.at.z);
            items.push(item);
        }
    }
    assert!(
        !items.is_empty(),
        "the scene is empty: turn on OPENSHARD_SCENE_STATICS, widen OPENSHARD_SCENE_RADIUS, \
         loosen OPENSHARD_SCENE_TILES, or add OPENSHARD_SCENE_EXTRA"
    );

    let viewport = viewport();
    let (look_sx, look_sy) = shift(anchor, (look.x, look.y));
    let mut camera = Camera::new(Point::new(look_sx, look_sy, look.z), viewport.0, viewport.1);
    camera.zoom_about(0, 0, Zoom::ONE);
    let (width, height) = camera.image_size();

    let land_atlas = if want_ground {
        LandAtlas::build(
            &art,
            ground::visible_graphics(&synthetic, &camera).iter().copied(),
        )
        .expect("land fits")
    } else {
        LandAtlas::build(&art, std::iter::empty()).expect("an empty atlas always fits")
    };
    let texmaps = {
        let texmaps_src = TexMaps::open(&dir).expect("texidx.mul and texmaps.mul");
        let wanted = match want_ground {
            true => ground::visible_graphics(&synthetic, &camera),
            false => Default::default(),
        };
        TexmapAtlas::build(&texmaps_src, &tiledata, wanted).expect("textures fit")
    };
    let ground_quads = match want_ground {
        true => ground::collect(&synthetic, &camera, &land_atlas, &texmaps, &Cutaway::OPEN),
        false => Vec::new(),
    };

    let animations = StaticAnimations::default();
    let needed = items::needed_graphics(&items, &animations);
    let static_atlas = StaticAtlas::build(&art, needed).expect("the scene's own items fit");
    let item_quads = items::collect(
        &items,
        &camera,
        &tiledata,
        &animations,
        &static_atlas,
        &Cutaway::OPEN,
        None,
    );

    let format = openshard_client_render::blit::WORLD_FORMAT;
    let world = openshard_client_render::blit::world_texture(&device, width, height);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let place = openshard_client_render::place::texture(&device, width, height);
    let place_view = place.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = renderer::depth_texture(&device, width, height);
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

    let mut ground_pass = GroundRenderer::new(&device, &queue, format, &land_atlas, &texmaps);
    let mut items_pass = SpriteRenderer::new(
        &device,
        &queue,
        format,
        static_atlas.pixels(),
        &openshard_client_render::hue::HueRamp::build(
            &openshard_uofiles::hues::Hues::load(dir.join("hues.mul")).expect("hues.mul"),
        ),
    );
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = Target {
        place: &place_view,
        view: &world_view,
        depth: &depth_view,
        width,
        height,
        projection: camera.projection(),
    };
    // Always run, even with no ground quads: the ground pass is what clears
    // the world/place/depth targets — see its own doc — so skipping it here
    // would leave the items pass drawing over whatever the textures happened
    // to hold.
    ground_pass.render(&device, &queue, &mut encoder, target, &ground_quads);
    items_pass.render(&device, &queue, &mut encoder, target, &item_quads);
    queue.submit([encoder.finish()]);

    let surface = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("surface"),
        size: wgpu::Extent3d {
            width: viewport.0,
            height: viewport.1,
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

    let mut lighting = light::collect(
        &synthetic,
        &items,
        &camera,
        &tiledata,
        &Cutaway::OPEN,
        light::NIGHT.flattened(),
        0.0,
        Some(&static_atlas),
        None,
    );
    eprintln!(
        "{} items, {} flames, {} standing cells",
        items.len(),
        lighting.lights.len(),
        lighting.occlusion.boxes().count(),
    );

    let wanted_view = env_opt("OPENSHARD_FRAME_VIEW")
        .and_then(|v| v.parse::<usize>().ok())
        .and_then(|v| openshard_client_render::debug::View::ALL.get(v).copied())
        .unwrap_or_default();
    lighting.view = wanted_view;

    let mut blit = Blit::new(&device, format);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blit.render(
        &device,
        &queue,
        &mut encoder,
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            place: &place_view,
            zoom: Zoom::ONE,
            rect: ViewportRect {
                x: 0,
                y: 0,
                width: viewport.0,
                height: viewport.1,
            },
        },
        &lighting,
    );
    queue.submit([encoder.finish()]);

    let (w, h) = viewport;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(w) * u64::from(h) * 4,
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
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
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
    let pixels = slice
        .get_mapped_range()
        .expect("the map completed above")
        .to_vec();

    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for pixel in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    let path = PathBuf::from(env("OPENSHARD_FRAME_DUMP"));
    std::fs::write(&path, ppm).expect("writing the frame");
    eprintln!("wrote {} ({:?})", path.display(), wanted_view);
}
