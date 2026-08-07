//! A general lit-pipeline scene of hand-built boxes: no client files, no
//! map, no tile-shaped footprints required — a generalisation of
//! `examples/two_cubes.rs`'s own "through the real lit pipeline" section
//! (see that module's doc for why it is `GroundRenderer`/`MeshFaceRenderer`/
//! `Blit` and not `SolidsRenderer`) to any number of boxes, each an
//! arbitrary axis-aligned span rather than always a whole tile.
//!
//! `two_cubes.rs`'s own two boxes both come from `occlusion::Builder::add`,
//! which only ever produces a whole-tile body or an edge panel — a real
//! static's footprint always is one of those two shapes, because that is
//! what `tiledata` states. A scene with no static at all is not bound by
//! that: this tool needs boxes narrower than a tile, and boxes stacked on
//! top of each other rather than standing side by side, and neither shape
//! exists in `tiledata`. `occlusion::Builder::add_raw` is the seam that
//! makes that honest instead of worked around — it stores exactly the AABB
//! given, in the same tile bucket every other occluder uses, so the walk
//! finds it exactly the way it finds a wall. Each [`BoxSpec`] here states
//! both its footprint and the one tile bucket that owns it, and both the
//! occluder and the visible mesh (`box_mesh`, copied from `two_cubes.rs`'s
//! own — see that module's doc for why it is built from exact corners and
//! not two `facing::Prism`s) are built from that same [`BoxSpec`], so the
//! two can never disagree the way `two_cubes.rs`'s session 13 bug did.
//!
//! - `OPENSHARD_BOXES_SCENE=tree|line` — which scene to build. Default
//!   `tree`: two boxes on one tile, the lower a half-tile footprint, the
//!   upper a third-tile footprint standing directly on top of it — small on
//!   purpose, to see whether a shape that narrow still throws a shadow with
//!   the outline its own silhouette suggests. `line` is `two_cubes.rs`'s own
//!   default shape (two whole-tile boxes, offset `1,0` — due east, a
//!   straight line rather than a diagonal) at a shorter height, built here
//!   only so both scenes go through one tool with one set of knobs.
//! - `OPENSHARD_SCENE_ZOOM=n` — notches of `Zoom::scale_up`. Default `7` for
//!   `tree`, `3` for `line` (`two_cubes.rs`'s own default): `tree`'s boxes
//!   are sub-tile, so the same 512×512 canvas needs a much closer zoom to
//!   fill it.
//! - `OPENSHARD_LIGHT_AT=dx,dy` / `OPENSHARD_LIGHT_Z` / `OPENSHARD_LIGHT_RADIUS`
//!   — same meaning as `two_cubes.rs`'s own, offset from the scene's first
//!   box's own tile. Both scenes' defaults put the flame up and to the
//!   boxes' `+x` side, close enough that the boxes and their own shadow sit
//!   inside the torch's own reach rather than the whole canvas — `NIGHT`
//!   ambient means most of a 512×512 frame around a single torch is dark
//!   regardless, and that is a fact about a torch at night, not a bug. Picked
//!   by looking at the rendered picture and not by arithmetic
//!   (`two_cubes.rs`'s own session 13 lesson), so treat either default as a
//!   starting point to override rather than a fact about which way is
//!   "screen right".
//! - `OPENSHARD_TREE_H1`/`_H2`/`_W1`/`_W2` — the `tree` scene's own two
//!   heights and two footprint widths (tile fractions), default `3`/`3`/
//!   `0.5`/`0.33333`. For pushing the shapes further apart than the default
//!   — a wider gap between the two boxes' own silhouettes is easier to read
//!   a shadow's own edge against.
//! - `OPENSHARD_FRAME_DUMP=/tmp/x` — base path; writes `<path>_lit.ppm` and
//!   `<path>_shadow.ppm` beside it, both marked with a lime crosshair at the
//!   flame's own projected position (`synthetic_stair.rs`'s own trick).
//!
//! # The oracle
//!
//! A rendered picture answers "does this look right" and nothing sharper —
//! session 14's own account (`docs/lighting_raymarch.md`) of chasing three
//! reported artefacts by eye through several renders is the argument against
//! trusting that alone twice. `oracle_visible`/[`segment_clear_of_box`] is a
//! second, deliberately independent answer: a bare point-light-vs-AABB
//! visibility test, the textbook slab method, written fresh rather than
//! calling the engine's own private `light::ray_vs_solid` — reusing the
//! thing under test to check the thing under test proves nothing about
//! either. Runs by default (`OPENSHARD_BOXES_ORACLE=0` to skip it), over a
//! grid of each box's own top, next to what the engine's own CPU walk
//! (`light::sample`, the same arithmetic `blit.wgsl` runs, held to it by the
//! parity suite `docs/lighting_raymarch.md` calls decision 9) says for the
//! same points, written as one side-by-side comparison PPM per box
//! (`<path>_oracle_box<N>.ppm`: oracle | engine | signed diff) plus a
//! disagreement count on stderr. `OPENSHARD_BOXES_ORACLE_EXACT=1` swaps the
//! engine side to `light::sample_exact` (`walk_cells_exact`, the ray-vs-Solid
//! primitive session 8-11 built) instead of `light::sample`'s own
//! `walk_cells` — this is how session 14 found that `walk_cells`'s `EDGE_ANY`
//! body arm (`light.rs:2269`) tests a candidate tile's `z`-span alone and
//! never its `x`/`y` footprint, so a body narrower than its own tile (every
//! box `occlusion::Builder::add_raw` can build, none `Builder::add` ever
//! could) shadows as if it filled the whole tile: `tree`'s default scene
//! disagreed with the oracle on 3027 of 9216 sampled points of the lower
//! box's own top through `walk_cells`, 480 through `walk_cells_exact` (the
//! remainder is the soft edge of a real penumbra against the oracle's own
//! hard step, not a further bug). **`walk_cells_exact` is not wired into
//! `blit.wgsl`, and wiring it would not be enough on its own even if it
//! were**: `Occlusion::solid_bytes` (`occlusion.rs:1259`), what the GPU
//! actually reads a solid's shape from, uploads four bytes a solid —
//! `(z_bottom, z_top, opacity, edges)` — no `x`/`y` at all, because every
//! real static's footprint is already implied by which tile bucket it is in
//! plus its own edges. See the plan doc's own backlog entry for where this
//! goes next.
//!
//! ```sh
//! OPENSHARD_FRAME_DUMP=/tmp/tree OPENSHARD_BOXES_SCENE=tree \
//!     cargo run --release -p openshard-client-render --example boxes
//! ```

use std::path::PathBuf;

use openshard_client_render::atlas::{LandAtlas, TexmapAtlas};
use openshard_client_render::blit::Frame as BlitFrame;
use openshard_client_render::camera::{Camera, WorldSpot, Zoom, project_exact};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::debug::View;
use openshard_client_render::depth;
use openshard_client_render::geometry::Vec2;
use openshard_client_render::light::{self, Light, Lighting, NIGHT};
use openshard_client_render::mesh::{Face, Mesh};
use openshard_client_render::mesh_face::{MeshFaceRow, MeshFaceVertex};
use openshard_client_render::occlusion::Builder;
use openshard_client_render::place::Stance;
use openshard_client_render::renderer::{self, GroundRenderer, MeshFaceRenderer, Target};
use openshard_client_render::solid::Solid;
use openshard_protocol::wire::Graphic;
use openshard_uofiles::tiledata::{StaticTile, TileFlags};

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn env_or(name: &str, default: &str) -> String {
    env_opt(name).unwrap_or_else(|| default.to_string())
}

fn parse_pair_f32(spec: &str) -> (f32, f32) {
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

/// A lime crosshair on a black backing plate — copied from
/// `examples/synthetic_stair.rs`'s own marker.
fn mark_crosshair(pixels: &mut [u8], width: u32, height: u32, at: (i32, i32)) {
    let mut mark = |dx: i32, dy: i32, colour: [u8; 3]| {
        let (px, py) = (at.0 + dx, at.1 + dy);
        if px < 0 || py < 0 || px as u32 >= width || py as u32 >= height {
            return;
        }
        let offset = ((py as u32 * width + px as u32) * 4) as usize;
        pixels[offset] = colour[0];
        pixels[offset + 1] = colour[1];
        pixels[offset + 2] = colour[2];
        pixels[offset + 3] = 255;
    };
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
}

fn dump(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &wgpu::Texture,
    width: u32,
    height: u32,
    path: &PathBuf,
    mark: Option<(i32, i32)>,
) -> Vec<u8> {
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
    let mut marked = pixels.clone();
    if let Some(at) = mark {
        mark_crosshair(&mut marked, width, height, at);
    }
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in marked.chunks_exact(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(path, ppm).expect("writing the frame");
    eprintln!("wrote {}", path.display());
    pixels
}

/// One hand-built occluder and its visible box, together: the tile bucket
/// [`openshard_client_render::occlusion::Builder::add_raw`] and
/// [`MeshFaceRow::tile`] both need, and the exact `min`/`max` corners both
/// [`box_mesh`] and the occluder's own `space` are built from — one spec,
/// so mesh and occlusion cannot disagree about where the box stands.
struct BoxSpec {
    tile: (u16, u16),
    min: (f64, f64, f64),
    max: (f64, f64, f64),
}

impl BoxSpec {
    fn solid(&self) -> Solid {
        Solid {
            min: WorldSpot {
                x: self.min.0,
                y: self.min.1,
                z: self.min.2,
            },
            max: WorldSpot {
                x: self.max.0,
                y: self.max.1,
                z: self.max.2,
            },
        }
    }
}

/// A "christmas tree": one tile, two boxes stacked on it, each narrower than
/// the one under it. The lower is a half-tile footprint, the upper a
/// third-tile footprint standing exactly on the lower's own top — the two
/// shapes `OPENSHARD_BOXES_SCENE`'s own doc says `occlusion::Builder::add`
/// cannot produce, which is the whole reason this tool exists.
fn scene_tree() -> Vec<BoxSpec> {
    let (tx, ty) = (100u16, 100u16);
    let (cx, cy) = (f64::from(tx) + 0.5, f64::from(ty) + 0.5);
    let h1: f64 = env_or("OPENSHARD_TREE_H1", "3").parse().expect("a number");
    let h2: f64 = env_or("OPENSHARD_TREE_H2", "3").parse().expect("a number");
    let w1: f64 = env_or("OPENSHARD_TREE_W1", "0.5").parse().expect("a number");
    let w2: f64 = env_or("OPENSHARD_TREE_W2", "0.33333").parse().expect("a number");
    vec![
        BoxSpec {
            tile: (tx, ty),
            min: (cx - w1 / 2.0, cy - w1 / 2.0, 0.0),
            max: (cx + w1 / 2.0, cy + w1 / 2.0, h1),
        },
        BoxSpec {
            tile: (tx, ty),
            min: (cx - w2 / 2.0, cy - w2 / 2.0, h1),
            max: (cx + w2 / 2.0, cy + w2 / 2.0, h1 + h2),
        },
    ]
}

/// Two whole-tile boxes in a straight line due east — `two_cubes.rs`'s own
/// default shape (`OPENSHARD_CUBE_OFFSET`'s default `1,1`) offset `1,0`
/// instead, at a shorter height than that tool's own default `11`.
fn scene_line() -> Vec<BoxSpec> {
    let (ax, ay) = (100u16, 100u16);
    let (bx, by) = (101u16, 100u16);
    let h = 4.0;
    vec![
        BoxSpec {
            tile: (ax, ay),
            min: (f64::from(ax), f64::from(ay), 0.0),
            max: (f64::from(ax) + 1.0, f64::from(ay) + 1.0, h),
        },
        BoxSpec {
            tile: (bx, by),
            min: (f64::from(bx), f64::from(by), 0.0),
            max: (f64::from(bx) + 1.0, f64::from(by) + 1.0, h),
        },
    ]
}

/// The same three-face box `two_cubes.rs` builds by hand — top, `+x`,
/// `+y` — from a `Solid`'s exact corners, so all three faces share
/// bit-identical vertices at the box's own edges instead of the crack two
/// independent `facing::Prism`s left (`two_cubes.rs`'s own doc has the full
/// account of that bug).
fn box_mesh(solid: Solid) -> Mesh {
    let (lo, hi) = (solid.min, solid.max);
    let mut mesh = Mesh::EMPTY;
    mesh.push(
        Face::new(
            &[
                WorldSpot {
                    x: lo.x,
                    y: lo.y,
                    z: hi.z,
                },
                WorldSpot {
                    x: hi.x,
                    y: lo.y,
                    z: hi.z,
                },
                WorldSpot {
                    x: hi.x,
                    y: hi.y,
                    z: hi.z,
                },
                WorldSpot {
                    x: lo.x,
                    y: hi.y,
                    z: hi.z,
                },
            ],
            [0.0, 0.0, 1.0],
        )
        .expect("a 4-corner ring"),
    );
    mesh.push(
        Face::new(
            &[
                WorldSpot {
                    x: hi.x,
                    y: lo.y,
                    z: hi.z,
                },
                WorldSpot {
                    x: hi.x,
                    y: hi.y,
                    z: hi.z,
                },
                WorldSpot {
                    x: hi.x,
                    y: hi.y,
                    z: lo.z,
                },
                WorldSpot {
                    x: hi.x,
                    y: lo.y,
                    z: lo.z,
                },
            ],
            [1.0, 0.0, 0.0],
        )
        .expect("a 4-corner ring"),
    );
    mesh.push(
        Face::new(
            &[
                WorldSpot {
                    x: lo.x,
                    y: hi.y,
                    z: hi.z,
                },
                WorldSpot {
                    x: hi.x,
                    y: hi.y,
                    z: hi.z,
                },
                WorldSpot {
                    x: hi.x,
                    y: hi.y,
                    z: lo.z,
                },
                WorldSpot {
                    x: lo.x,
                    y: hi.y,
                    z: lo.z,
                },
            ],
            [0.0, 1.0, 0.0],
        )
        .expect("a 4-corner ring"),
    );
    mesh
}

/// A point-light-vs-axis-aligned-box visibility test, the textbook slab
/// method, written fresh here rather than calling the engine's own private
/// `light::ray_vs_solid` — reusing the thing under test to check the thing
/// under test would only prove the two agree with each other, not that
/// either is right. `true` when the open segment `from..to` (both ends
/// excluded by a small margin, so touching the box's own surface at either
/// end does not count as a hit) never enters `min..max`.
fn segment_clear_of_box(
    from: (f64, f64, f64),
    to: (f64, f64, f64),
    min: (f64, f64, f64),
    max: (f64, f64, f64),
) -> bool {
    let d = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
    let (mut t0, mut t1): (f64, f64) = (1e-4, 1.0 - 1e-4);
    for (o, d, lo, hi) in [
        (from.0, d.0, min.0, max.0),
        (from.1, d.1, min.1, max.1),
        (from.2, d.2, min.2, max.2),
    ] {
        if d.abs() < 1e-12 {
            if o < lo || o > hi {
                return true;
            }
            continue;
        }
        let (mut near, mut far) = ((lo - o) / d, (hi - o) / d);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return true;
        }
    }
    false
}

/// Whether `point` can see `light` at all, geometrically — every box but
/// `skip` (the one `point` itself rests on, which must not shadow itself)
/// tested by [`segment_clear_of_box`].
fn oracle_visible(point: (f64, f64, f64), light: (f64, f64, f64), boxes: &[BoxSpec], skip: usize) -> bool {
    boxes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != skip)
        .all(|(_, b)| segment_clear_of_box(point, light, b.min, b.max))
}

/// A flat grayscale raster, `sampler` called once per pixel with the world
/// `(x, y)` a top-down orthographic view over `min..max` puts there — no
/// relation to the scene's own isometric camera, on purpose: reading this
/// image takes no more of the renderer's own arithmetic on trust than
/// [`oracle_visible`] already does.
fn raster_top_down(
    side: u32,
    min: (f64, f64),
    max: (f64, f64),
    sampler: impl Fn(f64, f64) -> f32,
) -> Vec<u8> {
    let mut pixels = vec![0u8; (side * side * 3) as usize];
    for row in 0..side {
        for col in 0..side {
            let u = (col as f64 + 0.5) / f64::from(side);
            let v = (row as f64 + 0.5) / f64::from(side);
            let x = min.0 + u * (max.0 - min.0);
            let y = min.1 + v * (max.1 - min.1);
            let value = (sampler(x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
            let at = ((row * side + col) * 3) as usize;
            pixels[at] = value;
            pixels[at + 1] = value;
            pixels[at + 2] = value;
        }
    }
    pixels
}

/// Three side-by-side [`raster_top_down`] strips — oracle, engine, and their
/// signed difference (red where the engine is darker than the oracle says it
/// should be, cyan where it is lighter) — written as one wide PPM so the two
/// pictures sit at the same scale without a second tool to align them.
fn write_oracle_comparison(path: &std::path::Path, side: u32, oracle: &[u8], engine: &[u8]) {
    let mut diff = vec![0u8; (side * side * 3) as usize];
    for i in 0..(side * side) as usize {
        let (o, e) = (i32::from(oracle[i * 3]), i32::from(engine[i * 3]));
        let d = e - o;
        if d < 0 {
            diff[i * 3] = d.unsigned_abs().min(255) as u8;
        } else {
            diff[i * 3 + 1] = d.min(255) as u8;
            diff[i * 3 + 2] = d.min(255) as u8;
        }
    }
    let width = side * 3 + 4;
    let mut ppm = format!("P6\n{width} {side}\n255\n").into_bytes();
    for row in 0..side {
        for (strip, gap) in [(oracle, true), (engine, true), (&diff[..], false)] {
            let start = (row * side * 3) as usize;
            ppm.extend_from_slice(&strip[start..start + (side * 3) as usize]);
            if gap {
                for _ in 0..2 {
                    ppm.extend_from_slice(&[64, 64, 64]);
                }
            }
        }
    }
    std::fs::write(path, ppm).expect("writing the oracle comparison");
    eprintln!("wrote {}", path.display());
}

fn main() {
    let (device, queue) = gpu().expect("an adapter");

    let scene_name = env_or("OPENSHARD_BOXES_SCENE", "tree");
    let boxes = match scene_name.as_str() {
        "tree" => scene_tree(),
        "line" => scene_line(),
        other => panic!("unknown OPENSHARD_BOXES_SCENE {other:?}, wanted tree or line"),
    };
    eprintln!("scene {scene_name:?}: {} boxes", boxes.len());
    for (i, b) in boxes.iter().enumerate() {
        eprintln!(
            "box {i}: tile {:?}, x {:.3}..{:.3} y {:.3}..{:.3} z {:.3}..{:.3}",
            b.tile, b.min.0, b.max.0, b.min.1, b.max.1, b.min.2, b.max.2
        );
    }

    let min_tx = boxes.iter().map(|b| b.tile.0).min().expect("at least one box");
    let max_tx = boxes.iter().map(|b| b.tile.0).max().expect("at least one box");
    let min_ty = boxes.iter().map(|b| b.tile.1).min().expect("at least one box");
    let max_ty = boxes.iter().map(|b| b.tile.1).max().expect("at least one box");
    let bounds = openshard_client_render::camera::TileBounds {
        min_x: i32::from(min_tx) - 5,
        max_x: i32::from(max_tx) + 5,
        min_y: i32::from(min_ty) - 5,
        max_y: i32::from(max_ty) + 5,
    };

    // NO_SHOOT so a box occludes light at all (`occlusion::opacity`'s own
    // doc: a graphic's own flags decide it, not the shape). `height` here is
    // only what `depth::static_priority_z` reads off it (whether the box has
    // any height at all); the occluder's real span comes from `add_raw`'s
    // own `space`, not from this.
    let cube_tile = StaticTile {
        flags: TileFlags::new(TileFlags::NO_SHOOT),
        height: 1,
        ..StaticTile::default()
    };

    let mut builder = Builder::new(bounds);
    for b in &boxes {
        builder.add_raw(b.tile.0, b.tile.1, b.solid());
    }
    let occlusion = builder.finish(&Cutaway::OPEN);

    let (width, height_px): (u32, u32) = (512, 512);
    // `tree`'s boxes are sub-tile, so its default sits three notches past
    // `line`'s — found by looking at a rendered frame at each notch, not by
    // arithmetic (`two_cubes.rs`'s own session 13 lesson): the fixed 512×512
    // canvas holds a whole-tile box comfortably at `line`'s zoom, and a
    // half-tile one only once zoomed in this much further.
    let default_zoom = if scene_name == "tree" { "7" } else { "3" };
    let zoom_notches: u32 = env_or("OPENSHARD_SCENE_ZOOM", default_zoom)
        .parse()
        .expect("a number");
    let centre_x = (i32::from(min_tx) + i32::from(max_tx)) / 2;
    let centre_y = (i32::from(min_ty) + i32::from(max_ty)) / 2;
    let mut camera = Camera::new(
        openshard_protocol::world::Point::new(centre_x as u16, centre_y as u16, 0),
        width,
        height_px,
    );
    let mut zoom = Zoom::ONE;
    for _ in 0..zoom_notches {
        zoom = zoom.scale_up();
    }
    camera.zoom_about((width / 2) as i32, (height_px / 2) as i32, zoom);

    let base_tile = depth::base_for(centre_x, centre_y);
    let mut rows: Vec<MeshFaceRow> = Vec::new();
    let mut vertices: Vec<MeshFaceVertex> = Vec::new();
    for b in &boxes {
        let solid = b.solid();
        let d = depth::Order {
            tile: i32::from(b.tile.0) + i32::from(b.tile.1),
            priority_z: depth::static_priority_z(solid.min.z.round() as i8, &cube_tile),
        }
        .to_depth(base_tile);
        let mesh = box_mesh(solid);
        for face in mesh.faces() {
            let id = rows.len() as u32;
            rows.push(MeshFaceRow {
                tile: (b.tile.0, b.tile.1),
                stance: Stance::of_normal(face.normal).expect("a box face's own axis-aligned normal"),
            });
            for corner in face.fan() {
                let screen = camera.to_view_exact(project_exact(corner));
                vertices.push(MeshFaceVertex {
                    screen,
                    world: [corner.x as f32, corner.y as f32, corner.z as f32],
                    depth: d,
                    id,
                    tile: [f32::from(b.tile.0), f32::from(b.tile.1)],
                });
            }
        }
    }
    eprintln!("{} mesh faces, {} vertices", rows.len(), vertices.len());

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let base = env_opt("OPENSHARD_FRAME_DUMP").unwrap_or_else(|| "boxes".to_string());

    let world = openshard_client_render::blit::world_texture(&device, width, height_px);
    let world_view = world.create_view(&wgpu::TextureViewDescriptor::default());
    let place_tex = openshard_client_render::place::texture(&device, width, height_px);
    let place_view = place_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_tex = renderer::depth_texture(&device, width, height_px);
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    // A floor for a shadow to fall on — `two_cubes.rs`'s own hand-built land
    // tile, repeated over the same bounds the occlusion grid covers.
    const FLOOR_GRAPHIC: Graphic = Graphic(3);
    let floor_pixel = openshard_uofiles::color::Color16((20 << 10) | (20 << 5) | 20);
    let floor_image = openshard_uofiles::image::Image::new(
        openshard_uofiles::art::LAND_TILE_SIZE,
        openshard_uofiles::art::LAND_TILE_SIZE,
        vec![floor_pixel; usize::from(openshard_uofiles::art::LAND_TILE_SIZE).pow(2)],
    );
    let blocks = (bounds.max_x as u32).div_ceil(openshard_uofiles::map::BLOCK_SIZE) + 1;
    let synthetic_map =
        openshard_uofiles::map::Map::from_blocks(blocks, blocks, |_x, _y| openshard_uofiles::map::LandCell {
            tile: FLOOR_GRAPHIC.0,
            z: 0,
        });
    let land = LandAtlas::pack([(FLOOR_GRAPHIC, floor_image)]).expect("one flat tile always fits");
    let texmaps = TexmapAtlas::pack([]).expect("nothing always fits");
    let ground_quads =
        openshard_client_render::ground::collect(&synthetic_map, &camera, &land, &texmaps, &Cutaway::OPEN);
    eprintln!("{} ground quads", ground_quads.len());

    let mut ground_pass = GroundRenderer::new(&device, &queue, format, &land, &texmaps);
    let mut mesh_pass = MeshFaceRenderer::new(&device);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = Target {
        place: &place_view,
        view: &world_view,
        depth: &depth_view,
        width,
        height: height_px,
        projection: camera.projection(),
    };
    ground_pass.render(&device, &queue, &mut encoder, target, &ground_quads);
    mesh_pass.render(&device, &queue, &mut encoder, target, &vertices, &rows);
    queue.submit([encoder.finish()]);

    let first = &boxes[0];
    // Picked by looking at a rendered frame, the same way the zoom default
    // above was: a radius wide enough to light both boxes and their own
    // shadow without pulling the whole 512×512 canvas into the torch's
    // reach, which is a fact about a torch at night and not a bug (`NIGHT`
    // ambient below is deliberate — see `light.rs`'s own doc).
    let (default_ldx, default_ldy, default_z, default_radius) = match scene_name.as_str() {
        "tree" => (1.5, -1.0, "6", "6"),
        _ => (2.5, -1.5, "6", "8"),
    };
    let (ldx, ldy) = env_opt("OPENSHARD_LIGHT_AT")
        .map(|s| parse_pair_f32(&s))
        .unwrap_or((default_ldx, default_ldy));
    let light_z: f32 = env_or("OPENSHARD_LIGHT_Z", default_z).parse().expect("a number");
    let light_radius: f32 = env_or("OPENSHARD_LIGHT_RADIUS", default_radius)
        .parse()
        .expect("a number");
    eprintln!("light: at ({ldx:+}, {ldy:+}) of box 0's own tile, z {light_z}, radius {light_radius}");
    let mut lighting = Lighting {
        ambient: NIGHT,
        lights: vec![Light {
            at: Vec2::new(f32::from(first.tile.0) + ldx, f32::from(first.tile.1) + ldy),
            z: light_z,
            radius: light_radius,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            beam: None,
        }],
        occlusion,
        sun: None,
        view: View::Lit,
    };
    let projection = camera.projection();
    let light_screen = camera.to_view_exact(project_exact(WorldSpot {
        x: f64::from(first.tile.0) + f64::from(ldx),
        y: f64::from(first.tile.1) + f64::from(ldy),
        z: f64::from(light_z),
    }));
    let light_pixel = (
        (light_screen.x - projection.origin.x) * projection.scale + width as f32 * 0.5,
        (light_screen.y - projection.origin.y) * projection.scale + height_px as f32 * 0.5,
    );
    let light_mark = (light_pixel.0.round() as i32, light_pixel.1.round() as i32);
    eprintln!("light pixel: {light_mark:?}");

    // The oracle: for each box's own top, an independent (`oracle_visible`,
    // no shared code with `light::sample`/`walk_cells`/`blit.wgsl`) answer
    // for whether the light can see each point of it at all, laid next to
    // what the engine's own CPU walk (`light::sample`, the same arithmetic
    // `blit.wgsl` runs, held to it by a parity test) says — a disagreement
    // here is a real defect, not a rendering nuance, because nothing about
    // either side depends on how a pixel projects to the screen.
    if env_opt("OPENSHARD_BOXES_ORACLE").as_deref() != Some("0") {
        let light_at = (
            f64::from(first.tile.0) + f64::from(ldx),
            f64::from(first.tile.1) + f64::from(ldy),
            f64::from(light_z),
        );
        for (index, b) in boxes.iter().enumerate() {
            let side = 96u32;
            let z = b.max.2;
            let oracle =
                raster_top_down(
                    side,
                    (b.min.0, b.min.1),
                    (b.max.0, b.max.1),
                    |x, y| match oracle_visible((x, y, z), light_at, &boxes, index) {
                        true => 1.0,
                        false => 0.0,
                    },
                );
            let engine = raster_top_down(side, (b.min.0, b.min.1), (b.max.0, b.max.1), |x, y| {
                let spot = light::Spot {
                    at: Vec2::new(x as f32, y as f32),
                    z: z as f32,
                    tile: (i32::from(b.tile.0), i32::from(b.tile.1)),
                    surface: light::Surface::Flat,
                };
                let sampler = match env_opt("OPENSHARD_BOXES_ORACLE_EXACT").as_deref() {
                    Some("1") => light::sample_exact,
                    _ => light::sample,
                };
                sampler(spot, &lighting)
                    .reaches
                    .first()
                    .map_or(0.0, |reach| if reach.within { reach.through } else { 0.0 })
            });
            let mut mismatches = 0usize;
            for i in 0..(side * side) as usize {
                let (o, e) = (oracle[i * 3], engine[i * 3]);
                let says_lit = o > 200;
                let reads_lit = e > 128;
                if says_lit != reads_lit {
                    mismatches += 1;
                }
            }
            eprintln!(
                "oracle vs engine, box {index}'s own top: {mismatches}/{} pixels disagree on lit-or-not",
                side * side
            );
            write_oracle_comparison(
                std::path::Path::new(&format!("{base}_oracle_box{index}.ppm")),
                side,
                &oracle,
                &engine,
            );
        }
    }

    let dummy_instances = openshard_client_render::blit::dummy_instances(&device);
    let mut blit = openshard_client_render::blit::Blit::new(&device, format);
    let mut shadow_pixels: Vec<u8> = Vec::new();
    for view in [View::Lit, View::Shadow] {
        lighting.view = view;
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
        blit.render(
            &device,
            &queue,
            &mut encoder,
            BlitFrame {
                target: &surface_view,
                world: &world_view,
                place: &place_view,
                face_instances: &dummy_instances,
                mobile_instances: &dummy_instances,
                mesh_instances: mesh_pass.rows_buffer(),
                ground_instances: ground_pass.instances_buffer(),
                zoom: Zoom::ONE,
                rect: openshard_client_render::blit::ViewportRect {
                    x: 0,
                    y: 0,
                    width,
                    height: height_px,
                },
            },
            &lighting,
        );
        queue.submit([encoder.finish()]);
        let pixels = dump(
            &device,
            &queue,
            &surface,
            width,
            height_px,
            &PathBuf::from(format!("{base}_{}.ppm", view.name())),
            Some(light_mark),
        );
        if view == View::Shadow {
            shadow_pixels = pixels;
        }
    }

    // A ground-plane companion to the box-top oracle above: that one only ever
    // asks about the tops of the boxes, so it structurally cannot see the bug
    // `docs/lighting_raymarch.md`'s backlog names ("A live CPU/GPU disagreement
    // on `boxes.rs`'s `tree` scene") — the ground immediately beside a box's own
    // base. This sweeps a top-down grid of *ground* points instead, projects
    // each through the scene's own isometric camera (not an ortho view, so it
    // reads the exact pixel the renderer itself drew), and compares the
    // rendered `View::Shadow` pixel there against `segment_clear_of_box`, the
    // same independent, no-shared-arithmetic oracle the box-top check already
    // trusts. A point inside any box's own footprint is skipped — the pixel the
    // camera finds there is the box's mesh, not the ground underneath it.
    type Mismatch = (f64, f64, f32, (u8, u8, u8));
    if env_opt("OPENSHARD_BOXES_GROUND_ORACLE").as_deref() != Some("0") {
        let light_at = (
            f64::from(first.tile.0) + f64::from(ldx),
            f64::from(first.tile.1) + f64::from(ldy),
            f64::from(light_z),
        );
        let margin = 1.5;
        let min_x = boxes.iter().map(|b| b.min.0).fold(f64::MAX, f64::min) - margin;
        let max_x = boxes.iter().map(|b| b.max.0).fold(f64::MIN, f64::max) + margin;
        let min_y = boxes.iter().map(|b| b.min.1).fold(f64::MAX, f64::min) - margin;
        let max_y = boxes.iter().map(|b| b.max.1).fold(f64::MIN, f64::max) + margin;
        let side = 240u32;
        let mut mismatches = 0usize;
        let mut too_dark = 0usize;
        let mut too_light = 0usize;
        let mut examples_too_dark: Vec<Mismatch> = Vec::new();
        let mut examples_too_light: Vec<Mismatch> = Vec::new();
        for row in 0..side {
            for col in 0..side {
                let u = (col as f64 + 0.5) / f64::from(side);
                let v = (row as f64 + 0.5) / f64::from(side);
                let x = min_x + u * (max_x - min_x);
                let y = min_y + v * (max_y - min_y);
                if boxes
                    .iter()
                    .any(|b| x >= b.min.0 && x <= b.max.0 && y >= b.min.1 && y <= b.max.1)
                {
                    continue;
                }
                let screen = camera.to_view_exact(project_exact(WorldSpot { x, y, z: 0.0 }));
                let px = (screen.x - projection.origin.x) * projection.scale + width as f32 * 0.5;
                let py = (screen.y - projection.origin.y) * projection.scale + height_px as f32 * 0.5;
                if px < 0.0 || py < 0.0 || px >= width as f32 || py >= height_px as f32 {
                    continue;
                }
                let (pxi, pyi) = (px.round() as u32, py.round() as u32);
                // `round`, not the `< width`/`< height_px` check just above, is
                // what can land exactly on the far edge — a coordinate at
                // `width - 0.3` passes the check and then rounds up to `width`.
                if pxi >= width || pyi >= height_px {
                    continue;
                }
                let offset = ((pyi * width + pxi) * 4) as usize;
                let gpu_lit = shadow_pixels[offset] > 128;
                // The GPU never lights the *continuous* `(x, y)` — `ground.wgsl`'s
                // `place_of` quantises the fragment's own tile-local fraction to
                // `SUB_TILE = 127` levels before `blit.wgsl` ever reads it back
                // (`docs/gbuffer.md` step 7's own g-buffer format). Comparing the
                // rendered picture against a query point that skips that
                // quantisation compares it to a fragment the rasteriser could
                // never actually produce — so the independent oracle and
                // `light::sample` are both asked about the *reconstructed* point
                // instead, the same lossy value the shader itself lights.
                let quantise = |v: f64, tile: f64| tile + (((v - tile) * 127.0).round() / 127.0);
                let (qx, qy) = (quantise(x, x.floor()), quantise(y, y.floor()));
                let independent = oracle_visible((qx, qy, 0.0), light_at, &boxes, usize::MAX);
                if independent != gpu_lit {
                    mismatches += 1;
                    let spot = light::Spot {
                        at: Vec2::new(qx as f32, qy as f32),
                        z: 0.0,
                        tile: (qx.floor() as i32, qy.floor() as i32),
                        surface: light::Surface::Flat,
                    };
                    let through = light::sample(spot, &lighting)
                        .reaches
                        .first()
                        .map_or(0.0, |reach| if reach.within { reach.through } else { 0.0 });
                    let rgb = (
                        shadow_pixels[offset],
                        shadow_pixels[offset + 1],
                        shadow_pixels[offset + 2],
                    );
                    if gpu_lit {
                        // Oracle says shadowed, picture says lit.
                        too_light += 1;
                        if examples_too_light.len() < 8 {
                            examples_too_light.push((qx, qy, through, rgb));
                        }
                    } else {
                        // Oracle says lit, picture says shadowed.
                        too_dark += 1;
                        if examples_too_dark.len() < 8 {
                            examples_too_dark.push((qx, qy, through, rgb));
                        }
                    }
                }
            }
        }
        eprintln!(
            "ground oracle vs rendered View::Shadow: {mismatches}/{} sampled ground points disagree \
             ({too_dark} rendered too dark, {too_light} rendered too light)",
            side * side
        );
        for (label, examples) in [
            ("too dark", &examples_too_dark),
            ("too light", &examples_too_light),
        ] {
            for (x, y, cpu_through, rgb) in examples {
                eprintln!(
                    "  [{label}] ({x:.3}, {y:.3}): light::sample through={cpu_through:.3}, pixel rgb={rgb:?}",
                );
            }
        }
    }
}
