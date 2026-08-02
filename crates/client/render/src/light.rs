//! Firelight: the pools of warm light a torch, a brazier or a campfire lays on
//! the ground around it.
//!
//! # In the world's own units, not the screen's
//!
//! A light is a tile, a height and a reach in tiles; a fragment is lit according
//! to the tile *its own picture* came from, which the world passes wrote into
//! [`crate::place`]. The screen never enters it. It cannot: the screen folds
//! height into `y`, so a brazier in a cellar lands a few pixels from a lantern
//! on the street above, and a wall's picture stands 44 pixels above the tile it
//! occludes from — which puts the lit face of a wall inside its own shadow the
//! moment shadows exist at all. `docs/lighting.md` is the argument at length.
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
use crate::occlusion::Occlusion;

/// One point light, where it stands in the world.
///
/// Tile coordinates and a `z`, not pixels: what a fragment is lit by depends on
/// the tile *it* came from — see [`crate::place`] — and a pool measured on the
/// screen would be a circle drawn over a projection that folds height into `y`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Light {
    /// The tile it burns on, `x` and `y`.
    ///
    /// Floats because the shader compares them against a fragment's tile and
    /// there is nothing to be gained by converting twice; every value here came
    /// from a `u16` and is exact.
    pub at: Vec2,
    /// Its height, in the map's own `z` units.
    pub z: f32,
    /// How far its pool reaches, **in tiles**. Nothing beyond this is touched at
    /// all, which is what keeps the shader's loop cheap and the pool a shape
    /// rather than a global tint.
    pub radius: f32,
    /// Its colour, linear, each channel in `0..=1`.
    pub color: [f32; 3],
    /// How brightly it burns at its centre, flicker already folded in. Above
    /// `1.0` is ordinary: a fire blows out the ground it stands on.
    pub intensity: f32,
}

/// The sun: one direction for the whole world, and what it does where nothing
/// stands in the way.
///
/// Not a sixty-fifth flame. A flame is a point and the walk to it is bounded by
/// its radius; the sun has no position, so every fragment walks the *same*
/// direction until the ray leaves the grid or is stopped — which is what gives a
/// wall a shadow lying across the street, and a window a bright patch on the
/// floor behind it. `docs/lighting.md`, decision 12.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sun {
    /// Which way the sun is, from anywhere: `x` and `y` in tiles and `z` in
    /// tiles as well — the same unit the distance to a flame is in, so that an
    /// elevation of 45° really is one tile up per tile along. Normalised by
    /// [`Sun::towards`], which is the only thing that builds one.
    pub toward: [f32; 3],
    /// Its colour, linear.
    pub color: [f32; 3],
    /// How much it adds where it reaches. Zero is "no sun", and the blit skips
    /// the walk entirely for it — which is what keeps a frame that has no sun
    /// exactly as cheap as it was before there was one.
    pub intensity: f32,
}

impl Sun {
    /// A sun `rise` tiles up for every tile along `(dx, dy)`.
    ///
    /// The elevation is stated as a slope rather than as an angle because that is
    /// what the walk uses and because a slope is the thing with a picture: `1.0`
    /// is 45°, and a wall twenty units tall — two tiles' worth of `z` — throws
    /// its shadow two tiles.
    ///
    /// A `(dx, dy)` of nothing at all is taken as straight down the `y` axis
    /// rather than left to produce a direction of zero length: a sun with no
    /// azimuth is overhead, and overhead in this projection is a degenerate case
    /// that would silently make every fragment sunlit.
    pub fn towards(dx: f32, dy: f32, rise: f32, color: [f32; 3], intensity: f32) -> Self {
        let (dx, dy) = match dx.abs() + dy.abs() < 1e-4 {
            true => (0.0, -1.0),
            false => (dx, dy),
        };
        let length = (dx * dx + dy * dy + rise * rise).sqrt();
        Self {
            toward: [dx / length, dy / length, rise / length],
            color,
            intensity,
        }
    }

    /// How steeply it climbs per tile along the ground: the slope
    /// [`Sun::towards`] was given back, whatever the direction was normalised to.
    pub fn rise_per_tile(self) -> f32 {
        let horizontal = (self.toward[0] * self.toward[0] + self.toward[1] * self.toward[1]).sqrt();
        match horizontal < 1e-6 {
            true => f32::INFINITY,
            false => self.toward[2] / horizontal,
        }
    }
}

/// How many tiles of grid one sunbeam's ray may walk.
///
/// The bound the ray needs and a flame's does not — see [`Sun`]. Thirty-two
/// tiles is a shadow long enough for a low sun over a city block, and it is
/// almost never reached: the walk stops as soon as the ray is above everything
/// in the grid, which for a street of one-storey buildings is two or three
/// steps. `blit.wgsl`'s `MAX_SUN_STEPS`, and the two are one number.
pub const MAX_SUN_STEPS: i32 = 32;

/// How many `z` units make one tile's width.
///
/// `TILE_WIDTH / Z_STEP`: a tile is 44 virtual pixels across and one unit of
/// height lifts a sprite four, so eleven units of `z` are one tile of ground.
/// It is what lets a distance have all three axes in one unit, and with it a
/// flame reaches as far up and down as it does sideways — which is what stops a
/// cellar's brazier from lighting the street even where nothing occludes.
pub const Z_PER_TILE: f32 = (crate::camera::TILE_WIDTH / crate::camera::Z_STEP) as f32;

/// Everything the blit needs to light a frame.
///
/// [`Lighting::NONE`] is the identity — full ambient, no lights — and the blit
/// multiplies by exactly `1.0` for it, so a frame test comparing the surface
/// with the world image texel for texel still holds.
#[derive(Clone, PartialEq, Debug)]
pub struct Lighting {
    /// What everything is multiplied by away from any flame — the daylight, or
    /// the lack of it. `[1.0; 3]` is "no lighting at all".
    pub ambient: [f32; 3],
    /// The flames themselves, nearest first and never more than
    /// [`Lighting::MAX`] of them.
    pub lights: Vec<Light>,
    /// What stands between them and the ground — see [`crate::occlusion`].
    ///
    /// Travels with the lights rather than beside them because it is the same
    /// frame's answer built from the same walk: a grid collected for one camera
    /// and used with another's flames would put shadows where the map has no
    /// walls.
    pub occlusion: Occlusion,
    /// The sun, where there is one — see [`Sun`]. `None` is night, or a frame
    /// that has not been given a sky yet, and costs nothing at all: the shader
    /// never walks a ray for it.
    pub sun: Option<Sun>,
    /// Which of the pass's own values to draw instead of the lit frame — see
    /// [`crate::debug::View`], and `docs/lighting.md`'s decision 8 for why the
    /// diagnostics are branches of this pass rather than a second one.
    ///
    /// Here rather than in [`crate::blit::Frame`] because it is read where the
    /// lights are read, out of the same uniform block, and a second channel into
    /// the same shader is a second thing to keep in step.
    pub view: crate::debug::View,
}

impl Lighting {
    /// How many lights one frame may carry.
    ///
    /// A fixed-size uniform array rather than a storage buffer, because the
    /// ceiling this crate draws under is WebGL2 and a storage buffer is not in
    /// it — see the crate docs. Sixty-four is a tavern's worth of candles;
    /// past that [`collect`] keeps the ones nearest the player.
    pub const MAX: usize = 64;

    /// The frame nothing lights: the world image, unchanged.
    pub const NONE: Self = Self {
        ambient: [1.0, 1.0, 1.0],
        lights: Vec::new(),
        occlusion: Occlusion::EMPTY,
        sun: None,
        view: crate::debug::View::Lit,
    };

    /// Whether this would change a single pixel.
    ///
    /// The occluders are not asked about: a wall with no flame to stop casts
    /// nothing. A debug view is never the identity, however empty the frame's
    /// lighting is — that is the whole of what it draws.
    pub fn is_identity(&self) -> bool {
        self.lights.is_empty() && self.ambient == [1.0, 1.0, 1.0] && self.view.is_lit()
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

/// What a daylit world is lit by *away from the sun*: the sky.
///
/// Well short of white, because with a sun in the frame the sun supplies the
/// rest — an ambient that already lit everything would leave every shadow the
/// sun casts invisible. And well short of black, because a shadow at noon is not
/// a hole: the reference isometrics draw one lit by the sky, and so does this.
pub const SKYLIGHT: [f32; 3] = [0.55, 0.55, 0.62];

/// The sun this client stands under until there is a time of day on the wire.
///
/// Towards `+x` and one tile up for every tile along — 45°, so a wall twenty
/// units tall throws a shadow two tiles long. Both numbers are placeholders in
/// exactly the way [`flame`] is: what a shard's sky is doing is the shard's to
/// say, and when it does, this is the function that goes and no call site
/// changes.
pub fn midday() -> Sun {
    Sun::towards(1.0, 0.0, 1.0, [1.0, 0.97, 0.88], 0.55)
}

/// How one kind of flame burns, before the flicker.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Flame {
    /// The pool's reach, in tiles. The world's own unit: what it lights is a
    /// span of ground, and no zoom changes how much ground that is.
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
    radius: 6.0,
    color: [1.0, 0.72, 0.36],
    intensity: 0.95,
    flicker: 0.10,
};

/// A campfire: wider, brighter, steadier.
const CAMPFIRE: Flame = Flame {
    radius: 9.0,
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

/// How far above its tile a flame burns, in `z` units.
///
/// A torch's flame is at the top of the sprite and the pool is centred under it,
/// not on the ground the sprite stands on. Half a tile up — [`Z_PER_TILE`] over
/// two — which is where the flame of a waist-high brazier is and close enough
/// for a wall sconce; the sprite's real height is not available here, and asking
/// the atlas for it would tie the lights to whether this frame's art happened to
/// be packed.
const FLAME_LIFT: f32 = Z_PER_TILE / 2.0;

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
/// Now that a reach is stated in tiles, the number *is* the widest pool, plus
/// one for the rounding. It is also the margin the occlusion grid is built over:
/// a wall outside it could not shadow anything the frame draws, because no flame
/// inside it reaches that far.
const LIGHT_MARGIN_TILES: i32 = CAMPFIRE.radius as i32 + 1;

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

/// Every flame a frame can see, flickering, with what stands in their way.
///
/// The statics come from the map and the items from what the server has
/// dropped, which is the same pair [`crate::statics`] and [`crate::items`] draw
/// — and they are tested against the same `cutaway`, so a brazier on the storey
/// above the player stops lighting the floor at the instant it stops being
/// drawn. A light that outlived its sprite is a glow with nothing making it.
///
/// The occluders come from the same walk of the same cells, for the same reason
/// in the other direction: a wall the frame did not draw must not darken the
/// street — see [`crate::occlusion`].
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
    let bounds = lit_tiles(camera);
    let mut lights = Vec::new();

    crate::statics::for_each_static_in(map, bounds, |item| {
        let tile = tiledata.static_tile(item.tile);
        if !tile.flags.is_light_source() || !cutaway::shows(cutaway, item.z, tile) {
            return;
        }
        lights.push(place(
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
        lights.push(place(item.at, item.graphic, time));
    }

    if lights.len() > Lighting::MAX {
        // Nearest the player first — which is the eye's tile, and at every zoom
        // the middle of what is drawn. A total order and not a partial one: two
        // lights at the same distance keep the order the map gave them, so one
        // frame is not a different sixty-four from the next for a camera that
        // has not moved.
        let (eye_x, eye_y) = camera.eye_tile();
        let eye = Vec2::new(eye_x as f32, eye_y as f32);
        lights.sort_by(|a, b| {
            let key = |light: &Light| {
                let (dx, dy) = (light.at.x - eye.x, light.at.y - eye.y);
                dx * dx + dy * dy
            };
            key(a).total_cmp(&key(b))
        });
        lights.truncate(Lighting::MAX);
    }

    Lighting {
        ambient,
        lights,
        occlusion: crate::occlusion::collect(map, items, bounds, tiledata, cutaway),
        // No sky here. What the sun is doing is not a property of the tiles this
        // walked — it is one direction for the whole world, and the caller that
        // knows the time of day sets it on the way to the blit.
        sun: None,
        // The ordinary picture. A caller wanting a diagnostic sets the field on
        // the way to the blit: which view is on is a property of the person
        // looking, not of the world walked here.
        view: crate::debug::View::Lit,
    }
}

/// One flame, from its tile to where it burns: the tile itself, lifted to the
/// height of the flame rather than the ground under it.
fn place(at: Point, graphic: Graphic, time: f32) -> Light {
    let flame = flame(graphic);
    Light {
        // The middle of the tile, not its corner: a fragment's own position is
        // fractional now — the world passes write where in its tile a pixel is —
        // and a flame at `(x, y)` exactly would sit on the tile's north corner
        // and light the tile north of it as brightly as its own.
        at: Vec2::new(f32::from(at.x) + 0.5, f32::from(at.y) + 0.5),
        z: f32::from(at.z) + FLAME_LIFT,
        radius: flame.radius,
        color: flame.color,
        intensity: flame.intensity * flicker(time, phase_of(at), flame.flicker),
    }
}

/// How many cells of the grid one shadow ray may look at.
///
/// `blit.wgsl`'s `MAX_RAY_STEPS`, and the two are one number: [`sample`] is the
/// shader's own arithmetic in Rust and a bound that differed would make the two
/// disagree exactly where a ray is longest. A pool reaches nine tiles at the
/// widest, so this is never actually reached; it exists so that a loop over data
/// cannot be made unbounded by a radius somebody widens later.
pub const MAX_RAY_STEPS: i32 = 16;

/// Below this, a ray has been stopped: `blit.wgsl`'s early exit, and under a
/// byte's worth of light either way.
const RAY_CUTOFF: f32 = 0.004;

/// A point in the world, as the lighting sees one: a fractional tile and a `z`.
///
/// Fractional because that is what the place attachment carries — where in its
/// tile a pixel is, to a hundred-and-twenty-eighth — and a pool is a gradient
/// only because of it. See [`crate::place`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Spot {
    /// Tile coordinates, the fraction being where in the tile the point is.
    pub at: Vec2,
    /// Its height, in the map's own `z` units.
    pub z: f32,
}

/// What one flame did to one spot, and why.
///
/// The *why* is the point: a pool that is missing has one of three causes — the
/// flame is too far, the ray was stopped, or the flame was never collected — and
/// a picture cannot tell the first two apart. This does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Reach {
    /// Which of [`Lighting::lights`] this is, by index.
    pub light: usize,
    /// How far the flame is, in tiles, with `z` divided into tiles — the same
    /// three-dimensional distance the falloff uses.
    pub distance: f32,
    /// Whether that distance is inside the flame's radius. `false` means the
    /// spot is outside the pool and nothing else was computed.
    pub within: bool,
    /// How much of the flame survived the walk: `1.0` for an open path, `0.0` for
    /// a wall, and between for a partial occluder. Only meaningful when
    /// [`Reach::within`].
    pub through: f32,
    /// The tile that stopped the ray, where one did.
    ///
    /// The *first* cell that took the survival to zero, which is the one worth
    /// naming: a ray crossing two walls is stopped by the first of them and the
    /// second is a fact about the map, not about this pixel.
    pub stopped_by: Option<(i32, i32)>,
    /// What this flame added to the multiplier, linear, per channel.
    pub added: [f32; 3],
}

/// Everything one point of the world receives, and from what.
///
/// [`sample`] is the CPU's copy of `blit.wgsl`'s fragment loop, and the copy
/// exists for two reasons: a test can assert on numbers instead of on pixels,
/// and the client can answer "why is this tile lit" in words. Both are worthless
/// if the copy drifts, so a GPU test runs the real blit over a synthetic place
/// attachment and asserts the two agree — see `docs/lighting.md`, decision 9.
#[derive(Clone, PartialEq, Debug)]
pub struct Sample {
    /// Where this was asked about.
    pub spot: Spot,
    /// What the art at this spot is multiplied by: the ambient plus every
    /// flame's contribution, unclamped. The shader clamps at the end; this does
    /// not, because a value over one is a real answer — it says the spot is
    /// blown out rather than merely lit.
    pub multiplier: [f32; 3],
    /// One entry per flame the frame carried, in the order [`Lighting::lights`]
    /// holds them — including the ones that reached nothing, which is exactly
    /// what a person asking "why is it dark here" needs to see.
    pub reaches: Vec<Reach>,
    /// How much of the sun reached this spot, and what stopped it — `None` where
    /// the frame had no sun at all, which is a different answer from `0.0`.
    pub sun: Option<Reach>,
}

impl Sample {
    /// How bright this spot came out, as one number: the mean of the channels.
    ///
    /// For a diagram and for a test that wants "brighter than" rather than a
    /// colour. Deliberately not luma-weighted — this is not a picture, and a
    /// weighting would make a blue ambient and a warm flame incomparable.
    pub fn brightness(&self) -> f32 {
        self.multiplier.iter().sum::<f32>() / 3.0
    }
}

impl std::fmt::Display for Sample {
    /// The report: the spot, what it came out at, and a line per flame saying
    /// what happened to it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "({:.2}, {:.2}, z {:.1}) -> {:.3} [{:.3} {:.3} {:.3}]",
            self.spot.at.x,
            self.spot.at.y,
            self.spot.z,
            self.brightness(),
            self.multiplier[0],
            self.multiplier[1],
            self.multiplier[2],
        )?;
        for reach in &self.reaches {
            write!(f, "  light {}: {:.2} tiles", reach.light, reach.distance)?;
            match (reach.within, reach.stopped_by) {
                (false, _) => writeln!(f, ", outside its radius")?,
                (true, Some((x, y))) => writeln!(f, ", stopped at ({x}, {y})")?,
                (true, None) => writeln!(
                    f,
                    ", through {:.2}, adds {:.3}",
                    reach.through,
                    reach.added.iter().sum::<f32>() / 3.0,
                )?,
            }
        }
        if let Some(sun) = self.sun {
            match sun.stopped_by {
                Some((x, y)) => writeln!(f, "  sun: in shadow of ({x}, {y})")?,
                None => writeln!(
                    f,
                    "  sun: through {:.2}, adds {:.3}",
                    sun.through,
                    sun.added.iter().sum::<f32>() / 3.0,
                )?,
            }
        }
        Ok(())
    }
}

/// What a frame's lighting does to one spot in the world, with the reasons.
///
/// `blit.wgsl`'s fragment loop, arithmetic for arithmetic: the same
/// three-dimensional distance with `z` in tiles, the same `(1 - d)²` falloff, the
/// same walk of the grid between the spot and each flame. The shader's clamp and
/// its multiply by the art are the two things left out, because neither is about
/// the lighting — see [`Sample::multiplier`].
pub fn sample(spot: Spot, lighting: &Lighting) -> Sample {
    let mut multiplier = lighting.ambient;
    let mut reaches = Vec::with_capacity(lighting.lights.len());
    for (index, light) in lighting.lights.iter().enumerate() {
        let offset = [
            light.at.x - spot.at.x,
            light.at.y - spot.at.y,
            (light.z - spot.z) / Z_PER_TILE,
        ];
        let distance = offset.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        let d = distance / light.radius.max(0.001);
        if d >= 1.0 {
            reaches.push(Reach {
                light: index,
                distance,
                within: false,
                through: 0.0,
                stopped_by: None,
                added: [0.0; 3],
            });
            continue;
        }
        let (through, stopped_by) = walk(spot, light, &lighting.occlusion);
        let fall = 1.0 - d;
        let added = light
            .color
            .map(|channel| channel * light.intensity * fall * fall * through);
        for (total, channel) in multiplier.iter_mut().zip(added) {
            *total += channel;
        }
        reaches.push(Reach {
            light: index,
            distance,
            within: true,
            through,
            stopped_by,
            added,
        });
    }
    // And the sun, which is one direction rather than a place and therefore not
    // in the loop above: no distance, no falloff, and a walk with no endpoint.
    let sun = lighting.sun.map(|sun| {
        let (through, stopped_by) = walk_sun(spot, sun, &lighting.occlusion);
        let added = sun.color.map(|channel| channel * sun.intensity * through);
        for (total, channel) in multiplier.iter_mut().zip(added) {
            *total += channel;
        }
        Reach {
            // The sun is not one of `lights`, and the index says so by being
            // past the end of it rather than by being a zero somebody might read
            // as "the first flame".
            light: lighting.lights.len(),
            distance: f32::INFINITY,
            within: true,
            through,
            stopped_by,
            added,
        }
    });

    Sample {
        spot,
        multiplier,
        reaches,
        sun,
    }
}

/// The sun's ray from a spot: how much of it arrives, and what stopped it.
///
/// `blit.wgsl`'s `sunlight`, and the same three differences from a flame's walk:
/// there is no endpoint, so the ray is bounded by [`MAX_SUN_STEPS`]; the step is
/// one tile along the ground, so the height climbs by the sun's slope each time;
/// and the walk stops as soon as the ray is above everything the grid holds,
/// which is what makes a daylit frame affordable at all.
///
/// The spot's own tile is skipped, as it is for a flame, and for the same reason
/// in reverse: a wall's own pixels are on a tile that stops light, and a wall
/// that shadowed itself would be black on the side the sun is on.
fn walk_sun(spot: Spot, sun: Sun, occlusion: &Occlusion) -> (f32, Option<(i32, i32)>) {
    let horizontal = (sun.toward[0] * sun.toward[0] + sun.toward[1] * sun.toward[1]).sqrt();
    if horizontal < 1e-6 {
        // Straight overhead: there is no direction to walk along the ground, and
        // the only thing that could shadow the spot is on its own tile — which is
        // exempt. Nothing stops it.
        return (1.0, None);
    }
    let step = [
        sun.toward[0] / horizontal,
        sun.toward[1] / horizontal,
        sun.toward[2] / horizontal * Z_PER_TILE,
    ];
    let ceiling = occlusion.tallest();
    let mut through = 1.0;
    for tile in 1..=MAX_SUN_STEPS {
        let along = tile as f32;
        let at = [
            spot.at.x + step[0] * along,
            spot.at.y + step[1] * along,
            spot.z + step[2] * along,
        ];
        match ceiling {
            // Above everything that could stop it: the rest of the ray is sky.
            Some(top) if at[2] > top as f32 => break,
            // Nothing in the grid stops anything, so neither does the walk.
            None => break,
            _ => {}
        }
        let (x, y) = (at[0].floor() as i32, at[1].floor() as i32);
        let Some(cell) = occlusion.at(x, y) else {
            continue;
        };
        if at[2] < cell.bottom as f32 || at[2] > cell.top as f32 {
            continue;
        }
        through *= 1.0 - f32::from(cell.opacity) / 255.0;
        if through <= RAY_CUTOFF {
            return (0.0, Some((x, y)));
        }
    }
    (through, None)
}

/// The ray from a spot to a flame, cell by cell: how much survives, and what
/// stopped it.
///
/// `blit.wgsl`'s `reaches`, including what it leaves out. Both ends of the walk
/// are skipped — the flame's own tile must not shadow it, because a sconce
/// stands *on* a wall, and the tile being lit must not shadow itself, which is
/// what keeps a wall's own face the brightest thing beside a torch. The height is
/// interpolated along the ray, so a cell stops the light only where the ray
/// passes through the span it occupies.
fn walk(spot: Spot, light: &Light, occlusion: &Occlusion) -> (f32, Option<(i32, i32)>) {
    let delta = [light.at.x - spot.at.x, light.at.y - spot.at.y, light.z - spot.z];
    // Chebyshev, as the game measures: one step is one tile on the longer axis.
    // Truncating rather than rounding, because that is what the shader's `i32()`
    // does to a float.
    let steps = (delta[0].abs().max(delta[1].abs()) as i32).min(MAX_RAY_STEPS);
    let mut through = 1.0;
    for step in 1..steps {
        let t = step as f32 / steps as f32;
        let at = [
            spot.at.x + delta[0] * t,
            spot.at.y + delta[1] * t,
            spot.z + delta[2] * t,
        ];
        let (x, y) = (at[0].floor() as i32, at[1].floor() as i32);
        let Some(cell) = occlusion.at(x, y) else {
            continue;
        };
        if at[2] < cell.bottom as f32 || at[2] > cell.top as f32 {
            continue;
        }
        through *= 1.0 - f32::from(cell.opacity) / 255.0;
        if through <= RAY_CUTOFF {
            return (0.0, Some((x, y)));
        }
    }
    (through, None)
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
        assert_eq!(
            (light.at.x, light.at.y),
            (100.5, 100.5),
            "the middle of its own tile"
        );
        assert_eq!(light.z, FLAME_LIFT, "burning above the ground it stands on");
        assert_eq!(light.radius, TORCH.radius, "six tiles, whatever the zoom");
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

    /// A pool covers the same ground at every zoom, and now says so by not
    /// changing at all.
    ///
    /// The bug this was written against — a torch lighting six tiles at 1:1 and
    /// one and a half at 4x — is unexpressible once a reach is in tiles rather
    /// than in pixels of an image whose scale is the zoom. It stays because
    /// "unexpressible" is a claim about the code and this is the thing that
    /// checks it: `collect` walks a camera, and a camera is what used to be
    /// folded into the number.
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
            assert_eq!(lighting.lights[0].radius, TORCH.radius, "at {zoom}");
            assert_eq!(lighting.lights[0].at, Vec2::new(100.5, 100.5), "at {zoom}");
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
            let drawn = camera.visible_tiles();

            let mut reaching = 0;
            for x in drawn.min_x - 40..=drawn.max_x + 40 {
                for y in drawn.min_y - 40..=drawn.max_y + 40 {
                    // Could a campfire on this tile light any tile the frame
                    // draws? In tiles now, which is the unit the reach is in —
                    // the nearest drawn tile is the one to ask about.
                    let near_x = x.clamp(drawn.min_x, drawn.max_x);
                    let near_y = y.clamp(drawn.min_y, drawn.max_y);
                    let (dx, dy) = ((x - near_x) as f32, (y - near_y) as f32);
                    if (dx * dx + dy * dy).sqrt() >= CAMPFIRE.radius {
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

    /// The occluders come back over the same cells the flames were looked for
    /// on, and a wall on one of them is in the grid.
    ///
    /// One rectangle and not two: a grid collected over a smaller region than
    /// the flames were would let a torch light through a wall that is on screen,
    /// and the two walks are written as one call for exactly that reason.
    #[test]
    fn the_occluders_cover_the_cells_the_flames_came_from() {
        let camera = Camera::new(Point::new(100, 100, 0), 800, 600);
        let graphic = Graphic(0x0006);
        let mut tiledata = TileData::empty();
        tiledata.set_static_tile(
            graphic.0,
            StaticTile {
                flags: TileFlags::new(TileFlags::NO_SHOOT),
                height: 20,
                ..StaticTile::default()
            },
        );
        let items = [GroundItem {
            at: Point::new(101, 100, 0),
            graphic,
            hue: Hue::NONE,
        }];
        let lighting = collect(&bare(), &items, &camera, &tiledata, &Cutaway::OPEN, NIGHT, 0.0);
        assert_eq!(lighting.occlusion.bounds(), lit_tiles(&camera));
        assert!(
            lighting.occlusion.at(101, 100).is_some(),
            "the wall the frame walked past is not in the grid",
        );
    }

    /// The grid a frame uploads stays small enough to upload every frame.
    ///
    /// It is the one *unconditional* cost this pass added: the lights are
    /// walked from the map either way, but the occluders become a texture that
    /// goes to the GPU on every frame whether anything burns or not. Four bytes
    /// a tile over the widest zoom's rectangle, and the number is asserted
    /// rather than assumed because it is the whole of the answer to "does this
    /// cost anything" — a rectangle that grew with the map instead of with the
    /// viewport would be megabytes and nobody would notice until a shard with a
    /// big facet ran it. Measured: 187x187 tiles at the widest zoom on a
    /// 1920x1080 viewport, which is 140KB a frame.
    #[test]
    fn the_grid_a_frame_uploads_is_a_few_tiles_across_and_not_a_map() {
        let mut camera = Camera::new(Point::new(500, 500, 0), 1920, 1080);
        let mut zoom = camera.zoom();
        while !zoom.is_widest() {
            zoom = zoom.scale_down();
        }
        camera.zoom_about(960, 540, zoom);
        let bounds = lit_tiles(&camera);
        let bytes = bounds.width() * bounds.height() * 4;
        assert!(
            bytes < 512 * 1024,
            "the occlusion grid is {}x{} tiles, {bytes} bytes a frame",
            bounds.width(),
            bounds.height(),
        );
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
