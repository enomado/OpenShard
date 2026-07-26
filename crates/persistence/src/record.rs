//! What outlives the process.
//!
//! # These are not components
//!
//! A [`CharacterRecord`] looks like `Position` plus `Name` plus `Body` flattened
//! into one struct, and the temptation is to serialise the components directly
//! and delete this file.
//!
//! The reason not to is that the two change for different reasons. A component
//! changes whenever the simulation wants a better shape — split `Body` in two,
//! move `Heading` into `Position`, add a field the tick needs — and none of that
//! should reach into a database that already has a million rows in it. A record
//! changes only when the *saved* meaning changes, which is rare and deliberate,
//! and when it does it comes with [`SCHEMA_VERSION`] and a migration.
//!
//! The conversion between them is the seam where that difference is absorbed.
//! Serialising components directly deletes the seam and welds the simulation's
//! internal shape to the on-disk format forever.

use serde::{Deserialize, Serialize};

/// The version of the saved shape.
///
/// Bumped when a record changes meaning, not when the simulation is refactored
/// around it. A store that opens a save from the future must refuse rather than
/// guess: reading a newer save with older code is how a shard silently drops
/// every field it does not recognise and then writes the loss back.
///
/// - v1: characters only.
/// - v2: items — a character's carried inventory, and loose things on the ground.
/// - v3: an item's `stackable` flag.
/// - v4: spawn regions and their respawn timers.
/// - v5: the whole world — NPC mobiles (with gear and vendor stock), decoration
///   (with door state), and an item's `price`/`name`.
/// - v6: a character's stats and skills (with their lock arrows).
/// - v7: a mobile's active effects — poison today, buffs and debuffs as they
///   land — so a relog cannot wash a debuff off, the way ServUO keeps a
///   logged-out mobile's timers running and Sphere saves its effect tags.
/// - v8: a spellbook's learned-spell bitmask, so a bought book still opens to
///   its spells after a relog.
/// - v9: a character's `dead` flag, so a player who logged out a ghost logs back
///   in a ghost — the ghost body is re-derived, but the fact of death is saved.
/// - v10: an account's credential is an argon2 hash, not plaintext. A mismatched
///   older database is recreated (the account rows re-seed from config, hashed),
///   which is the convention every bump above shares.
/// - v11: a character's quest log — an opaque JSON blob the community pack owns
///   and the engine only stores, so quest progress survives a relog. Empty for a
///   character with no quests, and old saves default it, so the bump is additive.
/// - v12: a facet's named regions, and the world clock. Both are things a restart
///   would otherwise lose to a shard that looks fine: no guards, no town music,
///   daylight in every dungeon, and every night starting over at boot.
/// - v13: quests, structurally. The v11 blob is **gone**, not migrated: it held a
///   format only the script pack understood, and the quest system that understands
///   it now lives in the engine, so there is nothing to translate it into that
///   would not be a guess. A shard upgrading past v13 loses quest progress and
///   keeps everything else, which is the recreate-on-mismatch convention every
///   bump above shares. What replaces it: a character's quests and their
///   cooldowns, and, on a mobile, whether it gives quests or can be escorted —
///   the last two being why quest givers went inert after every restart.
/// - v14: a townsperson's **trade** (`MobileRecord::title`) and, with the optional
///   routine, where it sleeps (`night_home`). The trade is the key its outfit, its
///   generated name and — every time anyone speaks near it — its keyword table are
///   all looked up by, so an NPC restored without it is a mute statue that a save
///   file cannot tell apart from a working one. This is the `quest_giver` lesson
///   applied before it could bite: a binding that lives only in the spawn call that
///   placed it is lost at the first restart, silently. Added with `serde(default)`,
///   so an older save reads as a town of trade-less NPCs rather than failing. It
///   also carries a vendor's **full shelf** (`restock`), for the same reason: the
///   crate's live contents are what is *left*, so a restock timer with nothing to
///   compare them against would forget what full meant at every reboot.
/// - v15: standing — a character's **fame**, **karma** and **murder count**. The first
///   two are new (ServUO's `Titles`); the third was a bug, not an addition: the count
///   that makes a repeat killer permanently red lived only in memory, so every restart
///   washed every murderer blue while the decay clock and the notoriety rule around it
///   were both already correct. Creature fame/karma rides the JSON `MobileRecord` and
///   `CreatureData` with no column of its own.
/// - v16: a character's **skill caps** and its three **stat arrows**, together
///   with when each stat last rose. All three are inputs the gain reads every
///   time it fires: a per-skill cap decides how much headroom is left, a "down"
///   arrow decides which skill gives ground at the total cap, and a stat arrow
///   decides whether a stat may rise at all. Kept only in memory they reset at
///   every restart, which reads as the arrows the player set quietly snapping
///   back to "up" — the `Murders` lesson from v15, caught before it shipped.
pub const SCHEMA_VERSION: u32 = 16;

/// An account, as saved.
///
/// # The credential is a hash, not a password
///
/// [`credential`](Self::credential) is an argon2 PHC string
/// (`$argon2id$v=19$...`), never the plaintext. The UO login sends the password
/// in the clear and that cannot be fixed; what is fixed is that it is hashed
/// before it reaches here and the plaintext is dropped. See
/// `openshard_login::password`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AccountRecord {
    /// The login name. Unique; this is the key.
    pub name: String,
    /// The credential — an argon2 PHC hash of the password.
    pub credential: String,
}

/// A character, as saved.
///
/// # Why the serial is in here
///
/// A serial is not an implementation detail the server may re-pick on load. It
/// is on the wire, in every packet a client has ever been sent, and — once there
/// are items — it is what a container's contents point at. A character that
/// comes back with a different serial is a different character with the same
/// name, and everything that referred to the old one now refers to nothing.
///
/// So it is saved, and [`openshard_entities::Registry::bind_serial`] reserves it
/// on the way back in rather than handing it out again to someone else.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CharacterRecord {
    /// The wire serial. Stable across restarts; see the type docs.
    pub serial: u32,
    /// Which account it belongs to.
    pub account: String,
    /// The character's name.
    pub name: String,
    /// The body graphic.
    pub body: u16,
    /// The body hue.
    pub hue: u16,
    /// Which facet.
    pub facet: u8,
    /// Where it stands.
    pub x: u16,
    /// Where it stands.
    pub y: u16,
    /// How high it stands. Signed: UO has basements.
    pub z: i8,
    /// Which way it faces, as the wire byte.
    pub facing: u8,
    /// Strength — caps hit points.
    #[serde(default = "default_stat")]
    pub strength: u16,
    /// Dexterity — the stamina pool and swing pace.
    #[serde(default = "default_stat")]
    pub dexterity: u16,
    /// Intelligence — caps mana.
    #[serde(default = "default_stat")]
    pub intelligence: u16,
    /// Every trained skill, as `(id, value in tenths, lock byte, cap)`. Empty for
    /// a character that has none yet.
    #[serde(default)]
    pub skills: Vec<SkillRecord>,
    /// Which way the three stats are set to train, and how long since each rose.
    #[serde(default)]
    pub stat_locks: StatLockRecord,
    /// Every timed effect working through it — poison, buffs, debuffs — so a
    /// relog cannot wash them off. Empty for a clean character.
    #[serde(default)]
    pub effects: Vec<EffectRecord>,
    /// Whether it logged out dead. A ghost that relogs comes back a ghost — the
    /// grey body and death shroud are re-derived on login; only the fact of death
    /// rides here. The `body`/`hue` above stay the *living* ones, so resurrection
    /// restores the character exactly. `false` for the living, the common case.
    #[serde(default)]
    pub dead: bool,
    /// How widely known the character is — ServUO's `Mobile.Fame`.
    #[serde(default)]
    pub fame: i32,
    /// Which way it is known — ServUO's `Mobile.Karma`.
    #[serde(default)]
    pub karma: i32,
    /// How many innocents this character has killed.
    ///
    /// Saved because it is a **standing**, and it was not: the fifth murder makes a
    /// character a murderer for good, and a count that lives only in memory washed
    /// every red blue at the next restart. Everything else about the flag — the decay
    /// clock, the notoriety it forces — was already right; the number underneath it
    /// simply went missing at the door.
    #[serde(default)]
    pub murders: u16,
    /// Every quest in progress, with how far each objective has got.
    #[serde(default)]
    pub quests: Vec<QuestRecord>,
    /// Every quest already finished, with the cooldown before it may be taken
    /// again. Kept separately from [`quests`](Self::quests) because a finished
    /// quest has no progress left to save — only a date.
    #[serde(default)]
    pub done_quests: Vec<DoneQuestRecord>,
}

/// A quest in progress, as saved.
///
/// `progress` and `seconds` run parallel to the *definition's* objective list, so
/// this is only meaningful next to the pack's definition of `key`. That is the
/// same bargain ServUO's own save makes (it matches objectives positionally
/// against a freshly constructed quest), and it means adding an objective to the
/// end of an existing quest is safe while reordering is not.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct QuestRecord {
    /// Which quest, by the pack's key. A key the pack no longer defines is
    /// dropped on load rather than failing the character.
    pub key: String,
    /// How far each objective has got.
    pub progress: Vec<u16>,
    /// Seconds left on each timed objective; `0` on the untimed ones. A *remaining
    /// span*, not a deadline: the tick counter starts again from zero at boot, so
    /// a saved absolute tick would mean something different every restart — the
    /// same rule [`EffectRecord::remaining`] follows.
    #[serde(default)]
    pub seconds: Vec<u32>,
    /// Whether a timer ran out on it.
    #[serde(default)]
    pub failed: bool,
    /// The serial of the NPC that gave it, if it is still known.
    #[serde(default)]
    pub giver: Option<u32>,
}

/// A finished quest and its cooldown.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DoneQuestRecord {
    /// Which quest, by the pack's key.
    pub key: String,
    /// Seconds until it may be taken again — again a remaining span, not a
    /// deadline. [`u32::MAX`] means never (a once-only quest).
    pub restart_in_secs: u32,
}

/// A timed effect on a mobile that a relog must not wash off — poison today,
/// buffs and debuffs (bless, curse, a stat drain) as they land. Deliberately one
/// shape for all of them, so a new effect kind rides the same list and column
/// with no schema change: a `kind` tag, its `amount` (a poison level, a stat
/// offset), and how much is `remaining` (poison pulses, or a buff's seconds).
/// The remaining *count* is stored, not a tick (which resets to zero at boot) —
/// the effect re-derives its next fire from "now" on restore, the way a
/// spawner's remaining wait does.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EffectRecord {
    /// What kind of effect: `0` poison, and future buffs/debuffs beyond it.
    pub kind: u8,
    /// Its magnitude — a poison level, or a stat offset (signed for a debuff).
    pub amount: i16,
    /// How much it has left — poison pulses, or a timed buff's seconds.
    pub remaining: u16,
}

/// The effect kind for poison — the first, and the pattern the rest follow.
pub const EFFECT_POISON: u8 = 0;

/// One skill a character has, as saved: which, how far trained, and the arrow
/// the player set it to train by.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SkillRecord {
    /// The skill id, zero-based.
    pub id: u8,
    /// The trained value, in tenths.
    pub value: u16,
    /// The lock arrow as its wire byte (0 up, 1 down, 2 locked).
    pub lock: u8,
    /// The ceiling on this skill, in tenths. Defaulted so a pre-v16 save reads as
    /// the ordinary 100.0 rather than as a skill capped at nothing.
    #[serde(default = "default_skill_cap")]
    pub cap: u16,
}

/// The cap a pre-v16 save (which stored none) loads each skill with — the classic
/// 100.0.
fn default_skill_cap() -> u16 {
    1000
}

/// Which way a character's three stats are set to train, and when each last rose.
///
/// Saved together because the gain reads them together, and because all four
/// numbers are worthless apart: an arrow with no timestamp lets a relog pour
/// points in, and a timestamp with no arrow has nothing to gate.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct StatLockRecord {
    /// Strength's arrow as its wire bits (0 up, 1 down, 2 locked).
    pub strength: u8,
    /// Dexterity's arrow.
    pub dexterity: u8,
    /// Intelligence's arrow.
    pub intelligence: u8,
    /// How many ticks ago strength last rose. Stored as an *age* and not as the
    /// absolute tick it happened on, because the tick counter restarts with the
    /// shard: an absolute stamp from the last run would sit in the future of this
    /// one and freeze the stat for ever.
    pub strength_age: u64,
    /// The same for dexterity.
    pub dexterity_age: u64,
    /// The same for intelligence.
    pub intelligence_age: u64,
}

/// The stat a pre-v6 save (which stored none) loads with — the flat hundred the
/// world handed out before stats were persisted.
fn default_stat() -> u16 {
    100
}

/// Where an item is, as saved. An item is in exactly one of three places, the
/// same three the live `Position`/`Contained`/`Equipped` components model — never
/// more than one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ItemLocation {
    /// Loose on the ground, at a world tile on a facet.
    Ground {
        /// Which facet.
        facet: u8,
        /// Where.
        x: u16,
        /// Where.
        y: u16,
        /// How high. Signed: UO has basements.
        z: i8,
    },
    /// Inside a container, by the container's serial and the slot in its gump.
    Contained {
        /// The container it is in, by serial.
        container: u32,
        /// Column in the gump.
        x: u16,
        /// Row in the gump.
        y: u16,
        /// Slot in the grid view.
        grid: u8,
    },
    /// Worn on a mobile, at a layer.
    Equipped {
        /// The wearer's serial.
        mobile: u32,
        /// The equipment layer.
        layer: u8,
    },
}

/// An item, as saved.
///
/// # Why the serial is here, like a character's
///
/// An item's serial is what a container's contents point at and what a worn item
/// is drawn under, so it is stable across restarts for the same reason a
/// character's is: change it and every reference to the old one dangles. It is
/// saved and reserved on the way back in.
///
/// `owner` is the character whose inventory this belongs to, or `0` for a loose
/// ground item that belongs to no one — the key a store replaces a whole
/// inventory by. `container_gump` is `Some` when the item is *itself* a container,
/// carrying the window the client opens for it, so a bag inside a bag comes back
/// openable.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ItemRecord {
    /// The wire serial. Stable across restarts; see the type docs.
    pub serial: u32,
    /// The character whose inventory this is in, or `0` for a ground item.
    pub owner: u32,
    /// The item graphic.
    pub graphic: u16,
    /// The item hue.
    pub hue: u16,
    /// The stack amount; `1` for a single item.
    pub amount: u16,
    /// Whether it stacks — a pile of gold merges with another, a sword does not.
    /// Saved so a restored pile still stacks; without it a lone gold coin would
    /// stop merging until re-lifted.
    pub stackable: bool,
    /// The container gump if this item is itself a container, else `None`.
    pub container_gump: Option<u16>,
    /// What one unit costs at a vendor, if this is priced stock. Defaulted so a
    /// v4 save loads.
    #[serde(default)]
    pub price: Option<u32>,
    /// The item's label, if it carries one — vendor stock names its wares.
    /// Defaulted so a v4 save loads.
    #[serde(default)]
    pub name: Option<String>,
    /// The learned-spell bitmask if this item is a spellbook, else `None`. Saved
    /// so a bought book still opens to its spells after a relog; without it a
    /// restored spellbook is a graphic with no `Spellbook` component and refuses
    /// to open. Defaulted so a pre-v8 save loads.
    #[serde(default)]
    pub spellbook: Option<u64>,
    /// Where it is.
    pub location: ItemLocation,
}

/// One creature kind a spawn region may put down, as saved — a plain mirror of
/// the world's creature template, kept here so the on-disk shape does not move
/// every time the simulation's does.
/// The serde default for [`CreatureData::aggression`]: aggressive.
fn aggressive() -> u8 {
    2
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CreatureData {
    /// The body graphic.
    pub body: u16,
    /// Its hue.
    pub hue: u16,
    /// Starting and maximum hit points.
    pub hits: u16,
    /// Health-bar colour, the notoriety wire value.
    pub notoriety: u8,
    /// Melee damage before resistance.
    pub damage: u16,
    /// Physical resistance, a percentage.
    pub resistance: u8,
    /// How widely known it is — what its killer inherits. Defaulted, so an older
    /// saved region restores creatures that give up no standing.
    #[serde(default)]
    pub fame: i32,
    /// Which way it is known. Negative is evil.
    #[serde(default)]
    pub karma: i32,
    /// Swing cadence in ticks; `0` derives it from dexterity.
    pub swing: u64,
    /// How far it notices a target.
    pub sight: u8,
    /// Whether it starts fights (2), answers them (1), or only runs (0).
    /// Defaults to aggressive, the only behaviour that existed before it.
    #[serde(default = "aggressive")]
    pub aggression: u8,
    /// Ticks between its beats while hunting; 0 takes the shard default.
    #[serde(default)]
    pub beat: u64,
    /// How far its ranged attack reaches, in tiles; 0 fights hand to hand.
    #[serde(default)]
    pub ranged: u8,
    /// The ranged attack's damage type wire value.
    #[serde(default)]
    pub ranged_kind: u8,
    /// Whether it drifts when idle.
    pub wander: bool,
    /// Trained combat skills, `(skill id, value in tenths)`. Defaulted so an older
    /// save (no skills) restores as a skill-less creature.
    #[serde(default)]
    pub skills: Vec<(u8, u16)>,
}

/// A spawn region, as saved.
///
/// # Why the timer is *remaining seconds*, not a wall-clock time
///
/// The requirement is that a rare spawn killed shortly before a restart comes back
/// with the same wait ahead of it — killed with five hours left, five hours left
/// on load, whatever the shard was down for. So the timer is stored as the seconds
/// still to wait, not an absolute time: on load it counts down from there, and
/// downtime does not eat into it. Seconds, not ticks, so it survives the tick
/// counter resetting to zero at boot.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SpawnerRecord {
    /// Its stable id, the key it is replaced by.
    pub id: u32,
    /// Which facet.
    pub facet: u8,
    /// The region's north-west corner and size.
    pub x: u16,
    /// North-west corner y.
    pub y: u16,
    /// Region width.
    pub width: u16,
    /// Region height.
    pub height: u16,
    /// The most live creatures it keeps.
    pub max_count: u16,
    /// The respawn delay, in seconds.
    pub respawn_secs: u64,
    /// Seconds still to wait before the next spawn; `0` is ready now.
    pub remaining_secs: u64,
    /// The creatures it may put down.
    pub creatures: Vec<CreatureData>,
}

/// An NPC mobile, as saved — the townsperson, the vendor, the creature on the
/// ground. The Sphere/ServUO model: every live mobile is persisted, so a restart
/// restores the world exactly (a wounded rare comes back wounded; a killed one
/// stays gone, its region timer counting down). Deliberately a sibling of
/// [`CharacterRecord`], not a variant of it: an NPC has no account, and the two
/// change for different reasons.
///
/// Its worn gear and vendor stock ride the same [`Inventory`]/[`ItemRecord`]
/// machinery a character's do, keyed by this serial.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct MobileRecord {
    /// The wire serial. Stable across restarts, like a character's.
    pub serial: u32,
    /// The body graphic.
    pub body: u16,
    /// The body hue.
    pub hue: u16,
    /// Which facet.
    pub facet: u8,
    /// Where it stands.
    pub x: u16,
    /// Where it stands.
    pub y: u16,
    /// How high it stands. Signed: UO has basements.
    pub z: i8,
    /// Which way it faces, as the wire byte.
    pub facing: u8,
    /// Its name, if it has one — a named townsperson; `None` for a beast.
    pub name: Option<String>,
    /// Hit points as they stood at the save — a wounded creature stays wounded.
    pub hits_current: u16,
    /// Maximum hit points.
    pub hits_max: u16,
    /// Health-bar colour, the notoriety wire value.
    pub notoriety: u8,
    /// Melee damage before resistance.
    pub damage: u16,
    /// Physical resistance, a percentage.
    pub resistance: u8,
    /// Swing cadence in ticks; `0` derives it from dexterity.
    pub swing: u64,
    /// How far it notices a target; `0` never picks a fight (and no brain).
    pub sight: u8,
    /// Whether it starts fights (2), answers them (1), or only runs (0).
    pub aggression: u8,
    /// Ticks between its beats while hunting; 0 takes the shard default.
    pub beat: u64,
    /// How far its ranged attack reaches, in tiles; 0 fights hand to hand.
    pub ranged: u8,
    /// The ranged attack's damage type wire value.
    pub ranged_kind: u8,
    /// Whether it drifts when idle.
    pub wander: bool,
    /// Whether it offers banking.
    pub banker: bool,
    /// Whether it keeps a shop.
    pub vendor: bool,
    /// The trade it plies, ServUO-style ("the blacksmith"). `None` for a creature.
    ///
    /// Saved because it is a *key*, not decoration: the speech table an NPC answers
    /// from is looked up by it on every word spoken nearby. Its generated outfit and
    /// name need it only once, at spawn, and those are already saved as worn items
    /// and a `Name` — this is what keeps it answering after a reboot.
    #[serde(default)]
    pub title: Option<String>,
    /// A townsperson's post `(x, y, z)`, if it keeps one.
    pub npc_home: Option<(u16, u16, i8)>,
    /// Where it sleeps, for the optional daily routine. `None` on every NPC unless
    /// the pack gave it one, and read only when `gameplay.npc_schedule` is on.
    #[serde(default)]
    pub night_home: Option<(u16, u16, i8)>,
    /// A vendor's shelf when full, and the seconds until it next refills.
    ///
    /// The seconds and not a tick count, for the reason `SpawnerRecord` states: a
    /// tick counter restarts at boot, so a saved tick would come back either already
    /// due or an hour early.
    #[serde(default)]
    pub restock: Option<RestockRecord>,
    /// How far from its post a townsperson drifts; meaningful with `npc_home`.
    pub npc_wander: u8,
    /// The spawn region that maintains it, if one does — restored so the region
    /// counts it and does not spawn over it.
    pub spawned_by: Option<u32>,
    /// Every timed effect working through it — poison and the rest.
    #[serde(default)]
    pub effects: Vec<EffectRecord>,
    /// Trained combat skills, `(skill id, value in tenths)`. Defaulted so an older
    /// save restores a skill-less creature.
    #[serde(default)]
    pub skills: Vec<(u8, u16)>,
    /// The quests this NPC offers, by key. Empty for an ordinary mobile.
    ///
    /// Saved because the binding has nowhere else to live that survives: the
    /// script that placed the NPC only knows it is a giver during the run that
    /// placed it, so before this the shard's quests worked exactly once — on the
    /// boot where the world was populated — and every restart afterwards left a
    /// town full of NPCs that answered nothing, with no error anywhere to say so.
    #[serde(default)]
    pub quest_giver: Vec<String>,
    /// The region this NPC asks to be escorted to, if it is escortable. `None` for
    /// an ordinary mobile; an empty string means "wherever the quest decides",
    /// chosen when someone accepts.
    #[serde(default)]
    pub escort_destination: Option<String>,
}

/// A vendor's shelf when full, as saved. See [`MobileRecord::restock`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RestockRecord {
    /// How many seconds until the shelf next refills.
    pub in_seconds: u64,
    /// The full shelf: `(graphic, hue, amount, price, name)` per line.
    pub lines: Vec<(u16, u16, u16, u32, String)>,
}

/// A shut-and-openable door's live state, inside a [`DecorationRecord`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DoorState {
    /// The graphic it shows shut.
    pub closed_graphic: u16,
    /// The graphic it shows open.
    pub open_graphic: u16,
    /// How far the leaf swings east-west when opened.
    pub offset_x: i16,
    /// How far the leaf swings north-south when opened.
    pub offset_y: i16,
    /// Whether it stood open at the save — a door left open stays open.
    pub is_open: bool,
}

/// A placed decoration, as saved — the statics, doors and town containers a pack
/// lays over the map's art. Saved like everything else in the world; the pack is
/// the *seed* (a staff Populate/Decorate, once), the save is the truth thereafter.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DecorationRecord {
    /// The wire serial. Stable across restarts.
    pub serial: u32,
    /// The graphic as it stands now (a door's current leaf).
    pub graphic: u16,
    /// Its hue.
    pub hue: u16,
    /// Which facet.
    pub facet: u8,
    /// Where it stands.
    pub x: u16,
    /// Where it stands.
    pub y: u16,
    /// How high. Signed: UO has basements.
    pub z: i8,
    /// Door state, if this decoration is a door.
    pub door: Option<DoorState>,
    /// The container gump if this decoration opens as one, else `None`.
    pub container_gump: Option<u16>,
    /// Which key opens it; `0` is unlocked. On the record rather than inside
    /// [`DoorState`] because a container locks too, and a lock is the same thing on
    /// either — ServUO's `ILockable`. Defaulted, so an older save reads as unlocked.
    #[serde(default)]
    pub key_value: u32,
}

/// One named area of a facet, as saved.
///
/// Regions are pure data the pack registers, so saving them looks redundant —
/// until a restart, when nothing re-registers them until a game master clicks
/// `.admin` again, and a shard silently loses its guards, its music and the dark
/// in its dungeons. The save is the truth here as everywhere else.
///
/// The rectangles ride as JSON for the same reason a spawner's creature list
/// does: a region holds a handful, and a table of them would be a join for no
/// gain.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RegionRecord {
    /// Which facet it belongs to.
    pub facet: u8,
    /// Its index on that facet, which is its id.
    pub id: u16,
    /// What the place is called.
    pub name: String,
    /// Which region wins where two overlap.
    pub priority: u8,
    /// Its boxes: `(x, y, width, height, z_min, z_max)`.
    pub rects: Vec<(u16, u16, u16, u16, i8, i8)>,
    /// Guards answer a call here.
    pub guarded: bool,
    /// No teleporting in, out or within.
    pub no_teleport: bool,
    /// No Recall or Gate.
    pub no_recall: bool,
    /// No house may be placed.
    pub no_housing: bool,
    /// No player may harm another.
    pub safe: bool,
    /// The client music track, as a `MusicName` index.
    pub music: Option<u16>,
    /// The light level inside, overriding the hour.
    pub light: Option<u8>,
}

/// The world's own scalars, as saved — one row, not one per anything.
///
/// Just the clock today: the UO minute the shard stopped at, so a restart does
/// not put the world back at midnight and make every night a fresh one. The tick
/// counter cannot carry it, since that resets at boot by design (every restored
/// timer is an offset from zero).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct WorldRecord {
    /// The world clock, in UO minutes.
    pub clock_minutes: u64,
}

/// A character's whole carried inventory, replaced as a unit.
///
/// A store saves a character's items by replacing everything under its `owner`
/// rather than tracking each item's comings and goings — see
/// [`crate::journal`]. `items` is every worn and contained item, at every depth.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Inventory {
    /// The character serial these items belong to.
    pub owner: u32,
    /// Every item worn or contained under that character, at any nesting depth.
    pub items: Vec<ItemRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_record_round_trips_through_json() {
        // Every field reachable by name from outside — a skipped field comes back
        // as its default, and an item that loads with a default location is on the
        // ground at 0,0 instead of in the pack it was saved in.
        for location in [
            ItemLocation::Ground {
                facet: 0,
                x: 1400,
                y: 1600,
                z: -5,
            },
            ItemLocation::Contained {
                container: 0x4000_0001,
                x: 40,
                y: 65,
                grid: 3,
            },
            ItemLocation::Equipped {
                mobile: 0x0000_0001,
                layer: 0x15,
            },
        ] {
            let record = ItemRecord {
                serial: 0x4000_0002,
                owner: 0x0000_0001,
                graphic: 0x0E75,
                hue: 0,
                amount: 1,
                stackable: false,
                container_gump: Some(0x003C),
                price: Some(11),
                name: Some("scissors".into()),
                spellbook: Some(0x0000_0000_00FF_00FF),
                location,
            };
            let json = serde_json::to_string(&record).expect("an item must serialise");
            let back: ItemRecord = serde_json::from_str(&json).expect("and come back");
            assert_eq!(back, record);
        }
    }

    #[test]
    fn a_character_record_round_trips_through_json() {
        // Not a test of serde. A test that every field is reachable by name from
        // outside the crate: a field that is private, skipped, or renamed by
        // accident is a field that comes back as its default, and a character
        // that loads with a default position is standing in the ocean.
        let record = CharacterRecord {
            serial: 0x0000_0001,
            account: "admin".into(),
            name: "Alpha".into(),
            body: 0x0190,
            hue: 0,
            facet: 0,
            x: 1363,
            y: 1600,
            z: 30,
            facing: 3,
            strength: 55,
            dexterity: 40,
            intelligence: 25,
            skills: vec![
                SkillRecord {
                    id: 25, // Magery
                    value: 501,
                    lock: 1, // down
                    cap: 1000,
                },
                SkillRecord {
                    id: 45, // Mining
                    value: 300,
                    lock: 0,
                    cap: 1200, // a raised cap: the field has to survive the trip
                },
            ],
            effects: vec![EffectRecord {
                kind: EFFECT_POISON,
                amount: 2,
                remaining: 5,
            }],
            dead: true,
            fame: 0,
            karma: 0,
            murders: 0,
            quests: vec![QuestRecord {
                key: "rat_cull".into(),
                progress: vec![3],
                seconds: vec![0],
                failed: false,
                giver: Some(0x4000_0001),
            }],
            done_quests: vec![DoneQuestRecord {
                key: "silk_gather".into(),
                restart_in_secs: 3600,
            }],
            stat_locks: StatLockRecord {
                strength: 0,     // up
                dexterity: 1,    // down
                intelligence: 2, // locked
                strength_age: 40,
                dexterity_age: 0,
                intelligence_age: 900,
            },
        };
        let json = serde_json::to_string(&record).expect("a record must serialise");
        let back: CharacterRecord = serde_json::from_str(&json).expect("and come back");
        assert_eq!(back, record);
    }

    #[test]
    fn a_negative_height_survives_the_round_trip() {
        // z is i8 and the obvious mistake is u8. UO has basements, mines and
        // dungeons at negative heights, and a character saved at z=-40 that
        // loads at z=216 is somewhere else entirely.
        let record = CharacterRecord {
            serial: 1,
            account: "admin".into(),
            name: "Alpha".into(),
            body: 0x0190,
            hue: 0,
            facet: 0,
            x: 5000,
            y: 500,
            z: -40,
            facing: 0,
            strength: 100,
            dexterity: 100,
            intelligence: 100,
            skills: Vec::new(),
            effects: Vec::new(),
            dead: false,
            fame: 0,
            karma: 0,
            murders: 0,
            quests: Vec::new(),
            done_quests: Vec::new(),
            stat_locks: StatLockRecord::default(),
        };
        let json = serde_json::to_string(&record).expect("a record must serialise");
        let back: CharacterRecord = serde_json::from_str(&json).expect("and come back");
        assert_eq!(back.z, -40);
    }
}
