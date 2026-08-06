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
use crate::facing::Face;
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
/// `bake` is the blocks of the occlusion grid a caller has already derived, and
/// it is an `Option` for the same reason `atlas` is: not every caller keeps one
/// across frames. `None` builds the grid from nothing, which is the same grid —
/// see [`occlusion::bake`](crate::occlusion::bake), whose first test is that the
/// two are equal.
// Nine, and every one of them is a different thing the frame knows: the world,
// what the server has put in it, where the eye is, what the client's files say,
// what the frame has cut away, what the sky is doing, when, the pictures, and
// what was built for the last frame. Grouping them into a struct would be one
// more type to keep in step with the call sites for no fewer facts.
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
    bake: Option<&mut crate::occlusion::bake::Bake>,
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

    // The grid before the flames are placed, because where a mounted flame burns
    // is a fact about what it is mounted *on* — see `mounted_at`.
    let occlusion = match bake {
        Some(bake) => crate::occlusion::bake::collect(bake, map, items, bounds, tiledata, cutaway, atlas),
        None => crate::occlusion::collect(map, items, bounds, tiledata, cutaway, atlas),
    };
    for light in &mut lights {
        light.at = mounted_at(light.at, &occlusion);
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
        occlusion,
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

/// How far outside the plane a mounted flame is placed, in tiles.
///
/// Half a tile takes it from its tile's centre to the plane the panel stands on,
/// and [`FACE_EDGE`] more takes it clear of the band the facing test softens
/// over — so the wall it hangs on is lit at full strength rather than at the half
/// a flame lying exactly in the plane would give it.
///
/// The consequence worth stating: it lands on the *next* tile, so the wall it is
/// mounted on stops being the flame's own cell and starts being an ordinary
/// occluder. That is what makes a sconce light the street and not the room behind
/// it, and it is the whole reason this is a move rather than an exemption.
const MOUNTED_CLEARANCE: f32 = 0.5 + FACE_EDGE;

/// Where a flame standing on a wall tile actually burns: outside the plane its
/// own tile names, on the side the wall's picture is drawn from.
///
/// A sconce, a lamp bracket, a torch in a wall ring — a static whose tile carries
/// a **panel** — is bolted to the *outside* of that panel, and the map does not
/// say so: it says the tile. Left at its tile's centre it is behind the plane of
/// the face it lights, and two things follow that a person can see. Its own wall
/// comes out dark, because a face is one-sided and the flame is behind it. And
/// its own tile is exempt from shadowing it (decisions 3 and 17), so the room on
/// the other side of that wall is lit exactly as brightly as the street.
///
/// `docs/lighting.md`'s backlog has carried the shape of this since the first
/// version of the pass — *"a lamp mounted on a wall wants pushing off it, not
/// exempting from it"* — and the grid already holds what it needs. Moving the
/// flame answers both, and it is what let the facing test lose its exemption for
/// a flame standing in a wall's line, which is a whole street long and lit every
/// wall in it.
///
/// A tile with no panel is not moved, and that covers the ordinary cases by
/// construction: a torch on the ground, a lamp post in the street, a brazier in a
/// room. So is a cell whose sides cancel — [`EDGE_ANY`](crate::occlusion::EDGE_ANY),
/// the whole-tile answer for a graphic the art would not name, and a lid — because
/// there is no direction in it to move along and a guess would be a wrong one.
fn mounted_at(at: Vec2, occlusion: &crate::occlusion::Occlusion) -> Vec2 {
    let Some(cell) = occlusion.at(at.x.floor() as i32, at.y.floor() as i32) else {
        return at;
    };
    // Componentwise and not along one normalised direction, so that a flame on a
    // **corner** — two panels, and every building has them — goes clear of both
    // planes rather than half clear of each.
    let toward = |positive: u8, negative: u8| match (cell.edges & positive != 0, cell.edges & negative != 0) {
        (true, false) => MOUNTED_CLEARANCE,
        (false, true) => -MOUNTED_CLEARANCE,
        // Neither side, or both: a lid, a whole-tile occluder, or a tile holding
        // two walls that face away from each other. No direction, no move.
        _ => 0.0,
    };
    Vec2::new(
        at.x + toward(crate::occlusion::EDGE_EAST, crate::occlusion::EDGE_WEST),
        at.y + toward(crate::occlusion::EDGE_SOUTH, crate::occlusion::EDGE_NORTH),
    )
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

/// How much of a **lid** is in the way of a ray that runs from `from` to `to` in
/// `z` across one cell: `1.0` where the ray went through the plane and out the
/// other side, `0.0` where it stayed on one side of it, and a gradient between
/// where the flame itself straddles the plane.
///
/// **A lid is a plane and not a slab, and that is the whole of why this is not
/// [`pierces`] and not the length rule beside it.** A floor is `height 0` in
/// `tiledata.mul` — 4,534 of the 4,647 lids over the block of Britain
/// `artscan`'s `column` example reads — so its span is zero deep, and a rule that
/// scales what an occluder stops by how far the ray ran *inside* the span gets
/// zero out of every floor in the world. That is what lit the storey above a
/// torch through its own floorboards; see `scene::storey_over_a_torch`.
///
/// The crossing test is **strict**, and that is the one thing here that has to be
/// argued rather than stated. A ray that runs exactly along the top of a lid — a
/// candle standing on the floor it lights, both at one `z` — has not gone through
/// anything, and a test that counted a touch would put half a floor's shadow
/// across every room lit from inside it. It is [`pierces`]'s asymmetry arriving
/// at the surface that has no thickness for a band to hang under.
///
/// The softness is the flame's own size and is measured at the *flame*: the plane
/// cuts the source, so what gets through is the share of it left on the lit
/// side. `source` is the ray's far end in `z` and `spread` how big the flame is,
/// in tiles — a sunbeam passes `0.0` and gets the hard edge a point source casts.
///
/// **How tall that flame is, though, is [`FLAME_DEPTH`] and not its width.** The
/// two were one number for a day and it lit the storey over every wall sconce in
/// Britain: a sconce burns four or five `z` under the floor above it, and a flame
/// eleven `z` tall — [`FLAME_SPREAD`] of one tile, which is the *lateral*
/// softness of a shadow edge — pokes a tenth of itself through the boards.
///
/// `blit.wgsl`'s `crosses`, and the two are one formula.
fn crosses(entering: f32, leaving: f32, low: f32, high: f32, source: f32, spread: f32) -> f32 {
    let (under, over) = (entering.min(leaving), entering.max(leaving));
    if under >= high || over <= low {
        return 0.0;
    }
    // How far past the lid the flame itself stands, on the side the ray left by.
    let beyond = match leaving >= entering {
        true => source - high,
        false => low - source,
    };
    (beyond / (spread * FLAME_DEPTH).max(1e-3) + 0.5).clamp(0.0, 1.0)
}

/// How tall a flame is, in `z` units, for the one question that asks: how much of
/// it a floor cuts off ([`crosses`]).
///
/// A **quarter of a tile**, and the art is what says so rather than another
/// constant: the projection draws four screen pixels to one `z`
/// ([`crate::camera::Z_STEP`]), and the flame a torch graphic actually has drawn
/// on it is eight or ten pixels tall — two and a half `z`. Half a tile was the
/// first answer here, taken from [`FLAME_LIFT`] because it was the only number
/// in the file about a flame's height, and it is twice what the pictures show.
///
/// What the difference is worth, on the corner of Britain's house at
/// `1509,1635`: a ray passing three quarters of a `z` under the top of the wall
/// beside it keeps `0.31` of its light at half a tile, `0.11` at a quarter and
/// nothing at an eighth. The middle one is the picture; the third would be
/// choosing the number to make one pixel dark.
///
/// Scaled by the caller's `spread` so that a point source stays a point: the sun
/// passes `0.0` and a plane cuts it cleanly, which is what a floor's own shadow
/// on the ground under it is made of.
///
/// **It is what turns a softness in tiles into one in `z`, everywhere.** A
/// penumbra is the size of the source *across the edge it spills over*, and
/// every edge this pass softens vertically — a wall's top, a hole's sill, a
/// lid's plane — is horizontal, so what blurs it is how tall the flame is and
/// not how wide. [`Z_PER_TILE`] did that conversion until a house's corner was
/// measured: a ray passing three quarters of a `z` under the top of a wall kept
/// two fifths of its light, because the band was seven and a half `z` — a flame
/// as tall as it is wide. The lateral softness is unchanged and still
/// [`FLAME_SPREAD`]'s; it is the axis that was being asked the wrong question.
const FLAME_DEPTH: f32 = Z_PER_TILE / 4.0;

/// How far in front of its own plane a **face** pixel is walked from, in tiles.
///
/// `statics.wgsl` places one at `INSIDE` — a hundred-and-twenty-seventh short of
/// the plane — because a fraction of exactly one names the *next* tile and the
/// attachment's tile is what a click selects. That is right for the attachment
/// and wrong for the walk: the pixel is drawn on the plane, the space it is lit
/// from is in front of it, and the floor whose edge meets that plane belongs to
/// the tile in front. Eight thousandths of a tile behind the boundary was enough
/// for a ray to cross a storey's floor *before* reaching the cell that floor is
/// in, which is the bright line a house wore along its floorboards.
///
/// Two steps of the seven-bit fraction, which is what it takes to get past the
/// boundary from `INSIDE` and is a third of a pixel of world. Only the walk moves
/// — the attachment still names the wall's own tile, so picking, the debug views
/// and the wireframe are untouched.
const STAND_OFF: f32 = 2.0 / 127.0;

/// And how far **above** whatever it lies on every point of the world is walked
/// from, in `z`.
///
/// The other half of the same sentence, and the half a lid needs: a plane is
/// crossed rather than travelled through ([`crosses`]), and the test is strict,
/// so a point whose `z` is exactly a floor's lies *in* that floor and a ray from
/// it to a flame below runs along the plane rather than through it. A pixel is
/// drawn on top of the boards, not in them; so is a candle standing on them,
/// which is why this moves the flame's end too.
///
/// Well under one `z` unit — the attachment quantises `z` to whole ones — and
/// well over the last bits of a float.
const ON_TOP: f32 = 1.0 / 128.0;

/// The two ends of a ray, moved off the surfaces they are drawn on: see
/// [`STAND_OFF`] and [`ON_TOP`].
///
/// The flame's end gets the height and not the offset. A mounted flame is
/// already outside the plane its tile names — decision 26's `mounted_at`, which
/// moves it by a good deal more than this — and a flame has no face of its own to
/// be in front of.
fn stand_clear(from: [f32; 3], to: [f32; 3], surface: Surface) -> ([f32; 3], [f32; 3]) {
    let [ahead, across] = match surface.face() {
        Some(face) => face.outward(),
        None => [0.0, 0.0],
    };
    (
        [
            from[0] + ahead * STAND_OFF,
            from[1] + across * STAND_OFF,
            from[2] + ON_TOP,
        ],
        [to[0], to[1], to[2] + ON_TOP],
    )
}

/// Whether a lit point lies **on** a surface: its `z` is inside the span that
/// surface occupies, its two edges included.
///
/// What the exemptions are asked, one surface at a time. "A surface does not
/// shadow itself" needs to know which surface a pixel *is* a point of, and a
/// tile of a two-storey house holds a wall for each storey: `0..20` and
/// `20..40`, two surfaces, and a pixel at `z 25` is on the second. The first is
/// under its feet and occludes it exactly as anybody else's wall would.
///
/// Inclusive at both ends on purpose: a wall's base is the ground it stands on
/// and its top is the cap somebody's floor pixel is lying on, and a pixel is a
/// point of the surface it is drawn from at both.
///
/// **And inclusive by [`ON_TOP`]**, which is the same nudge [`stand_clear`] gave
/// the point and has to be given back here. A pixel of a wall's top cap is at
/// exactly the wall's own `top`; moved a hair above it and asked without the
/// tolerance, it stopped being a point of its own wall and the wall shadowed it —
/// the room's own wall went dark at the one height its cap is drawn at.
///
/// `blit.wgsl`'s `on_surface`.
fn on_surface(z: f32, stands: &crate::occlusion::Solid) -> bool {
    z >= stands.bottom() as f32 - ON_TOP && z <= stands.top() as f32 + ON_TOP
}

/// A soft interval: `1.0` well inside `low..=high`, `0.0` well outside, and a
/// gradient `band` wide across each edge.
///
/// [`pierces`] with its one asymmetry taken out, and the asymmetry is why this
/// is a second function rather than a call of the first. A wall's *bottom* edge
/// is the ground it stands on and the ray a person looks at runs along it, so
/// that band hangs below rather than straddling. **A hole's edges are in the
/// middle of a surface** and no ray runs along them by construction, so a hole
/// softens the same amount in both directions or it is a hole that has been
/// moved half a penumbra downwards.
///
/// `blit.wgsl`'s `inside`, and the two are one formula.
fn inside(x: f32, low: f32, high: f32, band: f32) -> f32 {
    let band = band.max(1e-3);
    ((x - low).min(high - x) / band + 0.5).clamp(0.0, 1.0)
}

/// Where along a panel's own run a point of it lies, `0.0` to `1.0` across the
/// tile.
///
/// A panel on a north or south side lies in a plane of constant `y`, so what
/// runs along it is `x`; an east or west one is the other way round. That is the
/// whole of the surface's own coordinate system, and it is why an
/// [`Aperture`](crate::occlusion::Aperture) belongs only to a *named* panel: a
/// lid and a body have no run for this to be measured along.
///
/// `blit.wgsl`'s `run_v`.
fn run_v(edges: u8, px: f32, py: f32) -> f32 {
    let along = match edges & (crate::occlusion::EDGE_NORTH | crate::occlusion::EDGE_SOUTH) != 0 {
        true => px,
        false => py,
    };
    along - along.floor()
}

/// How much of a surface is **missing** where a ray goes through it: `1.0` well
/// inside the hole, `0.0` well outside it, and the same penumbra across its
/// edges that the top of a wall gets.
///
/// The two spans are combined with `min` and not with a product, so that the
/// corner of a hole is softened once rather than twice: a point that is halfway
/// into the hole across *and* halfway up is on the diagonal of one corner, and
/// two halves multiplied would make it a quarter of a hole.
///
/// `blit.wgsl`'s `hole`.
fn hole(aperture: Option<crate::occlusion::Aperture>, v: f32, z: f32, wide: f32, tall: f32) -> f32 {
    let Some(hole) = aperture else {
        return 0.0;
    };
    let across = inside(
        v,
        f32::from(hole.near) / crate::occlusion::RUN_STEPS,
        f32::from(hole.far) / crate::occlusion::RUN_STEPS,
        wide,
    );
    across.min(inside(z, hole.bottom as f32, hole.top as f32, tall))
}

/// How much of a surface stands in the way at the point a ray goes through its
/// plane: the span it occupies, less the hole in it.
///
/// The whole of step 21.3 in one line, and the reason it is one line is decision
/// 30.7: a panel was already *pierced at a point* rather than travelled through,
/// so the point was already being computed and a window is what that point is
/// asked about. `(px, py, z)` is where the ray crosses; `wide` is the penumbra
/// in tiles along the run and `tall` the same number in `z`.
///
/// `blit.wgsl`'s `pierced`.
fn pierced(stands: &crate::occlusion::Solid, px: f32, py: f32, z: f32, wide: f32, tall: f32) -> f32 {
    let across = pierces(z, stands.bottom() as f32, stands.top() as f32, tall);
    match stands.aperture {
        None => across,
        Some(_) => across * (1.0 - hole(stands.aperture, run_v(stands.edges, px, py), z, wide, tall)),
    }
}

/// How near two boundaries have to be, along the ray, for the ray to be crossing
/// a **corner** rather than a side — [`crate::occlusion::PANEL_THICKNESS`]
/// converted into the same `t` the DDA already steps in. `blit.wgsl`'s
/// `corner_tie`, and the two must answer the same comparison identically.
///
/// **Step 23.5, and derived rather than invented.** Before a panel had real
/// depth this was a bare float tolerance — "a thousand times the last bits of
/// a float", with no claim to be the width of anything — because there was no
/// geometry to size it from. Now two panels meeting at a corner physically
/// overlap in a `PANEL_THICKNESS`-wide square around the point they share, and
/// this is that overlap's width, in the walk's own units, not a number that
/// happens to work.
///
/// **The derivation, exactly, not approximately.** At `t = next` — the instant
/// the *nearer* axis reaches its grid line — the *farther* axis's coordinate
/// is still `|delta[far]| * (boundary[far] - next)` short of its own line,
/// because both coordinates are linear in `t` and each agrees with the grid
/// exactly at its own `boundary`. So the ray's distance from the corner point,
/// along the axis it has not yet crossed, is `|delta[far]| * |boundary[0] -
/// boundary[1]|` — world units — and asking that to be at most
/// `PANEL_THICKNESS` is exactly `|boundary[0] - boundary[1]| <=
/// PANEL_THICKNESS * per_tile[far]`, since `per_tile[far]` is `t` per world
/// unit on that axis. Which axis is `far` is `out_by_x`, already known at the
/// one call site.
fn corner_tie(per_tile: [f32; 2], out_by_x: bool) -> f32 {
    crate::occlusion::PANEL_THICKNESS as f32
        * match out_by_x {
            true => per_tile[1],
            false => per_tile[0],
        }
}

/// How much one cell stops a ray that crosses the sides in `crossed` at height
/// `z`, where the cell is a panel. Zero for open ground, for a lid, and for a
/// panel on a side the ray does not go through.
///
/// Which of a cell's sides are **the same wall the lit end is part of**, and
/// therefore must not shadow it.
///
/// `blit.wgsl`'s `own_run` argues it: a wall's face lies *on* the panel it is the
/// face of, so a ray leaving a wall pixel along the wall grazes the panels of the
/// tiles either side of it, and whether that counts as a crossing is decided by
/// the last bits of a float. It drew a thin dark stroke down every tile seam of
/// any wall lit by a lamp standing near it. A run of wall is one surface and no
/// part of a surface shadows another part of it.
///
/// Only the panels on the *same line*: the same row for a north or south face,
/// the same column for an east or west one. A wall tile that also carries the
/// perpendicular face of a corner stops the ray on that face as it always did.
fn own_run(own: u8, cell: (i32, i32), first: (i32, i32)) -> u8 {
    let mut line = 0;
    if cell.1 == first.1 {
        line |= crate::occlusion::EDGE_NORTH | crate::occlusion::EDGE_SOUTH;
    }
    if cell.0 == first.0 {
        line |= crate::occlusion::EDGE_EAST | crate::occlusion::EDGE_WEST;
    }
    own & line
}

/// `blit.wgsl`'s `panel_stop`. Split out of [`walk_cells`] for the corner case,
/// which has two cells to ask about and no crossing length to speak of.
///
/// The **largest** of the cell's surfaces and not their product, for the reason
/// [`walk_cells`] takes the largest: two panels of one corner are two faces of
/// one wall, and a ray that crosses both has gone through one thing once.
fn panel_stop<'a>(
    stands: impl Iterator<Item = &'a crate::occlusion::Solid>,
    crossed: u8,
    at: [f32; 3],
    wide: f32,
    tall: f32,
) -> f32 {
    stands
        .filter(|stands| stands.edges != 0 && stands.edges & crossed != 0)
        .map(|stands| f32::from(stands.opacity) / 255.0 * pierced(stands, at[0], at[1], at[2], wide, tall))
        .fold(0.0, f32::max)
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
    /// The tile this is a point of — **not** `at.x.floor()`/`at.y.floor()`.
    /// A point legitimately sits on a tile's own far edge (a stair tread's
    /// outer corner, `at.x` exactly whole) and `floor()` there picks whichever
    /// side happens to round down, not the side the geometry actually stands
    /// on. Every caller already knows which tile it means; carrying it here
    /// instead of re-deriving it in [`walk_cells`] is the CPU twin of
    /// `MeshFaceVertex::tile`'s fix to the same class of bug on the GPU side.
    /// `docs/lighting_raymarch.md` step 2.
    pub tile: (i32, i32),
    /// What surface of the world this is a point of.
    ///
    /// The polygon and not the tile: which way it looks, and therefore which
    /// flames can light it and which parts of its own tile can shadow it. It is
    /// what the place attachment's stance carries, per pixel, after
    /// `statics.wgsl` has resolved a corner to the face of the half the fragment
    /// is on. See [`Surface`].
    pub surface: Surface,
}

impl Spot {
    /// A point of something that stands up and names no side: a tree, a body, a
    /// wall whose art the detector would not read.
    ///
    /// The neutral answer, and what every caller that predates surfaces means:
    /// nothing is known about which way it looks, so every flame that reaches it
    /// lights it. See [`Spot::flat`] and [`Spot::face`] for the two that do know.
    pub fn at(at: Vec2, z: f32, tile: (i32, i32)) -> Self {
        Self {
            at,
            z,
            tile,
            surface: Surface::Upright,
        }
    }

    /// A point of the ground, a floor, a rug, or the top of a wall: a surface
    /// lying in its tile, looking up.
    pub fn flat(at: Vec2, z: f32, tile: (i32, i32)) -> Self {
        Self {
            at,
            z,
            tile,
            surface: Surface::Flat,
        }
    }

    /// A point of one of a tile's four vertical faces.
    pub fn face(at: Vec2, z: f32, tile: (i32, i32), face: Face) -> Self {
        Self {
            at,
            z,
            tile,
            surface: Surface::Face(face),
        }
    }
}

/// What kind of surface a lit point is a point of — the whole of what the
/// lighting asks about a pixel beyond where it is.
///
/// [`crate::place::Stance`] is the same question at the other end of the wire —
/// a corner is resolved to one of its two faces per fragment before the
/// attachment is written, and `docs/gbuffer.md` step 4c gave a mesh face
/// (a tread's top or riser) its own honest tag from this same set besides, so
/// what arrives here is always one of these four fixed normals, never a
/// computed one. `docs/lighting.md` decision 40 tried carrying a fifth,
/// computed case here (`Sloped`, a blended tread normal) before honest
/// per-face geometry existed to make it unnecessary; `docs/gbuffer.md` step 5
/// retired it once measuring against decision 40's own reproduction showed
/// the blend was compensating for a fake continuous-ramp sampling of the
/// flight, not for anything the real, decomposed geometry still needed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Surface {
    /// Standing up with nothing known about which way it runs.
    Upright,
    /// Lying in its tile, looking up: the ground, a floor, a rug, the lid on top
    /// of a wall.
    Flat,
    /// One of the tile's four vertical faces.
    Face(Face),
}

impl Surface {
    /// Which way this surface looks, in tiles — `x` and `y` across the map and
    /// `z` in tiles as well, which is the space a flame's offset is stated in.
    ///
    /// `None` for [`Surface::Upright`], which is a statement about what is *not*
    /// known: a billboard has no side, so every flame that reaches it lights it.
    ///
    /// A **flat** surface looks up, and that is the one this had missing. A wall's
    /// top cap is a flat static, so nothing tested which way it looked and a lamp
    /// standing beside a wall lit its top as fully as one standing over it —
    /// reported from the client as two walls "adding up" at a corner, and it is a
    /// bright diamond where a corner's cap is. `docs/lighting.md`, decision 27.
    pub fn normal(self) -> Option<[f32; 3]> {
        match self {
            Self::Upright => None,
            Self::Flat => Some([0.0, 0.0, 1.0]),
            Self::Face(face) => {
                let [x, y] = face.outward();
                Some([x, y, 0.0])
            }
        }
    }

    /// The face this is, where it is one.
    pub fn face(self) -> Option<Face> {
        match self {
            Self::Face(face) => Some(face),
            _ => None,
        }
    }

    /// What this surface's own tile may shadow it with.
    ///
    /// **Neither end of a ray is shadowed by the tile it is on** was the rule, and
    /// it is a rule about a *tile* where every other rule here is now about a
    /// surface. What it is really saying is that a surface does not shadow
    /// itself: a wall's face lies *on* the panel it is the face of, so the panel
    /// cannot be between it and anything; a tree's pixels are inside its own tile.
    ///
    /// A **floor** pixel on the same tile is not ambiguous in that way at all. It
    /// is the ground, it is inside the room, and a ray from it to a lamp in the
    /// street crosses the panel its own tile stands on — so the corner tile's own
    /// square of floor came out fully lit against a dark room. Which is the seam
    /// on the ground the corner report ended with.
    ///
    /// Only a **named** panel, though: a mask of all four is "it stands up and the
    /// art would not say", which is every tree, post and barrel, and testing those
    /// would put a dark square under each of them out of a fallback rather than
    /// out of a measurement. An unread graphic keeps today's behaviour exactly,
    /// which is the direction this file takes at every one of these forks.
    fn shadowed_by_own_tile(self, edges: u8) -> u8 {
        match self {
            Self::Flat if edges != crate::occlusion::EDGE_ANY => edges,
            _ => 0,
        }
    }
}

/// How wide the band is, in tiles, over which a flame passing behind a face
/// stops lighting it. `blit.wgsl`'s `FACE_EDGE`, and the two are one number.
///
/// Not a step, for the reason a beam's rim is not one: a hard edge is what the
/// eye finds first, and a lamp walking past the end of a wall would switch its
/// face off between two frames.
const FACE_EDGE: f32 = 0.2;

/// How much of a flame `toward` reaches a surface facing `normal` — `1.0` in
/// front of it, `0.0` behind it, and a gradient [`FACE_EDGE`] wide across the
/// plane. Both in tiles, `z` included: a horizontal surface is a surface, and
/// what decides for it is how far *above* its plane the flame is.
///
/// A half-space test and deliberately not a cosine. UO's art is pre-shaded —
/// every wall's picture already has a light in it — so a Lambert term would be a
/// second light fighting the first. What this answers is only whether the flame
/// is on the side the surface looks at.
///
/// `blit.wgsl`'s `faces`, and the two are one formula.
fn faces(normal: [f32; 3], toward: [f32; 3]) -> f32 {
    let along = normal[0] * toward[0] + normal[1] * toward[1] + normal[2] * toward[2];
    (along / FACE_EDGE + 0.5).clamp(0.0, 1.0)
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
    /// How much of the flame's [`Beam`] falls here, **and how much of it the
    /// surface is turned towards**: `1.0` for a fire that lights every direction
    /// falling on a floor, [`BEAM_SPILL`] for a spot behind a carried lantern, and
    /// `0.0` for a wall's face with the flame behind it.
    ///
    /// The two are one number because they are the same question asked of the two
    /// ends — which way the light points, and which way the surface looks — and a
    /// dark tile that is neither is what the shadow term is for.
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
        .sky_at(spot.tile.0, spot.tile.1));
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
        // And whether the flame is on the side this surface looks at — geometry
        // and nothing else. `blit.wgsl` argues why there is no longer an
        // exemption for a flame standing in the wall's own line, and
        // [`mounted_at`] is what replaced it.
        let facing = match spot.surface.normal() {
            None => 1.0,
            Some(normal) => faces(normal, offset),
        };
        let (through, stopped_by) = walk(spot, light, &lighting.occlusion);
        let fall = 1.0 - d;
        let added = light
            .color
            .map(|channel| channel * light.intensity * fall * fall * through * cone * facing);
        for (total, channel) in multiplier.iter_mut().zip(added) {
            *total += channel;
        }
        reaches.push(Reach {
            light: index,
            distance,
            within: true,
            through,
            cone: cone * facing,
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
    walk_cells(from, to, spot.surface, spot.tile, false, 0.0, occlusion)
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
        spot.surface,
        spot.tile,
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
    surface: Surface,
    tile: (i32, i32),
    skip_last: bool,
    spread: f32,
    occlusion: &Occlusion,
) -> (f32, Option<(i32, i32)>) {
    // **Both ends of the ray stand where they are drawn, not inside it.** See
    // [`STAND_OFF`] and [`ON_TOP`]: a face pixel is walked from a hair in front
    // of the plane it is the face of, and every point of the world from a hair
    // above whatever it is lying on. Without the pair, a wall pixel at exactly a
    // floor's `z` was lit by the room below it — the bright line along the floor
    // of a house, four screen pixels of it, and neither nudge closes it alone.
    let (from, to) = stand_clear(from, to, surface);
    let spot = Spot {
        at: Vec2::new(from[0], from[1]),
        z: from[2],
        tile,
        surface,
    };
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let ground = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    // The tile the caller carried in, not `from.floor()`: `from` can
    // legitimately sit exactly on this tile's own far edge (`stand_clear`'s
    // nudge is sub-tile, and a flat surface gets no nudge at all), and
    // flooring it back would pick whichever side happens to round down
    // rather than the side the geometry actually stands on. See [`Spot::tile`].
    let first = tile;
    if ground < 1e-6 {
        // Straight up or down. There is no direction to walk in and there never
        // was — but "the only cells on the line are the exempt ones", which is
        // what this used to say, stopped being true at decision 32: **a lid on
        // that one cell is between the two ends of the ray**, and a vertical ray
        // is the case a floor is most obviously in the way of. A torch on the
        // ground floor and the plank directly over it were the shortcut's own
        // counterexample.
        //
        // Only lids, and for the reason both ends exempt everything else: a panel
        // stands beside a vertical ray rather than across it, and a body is a
        // solid one of the ends is standing in.
        let stopped = occlusion
            .solids_at(first.0, first.1)
            .filter(|stands| stands.edges == 0)
            .map(|stands| {
                f32::from(stands.opacity) / 255.0
                    * crosses(
                        from[2],
                        to[2],
                        stands.bottom() as f32,
                        stands.top() as f32,
                        to[2],
                        spread,
                    )
            })
            .fold(0.0, f32::max);
        return match stopped >= 1.0 - RAY_CUTOFF {
            true => (0.0, Some(first)),
            false => (1.0 - stopped, None),
        };
    }
    let last = (to[0].floor() as i32, to[1].floor() as i32);
    let mut cell = first;
    // Which side of its own tile the lit end **is the face of** — see [`own_run`],
    // and the seam it was drawing down every wall. The pixel's own side and not
    // the tile's whole mask, which is what decision 23 says in words: a run of
    // wall is one surface, and a corner's *perpendicular* panel is a different
    // surface that stops the ray as it always did. A pixel that is not a face is
    // part of no run and gets nothing.
    let own = match surface.face() {
        Some(face) => crate::occlusion::edges_of(Some(crate::facing::Facing::One(face))),
        None => 0,
    };
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
        // The known tile's own edge, not `from.floor()` — the same reason
        // `first` reads `tile` above. A `from` sitting exactly on this axis'
        // boundary must seed `boundary[axis]` near zero (the ray is already
        // leaving `first`), and `from.floor()` there can just as well pick
        // the far side and seed a whole tile of slack that was never there.
        let edge = [tile.0, tile.1][axis] as f32;
        let ahead = match delta[axis] >= 0.0 {
            true => edge + 1.0 - from,
            false => from - edge,
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
        let stands = occlusion.ids_at(cell.0, cell.1);
        let own_cell = cell == first;
        // The union of the sides the tile's surfaces stand on. A *tile's* answer
        // and deliberately so: what a pixel is exempted from below is the tile it
        // is on and not one surface of it.
        //
        // Only on the ray's own cell, which is the only place it is read: every
        // other cell of the walk would be paying for a second pass over its
        // surfaces to answer a question about a pixel that is not on it.
        let sides = match own_cell {
            false => 0,
            true => stands
                .iter()
                .fold(0, |mask, id| mask | occlusion.solid(*id).edges),
        };
        // **A surface does not shadow itself**, which is what "neither end of the
        // ray is shadowed by the tile it is on" was always reaching for. The
        // flame's end is a whole tile — a sconce burns outside the plane its tile
        // names (`mounted_at`) and what is left on that tile is not between it and
        // anything. The lit end is a *surface*: a face lies on its own panel and a
        // billboard's pixels are inside their tile, but a **floor** pixel on a wall
        // tile is inside the room, and the ray from it to a lamp in the street
        // crosses the panel its own tile stands on. See
        // [`Surface::shadowed_by_own_tile`], and `blit.wgsl`'s `walk`.
        let lit_by_own_tile = match own_cell {
            false => 0,
            true => surface.shadowed_by_own_tile(sides),
        };
        // **Neither exemption is about a lid**, and that is decision 32 arriving
        // at the two ends of the ray. Both of them are statements about things
        // that *stand up*: a pixel lies on the panel it is the face of, a
        // billboard's pixels are inside their own tile, and a mounted flame burns
        // outside the plane its tile names. None of that is true of a horizontal
        // plane — the floor over a torch's tile really is between it and
        // everything above, and the floor over a pixel's tile really is between
        // it and everything below. It is exactly the case a house has: a sconce
        // at `z 36` and the upper storey's wall at `z 45` are the *same tile*
        // with the floor at `z 40` in between, and both ends exempted it.
        //
        // It costs nothing where it should: a ray only crosses a plane inside its
        // own cell when the other end is nearly straight above or below, so a
        // lamp in the street still lights the outward face of the storey over it
        // — that ray leaves the cell within a fraction of a tile and crosses the
        // floor somewhere else, or nowhere.
        if !stands.is_empty() {
            // How soft this cell's own edge is: the penumbra a source of
            // this size casts from this far along the ray. See
            // [`FLAME_SPREAD`]; a `spread` of zero is a point source, and the
            // clamp leaves it the narrowest edge the walk draws.
            let middle = (entered + leaves) * 0.5;
            let soft =
                (spread * middle / (1.0 - middle).max(1e-3)).clamp(SOFT_CROSSING_MIN, SOFT_CROSSING_MAX);
            // What the *cell* stops is the **largest** of what its surfaces stop,
            // and not the product of them. Two panels on one tile are a corner —
            // two faces of one wall — and a ray that crosses both has gone
            // through one thing once; multiplying would darken every corner in
            // the world twice over. Since step 21.2 a tile's surfaces carry
            // spans of their own — a lid and a wall no longer share one — so the
            // largest is also what keeps the air between two walls open where
            // the merged span used to close it.
            let mut stopped: f32 = 0.0;
            for stands in stands.iter().map(|id| occlusion.solid(*id)) {
                // On either end's own cell only a lid is asked — see `lids_only`
                // above — and on the lit end's, only the panels the surface
                // admits beside it: a pixel standing inside a solid is not
                // shadowed by the solid it stands in.
                //
                // **And an exemption reaches only as high as the surface it is
                // about.** A tile of a two-storey house carries a wall for each
                // storey — `0..20` and `20..40` — and they are two surfaces. A
                // pixel at `z 25` lies on the upper one; the lower one is under
                // its feet and is as much an occluder to it as anybody else's
                // wall. Exempting it let every ray out of the room below climb
                // the column of its own wall tile, which is the leak that
                // survived the two above: the floor stops at the wall, so the
                // storey's wall was the one tile with no plank over the room.
                // `on_surface` is the whole of the fix, and it is decision 28
                // said with the `z` it never had.
                // And which of this cell's sides are the **same run of wall**
                // the lit end is part of, which is [`own_run`] with the same `z`
                // test on it: a run is one surface, and the storey below is
                // another one that happens to be under it.
                let same_run = match on_surface(spot.z, stands) {
                    true => own_run(own, cell, first),
                    false => 0,
                };
                let lit_end = own_cell && on_surface(spot.z, stands);
                let flame_end = skip_last && cell == last && on_surface(to[2], stands);
                // **A flat surface at or above a panel's own top is its cap, not a
                // floor the panel rises through.** `shadowed_by_own_tile` reads
                // every flat pixel on a named-edge tile as the room floor decision
                // 28 was written for, and a tread's own top proves that wrong: it
                // sits at exactly its riser's `top()`, nothing of the riser stands
                // *above* that height, and the pixel is standing on the panel the
                // same way a face does — not behind it. A genuine floor with a wall
                // rising past it stays caught, because its `z` is at the panel's
                // `bottom()` and this is false there. `blit.wgsl`'s `walk`.
                let caps_this = surface == Surface::Flat && lit_end && spot.z >= stands.top() as f32 - ON_TOP;
                if stands.edges != 0
                    && ((lit_end && stands.edges & lit_by_own_tile == 0) || flame_end || caps_this)
                {
                    continue;
                }
                let (low, high) = (stands.bottom() as f32, stands.top() as f32);
                let opacity = f32::from(stands.opacity) / 255.0;
                let by_surface = match stands.edges {
                    // A **lid** — a floor, a rug, a plank — is a *plane*, and what
                    // a plane does to a ray is decided by whether the ray got to
                    // the other side of it inside this cell. See [`crosses`], and
                    // `blit.wgsl`'s `walk` for the same three lines.
                    0 => {
                        let from = spot.z + delta[2] * entered;
                        let to = spot.z + delta[2] * leaves;
                        opacity * crosses(from, to, low, high, spot.z + delta[2], spread)
                    }
                    // A **body** — a whole tile that stands up and whose art would
                    // not say which way. A solid the ray travels through, so what
                    // it stops is scaled by the length of the run inside the span.
                    // `blit.wgsl`'s `walk` argues why all four sides belongs here:
                    // a roof five `z` deep is pierced by neither of its sides at
                    // 45°, and a sealed house came out sunlit.
                    EDGE_ANY => {
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
                        let travelled = opacity * (crossed / soft).clamp(0.0, 1.0);
                        // And a thing that **stands up** is a surface on every
                        // side of its tile as well as a solid inside it, so the
                        // sides it is crossed by are pierced too — see
                        // `blit.wgsl`'s `walk`, and decision 24. The larger of the
                        // two answers: the length is what keeps a slab five `z`
                        // deep opaque to a ray that goes over neither of its
                        // sides, and the pierce is what closes the sliver a ray
                        // clipping a corner used to walk through.
                        //
                        let tall = soft * FLAME_DEPTH;
                        let stops = EDGE_ANY & !same_run;
                        let mut stopped = travelled;
                        for (side, at) in [(entry, entered), (exit, leaves)] {
                            if stops & side != 0 {
                                stopped =
                                    stopped.max(opacity * pierces(spot.z + delta[2] * at, low, high, tall));
                            }
                        }
                        stopped
                    }
                    // A **panel** — a surface on one side of the tile. What it does
                    // to a ray is decided where the ray *pierces* it, at a point
                    // and at a height, and not by how long the ray spent in the
                    // cell. `blit.wgsl`'s `walk` argues it: the length version let
                    // a ray through the corner between two panels and drew a fan
                    // of spokes out of every lamp near a wall.
                    edges => {
                        let tall = soft * FLAME_DEPTH;
                        // Less whatever of this cell is the same run of wall the
                        // lit end stands in — see [`own_run`] and `same_run`.
                        let stops = edges & !same_run;
                        let mut stopped: f32 = 0.0;
                        for (side, at) in [(entry, entered), (exit, leaves)] {
                            if stops & side != 0 {
                                // Where the ray goes through the plane, in all
                                // three, because a **hole** is a rectangle in it
                                // and the height alone cannot say whether the ray
                                // went through the window or through the wall
                                // beside it. See [`pierced`].
                                let cross = [
                                    spot.at.x + delta[0] * at,
                                    spot.at.y + delta[1] * at,
                                    spot.z + delta[2] * at,
                                ];
                                stopped = stopped
                                    .max(opacity * pierced(stands, cross[0], cross[1], cross[2], soft, tall));
                            }
                        }
                        stopped
                    }
                };
                stopped = stopped.max(by_surface);
            }
            through *= 1.0 - stopped;
            if through <= RAY_CUTOFF {
                return (0.0, Some(cell));
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
        if (boundary[0] - boundary[1]).abs() <= corner_tie(per_tile, out_by_x) {
            // **A corner** — `blit.wgsl`'s `walk` argues it: four tiles meet at
            // the point the ray leaves by, and the two the walk does not step
            // into are as much in the way as the one it does. Both are asked, and
            // then the walk steps diagonally past them.
            let by_x = (cell.0 + toward.0, cell.1);
            let by_y = (cell.0, cell.1 + toward.1);
            let cross = [
                spot.at.x + delta[0] * next,
                spot.at.y + delta[1] * next,
                spot.z + delta[2] * next,
            ];
            let wide = (spread * next / (1.0 - next).max(1e-3)).clamp(SOFT_CROSSING_MIN, SOFT_CROSSING_MAX);
            let tall = wide * FLAME_DEPTH;
            let mut corner: f32 = 0.0;
            let mut blamed = None;
            for (at, crossed) in [
                (by_x, enter_x | crate::occlusion::opposite(enter_y)),
                (by_y, enter_y | crate::occlusion::opposite(enter_x)),
            ] {
                if at == first || (skip_last && at == last) {
                    continue;
                }
                let crossed = crossed & !own_run(own, at, first);
                let stops = panel_stop(occlusion.solids_at(at.0, at.1), crossed, cross, wide, tall);
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

    /// A lid stops a ray that goes through it and nothing that runs along it.
    ///
    /// The three cases [`crosses`] exists for, on a floor of the height a real
    /// one has — **zero** — which is the number the rule it replaced could not
    /// answer for. A candle standing on a floor and the floor it lights are at
    /// one `z`, so the ray between them runs exactly along the plane; a test
    /// that only asked about the crossing would pass with a rule that laid half
    /// a floor's shadow across every room lit from inside it.
    ///
    /// The flame's own `z` is the fourth argument, and it is what softens the
    /// answer: a torch a storey below the floor is wholly under it, and what
    /// comes through is nothing.
    #[test]
    fn a_floor_stops_a_ray_through_it_and_not_one_along_it() {
        // A ray from a wall pixel at 25 down to a torch at 5, crossing this
        // cell between 22 and 18: through the floor at 20.
        assert_eq!(crosses(22.0, 18.0, 20.0, 20.0, 5.0, FLAME_SPREAD), 1.0);
        // The same floor, and a flame standing on it: the ray runs along the
        // plane and has gone through nothing.
        assert_eq!(crosses(20.0, 20.0, 20.0, 20.0, 20.0, FLAME_SPREAD), 0.0);
        // And a ray wholly above it — a lamp on the upper storey lighting the
        // upper storey — is not touched by the floor under both of them.
        assert_eq!(crosses(25.0, 23.0, 20.0, 20.0, 26.0, FLAME_SPREAD), 0.0);
        // A flame *in* the plane of the lid is half cut by it: it is a body
        // about a tile across, and half of it is on either side.
        assert!((crosses(22.0, 18.0, 20.0, 20.0, 20.0, FLAME_SPREAD) - 0.5).abs() < 1e-6);
    }

    /// `corner_tie`'s own claim, checked against the arithmetic rather than
    /// trusted from it: the width it returns converts back into exactly
    /// [`crate::occlusion::PANEL_THICKNESS`] of world distance on the axis
    /// `out_by_x` says has *not* been crossed yet.
    #[test]
    fn corner_tie_converts_back_into_exactly_one_panel_thickness_of_world_distance() {
        // A ray moving twice as fast along `x` as along `y`, so the two axes'
        // `per_tile` genuinely differ and a swapped index would be caught
        // rather than cancel out.
        let delta = [2.0_f32, 1.0];
        let per_tile = [1.0 / delta[0].abs(), 1.0 / delta[1].abs()];

        // `x` reached first: the axis not yet crossed is `y`, so the tie
        // should be exactly `PANEL_THICKNESS` of world distance along `y`.
        let along_y = delta[1].abs() * corner_tie(per_tile, true);
        assert!(
            (along_y - crate::occlusion::PANEL_THICKNESS as f32).abs() < 1e-6,
            "out_by_x should size the tie off the y axis, it converts to {along_y}",
        );
        // And the other way round.
        let along_x = delta[0].abs() * corner_tie(per_tile, false);
        assert!(
            (along_x - crate::occlusion::PANEL_THICKNESS as f32).abs() < 1e-6,
            "out_by_y should size the tie off the x axis, it converts to {along_x}",
        );
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
            None,
        );
        assert!(lighting.lights.is_empty());
    }

    /// **A tread's own top must not be shadowed by the riser it caps.**
    ///
    /// `occlusion::Builder::add`'s climbable branch gives a tread's top the
    /// `Stance::Flat` normal, which is the same one a room's floor gets, and
    /// [`Surface::shadowed_by_own_tile`] was written for that floor: "a floor
    /// pixel on a wall tile is inside the room, and the ray from it to a lamp in
    /// the street crosses the panel its own tile stands on." A tread's top sits
    /// at the exact height its own riser stops at — `top_z == riser.top()` — so
    /// the same rule reads it as a floor the riser walls in, even though the
    /// riser has nothing standing *above* that height to be between the pixel
    /// and anything. Found looking at a real staircase render: every tread top
    /// read dark towards its own riser regardless of where the torch stood.
    #[test]
    fn a_treads_top_is_not_shadowed_by_its_own_riser() {
        use crate::facing::Prism;
        use crate::occlusion::{Builder, Shape};

        let stair = StaticTile {
            flags: TileFlags::new(TileFlags::NO_SHOOT | TileFlags::CLIMBABLE),
            height: 20,
            ..StaticTile::default()
        };
        let prism = Prism::new(Face::West, &[1, 3, 5]).expect("three treads");
        let mut occlusion = Builder::new(crate::camera::TileBounds {
            min_x: 95,
            max_x: 105,
            min_y: 95,
            max_y: 105,
        });
        occlusion.add(100, 100, 0, Graphic(0x0736), &stair, Shape::solid(prism));
        let occlusion = occlusion.finish(&Cutaway::OPEN);

        // The highest tread's own top, off the built grid rather than
        // recomputed: `footprint`'s own fractions are not this test's subject.
        let top = occlusion
            .solids_at(100, 100)
            .filter(|solid| solid.edges == 0)
            .max_by_key(|solid| solid.top())
            .expect("the climb built three tops");
        let at = Vec2::new(
            ((top.space.min.x + top.space.max.x) / 2.0) as f32,
            ((top.space.min.y + top.space.max.y) / 2.0) as f32,
        );
        let spot = Spot::flat(at, top.top() as f32, (100, 100));

        // East of the stair, level with the top tread — the foot of the flight,
        // which is where a person actually stands a torch.
        let light = Light {
            at: Vec2::new(102.5, 100.5),
            z: top.top() as f32,
            radius: 6.0,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            beam: None,
        };
        let lighting = Lighting {
            ambient: NIGHT,
            lights: vec![light],
            occlusion,
            sun: None,
            view: crate::debug::View::default(),
        };

        let sample = sample(spot, &lighting);
        assert!(
            sample.reaches[0].through > 0.9,
            "a tread's own top should not be dimmed by the riser it caps: through {}",
            sample.reaches[0].through,
        );
    }
}
