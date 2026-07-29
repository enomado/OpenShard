//! Getting a character into the world, and walking it around.
//!
//! ```text
//!   client                                server
//!     │  0x5D character play                │
//!     │────────────────────────────────────>│
//!     │              0x1B start             │   puts the body in the world
//!     │<────────────────────────────────────│
//!     │              0xBF.0x08 map change   │
//!     │              0x20 player update     │
//!     │              0x4F light level       │
//!     │              0x55 login complete    │   the client starts drawing
//!     │<────────────────────────────────────│
//!     │  0x02 walk request                  │
//!     │────────────────────────────────────>│
//!     │              0x22 ack / 0x21 reject │
//!     │<────────────────────────────────────│
//! ```
//!
//! Layouts from SphereServer's `network/send.cpp` and `receive.cpp`.

use std::fmt;

use crate::codec::{PacketReader, PacketWriter};
use crate::direction::Facing;
use crate::error::{DecodeError, WrongPacket};
use crate::identity::RawCharacterName;
use crate::login::CHARACTER_NAME_LENGTH;
use crate::packet::{DecodePacket, EncodePacket, PacketLength};
use crate::version::ClientVersion;

/// Where something is.
///
/// `z` is signed and one byte: UO's world is 256 units tall and the client has
/// no way to express more.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Point {
    /// East-west tile.
    pub x: u16,
    /// North-south tile.
    pub y: u16,
    /// Height.
    pub z: i8,
}

impl Point {
    /// A point.
    pub const fn new(x: u16, y: u16, z: i8) -> Self {
        Self { x, y, z }
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}

// -- 0x5D character play --------------------------------------------------

/// `0x5D` — the client picks a character from the list. 73 bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CharacterPlay {
    /// The character's name, echoed from the 0xA9 list.
    pub name: String,
    /// Which slot, zero-based, into the list the server sent.
    pub slot: u32,
    /// The client's own claimed IPv4, as a raw dword. Not to be trusted or used.
    pub client_ip: u32,
}

impl DecodePacket for CharacterPlay {
    const ID: u8 = 0x5D;

    fn decode_body(
        reader: &mut PacketReader<'_>,
        _version: ClientVersion,
    ) -> Result<Self, DecodeError> {
        // A constant the client always sends. Sphere ignores it and so do we:
        // rejecting on it would be a compatibility risk for no gain.
        reader.skip(4)?;
        let name = reader.fixed_string(30)?;
        reader.skip(2)?; // unknown
        reader.skip(4)?; // client flags
        reader.skip(24)?; // unknown / login count
        let slot = reader.u32()?;
        let client_ip = reader.u32()?;
        Ok(Self {
            name,
            slot,
            client_ip,
        })
    }
}

impl CharacterPlay {
    /// Encode a whole 0x5D packet. Test fixtures only — see `login`'s module
    /// docs: this server never sends one, only ever decodes it.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = PacketWriter::with_capacity(73);
        writer.u8(Self::ID);
        writer.u32(0xEDED_EDED); // the constant the client sends
        writer.fixed_string(&self.name, 30);
        writer.zeros(2);
        writer.zeros(4);
        writer.zeros(24);
        writer.u32(self.slot);
        writer.u32(self.client_ip);
        writer.into_bytes()
    }
}

// -- 0x00 / 0xF8 create character -----------------------------------------

/// The race a player picked at character creation.
///
/// The world does not model races yet; this exists so the create packet can be
/// decoded without losing what the player chose, and so [`CreateCharacter::body`]
/// can pick the right graphic.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Race {
    /// The default, and the only one before Mondain's Legacy.
    Human,
    /// Since Mondain's Legacy.
    Elf,
    /// Since Stygian Abyss.
    Gargoyle,
}

/// One starting skill a player chose at creation: which skill, and its value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct SkillChoice {
    /// The skill id, as the client numbers them.
    pub skill: u8,
    /// Its starting value; the client sends whole points here. Stored raw.
    pub value: u8,
}

/// `0x00` / `0xF8` — the client asks to create a character.
///
/// # Two ids, one packet
///
/// `0x00` is the classic 104-byte form with three starting skills. `0xF8` is
/// what ClassicUO 7.0.16 and later send — 106 bytes, with a fourth skill. The
/// two are otherwise byte-for-byte identical, so they decode through one path
/// that differs only by how many skill pairs it reads. Which id a client uses is
/// the client's business; the shard accepts both.
///
/// The sex/race byte is read with the Stygian Abyss encoding (`0x2`–`0x7`), what
/// every client that reaches character creation on a modern shard sends. A
/// genuinely pre-SA client using the old `0x0`–`0x3` encoding would have its race
/// read one off; that is a deliberate simplification while the world models no
/// races, noted here so it is a choice and not a surprise.
///
/// # Why this is not a [`DecodePacket`]
///
/// [`DecodePacket`] assumes one packet has one `const ID`. This one logically
/// decodes across *two* ids (`0x00`, `0x1F8`) with two different fixed lengths —
/// the same shape of problem the Stage 2 pilot hit with `0xB9`
/// (`docs/protocol_rewrite.md`, "Amendments forced by the Stage 2 pilot"), and
/// the Stage 3 pilot's counterpart to it. So [`Self::decode`] stays a plain
/// inherent method rather than bending the trait to fit two ids.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreateCharacter {
    /// The new character's name.
    pub name: RawCharacterName,
    /// Client flags reported at creation.
    pub flags: u32,
    /// The chosen profession, or 0 for the "advanced"/custom option.
    pub profession: u8,
    /// The raw sex/race byte, exactly as sent. [`Self::race`] and
    /// [`Self::is_female`] interpret it.
    pub sex_race: u8,
    /// Starting strength.
    pub strength: u8,
    /// Starting dexterity.
    pub dexterity: u8,
    /// Starting intelligence.
    pub intelligence: u8,
    /// The starting skills: three for `0x00`, four for `0xF8`.
    pub skills: Vec<SkillChoice>,
    /// Skin hue.
    pub skin_hue: u16,
    /// Hair graphic.
    pub hair: u16,
    /// Hair hue.
    pub hair_hue: u16,
    /// Facial-hair graphic.
    pub beard: u16,
    /// Facial-hair hue.
    pub beard_hue: u16,
    /// Which starting city the player picked, as an index into the list the
    /// character-list packet offered.
    pub start_location: u8,
    /// Which character slot to fill.
    pub slot: u32,
    /// Shirt hue.
    pub shirt_hue: u16,
    /// Trousers hue.
    pub pants_hue: u16,
}

impl CreateCharacter {
    /// The classic create-character id: 104 bytes, three skills.
    pub const ID_CLASSIC: u8 = 0x00;
    /// The 7.0.16+ create-character id: 106 bytes, four skills.
    pub const ID_HIGH_SEAS: u8 = 0xF8;

    /// Decode either the `0x00` or the `0xF8` create-character packet.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = PacketReader::new(bytes);
        let id = reader.u8()?;
        let skill_count = match id {
            Self::ID_CLASSIC => 3,
            Self::ID_HIGH_SEAS => 4,
            found => {
                return Err(DecodeError::WrongPacket(WrongPacket {
                    expected: Self::ID_HIGH_SEAS,
                    found,
                }))
            }
        };

        // pattern1 (4), pattern2 (4), a "kuoc" byte (1) — constants the client
        // sends and the server has no use for.
        reader.skip(9)?;
        let name = RawCharacterName(reader.fixed_string(CHARACTER_NAME_LENGTH)?);
        reader.skip(2)?; // 0x0000
        let flags = reader.u32()?;
        reader.skip(8)?; // unknown
        let profession = reader.u8()?;
        reader.skip(15)?; // 0x00 * 15
        let sex_race = reader.u8()?;
        let strength = reader.u8()?;
        let dexterity = reader.u8()?;
        let intelligence = reader.u8()?;

        let mut skills = Vec::with_capacity(skill_count);
        for _ in 0..skill_count {
            let skill = reader.u8()?;
            let value = reader.u8()?;
            skills.push(SkillChoice { skill, value });
        }

        let skin_hue = reader.u16()?;
        let hair = reader.u16()?;
        let hair_hue = reader.u16()?;
        let beard = reader.u16()?;
        let beard_hue = reader.u16()?;
        reader.skip(1)?; // shard index
        let start_location = reader.u8()?;
        let slot = reader.u32()?;
        reader.skip(4)?; // the client's claimed ip; not to be trusted
        let shirt_hue = reader.u16()?;
        let pants_hue = reader.u16()?;

        Ok(Self {
            name,
            flags,
            profession,
            sex_race,
            strength,
            dexterity,
            intelligence,
            skills,
            skin_hue,
            hair,
            hair_hue,
            beard,
            beard_hue,
            start_location,
            slot,
            shirt_hue,
            pants_hue,
        })
    }

    /// Whether the character is female. Odd sex/race values are female on every
    /// client — Sphere notes this rule holds across versions.
    pub const fn is_female(&self) -> bool {
        !self.sex_race.is_multiple_of(2)
    }

    /// The chosen race, read with the Stygian Abyss encoding.
    pub const fn race(&self) -> Race {
        match self.sex_race {
            0x4 | 0x5 => Race::Elf,
            0x6 | 0x7 => Race::Gargoyle,
            // 0x2 / 0x3, and anything unexpected, is a human — the safe default
            // Sphere falls back to.
            _ => Race::Human,
        }
    }

    /// The body graphic for this character's race and sex.
    pub const fn body(&self) -> u16 {
        match (self.race(), self.is_female()) {
            (Race::Human, false) => 0x0190,
            (Race::Human, true) => 0x0191,
            (Race::Elf, false) => 0x025D,
            (Race::Elf, true) => 0x025E,
            (Race::Gargoyle, false) => 0x029A,
            (Race::Gargoyle, true) => 0x029B,
        }
    }

    /// Encode the packet. The `0xF8` (four-skill) form is written when four
    /// skills are present, the classic `0x00` form otherwise. Mostly for tests.
    pub fn encode(&self) -> Vec<u8> {
        let high_seas = self.skills.len() >= 4;
        let capacity = if high_seas { 106 } else { 104 };
        let mut writer = PacketWriter::with_capacity(capacity);
        writer.u8(if high_seas {
            Self::ID_HIGH_SEAS
        } else {
            Self::ID_CLASSIC
        });
        writer.zeros(9); // pattern1, pattern2, kuoc
        writer.fixed_string(&self.name.0, CHARACTER_NAME_LENGTH);
        writer.zeros(2);
        writer.u32(self.flags);
        writer.zeros(8);
        writer.u8(self.profession);
        writer.zeros(15);
        writer.u8(self.sex_race);
        writer.u8(self.strength);
        writer.u8(self.dexterity);
        writer.u8(self.intelligence);

        let count = if high_seas { 4 } else { 3 };
        for index in 0..count {
            let choice = self.skills.get(index).copied().unwrap_or_default();
            writer.u8(choice.skill);
            writer.u8(choice.value);
        }

        writer.u16(self.skin_hue);
        writer.u16(self.hair);
        writer.u16(self.hair_hue);
        writer.u16(self.beard);
        writer.u16(self.beard_hue);
        writer.zeros(1); // shard index
        writer.u8(self.start_location);
        writer.u32(self.slot);
        writer.zeros(4); // client ip
        writer.u16(self.shirt_hue);
        writer.u16(self.pants_hue);
        writer.into_bytes()
    }
}

// -- 0x1B start -----------------------------------------------------------

/// `0x1B` — put a body in the world. 37 bytes.
///
/// The first packet of the game proper. Until the client has this it has no
/// character and draws nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerStart {
    /// The player's serial.
    pub serial: u32,
    /// The body graphic.
    pub body: u16,
    /// Where.
    pub position: Point,
    /// Which way, and whether running.
    pub facing: Facing,
    /// Map width in tiles.
    pub map_width: u16,
    /// Map height in tiles.
    pub map_height: u16,
}

/// The map size Sphere sends when it has nothing better: Britannia's.
pub const DEFAULT_MAP_WIDTH: u16 = 0x1800;
/// The map size Sphere sends when it has nothing better: Britannia's.
pub const DEFAULT_MAP_HEIGHT: u16 = 0x1000;

impl EncodePacket for PlayerStart {
    const ID: u8 = 0x1B;
    const LENGTH: PacketLength = PacketLength::Fixed(37);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u32(self.serial);
        out.zeros(4);
        out.u16(self.body);
        out.u16(self.position.x);
        out.u16(self.position.y);
        // The z field is two bytes wide but only the low one is read, as a
        // signed byte. Sphere writes a zero and then the byte; writing z as a
        // big-endian i16 would put -10 on the wire as 0xFFF6 and the client
        // would read 0xFF.
        out.u8(0);
        out.u8(self.position.z as u8);
        out.u8(self.facing.to_bits());
        out.zeros(1);
        out.u32(0xFFFF_FFFF);
        out.zeros(4);
        out.u16(self.map_width);
        out.u16(self.map_height);
        out.zeros(6);
    }
}

// -- 0x20 player update ---------------------------------------------------

/// `0x20` — move or redraw the player's own body. 19 bytes.
///
/// Also clears weather on the client, which is why Sphere's comment warns about
/// sending it casually.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerUpdate {
    /// The player's serial.
    pub serial: u32,
    /// The body graphic.
    pub body: u16,
    /// The body hue.
    pub hue: u16,
    /// Status flags: poisoned, invisible, warmode.
    pub flags: u8,
    /// Where.
    pub position: Point,
    /// Which way, and whether running.
    pub facing: Facing,
}

impl EncodePacket for PlayerUpdate {
    const ID: u8 = 0x20;
    const LENGTH: PacketLength = PacketLength::Fixed(19);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u32(self.serial);
        out.u16(self.body);
        out.zeros(1);
        out.u16(self.hue);
        out.u8(self.flags);
        out.u16(self.position.x);
        out.u16(self.position.y);
        out.zeros(2);
        out.u8(self.facing.to_bits());
        out.u8(self.position.z as u8);
    }
}

// -- 0x2C death status ----------------------------------------------------

/// `0x2C` — tell a client its own character just died, or came back. 2 bytes.
///
/// A death byte of `0` puts the client into ghost mode: it greys the world and
/// switches to the gliding ghost walk. `2` is the "alive again" answer that
/// resurrection sends to lift it. ServUO's `DeathStatus` — the one packet that
/// makes the whole screen read as death, so a ghost body drawn without it looks
/// merely like a recoloured player.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeathStatus {
    /// Whether the character is dead.
    pub dead: bool,
}

impl EncodePacket for DeathStatus {
    const ID: u8 = 0x2C;
    const LENGTH: PacketLength = PacketLength::Fixed(2);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u8(if self.dead { 0 } else { 2 });
    }
}

// -- 0x02 walk request ----------------------------------------------------

/// `0x02` — the client asks to take one step. 7 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WalkRequest {
    /// Which way, and whether running.
    pub facing: Facing,
    /// The client's sequence number for this step. See `openshard-movement`.
    pub sequence: u8,
    /// The fastwalk key.
    ///
    /// Dead weight. It was a 1999 attempt to stop speed hacks, was broken
    /// immediately, and Sphere stopped reading it. Kept here only because the
    /// four bytes are on the wire.
    pub fastwalk_key: u32,
}

impl DecodePacket for WalkRequest {
    const ID: u8 = 0x02;

    fn decode_body(
        reader: &mut PacketReader<'_>,
        _version: ClientVersion,
    ) -> Result<Self, DecodeError> {
        Ok(Self {
            facing: Facing::from_bits(reader.u8()?),
            sequence: reader.u8()?,
            fastwalk_key: reader.u32()?,
        })
    }
}

impl WalkRequest {
    /// Encode a whole 0x02 packet. Test fixtures only — this server never sends
    /// one, only ever decodes it.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = PacketWriter::with_capacity(7);
        writer.u8(Self::ID);
        writer.u8(self.facing.to_bits());
        writer.u8(self.sequence);
        writer.u32(self.fastwalk_key);
        writer.into_bytes()
    }
}

/// `0x22` — the step is allowed. 3 bytes.
///
/// `notoriety` colours the player's own health bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WalkAck {
    /// The sequence number being acknowledged.
    pub sequence: u8,
    /// Colours the player's own health bar.
    pub notoriety: u8,
}

impl EncodePacket for WalkAck {
    const ID: u8 = 0x22;
    const LENGTH: PacketLength = PacketLength::Fixed(3);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u8(self.sequence);
        out.u8(self.notoriety);
    }
}

/// `0x21` — the step is refused; here is where you really are. 8 bytes.
///
/// The client snaps back to this position and resets its sequence to zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WalkReject {
    /// The sequence number being refused.
    pub sequence: u8,
    /// Where the client really is.
    pub position: Point,
    /// Which way it is really facing.
    pub facing: Facing,
}

impl EncodePacket for WalkReject {
    const ID: u8 = 0x21;
    const LENGTH: PacketLength = PacketLength::Fixed(8);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u8(self.sequence);
        out.u16(self.position.x);
        out.u16(self.position.y);
        out.u8(self.facing.to_bits());
        out.u8(self.position.z as u8);
    }
}

// -- the rest of the entry sequence ---------------------------------------

/// `0x55` — the client may start drawing. 1 byte.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LoginComplete;

impl EncodePacket for LoginComplete {
    const ID: u8 = 0x55;
    const LENGTH: PacketLength = PacketLength::Fixed(1);

    fn encode_body(&self, _out: &mut PacketWriter, _version: ClientVersion) {}
}

/// `0x4F` — overall light level. 2 bytes.
///
/// 0 is blinding daylight and 0x1F is pitch dark. Backwards from what the name
/// suggests, and the client clamps rather than complaining.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LightLevel {
    /// 0 (blinding daylight) to 0x1F (pitch dark).
    pub level: u8,
}

impl EncodePacket for LightLevel {
    const ID: u8 = 0x4F;
    const LENGTH: PacketLength = PacketLength::Fixed(2);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u8(self.level);
    }
}

/// `0x6D` — play a music track. 3 bytes.
///
/// The id indexes the client's own music list (ServUO's `MusicName` enum order,
/// `Server/Region.cs`), so no filename travels — the client owns the tracks. Both
/// references agree byte for byte: Sphere's `PacketPlayMusic`, ServUO's
/// `PlayMusic`. Sent when a mobile crosses into a region whose track differs from
/// the one it was hearing; re-sending the same id restarts the track, which is
/// why the crossing pass compares before it sends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayMusic {
    /// Indexes the client's own music list.
    pub track: u16,
}

impl EncodePacket for PlayMusic {
    const ID: u8 = 0x6D;
    const LENGTH: PacketLength = PacketLength::Fixed(3);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u16(self.track);
    }
}

/// `0xBC` — which season the client draws. 3 bytes.
///
/// `0` spring, `1` summer, `2` fall, `3` winter, `4` desolation. The second byte
/// asks the client to play the season's own sound as it changes; sending it on
/// world entry with the sound off avoids announcing a change that is really just
/// a login. Ported from ServUO's `SeasonChange`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Season {
    /// `0` spring, `1` summer, `2` fall, `3` winter, `4` desolation.
    pub season: u8,
    /// Whether to play the season's own change sound.
    pub play_sound: bool,
}

impl EncodePacket for Season {
    const ID: u8 = 0xBC;
    const LENGTH: PacketLength = PacketLength::Fixed(3);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u8(self.season);
        out.bool(self.play_sound);
    }
}

/// `0xD1` — the logout the client asked for is granted. 2 bytes.
///
/// The client's own `0xD1` is a *notification*: it announces that the player
/// pressed "Log Out" on the paperdoll and then waits to be told it may go. Both
/// references answer with this same two-byte packet and nothing else — Sphere's
/// `PacketLogout::onReceive` constructs a `PacketLogoutAck`, ServUO's `LogoutReq`
/// sends a `LogoutAck` — and a server that stays silent leaves the client sitting
/// on the "logging out" screen until it times out, with nothing in any log to say
/// why.
///
/// The `0x01` is the accept. Refusing (a `0x00`, "you are in combat") is a rule
/// this shard does not have: the disconnect path already saves whatever state the
/// character is in, so there is nothing to protect by holding a player hostage.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LogoutAck;

impl EncodePacket for LogoutAck {
    const ID: u8 = 0xD1;
    const LENGTH: PacketLength = PacketLength::Fixed(2);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u8(0x01);
    }
}

/// `0xBF` subcommand 0x08 — which map the client should draw. 6 bytes.
///
/// Without this the client draws Felucca whatever the server thinks.
///
/// # Fixed despite living under `0xBF`, and the length field is still hand-written
///
/// Every other `0xBF` packet this crate has seen so far is either genuinely
/// variable, or — like the `0xBF 0x19` stat-lock packet — fixed at a size the
/// `0xBF` envelope itself does not describe. This subcommand never carries a
/// list or a version-conditional tail, so its total size never moves: id,
/// length, subcommand, one map byte, six bytes always. `Fixed(6)` says that
/// directly, and is simpler than `Variable` for a body that never varies.
///
/// One consequence of choosing `Fixed`: [`crate::packet::frame_body`] only
/// back-patches a length field for [`PacketLength::Variable`], so this body
/// still writes its own `u16(6)` literal, exactly where `0xBF`'s general
/// envelope always puts one. It is a fixed constant here, not a length
/// [`frame_body`] computes — the two must simply agree, and a debug assert on
/// the body's total size (built into every `Fixed` payload) is what would
/// catch them drifting apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MapChange {
    /// Which map (facet) to draw.
    pub map: u8,
}

impl EncodePacket for MapChange {
    const ID: u8 = 0xBF;
    const LENGTH: PacketLength = PacketLength::Fixed(6);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u16(6); // this subcommand's own, constant length
        out.u16(0x08);
        out.u8(self.map);
    }
}

/// `0x76` — the client has changed facet: where it now stands, and how big the
/// new world is. 16 bytes.
///
/// This is the packet a *facet change* needs and login does not. `0x1B` carries
/// the map size too, but it is the "you are entering the world" packet and
/// re-sending it mid-session restarts the session; ServUO's `Mobile.Map` setter
/// sends this instead, after the `0xBF 0x08` that says which map to draw.
///
/// Both references define it identically — ServUO's `ServerChange` and Sphere's
/// `PacketZoneChange` are the same sixteen bytes in the same order. They differ
/// only in that Sphere never sends it, its resync being `0xBF 0x08` and a
/// redraw; ServUO's is the one that actually changes maps at runtime, so this
/// follows ServUO.
///
/// The three zeroed fields after `z` are unused in every client that reads it.
#[must_use]
pub fn encode_server_change(at: Point, width: u16, height: u16) -> Vec<u8> {
    let mut writer = PacketWriter::with_capacity(16);
    writer.u8(0x76);
    writer.u16(at.x);
    writer.u16(at.y);
    // Sign-extended, as ServUO's `(short)m.Z` is: a dungeon floor is negative,
    // and a zero-extended one puts the player 65,000 tiles in the air.
    writer.u16(i16::from(at.z) as u16);
    writer.zeros(5);
    writer.u16(width);
    writer.u16(height);
    debug_assert_eq!(writer.len(), 16);
    writer.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::packet::{client_packet_length, decode_packet, encode_packet};

    fn version() -> ClientVersion {
        ClientVersion::new(7, 0, 45, 65)
    }

    fn facing() -> Facing {
        Facing::running(Direction::SouthEast)
    }

    #[test]
    fn character_play_round_trips_at_the_declared_length() {
        let play = CharacterPlay {
            name: "Lord British".to_owned(),
            slot: 0,
            client_ip: 0x0A00_0001,
        };
        let bytes = play.encode();
        assert_eq!(
            client_packet_length(CharacterPlay::ID, None),
            Some(PacketLength::Fixed(73))
        );
        assert_eq!(bytes.len(), 73, "the table and the encoder must agree");
        assert_eq!(
            decode_packet::<CharacterPlay>(&bytes, version()).unwrap(),
            play
        );
    }

    #[test]
    fn character_play_rejects_a_truncated_packet() {
        assert!(decode_packet::<CharacterPlay>(&[0x5D, 0x00], version()).is_err());
    }

    fn sample_create(high_seas: bool) -> CreateCharacter {
        let mut skills = vec![
            SkillChoice {
                skill: 1,
                value: 50,
            },
            SkillChoice {
                skill: 2,
                value: 30,
            },
            SkillChoice {
                skill: 3,
                value: 20,
            },
        ];
        if high_seas {
            skills.push(SkillChoice { skill: 4, value: 0 });
        }
        CreateCharacter {
            name: RawCharacterName("Lord British".to_owned()),
            flags: 0x0000_001F,
            profession: 1,
            sex_race: 0x3, // human female
            strength: 60,
            dexterity: 20,
            intelligence: 20,
            skills,
            skin_hue: 0x83EA,
            hair: 0x203B,
            hair_hue: 0x044E,
            beard: 0,
            beard_hue: 0,
            start_location: 0,
            slot: 0,
            shirt_hue: 0x0386,
            pants_hue: 0x01BB,
        }
    }

    #[test]
    fn create_character_high_seas_round_trips_at_its_declared_length() {
        let create = sample_create(true);
        let bytes = create.encode();
        assert_eq!(bytes[0], CreateCharacter::ID_HIGH_SEAS);
        assert_eq!(bytes.len(), 106, "the 0xF8 form is 106 bytes, four skills");
        assert_eq!(
            client_packet_length(CreateCharacter::ID_HIGH_SEAS, None),
            Some(PacketLength::Fixed(106)),
            "the table and the encoder must agree"
        );
        assert_eq!(CreateCharacter::decode(&bytes).unwrap(), create);
    }

    #[test]
    fn create_character_classic_round_trips_at_its_declared_length() {
        let create = sample_create(false);
        let bytes = create.encode();
        assert_eq!(bytes[0], CreateCharacter::ID_CLASSIC);
        assert_eq!(bytes.len(), 104, "the 0x00 form is 104 bytes, three skills");
        assert_eq!(
            client_packet_length(CreateCharacter::ID_CLASSIC, None),
            Some(PacketLength::Fixed(104))
        );
        assert_eq!(CreateCharacter::decode(&bytes).unwrap(), create);
    }

    #[test]
    fn create_character_reads_the_name_and_skills_at_the_right_offsets() {
        // The whole risk in a fixed-layout packet is a field one byte out of
        // place, which shifts everything after it. Pin the name and the skills.
        let decoded = CreateCharacter::decode(&sample_create(true).encode()).unwrap();
        assert_eq!(decoded.name, "Lord British");
        assert_eq!(decoded.skin_hue, 0x83EA);
        assert_eq!(decoded.skills.len(), 4);
        assert_eq!(
            decoded.skills[0],
            SkillChoice {
                skill: 1,
                value: 50
            }
        );
        assert_eq!(decoded.start_location, 0);
    }

    #[test]
    fn create_character_maps_race_and_sex_to_a_body() {
        let human_female = CreateCharacter {
            sex_race: 0x3,
            ..sample_create(true)
        };
        assert!(human_female.is_female());
        assert_eq!(human_female.race(), Race::Human);
        assert_eq!(human_female.body(), 0x0191);

        let elf_male = CreateCharacter {
            sex_race: 0x4,
            ..sample_create(true)
        };
        assert!(!elf_male.is_female());
        assert_eq!(elf_male.race(), Race::Elf);
        assert_eq!(elf_male.body(), 0x025D);

        let gargoyle_female = CreateCharacter {
            sex_race: 0x7,
            ..sample_create(true)
        };
        assert!(gargoyle_female.is_female());
        assert_eq!(gargoyle_female.race(), Race::Gargoyle);
        assert_eq!(gargoyle_female.body(), 0x029B);
    }

    #[test]
    fn create_character_rejects_a_truncated_packet() {
        assert!(CreateCharacter::decode(&[CreateCharacter::ID_HIGH_SEAS, 0x00]).is_err());
    }

    #[test]
    fn create_character_rejects_the_wrong_id() {
        let mut bytes = sample_create(true).encode();
        bytes[0] = 0x5D;
        assert!(matches!(
            CreateCharacter::decode(&bytes),
            Err(DecodeError::WrongPacket(_))
        ));
    }

    #[test]
    fn player_start_matches_its_declared_length() {
        let start = PlayerStart {
            serial: 0x0000_0001,
            body: 0x0190,
            position: Point::new(1475, 1774, 0),
            facing: facing(),
            map_width: DEFAULT_MAP_WIDTH,
            map_height: DEFAULT_MAP_HEIGHT,
        };
        let bytes = encode_packet(&start, version());
        assert_eq!(bytes.len(), 37, "Sphere's PacketPlayerStart length");
        assert_eq!(bytes[0], 0x1B);
        assert_eq!(&bytes[1..5], &1u32.to_be_bytes());
        assert_eq!(&bytes[9..11], &0x0190u16.to_be_bytes(), "body");
        assert_eq!(&bytes[11..13], &1475u16.to_be_bytes(), "x");
        assert_eq!(&bytes[13..15], &1774u16.to_be_bytes(), "y");
        assert_eq!(bytes[17], facing().to_bits());
        assert_eq!(&bytes[19..23], &[0xFF; 4], "the 0xFFFFFFFF Sphere sends");
    }

    #[test]
    fn a_negative_z_survives_the_two_byte_field() {
        // The z field is two bytes but only the low one is read, as a signed
        // byte. Writing z as a big-endian i16 would put -10 on the wire as
        // 0xFFF6, and the client would take 0xFF — a height of -1.
        let start = PlayerStart {
            serial: 1,
            body: 0x0190,
            position: Point::new(100, 100, -10),
            facing: facing(),
            map_width: DEFAULT_MAP_WIDTH,
            map_height: DEFAULT_MAP_HEIGHT,
        };
        let bytes = encode_packet(&start, version());
        assert_eq!(bytes[15], 0x00, "the high byte is padding, not sign");
        assert_eq!(bytes[16] as i8, -10, "the low byte carries the height");
    }

    #[test]
    fn player_update_matches_its_declared_length() {
        let update = PlayerUpdate {
            serial: 1,
            body: 0x0190,
            hue: 0x83EA,
            flags: 0,
            position: Point::new(1475, 1774, -5),
            facing: facing(),
        };
        let bytes = encode_packet(&update, version());
        assert_eq!(bytes.len(), 19, "Sphere's PacketPlayerUpdate length");
        assert_eq!(bytes[0], 0x20);
        assert_eq!(&bytes[8..10], &0x83EAu16.to_be_bytes(), "hue");
        assert_eq!(bytes[17], facing().to_bits());
        assert_eq!(bytes[18] as i8, -5, "z is one signed byte here");
    }

    #[test]
    fn death_status_is_two_bytes_dead_is_zero() {
        let dead = encode_packet(&DeathStatus { dead: true }, version());
        assert_eq!(dead, vec![0x2C, 0x00], "0 puts the client in ghost mode");
        let alive = encode_packet(&DeathStatus { dead: false }, version());
        assert_eq!(alive, vec![0x2C, 0x02], "2 is the alive-again answer");
    }

    #[test]
    fn walk_request_round_trips_at_the_declared_length() {
        let request = WalkRequest {
            facing: facing(),
            sequence: 42,
            fastwalk_key: 0xDEAD_BEEF,
        };
        let bytes = request.encode();
        assert_eq!(
            client_packet_length(WalkRequest::ID, None),
            Some(PacketLength::Fixed(7))
        );
        assert_eq!(bytes.len(), 7);
        assert_eq!(
            decode_packet::<WalkRequest>(&bytes, version()).unwrap(),
            request
        );
    }

    #[test]
    fn walk_request_keeps_the_running_bit_out_of_the_direction() {
        let bytes = WalkRequest {
            facing: Facing::running(Direction::North),
            sequence: 0,
            fastwalk_key: 0,
        }
        .encode();
        assert_eq!(bytes[1], 0x80, "north, running");

        let decoded = decode_packet::<WalkRequest>(&bytes, version()).unwrap();
        assert_eq!(decoded.facing.direction, Direction::North);
        assert!(decoded.facing.running);
    }

    #[test]
    fn walk_ack_and_reject_match_their_declared_lengths() {
        assert_eq!(
            encode_packet(
                &WalkAck {
                    sequence: 7,
                    notoriety: 0x01
                },
                version()
            ),
            vec![0x22, 7, 0x01]
        );

        let reject = encode_packet(
            &WalkReject {
                sequence: 7,
                position: Point::new(1475, 1774, -5),
                facing: facing(),
            },
            version(),
        );
        assert_eq!(reject.len(), 8, "Sphere's PacketMovementRej length");
        assert_eq!(reject[0], 0x21);
        assert_eq!(reject[1], 7, "the sequence being rejected");
        assert_eq!(&reject[2..4], &1475u16.to_be_bytes());
        assert_eq!(&reject[4..6], &1774u16.to_be_bytes());
        assert_eq!(reject[6], facing().to_bits());
        assert_eq!(reject[7] as i8, -5);
    }

    #[test]
    fn the_small_entry_packets_are_the_right_shape() {
        assert_eq!(encode_packet(&LoginComplete, version()), vec![0x55]);
        assert_eq!(
            encode_packet(&LightLevel { level: 0 }, version()),
            vec![0x4F, 0]
        );
        // Music and season: three bytes each, the track big-endian. Both
        // references write exactly this.
        assert_eq!(
            encode_packet(&PlayMusic { track: 11 }, version()),
            vec![0x6D, 0x00, 11]
        );
        assert_eq!(
            encode_packet(&PlayMusic { track: 0x0102 }, version()),
            vec![0x6D, 0x01, 0x02]
        );
        assert_eq!(
            encode_packet(
                &Season {
                    season: 3,
                    play_sound: true
                },
                version()
            ),
            vec![0xBC, 3, 1]
        );
        assert_eq!(
            encode_packet(
                &Season {
                    season: 0,
                    play_sound: false
                },
                version()
            ),
            vec![0xBC, 0, 0]
        );
        // The logout ack is the same two bytes in both references, and the same
        // length the client's own table gives the id it comes back on.
        assert_eq!(encode_packet(&LogoutAck, version()), vec![0xD1, 0x01]);
        assert_eq!(
            crate::packet::client_packet_length(0xD1, None),
            Some(crate::packet::PacketLength::Fixed(2))
        );

        // 0xBF is variable-length on the client's own table, but this
        // subcommand's own body never varies, so it declares its own length at
        // offset 1 the same way every other fixed packet does.
        let map = encode_packet(&MapChange { map: 1 }, version());
        assert_eq!(map.len(), 6);
        assert_eq!(map[0], 0xBF);
        assert_eq!(
            u16::from_be_bytes([map[1], map[2]]),
            6,
            "declares its length"
        );
        assert_eq!(u16::from_be_bytes([map[3], map[4]]), 0x08, "subcommand");
        assert_eq!(map[5], 1, "Trammel");
    }

    /// The facet-change packet, byte for byte.
    ///
    /// ServUO's `ServerChange` and Sphere's `PacketZoneChange` agree exactly on
    /// this layout, which is as close to a specification as this genre gets, so
    /// it is worth pinning rather than trusting a reading of either.
    #[test]
    fn the_server_change_says_where_and_how_big() {
        let packet = encode_server_change(Point::new(1495, 1629, -20), 2304, 1600);

        assert_eq!(packet.len(), 16, "fixed at sixteen bytes");
        assert_eq!(packet[0], 0x76);
        assert_eq!(u16::from_be_bytes([packet[1], packet[2]]), 1495, "x");
        assert_eq!(u16::from_be_bytes([packet[3], packet[4]]), 1629, "y");
        assert_eq!(
            i16::from_be_bytes([packet[5], packet[6]]),
            -20,
            "z is signed — a dungeon floor is below zero"
        );
        assert_eq!(&packet[7..12], &[0; 5], "three unused fields");
        assert_eq!(
            u16::from_be_bytes([packet[12], packet[13]]),
            2304,
            "Ilshenar's width, not Britannia's"
        );
        assert_eq!(u16::from_be_bytes([packet[14], packet[15]]), 1600, "height");
    }

    #[test]
    fn a_point_at_the_edges_of_its_fields_encodes() {
        // z is the one that can go negative, and the map is 24 bits wide in
        // neither axis — u16 is the whole range the client has.
        let start = PlayerStart {
            serial: u32::MAX,
            body: u16::MAX,
            position: Point::new(u16::MAX, u16::MAX, i8::MIN),
            facing: Facing::walking(Direction::NorthWest),
            map_width: u16::MAX,
            map_height: u16::MAX,
        };
        assert_eq!(encode_packet(&start, version()).len(), 37);

        let update = PlayerUpdate {
            serial: u32::MAX,
            body: u16::MAX,
            hue: u16::MAX,
            flags: u8::MAX,
            position: Point::new(u16::MAX, u16::MAX, i8::MAX),
            facing: Facing::walking(Direction::NorthWest),
        };
        assert_eq!(encode_packet(&update, version()).len(), 19);
    }
}
