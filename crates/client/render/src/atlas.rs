//! Land sprites, packed into one texture.
//!
//! A draw call can bind one texture, and a screen of ground touches a few
//! hundred different graphics, so they go into a grid: every land tile is
//! exactly 44x44, which makes the packing a division rather than a bin-packing
//! problem, and makes a slot's position something a test can state outright.
//!
//! The atlas is built for a set of graphics, not for the whole file. A modern
//! client ships about 4,244 land tiles and the container is 155MB; what is on
//! screen is a fraction of that, and the browser is the reason the difference
//! matters rather than an optimisation nobody asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use openshard_protocol::wire::Graphic;
use openshard_uofiles::art::{Art, ArtError, LAND_TILE_SIZE, land_row};

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
    /// More distinct graphics than [`CAPACITY`].
    ///
    /// Not a "grow the texture" case: the cap is the web's, so the fix is to
    /// build an atlas per region, or to evict. Failing loudly is the point.
    Full {
        /// How many were asked for.
        wanted: usize,
    },
    /// The art container refused a graphic.
    Art(ArtError),
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full { wanted } => {
                write!(f, "{wanted} land graphics do not fit in an atlas of {CAPACITY}")
            }
            Self::Art(source) => write!(f, "reading land art: {source}"),
        }
    }
}

impl std::error::Error for AtlasError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Full { .. } => None,
            Self::Art(source) => Some(source),
        }
    }
}

impl From<ArtError> for AtlasError {
    fn from(source: ArtError) -> Self {
        Self::Art(source)
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
        if wanted.len() > CAPACITY {
            return Err(AtlasError::Full { wanted: wanted.len() });
        }

        let side = ATLAS_SIDE as usize;
        let mut pixels = vec![0u8; side * side * 4];
        let mut slots = BTreeMap::new();

        for graphic in wanted {
            let Some(image) = art.land(graphic)? else {
                continue;
            };
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

#[cfg(test)]
mod tests {
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
}
