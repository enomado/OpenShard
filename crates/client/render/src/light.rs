//! Firelight: the pools of warm light a torch, a brazier or a campfire lays on
//! the ground around it.
//!
//! # Why it is a pass over the finished image and not a term in three shaders
//!
//! Everything here ends up as a handful of point lights in the *drawn image's*
//! own pixels, applied once by [`crate::blit`] on the way to the surface. The
//! alternative — a light term in `ground.wgsl`, `statics.wgsl` and the mobile
//! pass — is three copies of one formula, three uniform blocks to keep in step,
//! and a frame where a body walking past a fire is lit by a slightly different
//! curve than the flagstone it is standing on. There is nothing a per-object
//! pass would buy: UO's art is flat pictures with no normals, so "lit" means
//! exactly *brighter near the flame*, and where a pixel is on the screen is the
//! whole of what that needs.
//!
//! # What a light is, and what says so
//!
//! [`TileFlags::LIGHT_SOURCE`] — the client's own answer. A graphic burns
//! because `tiledata.mul` says it burns, not because this file holds a list of
//! torch graphics, which would be a list somebody has to maintain against every
//! art patch and would silently miss a shard's custom brazier.
//!
//! What the flag does *not* carry is how big the pool is or what colour it
//! burns: the client reads those from `light.mul`, keyed by a light id this
//! workspace's `uofiles` does not parse yet. Until it does, [`flame`] picks a
//! shape from the graphic — one warm default, and a wider, brighter one for a
//! campfire. That is a deliberate stand-in and it is the one thing here that is
//! invention rather than port; see `docs/client.md`.
//!
//! # The flicker is on the CPU
//!
//! Two sine terms of incommensurable frequency, per light, sampled once per
//! frame and folded into the intensity that reaches the GPU. On the CPU because
//! a flame's brightness is one number for the whole pool — the shader would
//! recompute it identically for every pixel it touches — and because this crate
//! is not allowed to read a clock, so the time arrives as an argument and there
//! is exactly one place it is used.

use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

use crate::camera::Camera;
use crate::cutaway::{self, Cutaway};
use crate::geometry::Vec2;
use crate::items::GroundItem;

/// One point light, in the pixels of the image the world was drawn into.
///
/// *Not* [`ViewPixel`](crate::camera::ViewPixel): the blit runs after the
/// projection, so this is the view pixel with
/// [`Projection`](crate::camera::Projection) already applied — which is what
/// keeps one radius meaning the same span of ground at every zoom.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Light {
    /// Where the flame is.
    pub at: Vec2,
    /// How far its pool reaches, in the same pixels. Nothing beyond this is
    /// touched at all, which is what keeps the shader's loop cheap and the pool
    /// a shape rather than a global tint.
    pub radius: f32,
    /// Its colour, linear, each channel in `0..=1`.
    pub color: [f32; 3],
    /// How brightly it burns at its centre, flicker already folded in. Above
    /// `1.0` is ordinary: a fire blows out the ground it stands on.
    pub intensity: f32,
}

/// Everything the blit needs to light a frame.
///
/// [`Lighting::NONE`] is the identity — full ambient, no lights — and the blit
/// multiplies by exactly `1.0` for it, so a frame test comparing the surface
/// with the world image texel for texel still holds.
#[derive(Clone, PartialEq, Debug)]
pub struct Lighting {
    /// The size of the image being lit, in its own pixels. The blit turns a
    /// texture coordinate into one of these with it.
    pub image: Vec2,
    /// What everything is multiplied by away from any flame — the daylight, or
    /// the lack of it. `[1.0; 3]` is "no lighting at all".
    pub ambient: [f32; 3],
    /// The flames themselves, nearest first and never more than
    /// [`Lighting::MAX`] of them.
    pub lights: Vec<Light>,
}

impl Lighting {
    /// How many lights one frame may carry.
    ///
    /// A fixed-size uniform array rather than a storage buffer, because the
    /// ceiling this crate draws under is WebGL2 and a storage buffer is not in
    /// it — see the crate docs. Sixty-four is a tavern's worth of candles;
    /// past that [`collect`] keeps the ones nearest the middle of the screen,
    /// which is where the player is.
    pub const MAX: usize = 64;

    /// The frame nothing lights: the world image, unchanged.
    pub const NONE: Self = Self {
        image: Vec2::new(0.0, 0.0),
        ambient: [1.0, 1.0, 1.0],
        lights: Vec::new(),
    };

    /// Whether this would change a single pixel.
    ///
    /// The blit skips the whole uniform upload when it would not.
    pub fn is_identity(&self) -> bool {
        self.lights.is_empty() && self.ambient == [1.0, 1.0, 1.0]
    }
}

/// Night, as the reference isometrics draw it: dark, and *cooler* than the art.
///
/// The blue cast is what makes a fire read as warm — with a grey ambient the
/// pool and the dark are the same hue at two brightnesses, which the eye reads
/// as a spotlight rather than as firelight.
pub const NIGHT: [f32; 3] = [0.30, 0.33, 0.45];

/// Full daylight: the ambient at which lighting is a no-op.
pub const DAY: [f32; 3] = [1.0, 1.0, 1.0];

/// How one kind of flame burns, before the flicker.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Flame {
    /// The pool's reach in *virtual* pixels — the art's own grid, so it is a
    /// number of tiles rather than of screen pixels. 44 is one tile across.
    pub radius: f32,
    /// Its colour, linear.
    pub color: [f32; 3],
    /// Its brightness at the centre, before the flicker multiplies it.
    pub intensity: f32,
    /// How much the flicker swings that brightness, as a fraction of it. A
    /// candle gutters; a bonfire mostly does not.
    pub flicker: f32,
}

/// A torch, a candle, a lantern: the ordinary flame, and what anything flagged
/// as a light source gets unless it is named below.
const TORCH: Flame = Flame {
    // Six tiles. The reference isometrics light a good deal more than the tile
    // the fire is on — a pool a tile wide reads as a bug, not as a torch.
    radius: 6.0 * 44.0,
    color: [1.0, 0.72, 0.36],
    intensity: 0.95,
    flicker: 0.10,
};

/// A campfire: wider, brighter, steadier.
const CAMPFIRE: Flame = Flame {
    radius: 9.0 * 44.0,
    color: [1.0, 0.66, 0.30],
    intensity: 1.25,
    flicker: 0.07,
};

/// The graphics a campfire cycles through.
///
/// `0x0DE3` is the campfire the client draws for a lit camp, and the four after
/// it are the rest of its animation — see `crate::animate`, which is what
/// decides *which* of them is on screen. All five burn the same, so the range
/// is matched rather than the frame.
const CAMPFIRE_GRAPHICS: std::ops::RangeInclusive<u16> = 0x0DE3..=0x0DE7;

/// How a graphic burns.
///
/// The stand-in for `light.mul` described in this module's header: the flag
/// says a graphic is a light and this says what kind, by name where the graphic
/// is one worth naming and by a warm default everywhere else. When `light.mul`
/// is read, this is the function that goes — and its callers do not change.
pub fn flame(graphic: Graphic) -> Flame {
    match CAMPFIRE_GRAPHICS.contains(&graphic.0) {
        true => CAMPFIRE,
        false => TORCH,
    }
}

/// How much of a flame's own height the light sits above the tile it stands on.
///
/// A torch's flame is at the top of the sprite and the pool is centred under
/// it, not on the ground the sprite stands on. Half a tile up, which is where
/// the flame of a waist-high brazier is and close enough for a wall sconce —
/// the sprite's real height is not available here, and asking the atlas for it
/// would tie the lights to whether this frame's art happened to be packed.
const FLAME_LIFT: f32 = 22.0;

/// How many tiles beyond the drawn image a flame can still light it from.
///
/// **A light is not culled by where its sprite is.** [`Camera::visible_tiles`]
/// covers the tiles whose *pictures* can land in the frame, widened by a tile
/// for the sprite's own size — which is exactly the wrong rectangle here,
/// because a pool reaches [`CAMPFIRE`]`.radius` past the thing making it. Walked
/// with the drawing bounds, a lamp's pool vanishes the instant the lamp leaves
/// the screen instead of sliding off it, and every edge of the frame pops as the
/// camera pans. Measured on Britain at the widest zoom: 88 light sources stood
/// in the band this constant adds, all of them reaching into the frame and none
/// of them drawn.
///
/// The number is the widest pool in tiles, and it is derived rather than chosen:
/// one step in `x` or `y` moves half a tile on each screen axis, so a pool of
/// `r` pixels reaches `r / 22` tiles at most, whichever way the tiles run. Plus
/// one for the rounding.
const LIGHT_MARGIN_TILES: i32 = (CAMPFIRE.radius as i32) / (crate::camera::TILE_WIDTH / 2) + 1;

/// The cells a frame's flames can come from: what is drawn, grown by the reach
/// of the widest pool. See [`LIGHT_MARGIN_TILES`].
fn lit_tiles(camera: &Camera) -> crate::camera::TileBounds {
    let bounds = camera.visible_tiles();
    crate::camera::TileBounds {
        min_x: bounds.min_x - LIGHT_MARGIN_TILES,
        max_x: bounds.max_x + LIGHT_MARGIN_TILES,
        min_y: bounds.min_y - LIGHT_MARGIN_TILES,
        max_y: bounds.max_y + LIGHT_MARGIN_TILES,
    }
}

/// Every flame on screen, placed and flickering, ready for the blit.
///
/// The statics come from the map and the items from what the server has
/// dropped, which is the same pair [`crate::statics`] and [`crate::items`] draw
/// — and they are tested against the same `cutaway`, so a brazier on the storey
/// above the player stops lighting the floor at the instant it stops being
/// drawn. A light that outlived its sprite is a glow with nothing making it.
///
/// `time` is how long the client has been running, in seconds; only the flicker
/// reads it. It is an argument because this crate does not own a clock, and the
/// caller passes the same sampled instant every other clock in the frame was
/// advanced by.
pub fn collect(
    map: &Map,
    items: &[GroundItem],
    camera: &Camera,
    tiledata: &TileData,
    cutaway: &Cutaway,
    ambient: [f32; 3],
    time: f32,
) -> Lighting {
    let (image_width, image_height) = camera.image_size();
    let mut lights = Vec::new();

    crate::statics::for_each_static_in(map, lit_tiles(camera), |item| {
        let tile = tiledata.static_tile(item.tile);
        if !tile.flags.is_light_source() || !cutaway::shows(cutaway, item.z, tile) {
            return;
        }
        lights.push(place(
            camera,
            Point::new(item.x, item.y, item.z),
            Graphic(item.tile),
            time,
        ));
    });

    for item in items {
        let tile = tiledata.static_tile(item.graphic.0);
        if !tile.flags.is_light_source() || !cutaway::shows(cutaway, item.at.z, tile) {
            continue;
        }
        lights.push(place(camera, item.at, item.graphic, time));
    }

    // Off-screen flames are dropped *after* placing rather than by tile, because
    // the pool reaches further than the sprite: a fire a tile past the edge still
    // lights the corner of the frame, and culling it would put a hard line down
    // the side of the image as the camera walks.
    let (width, height) = (image_width as f32, image_height as f32);
    lights.retain(|light| {
        light.at.x + light.radius > 0.0
            && light.at.x - light.radius < width
            && light.at.y + light.radius > 0.0
            && light.at.y - light.radius < height
    });

    if lights.len() > Lighting::MAX {
        // Nearest the middle of the image first — the player is there. A total
        // order and not a partial one: two lights at the same distance keep the
        // order the map gave them, so one frame is not a different sixty-four
        // from the next for a camera that has not moved.
        let middle = Vec2::new(width / 2.0, height / 2.0);
        lights.sort_by(|a, b| {
            let key = |light: &Light| {
                let (dx, dy) = (light.at.x - middle.x, light.at.y - middle.y);
                dx * dx + dy * dy
            };
            key(a).total_cmp(&key(b))
        });
        lights.truncate(Lighting::MAX);
    }

    Lighting {
        image: Vec2::new(width, height),
        ambient,
        lights,
    }
}

/// One flame, from its tile to where it burns in the drawn image.
fn place(camera: &Camera, at: Point, graphic: Graphic, time: f32) -> Light {
    let flame = flame(graphic);
    let projection = camera.projection();
    let (width, height) = camera.image_size();
    let view = camera.to_screen(at);
    // The same line every world pass ends on — `Projection`'s own doc comment —
    // so a flame lands on the pixel the sprite that makes it landed on, at every
    // zoom. Written out rather than shared because the shaders have it as two
    // lines of WGSL and there is nothing in Rust to call.
    let image = Vec2::new(
        (view.x as f32 - projection.origin.x) * projection.scale + width as f32 / 2.0,
        (view.y as f32 - FLAME_LIFT - projection.origin.y) * projection.scale + height as f32 / 2.0,
    );
    Light {
        at: image,
        radius: flame.radius * projection.scale,
        color: flame.color,
        intensity: flame.intensity * flicker(time, phase_of(at), flame.flicker),
    }
}

/// A flame's own place in the flicker, so that two torches on one wall do not
/// pulse in step.
///
/// Any spread-out function of the tile would do; this is the ordinary
/// multiply-and-mix, and what matters about it is only that it is deterministic
/// — the same tile flickers the same way in two clients watching one fire.
fn phase_of(at: Point) -> f32 {
    let mixed = u32::from(at.x)
        .wrapping_mul(73_856_093)
        .wrapping_add(u32::from(at.y).wrapping_mul(19_349_663))
        .wrapping_add((at.z as i32 as u32).wrapping_mul(83_492_791));
    // Into `0..2π`, out of the top bits: the low ones of a multiplicative mix
    // are the least stirred.
    (mixed >> 8) as f32 / (1 << 24) as f32 * std::f32::consts::TAU
}

/// The brightness multiplier a flame is at, at `time` seconds.
///
/// Two sines whose frequencies have no common period, so the pattern does not
/// repeat on anything an eye can catch — one sine reads as a pulse, which is
/// what a machine does and not what a fire does. The amplitudes sum to `depth`,
/// so a `depth` of `0.1` swings the brightness by at most a tenth either way
/// and the flame never gutters out.
fn flicker(time: f32, phase: f32, depth: f32) -> f32 {
    let slow = (time * 6.7 + phase).sin();
    let fast = (time * 11.3 + phase * 2.3).sin();
    1.0 + depth * (0.6 * slow + 0.4 * fast)
}

#[cfg(test)]
mod tests {
    use openshard_protocol::wire::Hue;
    use openshard_uofiles::map::LandCell;
    use openshard_uofiles::tiledata::{StaticTile, TileFlags};

    use super::*;

    /// A tiledata table where exactly one graphic burns.
    fn lit(graphic: u16) -> TileData {
        let mut tiledata = TileData::empty();
        tiledata.set_static_tile(
            graphic,
            StaticTile {
                flags: TileFlags::new(TileFlags::LIGHT_SOURCE),
                ..StaticTile::default()
            },
        );
        tiledata
    }

    /// A map with ground and nothing standing on it: the statics in these tests
    /// come from the item list, which is the half a test can build without a
    /// client install.
    fn bare() -> Map {
        Map::from_blocks(1, 1, |_, _| LandCell { tile: 0, z: 0 })
    }

    /// The identity is exactly that: the blit has a case where it must not touch
    /// a single byte, and this is what says so.
    #[test]
    fn the_empty_lighting_is_the_identity() {
        assert!(Lighting::NONE.is_identity());
        assert!(
            !Lighting {
                ambient: NIGHT,
                ..Lighting::NONE
            }
            .is_identity()
        );
    }

    /// A dropped torch lights the tile it is on: the pool's centre is where the
    /// camera puts that tile, lifted to where the flame is rather than left on
    /// the ground.
    #[test]
    fn a_lit_item_makes_a_light_over_its_own_tile() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let graphic = Graphic(0x0A12);
        let items = [GroundItem {
            at: Point::new(100, 100, 0),
            graphic,
            hue: Hue::NONE,
        }];
        let lighting = collect(
            &bare(),
            &items,
            &camera,
            &lit(graphic.0),
            &Cutaway::OPEN,
            NIGHT,
            0.0,
        );
        assert_eq!(lighting.lights.len(), 1);
        let light = lighting.lights[0];
        let middle = camera.to_screen(items[0].at);
        assert_eq!(light.at.x, middle.x as f32);
        assert_eq!(light.at.y, middle.y as f32 - FLAME_LIFT);
        assert_eq!(
            light.radius, TORCH.radius,
            "unmagnified, a virtual pixel is a pixel"
        );
    }

    /// And an item that is not flagged makes none. The flag is the whole test:
    /// a barrel next to a torch must not glow.
    #[test]
    fn an_unflagged_item_makes_no_light() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let items = [GroundItem {
            at: Point::new(100, 100, 0),
            graphic: Graphic(0x0FAE),
            hue: Hue::NONE,
        }];
        let lighting = collect(
            &bare(),
            &items,
            &camera,
            // Flagged, but a *different* graphic.
            &lit(0x0A12),
            &Cutaway::OPEN,
            NIGHT,
            0.0,
        );
        assert!(lighting.lights.is_empty());
    }

    /// Magnifying scales the pool with the world. Without this a torch lights
    /// six tiles at 1:1 and one and a half at 4x — the same fire, a different
    /// amount of ground, which is the bug that hides in "pixels" meaning two
    /// things.
    #[test]
    fn a_pool_covers_the_same_ground_at_every_zoom() {
        let graphic = Graphic(0x0A12);
        let items = [GroundItem {
            at: Point::new(100, 100, 0),
            graphic,
            hue: Hue::NONE,
        }];
        let tiledata = lit(graphic.0);
        let mut camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let mut zoom = camera.zoom();
        loop {
            camera.zoom_about(400, 300, zoom);
            let lighting = collect(&bare(), &items, &camera, &tiledata, &Cutaway::OPEN, NIGHT, 0.0);
            let scale = camera.projection().scale;
            assert_eq!(lighting.lights[0].radius, TORCH.radius * scale, "at {zoom}");
            let next = zoom.scale_up();
            if next == zoom {
                break;
            }
            zoom = next;
        }
    }

    /// The flicker stays inside the band its depth promises, and two flames on
    /// two tiles are not at the same point of it.
    #[test]
    fn the_flicker_is_bounded_and_out_of_step() {
        let phase = phase_of(Point::new(100, 100, 0));
        let other = phase_of(Point::new(101, 100, 0));
        assert!((phase - other).abs() > 0.01, "two tiles flicker together");
        for step in 0..2_000 {
            let time = step as f32 * 0.017;
            let value = flicker(time, phase, 0.1);
            assert!((0.9..=1.1).contains(&value), "{value} at {time}");
        }
    }

    /// Every tile a pool could reach the frame from is walked.
    ///
    /// The bug this is written against, and it is the one a screenshot shows:
    /// walked with the *drawing* bounds, a lamp's light vanished the moment the
    /// lamp itself left the screen, so every edge of the frame popped as the
    /// camera panned — worst at the widest zoom, where a frame holds more edges
    /// of more pools. On Britain, 88 light sources stood in the band that was
    /// being skipped.
    ///
    /// Stated as the implication rather than as a margin in tiles: *if* a flame
    /// placed on a tile would light the image, *then* the walk has to visit that
    /// tile. That is checkable without a map, at every zoom, and it stays true
    /// if a wider flame is added later — which a constant compared against a
    /// constant would not.
    #[test]
    fn every_flame_that_can_reach_the_frame_is_walked() {
        let widest = Graphic(*CAMPFIRE_GRAPHICS.start());
        assert_eq!(flame(widest).radius, CAMPFIRE.radius, "the widest pool moved");
        let mut camera = Camera::new(Point::new(500, 500, 0), 800, 600);
        let mut zoom = camera.zoom();
        while !zoom.is_widest() {
            zoom = zoom.scale_down();
        }
        loop {
            camera.zoom_about(400, 300, zoom);
            let bounds = lit_tiles(&camera);
            let (width, height) = camera.image_size();
            let (width, height) = (width as f32, height as f32);
            let (eye_x, eye_y) = camera.eye_tile();

            let mut reaching = 0;
            for x in eye_x - 140..=eye_x + 140 {
                for y in eye_y - 140..=eye_y + 140 {
                    let light = place(&camera, Point::new(x as u16, y as u16, 0), widest, 0.0);
                    let touches = light.at.x + light.radius > 0.0
                        && light.at.x - light.radius < width
                        && light.at.y + light.radius > 0.0
                        && light.at.y - light.radius < height;
                    if !touches {
                        continue;
                    }
                    reaching += 1;
                    assert!(
                        x >= bounds.min_x && x <= bounds.max_x && y >= bounds.min_y && y <= bounds.max_y,
                        "at {zoom}, a flame on ({x}, {y}) lights the frame and is never walked",
                    );
                }
            }
            // A sweep that found nothing would assert nothing at all, and would
            // stay green for a `lit_tiles` that returned an empty rectangle.
            // Magnified, the sweep's own 281x281 window is the limit rather than
            // the reach — a pool is four times as wide in image pixels at 4x, so
            // most of what could light the frame is outside the tiles swept.
            // Five hundred is a floor that holds at every rung and is still
            // hundreds of assertions; a sweep that found nothing would assert
            // nothing at all and would stay green for a `lit_tiles` that
            // returned an empty rectangle.
            assert!(
                reaching > 500,
                "at {zoom}, only {reaching} tiles could light the frame"
            );

            let next = zoom.scale_up();
            if next == zoom {
                break;
            }
            zoom = next;
        }
    }

    /// A flame the cutaway has taken away takes its light with it: the roof over
    /// the player hides the brazier on it, and a glow with no fire under it is
    /// worse than no glow.
    #[test]
    fn a_hidden_flame_does_not_light() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let graphic = Graphic(0x0A12);
        let items = [GroundItem {
            at: Point::new(100, 100, 40),
            graphic,
            hue: Hue::NONE,
        }];
        let tiledata = lit(graphic.0);
        let lighting = collect(
            &bare(),
            &items,
            &camera,
            &tiledata,
            // Everything at or above z = 20 is cut away.
            &Cutaway {
                max_z: 20,
                ..Cutaway::OPEN
            },
            NIGHT,
            0.0,
        );
        assert!(lighting.lights.is_empty());
    }
}
