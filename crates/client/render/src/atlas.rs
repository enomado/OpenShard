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
    /// The art container refused a graphic.
    Art(ArtError),
    /// The texture maps refused a texture.
    TexMaps(TexMapError),
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full { wanted, capacity } => {
                write!(f, "{wanted} pictures do not fit in an atlas of {capacity}")
            }
            Self::Art(source) => write!(f, "reading land art: {source}"),
            Self::TexMaps(source) => write!(f, "reading a land texture: {source}"),
        }
    }
}

impl std::error::Error for AtlasError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Full { .. } => None,
            Self::Art(source) => Some(source),
            Self::TexMaps(source) => Some(source),
        }
    }
}

impl From<ArtError> for AtlasError {
    fn from(source: ArtError) -> Self {
        Self::Art(source)
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

            let atlas = ATLAS_SIDE as f32;
            regions.insert(
                graphic,
                Region {
                    u: origin_x as f32 / atlas,
                    v: origin_y as f32 / atlas,
                    du: f32::from(image.width()) / atlas,
                    dv: f32::from(image.height()) / atlas,
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

    /// Where a graphic's texture sits, or `None` if it has none.
    ///
    /// `None` is the common answer and means "draw this tile from its art",
    /// which is what the client does with a tile whose texture is missing — see
    /// `ground.wgsl`.
    pub fn region(&self, graphic: Graphic) -> Option<Region> {
        self.regions.get(&graphic).copied()
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
        assert_eq!(big.du * atlas_side, 128.0, "a 128 texture covers 128 pixels");
        assert_eq!(small.du * atlas_side, 64.0);

        // Largest first, so the 128 owns the corner and the 64 is beside it
        // rather than inside it.
        assert_eq!((big.u, big.v), (0.0, 0.0));
        assert!(
            small.u * atlas_side >= 128.0 || small.v * atlas_side >= 128.0,
            "the 64 landed at ({}, {}), inside the block the 128 took",
            small.u * atlas_side,
            small.v * atlas_side,
        );
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
            let (x, y) = ((region.u * side) as u32, (region.v * side) as u32);
            assert_eq!(region.du * side, f32::from(image.width()));
            assert!(x + u32::from(image.width()) <= ATLAS_SIDE);
            assert!(y + u32::from(image.height()) <= ATLAS_SIDE);
            assert_eq!((x % TEXMAP_CELL, y % TEXMAP_CELL), (0, 0), "off the cell grid");

            for cy in 0..u32::from(image.height()) / TEXMAP_CELL {
                for cx in 0..u32::from(image.width()) / TEXMAP_CELL {
                    let cell = ((y / TEXMAP_CELL + cy) * TEXMAP_CELLS_PER_ROW + x / TEXMAP_CELL + cx) as usize;
                    assert_eq!(claimed[cell], None, "{graphic:?} overlaps {:?}", claimed[cell]);
                    claimed[cell] = Some(graphic);
                }
            }
        }
    }

    /// More textures than cells is an error rather than a silent drop: a tile
    /// whose texture quietly vanished is drawn from its art, which looks like
    /// terrain and is the wrong terrain.
    #[test]
    fn an_atlas_that_cannot_hold_them_all_says_so() {
        let images: Vec<(Graphic, Image)> = (0..TEXMAP_CELLS as u16 + 1)
            .map(|i| (Graphic(i), texture(64, Color16(1))))
            .collect();
        assert!(matches!(
            TexmapAtlas::pack(images),
            Err(AtlasError::Full { .. })
        ));
    }
}
