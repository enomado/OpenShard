//! What stands between a flame and the ground it would light.
//!
//! A list of the surfaces this frame's flames can reach, and a grid over the
//! same tiles as its index: a cell is `(offset, count)` into the list, and a
//! surface says how much of a ray crossing it survives and between which
//! heights. [`crate::light`] hands both to the blit, which walks the cells
//! between a fragment and each flame — see `docs/lighting.md`, decisions 3
//! through 6, and decision 30 for why the cell is a list rather than one merged
//! span.
//!
//! Nothing appends to an [`Occlusion`]. It is built by a [`Builder`], which is
//! where a tile's occluders are merged, and packed by [`Builder::finish`] — a
//! tile's surfaces have to be contiguous for an `(offset, count)` to name them,
//! and they cannot be while anything can still be added.
//!
//! # Why a tile and not a wall's edge
//!
//! A wall stands on one edge of its tile, and **nothing in `tiledata.mul` says
//! which edge**: that is only in the shape of the sprite. So the occluder here
//! is the whole tile. It costs half a tile of reach at the wall, and it buys a
//! room whose wall tiles are a closed ring by construction — no corner of a
//! house leaks light into the street because two segments failed to meet.
//!
//! # What touches light is what stops an arrow — but not by as much
//!
//! `WINDOW | NO_SHOOT`, and not `BLOCK`. The two are different questions and the
//! reference keeps them apart: ServUO's `Map.LineOfSight` (`Server/Map.cs:3040`)
//! tests a static with `(flags & (TileFlag.Window | TileFlag.NoShoot)) != 0`
//! against the span `t.Z ..= t.Z + CalcHeight`, and impassability never enters
//! it. A barrel and a fence are `BLOCK` and you can see over both; a wall is
//! `NO_SHOOT` and you cannot see through it. Reading `BLOCK` instead would put a
//! shadow behind every crate on the street.
//!
//! Where this parts company with the reference is *how much* each stops. Line of
//! sight is a yes or a no, so a window is a wall in it; light is a fraction, and
//! a window is glass. So the grid carries an opacity byte — [`OPAQUE`] for a
//! wall, [`PANE`] for a window — and the shader multiplies by it either way.
//!
//! # Nothing occludes that was not drawn
//!
//! Every occluder is tested with the frame's [`Cutaway`], exactly as the flames
//! are. A shadow cast by a wall the cutaway took away is a dark band with
//! nothing in the picture making it, which is the worse bug of the two.
//!
//! # The sky a tile can see
//!
//! The grid answers a second question, and it is the cheapest one it can be
//! asked: *can this tile see straight up*. A tile that cannot does not get the
//! sky's share of the ambient — which is what makes the inside of a house darker
//! than the street outside it with nothing in either. `docs/lighting_world.md`,
//! decisions 1, 2, 3 and 14, and the field is [`Occlusion::sky_at`].
//!
//! Three things about it are not the shadow walk's answers, and each is a
//! decision rather than an accident:
//!
//! - **It ignores the [`Cutaway`].** Standing indoors deletes the roof so that
//!   the player can be seen; if the sky test read the *drawn* statics, walking
//!   through a door would flood the room with noon and the player would carry
//!   daylight into every building. A shadow from a static that is not in the
//!   picture is an artefact; the missing ambient of a roof the player walked
//!   under is the point. So this is the one reader of the walk that does not ask
//!   [`cutaway::shows`].
//! - **It is blurred by a tile.** A raw column test steps from 1 to 0 at the
//!   wall line, and a step is the artefact this whole track exists to remove.
//!   One 3x3 pass over a grid a few hundred tiles across makes the threshold of
//!   an open door brighter than the middle of the room and the eave of a roof
//!   brighter than what is under it. It is not a simulation of anything — it is
//!   the shape the right answer has, for one blur of a small array.
//! - **A pane passes its share.** The column multiplies by what each occluder
//!   leaves, so a glazed roof lets four fifths of the sky through where a slate
//!   one lets none. That is the crude half of decision 14, and it is what keeps
//!   a chapel from reading as a crypt until an aperture arrives.
//!
//! # One plane per answer, beside the cell and not inside it
//!
//! A [`Surface`] is four channels and all four are spoken for, so the sky needed
//! room. It gets a **third texture over the grid's rectangle** rather than a
//! wider surface — see [`Occlusion::field_bytes`], whose four channels are the
//! places the answers that are not about *stopping a ray* go: the sky today, an
//! aperture and a body's opacity when `docs/lighting.md`'s step 16 and
//! `docs/lighting_world.md`'s step 8 land. One decision for all three, which is
//! what the plans asked for, and the split is along the line that matters: a
//! surface is what a ray walks through, and this is what a *tile* is, read once
//! per fragment and never in a loop.
//!
//! So a frame uploads three: the index over the camera's rectangle, this field
//! over the same rectangle, and the surface list, whose length is what the
//! camera happens to be looking at rather than how big it is.

use openshard_protocol::wire::Graphic;

use crate::facing::{Face, Facing};
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::{StaticTile, TileData, TileFlags};

use crate::camera::TileBounds;
use crate::cutaway::{self, Cutaway};
use crate::items::GroundItem;

/// A tile that stops light entirely.
pub const OPAQUE: u8 = 255;

/// A tile light crosses untouched.
pub const CLEAR: u8 = 0;

/// Whether a static touches light at all — a wall or a pane, and not a barrel.
///
/// The reference's line of sight test, and it is still the right *membership*
/// question: what is in the grid is what stops an arrow. How much of the light
/// each member stops is [`opacity`]'s answer and no longer the same one. See
/// this module's header for why it is not `BLOCK`.
pub fn stops_light(tile: &StaticTile) -> bool {
    tile.flags.has(TileFlags::WINDOW | TileFlags::NO_SHOOT)
}

/// The four sides of a tile, as bits of a cell's fourth channel — and the
/// difference between an occluder that is a *tile* and one that is a *panel*.
///
/// `docs/lighting.md`'s decision 3 made an occluder a whole tile, because
/// `tiledata.mul` does not say which edge a wall stands on and guessing it from
/// the art was "a subsystem". Step 15 built that subsystem, so this is decision 3
/// revised: where [`crate::facing`] names an edge, the occluder is the panel on
/// that edge and a ray is stopped only where it **crosses** it. A ray running
/// alongside a wall passes, which is what a lamp mounted on a house needs in
/// order to light the street it hangs over.
///
/// A mask of zero is a **lid**: something horizontal, a floor or a roof, whose
/// occlusion is entirely its `z` span and which no vertical edge describes.
/// [`EDGE_ANY`] is "it stands up and nobody knows which way", which is exactly
/// the old whole-tile answer and therefore the safe fallback.
pub const EDGE_NORTH: u8 = 1;
/// The `x1` side. See [`EDGE_NORTH`].
pub const EDGE_EAST: u8 = 2;
/// The `y1` side. See [`EDGE_NORTH`].
pub const EDGE_SOUTH: u8 = 4;
/// The `x0` side. See [`EDGE_NORTH`].
pub const EDGE_WEST: u8 = 8;
/// All four: a thing that stands up whose facing the art would not name.
pub const EDGE_ANY: u8 = EDGE_NORTH | EDGE_EAST | EDGE_SOUTH | EDGE_WEST;
/// The bit that says a cell holds anything at all.
///
/// Separate from the mask because a lid's mask is legitimately zero, and the
/// shader tests presence before it tests edges. `bytes` writes `PRESENT | mask`
/// and `blit.wgsl` takes the two apart with the same constants.
pub const PRESENT: u8 = 0x80;

/// The side of the neighbouring tile that touches this one's `side`.
///
/// One line, and it is the whole of how a walk carries an edge across a
/// boundary: the line a ray crosses is one cell's east and the next one's west.
/// `blit.wgsl` has the same function and the parity test is what says so.
pub fn opposite(side: u8) -> u8 {
    match side {
        EDGE_NORTH => EDGE_SOUTH,
        EDGE_SOUTH => EDGE_NORTH,
        EDGE_EAST => EDGE_WEST,
        EDGE_WEST => EDGE_EAST,
        _ => 0,
    }
}

/// Which sides of its tile a static occupies, from what the art said about it.
///
/// `None` — a post, a tree, a graphic no atlas was offered — is [`EDGE_ANY`]:
/// the whole-tile answer, unchanged from before faces existed. A **corner** is
/// two bits, which is the panel path with two panels on it and not a new case —
/// see the `edges` arm of `light::walk_cells` and of `blit.wgsl`'s `walk`. A
/// [`Stance::Flat`](crate::place::Stance) static is not asked at all; see
/// [`Occlusion::add`].
///
/// Two bits and not four is the whole of what decision 25 buys the grid: a ray
/// running *alongside* a corner — down the street the corner stands on — crosses
/// neither of its two panels and passes, exactly as it does beside the runs of
/// wall either side of it, where before it was stopped by a whole-tile occluder.
pub fn edges_of(facing: Option<crate::facing::Facing>) -> u8 {
    let Some(facing) = facing else {
        return EDGE_ANY;
    };
    facing
        .faces()
        .map(|face| match face {
            Face::North => EDGE_NORTH,
            Face::East => EDGE_EAST,
            Face::South => EDGE_SOUTH,
            Face::West => EDGE_WEST,
        })
        .fold(0, |mask, side| mask | side)
}

/// How much of a ray crossing a pane of glass is stopped.
///
/// A fifth, which is a guess about glass and not a number from any file — the
/// client has none. What it is *not* is a guess about line of sight: an arrow is
/// stopped by a window and light is not, and `WINDOW` being in the same test as
/// `NO_SHOOT` in the reference is a fact about arrows. A window that stopped
/// light entirely is what makes a lit room read as a bunker, and it is the one
/// thing standing between a candle and the street it should be visible from.
pub const PANE: u8 = 51;

/// How much of a ray crossing this static is stopped, `0..=255`.
///
/// Three answers and not two: a wall stops everything, a pane dims, and
/// everything else — a barrel, a fence, a crate — passes light untouched even
/// where it stops an arrow. The byte was always here for this; what changed is
/// that `WINDOW` no longer borrows `NO_SHOOT`'s answer.
///
/// `NO_SHOOT` wins where a tile carries both. A shard's custom static that is
/// flagged as a solid window is more likely to be a shuttered one than a
/// transparent wall, and the union is the conservative direction — darkening is
/// visible, leaking a room into the street is a bug.
///
/// # Why the graphic, when the flags are right here
///
/// Because an **open door** has the flags of a shut one. `tiledata.mul` gives a
/// door's two leaves identical entries — measured over all 104 of ServUO's
/// open/shut pairs — so a door left to its flags lays a whole tile of wall
/// across its own doorway, which decision 3 makes the coarsest possible wrong
/// answer. [`crate::doors`] is the table that knows, and this is where it is
/// asked, before anything else: a leaf that has swung open stops nothing.
///
/// Which is the general shape and not a door-shaped patch. A flag is a fact
/// about a *picture*, and anything that opens, lifts or breaks is a fact about
/// the *thing*: a shutter, a portcullis, a drawbridge are all this question
/// again. So the argument is the graphic, and the flags are what it falls back
/// on.
pub fn opacity(graphic: Graphic, tile: &StaticTile) -> u8 {
    if crate::doors::is_open(graphic) {
        return CLEAR;
    }
    if tile.flags.has(TileFlags::NO_SHOOT) {
        return OPAQUE;
    }
    match tile.flags.has(TileFlags::WINDOW) {
        true => PANE,
        false => CLEAR,
    }
}

/// How tall a static stands, for the purpose of what it hides.
///
/// ServUO's `ItemData.CalcHeight` (`Server/TileData.cs:112`): a climbable
/// (`Bridge`) tile counts as half its stated height, because that is the height
/// you end up standing at on it. `movement`'s `platform_surface` halves the same
/// number for the same reason.
fn calc_height(tile: &StaticTile) -> i32 {
    let height = i32::from(tile.height);
    match tile.flags.is_climbable() {
        true => height / 2,
        false => height,
    }
}

/// One surface something stands on a tile: a plane, the heights it occupies,
/// and how much of a ray crossing it survives.
///
/// The element of the list a cell indexes — `docs/lighting.md`'s decision 30 —
/// and **the walk's two rules are its two kinds**: [`Surface::edges`] naming one
/// side is a *panel*, a ray is stopped where it crosses it; zero is a *lid* and
/// all four a *body*, and a ray is stopped by how far it ran inside the span.
///
/// It carries the same four fields [`Cell`] does and that is not an accident:
/// a cell is what the surfaces of one tile merge to, and the merge is what step
/// 21.2 takes apart. Until then a tile's surfaces all share its span and its
/// opacity, which is what makes this step a change of storage and nothing else.
///
/// The span is in `z` units — the map's own, not pixels — and it is inclusive of
/// `bottom` and `top`. A wall based at `z = 0` and 20 tall stops a ray passing
/// through `0..=20` and no other, which is what keeps a cellar's wall out of the
/// street and an upper storey's out of the ground floor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Surface {
    /// The lowest `z` this surface stops anything at.
    pub bottom: i32,
    /// The highest.
    pub top: i32,
    /// How much of a ray crossing it is stopped.
    pub opacity: u8,
    /// Which side of the tile it stands on: one of [`EDGE_NORTH`],
    /// [`EDGE_EAST`], [`EDGE_SOUTH`], [`EDGE_WEST`] for a panel, `0` for a lid,
    /// or [`EDGE_ANY`] for a body. Never two named sides — a corner is two
    /// panels, which is what the list is for.
    pub edges: u8,
}

/// One tile's worth of occlusion: how much it stops, and between which heights.
///
/// **The merged view**, and no longer what is stored: the union of everything on
/// the tile, folded out of [`Occlusion::surfaces_at`] for the readers whose
/// question is about a *tile* rather than about a surface — the wireframe
/// overlay, the plan view, the mounted flame's own cell. The walk does not ask
/// it any more.
///
/// The span is in `z` units — the map's own, not pixels — and it is inclusive of
/// `bottom` and `top`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    /// The lowest `z` this tile stops anything at.
    pub bottom: i32,
    /// The highest.
    pub top: i32,
    /// How much of a ray crossing the span is stopped.
    pub opacity: u8,
    /// Which sides of the tile the things standing here occupy — the union over
    /// all of them. A ray is stopped only where it crosses one of these.
    ///
    /// Zero is a **lid** and not "nothing": something horizontal, whose whole
    /// occlusion is the `z` span above. [`EDGE_ANY`] is the old whole-tile
    /// answer and what an unreadable static gets. See [`EDGE_NORTH`].
    pub edges: u8,
}

/// A tile with nothing at all over it: the whole of the sky.
pub const SKY_OPEN: u8 = 255;

/// Where one tile's surfaces are: the index `docs/lighting.md`'s decision 30.3
/// keeps the tile grid as.
///
/// A count of zero is open ground, and the offset is then meaningless — a caller
/// reads [`Occlusion::surfaces_at`], which hands back an empty slice for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Span {
    /// Where this tile's run begins in [`Occlusion::surfaces`]. Twenty-four bits
    /// of it survive the upload — see [`Occlusion::bytes`].
    offset: u32,
    /// How many surfaces stand on the tile. One byte, for the same reason.
    count: u8,
}

/// How wide the surface list is as a texture, in texels.
///
/// The list is one dimensional and a texture is not, so it is folded into rows
/// of this — `blit.wgsl`'s `SURFACE_ROW`, and the two are one number. A thousand
/// and twenty-four rather than the 2048 WebGL2 guarantees, because the guarantee
/// is the floor and a row that is exactly it leaves no room for the folding to
/// be wrong in only one direction.
pub const SURFACE_ROW: u32 = 1024;

/// The occluders of one frame: a list of surfaces, and the tile grid as its
/// index.
///
/// Decision 30 — a cell is `(offset, count)` into [`Occlusion::surfaces_at`],
/// and the walk iterates a tile's two or three rather than reading one merged
/// span. Empty cells are the ordinary case, most of a street being open sky, and
/// both [`Occlusion::at`] and [`Occlusion::surfaces_at`] answer for a tile
/// outside the rectangle without the caller having to know where the edge is.
///
/// Built by a [`Builder`] and immutable afterwards: the merge is the builder's
/// business, and what comes out of it is a list nothing appends to. That is what
/// lets a tile's surfaces be contiguous, which is what an `(offset, count)` is.
#[derive(Clone, PartialEq, Debug)]
pub struct Occlusion {
    bounds: TileBounds,
    /// Row-major over `bounds`, `x` fastest: the order [`Occlusion::bytes`]
    /// uploads and the shader indexes.
    index: Vec<Span>,
    /// Every surface in the frame, the ones of a tile contiguous. The order is
    /// the index's, which is what [`Occlusion::surface_bytes`] uploads.
    surfaces: Vec<Surface>,
    /// How much of the sky each tile can see, in the same order as the index —
    /// see this module's header and [`Occlusion::sky_at`].
    ///
    /// A byte and not an `Option`: every tile has an answer, and the answer for
    /// a tile with nothing over it is [`SKY_OPEN`] rather than "absent".
    sky: Vec<u8>,
}

impl Occlusion {
    /// A grid covering no tiles at all, which occludes nothing anywhere.
    ///
    /// A `const` and therefore empty `Vec`s, which allocate nothing: it is
    /// what [`Lighting::NONE`](crate::light::Lighting::NONE) is built from, and
    /// a daylit frame must not pay for a grid it will not read.
    pub const EMPTY: Self = Self {
        bounds: TileBounds {
            min_x: 0,
            max_x: -1,
            min_y: 0,
            max_y: -1,
        },
        index: Vec::new(),
        surfaces: Vec::new(),
        sky: Vec::new(),
    };

    /// The rectangle of tiles this covers.
    pub fn bounds(&self) -> TileBounds {
        self.bounds
    }

    /// Whether this grid covers no tiles at all — [`Occlusion::EMPTY`], and the
    /// grid a frame with no lighting binds.
    ///
    /// Not "nothing stands in it": a grid over real tiles with no occluder on
    /// any of them still answers [`Occlusion::sky_at`] for every one of them, and
    /// the caller that asks this — [`Lighting::is_identity`](crate::light::Lighting::is_identity)
    /// — is asking whether there is a field to read at all.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The surfaces standing on one tile, in no order the walk depends on — and
    /// an empty slice for open ground and for anything outside the rectangle.
    ///
    /// What the walk reads. A tile carries one lid, or one body, or a panel per
    /// side its art named; a caller combines them itself, and the combination is
    /// a rule rather than a fold — see `light::walk_cells`, which takes the
    /// largest and not the product, because two panels of one wall are one wall.
    pub fn surfaces_at(&self, x: i32, y: i32) -> &[Surface] {
        let Some(index) = self.index(x, y) else {
            return &[];
        };
        let span = self.index[index];
        let from = span.offset as usize;
        &self.surfaces[from..from + usize::from(span.count)]
    }

    /// What stands on one tile as one box, or `None` for open ground and for
    /// anything outside the rectangle.
    ///
    /// The **merged view** of [`Occlusion::surfaces_at`] and derived from it on
    /// every call: the union of the spans, the largest opacity and the union of
    /// the sides. For the readers whose question is genuinely about a tile — the
    /// wireframe, the plan view, which way a mounted flame steps out of its own
    /// cell — and not for the walk, which stopped asking it when the list
    /// arrived.
    pub fn at(&self, x: i32, y: i32) -> Option<Cell> {
        let surfaces = self.surfaces_at(x, y);
        let (first, rest) = surfaces.split_first()?;
        Some(rest.iter().fold(
            Cell {
                bottom: first.bottom,
                top: first.top,
                opacity: first.opacity,
                edges: first.edges,
            },
            |cell, surface| Cell {
                bottom: cell.bottom.min(surface.bottom),
                top: cell.top.max(surface.top),
                opacity: cell.opacity.max(surface.opacity),
                edges: cell.edges | surface.edges,
            },
        ))
    }

    /// How much of the sky one tile can see: [`SKY_OPEN`] under open air, `0`
    /// under a roof, and between under glass or beside a doorway.
    ///
    /// Open sky outside the rectangle, which is the honest default in the one
    /// direction that matters: the grid is grown by the widest pool's reach, so
    /// a tile outside it is a tile the frame does not draw, and a caller
    /// sampling one is asking about a place this frame knows nothing about.
    /// Answering "dark" there would put a band of night around every frame.
    pub fn sky_at(&self, x: i32, y: i32) -> u8 {
        match self.index(x, y) {
            Some(index) => self.sky[index],
            None => SKY_OPEN,
        }
    }

    /// Every tile something stands on, as `(x, y, cell)` — the grid as the boxes
    /// it is, for whatever wants to draw it.
    ///
    /// Open tiles are skipped: a grid is mostly nothing, and a caller drawing a
    /// box per cell would spend most of its work on cells with no box. The order
    /// is the rectangle's own, row by row, which is [`Occlusion::bytes`]'s and
    /// therefore stable frame to frame for a camera that has not moved.
    pub fn boxes(&self) -> impl Iterator<Item = (i32, i32, Cell)> + '_ {
        let bounds = self.bounds;
        let width = bounds.width();
        (0..self.index.len() as i32).filter_map(move |index| {
            let (x, y) = (bounds.min_x + index % width, bounds.min_y + index / width);
            Some((x, y, self.at(x, y)?))
        })
    }

    /// The second plane the shader reads: `Rgba8Uint`, one texel a tile, in
    /// [`Occlusion::bytes`]'s own order over the same rectangle.
    ///
    /// `(sky, 0, 0, 0)`. The three zeros are not padding, they are the format
    /// being decided once — see this module's header. What a tile *is* goes
    /// here; what a ray passes through stays in [`Occlusion::bytes`].
    pub fn field_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.sky.len() * 4);
        for sky in &self.sky {
            bytes.extend_from_slice(&[*sky, 0, 0, 0]);
        }
        bytes
    }

    /// The highest `z` anything in this grid stops light at, or `None` for a
    /// grid with nothing standing in it.
    ///
    /// What sunlight is bounded by. A flame's ray ends at the flame; the sun's
    /// has no end, so it needs something to stop walking at — and the honest
    /// answer is "as soon as the ray is above everything that could stop it".
    /// One number for the frame rather than a per-cell test, because the walk is
    /// leaving the grid upwards and what it has to beat is the tallest thing
    /// anywhere ahead of it.
    pub fn tallest(&self) -> Option<i32> {
        self.surfaces.iter().map(|surface| surface.top).max()
    }

    /// How many surfaces stand in the frame at all — what
    /// [`Occlusion::surface_bytes`] uploads, and the number decision 30.6 will
    /// have a distribution of.
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// Where a tile lives in [`Occlusion::index`].
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let bounds = self.bounds;
        if x < bounds.min_x || x > bounds.max_x || y < bounds.min_y || y > bounds.max_y {
            return None;
        }
        let (column, row) = (x - bounds.min_x, y - bounds.min_y);
        Some((row * bounds.width() + column) as usize)
    }

    /// The **index** as the texture the shader reads: `Rgba8Uint`, one texel a
    /// tile, row-major from the rectangle's `(min_x, min_y)` corner.
    ///
    /// `(offset & 255, offset >> 8, offset >> 16, count)` — decision 30.3's
    /// `(offset, count)`, with the offset spread over three channels because one
    /// byte holds 255 surfaces and a city block holds thousands. Twenty-four bits
    /// is sixteen million, which is four hundred surfaces on every tile of the
    /// widest frame this renderer draws.
    ///
    /// A count of zero is open ground, and it is the whole of the presence test:
    /// the offset of an empty tile is whatever the run before it ended at, and
    /// the shader never reads it. What used to be the `PRESENT` bit of a cell is
    /// now this — and `PRESENT` moved with the span it belongs to, into
    /// [`Occlusion::surface_bytes`].
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.index.len() * 4);
        for span in &self.index {
            bytes.extend_from_slice(&[
                (span.offset & 0xFF) as u8,
                ((span.offset >> 8) & 0xFF) as u8,
                ((span.offset >> 16) & 0xFF) as u8,
                span.count,
            ]);
        }
        bytes
    }

    /// The **list** as the texture the shader reads: `Rgba8Uint`, one texel a
    /// surface, folded into rows [`SURFACE_ROW`] wide and padded to a whole row.
    ///
    /// `(bottom + 128, top + 128, opacity, PRESENT | edges)` — which is what a
    /// cell's texel was before the list existed, moved down a level unchanged.
    /// The `z` offset is what makes an `i8` fit an unsigned channel, and both
    /// ends are clamped into it: a map's `z` is an `i8`, but `z + height` is not
    /// — a 255-tall static based at 100 has a top no channel holds, and a wall
    /// that reaches past the top of the world may as well stop there.
    ///
    /// [`PRESENT`] is still written, and it is still not padding: a lid's mask is
    /// legitimately zero, so a texel of all zeros has to be distinguishable from
    /// a horizontal surface at `z = -128`. What says a *tile* is empty is the
    /// index's count.
    pub fn surface_bytes(&self) -> Vec<u8> {
        let row = SURFACE_ROW as usize;
        let rows = self.surfaces.len().div_ceil(row).max(1);
        let mut bytes = Vec::with_capacity(rows * row * 4);
        for surface in &self.surfaces {
            let channel = |z: i32| (z.clamp(-128, 127) + 128) as u8;
            bytes.extend_from_slice(&[
                channel(surface.bottom),
                channel(surface.top),
                surface.opacity,
                PRESENT | surface.edges,
            ]);
        }
        bytes.resize(rows * row * 4, 0);
        bytes
    }
}

/// Builds one frame's [`Occlusion`]: everything that merges, merges here.
///
/// The grid is written tile by tile — a static at a time, in whatever order the
/// map walk finds them — and what comes out is a packed list nothing appends to.
/// The two are separate types because the two are separate shapes: a tile's
/// surfaces have to be contiguous for an `(offset, count)` to name them, and
/// they cannot be while anything can still be added.
///
/// The **union** lives here and only here, which is what makes
/// `docs/lighting.md`'s step 21.2 a change to one function.
#[derive(Clone, PartialEq, Debug)]
pub struct Builder {
    bounds: TileBounds,
    /// One merged cell a tile, row-major over `bounds` — see [`Builder::add`].
    cells: Vec<Option<Cell>>,
    /// How much of the sky each tile can see, in the same order.
    sky: Vec<u8>,
}

impl Builder {
    /// An empty grid over `bounds`: nothing stops anything, and every tile sees
    /// the whole sky.
    pub fn new(bounds: TileBounds) -> Self {
        let tiles = (bounds.width() * bounds.height()) as usize;
        Self {
            bounds,
            cells: vec![None; tiles],
            sky: vec![SKY_OPEN; tiles],
        }
    }

    /// Add one occluder, merging with whatever already stands on that tile.
    ///
    /// The merge is the **union** of the two spans, and it is deliberately the
    /// conservative direction: two walls on one tile with a gap between them
    /// close the gap. After the cutaway has removed the storeys the player is
    /// not on, a tile holding two opaque statics is a doorframe and a lintel far
    /// more often than it is two walls with air between, and darkening a foot of
    /// air is invisible where leaking a room into the street is not.
    ///
    /// It is also what step 21.2 takes apart — a lid and a panel on one tile
    /// have no business sharing a span or a mask — and keeping it while the
    /// storage changed under it is what lets this step promise that the picture
    /// does not move.
    ///
    /// A tile outside the rectangle is dropped rather than clamped: it is a
    /// caller walking wider than it asked the grid for, and folding it onto the
    /// edge would put a wall where the map has none.
    pub fn add(
        &mut self,
        x: u16,
        y: u16,
        z: i8,
        graphic: Graphic,
        tile: &StaticTile,
        facing: Option<Facing>,
    ) {
        let opacity = opacity(graphic, tile);
        if opacity == CLEAR {
            return;
        }
        let Some(index) = self.index(i32::from(x), i32::from(y)) else {
            return;
        };
        let bottom = i32::from(z);
        let span = Cell {
            bottom,
            top: bottom + calc_height(tile),
            opacity,
            // A floor or a rug is a **lid**: its occlusion is the `z` it lies at
            // and no vertical side of the tile describes it, so it names no
            // edge. Everything that stands up names the edge the art gave it, or
            // all four where the art would not say — see `edges_of`.
            //
            // The client's own `FLOOR` bit decides which, exactly as it does for
            // `place::Stance`, and asking it here rather than trusting the face
            // to be `None` is deliberate: a floor whose silhouette happened to
            // read as a wall would otherwise be given one edge out of four and
            // stop three quarters less light than it does today.
            edges: match tile.flags.is_background() {
                true => 0,
                false => edges_of(facing),
            },
        };
        self.cells[index] = Some(match self.cells[index] {
            None => span,
            // The union in every field, which is the conservative direction the
            // span already took: two walls on one tile close the gap between
            // them, and a wall beside a lid stops what either of them would.
            Some(had) => Cell {
                bottom: had.bottom.min(span.bottom),
                top: had.top.max(span.top),
                opacity: had.opacity.max(span.opacity),
                edges: had.edges | span.edges,
            },
        });
    }

    /// Take a tile's sky away, as far as one static standing over it does.
    ///
    /// `floor` is the height of the ground under the tile, and it is what makes
    /// this a *column over the floor* rather than a census of the tile: a
    /// cellar's wall is below the street it stands under and takes none of that
    /// street's sky, which is the same three-dimensional honesty the shadow walk
    /// gets from a surface's span.
    ///
    /// Multiplicative, so two roofs over one tile do not make it darker than
    /// black and a pane under a slate roof is as dark as the slate — and so that
    /// a pane on its own passes its share. Deliberately **not** filtered by the
    /// frame's [`Cutaway`]; the module header says why, and it is the one place
    /// this crate reads the map as it is rather than as it is drawn.
    ///
    /// A tile outside the rectangle is dropped, exactly as [`Builder::add`]
    /// drops one.
    pub fn shade(&mut self, x: u16, y: u16, z: i8, floor: i8, graphic: Graphic, tile: &StaticTile) {
        let opacity = opacity(graphic, tile);
        if opacity == CLEAR {
            return;
        }
        let top = i32::from(z) + calc_height(tile);
        if top < i32::from(floor) {
            return;
        }
        let Some(index) = self.index(i32::from(x), i32::from(y)) else {
            return;
        };
        let passes = u32::from(SKY_OPEN - opacity);
        self.sky[index] = ((u32::from(self.sky[index]) * passes) / u32::from(SKY_OPEN)) as u8;
    }

    /// How much of the sky one tile can see, part-built — [`Occlusion::sky_at`]'s
    /// own answer, asked of the grid before it is packed.
    pub fn sky_at(&self, x: i32, y: i32) -> u8 {
        match self.index(x, y) {
            Some(index) => self.sky[index],
            None => SKY_OPEN,
        }
    }

    /// Soften the sky field by a tile: one 3x3 pass, in place.
    ///
    /// The last thing done to the field and never done twice — [`collect`] calls
    /// it once, after every occluder has been shaded in, because a blur of a
    /// half-built field is a blur of the wrong picture.
    ///
    /// The edge of the rectangle repeats rather than falling off: a tile outside
    /// the grid is open sky by [`Occlusion::sky_at`]'s rule, and averaging that
    /// in would draw a bright rim around the inside of every frame's border —
    /// which is a picture of where the grid ends, not of where the roof does.
    pub fn blur_sky(&mut self) {
        let (width, height) = (self.bounds.width(), self.bounds.height());
        if width <= 0 || height <= 0 {
            return;
        }
        let mut blurred = vec![SKY_OPEN; self.sky.len()];
        for row in 0..height {
            for column in 0..width {
                let mut total = 0_u32;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let x = (column + dx).clamp(0, width - 1);
                        let y = (row + dy).clamp(0, height - 1);
                        total += u32::from(self.sky[(y * width + x) as usize]);
                    }
                }
                blurred[(row * width + column) as usize] = (total / 9) as u8;
            }
        }
        self.sky = blurred;
    }

    /// Pack the grid into the list the walk reads.
    ///
    /// One pass in the index's own order, so a tile's surfaces come out
    /// contiguous and in the order the tiles are in — which is what makes the
    /// grid's texture and the list's two views of one thing.
    pub fn finish(self) -> Occlusion {
        let mut index = Vec::with_capacity(self.cells.len());
        let mut surfaces = Vec::with_capacity(self.cells.len());
        for cell in &self.cells {
            let offset = surfaces.len() as u32;
            if let Some(cell) = *cell {
                push_surfaces(cell, &mut surfaces);
            }
            index.push(Span {
                offset,
                count: (surfaces.len() as u32 - offset) as u8,
            });
        }
        Occlusion {
            bounds: self.bounds,
            index,
            surfaces,
            sky: self.sky,
        }
    }

    /// Where a tile lives in [`Builder::cells`].
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let bounds = self.bounds;
        if x < bounds.min_x || x > bounds.max_x || y < bounds.min_y || y > bounds.max_y {
            return None;
        }
        let (column, row) = (x - bounds.min_x, y - bounds.min_y);
        Some((row * bounds.width() + column) as usize)
    }
}

/// The surfaces one merged cell is, appended in the order the sides are numbered
/// in.
///
/// **One to one with what the cell already meant**, which is the whole of why
/// this step can promise the picture does not move: a lid is one horizontal, a
/// body — all four sides, "it stands up and the art would not say which way" —
/// is one solid, and a named mask is a quad on each side it names, every one of
/// them carrying the cell's own span and opacity.
///
/// So a cell is never a mixture: it is one lid, or one body, or panels. That is
/// the union talking, and step 21.2 is where a tile gets to hold a lid *and* a
/// panel with spans of their own.
fn push_surfaces(cell: Cell, out: &mut Vec<Surface>) {
    let quad = |edges| Surface {
        bottom: cell.bottom,
        top: cell.top,
        opacity: cell.opacity,
        edges,
    };
    match cell.edges {
        0 | EDGE_ANY => out.push(quad(cell.edges)),
        named => {
            for side in [EDGE_NORTH, EDGE_EAST, EDGE_SOUTH, EDGE_WEST] {
                if named & side != 0 {
                    out.push(quad(side));
                }
            }
        }
    }
}

/// Everything on `bounds` that stands between a flame and the ground.
///
/// The same two sources the flames themselves come from — the map's statics and
/// the items the server has put on the ground — walked with the same bounds and
/// tested with the same [`Cutaway`]. Both halves matter: a wall is a static, and
/// **a door is an item**, sent by the server and swapped for its open graphic
/// when it is opened. A closed door that let the light through would be the one
/// occluder a player watches change.
/// The sky field is built out of the same walk, and the two halves of this
/// function are deliberately not the same rule: everything is shaded into the
/// sky, and only what the frame *draws* is added to the occluders. See the
/// module header.
pub fn collect(
    map: &Map,
    items: &[GroundItem],
    bounds: TileBounds,
    tiledata: &TileData,
    cutaway: &Cutaway,
    atlas: Option<&crate::atlas::StaticAtlas>,
) -> Occlusion {
    let mut occlusion = Builder::new(bounds);
    // The ground each tile's column is measured from. Off the map it is zero:
    // there is no floor there and nothing draws, and a static hanging over the
    // void still has to shade something rather than be skipped by an `unwrap`.
    let floor = |x: u16, y: u16| map.land(x, y).map_or(0, |cell| cell.z);
    // Which edge each graphic's wall stands on, measured once when its picture
    // was packed. `None` for the whole atlas is a caller that has no pictures —
    // a built scene, a test — and every occluder is then the whole tile it always
    // was. `None` for one graphic is the atlas not holding it, which happens at
    // the rim: the grid is grown by the widest pool's reach and the atlas by what
    // is drawn, and those are not the same rectangle. Both fall back the safe
    // way. See `Occlusion::add` and `crate::facing`.
    let facing = |graphic: Graphic| {
        atlas
            .and_then(|atlas| atlas.sprite(graphic))
            .and_then(|s| s.facing)
    };

    crate::statics::for_each_static_in(map, bounds, |item| {
        let tile = tiledata.static_tile(item.tile);
        occlusion.shade(
            item.x,
            item.y,
            item.z,
            floor(item.x, item.y),
            Graphic(item.tile),
            tile,
        );
        if cutaway::shows(cutaway, item.z, tile) {
            occlusion.add(
                item.x,
                item.y,
                item.z,
                Graphic(item.tile),
                tile,
                facing(Graphic(item.tile)),
            );
        }
    });

    for item in items {
        let tile = tiledata.static_tile(item.graphic.0);
        occlusion.shade(
            item.at.x,
            item.at.y,
            item.at.z,
            floor(item.at.x, item.at.y),
            item.graphic,
            tile,
        );
        if cutaway::shows(cutaway, item.at.z, tile) {
            occlusion.add(
                item.at.x,
                item.at.y,
                item.at.z,
                item.graphic,
                tile,
                facing(item.graphic),
            );
        }
    }

    occlusion.blur_sky();
    occlusion.finish()
}

#[cfg(test)]
mod tests {
    /// A graphic in none of `crate::doors`' families, for the tests here that are
    /// about flags rather than about doors. Zero is below every family base.
    const NOT_A_DOOR: Graphic = Graphic(0);

    use openshard_protocol::wire::{Graphic, Hue};
    use openshard_protocol::world::Point;
    use openshard_uofiles::map::{LandCell, Map};

    use super::*;

    /// A static tile with the flags and height a test is about.
    fn tile(flags: u64, height: u8) -> StaticTile {
        StaticTile {
            flags: TileFlags::new(flags),
            height,
            ..StaticTile::default()
        }
    }

    /// An open door leaves the grid, and the graphic is the only thing that says
    /// so.
    ///
    /// The defect this is the fix for: `tiledata.mul` gives an open leaf the
    /// flags of its shut twin, so a door read by its flags alone lays a whole
    /// tile of wall across its own doorway — and decision 3's occluder being a
    /// tile makes that a band of shadow with nothing visible casting it. The
    /// pair below is the same `StaticTile` twice, which is the point: nothing in
    /// it differs, and the answers do.
    #[test]
    fn an_open_door_stops_nothing_and_its_shut_twin_stops_everything() {
        // `MetalDoor` facing 0, from `crate::doors` — and the flags the client
        // actually gives both of its leaves.
        let (shut, open) = (Graphic(0x0675), Graphic(0x0676));
        let leaf = tile(TileFlags::NO_SHOOT | TileFlags::BLOCK | TileFlags::WALL, 20);
        assert_eq!(opacity(shut, &leaf), OPAQUE, "a shut door is a wall");
        assert_eq!(opacity(open, &leaf), CLEAR, "an open door is a doorway");

        // And the grid keeps no cell for it, which is what the shadow walk reads.
        let mut occlusion = Builder::new(bounds());
        occlusion.add(100, 100, 0, shut, &leaf, None);
        occlusion.add(101, 100, 0, open, &leaf, None);
        let occlusion = occlusion.finish();
        assert!(occlusion.at(100, 100).is_some(), "the shut leaf left the grid");
        assert_eq!(
            occlusion.at(101, 100),
            None,
            "the open leaf is still a tile of wall across its own doorway",
        );

        // The sky too, and for the same reason: a doorway you can see through is
        // a doorway you can see the sky through. `shade` and `add` reading one
        // `opacity` is what keeps those two from drifting apart.
        let mut occlusion = Builder::new(bounds());
        occlusion.shade(101, 100, 0, 0, open, &leaf);
        assert_eq!(occlusion.sky_at(101, 100), SKY_OPEN, "an open door took the sky");
    }

    /// A rectangle big enough for a few tiles around the origin of a test.
    fn bounds() -> TileBounds {
        TileBounds {
            min_x: 100,
            max_x: 110,
            min_y: 100,
            max_y: 110,
        }
    }

    /// The rule, said in every direction that matters. A wall stops light; a
    /// pane dims it; a barrel, which is `BLOCK` and nothing else, does not touch
    /// it. Reading impassability instead of the shooting flags is the mistake
    /// this was written against — it would put a shadow behind every crate on
    /// the street — and treating a window as a wall is the one beside it, which
    /// makes a lit room invisible from the road.
    #[test]
    fn a_wall_stops_light_a_pane_dims_it_and_a_barrel_does_not() {
        assert_eq!(opacity(NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20)), OPAQUE);
        assert_eq!(opacity(NOT_A_DOOR, &tile(TileFlags::WINDOW, 20)), PANE);
        assert_eq!(opacity(NOT_A_DOOR, &tile(TileFlags::BLOCK, 10)), CLEAR);
        // A real wall carries both, and the rule must not need the pair.
        assert_eq!(
            opacity(NOT_A_DOOR, &tile(TileFlags::NO_SHOOT | TileFlags::BLOCK, 20)),
            OPAQUE
        );
        // And a static flagged as both a window and solid is the solid one: the
        // union darkens, which is the direction that cannot leak a room.
        assert_eq!(
            opacity(NOT_A_DOOR, &tile(TileFlags::NO_SHOOT | TileFlags::WINDOW, 20)),
            OPAQUE
        );
        const { assert!(PANE > CLEAR && PANE < OPAQUE, "a pane is neither open nor a wall") };
    }

    /// A wall occupies the heights it occupies, and the grid says which.
    #[test]
    fn a_wall_carries_the_span_it_stands_in() {
        let mut occlusion = Builder::new(bounds());
        occlusion.add(102, 103, 5, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20), None);
        let occlusion = occlusion.finish();
        assert_eq!(
            occlusion.at(102, 103),
            Some(Cell {
                bottom: 5,
                top: 25,
                opacity: OPAQUE,
                edges: EDGE_ANY,
            })
        );
        assert_eq!(occlusion.at(103, 103), None, "its neighbour is open ground");
    }

    /// A corner stands on **two** of its tile's sides, and on the other two it
    /// stands on nothing.
    ///
    /// The grid's half of decision 25. Two bits and not four is what the walk
    /// reads as a panel rather than as a body — see the `edges` arm of
    /// `light::walk_cells` — so a ray crossing the sides the corner does
    /// not stand on passes, exactly as it does beside the runs of wall either
    /// side of it. Before this every corner in the world was `EDGE_ANY`.
    #[test]
    fn a_corner_stands_on_the_two_sides_its_art_named() {
        use crate::facing::{Face, Facing};

        let corner = Facing::Corner {
            right: Face::East,
            left: Face::South,
        };
        assert_eq!(edges_of(Some(corner)), EDGE_EAST | EDGE_SOUTH);
        // And each of the four pairings, so that a mask built from the right
        // half's answer twice would be caught.
        assert_eq!(
            edges_of(Some(Facing::Corner {
                right: Face::North,
                left: Face::West
            })),
            EDGE_NORTH | EDGE_WEST,
        );
        // A plain wall is still one side, and a graphic nothing measured is still
        // the whole tile: neither of those moved.
        assert_eq!(edges_of(Some(Facing::One(Face::South))), EDGE_SOUTH);
        assert_eq!(edges_of(None), EDGE_ANY);

        let mut occlusion = Builder::new(bounds());
        occlusion.add(
            102,
            103,
            0,
            NOT_A_DOOR,
            &tile(TileFlags::NO_SHOOT, 20),
            Some(corner),
        );
        let occlusion = occlusion.finish();
        assert_eq!(
            occlusion.at(102, 103).unwrap().edges,
            EDGE_EAST | EDGE_SOUTH,
            "the cell did not take the corner's two sides",
        );
        // And in the list it is **two surfaces**, which is the shape decision 30
        // gives a corner: one quad a side, each with the tile's own span. The
        // merged view above is a fold over exactly these.
        assert_eq!(
            occlusion.surfaces_at(102, 103),
            &[
                Surface {
                    bottom: 0,
                    top: 20,
                    opacity: OPAQUE,
                    edges: EDGE_EAST,
                },
                Surface {
                    bottom: 0,
                    top: 20,
                    opacity: OPAQUE,
                    edges: EDGE_SOUTH,
                },
            ],
        );
    }

    /// What a cell is in the list: a lid is one horizontal, a body is one solid,
    /// and a named mask is a quad a side.
    ///
    /// The claim that makes step 21.1 a change of storage and nothing else. A
    /// cell is never a mixture of the two kinds — that is the union in `add`
    /// talking — and the walk's two rules are exactly these two kinds, so a
    /// surface list built any other way would move the picture.
    #[test]
    fn a_cell_becomes_the_surfaces_it_always_meant() {
        let mut occlusion = Builder::new(bounds());
        // A floor: a lid, and one surface naming no side.
        occlusion.add(
            100,
            100,
            10,
            NOT_A_DOOR,
            &tile(TileFlags::NO_SHOOT | TileFlags::FLOOR, 0),
            None,
        );
        // A graphic nothing measured: a body, and one surface on all four sides
        // rather than four quads. The walk travels *through* it, which is a rule
        // about a solid and not about four planes.
        occlusion.add(101, 100, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20), None);
        let occlusion = occlusion.finish();

        assert_eq!(
            occlusion.surfaces_at(100, 100),
            &[Surface {
                bottom: 10,
                top: 10,
                opacity: OPAQUE,
                edges: 0,
            }],
        );
        assert_eq!(
            occlusion.surfaces_at(101, 100),
            &[Surface {
                bottom: 0,
                top: 20,
                opacity: OPAQUE,
                edges: EDGE_ANY,
            }],
        );
        assert_eq!(occlusion.surfaces_at(102, 100), &[], "open ground stands nothing");
        assert_eq!(occlusion.surfaces_at(0, 0), &[], "and neither does off the grid");
        assert_eq!(occlusion.surface_count(), 2, "and nothing else got into the list");
    }

    /// Stairs count as half their height, the way every other reader of this
    /// field here does. A stair that occluded its full height would shadow the
    /// landing it leads to.
    #[test]
    fn a_climbable_static_occludes_half_its_height() {
        let mut occlusion = Builder::new(bounds());
        occlusion.add(
            100,
            100,
            0,
            NOT_A_DOOR,
            &tile(TileFlags::NO_SHOOT | TileFlags::CLIMBABLE, 20),
            None,
        );
        assert_eq!(occlusion.finish().at(100, 100).unwrap().top, 10);
    }

    /// Two occluders on one tile become one span covering both. The union is the
    /// conservative direction and the doc comment on `add` argues for it; this
    /// pins that it is what happens.
    #[test]
    fn two_occluders_on_one_tile_span_both() {
        let mut occlusion = Builder::new(bounds());
        occlusion.add(105, 105, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20), None);
        occlusion.add(105, 105, 40, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20), None);
        let occlusion = occlusion.finish();
        assert_eq!(
            occlusion.at(105, 105),
            Some(Cell {
                bottom: 0,
                top: 60,
                opacity: OPAQUE,
                // Neither was given a face, so both are the whole tile.
                edges: EDGE_ANY,
            })
        );
    }

    /// Outside the rectangle is not the edge of it. A caller walking wider than
    /// the grid was built for must lose the occluder rather than fold it onto
    /// the border, where it would be a wall the map does not have.
    #[test]
    fn a_tile_outside_the_bounds_is_dropped_and_not_clamped() {
        let mut occlusion = Builder::new(bounds());
        occlusion.add(99, 100, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20), None);
        let occlusion = occlusion.finish();
        assert_eq!(occlusion.at(99, 100), None);
        assert_eq!(occlusion.at(100, 100), None, "and did not land on the edge");
    }

    /// The upload is two textures now: the index in the grid's own order, and
    /// the list it points into.
    ///
    /// The `z` offset and its clamp moved down into the surface with the span
    /// they belong to, and what is left in the grid is `(offset, count)` — so
    /// a tile that stands nothing is a count of zero and the offset beside it is
    /// not read. Getting the three-channel offset backwards would point every
    /// wall at some other wall's span, which is why each byte of it is named
    /// here.
    #[test]
    fn the_bytes_are_the_index_and_the_surfaces_it_points_into() {
        let mut occlusion = Builder::new(TileBounds {
            min_x: 0,
            max_x: 1,
            min_y: 0,
            max_y: 1,
        });
        occlusion.add(1, 0, -10, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20), None);
        occlusion.add(0, 1, 120, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 60), None);
        let occlusion = occlusion.finish();

        let bytes = occlusion.bytes();
        assert_eq!(bytes.len(), 4 * 4, "one texel a tile");
        assert_eq!(&bytes[0..4], &[0, 0, 0, 0], "(0,0) stands nothing");
        assert_eq!(
            &bytes[4..8],
            &[0, 0, 0, 1],
            "(1,0) is x-fastest, and the first surface"
        );
        assert_eq!(&bytes[8..12], &[1, 0, 0, 1], "(0,1) is the second");
        assert_eq!(
            &bytes[12..16],
            &[2, 0, 0, 0],
            "and (1,1) stands nothing after them"
        );

        // The list is the spans, in the index's order, with the fourth channel
        // `PRESENT` and the edge mask rather than a bare yes: neither of these
        // was given a face, so both are the whole tile.
        let whole = PRESENT | EDGE_ANY;
        let surfaces = occlusion.surface_bytes();
        assert_eq!(
            surfaces.len(),
            SURFACE_ROW as usize * 4,
            "one row, padded — the fold into a texture is `SURFACE_ROW` wide",
        );
        assert_eq!(&surfaces[0..4], &[118, 138, OPAQUE, whole]);
        assert_eq!(
            &surfaces[4..8],
            &[248, 255, OPAQUE, whole],
            "reaches past the top of the world and stops there",
        );
        assert_eq!(&surfaces[8..12], &[0, 0, 0, 0], "and nothing follows it");

        // A lid's mask is zero and it is still present, which is the one thing
        // `PRESENT` exists for — a fourth channel of zero has to mean nothing
        // stands here and nothing else.
        let mut lid = Builder::new(TileBounds {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
        });
        lid.add(
            0,
            0,
            20,
            NOT_A_DOOR,
            &tile(TileFlags::NO_SHOOT | TileFlags::FLOOR, 0),
            None,
        );
        assert_eq!(
            lid.finish().surface_bytes()[3],
            PRESENT,
            "a floor is present with no side of its own"
        );
    }

    /// The boxes are the cells, at the tiles they stand on — the claim the
    /// wireframe is drawn on the strength of. Getting the row-major arithmetic
    /// backwards here draws every wall at its tile's mirror image, which looks
    /// like a camera bug rather than like an index one.
    #[test]
    fn the_boxes_name_the_tiles_they_stand_on() {
        let mut occlusion = Builder::new(TileBounds {
            min_x: 100,
            max_x: 102,
            min_y: 200,
            max_y: 201,
        });
        occlusion.add(102, 200, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20), None);
        occlusion.add(100, 201, 5, NOT_A_DOOR, &tile(TileFlags::WINDOW, 10), None);
        let occlusion = occlusion.finish();
        let boxes: Vec<_> = occlusion.boxes().collect();
        assert_eq!(
            boxes,
            vec![
                (
                    102,
                    200,
                    Cell {
                        bottom: 0,
                        top: 20,
                        opacity: OPAQUE,
                        edges: EDGE_ANY,
                    }
                ),
                (
                    100,
                    201,
                    Cell {
                        bottom: 5,
                        top: 15,
                        opacity: PANE,
                        edges: EDGE_ANY,
                    }
                ),
            ],
            "row by row, x fastest, and open tiles are not in it",
        );
        assert_eq!(Occlusion::EMPTY.boxes().count(), 0, "and an empty grid has none");
    }

    /// The column, in every direction it has to be right in: a roof takes the
    /// sky, a pane passes most of it, a barrel takes none, and a wall down in a
    /// cellar takes none of the street's.
    ///
    /// Before the blur, because these are claims about the column test and the
    /// blur is a claim about the neighbourhood — mixing the two would leave
    /// every number here a function of what is on eight other tiles.
    #[test]
    fn the_column_over_a_tile_is_what_takes_its_sky() {
        let mut occlusion = Builder::new(bounds());
        assert_eq!(occlusion.sky_at(100, 100), SKY_OPEN, "nothing built yet");

        occlusion.shade(100, 100, 20, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 5));
        assert_eq!(occlusion.sky_at(100, 100), 0, "a roof over the floor");

        occlusion.shade(101, 100, 20, 0, NOT_A_DOOR, &tile(TileFlags::WINDOW, 5));
        assert_eq!(
            occlusion.sky_at(101, 100),
            204,
            "a glazed roof passes four fifths of the sky",
        );

        occlusion.shade(102, 100, 20, 0, NOT_A_DOOR, &tile(TileFlags::BLOCK, 5));
        assert_eq!(occlusion.sky_at(102, 100), SKY_OPEN, "a crate is not a lid");

        // A cellar's wall, twenty tall, standing forty below the street: its top
        // is still under the floor, so the street above it is open sky.
        occlusion.shade(103, 100, -40, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 20));
        assert_eq!(occlusion.sky_at(103, 100), SKY_OPEN);

        // And two panes are darker than one: the column multiplies.
        occlusion.shade(101, 100, 30, 0, NOT_A_DOOR, &tile(TileFlags::WINDOW, 5));
        assert!(occlusion.sky_at(101, 100) < 204);

        assert_eq!(occlusion.sky_at(0, 0), SKY_OPEN, "outside the grid is sky");
    }

    /// The blur is a tile wide and it does not brighten the border.
    ///
    /// The second half is the one worth a test: the grid's edge is where the
    /// *frame* ends, not where the roof does, and a blur that averaged in the
    /// open sky outside would draw a bright rim around the inside of every
    /// frame — a picture of the rectangle rather than of the world.
    #[test]
    fn the_blur_spreads_a_tile_and_leaves_the_border_alone() {
        let small = TileBounds {
            min_x: 0,
            max_x: 2,
            min_y: 0,
            max_y: 2,
        };
        let mut occlusion = Builder::new(small);
        for x in 0..=2u16 {
            for y in 0..=2u16 {
                occlusion.shade(x, y, 20, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 5));
            }
        }
        occlusion.blur_sky();
        for x in 0..=2 {
            for y in 0..=2 {
                assert_eq!(occlusion.sky_at(x, y), 0, "({x}, {y}) is under the roof");
            }
        }

        // One roofed tile in the middle of open ground: it lifts off zero and
        // its neighbours come down off the sky, which is the doorway's gradient
        // arriving from the other side.
        let mut one = Builder::new(bounds());
        one.shade(105, 105, 20, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 5));
        one.blur_sky();
        // Eight open neighbours and itself dark: 255 * 8 / 9.
        assert_eq!(one.sky_at(105, 105), 226);
        assert!(one.sky_at(106, 105) < SKY_OPEN, "the eave shades its neighbour");
        assert_eq!(one.sky_at(107, 105), SKY_OPEN, "and nothing two tiles away");
    }

    /// The sky is read off the map as it is, not as it is drawn.
    ///
    /// `docs/lighting_world.md`'s decision 3, and it is a real inversion of the
    /// rule beside it: the same roof that must stop casting a shadow the moment
    /// the cutaway removes it must go on keeping the daylight out. Otherwise
    /// walking through a door floods the room with noon, and the player carries
    /// daylight into every building they enter.
    #[test]
    fn the_cutaway_takes_a_roof_from_the_eye_and_not_from_the_sky() {
        let map = Map::from_blocks(1, 1, |_, _| LandCell { tile: 0, z: 0 });
        let graphic = Graphic(0x000A);
        let mut tiledata = TileData::empty();
        tiledata.set_static_tile(graphic.0, tile(TileFlags::NO_SHOOT, 5));
        // A patch of roof wide enough that the middle of it is roofed on all
        // nine of the tiles the blur reads.
        let items: Vec<GroundItem> = (2..=6u16)
            .flat_map(|x| {
                (2..=6u16).map(move |y| GroundItem {
                    at: Point::new(x, y, 20),
                    graphic,
                    hue: Hue::NONE,
                })
            })
            .collect();
        let bounds = TileBounds {
            min_x: 0,
            max_x: 7,
            min_y: 0,
            max_y: 7,
        };

        let open = collect(&map, &items, bounds, &tiledata, &Cutaway::OPEN, None);
        let cut = collect(
            &map,
            &items,
            bounds,
            &tiledata,
            &Cutaway {
                max_z: 20,
                no_draw_roofs: true,
                ..Cutaway::OPEN
            },
            None,
        );

        assert!(open.at(4, 4).is_some(), "with nothing cut it occludes");
        assert_eq!(cut.at(4, 4), None, "and the cutaway takes it out of the walk");
        assert_eq!(open.sky_at(4, 4), 0, "the roof keeps the sky off the floor");
        assert_eq!(
            cut.sky_at(4, 4),
            open.sky_at(4, 4),
            "the room brightened when the player walked in",
        );
    }

    /// The second plane is the field, in the same order as the cells, with the
    /// three channels the aperture and a body are going to want left at zero.
    #[test]
    fn the_field_bytes_are_the_sky_in_the_cells_own_order() {
        let mut occlusion = Builder::new(TileBounds {
            min_x: 0,
            max_x: 1,
            min_y: 0,
            max_y: 1,
        });
        occlusion.shade(1, 0, 20, 0, NOT_A_DOOR, &tile(TileFlags::NO_SHOOT, 5));
        let bytes = occlusion.finish().field_bytes();
        assert_eq!(bytes.len(), 4 * 4, "one texel a tile, four channels");
        assert_eq!(&bytes[0..4], &[SKY_OPEN, 0, 0, 0], "(0,0) is open sky");
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0], "(1,0) is x-fastest, and roofed");
    }

    /// A wall the cutaway has taken away casts no shadow. The storey above the
    /// player is not drawn, and a dark band under a wall that is not in the
    /// picture is worse than the light leaking.
    #[test]
    fn a_hidden_wall_occludes_nothing() {
        let map = Map::from_blocks(1, 1, |_, _| LandCell { tile: 0, z: 0 });
        let graphic = Graphic(0x0006);
        let mut tiledata = TileData::empty();
        tiledata.set_static_tile(graphic.0, tile(TileFlags::NO_SHOOT, 20));
        let items = [GroundItem {
            at: Point::new(4, 4, 40),
            graphic,
            hue: Hue::NONE,
        }];
        let bounds = TileBounds {
            min_x: 0,
            max_x: 7,
            min_y: 0,
            max_y: 7,
        };

        let open = collect(&map, &items, bounds, &tiledata, &Cutaway::OPEN, None);
        assert!(open.at(4, 4).is_some(), "with nothing cut away it occludes");

        let cut = collect(
            &map,
            &items,
            bounds,
            &tiledata,
            &Cutaway {
                max_z: 20,
                ..Cutaway::OPEN
            },
            None,
        );
        assert_eq!(cut.at(4, 4), None);
    }

    /// Britain's houses are dark inside and its streets are not.
    ///
    /// The scenes above are built, which is what makes them readable and is also
    /// what makes them unable to answer this: they contain a roof *this crate
    /// placed*, flagged the way this crate assumed a roof is flagged. The whole
    /// column test rests on a real roof being in the grid at all — membership is
    /// `WINDOW | NO_SHOOT`, which is a fact about arrows, and nothing said it was
    /// also a fact about lids. Measured here rather than assumed: every one of
    /// the 203 roof statics over this block of Britain carries `NO_SHOOT`, so the
    /// answer is yes and `TileFlags::ROOF` is not needed for it.
    ///
    /// The classifier is the cutaway, which is the client's own idea of indoors
    /// and was ported from `UpdateMaxDrawZ` — so the two are independent: one
    /// reads the tile the player stands on and the tile a roof draws on, the
    /// other reads the column over each tile. Where they agree, they agree for
    /// two reasons.
    ///
    /// Stated as means over a block and not per tile: the eaves and the
    /// thresholds are *meant* to be in between, and a per-tile assertion would
    /// either forbid the blur or have to name every doorway in Britain.
    ///
    /// Skipped without the client's files.
    #[test]
    fn britains_rooms_are_dark_and_its_streets_are_not() {
        let Some(dir) = std::env::var_os("OPENSHARD_CLIENT").map(std::path::PathBuf::from) else {
            return;
        };
        let map = Map::load_facet(&dir, 0).expect("Felucca");
        let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
        // The same block of Britain the cutaway's own tests walk: wide enough to
        // hold whole buildings and the streets between them.
        let (from, to) = ((1470u16, 1600u16), (1530u16, 1660u16));
        let bounds = TileBounds {
            min_x: i32::from(from.0),
            max_x: i32::from(to.0),
            min_y: i32::from(from.1),
            max_y: i32::from(to.1),
        };
        let grid = collect(&map, &[], bounds, &tiledata, &Cutaway::OPEN, None);

        let (mut indoors, mut outdoors) = (Vec::new(), Vec::new());
        let (mut roofs, mut roofs_in_the_grid) = (0, 0);
        for y in from.1..=to.1 {
            for x in from.0..=to.0 {
                let Some(land) = map.land(x, y) else { continue };
                let here = openshard_protocol::world::Point::new(x, y, land.z);
                let sky = grid.sky_at(i32::from(x), i32::from(y));
                match Cutaway::at(&map, &tiledata, here, true) == Cutaway::OPEN {
                    true => outdoors.push(sky),
                    false => indoors.push(sky),
                }
                for item in map.statics_at(x, y) {
                    let tile = tiledata.static_tile(item.tile);
                    if !tile.flags.is_roof() {
                        continue;
                    }
                    roofs += 1;
                    roofs_in_the_grid += usize::from(stops_light(tile));
                }
            }
        }

        // A sweep that found nothing would assert nothing at all.
        assert!(indoors.len() > 500, "only {} indoor tiles", indoors.len());
        assert!(outdoors.len() > 500, "only {} outdoor tiles", outdoors.len());
        assert!(roofs > 100, "only {roofs} roof statics over this block");
        assert_eq!(
            roofs_in_the_grid, roofs,
            "a roof is not in the occlusion grid, so no column test can find it",
        );

        let mean = |tiles: &[u8]| tiles.iter().map(|sky| u32::from(*sky)).sum::<u32>() / tiles.len() as u32;
        let (inside, outside) = (mean(&indoors), mean(&outdoors));
        assert!(inside < 64, "Britain's rooms average {inside} of the sky");
        assert!(outside > 200, "Britain's streets average {outside} of the sky");
    }
}
