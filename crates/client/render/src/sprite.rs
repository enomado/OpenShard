//! The quad two of the three passes draw: a picture, at its own size, in front
//! of or behind everything else.
//!
//! Ground is a shape — a diamond, or a patch over four heights — and the shader
//! decides which. A static and a mobile are not: each is a rectangle of pixels
//! standing somewhere, and the only thing that separates them is *where* the
//! rectangle goes and which atlas it came from. So they share this type, they
//! share [`SpriteRenderer`](crate::renderer::SpriteRenderer), and they share the
//! shader — what differs is entirely on the CPU, in [`crate::statics`] and
//! [`crate::mobiles`].
//!
//! Mirroring lives here rather than in the shader, and it is one subtraction:
//! a region whose width is negative samples its own texels backwards.

use crate::atlas::Region;

/// One sprite: where it goes, how big it is, and what to sample.
///
/// The position is the sprite's top-left corner in viewport pixels, height
/// already folded in — unlike a [`GroundQuad`](crate::ground::GroundQuad),
/// whose corners the shader lifts individually. A sprite has one `z` and one
/// picture, so there is nothing left for the shader to decide.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpriteQuad {
    /// Left edge, in viewport pixels.
    pub x: f32,
    /// Top edge, in viewport pixels.
    pub y: f32,
    /// The sprite's width in pixels. The quad is exactly this wide, so the art
    /// is drawn at its own scale and a texel is a pixel.
    pub width: f32,
    /// Its height in pixels.
    pub height: f32,
    /// Where its picture lives in whichever atlas this pass is bound to.
    ///
    /// A negative width is a mirrored sprite — see [`SpriteQuad::mirrored`].
    pub region: Region,
    /// What it hides and what hides it: smaller is nearer. See [`crate::depth`].
    pub depth: f32,
    /// The wire hue to tint this sprite with, or `0` for none.
    ///
    /// Carried raw rather than as [`openshard_protocol::wire::Hue`] — like every
    /// other field here, this is the instance buffer's own layout, and `.0`
    /// belongs at the boundary the value crosses into it. See
    /// [`crate::hue::HueRamp`] for what a nonzero value means to the shader.
    pub hue: u32,
}

impl SpriteQuad {
    /// Bytes one quad occupies in the instance buffer.
    ///
    /// Nine floats and a `u32`: position, size, region, depth, hue. Written by
    /// hand for the same reason
    /// [`GroundQuad::STRIDE`](crate::ground::GroundQuad::STRIDE) is —
    /// `bytemuck`'s derive emits `unsafe impl` and this workspace denies
    /// `unsafe_code`.
    pub const STRIDE: u64 = 10 * 4;

    /// The same region, sampled right to left.
    ///
    /// UO stores five of a mobile's eight facings and mirrors the rest, so this
    /// is not an effect: it is half of every creature that ever faces west. The
    /// flip is a region starting at the far edge with a negative width, which
    /// costs the shader nothing — it already interpolates `u + corner * du`.
    pub fn mirrored(region: Region) -> Region {
        Region {
            u: region.u + region.du,
            v: region.v,
            du: -region.du,
            dv: region.dv,
        }
    }

    /// Append this quad to an instance buffer.
    pub fn write(&self, out: &mut Vec<u8>) {
        for value in [
            self.x,
            self.y,
            self.width,
            self.height,
            self.region.u,
            self.region.v,
            self.region.du,
            self.region.dv,
            self.depth,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&self.hue.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instance layout is a contract with a shader the compiler never sees.
    #[test]
    fn a_quad_writes_its_stride_and_nothing_more() {
        let quad = SpriteQuad {
            x: 1.0,
            y: 2.0,
            width: 44.0,
            height: 88.0,
            region: Region {
                u: 0.25,
                v: 0.5,
                du: 0.125,
                dv: 0.25,
            },
            depth: 0.75,
            hue: 0x8021,
        };
        let mut out = Vec::new();
        quad.write(&mut out);
        assert_eq!(out.len() as u64, SpriteQuad::STRIDE);
        assert_eq!(&out[..4], &1.0f32.to_le_bytes());
        assert_eq!(&out[8..12], &44.0f32.to_le_bytes());
        assert_eq!(&out[16..20], &0.25f32.to_le_bytes());
        assert_eq!(&out[32..36], &0.75f32.to_le_bytes(), "depth");
        assert_eq!(&out[36..40], &0x8021u32.to_le_bytes(), "hue is last");
    }

    /// A mirrored region covers the same texels and starts at the other end.
    ///
    /// Both halves matter: the same *span*, or the sprite is drawn from
    /// somebody else's pixels, and the opposite *direction*, or nothing is
    /// mirrored at all. The second is what a single sign error leaves intact.
    #[test]
    fn a_mirrored_region_samples_the_same_texels_backwards() {
        let region = Region {
            u: 0.25,
            v: 0.5,
            du: 0.125,
            dv: 0.25,
        };
        let flipped = SpriteQuad::mirrored(region);
        assert_eq!(flipped.u, 0.375, "starts at the region's far edge");
        assert_eq!(flipped.u + flipped.du, region.u, "ends at its near one");
        assert_eq!((flipped.v, flipped.dv), (region.v, region.dv), "and only in x");
        // Twice is the original, which is the property a sign error breaks
        // without moving the sprite anywhere visible.
        let back = SpriteQuad::mirrored(flipped);
        assert_eq!((back.u, back.du), (region.u, region.du));
    }
}
