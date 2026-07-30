//! Turning a patch of map into the quads that draw it.
//!
//! This is the whole CPU side of the ground: read the land cells the camera can
//! see, project each one, and look its sprite up in the atlas. No GPU type
//! appears here, so it can be checked by counting and comparing numbers.

use std::collections::BTreeSet;

use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::map::Map;

use crate::atlas::{LandAtlas, Region, TexmapAtlas};
use crate::camera::Camera;

/// One ground quad: where it goes, how its corners stand, and what to sample.
///
/// The position is the diamond's centre **at height zero**. Height is not folded
/// in here because there is no single height to fold: a tile is a patch stretched
/// over four corners, and only the shader knows which corner a vertex is.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GroundQuad {
    /// The diamond's centre, in viewport pixels, ignoring height. Fractional
    /// never happens, but the GPU wants floats and converting once here keeps
    /// the buffer writer trivial.
    pub x: f32,
    /// The same, downwards.
    pub y: f32,
    /// The heights of the tile's four corners, in the order the shader indexes
    /// them: `(x, y)`, `(x+1, y)`, `(x, y+1)`, `(x+1, y+1)` — index `a + 2*b`
    /// for a unit-quad corner `(a, b)`. On screen that reads top, right, left,
    /// bottom.
    ///
    /// Floats for the same reason as the position; every value came from an
    /// `i8` and converts back exactly.
    pub corners: [f32; 4],
    /// Where its sprite lives in the land atlas.
    pub region: Region,
    /// Where its square texture lives in the texture atlas, if it has one.
    ///
    /// Absent is ordinary and means what it says: three quarters of the land
    /// index has no texture at all, and such a tile is drawn from `region` at
    /// whatever shape its corners make. The shader is told by a zero size, which
    /// no real region can have.
    pub texmap: Option<Region>,
}

impl GroundQuad {
    /// Bytes one quad occupies in the instance buffer.
    ///
    /// Fourteen floats: position, the four corner heights, the land region and
    /// the texture region. Written by hand rather than cast from a struct —
    /// `bytemuck`'s derive emits `unsafe impl`, and this workspace denies
    /// `unsafe_code` outright. Fourteen `to_le_bytes` is a cheaper price than an
    /// exception to that rule.
    pub const STRIDE: u64 = 14 * 4;

    /// Append this quad to an instance buffer.
    pub fn write(&self, out: &mut Vec<u8>) {
        // A tile with no texture writes a region of zero size. The shader tests
        // the size rather than the position, because (0, 0) is a legitimate
        // corner of the atlas and a zero *extent* is not a texture anything
        // could be sampled from.
        let texmap = self.texmap.unwrap_or(Region {
            u: 0.0,
            v: 0.0,
            du: 0.0,
            dv: 0.0,
        });
        for value in [
            self.x,
            self.y,
            self.corners[0],
            self.corners[1],
            self.corners[2],
            self.corners[3],
            self.region.u,
            self.region.v,
            self.region.du,
            self.region.dv,
            texmap.u,
            texmap.v,
            texmap.du,
            texmap.dv,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

/// Every distinct land graphic the camera can see.
///
/// Called before building the atlas, which is why it is separate from
/// [`collect`]: the atlas has to exist before a quad can be given a region.
pub fn visible_graphics(map: &Map, camera: &Camera) -> BTreeSet<Graphic> {
    let mut seen = BTreeSet::new();
    for_each_visible_cell(map, camera, |_, _, cell| {
        seen.insert(Graphic(cell.tile));
    });
    seen
}

/// The quads for everything visible, in the order they must be drawn.
///
/// A tile whose graphic is not in the atlas is dropped: either the client ships
/// no art for it, or the atlas was built for a different camera. Both are
/// "nothing to draw here", and neither is worth failing a frame over.
pub fn collect(map: &Map, camera: &Camera, atlas: &LandAtlas, texmaps: &TexmapAtlas) -> Vec<GroundQuad> {
    let mut quads: Vec<(i32, i32, GroundQuad)> = Vec::new();

    for_each_visible_cell(map, camera, |x, y, cell| {
        let Some(region) = atlas.region(Graphic(cell.tile)) else {
            return;
        };
        // `None` here is not a failure to find anything: most land graphics have
        // no texture, and the shader draws those from the art.
        let texmap = texmaps.region(Graphic(cell.tile));
        let corners = corner_heights(map, x, y, cell.z);
        // Height deliberately left at zero: the shader lifts each corner by its
        // own, and folding a representative height in here would count one of
        // them twice.
        let at = camera.to_screen(Point::new(x, y, 0));
        quads.push((
            // Painter's order for ground: further from the camera first. Depth
            // in UO is `x + y`, and height breaks the tie — a cliff face drawn
            // after the ground below it is the whole reason this is sorted
            // rather than emitted in scan order. Ground rarely overlaps ground,
            // so this is nearly free today and is the seam the statics need.
            i32::from(x) + i32::from(y),
            // The tie-break is the tile's highest corner: at equal depth the
            // tile that reaches further up the screen is the nearer one, and
            // the flat `cell.z` stopped describing the tile the moment a tile
            // became four heights.
            corners.iter().copied().fold(f32::NEG_INFINITY, f32::max) as i32,
            GroundQuad {
                x: at.x as f32,
                y: at.y as f32,
                corners,
                region,
                texmap,
            },
        ));
    });

    quads.sort_by_key(|(depth, z, _)| (*depth, *z));
    quads.into_iter().map(|(_, _, quad)| quad).collect()
}

/// The heights of a tile's four corners, in [`GroundQuad::corners`] order.
///
/// A land cell stores one height, and it is the height of the corner the tile
/// shares with the tiles north of it — the diamond's top. The other three belong
/// to the neighbours, which is exactly why the ground has no seams in the
/// client: adjacent tiles do not merely abut, they are stretched over *the same*
/// vertices, so a gap between them is not expressible.
///
/// Off the edge of the map there is no neighbour and the tile's own height
/// stands in, which flattens the border tiles rather than dropping them off a
/// cliff into nothing.
fn corner_heights(map: &Map, x: u16, y: u16, own: i8) -> [f32; 4] {
    let at = |x: Option<u16>, y: Option<u16>| -> f32 {
        let height = match (x, y) {
            (Some(x), Some(y)) => map.land(x, y).map_or(own, |cell| cell.z),
            _ => own,
        };
        f32::from(height)
    };
    let (east, south) = (x.checked_add(1), y.checked_add(1));
    [
        f32::from(own),
        at(east, Some(y)),
        at(Some(x), south),
        at(east, south),
    ]
}

/// Walk the visible rectangle, clamped to the map, calling back for each cell.
///
/// The clamp is why the camera may hand back negative bounds: the edge of the
/// world is the map's fact, not the camera's, and a camera that knew the map's
/// size would have to be rebuilt whenever the facet changed.
fn for_each_visible_cell(
    map: &Map,
    camera: &Camera,
    mut each: impl FnMut(u16, u16, openshard_uofiles::map::LandCell),
) {
    let bounds = camera.visible_tiles();
    let min_x = bounds.min_x.max(0) as u32;
    let min_y = bounds.min_y.max(0) as u32;
    let max_x = (bounds.max_x.max(0) as u32).min(map.width().saturating_sub(1));
    let max_y = (bounds.max_y.max(0) as u32).min(map.height().saturating_sub(1));

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            // Both fit a `u16` because they were clamped to the map's size, and
            // no facet is wider than 7,168 tiles.
            let (x, y) = (x as u16, y as u16);
            if let Some(cell) = map.land(x, y) {
                each(x, y, cell);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instance buffer's layout is a contract with the shader, and the
    /// shader is text the compiler here never sees. If one side changes, this
    /// is the only thing that notices.
    #[test]
    fn a_quad_writes_its_stride_and_nothing_more() {
        let quad = GroundQuad {
            x: 1.0,
            y: 2.0,
            corners: [3.0, 4.0, 5.0, 6.0],
            region: Region {
                u: 0.25,
                v: 0.5,
                du: 0.125,
                dv: 0.125,
            },
            texmap: Some(Region {
                u: 0.75,
                v: 0.5,
                du: 0.03125,
                dv: 0.03125,
            }),
        };
        let mut out = Vec::new();
        quad.write(&mut out);
        assert_eq!(out.len() as u64, GroundQuad::STRIDE);
        assert_eq!(&out[..4], &1.0f32.to_le_bytes());
        // The corner heights sit between the position and the region, and the
        // shader reads them as one `vec4` at offset 8.
        assert_eq!(&out[8..12], &3.0f32.to_le_bytes());
        assert_eq!(&out[20..24], &6.0f32.to_le_bytes());
        assert_eq!(&out[24..28], &0.25f32.to_le_bytes());
        // And the texture region is the last four, at offset 40.
        assert_eq!(&out[40..44], &0.75f32.to_le_bytes());
        assert_eq!(&out[52..56], &0.03125f32.to_le_bytes());
    }

    /// A tile with no texture still writes a full instance — the buffer is a
    /// stride, not a list — and says so with a zero *size* rather than a zero
    /// position, which is a real corner of the atlas.
    #[test]
    fn a_quad_with_no_texture_writes_a_region_of_no_size() {
        let quad = GroundQuad {
            x: 0.0,
            y: 0.0,
            corners: [0.0; 4],
            region: Region {
                u: 0.0,
                v: 0.0,
                du: 0.021484375,
                dv: 0.021484375,
            },
            texmap: None,
        };
        let mut out = Vec::new();
        quad.write(&mut out);
        assert_eq!(out.len() as u64, GroundQuad::STRIDE);
        assert_eq!(&out[48..52], &0.0f32.to_le_bytes(), "du");
        assert_eq!(&out[52..56], &0.0f32.to_le_bytes(), "dv");
    }

    /// The whole point of corner heights: a corner belongs to four tiles at
    /// once, and all four have to name the same number for it.
    ///
    /// Stated through [`corner_heights`] itself rather than against `map.land`,
    /// so it is a claim about the relation between neighbours and not a second
    /// copy of the lookup. Coverage measured at one camera would stay green
    /// through a swapped pair here; this would not.
    ///
    /// Skipped without the client's files, like everything else that needs a
    /// real map — `Map` cannot yet be built in memory.
    #[test]
    fn neighbours_agree_on_the_corner_they_share() {
        let Some(dir) = std::env::var_os("OPENSHARD_CLIENT") else {
            return;
        };
        let map = Map::load_facet(std::path::Path::new(&dir), 0).expect("Felucca");

        // Britain's hillside, where heights actually differ; a level patch would
        // satisfy any permutation of the four.
        let mut differing = 0;
        for y in 1600..1660u16 {
            for x in 1460..1520u16 {
                let own = map.land(x, y).expect("inside the facet").z;
                let corners = corner_heights(&map, x, y, own);
                assert_eq!(corners[0], f32::from(own), "({x}, {y}) lost its own height");
                for (index, (dx, dy)) in [(1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
                    let (nx, ny) = (x + dx, y + dy);
                    let neighbour = map.land(nx, ny).expect("inside the facet").z;
                    assert_eq!(
                        corners[index + 1],
                        f32::from(neighbour),
                        "({x}, {y}) and ({nx}, {ny}) disagree about the corner they share",
                    );
                }
                if corners.iter().any(|z| *z != corners[0]) {
                    differing += 1;
                }
            }
        }
        assert!(differing > 100, "only {differing} tiles in the patch slope");
    }
}
