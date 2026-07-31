//! Land sprites, packed into one texture. Twice: once for the flat art and once
//! for the textures a slope is stretched over.
//!
//! A draw call can bind one texture, and a screen of ground touches a few
//! hundred different graphics, so they go into a grid: every land tile is
//! exactly 44x44, which makes the packing a division rather than a bin-packing
//! problem, and makes a slot's position something a test can state outright.
//!
//! [`TexmapAtlas`] is the same idea one step less regular, because a texture map
//! is 64 or 128 on a side. Both atlases are keyed by the *land graphic* even
//! though the texture is looked up through `tiledata` by a different id, so a
//! quad asks both of them the same question.
//!
//! The atlases are built for a set of graphics, not for the whole file. A modern
//! client ships about 4,244 land tiles and the container is 155MB; what is on
//! screen is a fraction of that, and the browser is the reason the difference
//! matters rather than an optimisation nobody asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use openshard_protocol::wire::Graphic;
use openshard_uofiles::anim::{Anim, AnimError, AnimFrame};
use openshard_uofiles::art::{Art, ArtError, LAND_TILE_SIZE, land_row};
use openshard_uofiles::image::Image;
use openshard_uofiles::texmaps::{TexMapError, TexMaps};
use openshard_uofiles::tiledata::TileData;

/// The atlas texture's side, in pixels.
///
/// Not larger, and not negotiable: WebGL2 only guarantees `MAX_TEXTURE_SIZE` of
/// 2048, so a 4096 atlas would work on this machine and fail on a phone. At 44
/// pixels a tile that is 46 columns of 46 rows, or 2,116 slots.
const ATLAS_SIDE: u32 = 2048;

/// Slots per row, and per column.
const SLOTS_PER_ROW: u32 = ATLAS_SIDE / LAND_TILE_SIZE as u32;

/// How many graphics one atlas can hold.
pub const CAPACITY: usize = (SLOTS_PER_ROW * SLOTS_PER_ROW) as usize;

/// What can go wrong building one.
#[derive(Debug)]
pub enum AtlasError {
    /// More pictures than the atlas holds.
    ///
    /// Not a "grow the texture" case: the cap is the web's, so the fix is to
    /// build an atlas per region, or to evict. Failing loudly is the point.
    Full {
        /// How many were asked for.
        wanted: usize,
        /// How many would have fitted, at best.
        capacity: usize,
    },
    /// A sprite is bigger than the whole atlas, so no packing could hold it.
    ///
    /// Separate from [`Full`](Self::Full) because it is not a capacity problem:
    /// the tallest static a client ships is around 250 pixels, so a sprite over
    /// 2048 means the art was decoded wrongly rather than that too much was
    /// asked for.
    Oversized {
        /// Which graphic.
        graphic: Graphic,
        /// How wide it claims to be.
        width: u16,
        /// How tall.
        height: u16,
    },
    /// The art container refused a graphic.
    Art(ArtError),
    /// The animation files refused a body.
    Anim(AnimError),
    /// The texture maps refused a texture.
    TexMaps(TexMapError),
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full { wanted, capacity } => {
                write!(f, "{wanted} pictures do not fit in an atlas of {capacity}")
            }
            Self::Oversized {
                graphic,
                width,
                height,
            } => write!(
                f,
                "{graphic:?} is {width}x{height}, which does not fit an atlas {ATLAS_SIDE} on a side",
            ),
            Self::Art(source) => write!(f, "reading land art: {source}"),
            Self::Anim(source) => write!(f, "reading an animation: {source}"),
            Self::TexMaps(source) => write!(f, "reading a land texture: {source}"),
        }
    }
}

impl std::error::Error for AtlasError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Full { .. } | Self::Oversized { .. } => None,
            Self::Art(source) => Some(source),
            Self::Anim(source) => Some(source),
            Self::TexMaps(source) => Some(source),
        }
    }
}

impl From<ArtError> for AtlasError {
    fn from(source: ArtError) -> Self {
        Self::Art(source)
    }
}

impl From<AnimError> for AtlasError {
    fn from(source: AnimError) -> Self {
        Self::Anim(source)
    }
}

impl From<TexMapError> for AtlasError {
    fn from(source: TexMapError) -> Self {
        Self::TexMaps(source)
    }
}

/// Where in the atlas a graphic's sprite sits, in texture coordinates.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Region {
    /// Left edge, 0..1.
    pub u: f32,
    /// Top edge, 0..1.
    pub v: f32,
    /// Width, 0..1.
    pub du: f32,
    /// Height, 0..1.
    pub dv: f32,
}

/// Land art, packed and ready to upload.
///
/// Holds its pixels rather than a GPU handle: this crate does not decide when a
/// texture is created, and a test wants to read the pixels without a device.
pub struct LandAtlas {
    slots: BTreeMap<Graphic, u32>,
    /// `ATLAS_SIDE * ATLAS_SIDE` RGBA8 pixels, row-major.
    pixels: Vec<u8>,
}

impl fmt::Debug for LandAtlas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LandAtlas")
            .field("graphics", &self.slots.len())
            .field("side", &ATLAS_SIDE)
            .finish()
    }
}

impl LandAtlas {
    /// Pack every graphic in `wanted` that the client actually ships.
    ///
    /// A graphic the client has no art for is skipped, not an error: three
    /// quarters of the land index is genuinely empty, and a map referring to an
    /// empty slot is the file's business, not a failure to draw.
    pub fn build(art: &Art, wanted: impl IntoIterator<Item = Graphic>) -> Result<Self, AtlasError> {
        // Sorted and deduplicated, so the same input always produces the same
        // atlas — a frame that changes because a `HashSet` iterated differently
        // is not a frame a test can assert on.
        let wanted: BTreeSet<Graphic> = wanted.into_iter().collect();
        let mut images = Vec::with_capacity(wanted.len());
        for graphic in wanted {
            if let Some(image) = art.land(graphic)? {
                images.push((graphic, image));
            }
        }
        Self::pack(images)
    }

    /// Pack sprites somebody else decoded.
    ///
    /// What [`LandAtlas::build`] does once it has read the art, and the only way
    /// in that does not need a client install: a test can hand this a picture it
    /// chose and then assert on the pixels the frame comes back with. Every
    /// sprite is expected to be [`LAND_TILE_SIZE`] square, and only the diamond
    /// inside it is copied.
    pub fn pack(images: impl IntoIterator<Item = (Graphic, Image)>) -> Result<Self, AtlasError> {
        let images: BTreeMap<Graphic, Image> = images.into_iter().collect();
        if images.len() > CAPACITY {
            return Err(AtlasError::Full {
                wanted: images.len(),
                capacity: CAPACITY,
            });
        }

        let side = ATLAS_SIDE as usize;
        let mut pixels = vec![0u8; side * side * 4];
        let mut slots = BTreeMap::new();

        for (graphic, image) in images {
            // The grid is the format's constant, so a sprite of another size is
            // a caller's mistake rather than a file's. Said here because the
            // copy below indexes by `land_row`, which only stays inside a 44
            // square.
            assert_eq!(
                (image.width(), image.height()),
                (LAND_TILE_SIZE, LAND_TILE_SIZE),
                "a land sprite is always {LAND_TILE_SIZE} square",
            );
            let slot = slots.len() as u32;
            let (origin_x, origin_y) = slot_origin(slot);

            for y in 0..image.height() {
                // The diamond, not the colours, is what says which pixels exist.
                // Ground has no transparency: a zero pixel inside the diamond is
                // black, and real tiles contain a few. Reading the shape out of
                // the colours instead punches pinholes through the ground that
                // look like dark texture until something counts them.
                for x in land_row(y) {
                    // `pixel` is `None` only outside the image, and `land_row`
                    // stays inside it.
                    let color = image.pixel(x, y).unwrap();
                    let at =
                        ((origin_y + u32::from(y)) as usize * side + (origin_x + u32::from(x)) as usize) * 4;
                    let (r, g, b) = color.rgb8();
                    pixels[at] = r;
                    pixels[at + 1] = g;
                    pixels[at + 2] = b;
                    pixels[at + 3] = u8::MAX;
                }
            }
            slots.insert(graphic, slot);
        }

        Ok(Self { slots, pixels })
    }

    /// The atlas texture's side in pixels. Square.
    pub const fn side() -> u32 {
        ATLAS_SIDE
    }

    /// Its pixels, RGBA8 and row-major, ready for `write_texture`.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// How many graphics landed in it.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether nothing did.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Where a graphic sits, or `None` if the client ships no art for it.
    pub fn region(&self, graphic: Graphic) -> Option<Region> {
        let slot = *self.slots.get(&graphic)?;
        let (x, y) = slot_origin(slot);
        let side = ATLAS_SIDE as f32;
        let tile = LAND_TILE_SIZE as f32;
        Some(Region {
            u: x as f32 / side,
            v: y as f32 / side,
            du: tile / side,
            dv: tile / side,
        })
    }
}

/// The top-left pixel of a slot. Row-major, which is why slot 0 is the origin.
fn slot_origin(slot: u32) -> (u32, u32) {
    let tile = LAND_TILE_SIZE as u32;
    ((slot % SLOTS_PER_ROW) * tile, (slot / SLOTS_PER_ROW) * tile)
}

/// The texture-map atlas's grid is this many pixels on a side.
///
/// The smaller of the two sizes `texmaps.mul` holds. A 128 texture takes a 2x2
/// block of cells, which is what makes this a grid at all rather than a
/// bin-packing problem: every texture is a whole number of cells.
const TEXMAP_CELL: u32 = 64;

/// Cells across the texture atlas, and down.
const TEXMAP_CELLS_PER_ROW: u32 = ATLAS_SIDE / TEXMAP_CELL;

/// Cells one texture atlas holds. A 64 texture takes one and a 128 takes four,
/// so the number of *textures* that fit depends on what they are.
pub const TEXMAP_CELLS: usize = (TEXMAP_CELLS_PER_ROW * TEXMAP_CELLS_PER_ROW) as usize;

/// The square textures a sloped tile is stretched over, packed into one texture.
///
/// Keyed by the *land graphic*, not by the texture id: the id is `tiledata`'s
/// business and resolving it here means a quad asks this and [`LandAtlas`] the
/// same question. Two graphics sharing a texture id therefore hold two copies of
/// it, which costs a cell each and keeps the lookup one map deep.
pub struct TexmapAtlas {
    regions: BTreeMap<Graphic, Region>,
    /// `ATLAS_SIDE * ATLAS_SIDE` RGBA8 pixels, row-major.
    pixels: Vec<u8>,
}

impl fmt::Debug for TexmapAtlas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TexmapAtlas")
            .field("graphics", &self.regions.len())
            .field("side", &ATLAS_SIDE)
            .finish()
    }
}

impl TexmapAtlas {
    /// Pack the texture of every graphic in `wanted` that has one.
    ///
    /// A land graphic with no texture is skipped rather than refused, and it is
    /// the ordinary case: the client ships 4,116 textures for 16,384 slots, and
    /// a tile without one is drawn from its flat art however the ground stands.
    pub fn build(
        texmaps: &TexMaps,
        tiledata: &TileData,
        wanted: impl IntoIterator<Item = Graphic>,
    ) -> Result<Self, AtlasError> {
        // Sorted and deduplicated for the same reason as the land atlas: the
        // same input has to produce the same atlas, byte for byte.
        let wanted: BTreeSet<Graphic> = wanted.into_iter().collect();
        let mut images = Vec::new();
        for graphic in wanted {
            // The indirection this whole atlas exists to follow: a land graphic
            // names a `tiledata` entry, and that entry names a texture.
            let id = tiledata.land(graphic.0).texture;
            if let Some(image) = texmaps.texture(id)? {
                images.push((graphic, image));
            }
        }
        Self::pack(images)
    }

    /// Pack textures somebody else decoded, largest first.
    ///
    /// Largest first is what keeps the grid simple: a 128 needs a free 2x2 block
    /// and the 64s would otherwise have scattered themselves through every one
    /// of them. Deterministic given the same input, which the frame tests rely
    /// on.
    pub fn pack(images: impl IntoIterator<Item = (Graphic, Image)>) -> Result<Self, AtlasError> {
        let images: BTreeMap<Graphic, Image> = images.into_iter().collect();
        let mut order: Vec<(Graphic, Image)> = images.into_iter().collect();
        // Ties broken by graphic, which `BTreeMap` already ordered them by, and
        // `sort_by_key` is stable — so this is a total order and not a
        // "whichever the sort happened to visit first".
        order.sort_by_key(|(_, image)| std::cmp::Reverse(image.width()));

        let side = ATLAS_SIDE as usize;
        let mut pixels = vec![0u8; side * side * 4];
        let mut regions = BTreeMap::new();
        let mut grid = CellGrid::new();

        let wanted = order.len();
        for (graphic, image) in order {
            // Square, and a whole number of cells: both are the format's, and
            // `texmaps` has already refused anything else.
            assert_eq!(image.width(), image.height(), "a texture map is square");
            let span = image.width() as u32 / TEXMAP_CELL;
            assert!(
                span >= 1 && u32::from(image.width()) % TEXMAP_CELL == 0,
                "a {}-pixel texture is not a whole number of {TEXMAP_CELL}-pixel cells",
                image.width(),
            );
            let Some((cell_x, cell_y)) = grid.take(span) else {
                return Err(AtlasError::Full {
                    wanted,
                    capacity: TEXMAP_CELLS,
                });
            };
            let (origin_x, origin_y) = (cell_x * TEXMAP_CELL, cell_y * TEXMAP_CELL);

            // The whole square, corner to corner. A texture has no transparency
            // and no shape to recover: unlike a land sprite, every pixel of it
            // is drawn, zero words included.
            for y in 0..image.height() {
                for x in 0..image.width() {
                    let color = image.pixel(x, y).expect("inside the image");
                    let at =
                        ((origin_y + u32::from(y)) as usize * side + (origin_x + u32::from(x)) as usize) * 4;
                    let (r, g, b) = color.rgb8();
                    pixels[at] = r;
                    pixels[at + 1] = g;
                    pixels[at + 2] = b;
                    pixels[at + 3] = u8::MAX;
                }
            }

            // Half a texel in on every side, which is ClassicUO's
            // `CalculateHalfPixelUVs` and is not a nicety. A sloped quad's corner
            // texture coordinates are the region's own edges, and an edge is the
            // boundary *between* two texels: at `u + du` the sample lands on the
            // first texel of whatever was packed next door. Inset, the four
            // corners sample texel centres — 0.5 and side-0.5 — so the picture is
            // its own and nothing bleeds along the two far edges of every tile.
            let atlas = ATLAS_SIDE as f32;
            let half = 0.5 / atlas;
            regions.insert(
                graphic,
                Region {
                    u: origin_x as f32 / atlas + half,
                    v: origin_y as f32 / atlas + half,
                    du: f32::from(image.width()) / atlas - 2.0 * half,
                    dv: f32::from(image.height()) / atlas - 2.0 * half,
                },
            );
        }

        Ok(Self { regions, pixels })
    }

    /// The atlas texture's side in pixels. Square, and the same as the land
    /// atlas's — one constant, one ceiling.
    pub const fn side() -> u32 {
        ATLAS_SIDE
    }

    /// Its pixels, RGBA8 and row-major, ready for `write_texture`.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// How many graphics landed in it.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// Whether nothing did.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Where to sample a graphic's texture, or `None` if it has none.
    ///
    /// Where to *sample*, not where it sits: the region is half a texel inside
    /// the texture on every side, because a quad's corners sample the region's
    /// edges and an edge belongs to two texels. See [`TexmapAtlas::pack`].
    ///
    /// `None` is the common answer and means "draw this tile from its art",
    /// which is what the client does with a tile whose texture is missing — see
    /// `ground.wgsl`.
    pub fn region(&self, graphic: Graphic) -> Option<Region> {
        self.regions.get(&graphic).copied()
    }
}

/// One packed static sprite: where it is, and how big it is.
///
/// The size travels with the region because a static's quad *is* its sprite —
/// unlike ground, whose quad is 44 square whatever the art holds — so whoever
/// places the quad needs the pixels, and reading them back out of a normalised
/// region is a multiplication that can disagree with the one that produced it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sprite {
    /// Where to sample it.
    pub region: Region,
    /// Its width in pixels.
    pub width: u16,
    /// Its height in pixels.
    pub height: u16,
}

/// Static art, packed into one texture.
///
/// The third atlas and the first irregular one: a static sprite is any size from
/// a 2x2 pebble to a 250-pixel tree, so neither the land grid nor the texture
/// map's cells apply. Shelf packing is what fits — sort by height, fill a row,
/// start the next one below the tallest sprite in it — which is not optimal and
/// does not need to be: a screen of Britain holds a few hundred distinct
/// graphics and the waste is a few percent of one 2048 texture.
///
/// Keyed by the *static* graphic, which is `tiledata`'s static index and the
/// number a `map`'s static item carries. That is a different index space from
/// the land graphic [`LandAtlas`] is keyed by, and the two overlap numerically —
/// which is exactly why they are separate atlases rather than one with a prefix.
pub struct StaticAtlas {
    sprites: BTreeMap<Graphic, Sprite>,
    /// `ATLAS_SIDE * ATLAS_SIDE` RGBA8 pixels, row-major.
    pixels: Vec<u8>,
}

impl fmt::Debug for StaticAtlas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticAtlas")
            .field("graphics", &self.sprites.len())
            .field("side", &ATLAS_SIDE)
            .finish()
    }
}

impl StaticAtlas {
    /// Pack every graphic in `wanted` that the client actually ships.
    ///
    /// A graphic with no art is skipped rather than refused, the same way the
    /// land atlas skips an empty land slot: a map naming a static the client has
    /// no picture for is the file's business.
    pub fn build(art: &Art, wanted: impl IntoIterator<Item = Graphic>) -> Result<Self, AtlasError> {
        let wanted: BTreeSet<Graphic> = wanted.into_iter().collect();
        let mut images = Vec::with_capacity(wanted.len());
        for graphic in wanted {
            if let Some(image) = art.static_art(graphic)? {
                images.push((graphic, image));
            }
        }
        Self::pack(images)
    }

    /// Pack sprites somebody else decoded, tallest first.
    ///
    /// Tallest first is what makes a shelf worth using at all: rows started by a
    /// short sprite waste the whole difference under every tall one that lands
    /// beside it. Deterministic given the same input — same order in, same
    /// pixels out — which the frame tests depend on.
    pub fn pack(images: impl IntoIterator<Item = (Graphic, Image)>) -> Result<Self, AtlasError> {
        // Deduplicated and ordered by graphic first, so the sort below is a
        // total order rather than "whichever the caller happened to yield
        // first" — `sort_by_key` is stable and the tie-break is the graphic.
        let images: BTreeMap<Graphic, Image> = images.into_iter().collect();
        let wanted = images.len();
        let mut order: Vec<(Graphic, Image)> = images.into_iter().collect();
        order.sort_by_key(|(_, image)| std::cmp::Reverse(image.height()));

        let side = ATLAS_SIDE as usize;
        let mut pixels = vec![0u8; side * side * 4];
        let mut sprites = BTreeMap::new();
        let mut shelf = Shelf::default();

        for (graphic, image) in order {
            let (width, height) = (image.width(), image.height());
            // A sprite wider or taller than the whole atlas cannot be packed at
            // any offset. The client ships nothing near it — the tallest art is
            // around 250 pixels — so this is a corrupt-file case rather than a
            // capacity one, and it says which graphic.
            if u32::from(width) > ATLAS_SIDE || u32::from(height) > ATLAS_SIDE {
                return Err(AtlasError::Oversized {
                    graphic,
                    width,
                    height,
                });
            }
            let Some((origin_x, origin_y)) = shelf.take(u32::from(width), u32::from(height)) else {
                return Err(AtlasError::Full {
                    wanted,
                    capacity: sprites.len(),
                });
            };

            // Every pixel, transparent ones included: a static sprite genuinely
            // has transparency — it is a picture with a shape, not a diamond
            // with a known one — and the alpha channel is what the fragment
            // shader discards on.
            //
            // Zero *is* absent here, which is the opposite of the rule for land
            // art and is the client's own: `ArtLoader.ReadStaticArt` writes a
            // run's pixel only `if (val != 0)`, leaving the rest of the buffer
            // at zero alpha. So a zero inside a run and a column no run covered
            // are the same thing to the client, and `Color16::TRANSPARENT` for
            // both loses nothing.
            copy_sprite(&mut pixels, &image, origin_x, origin_y);

            sprites.insert(
                graphic,
                Sprite {
                    region: region_at(origin_x, origin_y, width, height),
                    width,
                    height,
                },
            );
        }

        Ok(Self { sprites, pixels })
    }

    /// The atlas texture's side in pixels. Square, and the same as the others'.
    pub const fn side() -> u32 {
        ATLAS_SIDE
    }

    /// Its pixels, RGBA8 and row-major, ready for `write_texture`.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// How many graphics landed in it.
    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    /// Whether nothing did.
    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
    }

    /// Where a graphic sits and how big it is, or `None` if it is not packed.
    pub fn sprite(&self, graphic: Graphic) -> Option<Sprite> {
        self.sprites.get(&graphic).copied()
    }
}

/// Which picture of an animation this is.
///
/// A body alone is not a sprite: it is a body, an action, a facing and a moment
/// in that action, and the file is indexed by exactly that tuple. Carried as one
/// value so an atlas can be keyed by it — and so that a caller cannot pass the
/// group where the direction goes, which the file would answer with somebody
/// else's frames rather than with nothing.
///
/// The direction is the *stored* one, 0 to 4: the other three facings are
/// mirrors of these and share their pictures, so they share an atlas entry too.
/// Mirroring is [`SpriteQuad::mirrored`](crate::sprite::SpriteQuad::mirrored)
/// and it happens where the quad is built.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct FrameKey {
    /// The body id.
    pub body: u16,
    /// Which animation group — standing, walking, attacking.
    pub group: u8,
    /// The stored direction, 0 to 4.
    pub direction: u8,
    /// Which frame of that animation.
    pub frame: u16,
}

/// One packed animation frame: where it is, how big, and where the feet are.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PackedFrame {
    /// Where to sample it, and how big it is.
    pub sprite: Sprite,
    /// The frame's own centre offsets, carried through unchanged.
    ///
    /// They are not the middle of the picture and they are not the atlas's
    /// business: a walking frame leans, and the lean lives in these two numbers
    /// rather than in the pixels. See [`AnimFrame`].
    pub center_x: i16,
    /// The vertical half of the same pair.
    pub center_y: i16,
}

/// Animation frames, packed into one texture.
///
/// The same shelf packing [`StaticAtlas`] uses, keyed by [`FrameKey`] instead of
/// a graphic. Separate from the statics atlas rather than sharing one: a screen
/// holds a few hundred static graphics and a handful of mobiles, they are
/// rebuilt on completely different triggers — the camera moving against a
/// creature turning — and a draw call binds one texture either way.
pub struct AnimAtlas {
    frames: BTreeMap<FrameKey, PackedFrame>,
    /// `ATLAS_SIDE * ATLAS_SIDE` RGBA8 pixels, row-major.
    pixels: Vec<u8>,
}

impl fmt::Debug for AnimAtlas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnimAtlas")
            .field("frames", &self.frames.len())
            .field("side", &ATLAS_SIDE)
            .finish()
    }
}

impl AnimAtlas {
    /// Read and pack every frame of the bodies, groups and directions asked for.
    ///
    /// `wanted` is body-group-direction triples, each of which brings its whole
    /// animation: a caller that wanted one frame of a walk would still have to
    /// read the entry the others are in, so packing them all costs nothing but
    /// atlas space and saves re-reading 195MB the moment the frame advances.
    ///
    /// A triple the client ships no animation for is skipped, not refused. Most
    /// of the index is empty — see [`Anim`] — and a body without a group is the
    /// ordinary case rather than a failure.
    pub fn build(
        anim: &mut Anim,
        wanted: impl IntoIterator<Item = (u16, u8, u8)>,
    ) -> Result<Self, AtlasError> {
        // Sorted and deduplicated, so the same request always packs the same
        // atlas — the frame tests depend on it, and so does not re-reading a
        // body twice because the caller listed it twice.
        let wanted: BTreeSet<(u16, u8, u8)> = wanted.into_iter().collect();
        let mut images = Vec::new();
        for (body, group, direction) in wanted {
            let Some(frames) = anim.frames(body, group, direction)? else {
                continue;
            };
            for (index, frame) in frames.into_iter().enumerate() {
                // A blank frame is a real thing in these files, and it packs to
                // nothing: an empty picture has no pixels to copy and no region
                // worth handing back.
                if frame.image.width() == 0 || frame.image.height() == 0 {
                    continue;
                }
                images.push((
                    FrameKey {
                        body,
                        group,
                        direction,
                        frame: index as u16,
                    },
                    frame,
                ));
            }
        }
        Self::pack(images)
    }

    /// Pack frames somebody else decoded.
    ///
    /// The way in that needs no client install, exactly as
    /// [`StaticAtlas::pack`] is: a test hands this the pictures it chose and
    /// then asserts on the pixels the frame comes back with.
    pub fn pack(frames: impl IntoIterator<Item = (FrameKey, AnimFrame)>) -> Result<Self, AtlasError> {
        let frames: BTreeMap<FrameKey, AnimFrame> = frames.into_iter().collect();
        let wanted = frames.len();
        let mut order: Vec<(FrameKey, AnimFrame)> = frames.into_iter().collect();
        order.sort_by_key(|(_, frame)| std::cmp::Reverse(frame.image.height()));

        let side = ATLAS_SIDE as usize;
        let mut pixels = vec![0u8; side * side * 4];
        let mut packed = BTreeMap::new();
        let mut shelf = Shelf::default();

        for (key, frame) in order {
            let image = &frame.image;
            let (width, height) = (image.width(), image.height());
            if u32::from(width) > ATLAS_SIDE || u32::from(height) > ATLAS_SIDE {
                return Err(AtlasError::Oversized {
                    // Reported as the body, which is the only part of the key
                    // a `Graphic` can carry and the part worth naming.
                    graphic: Graphic(key.body),
                    width,
                    height,
                });
            }
            let Some((origin_x, origin_y)) = shelf.take(u32::from(width), u32::from(height)) else {
                return Err(AtlasError::Full {
                    wanted,
                    capacity: packed.len(),
                });
            };

            copy_sprite(&mut pixels, image, origin_x, origin_y);
            packed.insert(
                key,
                PackedFrame {
                    sprite: Sprite {
                        region: region_at(origin_x, origin_y, width, height),
                        width,
                        height,
                    },
                    center_x: frame.center_x,
                    center_y: frame.center_y,
                },
            );
        }

        Ok(Self {
            frames: packed,
            pixels,
        })
    }

    /// The atlas texture's side in pixels. Square, like every other atlas here.
    pub const fn side() -> u32 {
        ATLAS_SIDE
    }

    /// Its pixels, RGBA8 and row-major, ready for `write_texture`.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// How many frames landed in it.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether nothing did.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// One frame, or `None` if it was never packed.
    pub fn frame(&self, key: FrameKey) -> Option<PackedFrame> {
        self.frames.get(&key).copied()
    }

    /// How many frames a body's animation has, as packed.
    ///
    /// What a caller needs to advance one: the count is the animation's, not a
    /// constant, and asking the atlas rather than remembering it is what keeps
    /// "frame 7 of a 6-frame walk" from being expressible.
    pub fn frame_count(&self, body: u16, group: u8, direction: u8) -> u16 {
        let first = FrameKey {
            body,
            group,
            direction,
            frame: 0,
        };
        let last = FrameKey {
            body,
            group,
            direction,
            frame: u16::MAX,
        };
        self.frames.range(first..=last).count() as u16
    }
}

/// Copy a whole picture into an atlas, alpha from the file's own zeroes.
///
/// Shared by the two irregular atlases because they mean the same thing by a
/// transparent pixel: absent. Ground is the exception — there a zero is black —
/// and that copy stays in [`LandAtlas::pack`] where the diamond's shape is.
fn copy_sprite(pixels: &mut [u8], image: &Image, origin_x: u32, origin_y: u32) {
    let side = ATLAS_SIDE as usize;
    for y in 0..image.height() {
        for x in 0..image.width() {
            let color = image.pixel(x, y).expect("inside the image");
            let at = ((origin_y + u32::from(y)) as usize * side + (origin_x + u32::from(x)) as usize) * 4;
            let (r, g, b) = color.rgb8();
            pixels[at] = r;
            pixels[at + 1] = g;
            pixels[at + 2] = b;
            pixels[at + 3] = if color.is_transparent() { 0 } else { u8::MAX };
        }
    }
}

/// The region a picture packed at a pixel origin occupies.
fn region_at(origin_x: u32, origin_y: u32, width: u16, height: u16) -> Region {
    let atlas = ATLAS_SIDE as f32;
    Region {
        u: origin_x as f32 / atlas,
        v: origin_y as f32 / atlas,
        du: f32::from(width) / atlas,
        dv: f32::from(height) / atlas,
    }
}

/// A shelf packer: rows of sprites, each row as tall as its tallest member.
///
/// Deliberately not a general bin packer. Fed tallest-first — which
/// [`StaticAtlas::pack`] guarantees — a shelf's waste is bounded by the height
/// difference *within* a row, and sorted input keeps that small. A better
/// packer would buy a few percent of one texture and cost a data structure
/// nobody can check by hand.
#[derive(Default)]
struct Shelf {
    /// Where the current row starts, from the top of the atlas.
    top: u32,
    /// How far along the current row is filled.
    used: u32,
    /// How tall the current row is, which the next one starts below.
    height: u32,
}

impl Shelf {
    /// Take a `width` x `height` box, or `None` when the atlas is full.
    fn take(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if self.used + width > ATLAS_SIDE {
            // The row is full: drop to the next one. Its height is this row's,
            // which is the tallest sprite that landed in it.
            self.top += self.height;
            self.used = 0;
            self.height = 0;
        }
        if self.top + height > ATLAS_SIDE {
            return None;
        }
        let at = (self.used, self.top);
        self.used += width;
        // Tallest-first means this is only ever set by the row's first sprite,
        // but a caller that fed us unsorted input would still get a correct
        // atlas rather than an overlapping one.
        self.height = self.height.max(height);
        Some(at)
    }
}

/// Which cells of the texture atlas are spoken for.
///
/// A first-fit scan rather than a running index, because the two sizes cannot
/// share one: a 128 needs four cells that form a square, and after one of those
/// the next free *cell* and the next free *block* are different places.
struct CellGrid {
    taken: Vec<bool>,
}

impl CellGrid {
    fn new() -> Self {
        Self {
            taken: vec![false; TEXMAP_CELLS],
        }
    }

    /// Take the first free `span` x `span` block, top-left first, or `None` when
    /// the atlas is full.
    fn take(&mut self, span: u32) -> Option<(u32, u32)> {
        let per_row = TEXMAP_CELLS_PER_ROW;
        for y in 0..per_row.saturating_sub(span - 1) {
            'block: for x in 0..per_row.saturating_sub(span - 1) {
                for dy in 0..span {
                    for dx in 0..span {
                        if self.taken[((y + dy) * per_row + x + dx) as usize] {
                            continue 'block;
                        }
                    }
                }
                for dy in 0..span {
                    for dx in 0..span {
                        self.taken[((y + dy) * per_row + x + dx) as usize] = true;
                    }
                }
                return Some((x, y));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use openshard_uofiles::color::Color16;

    use super::*;

    #[test]
    fn slots_fill_a_row_before_starting_the_next() {
        assert_eq!(slot_origin(0), (0, 0));
        assert_eq!(slot_origin(1), (44, 0));
        assert_eq!(slot_origin(SLOTS_PER_ROW - 1), (44 * (SLOTS_PER_ROW - 1), 0));
        assert_eq!(slot_origin(SLOTS_PER_ROW), (0, 44));
    }

    /// The last slot has to end inside the texture. Off by one here is a sprite
    /// wrapping to the far side of the atlas, which looks like corrupt art.
    #[test]
    fn the_last_slot_fits() {
        let (x, y) = slot_origin(CAPACITY as u32 - 1);
        assert!(x + LAND_TILE_SIZE as u32 <= ATLAS_SIDE);
        assert!(y + LAND_TILE_SIZE as u32 <= ATLAS_SIDE);
    }

    /// A square of one colour, at a side the texture atlas accepts.
    fn texture(side: u16, color: Color16) -> Image {
        Image::new(side, side, vec![color; usize::from(side) * usize::from(side)])
    }

    /// The whole reason the texture atlas is not the land atlas: two sizes in
    /// one grid, and a big one must not be cut in half by a small one that was
    /// packed into the middle of it.
    #[test]
    fn a_large_texture_gets_a_whole_block_and_a_small_one_stays_out_of_it() {
        let atlas = TexmapAtlas::pack([
            (Graphic(1), texture(64, Color16(0x1F))),
            (Graphic(2), texture(128, Color16(0x7C00))),
        ])
        .expect("two textures fit");

        let big = atlas.region(Graphic(2)).expect("packed");
        let small = atlas.region(Graphic(1)).expect("packed");
        let atlas_side = ATLAS_SIDE as f32;
        // A texture of `n` pixels spans the centres of `n` texels, which is
        // `n - 1` apart: the half-texel inset, stated in pixels.
        assert_eq!(
            big.du * atlas_side,
            127.0,
            "a 128 texture spans 128 texel centres"
        );
        assert_eq!(small.du * atlas_side, 63.0);

        // Largest first, so the 128 owns the corner and the 64 is beside it
        // rather than inside it.
        assert_eq!((big.u * atlas_side, big.v * atlas_side), (0.5, 0.5));
        assert!(
            small.u * atlas_side >= 128.0 || small.v * atlas_side >= 128.0,
            "the 64 landed at ({}, {}), inside the block the 128 took",
            small.u * atlas_side,
            small.v * atlas_side,
        );
    }

    /// The inset samples the texture's own first and last texel, and nothing
    /// beyond them. Without it a quad's far corners sample the neighbour packed
    /// next door — a one-texel fringe along two edges of every sloped tile,
    /// which reads as terrain and is somebody else's.
    #[test]
    fn a_regions_corners_sample_the_first_and_last_texel_of_its_own_texture() {
        let atlas = TexmapAtlas::pack([
            (Graphic(1), texture(64, Color16(1))),
            (Graphic(2), texture(64, Color16(2))),
        ])
        .expect("two textures fit");
        let region = atlas.region(Graphic(1)).expect("packed");
        let side = ATLAS_SIDE as f32;

        // What the shader computes at the quad's two extreme corners, in texels.
        let first = region.u * side;
        let last = (region.u + region.du) * side;
        assert_eq!(first.floor(), 0.0);
        assert_eq!(last.floor(), 63.0, "the far corner is not the neighbour's texel");
    }

    /// Every region has to be inside the texture and disjoint from every other,
    /// which the grid gives by construction — and which one wrong bound in
    /// `CellGrid::take` would take away without changing anything visible until
    /// two terrains started sharing a texel.
    #[test]
    fn packed_textures_never_overlap_and_never_leave_the_atlas() {
        // Enough of both sizes to fill several rows, and interleaved so the
        // allocator cannot be right by accident of ordering.
        let images: Vec<(Graphic, Image)> = (0..200u16)
            .map(|i| {
                let side = if i % 3 == 0 { 128 } else { 64 };
                (Graphic(i), texture(side, Color16(i | 1)))
            })
            .collect();
        let atlas = TexmapAtlas::pack(images.clone()).expect("200 textures fit in 1024 cells");

        let mut claimed = vec![None; TEXMAP_CELLS];
        for (graphic, image) in images {
            let region = atlas.region(graphic).expect("packed");
            let side = ATLAS_SIDE as f32;
            // Back out the half-texel inset: the texel the region starts on is
            // the pixel the texture was packed at.
            let (x, y) = ((region.u * side) as u32, (region.v * side) as u32);
            assert_eq!(region.du * side + 1.0, f32::from(image.width()));
            assert!(x + u32::from(image.width()) <= ATLAS_SIDE);
            assert!(y + u32::from(image.height()) <= ATLAS_SIDE);
            assert_eq!((x % TEXMAP_CELL, y % TEXMAP_CELL), (0, 0), "off the cell grid");

            for cy in 0..u32::from(image.height()) / TEXMAP_CELL {
                for cx in 0..u32::from(image.width()) / TEXMAP_CELL {
                    let cell =
                        ((y / TEXMAP_CELL + cy) * TEXMAP_CELLS_PER_ROW + x / TEXMAP_CELL + cx) as usize;
                    assert_eq!(claimed[cell], None, "{graphic:?} overlaps {:?}", claimed[cell]);
                    claimed[cell] = Some(graphic);
                }
            }
        }
    }

    /// A rectangle of one colour, for the shelf packer's tests.
    fn sprite(width: u16, height: u16) -> Image {
        Image::new(
            width,
            height,
            vec![Color16(0x1F); usize::from(width) * usize::from(height)],
        )
    }

    /// The property the shelf packer exists to give: every sprite gets its own
    /// pixels. Overlap here is two statics sharing a picture, which reads as
    /// corrupt art rather than as a packing bug.
    #[test]
    fn packed_sprites_never_overlap_and_never_leave_the_atlas() {
        // Sizes that do not divide the atlas evenly, so a row's leftover is
        // never zero and the wrap to the next shelf is exercised.
        let images: Vec<(Graphic, Image)> = (0..300u16)
            .map(|i| (Graphic(i), sprite(30 + i % 7, 20 + i % 11)))
            .collect();
        let atlas = StaticAtlas::pack(images.clone()).expect("300 small sprites fit");

        let side = ATLAS_SIDE as usize;
        let mut claimed = vec![None; side * side];
        for (graphic, image) in images {
            let packed = atlas.sprite(graphic).expect("packed");
            assert_eq!((packed.width, packed.height), (image.width(), image.height()));
            let x = (packed.region.u * ATLAS_SIDE as f32) as usize;
            let y = (packed.region.v * ATLAS_SIDE as f32) as usize;
            assert!(
                x + usize::from(packed.width) <= side,
                "{graphic:?} runs off the right"
            );
            assert!(
                y + usize::from(packed.height) <= side,
                "{graphic:?} runs off the bottom"
            );
            for row in y..y + usize::from(packed.height) {
                for column in x..x + usize::from(packed.width) {
                    let cell = &mut claimed[row * side + column];
                    assert_eq!(*cell, None, "{graphic:?} overlaps {cell:?}");
                    *cell = Some(graphic);
                }
            }
        }
    }

    /// A static's shape is its alpha, and the alpha comes from the file's zero
    /// pixels. The land atlas does the opposite — there a zero is black — so
    /// this is the one place the two rules meet, and getting it backwards
    /// either punches holes through solid art or draws every sprite's bounding
    /// box as a black rectangle.
    #[test]
    fn a_zero_pixel_is_absent_and_everything_else_is_opaque() {
        let mut pixels = vec![Color16(0x7C00); 4];
        pixels[1] = Color16::TRANSPARENT;
        let atlas = StaticAtlas::pack([(Graphic(1), Image::new(2, 2, pixels))]).expect("fits");
        let packed = atlas.sprite(Graphic(1)).expect("packed");
        let x = (packed.region.u * ATLAS_SIDE as f32) as usize;
        let y = (packed.region.v * ATLAS_SIDE as f32) as usize;
        let alpha = |column: usize, row: usize| {
            atlas.pixels()[((y + row) * ATLAS_SIDE as usize + x + column) * 4 + 3]
        };
        assert_eq!(alpha(0, 0), u8::MAX);
        assert_eq!(alpha(1, 0), 0, "a zero pixel is the sprite's shape, not a colour");
        assert_eq!(alpha(0, 1), u8::MAX);
    }

    /// Tallest first, or a shelf wastes the difference under every tall sprite
    /// that lands beside a short one. Stated as "the tall one is on the first
    /// row", which is the observable consequence.
    #[test]
    fn the_tallest_sprite_starts_the_first_shelf() {
        let atlas = StaticAtlas::pack([
            (Graphic(1), sprite(40, 20)),
            (Graphic(2), sprite(40, 200)),
            (Graphic(3), sprite(40, 60)),
        ])
        .expect("three sprites fit");
        assert_eq!(atlas.sprite(Graphic(2)).expect("packed").region.v, 0.0);
        // And the shorter two share that row rather than starting their own.
        assert_eq!(atlas.sprite(Graphic(3)).expect("packed").region.v, 0.0);
        assert_eq!(atlas.sprite(Graphic(1)).expect("packed").region.v, 0.0);
    }

    /// A sprite bigger than the atlas is its own error, because it is not a
    /// capacity problem: no packing of any kind could place it.
    #[test]
    fn a_sprite_larger_than_the_atlas_says_which_one_it_was() {
        let huge = Image::new(
            1,
            ATLAS_SIDE as u16 + 1,
            vec![Color16(1); ATLAS_SIDE as usize + 1],
        );
        assert!(matches!(
            StaticAtlas::pack([(Graphic(7), huge)]),
            Err(AtlasError::Oversized {
                graphic: Graphic(7),
                ..
            })
        ));
    }

    /// More textures than cells is an error rather than a silent drop: a tile
    /// whose texture quietly vanished is drawn from its art, which looks like
    /// terrain and is the wrong terrain.
    #[test]
    fn an_atlas_that_cannot_hold_them_all_says_so() {
        let images: Vec<(Graphic, Image)> = (0..TEXMAP_CELLS as u16 + 1)
            .map(|i| (Graphic(i), texture(64, Color16(1))))
            .collect();
        assert!(matches!(TexmapAtlas::pack(images), Err(AtlasError::Full { .. })));
    }
}
