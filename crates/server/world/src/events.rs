//! What the world says happened.
//!
//! # These live here, not in `openshard-events`
//!
//! That crate is machinery: `Events<E>`, `Cursor<E>`, a bus. It defines no game
//! events and must not, or every crate ends up depending on a file every other
//! crate edits.
//!
//! So a domain event lives with the rule that emits it. These are the world's,
//! because the world's tick is what decides a player moved. `NpcKilled` will
//! belong to combat; `HouseCreated` to housing.

use openshard_entities::EntityId;
use openshard_protocol::serial::Serial;
use openshard_protocol::world::Point;
use openshard_protocol::Facing;

/// A character entered the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerEntered {
    /// The entity.
    pub entity: EntityId,
    /// Its wire identity.
    pub serial: Serial,
    /// Where it appeared.
    pub position: Point,
}

// `ItemSpawned` moved to `openshard-items` with the item system that emits it.
// `world` re-exports it.

/// A client asked to cast a spell — from its spellbook or a macro.
///
/// The request off the wire, no more: what the spell *costs* and *does* — mana,
/// reagents, damage — is a script's, read off this event, the same script-first
/// decoupling `MobileSpoke` and `SkillUsed` have. The world hears "this mobile
/// wants spell N" and says so; a script turns that into an actual cast.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SpellRequested {
    /// The would-be caster.
    pub entity: EntityId,
    /// Its wire identity.
    pub serial: Serial,
    /// Which spell, zero-based.
    pub spell: u16,
}

// `MobileSpawned` moved to `openshard-npc` with the spawn rule that emits it.
// Re-exported here so readers keep finding it beside `PlayerEntered`.
pub use openshard_npc::MobileSpawned;

/// An NPC mobile came back from the store at boot, exactly as it was saved.
///
/// Deliberately *not* a `MobileSpawned`: spawning means "this did not exist a
/// moment ago", and a restore means the opposite. Anything that acts on a spawn
/// by *creating* something — a vendor's stock crate is the standing example —
/// would duplicate it on every reboot if the two shared an event.
///
/// But a world that comes back silently is a world the layers above it cannot
/// see. Everything a script learned about the NPCs it placed — which one gives
/// which quest, which one can be escorted — lived only in the memory of the run
/// that placed them, so the shard worked once and was inert after the first
/// restart, with nothing in any log to say so. This is the announcement that
/// closes that: a listener that binds rather than creates reads it and re-binds,
/// idempotently, on every boot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MobileRestored {
    /// The restored entity.
    pub entity: EntityId,
    /// Its wire identity — the same serial it had before the restart.
    pub serial: Serial,
    /// Its body, so a listener matches the kind without a lookup.
    pub body: u16,
    /// Where it stands right now.
    pub at: Point,
    /// The post it belongs to — its [`Npc`] home, or where it stands if it has
    /// none (a creature, a guard, anything without a beat).
    ///
    /// # Bind by this, not by `at`
    ///
    /// A pack binds its NPCs by tile: the tile a quest giver was *placed* on is
    /// the key it looks the quest up by. `at` is where the mobile was standing
    /// when the save was taken, and a townsperson does not stand still — with
    /// `npc_schedule` on it is somewhere else entirely for a third of the day.
    /// So a save taken at night would hand every quest to whichever townsperson
    /// happened to be nearest the giver's post, permanently, because the binding
    /// is itself persisted. A home does not move, which is what makes it a key.
    ///
    /// [`Npc`]: openshard_state::components::Npc
    pub home: Point,
}

/// A mobile took a step.
///
/// Emitted for the step, not for the turn: a turn changes no tile, and a
/// listener that cares about *where* things are should not have to filter out
/// events where nothing went anywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MobileMoved {
    /// The entity.
    pub entity: EntityId,
    /// Its wire identity.
    pub serial: Serial,
    /// Where it was.
    pub from: Point,
    /// Where it is.
    pub to: Point,
    /// Which way it now faces.
    pub facing: Facing,
}

/// A mobile turned on the spot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MobileTurned {
    /// The entity.
    pub entity: EntityId,
    /// Its wire identity.
    pub serial: Serial,
    /// Which way it now faces.
    pub facing: Facing,
}

/// A step was refused.
///
/// Worth an event rather than a log line: this is what a speedhack looks like
/// from the outside, and metrics and a GM tool both want to count it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StepRefused {
    /// The entity.
    pub entity: EntityId,
    /// Its wire identity.
    pub serial: Serial,
    /// Why.
    pub reason: RefusedReason,
}

/// Why a step was refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum RefusedReason {
    /// The client's walk sequence was out of step.
    OutOfSequence,
    /// Something is in the way, or the ground is not there.
    Blocked,
    /// The client is moving faster than a body can move.
    TooFast,
}

/// A character left the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerLeft {
    /// The entity, now despawned.
    pub entity: EntityId,
    /// Its wire identity, now released.
    pub serial: Serial,
}

/// A creature's corpse was laid — the loot hook.
///
/// The tick's `reap` emits this the instant a slain creature's corpse hits the
/// ground, carrying the corpse's serial and the body it was, so a script can fill
/// it with per-creature loot: the "default in core, customise in the pack" split
/// combat and magic already use. The core drops a flat baseline of gold first
/// (so a bare shard still loots); the pack *adds* the real table — items, rares,
/// a richer gold roll — off this event, by serial, through `op_add_loot`. Only a
/// creature corpse fires it; a player corpse holds the player's own gear, not
/// generated loot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CorpseCreated {
    /// The corpse item's wire identity — what a script fills.
    pub corpse: Serial,
    /// The body the creature was, so a pack matches its loot table with no
    /// lookup — the same key `creature_name`/`creature_base_sound` use.
    pub body: u16,
}

/// A mobile crossed from one region into another.
///
/// One event for the crossing, carrying both sides, rather than a pair — a step
/// out of one town and straight into another is a single thing that happened,
/// and a reader that has to correlate two events to see it will eventually
/// correlate them wrongly. Either side is optional: the unnamed wilds are
/// `None`, so walking out of a town is a `to` of nothing and walking in is a
/// `from` of nothing.
///
/// The engine reads it to hunt murderers who walk into a guarded town; the pack
/// reads it for everything a place *means* — a town's name for an escort's
/// destination, an ambush that fires when someone enters a valley. Carries an
/// owned `String`, so it is `Clone`, not `Copy`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionChanged {
    /// Who crossed.
    pub entity: EntityId,
    /// Their wire identity.
    pub serial: Serial,
    /// The facet the crossing happened on.
    pub facet: u8,
    /// The region left, if they were in one.
    pub from: Option<u16>,
    /// The region entered, if they are in one now.
    pub to: Option<u16>,
    /// The entered region's name, so a reader needs no lookup. Empty when the
    /// crossing was out into unnamed land.
    pub name: String,
}

/// A game master pressed a button in the `.admin` menu.
///
/// The engine carries the verb across; the script pack decides what it does —
/// which spawn set to register, what to clear. Emitted on the bus so a script
/// reads it like any other domain event, which is how a staff tool reaches the
/// pack. Carries an owned `String`, so it is `Clone`, not `Copy`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AdminMenuAction {
    /// The game master's wire identity.
    pub serial: Serial,
    /// The action the button asked for, e.g. `"populate:britain"`.
    pub action: String,
}

/// A player answered a pack-built gump (a `0xB1` for a gump that is *not* the
/// engine's own admin menu) — the reply seam for [`op_gump`]. The pack that
/// opened the dialog matches on `gump_id` and reads the `button` (and any text or
/// switches) to know what was chosen. Carries owned `String`s, so `Clone`.
///
/// [`op_gump`]: the `op_gump` scripting op.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GumpAnswered {
    /// Who answered, by wire identity.
    pub serial: Serial,
    /// Which dialog — the `gump_id` the pack sent.
    pub gump_id: u32,
    /// The button pressed; `0` is the close box (dismissed without a choice).
    pub button: u32,
    /// The switch (checkbox/radio) ids left on.
    pub switches: Vec<u32>,
    /// Any text fields, as `(field id, contents)`.
    pub text_entries: Vec<(u16, String)>,
}

// `MobileDamaged` and `MobileDied` moved to `openshard-combat` with the combat
// system that emits them. `world` re-exports both.

// `SkillUsed` moved to `openshard-skills` with the skill system that emits it.
// `world` re-exports it.

// `SpellCast` moved to `openshard-magic` with the casting system that emits it.
// `world` re-exports it.

// `MobileSpoke` moved to `openshard-chat` with the speech system that emits it —
// "domain events live with the crate that owns the rule". `world` re-exports it.
