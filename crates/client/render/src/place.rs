//! Which tile a pixel belongs to.
//!
//! The world passes draw pictures; this is the second thing they write while
//! they do it — for every pixel, the tile and height of *the thing drawn there*.
//! [`crate::blit`] reads it and lights the frame in world coordinates, which is
//! the whole of `docs/lighting.md`'s first decision.
//!
//! # Why the picture alone is not enough
//!
//! The screen folds height into `y`: a brazier in a cellar and a lantern on the
//! street above it land a few pixels apart. Worse, a wall's sprite *stands* on
//! the tile it occludes from — 44 pixels of picture rising out of a diamond at
//! the floor — so anything decided from a pixel's screen position alone puts the
//! face of the wall nearest the flame into the shadow the wall itself casts.
//! There is no arrangement of screen-space masks that separates those; the tile
//! the pixel came from does it exactly.
//!
//! # The format
//!
//! `Rgba16Uint`, as `(x, y, z + 128, kind and the fraction)`. Integers because
//! these are tile indices and a `z`, and a `u16` holds a coordinate on the
//! largest facet a client ships (7,168 across) exactly. `Rgba16Uint` is
//! colour-renderable in WebGL2, which is the ceiling this crate draws under —
//! see the crate docs.
//!
//! The fourth channel carries **the kind in its low two bits, then seven bits of
//! tile-local `x` and seven of tile-local `y`** — where in its tile the pixel
//! is, to a hundred-and-twenty-eighth of one. That fraction is not decoration:
//! without it every pixel of a tile is the same distance from every flame, and a
//! pool of light comes out as flat 44-pixel tiles with a step at each edge
//! rather than as a gradient. It is written by the shaders and read by
//! `blit.wgsl`; the packing appears in three files and only a person reading all
//! three can check that they agree.
//!
//! A sprite has no side of its tile to be on — it is a billboard standing on one
//! — so both fractions are the middle for it, and what varies down its picture
//! is the `z`: four pixels up a wall is one unit of height, which is what gives
//! a wall's face a gradient instead of one brightness.
//!
//! A fragment a sprite discarded writes nothing here either, so what this holds
//! is what is *visible*, which is the question lighting asks.

/// What kind of thing wrote a pixel, or [`Place::NOWHERE`]'s zero for "nothing
/// did".
///
/// The kinds are distinct rather than a single "something is here" bit because
/// they cost nothing — the channel is 16 bits wide and holds a 2 — and the
/// question "is this pixel a mobile" is one a later pass (an outline, a
/// selection) asks without wanting a second attachment for it.
///
/// `wgsl` has these as constants in `blit.wgsl`, and the two must agree; there
/// is a test below that states each value, which is the only thing that can be
/// compared against text a Rust compiler never reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    /// Nothing was drawn here — the cleared background, or a pass that draws
    /// something which is not part of the world at all.
    ///
    /// **Such a pixel is not lit and not dimmed**: the blit passes it through
    /// exactly as the world pass left it. The two things with no place in the
    /// world are the background, which is black either way, and the text drawn
    /// over a speaker's head, which is a message rather than a thing standing in
    /// the street — night must not make it unreadable.
    Nothing = 0,
    /// A land tile.
    Land = 1,
    /// A static, or an item the server put on the ground.
    Static = 2,
    /// A mobile, or something it is wearing.
    Mobile = 3,
}

/// Where in the world a pixel's picture came from.
///
/// Not a [`Point`](openshard_protocol::world::Point): the `kind` is half of what
/// the attachment carries, and a `Point` with a kind beside it in every quad
/// struct is the same thing said in two fields that can disagree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Place {
    /// The tile's `x`.
    pub x: u16,
    /// The tile's `y`.
    pub y: u16,
    /// The height it was drawn at.
    ///
    /// The ground pass ignores this and writes the height its corner
    /// interpolation gives the pixel — a hillside's pixels each carry their own
    /// — so for a [`GroundQuad`](crate::ground::GroundQuad) it is the tile's
    /// base and nothing reads it.
    pub z: i8,
    /// What drew it.
    pub kind: Kind,
}

impl Place {
    /// No place at all: the clear value, and what a pass that draws something
    /// outside the world writes.
    pub const NOWHERE: Self = Self {
        x: 0,
        y: 0,
        z: 0,
        kind: Kind::Nothing,
    };

    /// A pixel of the land on a tile. See [`Place::z`] for why the height is not
    /// an argument.
    pub fn land(x: u16, y: u16) -> Self {
        Self {
            x,
            y,
            z: 0,
            kind: Kind::Land,
        }
    }

    /// A pixel of a static or a ground item standing at `at`.
    pub fn of_static(at: openshard_protocol::world::Point) -> Self {
        Self {
            x: at.x,
            y: at.y,
            z: at.z,
            kind: Kind::Static,
        }
    }

    /// A pixel of a mobile, or of what it is wearing, standing at `at`.
    pub fn of_mobile(at: openshard_protocol::world::Point) -> Self {
        Self {
            x: at.x,
            y: at.y,
            z: at.z,
            kind: Kind::Mobile,
        }
    }

    /// The two words an instance buffer carries this in.
    ///
    /// Packed rather than four fields because a vertex attribute is fetched in
    /// four-byte words either way, and two `u32`s is the smallest this fits in:
    /// `(x | y << 16, (z + 128) | kind << 8)`. The shader takes it apart with
    /// the same two shifts, which are written out there rather than shared —
    /// there is nothing in Rust for a WGSL function to call.
    pub fn packed(self) -> [u32; 2] {
        [
            u32::from(self.x) | u32::from(self.y) << 16,
            (i32::from(self.z) + 128) as u32 | (self.kind as u32) << 8,
        ]
    }
}

/// The format of the attachment. See this module's header.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Uint;

/// Create the attachment at a render target's size.
///
/// Here rather than in the caller for the reason
/// [`depth_texture`](crate::renderer::depth_texture) is: the format is this
/// crate's decision, and a texture created with another one fails at
/// pipeline-bind time with an error that names neither side.
pub fn texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("place"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            // So a frame test can read it back and assert that a wall's pixel
            // names the wall's tile, which is the only way to know the channel
            // is right rather than merely present.
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// What an untouched pixel of the attachment is left as: [`Kind::Nothing`]
/// everywhere.
pub const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

#[cfg(test)]
mod tests {
    use openshard_protocol::world::Point;

    use super::*;

    /// The packing, stated in numbers. It is a contract with two shaders that no
    /// compiler checks, so the bits are pinned here.
    #[test]
    fn a_place_packs_into_two_words() {
        let packed = Place::of_static(Point::new(0x1234, 0x5678, -3)).packed();
        assert_eq!(packed[0], 0x5678_1234, "y in the high half, x in the low");
        assert_eq!(packed[1], (125) | (2 << 8), "z offset by 128, then the kind");
    }

    /// A cleared texel and a [`Place::NOWHERE`] quad say the same thing, and it
    /// is the *kind* that says it.
    ///
    /// The clear is all zeros, which decodes to a `z` of -128; `NOWHERE` writes
    /// a `z` of 0. They differ in that channel and must not differ in the one
    /// that is read: [`Kind::Nothing`] is zero on both sides, and a reader that
    /// looked at the height of a pixel nothing drew would be reading a number
    /// nobody wrote.
    #[test]
    fn nothing_drawn_and_nothing_cleared_are_one_kind() {
        assert_eq!(Kind::Nothing as u32, 0, "the clear value's kind");
        assert_eq!(Place::NOWHERE.packed()[0], 0);
        assert_eq!(Place::NOWHERE.packed()[1] >> 8, 0, "and nothing else is read");
        // The shaders write the kind into the low bits of that channel and the
        // sub-tile fraction above it, so "nothing here" has to be a value the
        // fraction cannot reach into: it is the *kind* that is zero, and the
        // kinds occupy two bits.
        assert!((Kind::Mobile as u32) < 4, "a kind no longer fits its two bits");
    }

    /// The kinds, one number each. `blit.wgsl` holds the same four and cannot be
    /// checked against this by anything but a person reading both.
    #[test]
    fn the_kinds_are_the_numbers_the_shader_has() {
        assert_eq!(Kind::Land as u32, 1);
        assert_eq!(Kind::Static as u32, 2);
        assert_eq!(Kind::Mobile as u32, 3);
    }

    /// The lowest and highest `z` a map holds both survive the offset, which is
    /// the whole reason there is one.
    #[test]
    fn the_ends_of_the_z_range_survive_the_offset() {
        assert_eq!(Place::of_static(Point::new(1, 1, -128)).packed()[1] & 0xFF, 0);
        assert_eq!(Place::of_static(Point::new(1, 1, 127)).packed()[1] & 0xFF, 255);
    }
}
