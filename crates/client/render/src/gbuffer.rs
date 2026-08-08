//! The attachments a world pass writes beside the picture.
//!
//! One thing per pixel is not enough and never was. A frame's lighting asks
//! where a fragment is, which way its surface looks, what colour it is and what
//! object it belongs to — four different questions, and
//! `docs/lighting_rebuild.md` phase 2 is the decision to answer each of them
//! with *data* rather than with something reconstructed downstream from a
//! packed byte.
//!
//! Two planes today: [`crate::place`]'s `Rgba16Uint` and the position below.
//! The normal and the id plane are the rest of that phase.
//!
//! This module exists so that adding one is not thirty edits. Every fixture
//! that draws a frame — the app, four examples, four test harnesses — used to
//! create the `place` texture by hand, take a view of it, and hand that view to
//! [`Target`](crate::renderer::Target) and to [`Frame`](crate::blit::Frame) as a
//! separate argument. A second attachment repeats all of that; a fourth repeats
//! it three times more, and a fixture that forgot one does not fail to compile
//! until the shader reads a plane nobody bound. A [`Gbuffer`] is the whole set,
//! created once and passed whole.
//!
//! **Owning the textures and lending the views** is the split, and it is forced
//! rather than chosen: `wgpu::Texture::create_view` returns an owned
//! `TextureView`, and a render pass borrows one for as long as it runs. So a
//! caller holds a [`Gbuffer`] for the frame's lifetime and a [`Views`] for the
//! pass's, which is exactly the two lifetimes the hand-written code had — only
//! now they are named.

/// Where each pixel's fragment is in the world: `(x, y, z, 1)`.
///
/// `Rgba32Float`, and the four channels are not a packing — they are the number
/// itself. What this replaces is a `tile` fetched from an instance row, a
/// seven-bit fraction of it read out of the `place` attachment's fourth
/// channel, and a height read out of the third as eight whole units and four
/// sixteenths. Every one of those three is exact in the pass that computed it
/// and quantised on the way here, and every constant on the height track — see
/// `docs/lighting_rebuild.md`'s three roots — exists to compensate for the fact
/// that a fragment's own position was not exactly known.
///
/// Full `f32` and not a smaller float because the thing stored is a map
/// coordinate: `Rgba16Float` has ten bits of mantissa, so a tile at 7,000 —
/// inside a real facet — resolves to about four tiles. The plane is 16 bytes a
/// pixel at the world image's size, which is under 4 MB at any zoom this client
/// draws.
///
/// `z` is in **`z` units**, the map's own, exactly as
/// [`crate::place::unpacked_height`] returned it. `docs/lighting_rebuild.md`'s
/// "one metric" — `z` divided into tiles once, where the map is read — is the
/// next step and not this one: the occlusion grid, every solid's span and the
/// whole shadow walk are stated in `z` units, and a G-buffer that alone counted
/// differently would be a second metric rather than one.
pub const POSITION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// What an untouched pixel of [`POSITION_FORMAT`] is left as.
///
/// Never read: a pixel nothing drew is [`crate::place::Kind::Nothing`], which
/// the blit passes through before it looks at any of this. The fourth channel
/// is `1` where a fragment wrote one and `0` here, so a plane read on its own —
/// by a diagnostic, or by a test — can still tell the two apart without a
/// second fetch.
pub const POSITION_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// The attachments of one frame, at one size.
///
/// Created with the render target and dropped with it: these are the size of
/// the world image, which changes on a resize and a zoom step and on nothing
/// else. See [`Gbuffer::views`] for what a pass is actually handed.
#[derive(Debug)]
pub struct Gbuffer {
    /// Which tile each pixel came from, its height, its stance and its kind —
    /// [`crate::place`]'s `Rgba16Uint`.
    place: wgpu::Texture,
    /// Where that pixel's fragment is, exactly — [`POSITION_FORMAT`].
    position: wgpu::Texture,
}

impl Gbuffer {
    /// Create the set at a render target's size.
    ///
    /// Here rather than in the caller for the reason
    /// [`depth_texture`](crate::renderer::depth_texture) is: a plane's format
    /// is this crate's decision, and one created with another fails at
    /// pipeline-bind time with an error that names neither side.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self {
            place: crate::place::texture(device, width, height),
            position: position_texture(device, width, height),
        }
    }

    /// The views a pass attaches and a reader binds.
    ///
    /// A fresh set per call rather than a field: a view is cheap to make, and
    /// one cached inside the struct that also owns the textures is a borrow
    /// with no good answer the moment a caller wants both.
    pub fn views(&self) -> Views {
        Views {
            place: self.place.create_view(&wgpu::TextureViewDescriptor::default()),
            position: self.position.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// The place plane itself, for a test that copies it back and decodes a
    /// pixel — which is the only way to know a producer stamps the bits it
    /// claims to, rather than merely drawing something plausible.
    pub fn place(&self) -> &wgpu::Texture {
        &self.place
    }

    /// The position plane itself, on the same terms: the phase's own "done
    /// when" is a test that reads this back and finds the world position the
    /// mesh pass computed, to the float.
    pub fn position(&self) -> &wgpu::Texture {
        &self.position
    }
}

/// One view per plane of a [`Gbuffer`], borrowed by a pass for as long as it
/// runs.
#[derive(Debug)]
pub struct Views {
    /// [`Gbuffer::place`]'s.
    pub place: wgpu::TextureView,
    /// [`Gbuffer::position`]'s.
    pub position: wgpu::TextureView,
}

/// One texel of a G-buffer, written by hand.
///
/// Two things build a G-buffer without drawing a frame: [`crate::plan`]'s
/// diagnostic pictures, which say "every pixel is flat ground on the tile above
/// it" and light that, and the fixtures in `tests/` that upload an attachment
/// they chose rather than rendering sprites into one — which is what lets a test
/// about the lighting exist without a client install and without art.
///
/// Before this they each spelled the planes out: a `[u16; 4]` with the kind in
/// two bits and the fraction in fourteen, `packed_height` beside it, and — the
/// day a second plane arrived — a position that had to agree with all of that.
/// Three copies of one format, and the failure mode is not a compile error but a
/// picture of a surface nobody would have drawn. This is the format once, taking
/// the numbers a producer actually knows.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Fragment {
    /// Which tile the thing drawn here stands on.
    pub tile: (u16, u16),
    /// Where in that tile, each axis in `0..1` — [`crate::place::Stance`]
    /// decides what the pair *means*, and this is the number itself.
    pub sub: (f32, f32),
    /// How high, in `z` units.
    pub z: f32,
    /// What drew it.
    pub kind: crate::place::Kind,
    /// Which way its surface looks.
    pub stance: crate::place::Stance,
}

impl Fragment {
    /// The four `u16`s of [`crate::place::FORMAT`]'s texel.
    pub fn place(self) -> [u16; 4] {
        let step = |value: f32| (value.clamp(0.0, 1.0) * crate::place::SUB_TILE).round() as u16;
        [
            self.tile.0,
            self.tile.1,
            crate::place::packed_height(self.z, self.stance),
            self.kind as u16 | step(self.sub.0) << 2 | step(self.sub.1) << 9,
        ]
    }

    /// The four floats of [`POSITION_FORMAT`]'s.
    ///
    /// The *unquantised* pair, which is the whole point of the plane: what the
    /// place texel above rounds to a hundred-and-twenty-seventh of a tile and a
    /// sixteenth of a `z` unit, this states outright.
    pub fn position(self) -> [f32; 4] {
        [
            f32::from(self.tile.0) + self.sub.0,
            f32::from(self.tile.1) + self.sub.1,
            self.z,
            1.0,
        ]
    }
}

/// Create the position plane on its own — [`Gbuffer::new`]'s second half,
/// separate only because the usage flags want saying once.
fn position_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("position"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: POSITION_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            // So a test can read a fragment's position back and compare it
            // against the position the pass that drew it computed.
            | wgpu::TextureUsages::COPY_SRC
            // And so a fixture can write one: the parity frames upload a
            // G-buffer they built by hand rather than drawing sprites, which
            // is what lets a test about the lighting exist without an install
            // and without art. See `crate::place::texture`, which says the
            // same of its own plane.
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
