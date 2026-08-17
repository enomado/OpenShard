//! One pixel per tile: the reduction a radar and a facet map are both made of.
//!
//! No GPU, no window, no camera. It is a function of the map and
//! [`RadarColors`], which is what lets the player's radar and `client.md`'s M3b
//! facet map share it — the two differ in what they draw the pixels *into*, and
//! in nothing else.
//!
//! # The walk is block-major, and that is not a micro-optimisation
//!
//! `Map::statics_at` binary-searches a block per call, and `map.rs:568`'s own
//! doc records what that costs at scale: asked per tile over a frame it was
//! *the largest single phase of the lighting pass*. A radar covering 256 tiles
//! square would ask it sixty-five thousand times.
//!
//! [`Map::statics_in_block`] hands back the whole block as a slice with no
//! search at all, so the walk asks once per block and buckets what it finds —
//! a thousand slice fetches instead of a hundred and thirty thousand searches,
//! for the same answer.
//!
//! # What a tile's colour is
//!
//! Its land tile, overridden by the **highest static standing on it**. Three
//! details that are each a bug if got wrong:
//!
//! - **`Map::statics_at` is not sorted by z.** Its key is `(y, x)` and nothing
//!   else, so "the last one" is not "the highest one" and the comparison is
//!   explicit.
//! - **The comparison against the land is `>=`, not `>`.** A floor tile lies at
//!   the ground's own height, and `>` would draw grass through a marble floor.
//! - **A tile with no colour at all is [`UNKNOWN`], never transparent.** Zero is
//!   how these files spell *absent*, and a transparent radar pixel would punch a
//!   hole through the window it is drawn in rather than reading as unmapped
//!   ground.

use openshard_protocol::wire::Graphic;
use openshard_uofiles::color::Color16;
use openshard_uofiles::map::{BLOCK_SIZE, Map};
use openshard_uofiles::radarcol::RadarColors;

/// What a tile with no colour of its own draws as.
///
/// Deliberately non-zero: `Color16(0)` is *absent* in every one of these files,
/// and a pixel that is absent is a hole. Near-black, so unmapped ground reads as
/// unmapped rather than as a mistake.
pub const UNKNOWN: Color16 = Color16(0x0001);

/// How many tiles a map block is across. Re-exported so a caller sizing a buffer
/// does not reach past this module for it.
pub const BLOCK_TILES: u16 = BLOCK_SIZE as u16;

/// The colour of one tile.
///
/// The whole rule in one place, so the block walk below and any caller asking
/// about a single tile cannot come to disagree. Off the map is [`UNKNOWN`].
#[must_use]
pub fn tile_color(map: &Map, colors: &RadarColors, x: u16, y: u16) -> Color16 {
    let Some(land) = map.land(x, y) else {
        return UNKNOWN;
    };
    let mut best = colors.land(land.tile);
    let mut best_z = land.z;
    for item in map.statics_at(x, y) {
        if item.z < best_z {
            continue;
        }
        let color = colors.statik(item.tile);
        if color == Color16::TRANSPARENT {
            continue;
        }
        best = color;
        best_z = item.z;
    }
    if best == Color16::TRANSPARENT {
        UNKNOWN
    } else {
        best
    }
}

/// Fill `into` with the colours of a `width` × `height` rectangle of tiles whose
/// north-west corner is `origin`.
///
/// Row-major, `width` per row, so a caller uploading it as a texture needs no
/// stride of its own. `into` must hold `width * height` colours; anything it
/// cannot reach is left alone, which is the safe half of a caller's arithmetic
/// error rather than a panic in a render path.
///
/// See the module header for why this walks blocks rather than tiles.
pub fn fill(
    map: &Map,
    colors: &RadarColors,
    origin: (u16, u16),
    width: u16,
    height: u16,
    into: &mut [Color16],
) {
    let (origin_x, origin_y) = origin;
    // Every tile starts as its land, and the statics are laid over it block by
    // block. Two passes rather than one because the land is a direct lookup and
    // the statics are not: interleaving them would put a binary search back in
    // the inner loop, which is the whole thing this avoids.
    for row in 0..height {
        for column in 0..width {
            let Some(cell) = into.get_mut(usize::from(row) * usize::from(width) + usize::from(column)) else {
                continue;
            };
            let (Some(x), Some(y)) = (origin_x.checked_add(column), origin_y.checked_add(row)) else {
                *cell = UNKNOWN;
                continue;
            };
            *cell = match map.land(x, y) {
                Some(land) => {
                    let color = colors.land(land.tile);
                    if color == Color16::TRANSPARENT {
                        UNKNOWN
                    } else {
                        color
                    }
                }
                None => UNKNOWN,
            };
        }
    }

    // The highest static on each tile, one block at a time. `best_z` starts at
    // the land's own height so a static below the ground is skipped, and a floor
    // *at* it is not — see the module header.
    let mut best_z = vec![i8::MIN; into.len().min(usize::from(width) * usize::from(height))];
    for row in 0..height {
        for column in 0..width {
            let index = usize::from(row) * usize::from(width) + usize::from(column);
            let (Some(z), Some(x), Some(y)) = (
                best_z.get_mut(index),
                origin_x.checked_add(column),
                origin_y.checked_add(row),
            ) else {
                continue;
            };
            if let Some(land) = map.land(x, y) {
                *z = land.z;
            }
        }
    }

    let last_x = origin_x.saturating_add(width.saturating_sub(1));
    let last_y = origin_y.saturating_add(height.saturating_sub(1));
    let (first_block_x, first_block_y) = (origin_x / BLOCK_TILES, origin_y / BLOCK_TILES);
    let (last_block_x, last_block_y) = (last_x / BLOCK_TILES, last_y / BLOCK_TILES);
    for block_x in first_block_x..=last_block_x {
        for block_y in first_block_y..=last_block_y {
            for item in map.statics_in_block(u32::from(block_x), u32::from(block_y)) {
                let (Some(column), Some(row)) = (item.x.checked_sub(origin_x), item.y.checked_sub(origin_y))
                else {
                    continue;
                };
                if column >= width || row >= height {
                    continue;
                }
                let index = usize::from(row) * usize::from(width) + usize::from(column);
                let (Some(cell), Some(z)) = (into.get_mut(index), best_z.get_mut(index)) else {
                    continue;
                };
                if item.z < *z {
                    continue;
                }
                let color = colors.statik(item.tile);
                if color == Color16::TRANSPARENT {
                    continue;
                }
                *cell = color;
                *z = item.z;
            }
        }
    }
}

/// Stamp a marker over a filled buffer, at the tile `column`, `row` in from its
/// north-west corner.
///
/// A cross rather than a pixel, and that is not decoration. At one pixel a tile
/// a single dot is a single pixel — indistinguishable from a lamp post, and
/// invisible against ground of a similar colour. Five pixels in a shape nothing
/// in `radarcol.mul` produces is the smallest thing a person can actually find.
///
/// Written into the bitmap rather than drawn as a second quad, so the marker
/// travels with the upload the map already costs and the pass stays one draw.
/// The arms clip at the edges, so a marker on the first row keeps the four
/// pixels that are on the map instead of wrapping to the last.
pub fn mark(into: &mut [Color16], width: u16, height: u16, at: (u16, u16), color: Color16) {
    let (column, row) = at;
    if column >= width || row >= height {
        return;
    }
    let arms = [(0i32, 0i32), (-1, 0), (1, 0), (0, -1), (0, 1)];
    for (dx, dy) in arms {
        let (Ok(x), Ok(y)) = (
            u16::try_from(i32::from(column) + dx),
            u16::try_from(i32::from(row) + dy),
        ) else {
            continue;
        };
        if x >= width || y >= height {
            continue;
        }
        if let Some(cell) = into.get_mut(usize::from(y) * usize::from(width) + usize::from(x)) {
            *cell = color;
        }
    }
}

/// The colour a static of this graphic draws as, or [`UNKNOWN`] when the table
/// has none. Named so a caller drawing a marker over the map uses the same
/// widening as the map itself.
#[must_use]
pub fn static_color(colors: &RadarColors, graphic: Graphic) -> Color16 {
    match colors.statik(graphic) {
        Color16::TRANSPARENT => UNKNOWN,
        color => color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openshard_protocol::wire::Hue;
    use openshard_uofiles::map::{LandCell, LandTile, StaticItem};
    use openshard_uofiles::tiledata::LAND_TILE_COUNT;

    /// Land id 1 is green, land id 2 is blue; static 1 is red, static 2 is
    /// white, static 3 has no colour at all.
    fn colors() -> RadarColors {
        let mut bytes = vec![0u8; LAND_TILE_COUNT * 2];
        bytes[2..4].copy_from_slice(&0x03E0u16.to_le_bytes()); // land 1: green
        bytes[4..6].copy_from_slice(&0x001Fu16.to_le_bytes()); // land 2: blue
        bytes.extend_from_slice(&0u16.to_le_bytes()); // static 0: absent
        bytes.extend_from_slice(&0x7C00u16.to_le_bytes()); // static 1: red
        bytes.extend_from_slice(&0x7FFFu16.to_le_bytes()); // static 2: white
        bytes.extend_from_slice(&0u16.to_le_bytes()); // static 3: absent
        RadarColors::parse(&bytes).expect("a whole table")
    }

    const GREEN: Color16 = Color16(0x03E0);
    const BLUE: Color16 = Color16(0x001F);
    const RED: Color16 = Color16(0x7C00);
    const WHITE: Color16 = Color16(0x7FFF);

    /// A one-block facet, every tile land id 1 at z 0.
    fn a_field() -> Map {
        Map::from_blocks(1, 1, |_, _| LandCell {
            tile: LandTile(1),
            z: 0,
        })
    }

    fn put(map: &mut Map, graphic: u16, x: u16, y: u16, z: i8) {
        map.place_static(StaticItem {
            tile: Graphic(graphic),
            x,
            y,
            z,
            hue: Hue(0),
        });
    }

    #[test]
    fn a_bare_tile_is_its_land() {
        let map = Map::from_blocks(1, 1, |x, _| LandCell {
            tile: LandTile(if x < 4 { 1 } else { 2 }),
            z: 0,
        });
        let colors = colors();

        assert_eq!(tile_color(&map, &colors, 3, 3), GREEN);
        assert_eq!(
            tile_color(&map, &colors, 5, 3),
            BLUE,
            "a different land tile is a different colour, so the land id is the key"
        );
    }

    /// Off the map is unmapped, not transparent — a hole in the window is worse
    /// than a dark tile.
    #[test]
    fn off_the_map_is_unknown() {
        let map = a_field();
        assert_eq!(tile_color(&map, &colors(), 99, 99), UNKNOWN);
    }

    /// **The comparison is `>=`, not `>`.** A floor lies at the ground's own
    /// height, and `>` would draw grass through it.
    #[test]
    fn a_static_at_the_grounds_own_height_covers_it() {
        let mut map = a_field();
        put(&mut map, 1, 2, 2, 0);
        assert_eq!(tile_color(&map, &colors(), 2, 2), RED);
    }

    /// And one below the ground does not.
    #[test]
    fn a_static_under_the_ground_does_not_cover_it() {
        let mut map = a_field();
        put(&mut map, 1, 2, 2, -5);
        assert_eq!(tile_color(&map, &colors(), 2, 2), GREEN);
    }

    /// **`statics_at` is keyed by `(y, x)` and not by z**, so the highest is
    /// picked by comparing rather than by taking the last.
    #[test]
    fn the_highest_static_wins_whatever_order_they_are_in() {
        let mut map = a_field();
        put(&mut map, 2, 4, 4, 20); // white, high
        put(&mut map, 1, 4, 4, 5); // red, low
        assert_eq!(
            tile_color(&map, &colors(), 4, 4),
            WHITE,
            "the lower static was drawn over the higher one"
        );
    }

    /// A static the radar table has no colour for falls through to what is under
    /// it. Zero is *absent* in these files, not black.
    #[test]
    fn a_static_with_no_colour_falls_through() {
        let mut map = a_field();
        put(&mut map, 3, 6, 6, 20);
        assert_eq!(tile_color(&map, &colors(), 6, 6), GREEN);
    }

    /// The rectangle walk agrees with the single-tile rule, tile for tile. They
    /// are two readers of one answer and the block walk is the one that could
    /// drift.
    #[test]
    fn the_block_walk_agrees_with_the_single_tile_rule() {
        let mut map = a_field();
        put(&mut map, 1, 1, 1, 10);
        put(&mut map, 2, 5, 6, 30);
        put(&mut map, 3, 3, 4, 30); // no colour: falls through
        put(&mut map, 1, 7, 0, -1); // below ground: ignored
        let colors = colors();

        let mut pixels = vec![Color16::TRANSPARENT; 64];
        fill(&map, &colors, (0, 0), 8, 8, &mut pixels);

        for y in 0..8u16 {
            for x in 0..8u16 {
                assert_eq!(
                    pixels[usize::from(y) * 8 + usize::from(x)],
                    tile_color(&map, &colors, x, y),
                    "the block walk and the tile rule disagree at ({x}, {y})"
                );
            }
        }
        assert_eq!(pixels[8 + 1], RED, "(1, 1)");
        assert_eq!(pixels[6 * 8 + 5], WHITE, "(5, 6)");
    }

    /// A marker is five pixels, and its arms clip rather than wrap.
    #[test]
    fn a_marker_at_a_corner_keeps_only_the_arms_on_the_map() {
        let mut pixels = vec![Color16::TRANSPARENT; 16];
        mark(&mut pixels, 4, 4, (0, 0), RED);

        assert_eq!(pixels[0], RED, "the centre");
        assert_eq!(pixels[1], RED, "the arm east of it");
        assert_eq!(pixels[4], RED, "and the one south");
        assert_eq!(
            pixels.iter().filter(|&&c| c == RED).count(),
            3,
            "the west and north arms wrapped instead of clipping",
        );
    }

    /// Away from an edge it is the whole cross, and nothing else.
    #[test]
    fn a_marker_in_the_middle_is_a_cross() {
        let mut pixels = vec![Color16::TRANSPARENT; 25];
        mark(&mut pixels, 5, 5, (2, 2), WHITE);

        for index in [12, 11, 13, 7, 17] {
            assert_eq!(pixels[index], WHITE, "pixel {index} is part of the cross");
        }
        assert_eq!(pixels.iter().filter(|&&c| c == WHITE).count(), 5);
    }

    /// Off the buffer entirely is nothing, not a wrapped pixel on the far side.
    #[test]
    fn a_marker_off_the_map_draws_nothing() {
        let mut pixels = vec![Color16::TRANSPARENT; 16];
        mark(&mut pixels, 4, 4, (4, 0), RED);
        mark(&mut pixels, 4, 4, (0, 9), RED);
        assert!(pixels.iter().all(|&c| c == Color16::TRANSPARENT));
    }

    /// A rectangle that runs off the map is filled to its edge and unmapped
    /// past it, rather than short or panicking.
    #[test]
    fn a_rectangle_past_the_edge_is_unmapped_not_missing() {
        let map = a_field();
        let mut pixels = vec![Color16::TRANSPARENT; 16];
        fill(&map, &colors(), (6, 6), 4, 4, &mut pixels);

        assert_eq!(pixels[0], GREEN, "(6, 6) is on the map");
        assert_eq!(pixels[3], UNKNOWN, "(9, 6) is not");
        assert!(
            pixels.iter().all(|&color| color != Color16::TRANSPARENT),
            "a transparent pixel would be a hole in the window"
        );
    }

    /// Nothing is written past the buffer a caller supplied. A render path
    /// should not panic over a caller's arithmetic.
    #[test]
    fn a_buffer_too_small_is_filled_as_far_as_it_goes() {
        let map = a_field();
        let mut pixels = vec![Color16::TRANSPARENT; 4];
        fill(&map, &colors(), (0, 0), 8, 8, &mut pixels);
        assert!(pixels.iter().all(|&color| color == GREEN));
    }
}
