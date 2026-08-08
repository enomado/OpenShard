//! The attachments a world pass writes beside the picture.
//!
//! One thing per pixel is not enough and never was. A frame's lighting asks
//! where a fragment is, which way its surface looks, what colour it is and what
//! object it belongs to — four different questions, and
//! `docs/lighting_rebuild.md` phase 2 is the decision to answer each of them
//! with *data* rather than with something reconstructed downstream from a
//! packed byte.
//!
//! Three planes: the position below, the normal beside it, and the id plane
//! that says what a fragment *is*. Albedo is the fourth and it is phase 6's —
//! a mesh face has none today, and the day it does the picture stops being a
//! separate attachment because the albedo *is* the picture.
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
//!
//! # What a G-buffer costs to bind, and the one limit it does not fit under
//!
//! A world pass writes the picture and every plane here in one draw, and WebGPU
//! bounds the *total* bytes a fragment may write across a pass's colour
//! attachments: `maxColorAttachmentBytesPerSample`, whose guaranteed minimum is
//! **32**. See [`required_limits`] — this is the one place in the crate that
//! asks an adapter for more than the floor, and the arithmetic is written out
//! there.

/// The device limits this crate's pipelines need, above the guaranteed
/// minimum — one spelling, because eleven call sites request a device and a
/// tenth of them getting this right is a pipeline that fails to create on one
/// machine and not on another.
///
/// **Only `max_color_attachment_bytes_per_sample`, and only because deferred
/// shading is what it bounds.** A world pass writes the picture and every plane
/// of the G-buffer in one draw, and WebGPU counts the bytes a fragment writes
/// across all of them together. The floor is 32 and the set here is past it —
/// [`ATTACHMENT_BYTES_PER_SAMPLE`] is the sum, spelled out and pinned by a test,
/// so that a plane added or a format widened moves a number a person can read
/// rather than turning into a validation error naming no cause.
///
/// This is a real portability cost and it is worth stating plainly: **a device
/// that reports only the guaranteed minimum cannot run this client.** The
/// ceiling is WebGPU (see the crate docs) and desktop adapters report far more
/// than 32 — but the floor is a floor, and asking for more than it is a choice
/// rather than something that happened. The choice is
/// `docs/lighting_rebuild.md`'s: a fragment's position, its normal and what it
/// belongs to are *data*, and data has a size. Phase 2's own end brings the sum
/// back down — the target layout there is position, normal, albedo and ids at
/// exactly 32, with no separate picture beside them, because the albedo *is*
/// the picture.
///
/// The id plane bought four of the sixteen back on arrival: [`IDS_FORMAT`] is
/// an `R32Uint` where the `place` attachment it replaced was an `Rgba16Uint`,
/// and the eight bytes that plane charged were mostly a height and a fraction
/// the position plane now carries exactly. The twelve still over the floor are
/// [`NORMAL_FORMAT`]'s, and its own doc says what would buy them.
pub fn required_limits() -> wgpu::Limits {
    wgpu::Limits {
        max_color_attachment_bytes_per_sample: attachment_bytes_per_sample(),
        ..wgpu::Limits::default()
    }
}

/// What one fragment of a world pass writes, in bytes: the picture plus every
/// plane of a [`Gbuffer`].
///
/// Summed from [`ATTACHMENTS`] rather than written down, so that a plane added
/// or a format widened moves the limit with it and cannot be forgotten. What is
/// written down is [`ATTACHMENT_BYTES_PER_SAMPLE`], the number this is expected
/// to come to — the two are compared by `a_g_buffer_costs_what_it_says`, which
/// is what makes a change to the set show up as a number a person reads rather
/// than as a silently wider request.
///
/// Not a count of the bytes the *formats* obviously are — WebGPU charges each
/// attachment a per-format cost of its own, rounded for alignment, so an
/// `Rgba8Unorm` picture costs 8 rather than 4.
fn attachment_bytes_per_sample() -> u32 {
    ATTACHMENTS
        .iter()
        .map(|format| {
            format
                .target_pixel_byte_cost()
                .expect("every attachment of a world pass is colour-renderable")
        })
        .sum()
}

/// What that sum is expected to be. See [`attachment_bytes_per_sample`].
pub const ATTACHMENT_BYTES_PER_SAMPLE: u32 = 44;

/// Every colour attachment a world pass writes, in the order it binds them.
///
/// Here rather than in the test that sums them so the list is beside the number
/// it justifies — and so that the day the picture stops being a separate
/// attachment (phase 6's impostor, where the albedo *is* the G-buffer) the
/// deletion is one line in one place.
const ATTACHMENTS: [wgpu::TextureFormat; 4] = [
    crate::blit::WORLD_FORMAT,
    IDS_FORMAT,
    POSITION_FORMAT,
    NORMAL_FORMAT,
];

/// What drew each pixel: its [`crate::place::Kind`], its
/// [`crate::place::Stance`], and the instance row the picture came from.
///
/// `R32Uint`, and the layout is [`pack_ids`]. Integers because these are three
/// identities: a kind is an enumeration, a stance is an enumeration, and an id
/// is a row number. `R32Uint` is colour-renderable in WebGL2, which is the
/// ceiling this crate draws under — see the crate docs.
///
/// **This is what is left of the `place` attachment**, which was four `u16`
/// channels holding all of the above plus a height in whole units and
/// sixteenths and seven bits of tile-local `x` and `y`.
/// `docs/lighting_rebuild.md` phase 2 moved the height and the fraction into
/// [`POSITION_FORMAT`] and the facing they stood in for into
/// [`NORMAL_FORMAT`]; what was left fitted in six bits and an id, so eight
/// bytes a fragment became four. See [`required_limits`] for what that bought.
///
/// One `u32` and not four narrower channels because there is nothing to split:
/// a reader wants the whole word and asks it three questions, and three
/// channels of a `Rgba8Uint` would cap the id at 255. Nor is it folded into
/// the position plane's fourth channel, which is a *coverage* bit — a float
/// that has to round-trip an integer id is a format with a silent ceiling,
/// and this one has an honest one.
pub const IDS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// What an untouched pixel of [`IDS_FORMAT`] is left as: zero, which is
/// [`crate::place::Kind::Nothing`] with no stance and no row.
///
/// **The one clear value in this module that is read.** The other two planes'
/// are never looked at — the lighting has already passed a `Kind::Nothing`
/// pixel through untouched by the time it would reach them — and that test is
/// this word. So the kind lives at the bottom of [`pack_ids`]'s layout, where a
/// zero word and a stamped-as-nothing word are the same number;
/// `nothing_drawn_and_nothing_cleared_are_one_kind` is the other half of it.
pub const IDS_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// Where a [`crate::place::Stance`] rides in an id word, above the kind's two
/// bits: four bits, which is what its eleven values need.
///
/// `place_format.wesl`'s `IDS_STANCE_SHIFT`, and nothing but a person reading
/// both files can compare them — [`pack_ids`] below is what everything on this
/// side goes through.
pub const IDS_STANCE_SHIFT: u32 = 2;

/// The stance's mask: `place_format.wesl`'s `IDS_STANCE_MASK`.
pub const IDS_STANCE_MASK: u32 = 15;

/// And where the row's own number rides, above both: twenty-six bits.
///
/// Not a budget anyone is near — an id is a row in one of four instance
/// buffers and the widest frame this client has drawn is in the low thousands
/// — but a real ceiling rather than an unstated one, which is why
/// [`pack_ids`] asserts against it.
pub const IDS_ID_SHIFT: u32 = 6;

/// One id word, from the three things it is made of — the Rust twin of
/// `place_format.wesl`'s `pack_ids`.
///
/// For the two things on this side that build a G-buffer without drawing a
/// frame: [`crate::plan`]'s diagnostic pictures and the fixtures in `tests/`.
/// See [`Fragment`], which is what they actually call.
pub fn pack_ids(id: u32, stance: crate::place::Stance, kind: crate::place::Kind) -> u32 {
    debug_assert!(
        id < 1 << (32 - IDS_ID_SHIFT),
        "an instance id past what an id word holds: {id}",
    );
    kind as u32 | (stance as u32) << IDS_STANCE_SHIFT | id << IDS_ID_SHIFT
}

/// The kind an id word names — the twin of `place_format.wesl`'s `ids_kind`,
/// for a test that reads a rendered plane back.
///
/// `None` for a word no [`crate::place::Kind`] spells, which the two bits
/// cannot produce; it is a `match`'s honest arm rather than a case a caller
/// has to handle.
pub fn ids_kind(word: u32) -> Option<crate::place::Kind> {
    use crate::place::Kind;

    match word & 3 {
        0 => Some(Kind::Nothing),
        1 => Some(Kind::Land),
        2 => Some(Kind::Static),
        3 => Some(Kind::Mobile),
        _ => None,
    }
}

/// The stance it names, and the row it names: `place_format.wesl`'s
/// `ids_stance` and `ids_id`.
pub fn ids_stance(word: u32) -> u32 {
    (word >> IDS_STANCE_SHIFT) & IDS_STANCE_MASK
}

/// See [`ids_stance`].
pub fn ids_id(word: u32) -> u32 {
    word >> IDS_ID_SHIFT
}

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

/// Which way each pixel's surface looks: `(x, y, z, 1)`, a unit vector, or all
/// zeros for a surface with no known facing.
///
/// **The zero vector is a value here and not an absence**, which is what
/// decides the format. A billboard has no side — a tree, a body, a wall whose
/// art named no edge — so every flame that reaches it lights it, and
/// `blit.wesl` tests `any(normal != 0)` for exactly that. Land is the same
/// answer for a different reason (see `docs/lighting_rebuild.md`'s third open
/// question, which is whether it should be). Both are transitional: phase 6
/// gives an upright static an impostor normal and phase 7 gives a mobile one,
/// and when they do, this channel stops having a third case.
///
/// **Not `Rg16Snorm`, octahedral, which is what `docs/lighting_rebuild.md`'s
/// own table says.** Two reasons, and the first is decisive: every 16-bit norm
/// format is behind `wgpu::Features::TEXTURE_FORMAT_16BIT_NORM`, which is
/// native-only and not in WebGPU's core set — the ceiling this crate draws
/// under (see the crate docs). The nearest compact renderable format is
/// `Rgba16Float`, and it is not taken either: [`crate::plan`]'s diagnostic
/// pictures and the fixtures in `tests/` *write* this plane from the CPU, and
/// there is no `f16` on that side, so it would mean a hand-rolled encoder — a
/// second spelling of a float format with no compiler comparing the two, which
/// is the class of defect this crate has already paid for twice. The second
/// reason is the octahedral mapping itself: it has no zero, so the case above
/// would need a channel of its own anyway.
///
/// So it is the position plane's own format, written and read the same way.
/// Shrinking it is welcome later, on the same terms the position plane states
/// for its own reconstruction: gated on a test that the encoded normal comes
/// back equal to the stored one. What it would buy is not memory but
/// [`ATTACHMENT_BYTES_PER_SAMPLE`] — sixteen bytes a fragment against the four
/// an octahedral pair needs, which is half of what this crate asks an adapter
/// for above the floor.
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// What an untouched pixel of [`NORMAL_FORMAT`] is left as.
///
/// All zeros, and the fourth channel is the one that separates "nothing drew
/// here" from "something drew here and has no facing" — the same coverage bit
/// [`POSITION_CLEAR`] carries, for the same reason. The lighting never asks:
/// it has already passed a [`crate::place::Kind::Nothing`] pixel through
/// untouched. `View::Normal` does.
pub const NORMAL_CLEAR: wgpu::Color = wgpu::Color {
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
    /// What drew each pixel and which row it came from — [`IDS_FORMAT`].
    ids: wgpu::Texture,
    /// Where that pixel's fragment is, exactly — [`POSITION_FORMAT`].
    position: wgpu::Texture,
    /// And which way its surface looks — [`NORMAL_FORMAT`].
    normal: wgpu::Texture,
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
            ids: plane(device, "ids", IDS_FORMAT, width, height),
            position: plane(device, "position", POSITION_FORMAT, width, height),
            normal: plane(device, "normal", NORMAL_FORMAT, width, height),
        }
    }

    /// The views a pass attaches and a reader binds.
    ///
    /// A fresh set per call rather than a field: a view is cheap to make, and
    /// one cached inside the struct that also owns the textures is a borrow
    /// with no good answer the moment a caller wants both.
    pub fn views(&self) -> Views {
        Views {
            ids: self.ids.create_view(&wgpu::TextureViewDescriptor::default()),
            position: self.position.create_view(&wgpu::TextureViewDescriptor::default()),
            normal: self.normal.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// The id plane itself, for a test that copies it back and decodes a
    /// pixel — which is the only way to know a producer stamps the bits it
    /// claims to, rather than merely drawing something plausible.
    pub fn ids(&self) -> &wgpu::Texture {
        &self.ids
    }

    /// The position plane itself, on the same terms: the phase's own "done
    /// when" is a test that reads this back and finds the world position the
    /// mesh pass computed, to the float.
    pub fn position(&self) -> &wgpu::Texture {
        &self.position
    }

    /// The normal plane itself, on the same terms again: the other half of the
    /// phase's "done when" is a test that reads this back and finds the
    /// geometry's own normal, and nothing derived from a stance.
    pub fn normal(&self) -> &wgpu::Texture {
        &self.normal
    }
}

/// One view per plane of a [`Gbuffer`], borrowed by a pass for as long as it
/// runs.
#[derive(Debug)]
pub struct Views {
    /// [`Gbuffer::ids`]'s.
    pub ids: wgpu::TextureView,
    /// [`Gbuffer::position`]'s.
    pub position: wgpu::TextureView,
    /// [`Gbuffer::normal`]'s.
    pub normal: wgpu::TextureView,
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
/// two bits and the fraction in fourteen, a packed height beside it, and — the
/// day a second plane arrived — a position that had to agree with all of that.
/// Three copies of one format, and the failure mode is not a compile error but a
/// picture of a surface nobody would have drawn. This is the format once, taking
/// the numbers a producer actually knows.
///
/// **The tile is here and the id is not**, which is the one asymmetry and it is
/// the caller's shape rather than this type's: a fixture describes a surface by
/// where it stands, and the id is a row number it can only hand out once it has
/// seen every fragment it means to draw. So the tile is a field and the id is
/// [`Fragment::ids`]'s argument — see [`crate::plan`]'s own two passes over its
/// texels for what that looks like.
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
    /// The one word of [`IDS_FORMAT`]'s texel, given the row this fragment's
    /// picture came from. See [`pack_ids`].
    pub fn ids(self, id: u32) -> u32 {
        pack_ids(id, self.stance, self.kind)
    }

    /// The four floats of [`POSITION_FORMAT`]'s.
    ///
    /// The *unquantised* pair, which is the whole point of the plane: what the
    /// place attachment rounded to a hundred-and-twenty-seventh of a tile and a
    /// sixteenth of a `z` unit, this states outright.
    pub fn position(self) -> [f32; 4] {
        [
            f32::from(self.tile.0) + self.sub.0,
            f32::from(self.tile.1) + self.sub.1,
            self.z,
            1.0,
        ]
    }

    /// And the four of [`NORMAL_FORMAT`]'s.
    ///
    /// Derived from the stance rather than carried as a fifth field, because
    /// that is what the world passes themselves do: `statics.wesl` writes
    /// `outward` of the stance it just resolved a corner into, and there is no
    /// producer anywhere that knows a facing this enum cannot name. The one
    /// place the two part company is **land**, which carries
    /// [`crate::place::Stance::Flat`] and no normal — `ground.wesl`'s own
    /// choice, and see [`crate::place::Stance::normal`] for why the kind and
    /// not the stance is what tells the ground from a wall's flat cap.
    ///
    /// The day the mesh pass is not the only producer with geometry of its own
    /// — phase 6's impostor, phase 7's billboard — this grows a field and the
    /// derivation goes.
    pub fn normal(self) -> [f32; 4] {
        let [x, y, z] = match self.kind {
            crate::place::Kind::Land => [0.0; 3],
            _ => self.stance.normal(),
        };
        [x, y, z, 1.0]
    }
}

/// Create one plane of the set — all three share every flag, and naming them
/// once is what keeps a fourth from arriving with a usage the others have and
/// it does not.
fn plane(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// [`ATTACHMENT_BYTES_PER_SAMPLE`] is the sum of what the attachments
    /// actually cost, and it is past the floor.
    ///
    /// The number is a device limit this crate asks an adapter for, so getting
    /// it wrong is not a wrong picture: it is `request_device` refusing on one
    /// machine, or a pipeline failing to create with an error that names a total
    /// and no attachment. Summed from wgpu's own table rather than from the
    /// widths a person reads off the format names — WebGPU charges for
    /// alignment, and an `Rgba8Unorm` picture costs eight bytes and not four,
    /// which is exactly the sort of thing nobody guesses right.
    #[test]
    fn a_g_buffer_costs_what_it_says() {
        let total = attachment_bytes_per_sample();
        assert_eq!(total, ATTACHMENT_BYTES_PER_SAMPLE);
        assert_eq!(required_limits().max_color_attachment_bytes_per_sample, total);
        // And the fact that makes this constant exist at all. If a plane is ever
        // shrunk back under the floor, this is the line that says so — and the
        // whole of `required_limits` goes with it.
        assert!(
            total > wgpu::Limits::default().max_color_attachment_bytes_per_sample,
            "the G-buffer fits the guaranteed minimum: ask for nothing above it",
        );
    }

    /// An id word round-trips: the three things put into it come back out, and
    /// none of them reaches into another's bits.
    ///
    /// The layout is a contract with `place_format.wesl`, which no compiler
    /// reads, so what can be pinned on this side is that the four functions
    /// here are consistent with each other and that the fields are disjoint.
    /// The case worth the test is the **top** one: an id is shifted six bits
    /// left, so a row number near the ceiling is exactly where a field would
    /// start losing its high end silently — a wrong picture rather than a
    /// fault.
    #[test]
    fn an_id_word_holds_three_things_and_gives_all_three_back() {
        use crate::place::{Kind, Stance};

        for id in [0, 1, 4095, (1 << 26) - 1] {
            for stance in [Stance::Upright, Stance::Flat, Stance::MeshFace] {
                for kind in [Kind::Land, Kind::Static, Kind::Mobile] {
                    let word = pack_ids(id, stance, kind);
                    assert_eq!(ids_id(word), id, "{id}/{stance:?}/{kind:?}");
                    assert_eq!(ids_stance(word), stance as u32, "{id}/{stance:?}/{kind:?}");
                    assert_eq!(ids_kind(word), Some(kind), "{id}/{stance:?}/{kind:?}");
                }
            }
        }
        // And the invariant every reader's first branch rests on: a pixel
        // nothing drew and a pixel stamped as nothing are the same word.
        assert_eq!(pack_ids(0, Stance::Upright, Kind::Nothing), 0);
        assert_eq!(ids_kind(0), Some(Kind::Nothing));
    }

    /// A fragment's normal comes off its stance, except for land, which has
    /// none — the pair `blit.wgsl` used to make with a `select` on the kind and
    /// the producers make now.
    #[test]
    fn a_land_fragment_has_no_facing_and_a_floor_does() {
        let at = |kind, stance| {
            Fragment {
                tile: (100, 200),
                sub: (0.25, 0.75),
                z: 3.5,
                kind,
                stance,
            }
            .normal()
        };
        // The ground and a rug lying on it are the same stance and the same
        // shape of pixel, and only one of them is gated by which side of its own
        // plane a flame is on. See `ground.wesl`, which has the measurement.
        assert_eq!(
            at(crate::place::Kind::Land, crate::place::Stance::Flat),
            [0.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(
            at(crate::place::Kind::Static, crate::place::Stance::Flat),
            [0.0, 0.0, 1.0, 1.0],
        );
        // A billboard has no side, and says so with a zero — which is why the
        // fourth channel is coverage and not part of the vector.
        assert_eq!(
            at(crate::place::Kind::Mobile, crate::place::Stance::Upright),
            [0.0, 0.0, 0.0, 1.0],
        );
    }
}
