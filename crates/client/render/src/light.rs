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

use openshard_protocol::direction::Direction;
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

use crate::camera::Camera;
use crate::cutaway::{self, Cutaway};
use crate::geometry::Vec2;
use crate::items::GroundItem;
use crate::occlusion::{EDGE_ANY, Occlusion};

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
    /// Which way it throws its light, where it throws it one way at all — see
    /// [`Beam`]. `None` is a fire in the open, which lights every direction
    /// equally, and it is what everything on the map is.
    pub beam: Option<Beam>,
}

/// A flame that lights one direction and not the others: a hooded lantern, or a
/// torch held out in front of a face.
///
/// A cone and not a second radius. Everything else about the light is unchanged
/// — the same falloff, the same three-dimensional distance, the same walk of the
/// grid for what stands in the way — and this multiplies the result by how far
/// inside the cone the lit spot is. That ordering is the whole of why a beam is
/// cheap: a fragment outside the radius never asks about the angle, and one
/// outside the cone never walks the ray.
///
/// Both ends of the cone are cosines rather than angles because the test is a
/// dot product: the direction from the flame to the spot against the axis, both
/// unit vectors in the same units the distance is in — `x` and `y` in tiles and
/// `z` in tiles as well, which is [`Z_PER_TILE`]'s doing and is what keeps a
/// beam pointing along the ground from lighting the storey above.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Beam {
    /// Where it points, unit length. Built by [`Beam::towards`], which is the
    /// only thing that makes one — a direction of some other length would make
    /// the dot product below mean nothing.
    pub toward: [f32; 3],
    /// The cosine of its half-angle: the rim of the cone. A spot whose direction
    /// is below this gets nothing at all.
    pub cos_half: f32,
}

/// How far in from the rim of a beam its edge finishes softening, as a share of
/// the way from the rim to the axis.
///
/// A cone with a hard rim reads as a stencil laid over the scene rather than as
/// light — the eye finds the straight edge immediately, the same way it finds
/// the tile boundary a shadow used to end on. A quarter of the way in is enough
/// to lose it and narrow enough that a sixty-degree beam still looks sixty
/// degrees wide. Invented here, like [`FLAME_SPREAD`] and
/// [`crate::occlusion::PANE`]: no client file has a number for the shape of a
/// lantern's shutter. `blit.wgsl`'s `BEAM_EDGE`, and the two are one number.
const BEAM_EDGE: f32 = 0.25;

/// How much of a beamed flame escapes it in every other direction.
///
/// A hand is not a shutter. What makes a carried torch a beam at all is that the
/// arm holds it out in front and the body is behind it, and neither of those
/// stops the flame from being a flame: the ground at the character's feet is lit,
/// and so is the character. A cone with nothing outside it puts the one thing the
/// player is looking at — their own body — in the only black hole in the frame,
/// which is the opposite of what a light in the hand is for.
///
/// A quarter, so that the beam is still obviously a beam: what is in front is
/// four times what is beside, which reads as a direction at a glance. Invented
/// here like [`BEAM_EDGE`], and `blit.wgsl`'s `BEAM_SPILL` is the same number.
pub const BEAM_SPILL: f32 = 0.25;

impl Beam {
    /// A beam of `degrees` across — the *full* angle, the way a lamp is
    /// described — pointing along `(dx, dy)` with `rise` tiles of climb for
    /// every tile along the ground.
    ///
    /// The full angle and not the half is what a person says out loud, and the
    /// halving belongs at the one place the number is turned into a cosine
    /// rather than at every call site.
    ///
    /// A direction of no length at all is taken as north, for the reason
    /// [`Sun::towards`] takes it as south: a zero axis would make every dot
    /// product zero and the cone would silently become a hemisphere.
    pub fn towards(dx: f32, dy: f32, rise: f32, degrees: f32) -> Self {
        let (dx, dy) = match dx.abs() + dy.abs() < 1e-4 {
            true => (0.0, -1.0),
            false => (dx, dy),
        };
        let length = (dx * dx + dy * dy + rise * rise).sqrt();
        Self {
            toward: [dx / length, dy / length, rise / length],
            cos_half: (degrees.to_radians() / 2.0).cos(),
        }
    }

    /// How much of this beam falls on a spot `offset` away from the flame —
    /// `x` and `y` in tiles, `z` in tiles as well, pointing *from* the flame
    /// *to* the spot.
    ///
    /// `blit.wgsl`'s `cone`, arithmetic for arithmetic, and the parity test of
    /// `docs/lighting.md`'s decision 9 is what says so. The smoothstep is
    /// written out rather than called, because WGSL's built-in and a Rust crate's
    /// are two texts that can disagree and this is one polynomial either way.
    ///
    /// Never zero: [`BEAM_SPILL`] is the floor, and a spot at the flame itself
    /// gets the whole of it — there is no direction from a point to itself, and
    /// the tile a lantern is standing on is not the place to start refusing
    /// light.
    pub fn lights(self, offset: [f32; 3]) -> f32 {
        let length = offset.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        if length < 1e-6 {
            return 1.0;
        }
        let along = offset
            .iter()
            .zip(self.toward)
            .map(|(axis, toward)| axis / length * toward)
            .sum::<f32>();
        let inner = self.cos_half + (1.0 - self.cos_half) * BEAM_EDGE;
        let t = ((along - self.cos_half) / (inner - self.cos_half).max(1e-6)).clamp(0.0, 1.0);
        BEAM_SPILL + (1.0 - BEAM_SPILL) * t * t * (3.0 - 2.0 * t)
    }
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

/// How far along the ground one sunbeam may run, in tiles.
///
/// The bound the ray needs and a flame's does not — see [`Sun`]. What ends a
/// sunbeam is the grid's ceiling: a ray that has climbed above everything in the
/// frame is looking at sky, which for a street of one-storey buildings is two or
/// three tiles out. This is what is left for a sun so low that it never climbs
/// out — a shadow thirty-two tiles long is already longer than any frame, and
/// without it a sunset would be a segment with no end. `blit.wgsl`'s
/// `MAX_SUN_TILES`, and the two are one number.
pub const MAX_SUN_TILES: f32 = 32.0;

/// How many `z` units make one tile's width.
///
/// `TILE_WIDTH / Z_STEP`: a tile is 44 virtual pixels across and one unit of
/// height lifts a sprite four, so eleven units of `z` are one tile of ground.
/// It is what lets a distance have all three axes in one unit, and with it a
/// flame reaches as far up and down as it does sideways — which is what stops a
/// cellar's brazier from lighting the street even where nothing occludes.
pub const Z_PER_TILE: f32 = (crate::camera::TILE_WIDTH / crate::camera::Z_STEP) as f32;

/// The light a place has before anything burns in it: the sky's share, and the
/// floor under it.
///
/// `docs/lighting_world.md`, decision 1. One colour for the whole frame lit the
/// inside of a house exactly as brightly as the street outside it, because
/// nothing in the ambient knew what a roof was — a dungeon was dark only because
/// the server had said the whole world was. Split in two:
///
/// ```text
/// ambient(tile) = sky * sky(tile) + ground
/// ```
///
/// `sky(tile)` is [`crate::occlusion::Occlusion::sky_at`]'s byte, and `ground`
/// is the small, cold floor a windowless cellar still gets — so that a room with
/// no torch in it is deep rather than pure black. An unlit black rectangle is not
/// atmosphere, it is a bug report.
///
/// Both terms are colours and not levels: a sky is blue where a cellar's floor
/// light is bluer still, and a term that was one number could only ever say how
/// *much* light a place has and never what kind.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ambient {
    /// What a tile with an open column above it gets, in full.
    pub sky: [f32; 3],
    /// What every tile gets, roof or no roof.
    pub ground: [f32; 3],
}

impl Ambient {
    /// Full daylight under an open column and nothing under a lid: the ambient
    /// at which the blit is a copy of the world image.
    ///
    /// Only the *open* half is the identity, which is the whole of decision 1
    /// arriving in the one constant that used to mean "no lighting at all" —
    /// see [`Lighting::is_identity`], which is why it now asks about the grid.
    pub const DAY: Self = Self {
        sky: [1.0, 1.0, 1.0],
        ground: [0.0, 0.0, 0.0],
    };

    /// The same light with the sky's share folded into the floor: one colour for
    /// every tile, whatever stands over it.
    ///
    /// **The ambient this pass had before the sky field existed**, and the switch
    /// back to it is deliberate rather than a leftover. What a roof does to the
    /// light under it is a whole plan of its own
    /// (`docs/lighting_world.md`), and while the *point* lights are being got
    /// right it is a second thing changing every tile of every picture: a pool
    /// that looks wrong indoors is then two questions, and the field answers the
    /// one nobody asked. Flat is also the honest baseline — it is what a shard
    /// with no time of day and no roofs looks like — so a difference between the
    /// two pictures is the field's whole contribution, which is what a person
    /// turning it on wants to see.
    ///
    /// The sum and not either half: the two terms were split out of one colour and
    /// they still add up to it, so a flattened [`NIGHT`] is exactly the night this
    /// had before the split.
    pub fn flattened(self) -> Self {
        let mut ground = self.ground;
        for (channel, sky) in ground.iter_mut().zip(self.sky) {
            *channel += sky;
        }
        Self {
            sky: [0.0; 3],
            ground,
        }
    }

    /// What a tile is multiplied by, given how much of the sky it can see.
    ///
    /// `blit.wgsl` does this same arithmetic per fragment out of the field
    /// plane, and the two are held together by the parity test of
    /// `docs/lighting.md`'s decision 9.
    pub fn at(self, sky: u8) -> [f32; 3] {
        let share = f32::from(sky) / f32::from(crate::occlusion::SKY_OPEN);
        let mut lit = self.ground;
        for (channel, sky) in lit.iter_mut().zip(self.sky) {
            *channel += sky * share;
        }
        lit
    }
}

/// Everything the blit needs to light a frame.
///
/// [`Lighting::NONE`] is the identity — full ambient, no lights, nothing
/// standing anywhere — and the blit multiplies by exactly `1.0` for it, so a
/// frame test comparing the surface with the world image texel for texel still
/// holds.
#[derive(Clone, PartialEq, Debug)]
pub struct Lighting {
    /// What everything is multiplied by away from any flame — the daylight, or
    /// the lack of it, per tile. [`Ambient::DAY`] over an empty grid is "no
    /// lighting at all".
    pub ambient: Ambient,
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
        ambient: Ambient::DAY,
        lights: Vec::new(),
        occlusion: Occlusion::EMPTY,
        sun: None,
        view: crate::debug::View::Lit,
    };

    /// Whether this would change a single pixel.
    ///
    /// The occluders *are* asked about now, and that is decision 1 of
    /// `docs/lighting_world.md` arriving here: a wall with no flame to stop
    /// still casts nothing, but a roof takes the sky's share of the ambient away
    /// from the tile under it whether anything burns or not. A grid with
    /// something in it is therefore a frame that may be darker than its world
    /// image, and only an empty one is a copy.
    ///
    /// Put a flame into the frame that no walk of the map could have found: the
    /// one the player is carrying.
    ///
    /// First in the list and never the one dropped. [`collect`] keeps the
    /// [`MAX`](Self::MAX) flames nearest the eye, and the flame *in the eye's own
    /// hand* is the one whose absence would be noticed instantly — a torch that
    /// went out because the player walked into a lit tavern is a worse frame than
    /// one candle at the far end of it going missing.
    pub fn hold(&mut self, light: Light) {
        self.lights.insert(0, light);
        self.lights.truncate(Self::MAX);
    }

    /// A debug view is never the identity, however empty the frame's lighting is
    /// — that is the whole of what it draws.
    pub fn is_identity(&self) -> bool {
        self.lights.is_empty()
            && self.ambient == Ambient::DAY
            && self.occlusion.is_empty()
            && self.view.is_lit()
    }
}

/// The floor under the darkness: what a tile with no sky at all still gets.
///
/// Decision 1's `GROUND_AMBIENT`, and it is small and cold on purpose. Small,
/// because the whole of what the split buys is that a room is darker than the
/// road outside it, and a generous floor gives that back. Cold, because it
/// stands in for light that has bounced off a stone floor and a plastered wall
/// rather than for a source — and because a warm floor would take the one hue a
/// flame has to itself.
///
/// Invented here, in the way `docs/lighting_world.md`'s decision 11 says every
/// number in this plan is: held by a scene, not argued into existence.
pub const GROUND_AMBIENT: [f32; 3] = [0.12, 0.13, 0.18];

/// Night, as the reference isometrics draw it: dark, and *cooler* than the art.
///
/// The blue cast is what makes a fire read as warm — with a grey ambient the
/// pool and the dark are the same hue at two brightnesses, which the eye reads
/// as a spotlight rather than as firelight.
///
/// The two terms sum to the `[0.30, 0.33, 0.45]` this was one colour of before
/// the split, so a street at night is exactly as dark as it was and what changed
/// is only what happens indoors.
pub const NIGHT: Ambient = Ambient {
    sky: [0.20, 0.22, 0.31],
    ground: [0.10, 0.11, 0.14],
};

/// What a daylit world is lit by *away from the sun*: the sky.
///
/// Well short of white, because with a sun in the frame the sun supplies the
/// rest — an ambient that already lit everything would leave every shadow the
/// sun casts invisible. And well short of black, because a shadow at noon is not
/// a hole: the reference isometrics draw one lit by the sky, and so does this.
///
/// Split like [`NIGHT`] and for the same reason: the two terms sum to the
/// `[0.55, 0.55, 0.62]` a daylit frame had everywhere, so the street is
/// unchanged and the room under the roof is what moved.
pub const SKYLIGHT: Ambient = Ambient {
    sky: [0.43, 0.42, 0.44],
    ground: GROUND_AMBIENT,
};

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

/// Whether a static is a flame at all: it says it is a light source, **and it is
/// not something light cannot get through**.
///
/// The second half is what stops a city burning at every window. 615 of the
/// install's statics carry `LIGHT_SOURCE` and 80 of the 163 named "window" are
/// among them — `0x0103`, `0x2BBF`, the shutters at `0x2501`, the windowed walls
/// at `0x2B7D`. Every one of them is also a wall: `WALL | BLOCK | WINDOW`, which
/// is an occluder, and [`flame`] answers `TORCH` for any graphic it has no name
/// for. So a street of houses was a street of six-tile warm pools with nothing
/// burning in them, each one standing inside the very panel that then cut it into
/// slices.
///
/// **A window is not an emitter.** It is a hole with glass in it, it is already
/// in the occlusion grid as [`crate::occlusion::PANE`], and what should make it
/// glow is a candle behind it — which is the one thing this pass can already do.
/// The flag on those graphics is the client's way of saying "draw a glow here",
/// and this renderer answers that question with geometry instead.
///
/// Stated as "does it stop light" rather than as a list of window graphics,
/// because that is the property that matters and it is already computed for the
/// grid: a torch, a candle and a brazier stop nothing and burn; a glazed wall
/// stops four fifths and does not. A shard's custom lantern goes on burning for
/// free, and a shard's custom glowing wall stops — which is the conservative
/// direction, a missing pool being easier to see than sixty invented ones.
pub fn burns(graphic: Graphic, tile: &openshard_uofiles::tiledata::StaticTile) -> bool {
    tile.flags.is_light_source() && crate::occlusion::opacity(graphic, tile) == crate::occlusion::CLEAR
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
///
/// Public because it is the rectangle *the grid is*, and a second caller that
/// wants the same grid must not guess at it: the app's occluder overlay
/// (`docs/lighting.md`, step 14) rebuilds the grid to draw it, and a wireframe
/// over a rectangle the shader did not walk is an instrument that lies about
/// exactly the edge it exists to show.
pub fn lit_tiles(camera: &Camera) -> crate::camera::TileBounds {
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
///
/// `atlas` is where an occluder's *facing* comes from, and it is an `Option`
/// because not every caller has pictures: a built scene has a map and an item
/// list and no art at all. Without it every occluder is the whole tile it was
/// before [`crate::facing`] existed, which is the safe answer and not a broken
/// one — see [`occlusion::collect`](crate::occlusion::collect).
// Eight, and every one of them is a different thing the frame knows: the world,
// what the server has put in it, where the eye is, what the client's files say,
// what the frame has cut away, what the sky is doing, when, and the pictures.
// Grouping them into a struct would be one more type to keep in step with the
// call sites for no fewer facts — and the call sites are three.
#[allow(clippy::too_many_arguments)]
pub fn collect(
    map: &Map,
    items: &[GroundItem],
    camera: &Camera,
    tiledata: &TileData,
    cutaway: &Cutaway,
    ambient: Ambient,
    time: f32,
    atlas: Option<&crate::atlas::StaticAtlas>,
) -> Lighting {
    let bounds = lit_tiles(camera);
    let mut lights = Vec::new();

    crate::statics::for_each_static_in(map, bounds, |item| {
        let tile = tiledata.static_tile(item.tile);
        if !burns(Graphic(item.tile), tile) || !cutaway::shows(cutaway, item.z, tile) {
            return;
        }
        lights.push(place(
            Point::new(item.x, item.y, item.z),
            flame(Graphic(item.tile)),
            time,
        ));
    });

    for item in items {
        let tile = tiledata.static_tile(item.graphic.0);
        if !burns(item.graphic, tile) || !cutaway::shows(cutaway, item.at.z, tile) {
            continue;
        }
        lights.push(place(item.at, flame(item.graphic), time));
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
        occlusion: crate::occlusion::collect(map, items, bounds, tiledata, cutaway, atlas),
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
///
/// The [`Flame`] and not the [`Graphic`] it came from, because [`carried`] has
/// no graphic at all — nothing on the wire says a hand is holding a torch — and
/// a stand-in graphic passed in only to be looked up again would be a second
/// place the mapping lives.
fn place(at: Point, flame: Flame, time: f32) -> Light {
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
        // Every fire standing in the world burns in every direction. A beam is
        // something a hand does to a flame — see [`carried`].
        beam: None,
    }
}

/// How many cells of the grid one ray may look at.
///
/// `blit.wgsl`'s `MAX_WALK_STEPS`, and the two are one number: [`sample`] is the
/// shader's own arithmetic in Rust and a bound that differed would make the two
/// disagree exactly where a ray is longest. One number for both rays, because
/// [`walk_cells`] is one walk: a pool reaches nine tiles at the widest, a
/// sunbeam's segment runs [`MAX_SUN_TILES`], and a walk visits every cell the
/// segment crosses, which on a diagonal is both axes' worth. It is never actually
/// reached; it exists so that a loop over data cannot be made unbounded by a
/// radius somebody widens later.
pub const MAX_WALK_STEPS: i32 = 72;

/// How far a ray must travel inside an occluding cell for that cell to stop all
/// it can, in tiles. `blit.wgsl`'s `SOFT_CROSSING`.
///
/// The walk knows the length of each cell it crosses, and spending it is what
/// makes a shadow's edge a gradient rather than a step at a tile boundary: a ray
/// that clips a wall tile's corner keeps most of its light, one that crosses the
/// tile squarely keeps none.
///
/// It is not one length. A flame is a body, not a point, so an occluder close to
/// what it shadows draws a sharp edge and a distant one draws a wide penumbra
/// whose width is the flame's own size times `t / (1 - t)` — `t` being how far
/// along the ray the occluder is from the lit end. That is where these three
/// numbers go: [`FLAME_SPREAD`] is the size in tiles, and the bounds keep the
/// ends of the ratio finite. Invented here, the way [`crate::occlusion::PANE`]
/// is — no client file says how big a flame is.
const FLAME_SPREAD: f32 = 1.0;

/// The narrowest a shadow's edge gets: an occluder the fragment is against.
const SOFT_CROSSING_MIN: f32 = 0.05;

/// And the widest, for an occluder almost at the flame.
const SOFT_CROSSING_MAX: f32 = 0.7;

/// Below this, a ray has been stopped: `blit.wgsl`'s early exit, and under a
/// byte's worth of light either way.
const RAY_CUTOFF: f32 = 0.004;

/// How much of a panel a ray pierces at height `z` runs into: `1.0` well inside
/// the span it occupies, `0.0` well outside, and a gradient `tall` `z` units wide
/// across its edges.
///
/// The vertical half of decision 14's penumbra, and all that is left of it: a
/// flame is a body rather than a point, so a ray grazing the top of a wall is
/// dimmed rather than switched.
///
/// The band is centred on the *top* edge and hangs below the bottom one, for the
/// reason `blit.wgsl`'s `pierces` states at length: a wall is based on the ground
/// it stands on and the ray a person looks at runs along that base, so a band
/// centred there would let half of every flame along every wall in the frame.
///
/// `blit.wgsl`'s `pierces`, and the two are one formula.
fn pierces(z: f32, low: f32, high: f32, tall: f32) -> f32 {
    let band = tall.max(1e-3);
    ((z - low + band * 0.5).min(high - z) / band + 0.5).clamp(0.0, 1.0)
}

/// How near two boundaries have to be, along the ray, for the ray to be crossing
/// a **corner** rather than a side. `blit.wgsl`'s `CORNER_TIE`, and the two are
/// one number — this is a comparison the two implementations have to answer the
/// same way, and what makes that safe is that the tolerance is a thousand times
/// the last bits of a float and a thirtieth of a pixel of world.
const CORNER_TIE: f32 = 1e-4;

/// How much one cell stops a ray that crosses the sides in `crossed` at height
/// `z`, where the cell is a panel. Zero for open ground, for a lid, and for a
/// panel on a side the ray does not go through.
///
/// `blit.wgsl`'s `panel_stop`. Split out of [`walk_cells`] for the corner case,
/// which has two cells to ask about and no crossing length to speak of.
fn panel_stop(stands: Option<crate::occlusion::Cell>, crossed: u8, z: f32, tall: f32) -> f32 {
    match stands.filter(|stands| stands.edges != 0 && stands.edges & crossed != 0) {
        None => 0.0,
        Some(stands) => {
            f32::from(stands.opacity) / 255.0 * pierces(z, stands.bottom as f32, stands.top as f32, tall)
        }
    }
}

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
    /// How much of the flame's [`Beam`] falls here: `1.0` for a fire that lights
    /// every direction and for a spot the beam points straight at,
    /// [`BEAM_SPILL`] for one behind it.
    ///
    /// A separate number from [`Reach::through`] and not folded into it, because
    /// the two answer the questions a person asks in the order they ask them:
    /// "is the light pointing at me" comes before "is something in the way", and
    /// a report that gave one number could not tell a spot behind the player
    /// from a spot behind a wall.
    pub cone: f32,
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
            // In the order the questions are asked: is it near enough, is
            // anything in between, and how much of its beam this spot is in —
            // see [`Reach::cone`], which is the number that says whether a dark
            // tile is behind a wall or behind the character.
            match (reach.within, reach.stopped_by) {
                (false, _) => writeln!(f, ", outside its radius")?,
                (true, Some((x, y))) => writeln!(f, ", stopped at ({x}, {y})")?,
                (true, None) => writeln!(
                    f,
                    ", through {:.2}, beam {:.2}, adds {:.3}",
                    reach.through,
                    reach.cone,
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
    // The ambient this *tile* has, and not the frame's: how much of the sky the
    // column over it can see decides how much of the sky term it gets. The tile
    // and not the fractional spot, because the field is a byte a tile — the blur
    // of `docs/lighting_world.md`'s decision 2 is what softens its edges, and a
    // second interpolation here would be a different picture from the shader's.
    let mut multiplier = lighting.ambient.at(lighting
        .occlusion
        .sky_at(spot.at.x.floor() as i32, spot.at.y.floor() as i32));
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
                cone: 0.0,
                stopped_by: None,
                added: [0.0; 3],
            });
            continue;
        }
        // Which way the light is pointing, before anything is asked about what
        // stands in the way: a beam that misses this spot has nothing to be
        // stopped by. The offset is from the spot to the flame, and a beam's
        // axis points the other way, so the sign flips here.
        let cone = match light.beam {
            Some(beam) => beam.lights(offset.map(|axis| -axis)),
            None => 1.0,
        };
        let (through, stopped_by) = walk(spot, light, &lighting.occlusion);
        let fall = 1.0 - d;
        let added = light
            .color
            .map(|channel| channel * light.intensity * fall * fall * through * cone);
        for (total, channel) in multiplier.iter_mut().zip(added) {
            *total += channel;
        }
        reaches.push(Reach {
            light: index,
            distance,
            within: true,
            through,
            cone,
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
            // The sun is a direction and not a beam: it lights everything it can
            // see, and there is nothing for a cone to exclude.
            cone: 1.0,
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
/// `blit.wgsl`'s `sunlight`. What the sun has instead of a position is a
/// *direction*, so the only thing this does that [`walk`] does not is work out
/// where the segment ends: the point at which the ray leaves the grid's ceiling,
/// because from there on it is looking at sky. Everything after that is
/// [`walk_cells`], the same walk a flame's ray takes.
///
/// The spot's own tile is skipped, as it is for a flame, and for the same reason
/// in reverse: a wall's own pixels are on a tile that stops light, and a wall
/// that shadowed itself would be black on the side the sun is on. The far end is
/// *not* skipped — there is no tile there, only a point in the sky.
fn walk_sun(spot: Spot, sun: Sun, occlusion: &Occlusion) -> (f32, Option<(i32, i32)>) {
    let horizontal = (sun.toward[0] * sun.toward[0] + sun.toward[1] * sun.toward[1]).sqrt();
    if horizontal < 1e-6 {
        // Straight overhead: there is no direction to walk along the ground, and
        // the only thing that could shadow the spot is on its own tile — which is
        // exempt. Nothing stops it.
        return (1.0, None);
    }
    // One tile of ground a unit, so `z` climbs by the sun's own slope.
    let step = [
        sun.toward[0] / horizontal,
        sun.toward[1] / horizontal,
        sun.toward[2] / horizontal * Z_PER_TILE,
    ];
    let mut tiles = MAX_SUN_TILES;
    if let (Some(ceiling), true) = (occlusion.tallest(), step[2] > 1e-6) {
        tiles = tiles.min((ceiling as f32 - spot.z) / step[2]);
    }
    if occlusion.tallest().is_none() || tiles <= 0.0 {
        // Nothing in the grid stops anything, or the spot is already above
        // everything that could — either way the ray is in the sky from here.
        return (1.0, None);
    }
    let from = [spot.at.x, spot.at.y, spot.z];
    let to = [
        from[0] + step[0] * tiles,
        from[1] + step[1] * tiles,
        from[2] + step[2] * tiles,
    ];
    // No tile to exempt at the far end, and a point source: the sun subtends half
    // a degree, so its penumbra is the narrowest the walk draws.
    walk_cells(from, to, false, 0.0, occlusion)
}

/// The ray from a spot to a flame: [`walk_cells`] with a flame's two ends.
///
/// The flame's own tile must not shadow it — a sconce stands *on* a wall — and a
/// flame is a body about a tile across, which is what its penumbra is made of.
/// Those two facts are the whole difference between this ray and the sun's.
fn walk(spot: Spot, light: &Light, occlusion: &Occlusion) -> (f32, Option<(i32, i32)>) {
    walk_cells(
        [spot.at.x, spot.at.y, spot.z],
        [light.at.x, light.at.y, light.z],
        true,
        FLAME_SPREAD,
        occlusion,
    )
}

/// One segment of the world, cell by cell: how much of a ray survives it, and
/// what stopped it.
///
/// `blit.wgsl`'s `walk`, including what it leaves out, and **one walk for both
/// the flame and the sun** — see the shader for the argument, and for the
/// measurement that produced it. The ends are the parameters: `skip_last` is the
/// flame's own tile, and `spread` is how big the source is, in tiles. A sunbeam
/// passes `false` and `0.0`.
///
/// Every cell the segment crosses, in order, with the length of each crossing:
/// not a fixed number of samples, which at two tiles apart was one interior
/// point and put every shadow's edge on a tile boundary. What a cell stops is
/// its opacity scaled by how far the ray ran inside it — [`FLAME_SPREAD`] and its
/// two bounds — and by how much of that run was inside the span the tile
/// occupies, so a ray grazing the top of a wall or clipping its corner is dimmed
/// rather than cut.
///
/// The starting cell is always skipped: the tile being lit must not shadow
/// itself, which is what keeps a wall's own face the brightest thing beside a
/// torch.
fn walk_cells(
    from: [f32; 3],
    to: [f32; 3],
    skip_last: bool,
    spread: f32,
    occlusion: &Occlusion,
) -> (f32, Option<(i32, i32)>) {
    let spot = Spot {
        at: Vec2::new(from[0], from[1]),
        z: from[2],
    };
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let ground = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    if ground < 1e-6 {
        // Straight up or down: the only cells on the line are the exempt ones,
        // and there is no direction to walk in.
        return (1.0, None);
    }
    let first = (from[0].floor() as i32, from[1].floor() as i32);
    let last = (to[0].floor() as i32, to[1].floor() as i32);
    let mut cell = first;
    // Which way each axis steps, how much of the whole segment one tile of it is
    // worth, and how far along the segment the first boundary is. An axis the
    // ray does not move along never reaches its boundary, which is what the
    // enormous `t` says.
    let toward = (
        match delta[0] >= 0.0 {
            true => 1,
            false => -1,
        },
        match delta[1] >= 0.0 {
            true => 1,
            false => -1,
        },
    );
    let mut per_tile = [1e30_f32; 2];
    let mut boundary = [1e30_f32; 2];
    for axis in 0..2 {
        if delta[axis].abs() <= 1e-6 {
            continue;
        }
        per_tile[axis] = 1.0 / delta[axis].abs();
        let from = [spot.at.x, spot.at.y][axis];
        let ahead = match delta[axis] >= 0.0 {
            true => from.floor() + 1.0 - from,
            false => from - from.floor(),
        };
        boundary[axis] = ahead * per_tile[axis];
    }

    let mut entered = 0.0;
    let mut through = 1.0;
    // Which side of the current cell the ray came in through, and which it is
    // about to leave by. `blit.wgsl`'s `walk`, line for line — see there for why
    // an occluder is a panel on one side rather than a whole tile.
    let mut entry = 0u8;
    for _ in 0..MAX_WALK_STEPS {
        let next = boundary[0].min(boundary[1]);
        let leaves = next.min(1.0);
        let out_by_x = boundary[0] < boundary[1];
        let exit = match next < 1.0 {
            false => 0,
            true => match (out_by_x, out_by_x && toward.0 > 0 || !out_by_x && toward.1 > 0) {
                (true, true) => crate::occlusion::EDGE_EAST,
                (true, false) => crate::occlusion::EDGE_WEST,
                (false, true) => crate::occlusion::EDGE_SOUTH,
                (false, false) => crate::occlusion::EDGE_NORTH,
            },
        };
        let stands = occlusion.at(cell.0, cell.1);
        // Neither end of the ray is shadowed by the tile it is on: the lit end
        // because a wall's two faces are one tile and there is no telling which of
        // them a pixel is on, and the flame's end because a sconce is mounted on a
        // wall. `blit.wgsl`'s `walk` argues both, and carries the measurement that
        // settled the second.
        if cell != first && (!skip_last || cell != last) {
            if let Some(stands) = stands {
                let (low, high) = (stands.bottom as f32, stands.top as f32);
                let opacity = f32::from(stands.opacity) / 255.0;
                // How soft this cell's own edge is: the penumbra a source of
                // this size casts from this far along the ray. See
                // [`FLAME_SPREAD`]; a `spread` of zero is a point source, and the
                // clamp leaves it the narrowest edge the walk draws.
                let middle = (entered + leaves) * 0.5;
                let soft =
                    (spread * middle / (1.0 - middle).max(1e-3)).clamp(SOFT_CROSSING_MIN, SOFT_CROSSING_MAX);
                let stopped = match stands.edges {
                    // A **body** — a lid (a floor, a roof) or a whole tile that
                    // stands up and whose art would not say which way. Either is a
                    // solid the ray travels through, so what it stops is scaled by
                    // the length of the run inside the span. `blit.wgsl`'s `walk`
                    // argues why all four sides belongs here: a roof five `z` deep
                    // is pierced by neither of its sides at 45°, and a sealed
                    // house came out sunlit.
                    0 | EDGE_ANY => {
                        let from = spot.z + delta[2] * entered;
                        let to = spot.z + delta[2] * leaves;
                        let (bottom, top) = (from.min(to), from.max(to));
                        let share = match top - bottom > 1e-6 {
                            true => (top.min(high) - bottom.max(low)).max(0.0) / (top - bottom),
                            // A level ray: it is inside the span or it is not, and
                            // there is no length of it to take a share of.
                            false => match bottom >= low && bottom <= high {
                                true => 1.0,
                                false => 0.0,
                            },
                        };
                        let crossed = (leaves - entered) * ground * share;
                        opacity * (crossed / soft).clamp(0.0, 1.0)
                    }
                    // A **panel** — a surface on one side of the tile. What it does
                    // to a ray is decided where the ray *pierces* it, at a point
                    // and at a height, and not by how long the ray spent in the
                    // cell. `blit.wgsl`'s `walk` argues it: the length version let
                    // a ray through the corner between two panels and drew a fan
                    // of spokes out of every lamp near a wall.
                    edges => {
                        let tall = soft * Z_PER_TILE;
                        let mut stopped: f32 = 0.0;
                        for (side, at) in [(entry, entered), (exit, leaves)] {
                            if edges & side != 0 {
                                stopped =
                                    stopped.max(opacity * pierces(spot.z + delta[2] * at, low, high, tall));
                            }
                        }
                        stopped
                    }
                };
                through *= 1.0 - stopped;
                if through <= RAY_CUTOFF {
                    return (0.0, Some(cell));
                }
            }
        }
        if next >= 1.0 {
            break;
        }
        entered = next;
        // Which sides the neighbours touch this corner or this boundary by: a ray
        // moving east leaves through an east side and enters a west one.
        let enter_x = match toward.0 > 0 {
            true => crate::occlusion::EDGE_WEST,
            false => crate::occlusion::EDGE_EAST,
        };
        let enter_y = match toward.1 > 0 {
            true => crate::occlusion::EDGE_NORTH,
            false => crate::occlusion::EDGE_SOUTH,
        };
        if (boundary[0] - boundary[1]).abs() <= CORNER_TIE {
            // **A corner** — `blit.wgsl`'s `walk` argues it: four tiles meet at
            // the point the ray leaves by, and the two the walk does not step
            // into are as much in the way as the one it does. Both are asked, and
            // then the walk steps diagonally past them.
            let by_x = (cell.0 + toward.0, cell.1);
            let by_y = (cell.0, cell.1 + toward.1);
            let z = spot.z + delta[2] * next;
            let tall = (spread * next / (1.0 - next).max(1e-3)).clamp(SOFT_CROSSING_MIN, SOFT_CROSSING_MAX)
                * Z_PER_TILE;
            let mut corner: f32 = 0.0;
            let mut blamed = None;
            for (at, crossed) in [
                (by_x, enter_x | crate::occlusion::opposite(enter_y)),
                (by_y, enter_y | crate::occlusion::opposite(enter_x)),
            ] {
                if at == first || (skip_last && at == last) {
                    continue;
                }
                let stops = panel_stop(occlusion.at(at.0, at.1), crossed, z, tall);
                if stops > corner {
                    corner = stops;
                    blamed = Some(at);
                }
            }
            through *= 1.0 - corner;
            if through <= RAY_CUTOFF {
                return (0.0, blamed);
            }
            cell = (by_x.0, by_y.1);
            boundary[0] += per_tile[0];
            boundary[1] += per_tile[1];
            // The cell beyond is entered by *both* the sides that meet at the
            // corner, so a wall on either of them stops the ray there too.
            entry = enter_x | enter_y;
            continue;
        }
        // The neighbour's own entry is this cell's exit seen from the other
        // side: leaving east is entering west.
        entry = crate::occlusion::opposite(exit);
        // Into the neighbour across whichever boundary is nearer.
        match out_by_x {
            true => {
                cell.0 += toward.0;
                boundary[0] += per_tile[0];
            }
            false => {
                cell.1 += toward.1;
                boundary[1] += per_tile[1];
            }
        }
    }
    (through, None)
}

/// How wide the flame in a hand throws its light: the full angle, in degrees.
///
/// Sixty is a lamp rather than a searchlight — wide enough that walking is not
/// done down a tube, narrow enough that the direction the character is facing is
/// legible from the picture alone, which is the whole of what a carried light is
/// worth. It is a stand-in in exactly the way [`flame`] is: nothing on the wire
/// says a mobile is holding anything, so this is the client's own guess until
/// the equipment layers are read for a torch.
pub const HELD_BEAM_DEGREES: f32 = 60.0;

/// The flame the player carries: where it burns, which way it points, and how
/// it flickers.
///
/// Not a static and not a ground item, so no walk of the map could produce it —
/// [`Lighting::hold`] is how it gets into a frame. It is a [`TORCH`] in
/// everything but the [`Beam`], which is what makes the difference between a
/// character who glows and a character who is *carrying* something: an
/// omnidirectional pool centred on a body lights the wall behind it exactly as
/// brightly as the one it is walking towards, and the eye reads that as the
/// character being the source rather than the hand being it.
///
/// The axis is level with the ground and not tilted down at it. A torch aimed at
/// the floor two tiles ahead lights that floor beautifully and leaves the top of
/// every wall in front of it outside the cone — with a level axis the pool on the
/// ground is only a little shorter and a wall three tiles off is lit to nearly
/// its full height, which is the picture that says a beam has hit something.
pub fn carried(at: Point, facing: Direction, time: f32) -> Light {
    let (dx, dy) = facing.step();
    Light {
        beam: Some(Beam::towards(dx as f32, dy as f32, 0.0, HELD_BEAM_DEGREES)),
        ..place(at, TORCH, time)
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
            None,
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
            None,
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
            let lighting = collect(
                &bare(),
                &items,
                &camera,
                &tiledata,
                &Cutaway::OPEN,
                NIGHT,
                0.0,
                None,
            );
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
        let lighting = collect(
            &bare(),
            &items,
            &camera,
            &tiledata,
            &Cutaway::OPEN,
            NIGHT,
            0.0,
            None,
        );
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
            None,
        );
        assert!(lighting.lights.is_empty());
    }
}
