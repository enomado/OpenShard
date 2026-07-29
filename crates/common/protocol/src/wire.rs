//! The small named things packet fields are made of.
//!
//! A UO packet is mostly numbers, and the numbers are not interchangeable: a
//! graphic is not a hue, a sound is not a cliloc, and the compiler is the only
//! thing that will ever notice the difference. So every id-shaped field gets a
//! newtype, and `.0` is unwrapped inside a codec and nowhere else.
//!
//! # These arrive as they are needed
//!
//! A newtype for a packet nobody has read closely yet is a guess, and a guess
//! that hardens before it is right is worse than a bare `u16`. So this module
//! grows one type at a time, in the stage that first has a field for it — see
//! `docs/protocol_rewrite.md`. [`Serial`](crate::serial::Serial) is the
//! exception and lives in its own module: it carries a validity rule and a
//! pool split, not just a name.

/// An art id: what the client draws. Tiles, items, effect sprites and gump art
/// all index the same `art.mul`, so they share one type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Graphic(pub u16);

/// A colour index into `hues.mul`. `0` means "as the art was drawn".
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Hue(pub u16);

impl Hue {
    /// No tint: the art's own colours.
    pub const NONE: Self = Self(0);
}

/// An index into the client's sound files.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SoundId(pub u16);

/// The id a targeting cursor request carries and its response echoes back.
///
/// Opaque to the client: the server picks it, the client repeats it, and that is
/// how a click is matched to the request that asked for it. Nothing about it is
/// a serial, even where a server happens to use one as the value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct CursorId(pub u32);
