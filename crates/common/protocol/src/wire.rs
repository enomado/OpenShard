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

use std::fmt;

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

/// A number in the client's own `cliloc.enu`: a line of text the client already
/// has a translation for, so a message costs four bytes instead of a string.
///
/// Always the server's choice — the client only ever looks one up and draws it —
/// so there is no `Raw` counterpart. It lives here rather than beside the packet
/// that first needed it because five carry one: `0xC1` and `0xCC` speech,
/// `0x14`'s context-menu entries, `0xD6`'s property lists, and the start-city
/// descriptions in a `0xA9`. Same reason [`Layer`] is here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ClilocId(pub u32);

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

/// A layer exactly as a client packet proposed it.
///
/// Only `0x13` carries one inbound — the client works the slot out from the
/// item's tiledata and offers it — so by N4's counting rule in
/// `docs/protocol_newtypes.md` this would live in `items.rs`. It is here
/// instead, beside its validated twin, the way [`RawHue`] sits beside [`Hue`]
/// and `RawSerial` beside `Serial`: a pair split across two modules is a pair
/// the next reader has to be told about.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct RawLayer(pub u8);

impl RawLayer {
    /// The [`Layer`] this names.
    ///
    /// Total, and structurally identical to what it wraps, for
    /// `RawStepSequence`'s reason: every byte names a slot, because a layer is
    /// a name and not a range — see [`Layer`]. What the pair records is
    /// *provenance*, which is the only thing that differs between the layer a
    /// client proposed and the layer a server sends back. Whether the slot may
    /// be *worn into* is a different question, and a gameplay one:
    /// `openshard_items::equip_item` answers it.
    #[inline]
    #[must_use]
    pub const fn interpret(self) -> Layer {
        Layer(self.0)
    }
}

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

/// Which character slot a client asked to fill, play, or delete, exactly as
/// sent.
///
/// Three packets carry one and only the third reads it. `create_character`
/// fills the first free slot and `character_play` looks the character up by
/// name, so for `0x00`/`0xF8` and `0x5D` this stays what the pilot called it:
/// a class D value, named and ignored. `0x83` delete is different — the slot
/// *is* the whole request — so the type grew [`validate`](Self::validate) in
/// N6, and the promotion is there for the other two the day slot choice is
/// honoured. See `docs/protocol_newtypes.md`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct RawCharacterSlot(pub u32);

impl RawCharacterSlot {
    /// The slot this names, when the account has `held` characters.
    ///
    /// The same "is this one I offered" check `RawContextMenuIndex::validate`
    /// makes, against a different list: the client is answering the character
    /// list it was last sent, so that list's length is the whole domain. A slot
    /// past the end is a stale screen or a crafted packet, and is refused
    /// rather than clamped — clamping would delete *some* character instead of
    /// none.
    ///
    /// `held` is a count, so an empty account refuses every slot, zero
    /// included.
    pub const fn validate(self, held: usize) -> Result<CharacterSlot, InvalidCharacterSlot> {
        if (self.0 as usize) < held {
            Ok(CharacterSlot(self.0))
        } else {
            Err(InvalidCharacterSlot { slot: self.0, held })
        }
    }
}

/// A character slot the account actually has: an index into the list the client
/// was last sent, counted from zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct CharacterSlot(pub u32);

/// A packet named a character slot the account does not have.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InvalidCharacterSlot {
    /// The slot the client sent.
    pub slot: u32,
    /// How many characters the account actually holds.
    pub held: usize,
}

impl fmt::Display for InvalidCharacterSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "character slot {} was named on an account holding {}",
            self.slot, self.held
        )
    }
}

impl std::error::Error for InvalidCharacterSlot {}

/// A client's self-reported IPv4 address, exactly as sent. Never trusted,
/// never read — the server already knows the real address from the socket.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct RawClientIp(pub u32);

/// A skill id exactly as a client packet carried it, not yet checked against
/// `openshard_state::Skill`'s known ids.
///
/// No promotion method here, and none is coming: the domain type,
/// `openshard_state::Skill`, lives in a server crate above `protocol` (its
/// meaning is gameplay content, not wire shape), so the check that turns this
/// into one is `Skill::from_id`, called at whichever seam has the domain in
/// hand — the same licence `RawSerial::validate` documents for `Serial::new`.
/// Named here rather than in `world.rs`, where the pilot first needed it,
/// because `skill.rs` is its second user — N4's "two or more modules"
/// counting rule. See `docs/protocol_newtypes.md`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct RawSkillId(pub u8);
