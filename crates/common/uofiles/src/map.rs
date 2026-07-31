//! `map*.mul` and `statics*.mul`: the ground, and everything standing on it.
//!
//! # Block order is column-major, and nothing tells you
//!
//! `map0.mul` is a flat array of 8×8 blocks indexed
//! `block_x * (height_in_blocks) + block_y`. Column-major — x is the *outer*
//! stride. Get it the other way round and the file still parses, every block is
//! still 196 bytes, and every read lands somewhere plausible. The map is simply
//! transposed, and you find out when a player walks into an ocean that should be
//! a coastline. Sphere's `CServerMap.cpp:445` is the authority.
//!
//! # The map size is not in the file either
//!
//! `map0.mul` has no header. The only thing that says how wide a facet is, is
//! the file's own length divided by the block size. A modern map0 is 7168×4096
//! — the post-ML expansion — not the 6144×4096 of every tutorial.
//!
//! # And the block count does not always name one facet
//!
//! Malas is 2560×4096/8² and Ter Mur is 1280×4096/8², and both come to **81,920
//! blocks**. The two files are the same length, neither carries a header, and
//! their `staidx` files are the same length too — so every check the block count
//! can make passes for the wrong one. Loading Ter Mur as Malas is the exact
//! failure the block-order note above describes: 512 blocks per column read as
//! 256, so everything past the first column lands somewhere else, and it parses
//! perfectly. The facet's *number* is the only thing that separates them, which
//! is why [`Map::load_facet`] carries it into the size decision and [`Map::load`]
//! — which has only a path — cannot.

use std::fmt;
use std::path::{Path, PathBuf};

use openshard_protocol::world::Point;

/// Tiles along each side of a map block.
pub const BLOCK_SIZE: u32 = 8;
/// Cells in a block.
const CELLS_PER_BLOCK: usize = (BLOCK_SIZE * BLOCK_SIZE) as usize;
/// A cell: `u16` tile id and an `i8` height. Sphere's `CUOMapMeter`.
const CELL_BYTES: usize = 3;
/// Every block carries a 4-byte header that nothing reads.
const BLOCK_HEADER: usize = 4;
/// Bytes per block on disk.
pub const BLOCK_BYTES: usize = BLOCK_HEADER + CELLS_PER_BLOCK * CELL_BYTES;
/// Bytes per `staidx` entry: offset, length, extra.
const STAIDX_ENTRY: usize = 12;
/// Bytes per static on disk: tile id, x, y, z, hue.
const STATIC_BYTES: usize = 7;

/// One cell of ground.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LandCell {
    /// Index into the land table of `tiledata.mul`.
    pub tile: u16,
    /// The ground's height here.
    pub z: i8,
}

/// One thing standing on the ground.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StaticItem {
    /// Index into the static table of `tiledata.mul`.
    pub tile: u16,
    /// Where in the world, not in the block: resolved on load.
    pub x: u16,
    /// Where in the world.
    pub y: u16,
    /// Its base height. What you stand on is this plus the tile's height.
    pub z: i8,
    /// Its colour.
    pub hue: u16,
}

/// The known sizes of a Britannia facet, in tiles.
///
/// Only used to name what was found; the size itself comes from the file.
fn describe_size(width: u32, height: u32) -> &'static str {
    match (width, height) {
        (6144, 4096) => "Felucca/Trammel (classic)",
        (7168, 4096) => "Felucca/Trammel (post-ML)",
        (2304, 1600) => "Ilshenar",
        (2560, 2048) => "Malas",
        (1448, 1448) => "Tokuno",
        (1280, 4096) => "Ter Mur",
        _ => "unknown facet",
    }
}

/// A map file could not be read.
#[derive(Debug)]
#[non_exhaustive]
pub enum MapError {
    /// A file could not be read.
    Read {
        /// Which file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The file does not divide into whole blocks, so it is not a map.
    NotABlockMap {
        /// Which file.
        path: PathBuf,
        /// How big it is.
        size: usize,
    },
    /// The block count does not factor into any known facet.
    UnknownFacet {
        /// Which file.
        path: PathBuf,
        /// How many blocks it holds.
        blocks: usize,
    },
    /// The UOP container could not be read.
    Uop {
        /// Which file.
        path: PathBuf,
        /// Why.
        source: Box<crate::uop::UopError>,
    },
    /// `staidx` and `map` disagree about how many blocks there are.
    IndexMismatch {
        /// Blocks in the map.
        map_blocks: usize,
        /// Entries in the index.
        index_entries: usize,
    },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::NotABlockMap { path, size } => write!(
                f,
                "{} is {size} bytes, which is not a whole number of {BLOCK_BYTES}-byte blocks",
                path.display()
            ),
            Self::UnknownFacet { path, blocks } => write!(
                f,
                "{} holds {blocks} blocks, which is not the size of any known facet",
                path.display()
            ),
            Self::Uop { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::IndexMismatch {
                map_blocks,
                index_entries,
            } => write!(
                f,
                "the map has {map_blocks} blocks but staidx has {index_entries} entries; \
                 they are from different clients"
            ),
        }
    }
}

impl std::error::Error for MapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Uop { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// One facet: the ground and the statics on it.
///
/// The whole thing is in memory. That is the design — the database is never
/// touched inside a tick, and a facet is under 100MB.
pub struct Map {
    width: u32,
    height: u32,
    /// Land, block-ordered exactly as on disk.
    cells: Vec<LandCell>,
    /// Statics per block, indexed the same way as `cells`.
    statics: Vec<Vec<StaticItem>>,
}

impl fmt::Debug for Map {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Map")
            .field("size", &format!("{}x{}", self.width, self.height))
            .field("facet", &describe_size(self.width, self.height))
            .field("statics", &self.statics.iter().map(Vec::len).sum::<usize>())
            .finish()
    }
}

impl Map {
    /// Load a facet from a path alone, working out its size from the file.
    ///
    /// `statics` is optional: a map with no statics is bare ground, which is
    /// wrong but not unusable, and being able to load one makes the map testable
    /// on its own.
    ///
    /// # This cannot tell Malas from Ter Mur
    ///
    /// They are the same number of blocks and a path is not a facet number, so
    /// an 81,920-block file loads as Malas. Prefer [`Map::load_facet`], which
    /// knows which facet it asked for; this exists for a file that is not in a
    /// client install at all.
    pub fn load(
        map_path: impl AsRef<Path>,
        statics_paths: Option<(impl AsRef<Path>, impl AsRef<Path>)>,
    ) -> Result<Self, MapError> {
        let map_path = map_path.as_ref();
        let bytes = read(map_path)?;
        Self::from_bytes(map_path, bytes, statics_paths, None)
    }

    /// Load a facet, preferring the UOP container over the `.mul`.
    ///
    /// # Why the `.mul` is the fallback and not the source
    ///
    /// Modern clients ship both, and the `.mul` may be a stub full of zeroes.
    /// It is the right size, it parses perfectly, and it describes a flat empty
    /// world. Reading it produces no error and no map.
    ///
    /// So: if `<name>LegacyMUL.uop` exists next to `<name>.mul`, it wins. That
    /// is the file the client itself reads.
    pub fn load_facet(client_dir: impl AsRef<Path>, facet: u8) -> Result<Self, MapError> {
        let dir = client_dir.as_ref();
        let uop = dir.join(format!("map{facet}LegacyMUL.uop"));
        let statics = Some((
            dir.join(format!("staidx{facet}.mul")),
            dir.join(format!("statics{facet}.mul")),
        ));

        // Whichever file was actually read is the one an error should name.
        let (source_path, bytes) = if uop.exists() {
            let pattern = |index: usize| format!("build/map{facet}legacymul/{index:08}.dat");
            let bytes = crate::uop::read_concatenated(&uop, &pattern).map_err(|source| MapError::Uop {
                path: uop.clone(),
                source: Box::new(source),
            })?;
            (uop, bytes)
        } else {
            let mul = dir.join(format!("map{facet}.mul"));
            let bytes = read(&mul)?;
            (mul, bytes)
        };

        Self::from_bytes(&source_path, bytes, statics, Some(facet))
    }

    /// `facet` is the facet number when the caller knows it, and is what breaks
    /// the Malas/Ter Mur tie described in this module's header.
    fn from_bytes(
        map_path: &Path,
        mut bytes: Vec<u8>,
        statics_paths: Option<(impl AsRef<Path>, impl AsRef<Path>)>,
        facet: Option<u8>,
    ) -> Result<Self, MapError> {
        // A UOP container is allocated in fixed chunks and comes out a block or
        // two longer than the facet. Trim to the largest whole facet that fits
        // rather than refusing: the tail is padding, not data.
        if let Some(size) = largest_facet_within(bytes.len() / BLOCK_BYTES, facet) {
            bytes.truncate(size * BLOCK_BYTES);
        }

        if !bytes.len().is_multiple_of(BLOCK_BYTES) || bytes.is_empty() {
            return Err(MapError::NotABlockMap {
                path: map_path.to_owned(),
                size: bytes.len(),
            });
        }
        let blocks = bytes.len() / BLOCK_BYTES;
        let (width, height) = facet_size(blocks, facet).ok_or_else(|| MapError::UnknownFacet {
            path: map_path.to_owned(),
            blocks,
        })?;
        let bytes = &bytes[..];

        let mut cells = Vec::with_capacity(blocks * CELLS_PER_BLOCK);
        for block in 0..blocks {
            let base = block * BLOCK_BYTES + BLOCK_HEADER;
            for cell in 0..CELLS_PER_BLOCK {
                let at = base + cell * CELL_BYTES;
                cells.push(LandCell {
                    // Little-endian: the files are, the network is not.
                    tile: u16::from_le_bytes([bytes[at], bytes[at + 1]]),
                    z: bytes[at + 2] as i8,
                });
            }
        }

        let statics = match statics_paths {
            Some((index_path, data_path)) => {
                Self::load_statics(index_path.as_ref(), data_path.as_ref(), blocks, height)?
            }
            None => vec![Vec::new(); blocks],
        };

        Ok(Self {
            width,
            height,
            cells,
            statics,
        })
    }

    fn load_statics(
        index_path: &Path,
        data_path: &Path,
        blocks: usize,
        height: u32,
    ) -> Result<Vec<Vec<StaticItem>>, MapError> {
        let index = read(index_path)?;
        let data = read(data_path)?;

        let entries = index.len() / STAIDX_ENTRY;
        if entries != blocks {
            return Err(MapError::IndexMismatch {
                map_blocks: blocks,
                index_entries: entries,
            });
        }

        let blocks_down = height / BLOCK_SIZE;
        let mut out: Vec<Vec<StaticItem>> = vec![Vec::new(); blocks];
        for (block, slot) in out.iter_mut().enumerate() {
            let at = block * STAIDX_ENTRY;
            let offset = u32::from_le_bytes([index[at], index[at + 1], index[at + 2], index[at + 3]]);
            let length = u32::from_le_bytes([index[at + 4], index[at + 5], index[at + 6], index[at + 7]]);

            // 0xFFFFFFFF means "no statics here", and it is the common case —
            // most of Britannia is empty ground. A length that runs past the end
            // of the file means a truncated download, and reading it would
            // panic, so both are simply "nothing here".
            if offset == u32::MAX || length == u32::MAX || length == 0 {
                continue;
            }
            let (Ok(offset), Ok(length)) = (usize::try_from(offset), usize::try_from(length)) else {
                continue;
            };
            let Some(chunk) = data.get(offset..offset + length) else {
                continue;
            };

            // Block index is column-major, so recovering the block's world
            // origin has to undo the same formula.
            let block_x = (block / blocks_down as usize) as u32 * BLOCK_SIZE;
            let block_y = (block % blocks_down as usize) as u32 * BLOCK_SIZE;

            let mut items = Vec::with_capacity(chunk.len() / STATIC_BYTES);
            for entry in chunk.chunks_exact(STATIC_BYTES) {
                items.push(StaticItem {
                    tile: u16::from_le_bytes([entry[0], entry[1]]),
                    // The file stores an offset within the block; a world
                    // coordinate is more use to everyone downstream.
                    x: (block_x + u32::from(entry[2] & 0x7)) as u16,
                    y: (block_y + u32::from(entry[3] & 0x7)) as u16,
                    z: entry[4] as i8,
                    hue: u16::from_le_bytes([entry[5], entry[6]]),
                });
            }
            *slot = items;
        }
        Ok(out)
    }

    /// The facet's width in tiles.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The facet's height in tiles.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// What this facet appears to be.
    pub fn facet_name(&self) -> &'static str {
        describe_size(self.width, self.height)
    }

    /// Whether a point is on the map at all.
    pub const fn contains(&self, x: u16, y: u16) -> bool {
        (x as u32) < self.width && (y as u32) < self.height
    }

    /// Where a tile's block starts in `cells`.
    ///
    /// Column-major, from Sphere's `CServerMap.cpp:445`:
    /// `bx * (SizeY / UO_BLOCK_SIZE) + by`.
    fn cell_index(&self, x: u16, y: u16) -> Option<usize> {
        if !self.contains(x, y) {
            return None;
        }
        let (x, y) = (u32::from(x), u32::from(y));
        let blocks_down = self.height / BLOCK_SIZE;
        let block = (x / BLOCK_SIZE) * blocks_down + (y / BLOCK_SIZE);
        // Within the block, Sphere reads `m_Meter[yo * UO_BLOCK_SIZE + xo]`.
        let cell = (y % BLOCK_SIZE) * BLOCK_SIZE + (x % BLOCK_SIZE);
        Some(block as usize * CELLS_PER_BLOCK + cell as usize)
    }

    /// The ground at a point, or `None` off the map.
    pub fn land(&self, x: u16, y: u16) -> Option<LandCell> {
        self.cells.get(self.cell_index(x, y)?).copied()
    }

    /// Every static standing on a point.
    ///
    /// Scans the point's block, which holds at most a few dozen. A per-tile
    /// index would be faster and is not worth it until something proves
    /// otherwise.
    pub fn statics_at(&self, x: u16, y: u16) -> impl Iterator<Item = &StaticItem> + '_ {
        let block = self.block_index(x, y);
        block
            .and_then(|block| self.statics.get(block))
            .into_iter()
            .flatten()
            .filter(move |item| item.x == x && item.y == y)
    }

    fn block_index(&self, x: u16, y: u16) -> Option<usize> {
        if !self.contains(x, y) {
            return None;
        }
        let blocks_down = self.height / BLOCK_SIZE;
        Some(((u32::from(x) / BLOCK_SIZE) * blocks_down + (u32::from(y) / BLOCK_SIZE)) as usize)
    }

    /// How many statics the facet holds.
    pub fn static_count(&self) -> usize {
        self.statics.iter().map(Vec::len).sum()
    }

    /// A point on the ground, for a caller that only has x and y.
    pub fn ground(&self, x: u16, y: u16) -> Option<Point> {
        self.land(x, y).map(|cell| Point::new(x, y, cell.z))
    }
}

fn read(path: &Path) -> Result<Vec<u8>, MapError> {
    std::fs::read(path).map_err(|source| MapError::Read {
        path: path.to_owned(),
        source,
    })
}

/// How many blocks a facet of this shape holds.
const fn blocks_in(width: u32, height: u32) -> usize {
    ((width / BLOCK_SIZE) * (height / BLOCK_SIZE)) as usize
}

/// The largest whole facet that fits in `blocks`, so a padded UOP can be
/// trimmed to it.
///
/// Bounded by `facet`'s own shapes when the caller knows which facet this is:
/// trimming to a shape this facet could never be would be padding removed by
/// coincidence.
fn largest_facet_within(blocks: usize, facet: Option<u8>) -> Option<usize> {
    candidate_shapes(facet)
        .iter()
        .map(|(width, height)| blocks_in(*width, *height))
        .filter(|size| *size <= blocks)
        .max()
}

/// Every facet shape a client ships, largest first.
const KNOWN_FACETS: [(u32, u32); 6] = [
    (7168, 4096),
    (6144, 4096),
    (2560, 2048),
    (2304, 1600),
    (1448, 1448),
    (1280, 4096),
];

/// The shapes each facet number is allowed to be.
///
/// Britannia is listed twice because it grew: map0 and map1 were 6144 wide until
/// Mondain's Legacy widened them to 7168, and a client of either age is a client
/// somebody runs. Every other facet has exactly one shape, and that is what
/// makes this table able to answer a question the block count cannot — Malas and
/// Ter Mur are both 81,920 blocks, and only their *number* tells them apart.
const FACET_SHAPES: [&[(u32, u32)]; 6] = [
    &[(7168, 4096), (6144, 4096)], // 0 Felucca
    &[(7168, 4096), (6144, 4096)], // 1 Trammel
    &[(2304, 1600)],               // 2 Ilshenar
    &[(2560, 2048)],               // 3 Malas
    &[(1448, 1448)],               // 4 Tokuno
    &[(1280, 4096)],               // 5 Ter Mur
];

/// The shapes worth considering for `facet`.
///
/// A facet number past the table is one no client ships. It falls back to every
/// known shape rather than refusing, because the caller has a file in hand and
/// the block count may well still identify it.
fn candidate_shapes(facet: Option<u8>) -> &'static [(u32, u32)] {
    facet
        .and_then(|facet| FACET_SHAPES.get(facet as usize).copied())
        .unwrap_or(&KNOWN_FACETS)
}

/// Work out a facet's dimensions from its block count, and from which facet it
/// is when the count alone cannot say.
///
/// The file has no header, so this is the only source of truth. Anything that
/// matches no candidate is refused rather than guessed, because a wrong guess
/// transposes the map silently.
fn facet_size(blocks: usize, facet: Option<u8>) -> Option<(u32, u32)> {
    candidate_shapes(facet)
        .iter()
        .copied()
        .find(|(width, height)| blocks == blocks_in(*width, *height))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of one test's own, removed when the test ends.
    ///
    /// The fixtures below are written to disk because `Map::load` takes a path,
    /// and a fixed name under `temp_dir()` is shared state: two runs of this
    /// suite at once — a second `cargo test`, or CI's — write and delete each
    /// other's file, and the loser fails on a file that was whole a moment ago.
    /// It happened once on a full workspace run and never on the test alone,
    /// which is exactly how that class of flake presents. The pid and the
    /// counter make the name unique across processes and within one; `Drop`
    /// does the cleanup so a failing assertion still leaves no litter.
    struct ScratchDir(std::path::PathBuf);

    impl ScratchDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("openshard-map-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn join(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_block_is_the_size_sphere_says() {
        assert_eq!(BLOCK_BYTES, 196, "4-byte header plus 64 cells of 3 bytes");
    }

    #[test]
    fn the_real_clients_map0_is_the_expanded_facet() {
        // 89,915,392 bytes. Every tutorial says Felucca is 6144x4096; a modern
        // one is the post-ML 7168x4096, and assuming the classic size would put
        // every block after the first column in the wrong place.
        let blocks = 89_915_392 / BLOCK_BYTES;
        assert_eq!(blocks, 458_752);
        assert_eq!(facet_size(blocks, Some(0)), Some((7168, 4096)));
        assert_eq!(facet_size(blocks, None), Some((7168, 4096)));
        assert_eq!(describe_size(7168, 4096), "Felucca/Trammel (post-ML)");
    }

    #[test]
    fn the_classic_facet_is_still_recognised() {
        let blocks = blocks_in(6144, 4096);
        assert_eq!(facet_size(blocks, Some(0)), Some((6144, 4096)));
        assert_eq!(facet_size(blocks, None), Some((6144, 4096)));
    }

    #[test]
    fn an_unknown_block_count_is_refused_rather_than_guessed() {
        // Guessing would transpose the map, and it would parse cleanly.
        assert_eq!(facet_size(0, None), None);
        assert_eq!(facet_size(1, None), None);
        assert_eq!(facet_size(458_751, None), None);
    }

    #[test]
    fn malas_and_ter_mur_are_the_same_size_and_only_the_facet_number_parts_them() {
        // The collision itself, in arithmetic: 320x256 and 160x512.
        assert_eq!(blocks_in(2560, 2048), 81_920);
        assert_eq!(blocks_in(1280, 4096), 81_920);

        // With the facet number, each file is what it says it is.
        assert_eq!(facet_size(81_920, Some(3)), Some((2560, 2048)), "Malas");
        assert_eq!(facet_size(81_920, Some(5)), Some((1280, 4096)), "Ter Mur");

        // Without it there is no honest answer, and the documented one is Malas.
        // Pinned so that changing it is a decision rather than a reordering of
        // `KNOWN_FACETS`.
        assert_eq!(facet_size(81_920, None), Some((2560, 2048)));
    }

    #[test]
    fn a_facet_number_no_client_ships_falls_back_to_every_shape() {
        // Not an error: the caller has a file, and the block count may still
        // identify it. It must not silently become "no known facet".
        assert_eq!(facet_size(blocks_in(1448, 1448), Some(9)), Some((1448, 1448)));
        assert_eq!(facet_size(7, Some(9)), None);
    }

    #[test]
    fn a_padded_container_is_trimmed_to_the_shape_its_facet_can_be() {
        // A UOP is allocated in chunks and runs past the facet. Ter Mur's
        // container holds a few blocks more than 81,920; trimming must land on
        // Ter Mur's own size and not on some larger facet's.
        assert_eq!(largest_facet_within(81_920 + 5, Some(5)), Some(81_920));
        assert_eq!(largest_facet_within(81_920 + 5, Some(3)), Some(81_920));
        // Tokuno is smaller than either, and its own shape is the right answer
        // even though bigger shapes exist.
        assert_eq!(largest_facet_within(81_920, Some(4)), Some(blocks_in(1448, 1448)));
        // Nothing fits inside a file smaller than the smallest facet.
        assert_eq!(largest_facet_within(3, Some(4)), None);
    }

    /// A 2x2-block map (16x16 tiles) where every cell records its own index.
    fn synthetic_map() -> Vec<u8> {
        let blocks = 4;
        let mut bytes = Vec::with_capacity(blocks * BLOCK_BYTES);
        for block in 0..blocks {
            bytes.extend_from_slice(&[0u8; BLOCK_HEADER]);
            for cell in 0..CELLS_PER_BLOCK {
                let tile = (block * CELLS_PER_BLOCK + cell) as u16;
                bytes.extend_from_slice(&tile.to_le_bytes());
                bytes.push(cell as u8);
            }
        }
        bytes
    }

    /// A map built by hand rather than loaded, for the indexing tests.
    fn map_16x16() -> Map {
        let bytes = synthetic_map();
        let mut cells = Vec::new();
        for block in 0..4 {
            let base = block * BLOCK_BYTES + BLOCK_HEADER;
            for cell in 0..CELLS_PER_BLOCK {
                let at = base + cell * CELL_BYTES;
                cells.push(LandCell {
                    tile: u16::from_le_bytes([bytes[at], bytes[at + 1]]),
                    z: bytes[at + 2] as i8,
                });
            }
        }
        Map {
            width: 16,
            height: 16,
            cells,
            statics: vec![Vec::new(); 4],
        }
    }

    #[test]
    fn block_order_is_column_major() {
        // The one that silently transposes the world. With a 16x16 map there are
        // two blocks down, so block (1,0) is index 2 — not 1.
        let map = map_16x16();

        // (0,0) is block 0, cell 0.
        assert_eq!(map.land(0, 0).unwrap().tile, 0);
        // (8,0) is block *2*: bx=1, by=0, blocks_down=2 -> 1*2+0 = 2.
        assert_eq!(map.land(8, 0).unwrap().tile, (2 * CELLS_PER_BLOCK) as u16);
        // (0,8) is block 1: bx=0, by=1 -> 0*2+1 = 1.
        assert_eq!(map.land(0, 8).unwrap().tile, CELLS_PER_BLOCK as u16);
    }

    #[test]
    fn cells_within_a_block_are_row_major() {
        // Sphere: `m_Meter[yo * UO_BLOCK_SIZE + xo]`. The opposite of the block
        // order, which is exactly why it is worth a test.
        let map = map_16x16();
        assert_eq!(map.land(1, 0).unwrap().tile, 1, "x moves by one");
        assert_eq!(map.land(0, 1).unwrap().tile, 8, "y moves by a row");
        assert_eq!(map.land(7, 7).unwrap().tile, 63, "the block's far corner");
    }

    #[test]
    fn every_cell_is_reachable_exactly_once() {
        // If the indexing were wrong in a way the spot-checks missed, two points
        // would map to one cell and some cell would be unreachable.
        let map = map_16x16();
        let mut seen = std::collections::HashSet::new();
        for y in 0..16u16 {
            for x in 0..16u16 {
                let index = map.cell_index(x, y).expect("on the map");
                assert!(seen.insert(index), "({x},{y}) collides with another point");
            }
        }
        assert_eq!(seen.len(), 16 * 16);
    }

    #[test]
    fn off_the_map_is_none_not_a_panic() {
        let map = map_16x16();
        assert_eq!(map.land(16, 0), None);
        assert_eq!(map.land(0, 16), None);
        assert_eq!(map.land(u16::MAX, u16::MAX), None);
        assert!(!map.contains(16, 15));
        assert!(map.contains(15, 15));
    }

    #[test]
    fn a_map_that_is_not_whole_blocks_is_refused() {
        let dir = ScratchDir::new();
        let path = dir.join("ragged.mul");
        std::fs::write(&path, [0u8; BLOCK_BYTES + 1]).unwrap();

        let result = Map::load(&path, None::<(&Path, &Path)>);
        assert!(matches!(result, Err(MapError::NotABlockMap { .. })));
    }

    #[test]
    fn a_map_with_no_statics_loads_as_bare_ground() {
        let dir = ScratchDir::new();
        let path = dir.join("tiny.mul");
        // A whole facet's worth of blocks would be 90MB; use Tokuno's shape.
        let blocks = ((1448 / BLOCK_SIZE) * (1448 / BLOCK_SIZE)) as usize;
        std::fs::write(&path, vec![0u8; blocks * BLOCK_BYTES]).unwrap();

        let map = Map::load(&path, None::<(&Path, &Path)>).unwrap();
        assert_eq!((map.width(), map.height()), (1448, 1448));
        assert_eq!(map.facet_name(), "Tokuno");
        assert_eq!(map.static_count(), 0);
    }
}
