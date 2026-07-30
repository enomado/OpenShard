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

/// The key a `0x8C` relay hands out and a `0x91` game login must echo back.
///
/// Opaque the same way [`CursorId`] is: the login server picks it from OS
/// entropy (see `openshard_login::auth::AuthKeys::issue`), and the only thing
/// that makes it valid is that it was issued and not yet redeemed — nothing
/// about the number itself means anything.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct AuthKey(pub u32);

/// Where a worn item sits on a mobile: the hand, the head, the mount slot.
///
/// The numbers are the client's — it decides which sprite a layer draws over
/// which — so the type is a byte with a name and nothing more, exactly as
/// [`StatusFlags`](crate::mobile::StatusFlags) is. Modelling the twenty-odd
/// layers as an enum would be a guess about the ones this engine has never sent.
///
/// It lives here rather than beside either packet that carries it because both
/// do: a mobile's `0x78` outfit list and an item's `0x2E`/`0x13` equip pair.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Layer(pub u8);

/// A colour choice exactly as a client packet carried it: not yet checked
/// against the set of hues this shard actually allows. See
/// `docs/protocol_newtypes.md` — the allowed set is content, so the check that
/// turns this into a real [`Hue`] lives above `protocol`, and does not exist
/// yet.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct RawHue(pub u16);

/// An art id exactly as a client packet carried it — a hairstyle, a beard —
/// not yet checked against the set this shard actually allows. Same status as
/// [`RawHue`]: the check does not exist yet.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct RawGraphic(pub u16);

/// Which character slot a client asked to fill or play, exactly as sent.
///
/// Class D for now: `create_character` fills the first free slot and
/// `character_play` looks the character up by name, so neither reads this
/// value. It becomes class C — validated against the account's actual
/// character count — the day slot choice is honoured; see
/// `docs/protocol_newtypes.md`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct RawCharacterSlot(pub u32);

/// A client's self-reported IPv4 address, exactly as sent. Never trusted,
/// never read — the server already knows the real address from the socket.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct RawClientIp(pub u32);
