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
//! # Profiling a segment instead of drawing it
//!
//! Setting `OPENSHARD_SCENE_PROFILE_FACE` switches the tool from a picture to a
//! printed table and skips every GPU pass after the scene's lighting is built —
//! `OPENSHARD_FRAME_DUMP`/`_VIEW` are ignored in this mode. For a question a
//! picture cannot answer on its own: whether a hard edge in the render is the
//! occlusion walk (`Reach::through`) or the face's `faces()` cutoff folded into
//! `Reach::cone` (see `light.rs`'s own doc on the two).
//!
//! - `OPENSHARD_SCENE_PROFILE_FACE=north|east|south|west|flat|upright|tread` —
//!   which [`light::Surface`] to sample. The four faces are `Spot::face(...)`'s
//!   `Face`; `tread` is `Surface::Sloped`, decision 40's tread normal, computed
//!   from a `Prism` built off `OPENSHARD_SCENE_PROFILE_TREAD_UP` (which edge it
//!   climbs towards) and `OPENSHARD_SCENE_PROFILE_TREAD_HEIGHTS` (comma-separated
//!   tread heights, low to high) — the same two the real detector would read off
//!   the art. Required to enter profile mode.
//! - `OPENSHARD_SCENE_PROFILE_FROM=x,y,z` / `_TO=x,y,z` — the segment to walk,
//!   in real (fractional allowed) map coordinates. For `tread`, this segment's
//!   own progress (`0` at `_FROM`, `1` at `_TO`) doubles as the run fraction
//!   [`Prism::height_at`] samples, so it should span one tread's own climb.
//! - `OPENSHARD_SCENE_PROFILE_STEPS=n` — how many samples along it, both ends
//!   inclusive. Default `40`.
//! - `OPENSHARD_SCENE_PROFILE_LIGHT=n` — print only [`light::Reach`]s for this
//!   light index. Default: every light the scene collected.
//!
//! ```sh
//! OPENSHARD_CLIENT=… \
//!     OPENSHARD_SCENE_AT=1497,1626,10 OPENSHARD_SCENE_TILES=0x0739,0x0738 \
//!     OPENSHARD_SCENE_GROUND=0 OPENSHARD_SCENE_EXTRA=1498,1626,10,2852 \
//!     OPENSHARD_SCENE_PROFILE_FACE=south \
//!     OPENSHARD_SCENE_PROFILE_FROM=1497.0,1627.0,10 OPENSHARD_SCENE_PROFILE_TO=1497.0,1627.0,16 \
//!     cargo run --release -p openshard-client-render --example isolated_scene
//! ```
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
use openshard_client_render::geometry::Vec2;
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

/// `x,y,z`, all three fractional — [`parse_point`]'s shape, but for a spot
/// [`run_profile`] wants to place anywhere inside a tile rather than on one.
fn parse_fpoint(spec: &str) -> (f32, f32, f32) {
    let coords: Vec<&str> = spec.split(',').collect();
    let [x, y, z] = coords[..] else {
        panic!("wanted `x,y,z`, got {spec:?}");
    };
    (
        x.trim().parse().unwrap_or_else(|_| panic!("x: {x:?}")),
        y.trim().parse().unwrap_or_else(|_| panic!("y: {y:?}")),
        z.trim().parse().unwrap_or_else(|_| panic!("z: {z:?}")),
    )
}

/// `OPENSHARD_SCENE_PROFILE_FACE`'s value, which names a whole [`light::Surface`]
/// and not just a [`Face`](openshard_client_render::facing::Face) — `flat` and
/// `upright` are the other two the place attachment can carry.
///
/// `"tread"` is the fourth, [`light::Surface::Sloped`] (decision 40), and is not
/// resolved here: its normal comes from a [`Prism`](openshard_client_render::facing::Prism)
/// built off [`parse_tread_prism`] plus a tread index, which `run_profile` reads
/// from its own env vars once it knows this is the case that wants them.
fn parse_surface(spec: &str) -> Option<light::Surface> {
    use openshard_client_render::facing::Face;
    match spec.trim().to_ascii_lowercase().as_str() {
        "north" => Some(light::Surface::Face(Face::North)),
        "east" => Some(light::Surface::Face(Face::East)),
        "south" => Some(light::Surface::Face(Face::South)),
        "west" => Some(light::Surface::Face(Face::West)),
        "flat" => Some(light::Surface::Flat),
        "upright" => Some(light::Surface::Upright),
        "tread" => None,
        _ => panic!(
            "OPENSHARD_SCENE_PROFILE_FACE wants north/east/south/west/flat/upright/tread, got {spec:?}"
        ),
    }
}

/// `OPENSHARD_SCENE_PROFILE_TREAD_UP` and `_HEIGHTS`, the two env vars that
/// build the [`Prism`](openshard_client_render::facing::Prism) a `"tread"`
/// profile's normal comes from — required only in that case, since every other
/// [`light::Surface`] needs no geometry beyond its own tag.
fn parse_tread_prism() -> openshard_client_render::facing::Prism {
    use openshard_client_render::facing::{Face, Prism};
    let up = match env("OPENSHARD_SCENE_PROFILE_TREAD_UP")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "north" => Face::North,
        "east" => Face::East,
        "south" => Face::South,
        "west" => Face::West,
        other => panic!("OPENSHARD_SCENE_PROFILE_TREAD_UP wants north/east/south/west, got {other:?}"),
    };
    let heights: Vec<u8> = env("OPENSHARD_SCENE_PROFILE_TREAD_HEIGHTS")
        .split(',')
        .map(|h| h.trim().parse().unwrap_or_else(|_| panic!("tread height: {h:?}")))
        .collect();
    Prism::new(up, &heights).unwrap_or_else(|| {
        panic!("OPENSHARD_SCENE_PROFILE_TREAD_HEIGHTS: {heights:?} is empty or past MAX_TREADS")
    })
}

/// [`shift`], but for the fractional coordinates [`run_profile`] samples at
/// rather than the whole tiles the rest of this file moves onto the synthetic
/// map.
fn shift_f(anchor: (u16, u16), x: f32, y: f32) -> (f32, f32) {
    (
        x - f32::from(anchor.0) + SYN_ANCHOR.0 as f32,
        y - f32::from(anchor.1) + SYN_ANCHOR.1 as f32,
    )
}

/// The profile mode: walk [`Spot::face`](light::Spot::face) along a segment and
/// print what [`light::sample`] says at each step, instead of drawing a frame.
///
/// `through` and `cone` are read straight off [`light::Reach`] rather than
/// re-derived here, which is the whole point — they are the production
/// function's own account of "was something in the way" and "was the surface
/// turned towards the flame", already kept apart for exactly this question. For
/// a scene with one point light (no [`crate::light::Beam`]), `cone` *is* the
/// `faces()` term alone.
fn run_profile(anchor: (u16, u16), lighting: &light::Lighting) {
    let face_spec = env("OPENSHARD_SCENE_PROFILE_FACE");
    let surface = parse_surface(&face_spec);
    // `"tread"` has no fixed normal — it is read off a `Prism` built from the
    // static's own art (`OPENSHARD_SCENE_PROFILE_TREAD_UP`/`_HEIGHTS`), and which
    // tread `t` (this segment's own progress, 0 at `_FROM` and 1 at `_TO`) falls
    // on is taken the same way `Prism::height_at` takes it — the segment this
    // tool profiles a tread with is the run of one tread's own climb, so `t`
    // *is* that tread's run fraction.
    let tread = surface.is_none().then(parse_tread_prism);
    let (fx, fy, fz) = parse_fpoint(&env("OPENSHARD_SCENE_PROFILE_FROM"));
    let (tx, ty, tz) = parse_fpoint(&env("OPENSHARD_SCENE_PROFILE_TO"));
    let steps: u32 = env_opt("OPENSHARD_SCENE_PROFILE_STEPS")
        .map(|s| {
            s.parse()
                .unwrap_or_else(|_| panic!("OPENSHARD_SCENE_PROFILE_STEPS: {s:?}"))
        })
        .unwrap_or(40);
    let only_light: Option<usize> = env_opt("OPENSHARD_SCENE_PROFILE_LIGHT").map(|s| {
        s.parse()
            .unwrap_or_else(|_| panic!("OPENSHARD_SCENE_PROFILE_LIGHT: {s:?}"))
    });

    println!("profile: surface {face_spec}, {steps} steps from ({fx},{fy},{fz}) to ({tx},{ty},{tz})");
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (x, y, z) = (fx + (tx - fx) * t, fy + (ty - fy) * t, fz + (tz - fz) * t);
        let (sx, sy) = shift_f(anchor, x, y);
        let surface = match (surface, &tread) {
            (Some(surface), None) => surface,
            (None, Some(prism)) => {
                let treads = prism.treads();
                let index = ((t.clamp(0.0, 1.0) * treads.len() as f32) as usize).min(treads.len() - 1);
                light::Surface::Sloped(prism.tread_normal(index))
            }
            _ => unreachable!("parse_surface and parse_tread_prism agree on which case this is"),
        };
        let spot = light::Spot {
            at: Vec2::new(sx, sy),
            z,
            surface,
        };
        let sample = light::sample(spot, lighting);
        print!(
            "t={t:.3} ({x:.2}, {y:.2}, z {z:.1}) -> brightness {:.3}",
            sample.brightness()
        );
        for reach in &sample.reaches {
            if only_light.is_some_and(|want| want != reach.light) {
                continue;
            }
            match (reach.within, reach.stopped_by) {
                (false, _) => print!("  | light {}: outside radius", reach.light),
                (true, Some((cx, cy))) => print!("  | light {}: stopped at ({cx}, {cy})", reach.light),
                (true, None) => print!(
                    "  | light {}: through {:.3} cone {:.3}",
                    reach.light, reach.through, reach.cone
                ),
            }
        }
        println!();
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
    // Every solid `_AT`'s own tile holds, in real coordinates — the geometry a
    // profile below is walking across, printed once so the segment it is given
    // does not have to be guessed from the picture.
    let (syn_x, syn_y) = shift(anchor, (at.x, at.y));
    for solid in lighting.occlusion.solids_at(i32::from(syn_x), i32::from(syn_y)) {
        eprintln!(
            "  solid: x {:.3}..{:.3}, y {:.3}..{:.3}, z {:.1}..{:.1}, edges {:#06b}, opacity {}",
            solid.space.min.x,
            solid.space.max.x,
            solid.space.min.y,
            solid.space.max.y,
            solid.space.min.z,
            solid.space.max.z,
            solid.edges,
            solid.opacity,
        );
    }

    // A profile instead of a picture: `light::sample` walked along a segment
    // rather than `Blit` rasterising a frame, for the question a picture cannot
    // answer — whether a hard edge on screen is `Reach::through` (occlusion) or
    // `Reach::cone` (which one of a face's `faces()` and its beam folds into,
    // see `light.rs`) that is doing the cutting. No GPU work past this point.
    if env_opt("OPENSHARD_SCENE_PROFILE_FACE").is_some() {
        run_profile(anchor, &lighting);
        return;
    }

    let wanted_view = env_opt("OPENSHARD_FRAME_VIEW")
        .and_then(|v| v.parse::<usize>().ok())
        .and_then(|v| openshard_client_render::debug::View::ALL.get(v).copied())
        .unwrap_or_default();
    lighting.view = wanted_view;

    let mut blit = Blit::new(&device, format);
    // No mobile pass in this scene: the dummy stands in for it.
    let dummy_instances = openshard_client_render::blit::dummy_instances(&device);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    blit.render(
        &device,
        &queue,
        &mut encoder,
        openshard_client_render::blit::Frame {
            target: &surface_view,
            world: &world_view,
            place: &place_view,
            face_instances: items_pass.instances_buffer(),
            mobile_instances: &dummy_instances,
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
