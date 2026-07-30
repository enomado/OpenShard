//! Turning a patch of map into the quads that draw it.
//!
//! This is the whole CPU side of the ground: read the land cells the camera can
//! see, project each one, and look its sprite up in the atlas. No GPU type
//! appears here, so it can be checked by counting and comparing numbers.

use std::collections::BTreeSet;

use openshard_protocol::wire::Graphic;
use openshard_protocol::world::Point;
use openshard_uofiles::map::Map;

use crate::atlas::{LandAtlas, Region};
use crate::camera::{Camera, TILE_HEIGHT, TILE_WIDTH};

/// One ground quad: where it goes, and what to sample.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GroundQuad {
    /// Left edge in viewport pixels. Fractional never happens, but the GPU
    /// wants floats and converting once here keeps the buffer writer trivial.
    pub x: f32,
    /// Top edge in viewport pixels.
    pub y: f32,
    /// Where its sprite lives in the atlas.
    pub region: Region,
}

impl GroundQuad {
    /// Bytes one quad occupies in the instance buffer.
    ///
    /// Six floats: position, then the atlas region. Written by hand rather than
    /// cast from a struct — `bytemuck`'s derive emits `unsafe impl`, and this
    /// workspace denies `unsafe_code` outright. Six `to_le_bytes` is a cheaper
    /// price than an exception to that rule.
    pub const STRIDE: u64 = 6 * 4;

    /// Append this quad to an instance buffer.
    pub fn write(&self, out: &mut Vec<u8>) {
        for value in [
            self.x,
            self.y,
            self.region.u,
            self.region.v,
            self.region.du,
            self.region.dv,
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
    for_each_visible_cell(map, camera, |_, cell| {
        seen.insert(Graphic(cell.tile));
    });
    seen
}

/// The quads for everything visible, in the order they must be drawn.
///
/// A tile whose graphic is not in the atlas is dropped: either the client ships
/// no art for it, or the atlas was built for a different camera. Both are
/// "nothing to draw here", and neither is worth failing a frame over.
pub fn collect(map: &Map, camera: &Camera, atlas: &LandAtlas) -> Vec<GroundQuad> {
    let mut quads: Vec<(i32, i32, GroundQuad)> = Vec::new();

    for_each_visible_cell(map, camera, |point, cell| {
        let Some(region) = atlas.region(Graphic(cell.tile)) else {
            return;
        };
        let at = camera.to_screen(Point::new(point.x, point.y, cell.z));
        quads.push((
            // Painter's order for ground: further from the camera first. Depth
            // in UO is `x + y`, and height breaks the tie — a cliff face drawn
            // after the ground below it is the whole reason this is sorted
            // rather than emitted in scan order. Ground rarely overlaps ground,
            // so this is nearly free today and is the seam the statics need.
            i32::from(point.x) + i32::from(point.y),
            i32::from(cell.z),
            GroundQuad {
                // The projection gives the diamond's centre; the sprite is drawn
                // from its top-left corner.
                x: (at.x - TILE_WIDTH / 2) as f32,
                y: (at.y - TILE_HEIGHT / 2) as f32,
                region,
            },
        ));
    });

    quads.sort_by_key(|(depth, z, _)| (*depth, *z));
    quads.into_iter().map(|(_, _, quad)| quad).collect()
}

/// Walk the visible rectangle, clamped to the map, calling back for each cell.
///
/// The clamp is why the camera may hand back negative bounds: the edge of the
/// world is the map's fact, not the camera's, and a camera that knew the map's
/// size would have to be rebuilt whenever the facet changed.
fn for_each_visible_cell(
    map: &Map,
    camera: &Camera,
    mut each: impl FnMut(Point, openshard_uofiles::map::LandCell),
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
                each(Point::new(x, y, cell.z), cell);
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
            region: Region {
                u: 0.25,
                v: 0.5,
                du: 0.125,
                dv: 0.125,
            },
        };
        let mut out = Vec::new();
        quad.write(&mut out);
        assert_eq!(out.len() as u64, GroundQuad::STRIDE);
        assert_eq!(&out[..4], &1.0f32.to_le_bytes());
        assert_eq!(&out[8..12], &0.25f32.to_le_bytes());
    }
}
