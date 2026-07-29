//! What a thing in the world is made of.
//!
//! # Small, plain, and owned by the rule that needs them
//!
//! Nothing here is a "GameObject". A player is an entity that happens to carry a
//! [`Body`], a [`Position`] and a [`Client`]; an NPC is the same minus the
//! `Client`; a rock is a `Position` and a `Graphic`. What a thing *is* falls out
//! of what it carries, which is the whole reason for an ECS.
//!
//! These are the ones the world itself needs to put a character on screen and
//! move it. Combat's components belong to combat, housing's to housing. A
//! `Components` grab-bag every crate imports from would be an inheritance tree
//! with extra steps.

use std::collections::HashMap;

use openshard_entities::{EntityId, Serial};
use openshard_gateway::ConnectionId;
use openshard_movement::Walker;
use openshard_protocol::{AccessLevel, ClientVersion, Facing, Point, SkillLock};

/// Where a mobile or item is.
///
/// Separate from [`Walker`] because most things that have a position never walk:
/// a tree, a corpse, a chest. Giving them a walk sequence and a pace budget
/// would be storage spent on a question nobody asks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Position(pub Point);

/// Which way something faces.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Heading(pub Facing);

/// The graphic a mobile is drawn as.
///
/// UO calls this the "body". 0x0190 is a human male, 0x0191 a human female;
/// everything else is a creature.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Body {
    /// The body graphic id.
    pub id: u16,
    /// Its colour.
    pub hue: u16,
}

/// The graphic an item is drawn as: its tiledata id and hue.
///
/// The item counterpart of [`Body`]. An entity carries one or the other — a
/// mobile a `Body`, a thing on the ground a `Graphic` — and that is what the
/// interest system reads to decide which packet draws it: `0x78` for a body,
/// `0x1A` for a graphic. Kept in `world` and not in a gameplay crate for the
/// same reason `Body` is: drawing a thing in the world is the world's job, and
/// the crate that owns item *rules* (stacking, decay, containment) builds on
/// this rather than the other way round.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Graphic {
    /// The tiledata id.
    pub id: u16,
    /// Its colour, or 0 for none.
    pub hue: u16,
}

/// How many of a stackable item this entity is: a pile of 500 gold is one entity
/// with `Amount(500)`, not 500 entities.
///
/// Separate from [`Graphic`] because most items are single and storing a `1` on
/// every one of them is a column of ones. An item with no `Amount` is a single.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Amount(pub u16);

/// Marks an item as a container: something other items can be put inside.
///
/// The `gump` is the window the client draws when the container is opened — a
/// backpack, a wooden chest, a bank box each have their own. An item is a
/// container exactly when it carries this; nothing else changes about it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Container {
    /// The gump graphic the client opens for it.
    pub gump: u16,
}

/// Marks an item as being *inside* a container rather than on the ground.
///
/// An item carries either a [`Position`] (on the ground, in the sector grid and
/// on nearby screens) or a `Contained` (in a container, on nobody's ground) —
/// never both. The `x`/`y` are where it sits in the container's gump, not world
/// tiles; `grid` is its slot in the enhanced grid view.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Contained {
    /// The container it is in, by serial.
    pub container: Serial,
    /// Its column in the gump.
    pub x: u16,
    /// Its row in the gump.
    pub y: u16,
    /// Its slot in the grid view.
    pub grid: u8,
}

/// Marks an item as *worn* by a mobile, at a layer.
///
/// The third and last place an item can be, alongside [`Position`] (the ground)
/// and [`Contained`] (a container) — and exclusive with both. A layer holds at
/// most one item: a right hand has one weapon, a torso one shirt. Which layer an
/// item belongs on comes from its tiledata; the client proposes it and the
/// server checks the slot is free.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Equipped {
    /// The mobile wearing it.
    pub mobile: Serial,
    /// Which layer it sits on.
    pub layer: u8,
}

/// Marks an item as one that stacks: two of them of the same graphic and hue
/// are one pile, not two objects.
///
/// A marker, not a rule engine. Gold, arrows and reagents carry it; a sword does
/// not, which is why dropping a sword on a sword leaves two swords. Whether a
/// graphic stacks is really a tiledata fact, but keeping it an explicit component
/// set at spawn keeps the rule where a script can see it — the §6 way.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Stackable;

/// When an item on the ground will rot away, as a tick number.
///
/// A tick count and not an `Instant` on purpose: the tick already counts itself,
/// so decay is checked against the world's tick counter and stays as
/// deterministic and replayable as everything else the tick does — no clock read
/// inside it. An item carries this only while it is on the ground; lifting it,
/// putting it in a container or wearing it takes the clock off it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Decays {
    /// The tick at or after which it rots.
    pub at_tick: u64,
}

/// What something is called.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Name(pub String);

/// One quest a character has taken, and how far along it is.
///
/// `progress` runs parallel to the definition's objective list — one count per
/// objective, in the same order. Positional, like ServUO's own save, which is why
/// **reordering a quest's objectives invalidates saved progress on it**; adding
/// one to the end is safe.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestState {
    /// Which quest, by the pack's key.
    pub key: String,
    /// How far each objective has got.
    pub progress: Vec<u16>,
    /// Ticks left on each timed objective; `0` on the untimed ones.
    pub seconds_left: Vec<u32>,
    /// Whether a timer ran out on it. A failed quest stays in the log, in red,
    /// until it is resigned — ServUO shows it rather than removing it, so the
    /// player finds out why it stopped counting.
    pub failed: bool,
    /// Who gave it, so the turn-in knows where to go back to. `None` once that
    /// mobile is gone.
    pub giver: Option<Serial>,
}

/// A quest a character has finished, and when they may take it again.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DoneQuest {
    /// Which quest, by the pack's key.
    pub key: String,
    /// The tick it may be offered again at. [`u64::MAX`] never.
    pub restart_at: u64,
}

/// A player's quest log: what they are doing, and what they have done.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct QuestLog {
    /// Quests in progress, newest last. The gump lists them newest first.
    pub active: Vec<QuestState>,
    /// Quests finished, with their cooldowns.
    pub done: Vec<DoneQuest>,
}

impl QuestLog {
    /// The state of an active quest, if it is one.
    #[must_use]
    pub fn active_quest(&self, key: &str) -> Option<&QuestState> {
        self.active.iter().find(|quest| quest.key == key)
    }

    /// The state of an active quest, to change.
    pub fn active_quest_mut(&mut self, key: &str) -> Option<&mut QuestState> {
        self.active.iter_mut().find(|quest| quest.key == key)
    }
}

/// An NPC that offers quests — ServUO's `MondainQuester`, as a component.
///
/// The binding lives on the mobile and is **saved with it**, which is the whole
/// point: the script that placed the NPC knows it is a giver only during the run
/// that placed it, so a binding held anywhere else is lost at the first restart
/// and the NPC goes quietly inert.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestGiver {
    /// Which quests it may offer, by key, in preference order.
    pub keys: Vec<String>,
}

/// An NPC that can be escorted somewhere — ServUO's `BaseEscortable`.
///
/// Saved with the mobile for the same reason [`QuestGiver`] is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Escortable {
    /// The region it wants to reach, by name. Empty means "wherever the escorter's
    /// quest says", picked when the escort is accepted.
    pub destination: String,
    /// Who is leading it, once someone is.
    pub escorter: Option<Serial>,
    /// The last tick its escorter was within sight. An escortable left behind
    /// gives up rather than following a ghost across the map.
    pub last_seen: u64,
}

/// The account a player character belongs to.
///
/// Kept out of [`Client`] so that stays `Copy` — this is a heap string, and the
/// only thing that needs it is persistence, turning an entity into a record that
/// remembers whose character it is. An NPC has no account and no `Client`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Account(pub String);

/// Marks an item as script-placed decoration: a sign, a piece of furniture, an
/// ankh — the things a shard adds on top of the static art the client's map
/// already draws.
///
/// It sets the item apart from loose clutter: decoration never decays and cannot
/// be picked up (a town's fittings are not loot), and clearing decoration finds
/// its items by this. Placed through `op_decorate`; the client draws it as an
/// ordinary `0x1A` item.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Decoration;

/// Marks an item as a door: a decoration that opens and closes on double-click.
///
/// A UO door is two graphics and a small position shift. Closed it draws
/// `closed`; opened it draws `open` (always `closed + 1` in the client's art) and
/// hops one tile off its frame by `(offset_x, offset_y)` — the hinge swing. The
/// same double-click toggles it back. `open_at` is the tick the door auto-closes
/// on, mirroring the real client's self-closing door; `0` means it is shut.
///
/// The graphic and offset are the client's, computed once from ServUO's door
/// tables when the pack places the door, so the engine stays a generic toggle and
/// knows nothing about door *families*.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Door {
    /// The graphic drawn while shut.
    pub closed: u16,
    /// The graphic drawn while open.
    pub open: u16,
    /// How far the door hops east/west when it swings open.
    pub offset_x: i16,
    /// How far it hops north/south.
    pub offset_y: i16,
    /// Whether the door is currently open.
    pub is_open: bool,
    /// The tick it auto-closes on when open; `0` when shut.
    pub close_at: u64,
}

/// How widely known a mobile is — ServUO's `Mobile.Fame`, `0..=32000`.
///
/// Earned by killing things, and by killing *famous* things in particular: a creature
/// gives up its own fame. Half of what a character's title is computed from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Fame(pub i32);

/// Which way a mobile is known — ServUO's `Mobile.Karma`, `-32000..=32000`.
///
/// Killing something evil earns karma; killing something innocent loses it. The other
/// half of the title, and the axis a creature's own notoriety colour is derived from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Karma(pub i32);

/// A lock on a door or a container — ServUO's `ILockable`: `Locked` plus a
/// `KeyValue` that says which key fits.
///
/// # A lock is a refusal, not a second kind of door
///
/// Everything about a locked door is the same as an unlocked one — the graphic, the
/// offset, the auto-close, the obstruction it registers while shut. The only
/// difference is that the thing which would open it does not. So this is a marker
/// beside [`Door`] rather than a field inside it, and the two places that open a
/// door consult it: a player's double-click (answered with cliloc 502503, "That is
/// locked.") and the AI's decree, which is what stops a townsperson strolling
/// through a locked shopfront on its way home.
///
/// `key_value` is ServUO's: a key fits when its own value matches, and `0` is a lock
/// no key in the world opens — a set-piece door, not a mistake.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Lock {
    /// Which key opens it. `0` fits no key.
    pub key_value: u32,
    /// The Lockpicking a thief needs before the lock will even be tried, in tenths
    /// — ServUO's `LockLevel`. Zero is a lock anybody may attempt.
    pub required_skill: u16,
    /// The skill at which it is no challenge at all, in tenths — ServUO's
    /// `MaxLockLevel`, the top of the band a pick is rolled against.
    pub max_skill: u16,
}

/// A key, and what it opens — ServUO's `Key.KeyValue`.
///
/// Using a key raises a target cursor; clicking a [`Lock`] whose `key_value` matches
/// turns it. The value and not the item is what matters, so a copied key works and a
/// key to another door does not.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KeyValue(pub u32);

/// Which spawn region put this mobile here — an index into the world's spawner
/// list.
///
/// The region counts its live creatures by this to know when to refill. A
/// creature dies and is despawned, the component goes with it, the count drops,
/// and the region spawns another. Absent on players and on script- or GM-spawned
/// mobiles, which no region maintains.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SpawnedBy(pub u32);

/// A mobile's staff authority — what privileged commands it may run.
///
/// Set on world entry from the account's configured level, not saved with the
/// character: authority is a property of who is logged in, re-derived each login,
/// so a demoted account loses it the next time it plays. A mobile with no `Access`
/// is a [`AccessLevel::Player`], the same as the default the level carries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Access(pub AccessLevel);

/// A mobile that is *acting* as staff right now — Sphere's `PRIV_GM` flag.
///
/// The other half of the split [`Access`] starts: the level says who *may*
/// command, this says who is currently held to none of the game's rules. A staff
/// account gets it at login and `.gm` takes it off, so a game master can walk
/// under a player's rules — tiring, blind to ghosts — without giving up the
/// commands that let them switch back. Never saved: like [`Access`], it is
/// derived from the account, not from the character.
///
/// Every in-game exemption reads it through
/// [`WorldState::is_staff`](crate::WorldState::is_staff); nothing should test
/// `Access` for one, or the two halves drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Staff;

/// Which facet a mobile is on: 0 Felucca, 1 Trammel, and so on.
///
/// A mobile only ever interacts with others on the same facet — the world keeps
/// a separate map and interest grid per facet — so this is what selects which.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Facet(pub u8);

/// Marks an entity as driven by a person rather than by the server.
///
/// Carries the connection so the world can answer it, and the version so
/// encoders can ask what this particular client understands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Client {
    /// Which connection.
    pub connection: ConnectionId,
    /// What it claims to be. Every feature gate reads this.
    pub version: ClientVersion,
}

/// A mobile's three stats: strength, dexterity, intelligence.
///
/// The numbers everything derived hangs off. Strength sets how many hit points a
/// mobile can have, intelligence how much mana; dexterity will pace its swings
/// and its stamina once those derive rather than sit as constants. A script sets
/// them (character creation, a monster's build); the maxima follow.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Stats {
    /// Raw might — the cap on hit points.
    pub strength: u16,
    /// Quickness — the cap on stamina, and the pace of a swing, to come.
    pub dexterity: u16,
    /// Wits — the cap on mana.
    pub intelligence: u16,
}

/// A mobile's hit points: how much it has, and how much it can have.
///
/// The thing combat spends. A mobile is alive while `current > 0` and dead at
/// zero. Only mobiles carry it — an item on the ground has no health to lose.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hitpoints {
    /// How much it has now.
    pub current: u16,
    /// The most it can have.
    pub max: u16,
}

/// Marks a mobile as temporarily a criminal: grey, and freely attackable,
/// until the tick it wears off.
///
/// The consequence of an aggressive act on someone blue — the flag that stops a
/// player attacking innocents in a town with no cost. A tick number, like
/// [`Decays`]; when the tick counter passes it the mobile goes back to innocent
/// (or to murderer, if it has become one — see [`Murders`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CriminalUntil {
    /// The tick the flag lifts.
    pub tick: u64,
}

/// A mobile that cannot move until its tick — paralysis, from the Paralyze spell
/// or a Paralyze Field. The one hard rule of paralysis: the walk and the step both
/// refuse while it holds; a blow lifts it (damage wakes you); it lapses on the tick
/// counter. Casting and swinging are *not* barred (the classic engine leaves those
/// to the client), only movement. A tick number, like [`CriminalUntil`], so it
/// replays.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Frozen {
    /// The tick the mobile can move again.
    pub until: u64,
}

/// Poison working through a mobile: its strength, the tick its next pulse lands,
/// and how many pulses remain before it clears. Tick counts, never a clock — a
/// poisoned fight replays like decay and the criminal flag — so `poison_tick`
/// reads only the world's counter.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Poisoned {
    /// The poison level, 0 (lesser) .. 4 (lethal) — sets the damage per pulse.
    pub level: u8,
    /// The tick the next pulse of damage lands.
    pub next_pulse: u64,
    /// Pulses left before the poison wears off.
    pub pulses_left: u8,
}

/// Poison an *item* carries: a dose in a bottle, or a coating on a blade.
///
/// One component for both because they are the same fact — how strong the poison is
/// and how much of it is left — and what an item can *do* with it is decided by
/// what the item is, exactly as ServUO decides (`targeted is BasePoisonPotion`
/// against `targeted is BaseWeapon`). A potion holds one dose; a blade the Poisoning
/// skill has coated holds `18 - level * 2`, spent a charge per landed blow.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PoisonCharges {
    /// The poison level, 0 (lesser) .. 4 (lethal) — the same scale [`Poisoned`] uses.
    pub level: u8,
    /// Doses left. Zero means spent, and a spent coating is removed rather than
    /// kept at zero, so this is never `0` on a live component.
    pub charges: u16,
}

/// A musical instrument, and how many tunes are left in it — ServUO's
/// `BaseInstrument.UsesRemaining`.
///
/// The bard skills all need one in the pack, and each attempt spends a use. Which
/// *sounds* it makes is a property of the class, so it lives in the core table
/// keyed by graphic; how worn this particular one is lives here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Instrument {
    /// Tunes left. At zero the instrument plays its last and is gone.
    pub uses_left: u16,
}

/// A harvesting tool, and how many swings are left in it — ServUO's
/// `BaseHarvestTool.UsesRemaining`.
///
/// The sibling of [`Instrument`], and the same interface in ServUO
/// (`IUsesRemaining`): which *system* a tool drives is a property of its class and
/// lives in the core table ([`crate::harvest::tool_data`]), how worn this
/// particular pickaxe is lives here. At zero the tool breaks and is gone.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Tool {
    /// Swings left.
    pub uses_left: u16,
}

/// A harvest in progress — ServUO's `HarvestTimer`.
///
/// The one gathering fact that is genuinely stateful, and the reason it is a
/// component rather than a local: a swing takes several beats, and between them
/// the harvester can walk away, the vein can be emptied by somebody else, or the
/// shard can tick a hundred times. Every field but the target is answered by the
/// tick counter, like [`Decays`] and a swing timer, so a harvest replays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Harvesting {
    /// The tool being swung. Re-checked each beat: a pickaxe dropped mid-swing
    /// mines nothing.
    pub tool: EntityId,
    /// The tile being worked.
    pub at: Point,
    /// Which system this is, so the beat needs no second lookup.
    pub kind: crate::harvest::HarvestKind,
    /// The tile id, as [`crate::harvest::tile_key`] matched it — kept so the beat
    /// can confirm the ground has not changed under the swing.
    pub tile: u16,
    /// Beats still to come. The last one delivers.
    pub beats_left: u16,
    /// The tick the next beat falls on.
    pub next_beat: u64,
    /// The tick this beat's *sound* falls on, or [`u64::MAX`] once it has played.
    ///
    /// A second clock rather than one, because ServUO gives the swing and the
    /// noise it makes different delays (`EffectDelay` against `EffectSoundDelay`):
    /// a pick is raised, and the chink comes most of a second later. Collapsing
    /// them makes a miner sound like a metronome.
    pub next_sound: u64,
}

/// A craft in progress — ServUO's `CraftItem.InternalTimer`.
///
/// The sibling of [`Harvesting`], and stateful for the same reason: a craft takes
/// a beat or several, and in between the crafter can walk away from the forge,
/// hand the ingots to a friend, or wear the tongs out on something else. Every
/// gate is re-checked on the last beat rather than trusted from the first, which
/// is why the recipe is held as a pair of indices and not as a resolved plan.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Crafting {
    /// Which craft system, by its index in the core table.
    pub system: u8,
    /// Which of that system's recipes, by index.
    pub recipe: u16,
    /// The tool in hand. Re-checked each beat: tongs dropped mid-craft make
    /// nothing.
    pub tool: EntityId,
    /// Which material off the system's axis — the ore or wood the player chose in
    /// the gump.
    pub sub_res: u8,
    /// Beats still to come. The last one resolves.
    pub beats_left: u8,
    /// The tick the next beat falls on.
    pub next_beat: u64,
}

/// How well a crafted item came out — ServUO's `IQuality.Quality`.
///
/// Only ever present on an *exceptional* piece: an ordinary item carries no
/// component at all, which is what keeps the column the size of the handful of
/// masterpieces on a shard rather than the size of every item in it.
///
/// Read where it matters and folded into nothing — the armour rating derives it
/// at the read site the way a weapon's speed derives from what is on the hand, so
/// a fine breastplate coming off leaves no bookkeeping behind.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Quality {
    /// Whether it is exceptional. A field rather than a bare marker because
    /// ServUO's scale has a low grade too, and a shard that wants it should widen
    /// this rather than add a second component.
    pub exceptional: bool,
}

/// Who made it — ServUO's `ICraftable.Crafter`, the maker's mark.
///
/// A **name and not a serial**, for the reason [`Corpse`]'s killer is one: the
/// smith logs out, retires, or is deleted, and the sword outlives all three. A
/// serial would leave "crafted by (nobody)" on every good blade on the shard.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CraftedBy(pub String);

/// A mobile a bard has calmed — ServUO's `BaseCreature.BardPacified`.
///
/// It does not swing and it does not pick fights while this holds, which is read at
/// combat's and the AI's own decision points rather than folded into either. A tick
/// count, like every other expiry.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pacified {
    /// The tick the calm lifts.
    pub until: u64,
}

/// A mobile a bard has put out of tune — ServUO's Discordance.
///
/// `penalty` is a percentage taken off everything the target is good at. It is read
/// in exactly one place, `skills::skill_value`, which is what every other system
/// asks when it wants to know how good somebody is — so a discorded creature hits
/// worse, resists worse and casts worse without any of those three knowing what a
/// lute is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Discorded {
    /// How much worse at everything, as a percentage.
    pub penalty: u16,
    /// The tick the song wears off.
    pub until: u64,
}

/// What a trap on a container does when it goes off — ServUO's `TrapType`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TrapKind {
    /// A flash and a jolt: damage to whoever is standing at the lid.
    Magic,
    /// A blast: the heaviest damage, and it reaches three tiles.
    Explosion,
    /// A dart in the flesh — physical damage.
    Dart,
    /// A noxious green cloud: poison rather than damage.
    Poison,
}

/// A trap on a container: what it does, how hard it hits, and how hard it is to
/// take off — ServUO's `TrapableContainer` fields (`TrapType`, `TrapPower`,
/// `TrapLevel`).
///
/// It springs when the container is opened by anyone but staff, and Remove Trap is
/// the skill that takes it off. Both halves matter: without the trigger a trap is a
/// decoration, and without the disarm it is a wall.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Trap {
    /// What it does.
    pub kind: TrapKind,
    /// How hard it hits when `level` is zero, and the difficulty Remove Trap is
    /// rolled against either way (`power .. power + 10`).
    pub power: u16,
    /// The chest's level, which scales the damage instead of `power` when set.
    pub level: u8,
}

/// The item graphic every poison potion shares — `0x0F0A`, ServUO's
/// `BasePoisonPotion : base(0xF0A, effect)`.
///
/// All four strengths are the same bottle: which poison one holds is on the item
/// (a [`PoisonCharges`]), not in its graphic, which is why the core cannot key
/// poison off a table the way it keys a weapon's damage.
pub const POISON_POTION_GRAPHIC: u16 = 0x0F0A;

/// The empty bottle a used potion leaves behind — ServUO hands one back on every
/// `Consume`.
pub const EMPTY_BOTTLE_GRAPHIC: u16 = 0x0F0E;

/// What a persistent field does — the behaviour a field-tile entity carries.
///
/// A spell lays a row of ground tiles that either pulse harm or bar the way, on
/// the tick counter like [`Poisoned`] and decay.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FieldKind {
    /// Fire Field — pulses fire damage to whoever stands on it; not a wall.
    Fire,
    /// Poison Field — poisons whoever stands on it; not a wall.
    Poison,
    /// Energy Field — an impassable wall; no damage.
    Energy,
    /// Wall of Stone — an impassable wall; no damage.
    Stone,
    /// Paralyze Field — freezes whoever walks onto it ([`Frozen`](super::Frozen));
    /// not a wall, because you must be able to step on to be caught.
    Paralyze,
}

impl FieldKind {
    /// Whether a mobile cannot walk onto this field — a wall (Energy, Stone), not
    /// a hazard you cross and are caught by (Fire, Poison, Paralyze).
    #[must_use]
    pub fn blocks(self) -> bool {
        matches!(self, Self::Energy | Self::Stone)
    }

    /// Whether this field acts on whoever stands on it each cadence (damage for
    /// Fire/Poison, a freeze for Paralyze) — as opposed to a passive wall.
    #[must_use]
    pub fn pulses(self) -> bool {
        matches!(self, Self::Fire | Self::Poison | Self::Paralyze)
    }
}

/// One tile of a persistent field — a ground entity that pulses harm or blocks the
/// way until its tick comes. The field counterpart of [`Poisoned`]: `next_pulse`
/// and `expires_at` are tick counts, so a field replays like decay.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Field {
    /// What the field does.
    pub kind: FieldKind,
    /// Who laid it — a Fire Field's damage is credited to the caster, so a field
    /// kill counts.
    pub caster: Serial,
    /// The tick the next pulse of harm lands (Fire, Poison); unused for a wall.
    pub next_pulse: u64,
    /// The tick the tile vanishes.
    pub expires_at: u64,
    /// Whether the tile is registered in the obstruction index (Energy, Stone).
    pub blocks: bool,
}

/// The z-span a wall-like field tile occupies in the obstruction index — tall
/// enough that a mobile's own span always intersects it, so it bars the way like a
/// shut door.
pub const FIELD_HEIGHT: u8 = 20;

/// The kind tag on a saved effect and a live [`StatMod`], canonical across the
/// engine.
///
/// One numbering, shared by everything that reads or writes an effect: the
/// persistence [`EffectRecord`](openshard_persistence) stores this `u8`, `magic`
/// tags a [`StatMod`] with it, and the world's save/restore translates the two.
/// Poison (`0`) is the odd one out — its live form is [`Poisoned`], not a
/// `StatMod` — but it shares the numbering so one effects list carries the lot.
/// The order is frozen: a saved `4` must always mean Bless, or old saves rot.
pub mod effect {
    /// Poison — [`Poisoned`](super::Poisoned), not a stat modifier.
    pub const POISON: u8 = 0;
    /// Strength: `+str`.
    pub const STRENGTH: u8 = 1;
    /// Agility: `+dex`.
    pub const AGILITY: u8 = 2;
    /// Cunning: `+int`.
    pub const CUNNING: u8 = 3;
    /// Bless: `+` all three.
    pub const BLESS: u8 = 4;
    /// Weaken: `-str`.
    pub const WEAKEN: u8 = 5;
    /// Clumsy: `-dex`.
    pub const CLUMSY: u8 = 6;
    /// Feeblemind: `-int`.
    pub const FEEBLEMIND: u8 = 7;
    /// Curse: `-` all three.
    pub const CURSE: u8 = 8;
    /// Night Sight — a personal light override, not a stat. See
    /// [`BehaviourBuffs`](super::BehaviourBuffs).
    pub const NIGHT_SIGHT: u8 = 9;
    /// Protection — a chance a blow does not break concentration mid-cast.
    pub const PROTECTION: u8 = 10;
    /// Reactive Armor — a share of melee physical damage reflected to the attacker.
    pub const REACTIVE_ARMOR: u8 = 11;
    /// Magic Reflection — bounces the next offensive spell back at its caster.
    pub const MAGIC_REFLECT: u8 = 12;
    /// Paralyze — a [`Frozen`](super::Frozen) mobile that cannot move until it lifts.
    pub const PARALYZE: u8 = 13;
}

/// Which stats a stat-modifying effect shifts, and by how much.
///
/// The `kind` names *which* stats (Strength touches str, Bless all three); the
/// signed `offset` carries the magnitude and direction. Returns the delta for
/// `(strength, dexterity, intelligence)`. A debuff simply arrives with a negative
/// `offset` — so the same function undoes a buff by being called with the offset
/// negated, which is exactly how [`StatMod`] reversal works.
#[must_use]
pub fn stat_shift(kind: u8, offset: i16) -> (i16, i16, i16) {
    use effect::*;
    match kind {
        STRENGTH | WEAKEN => (offset, 0, 0),
        AGILITY | CLUMSY => (0, offset, 0),
        CUNNING | FEEBLEMIND => (0, 0, offset),
        BLESS | CURSE => (offset, offset, offset),
        _ => (0, 0, 0),
    }
}

/// Whether an effect kind lowers a stat rather than raising it — the sign the
/// caster gives its magnitude.
#[must_use]
pub fn is_debuff(kind: u8) -> bool {
    use effect::*;
    matches!(kind, WEAKEN | CLUMSY | FEEBLEMIND | CURSE)
}

/// One timed stat modifier: which effect, how much, and the tick it lifts.
///
/// The `offset` is signed and pre-distributed by [`stat_shift`] from the `kind`;
/// it is kept whole so expiry can reverse *exactly* what was applied, even if the
/// base stat changed underneath it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StatMod {
    /// Which effect — an [`effect`] kind (Strength..Curse).
    pub kind: u8,
    /// The signed magnitude applied to each stat the kind selects.
    pub offset: i16,
    /// The tick it wears off.
    pub expires_at: u64,
}

/// The stat modifiers working through a mobile — the Bless/Curse family.
///
/// A mobile can carry several at once (Bless stacked over Strength); re-casting
/// one kind refreshes its entry rather than stacking a duplicate. The shift is
/// folded into the live [`Stats`] (and the derived [`Hitpoints`]/[`Mana`] maxima)
/// when the effect lands, so everything that reads a stat sees the buffed value;
/// this component is the ledger that says how much to give back, and when. Tick
/// counts, like every other timed effect, so a buffed fight replays.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StatMods {
    /// The active modifiers, at most one per kind.
    pub active: Vec<StatMod>,
}

/// One timed behaviour buff — a spell that changes *how* a mobile acts rather than
/// a stat number: Night Sight, Protection, Reactive Armor, Magic Reflection.
///
/// Unlike a [`StatMod`], nothing is folded into a stat, so there is nothing to
/// back out on expiry — the buff simply stops being read at its decision point.
/// The `amount` carries what that point needs (a Protection chance, a Reactive
/// Armor reflect percent); it is unused for the markers (Night Sight, Magic
/// Reflect). Tick counts, like every timed effect, so it replays.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BehaviourBuff {
    /// Which buff — an [`effect`] kind (`NIGHT_SIGHT`..`MAGIC_REFLECT`).
    pub kind: u8,
    /// The magnitude the buff's decision point reads (chance, reflect percent),
    /// or `0` for a bare marker.
    pub amount: i16,
    /// The tick it wears off.
    pub expires_at: u64,
}

/// The behaviour buffs working through a mobile — the non-stat magical family.
///
/// The sibling of [`StatMods`] for effects that modify a behaviour, not a stat:
/// at most one entry per kind, a recast refreshes rather than stacks, and each
/// entry rides the same saved effects list. Read at the point the behaviour is
/// decided — Reactive Armor in the damage door, Protection at cast disturbance,
/// Magic Reflection where a spell resolves.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct BehaviourBuffs {
    /// The active buffs, at most one per kind.
    pub active: Vec<BehaviourBuff>,
}

/// How many innocents a mobile has killed — the tally that turns it red.
///
/// The deeper standing [`CriminalUntil`] left for later: a persistent count, not
/// a lapsing timer. Once it passes the murder threshold the mobile is a murderer;
/// the grey criminal flag comes and goes, this only fades slowly, one kill at a
/// time, on a [`MurderDecay`] clock.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Murders(pub u16);

/// When a mobile's murder count next drops by one.
///
/// A tick number, like [`Decays`]: old kills age off rather than staying forever,
/// so a reformed killer eventually washes blue again. One count fades per fire,
/// and the clock reschedules until the tally is empty. (Sphere's separate
/// short-term and long-term counts are a finer model this stands in for.)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MurderDecay {
    /// The tick the next count fades.
    pub at_tick: u64,
}

/// The ceiling one skill trains to when nothing has raised or lowered it, in
/// tenths — 100.0. ServUO's per-`Skill` `m_Cap` default.
pub const DEFAULT_SKILL_CAP: u16 = 1000;

/// What a mobile is trained in: each skill it has, by id, as a value in tenths
/// (so 75.5 is stored as 755, and the skill cap is 1000).
///
/// Sparse on purpose — a mobile knows the handful of skills it has been given,
/// not all fifty-odd at zero. An id it has never trained reads as zero.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Skills {
    values: HashMap<u8, u16>,
    /// How the window trains each skill — `Up` unless the player set an arrow.
    /// Sparse like the values: an untouched skill trains up.
    locks: HashMap<u8, SkillLock>,
    /// The ceiling on each skill, in tenths. Sparse like the rest: an untouched
    /// skill caps at [`DEFAULT_SKILL_CAP`]. Per-skill and not one shard-wide
    /// number because the gain chance reads *this* skill's headroom, and because
    /// a reward or a profession raises one skill's ceiling alone.
    caps: HashMap<u8, u16>,
}

impl Skills {
    /// The value of `skill`, in tenths; zero if the mobile has never had it.
    pub fn get(&self, skill: u8) -> u16 {
        self.values.get(&skill).copied().unwrap_or(0)
    }

    /// Set `skill` to `value` tenths.
    pub fn set(&mut self, skill: u8, value: u16) {
        self.values.insert(skill, value);
    }

    /// How `skill` is set to train; `Up` unless the player moved its arrow.
    pub fn lock(&self, skill: u8) -> SkillLock {
        self.locks.get(&skill).copied().unwrap_or_default()
    }

    /// Set how `skill` trains — the up/down/lock arrow.
    pub fn set_lock(&mut self, skill: u8, lock: SkillLock) {
        self.locks.insert(skill, lock);
    }

    /// The ceiling on `skill`, in tenths; [`DEFAULT_SKILL_CAP`] unless one was set.
    pub fn cap(&self, skill: u8) -> u16 {
        self.caps.get(&skill).copied().unwrap_or(DEFAULT_SKILL_CAP)
    }

    /// Set the ceiling on `skill`, in tenths.
    pub fn set_cap(&mut self, skill: u8, cap: u16) {
        self.caps.insert(skill, cap);
    }

    /// Everything trained, added up, in tenths — ServUO's `Skills.Total`, the
    /// number the total cap is weighed against and the gain chance reads.
    ///
    /// Summed on demand rather than kept as a running field: a mirror updated
    /// beside every `set` is one more thing to forget, and the map holds a
    /// handful of entries, not fifty-eight.
    pub fn total(&self) -> u32 {
        self.values.values().map(|&v| u32::from(v)).sum()
    }

    /// Every trained skill and its lock, for persistence — `(id, value, lock)`,
    /// in no particular order. A skill at zero with a moved arrow still counts,
    /// so a "down" lock the player set is not forgotten.
    pub fn entries(&self) -> impl Iterator<Item = (u8, u16, SkillLock)> + '_ {
        self.ids().map(move |id| (id, self.get(id), self.lock(id)))
    }

    /// Every skill id this mobile has a value, a lock or a cap for, ascending.
    /// The one place the three sparse maps are unioned.
    pub fn ids(&self) -> impl Iterator<Item = u8> + '_ {
        self.values
            .keys()
            .chain(self.locks.keys())
            .chain(self.caps.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
    }
}

/// How a stat is set to train — ServUO's `StatLockType`, the arrows on the
/// paperdoll's status bar. The mirror of [`SkillLock`] for strength, dexterity
/// and intelligence, and read by the same gain path: a skill that trains nudges
/// its governing stat only where that stat's arrow points up.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum StatLock {
    /// Train up on use — the default.
    #[default]
    Up,
    /// Give ground, so another stat can rise past the total cap.
    Down,
    /// Held fixed.
    Locked,
}

impl StatLock {
    /// The wire bits — two per stat inside the `0xBF 0x19` lock byte.
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        match self {
            Self::Up => 0,
            Self::Down => 1,
            Self::Locked => 2,
        }
    }

    /// From the wire byte. ServUO's handler folds anything above 2 to `Up`.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        match bits {
            1 => Self::Down,
            2 => Self::Locked,
            _ => Self::Up,
        }
    }
}

/// When a mobile may next use a skill from the window.
///
/// ServUO's `Mobile.NextSkillTime`, as a tick count like every other timer here.
/// One clock for all skills, not one per skill: the classic client's window is a
/// list of buttons, and holding any of them down is the thing being prevented.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SkillCooldown {
    /// The tick the next use is allowed on.
    pub until: u64,
}

/// Which way each of a mobile's three stats trains.
///
/// All `Up` by default, so a mobile that has never been told otherwise behaves
/// like every character does on a fresh shard.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct StatLocks {
    /// Strength's arrow.
    pub strength: StatLock,
    /// Dexterity's arrow.
    pub dexterity: StatLock,
    /// Intelligence's arrow.
    pub intelligence: StatLock,
}

/// When each stat last went up, as a tick count.
///
/// ServUO's `LastStrGain`/`LastDexGain`/`LastIntGain` — a per-stat cooldown so a
/// flurry of skill uses cannot pour points into one stat. A tick count and not a
/// clock, like [`Decays`] and [`CriminalUntil`], so it replays.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct LastStatGain {
    /// The tick strength last rose.
    pub strength: u64,
    /// The tick dexterity last rose.
    pub dexterity: u64,
    /// The tick intelligence last rose.
    pub intelligence: u64,
}

/// A living mobile that can hear the dead, until `until` — ServUO's
/// `Mobile.CanHearGhosts`, which Spirit Speak turns on for a while.
///
/// It gates **hearing only**, never drawing: a ghost stays invisible to the living
/// however much Spirit Speak they have, and the point of the classic skill is that
/// you catch a voice with nobody there. So the two questions are two predicates —
/// [`WorldState::can_see_mobile`] and [`WorldState::can_hear_mobile`] — and only the
/// second consults this.
///
/// A tick count, like every other expiry in the engine, and deliberately **not**
/// saved: fifteen seconds to three minutes puts it in the same class as a cast in
/// flight or a field on the ground.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HearsGhosts {
    /// The tick the contact fades.
    pub until: u64,
}

/// A creature that can be tamed, and what it takes — ServUO's `BaseCreature`
/// `Tamable`/`MinTameSkill`/`ControlSlots`.
///
/// Data about the *kind*, which is why the core keeps a table of it keyed by body
/// ([`crate::tame`]) and a spawn may override it: a shard's pack decides what walks
/// in its woods, and the engine decides what a horse is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Tamable {
    /// The Animal Taming needed even to try, in tenths — ServUO's `MinTameSkill`.
    pub min_skill: u16,
    /// How much of a tamer's following it takes up, in slots.
    pub slots: u8,
}

/// What a tamed creature is doing, and for whom — ServUO's `ControlOrder`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PetOrder {
    /// Walk at the owner's heel.
    Follow,
    /// Come here, then stand.
    Come,
    /// Stay where you are.
    Stay,
    /// Stand watch and answer anything that strikes the owner.
    Guard,
    /// Kill what the owner named.
    Attack,
    /// Stop whatever you were doing.
    Stop,
}

/// A tamed creature: whose it is, and what it was last told.
///
/// The pet's *brain* reads this and decides a step, exactly as a wild creature's
/// does — a pet is not a second kind of mobile, it is a creature with an owner and
/// an order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pet {
    /// Whose it is, by wire serial — a serial rather than an entity because the
    /// owner logs out and comes back while the pet stands where it was.
    pub owner: Serial,
    /// How many follower slots it fills.
    pub slots: u8,
    /// What it was last told to do.
    pub order: PetOrder,
    /// Whom that order was about, for Attack.
    pub order_target: Option<Serial>,
}

/// A mobile nobody can see — ServUO's `Mobile.Hidden`.
///
/// The marker the whole stealth family hangs off. It is read in exactly one place,
/// [`WorldState::can_see_mobile`], which is the same choke point `Ghost` uses and
/// the reason a hidden mobile is drawn to nobody without a single draw site knowing
/// what hiding is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Hidden;

/// A hidden mobile that may move without being seen, for a few steps — ServUO's
/// `AllowedStealthSteps`.
///
/// Hiding alone is broken by the first step; Stealth buys `value / 10` of them
/// (pre-AoS), counted down by the movement paths. When they run out the next step
/// breaks cover, which is what makes the skill a budget rather than a switch.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Stealthing {
    /// Steps left before the next one gives you away.
    pub steps_left: u16,
}

/// A healer part way through a bandage — ServUO's `BandageContext`.
///
/// The one skill in the engine whose *duration* is the mechanic: it takes seconds,
/// the patient can be hurt again meanwhile, and it finishes on the tick counter
/// like everything else that waits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Bandaging {
    /// Who is being patched up.
    pub patient: EntityId,
    /// The tick the work is done.
    pub done_at: u64,
}

/// A mobile sitting in a meditative trance — ServUO's `Mobile.Meditating`.
///
/// A marker, not a timer: a trance has no duration and ends when something breaks
/// it, which is any *disruptive* action (the same set that reveals someone hidden).
/// While it holds, mana comes back twice as fast — see the mana regen rate, which
/// reads this at the moment it decides, with nothing folded in and nothing to undo.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Meditating;

/// A spell in progress — the rooted cast delay of the "servuo" cast style. The
/// mobile is committed to `spell` and cannot walk until `complete_at`, the tick
/// the cast resolves; taking damage in the meantime disturbs it if the shard
/// runs with `spell_disturb`. The "sphere" style never sets this — it resolves a
/// cast as it is made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Casting {
    /// The spell being cast, by id.
    pub spell: u16,
    /// The tick the cast finishes and resolves.
    pub complete_at: u64,
}

/// Marks a mobile as run by the server rather than a person: it has a brain.
///
/// The built-in brain, deliberately simple — notice a nearby foe, chase it,
/// swing (through the same `Combat` a player uses); wander when there is nothing
/// to fight. What it *is* is a couple of knobs a script sets at spawn, so an
/// aggressive ogre and a placid deer differ by data, not code. A brain a script
/// drives itself — a per-tick hook, which the scripting benchmark exists to make
/// affordable — is the richer path this leaves room for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Brain {
    /// How far, in tiles, it notices a foe. Zero never picks a fight.
    pub sight: u8,
    /// Whether it drifts around when it has nothing to fight.
    pub wander: bool,
    /// The tick it next gets to act — brains think in beats, not every tick.
    pub next_think: u64,
    /// Standing watch until this tick after a chase found no way through —
    /// the give-up both reference emulators use instead of wall-shuffling.
    /// Zero means not guarding.
    pub guard_until: u64,
    /// Whether it opens a shut door in its way rather than treating it as
    /// wall. Humanoids do; animals do not — ServUO's `CanOpenDoors`.
    pub opens_doors: bool,
    /// Whether it starts fights, only answers them, or only runs.
    pub aggression: Aggression,
    /// Ticks between its beats while hunting; `0` takes the shard's default
    /// (`Gameplay::creature_step_ticks`). Idle, it ambles at twice this.
    pub beat_ticks: u64,
}

/// How a creature relates to the people around it — ServUO's `FightMode`,
/// folded to the three postures that matter: fauna that never fights, the
/// guard-dog that answers force with force, and the monster that starts it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Aggression {
    /// Never fights; runs from whoever hurts it. A deer.
    Passive,
    /// Fights only whoever attacked it first. A guard dog.
    Defensive,
    /// Attacks what it sees first. A monster — and the default, because every
    /// spawn before this knob existed behaved this way.
    #[default]
    Aggressive,
}

impl Aggression {
    /// The wire/config byte: 0 passive, 1 defensive, anything else aggressive.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Passive,
            1 => Self::Defensive,
            _ => Self::Aggressive,
        }
    }

    /// The byte [`from_bits`](Self::from_bits) reads — what a save writes.
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        match self {
            Self::Passive => 0,
            Self::Defensive => 1,
            Self::Aggressive => 2,
        }
    }
}

/// A Magery spellbook's contents: a bit per spell, bit `n` set when the book
/// holds spell `n` (0-based, the same numbering `magic::info` uses). A spellbook
/// is an ordinary item (graphic [`SPELLBOOK_GRAPHIC`]) that also carries this;
/// double-clicking it sends the client the mask (`0xBF 0x1B`), dropping a scroll
/// on it sets a bit, and casting checks one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Spellbook(pub u64);

impl Spellbook {
    /// Whether the book holds spell `n` (0-based).
    #[must_use]
    pub const fn has(self, spell: u8) -> bool {
        spell < SPELL_COUNT && self.0 & (1u64 << spell) != 0
    }

    /// Add spell `n` (0-based); a no-op past the eighth circle.
    pub fn learn(&mut self, spell: u8) {
        if spell < SPELL_COUNT {
            self.0 |= 1u64 << spell;
        }
    }

    /// Every Magery spell — the "full" book the mage sells for testing.
    #[must_use]
    pub const fn full() -> Self {
        Self(u64::MAX) // all 64 bits; the client reads only the first 64 spells
    }
}

/// The 64 Magery spells, first through eighth circle.
pub const SPELL_COUNT: u8 = 64;

/// A Magery spellbook's item graphic.
pub const SPELLBOOK_GRAPHIC: u16 = 0x0EFA;

/// A recall rune's item graphic — ServUO's `RecallRune`.
pub const RECALL_RUNE_GRAPHIC: u16 = 0x1F14;

/// A runebook's item graphic — ServUO's `Runebook`, whose constructor defaults
/// to this id.
pub const RUNEBOOK_GRAPHIC: u16 = 0x22C5;

/// Where a recall rune points, once the Mark spell has written it.
///
/// A rune with no `RuneMark` is a blank one, which is what makes the component's
/// absence the answer to "is this marked" — there is no `marked: bool` to keep
/// honest beside a destination that means nothing when it is false.
///
/// The facet is part of the destination and not a detail: a rune is an object,
/// it can be carried anywhere, and a rune marked in Britain and read in Ilshenar
/// has to still mean Britain.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RuneMark {
    /// Which facet the destination is on.
    pub facet: u8,
    /// The tile the rune was marked on.
    pub destination: Point,
}

/// One destination bound into a [`Runebook`].
///
/// Carries its own description rather than pointing at the rune it came from,
/// because the rune is consumed when it is bound — ServUO deletes it — so there
/// would be nothing left to ask.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RunebookEntry {
    /// Which facet the destination is on.
    pub facet: u8,
    /// The tile bound.
    pub destination: Point,
    /// What to call it in the window — the region's name where there is one.
    pub description: String,
}

/// A book of up to [`RUNEBOOK_ENTRIES`] destinations, and the charges that let it
/// cast to them on its own — ServUO's `Runebook`.
///
/// Not `Copy`, unlike nearly every other component here: it owns its entries and
/// their names. The bus has never required `Copy` — only the enums assumed it —
/// and a component is under no such rule at all.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Runebook {
    /// The destinations bound, in the order they were added.
    pub entries: Vec<RunebookEntry>,
    /// Charges left, each good for one free Recall from the book itself.
    pub charges: u8,
    /// The ceiling recharging fills to — set when the book is made.
    pub max_charges: u8,
    /// Which entry the Recall spell takes when aimed at the book rather than at
    /// a row, if any.
    pub default_entry: Option<u8>,
    /// The tick the book may be opened again — ServUO's `NextUse`.
    ///
    /// Not saved: it is a couple of seconds long, and a restart re-arming it at
    /// zero errs in the generous direction.
    pub next_use: u64,
}

/// How many destinations one runebook holds — ServUO's `Runebook.MaxEntries`.
pub const RUNEBOOK_ENTRIES: usize = 16;

/// The corpse item graphic. A protocol special case: for item `0x2006` the
/// client reads the `Amount` field as the dead body id, so a corpse draws as the
/// creature it was. A corpse is a container (the loot window) that decays.
pub const CORPSE_GRAPHIC: u16 = 0x2006;

/// The gump the client opens for a corpse — the loot window, not a chest.
pub const CORPSE_GUMP: u16 = 0x0009;

/// What a corpse remembers about how it came to be one — ServUO's `Corpse` fields
/// (`Owner`, `Killer`, `m_Forensicist`, `Looters`).
///
/// A corpse is already a container item with a graphic, a name and a decay clock;
/// this is the part only Forensic Evaluation reads, and it is on the corpse rather
/// than in a side table for the reason every other fact about an item is: the item
/// is swept whole by the save, so the story survives a restart with it.
///
/// The killer and the looters are kept as **names**, not serials. ServUO holds live
/// `Mobile` references and reads `.Name` when the corpse is examined, which cannot
/// answer once the killer has logged out — and a corpse outliving its killer's
/// session is the ordinary case, not the corner one.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Corpse {
    /// Who this was.
    pub owner: String,
    /// Who struck the killing blow, if anybody did. An unattributed death (a fall
    /// into a fire field with no caster, say) leaves `None`, which Forensics reads
    /// out as ServUO's "no one".
    pub killer: Option<String>,
    /// The first forensicist to read it, so a second one is told the work is done —
    /// ServUO's `m_Forensicist`, which it sets on the first successful examination.
    pub examined_by: Option<String>,
    /// Everyone who has taken something off it, in the order they did.
    pub looters: Vec<String>,
}

/// The death shroud a fresh ghost wears — item `0x204E` on the outer-torso
/// layer, the grey robe a dead player rises in. ServUO's `deathShroud`.
pub const DEATH_SHROUD_GRAPHIC: u16 = 0x204E;

/// The ghost body a dead player wears — ServUO's `Race.GhostBody`. Female bodies
/// rise as `0x0193`, every other as `0x0192`; the client greys the world once it
/// draws the player in one.
#[must_use]
pub const fn ghost_body(body: u16) -> u16 {
    if body_is_female(body) {
        0x0193
    } else {
        0x0192
    }
}

/// The item graphic of the scroll for a Magery spell, `0-based` — the classic
/// run `0x1F2D..` (Reactive Armor, Clumsy, …), one per spell.
#[must_use]
pub const fn spell_scroll_graphic(spell: u8) -> u16 {
    0x1F2D + spell as u16
}

/// The Magery spell a scroll graphic teaches, if it is a Magery scroll.
#[must_use]
pub const fn scroll_spell(graphic: u16) -> Option<u8> {
    let base = 0x1F2D;
    if graphic >= base && graphic < base + SPELL_COUNT as u16 {
        Some((graphic - base) as u8)
    } else {
        None
    }
}

/// What kind of thing a body is — ServUO's `BodyType`, from `Data/bodyTable.cfg`.
///
/// The table this reads replaced two hand-kept body-id lists (which bodies open doors,
/// which can be ridden). Both were "the safe core of the set" rather than the set, and
/// a list you have to remember to extend is one that goes stale silently.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum BodyType {
    /// Not in the table: ServUO's `BodyType.Empty`, and the default.
    #[default]
    Empty,
    /// A monster — an orc, a lich, a dragon. Has hands, near enough.
    Monster,
    /// A sea creature. Cannot leave the water, cannot work a handle.
    Sea,
    /// An animal. Four legs and no thumbs.
    Animal,
    /// A person.
    Human,
}

/// Every body ServUO's `Data/bodyTable.cfg` gives a type, sorted by id.
///
/// `Equipment` entries are dropped: they are item art, never a mobile. What is left
/// is what a creature can be.
const BODY_TYPES: &[(u16, BodyType)] = &[
    (0x0001, BodyType::Monster),
    (0x0002, BodyType::Monster),
    (0x0003, BodyType::Monster),
    (0x0004, BodyType::Monster),
    (0x0005, BodyType::Animal),
    (0x0006, BodyType::Animal),
    (0x0007, BodyType::Monster),
    (0x0008, BodyType::Monster),
    (0x0009, BodyType::Monster),
    (0x000a, BodyType::Monster),
    (0x000b, BodyType::Monster),
    (0x000c, BodyType::Monster),
    (0x000d, BodyType::Monster),
    (0x000e, BodyType::Monster),
    (0x000f, BodyType::Monster),
    (0x0010, BodyType::Monster),
    (0x0011, BodyType::Monster),
    (0x0012, BodyType::Monster),
    (0x0013, BodyType::Monster),
    (0x0014, BodyType::Monster),
    (0x0015, BodyType::Monster),
    (0x0016, BodyType::Monster),
    (0x0017, BodyType::Animal),
    (0x0018, BodyType::Monster),
    (0x0019, BodyType::Animal),
    (0x001a, BodyType::Monster),
    (0x001b, BodyType::Animal),
    (0x001c, BodyType::Monster),
    (0x001d, BodyType::Animal),
    (0x001e, BodyType::Monster),
    (0x001f, BodyType::Monster),
    (0x0021, BodyType::Monster),
    (0x0022, BodyType::Animal),
    (0x0023, BodyType::Monster),
    (0x0024, BodyType::Monster),
    (0x0025, BodyType::Animal),
    (0x0026, BodyType::Monster),
    (0x0027, BodyType::Monster),
    (0x0028, BodyType::Monster),
    (0x0029, BodyType::Monster),
    (0x002a, BodyType::Monster),
    (0x002b, BodyType::Monster),
    (0x002c, BodyType::Monster),
    (0x002d, BodyType::Monster),
    (0x002e, BodyType::Monster),
    (0x002f, BodyType::Monster),
    (0x0030, BodyType::Monster),
    (0x0031, BodyType::Monster),
    (0x0032, BodyType::Monster),
    (0x0033, BodyType::Monster),
    (0x0034, BodyType::Animal),
    (0x0035, BodyType::Monster),
    (0x0036, BodyType::Monster),
    (0x0037, BodyType::Monster),
    (0x0038, BodyType::Monster),
    (0x0039, BodyType::Monster),
    (0x003a, BodyType::Monster),
    (0x003b, BodyType::Monster),
    (0x003c, BodyType::Monster),
    (0x003d, BodyType::Monster),
    (0x003e, BodyType::Monster),
    (0x003f, BodyType::Animal),
    (0x0040, BodyType::Animal),
    (0x0041, BodyType::Animal),
    (0x0042, BodyType::Monster),
    (0x0043, BodyType::Monster),
    (0x0044, BodyType::Monster),
    (0x0045, BodyType::Monster),
    (0x0046, BodyType::Monster),
    (0x0047, BodyType::Monster),
    (0x0048, BodyType::Monster),
    (0x0049, BodyType::Monster),
    (0x004a, BodyType::Monster),
    (0x004b, BodyType::Monster),
    (0x004c, BodyType::Monster),
    (0x004d, BodyType::Monster),
    (0x004e, BodyType::Monster),
    (0x004f, BodyType::Monster),
    (0x0050, BodyType::Monster),
    (0x0051, BodyType::Animal),
    (0x0052, BodyType::Monster),
    (0x0053, BodyType::Monster),
    (0x0054, BodyType::Monster),
    (0x0055, BodyType::Monster),
    (0x0056, BodyType::Monster),
    (0x0057, BodyType::Monster),
    (0x0058, BodyType::Animal),
    (0x0059, BodyType::Monster),
    (0x005a, BodyType::Monster),
    (0x005b, BodyType::Monster),
    (0x005c, BodyType::Monster),
    (0x005d, BodyType::Monster),
    (0x005e, BodyType::Monster),
    (0x005f, BodyType::Animal),
    (0x0060, BodyType::Monster),
    (0x0061, BodyType::Animal),
    (0x0062, BodyType::Animal),
    (0x0063, BodyType::Animal),
    (0x0064, BodyType::Animal),
    (0x0065, BodyType::Monster),
    (0x0066, BodyType::Monster),
    (0x0067, BodyType::Monster),
    (0x0068, BodyType::Monster),
    (0x006a, BodyType::Monster),
    (0x006b, BodyType::Monster),
    (0x006c, BodyType::Monster),
    (0x006d, BodyType::Monster),
    (0x006e, BodyType::Monster),
    (0x006f, BodyType::Monster),
    (0x0070, BodyType::Monster),
    (0x0071, BodyType::Monster),
    (0x0072, BodyType::Animal),
    (0x0073, BodyType::Animal),
    (0x0074, BodyType::Animal),
    (0x0075, BodyType::Animal),
    (0x0076, BodyType::Animal),
    (0x0077, BodyType::Animal),
    (0x0078, BodyType::Animal),
    (0x0079, BodyType::Animal),
    (0x007a, BodyType::Animal),
    (0x007b, BodyType::Monster),
    (0x007c, BodyType::Monster),
    (0x007d, BodyType::Monster),
    (0x007e, BodyType::Monster),
    (0x007f, BodyType::Animal),
    (0x0080, BodyType::Monster),
    (0x0081, BodyType::Monster),
    (0x0082, BodyType::Monster),
    (0x0083, BodyType::Monster),
    (0x0084, BodyType::Animal),
    (0x0085, BodyType::Animal),
    (0x0086, BodyType::Animal),
    (0x0087, BodyType::Monster),
    (0x0088, BodyType::Monster),
    (0x0089, BodyType::Monster),
    (0x008a, BodyType::Monster),
    (0x008b, BodyType::Monster),
    (0x008c, BodyType::Monster),
    (0x008d, BodyType::Monster),
    (0x008e, BodyType::Monster),
    (0x008f, BodyType::Monster),
    (0x0090, BodyType::Sea),
    (0x0091, BodyType::Sea),
    (0x0092, BodyType::Monster),
    (0x0093, BodyType::Monster),
    (0x0094, BodyType::Monster),
    (0x0095, BodyType::Monster),
    (0x0096, BodyType::Sea),
    (0x0097, BodyType::Sea),
    (0x0098, BodyType::Monster),
    (0x0099, BodyType::Monster),
    (0x009a, BodyType::Monster),
    (0x009b, BodyType::Monster),
    (0x009c, BodyType::Animal),
    (0x009d, BodyType::Monster),
    (0x009e, BodyType::Monster),
    (0x009f, BodyType::Monster),
    (0x00a0, BodyType::Monster),
    (0x00a1, BodyType::Monster),
    (0x00a2, BodyType::Monster),
    (0x00a3, BodyType::Monster),
    (0x00a4, BodyType::Monster),
    (0x00a5, BodyType::Monster),
    (0x00a6, BodyType::Monster),
    (0x00a7, BodyType::Animal),
    (0x00a8, BodyType::Monster),
    (0x00a9, BodyType::Animal),
    (0x00aa, BodyType::Animal),
    (0x00ab, BodyType::Animal),
    (0x00ac, BodyType::Monster),
    (0x00ad, BodyType::Monster),
    (0x00ae, BodyType::Monster),
    (0x00af, BodyType::Monster),
    (0x00b0, BodyType::Monster),
    (0x00b1, BodyType::Animal),
    (0x00b2, BodyType::Animal),
    (0x00b3, BodyType::Animal),
    (0x00b4, BodyType::Monster),
    (0x00b5, BodyType::Monster),
    (0x00b6, BodyType::Monster),
    (0x00b7, BodyType::Human),
    (0x00b8, BodyType::Human),
    (0x00b9, BodyType::Human),
    (0x00ba, BodyType::Human),
    (0x00bb, BodyType::Animal),
    (0x00bc, BodyType::Animal),
    (0x00bd, BodyType::Monster),
    (0x00be, BodyType::Animal),
    (0x00bf, BodyType::Animal),
    (0x00c0, BodyType::Animal),
    (0x00c1, BodyType::Animal),
    (0x00c2, BodyType::Animal),
    (0x00c3, BodyType::Animal),
    (0x00c4, BodyType::Monster),
    (0x00c5, BodyType::Monster),
    (0x00c6, BodyType::Monster),
    (0x00c7, BodyType::Monster),
    (0x00c8, BodyType::Animal),
    (0x00c9, BodyType::Animal),
    (0x00ca, BodyType::Animal),
    (0x00cb, BodyType::Animal),
    (0x00cc, BodyType::Animal),
    (0x00cd, BodyType::Animal),
    (0x00ce, BodyType::Monster),
    (0x00cf, BodyType::Animal),
    (0x00d0, BodyType::Animal),
    (0x00d1, BodyType::Animal),
    (0x00d2, BodyType::Animal),
    (0x00d3, BodyType::Animal),
    (0x00d4, BodyType::Animal),
    (0x00d5, BodyType::Animal),
    (0x00d6, BodyType::Animal),
    (0x00d7, BodyType::Monster),
    (0x00d8, BodyType::Animal),
    (0x00d9, BodyType::Animal),
    (0x00da, BodyType::Animal),
    (0x00db, BodyType::Animal),
    (0x00dc, BodyType::Animal),
    (0x00dd, BodyType::Animal),
    (0x00df, BodyType::Animal),
    (0x00e1, BodyType::Animal),
    (0x00e2, BodyType::Animal),
    (0x00e4, BodyType::Animal),
    (0x00e7, BodyType::Animal),
    (0x00e8, BodyType::Animal),
    (0x00e9, BodyType::Animal),
    (0x00ea, BodyType::Animal),
    (0x00ed, BodyType::Animal),
    (0x00ee, BodyType::Animal),
    (0x00f0, BodyType::Monster),
    (0x00f1, BodyType::Monster),
    (0x00f2, BodyType::Monster),
    (0x00f3, BodyType::Animal),
    (0x00f4, BodyType::Monster),
    (0x00f5, BodyType::Monster),
    (0x00f6, BodyType::Animal),
    (0x00f7, BodyType::Monster),
    (0x00f8, BodyType::Animal),
    (0x00f9, BodyType::Monster),
    (0x00fa, BodyType::Monster),
    (0x00fb, BodyType::Monster),
    (0x00fc, BodyType::Monster),
    (0x00fd, BodyType::Monster),
    (0x00fe, BodyType::Animal),
    (0x00ff, BodyType::Monster),
    (0x0100, BodyType::Monster),
    (0x0101, BodyType::Monster),
    (0x0102, BodyType::Monster),
    (0x0103, BodyType::Monster),
    (0x0104, BodyType::Monster),
    (0x0105, BodyType::Monster),
    (0x0106, BodyType::Monster),
    (0x0107, BodyType::Monster),
    (0x0108, BodyType::Monster),
    (0x0109, BodyType::Monster),
    (0x010a, BodyType::Monster),
    (0x010b, BodyType::Monster),
    (0x010d, BodyType::Monster),
    (0x010e, BodyType::Monster),
    (0x010f, BodyType::Monster),
    (0x0110, BodyType::Monster),
    (0x0111, BodyType::Monster),
    (0x0114, BodyType::Animal),
    (0x0115, BodyType::Animal),
    (0x0116, BodyType::Animal),
    (0x0117, BodyType::Animal),
    (0x0118, BodyType::Monster),
    (0x0119, BodyType::Monster),
    (0x011a, BodyType::Animal),
    (0x011b, BodyType::Animal),
    (0x011c, BodyType::Animal),
    (0x011d, BodyType::Monster),
    (0x011e, BodyType::Monster),
    (0x011f, BodyType::Monster),
    (0x0122, BodyType::Animal),
    (0x0123, BodyType::Animal),
    (0x0124, BodyType::Animal),
    (0x0125, BodyType::Monster),
    (0x012c, BodyType::Monster),
    (0x012d, BodyType::Monster),
    (0x012e, BodyType::Monster),
    (0x012f, BodyType::Monster),
    (0x0130, BodyType::Monster),
    (0x0131, BodyType::Monster),
    (0x0132, BodyType::Monster),
    (0x0133, BodyType::Monster),
    (0x0134, BodyType::Monster),
    (0x0135, BodyType::Monster),
    (0x0136, BodyType::Monster),
    (0x0137, BodyType::Monster),
    (0x0138, BodyType::Monster),
    (0x0139, BodyType::Monster),
    (0x013a, BodyType::Monster),
    (0x013b, BodyType::Monster),
    (0x013c, BodyType::Monster),
    (0x013d, BodyType::Monster),
    (0x013e, BodyType::Monster),
    (0x013f, BodyType::Monster),
    (0x014e, BodyType::Monster),
    (0x0190, BodyType::Human),
    (0x0191, BodyType::Human),
    (0x0192, BodyType::Human),
    (0x0193, BodyType::Human),
    (0x023e, BodyType::Monster),
    (0x025d, BodyType::Human),
    (0x025e, BodyType::Human),
    (0x025f, BodyType::Human),
    (0x0260, BodyType::Human),
    (0x029a, BodyType::Human),
    (0x029b, BodyType::Human),
    (0x02b1, BodyType::Monster),
    (0x02b4, BodyType::Monster),
    (0x02c0, BodyType::Monster),
    (0x02c9, BodyType::Monster),
    (0x02ca, BodyType::Monster),
    (0x02cb, BodyType::Monster),
    (0x02cc, BodyType::Monster),
    (0x02cd, BodyType::Monster),
    (0x02ce, BodyType::Monster),
    (0x02cf, BodyType::Monster),
    (0x02d0, BodyType::Monster),
    (0x02d1, BodyType::Monster),
    (0x02d2, BodyType::Monster),
    (0x02d3, BodyType::Monster),
    (0x02d4, BodyType::Monster),
    (0x02d5, BodyType::Monster),
    (0x02d6, BodyType::Monster),
    (0x02d7, BodyType::Monster),
    (0x02d8, BodyType::Monster),
    (0x02d9, BodyType::Monster),
    (0x02da, BodyType::Monster),
    (0x02dc, BodyType::Monster),
    (0x02dd, BodyType::Monster),
    (0x02de, BodyType::Monster),
    (0x02df, BodyType::Monster),
    (0x02e0, BodyType::Monster),
    (0x02e1, BodyType::Monster),
    (0x02e2, BodyType::Monster),
    (0x02e3, BodyType::Monster),
    (0x02e4, BodyType::Monster),
    (0x02e5, BodyType::Monster),
    (0x02e6, BodyType::Monster),
    (0x02e7, BodyType::Monster),
    (0x02e8, BodyType::Human),
    (0x02e9, BodyType::Human),
    (0x02ea, BodyType::Monster),
    (0x02eb, BodyType::Monster),
    (0x02ec, BodyType::Monster),
    (0x02ed, BodyType::Monster),
    (0x02ee, BodyType::Human),
    (0x02ef, BodyType::Human),
    (0x02f0, BodyType::Monster),
    (0x02f1, BodyType::Monster),
    (0x02f2, BodyType::Monster),
    (0x02f3, BodyType::Monster),
    (0x02f4, BodyType::Monster),
    (0x02f5, BodyType::Monster),
    (0x02f6, BodyType::Monster),
    (0x02fb, BodyType::Monster),
    (0x02fc, BodyType::Monster),
    (0x02fd, BodyType::Monster),
    (0x02fe, BodyType::Monster),
    (0x02ff, BodyType::Monster),
    (0x0300, BodyType::Monster),
    (0x0301, BodyType::Monster),
    (0x0302, BodyType::Monster),
    (0x0303, BodyType::Monster),
    (0x0304, BodyType::Monster),
    (0x0305, BodyType::Monster),
    (0x0306, BodyType::Monster),
    (0x0307, BodyType::Monster),
    (0x0308, BodyType::Monster),
    (0x0309, BodyType::Monster),
    (0x030a, BodyType::Monster),
    (0x030b, BodyType::Monster),
    (0x030c, BodyType::Monster),
    (0x030d, BodyType::Monster),
    (0x030e, BodyType::Monster),
    (0x030f, BodyType::Monster),
    (0x0310, BodyType::Monster),
    (0x0311, BodyType::Monster),
    (0x0312, BodyType::Monster),
    (0x0313, BodyType::Monster),
    (0x0314, BodyType::Monster),
    (0x0315, BodyType::Monster),
    (0x0316, BodyType::Monster),
    (0x0317, BodyType::Animal),
    (0x0318, BodyType::Monster),
    (0x0319, BodyType::Animal),
    (0x031a, BodyType::Animal),
    (0x031b, BodyType::Monster),
    (0x031c, BodyType::Monster),
    (0x031d, BodyType::Monster),
    (0x031e, BodyType::Monster),
    (0x031f, BodyType::Animal),
    (0x0324, BodyType::Monster),
    (0x0325, BodyType::Monster),
    (0x0326, BodyType::Monster),
    (0x0327, BodyType::Monster),
    (0x0328, BodyType::Monster),
    (0x033a, BodyType::Monster),
    (0x033d, BodyType::Monster),
    (0x033e, BodyType::Monster),
    (0x033f, BodyType::Monster),
    (0x0340, BodyType::Monster),
    (0x03db, BodyType::Human),
    (0x03dc, BodyType::Human),
    (0x03de, BodyType::Human),
    (0x03df, BodyType::Human),
    (0x03e2, BodyType::Human),
    (0x03e6, BodyType::Monster),
    (0x03e7, BodyType::Monster),
    (0x0402, BodyType::Monster),
    (0x042c, BodyType::Sea),
    (0x042d, BodyType::Animal),
    (0x04dc, BodyType::Sea),
    (0x04dd, BodyType::Sea),
    (0x04de, BodyType::Monster),
    (0x04df, BodyType::Monster),
    (0x04e0, BodyType::Monster),
    (0x04e5, BodyType::Human),
    (0x04e6, BodyType::Animal),
    (0x04e7, BodyType::Animal),
    (0x0505, BodyType::Animal),
    (0x0506, BodyType::Animal),
    (0x0507, BodyType::Animal),
    (0x0508, BodyType::Animal),
    (0x0509, BodyType::Animal),
    (0x050a, BodyType::Animal),
    (0x050b, BodyType::Animal),
    (0x050c, BodyType::Animal),
    (0x050d, BodyType::Animal),
    (0x050e, BodyType::Animal),
    (0x051c, BodyType::Animal),
    (0x051d, BodyType::Animal),
    (0x0578, BodyType::Animal),
    (0x057a, BodyType::Monster),
    (0x057b, BodyType::Monster),
    (0x057c, BodyType::Monster),
    (0x057d, BodyType::Monster),
    (0x057e, BodyType::Monster),
    (0x057f, BodyType::Animal),
    (0x0580, BodyType::Animal),
    (0x0582, BodyType::Animal),
    (0x0587, BodyType::Animal),
    (0x0588, BodyType::Animal),
    (0x0589, BodyType::Monster),
    (0x058a, BodyType::Monster),
    (0x058b, BodyType::Monster),
    (0x058c, BodyType::Monster),
    (0x058e, BodyType::Monster),
    (0x058f, BodyType::Animal),
    (0x0590, BodyType::Animal),
    (0x0591, BodyType::Animal),
    (0x0592, BodyType::Animal),
    (0x0593, BodyType::Animal),
    (0x0594, BodyType::Monster),
    (0x0597, BodyType::Animal),
    (0x0598, BodyType::Animal),
    (0x0599, BodyType::Monster),
    (0x05a0, BodyType::Animal),
    (0x05a1, BodyType::Animal),
    (0x05c7, BodyType::Monster),
    (0x05cc, BodyType::Monster),
    (0x05cd, BodyType::Monster),
    (0x05e6, BodyType::Monster),
    (0x05e7, BodyType::Monster),
    (0x05e8, BodyType::Monster),
];

const MOUNTS: &[(u16, u16)] = &[
    (0x0074, 0x3ea7),
    (0x0075, 0x3ea8),
    (0x007a, 0x3eb4),
    (0x0084, 0x3ead),
    (0x0090, 0x3eb3),
    (0x00a9, 0x3e95),
    (0x00bb, 0x3eba),
    (0x00bc, 0x3eb8),
    (0x00be, 0x3e9e),
    (0x00c8, 0x3e9f),
    (0x00cc, 0x3ea2),
    (0x00d2, 0x3ea3),
    (0x00da, 0x3ea4),
    (0x00db, 0x3ea5),
    (0x00dc, 0x3ea6),
    (0x00e2, 0x3ea0),
    (0x00e4, 0x3ea1),
    (0x00f3, 0x3e94),
    (0x0114, 0x3e90),
    (0x0115, 0x3e91),
    (0x0317, 0x3ebc),
    (0x0319, 0x3ebb),
    (0x031a, 0x3ebd),
    (0x031f, 0x3ebe),
    (0x057f, 0x3ecb),
    (0x0580, 0x3ecd),
    (0x0582, 0x3ecc),
    (0x05a0, 0x3ecf),
    (0x05a1, 0x3ed0),
    (0x05e6, 0x3ed1),
];

/// The type ServUO gives this body, or [`BodyType::Empty`] for one it does not list.
///
/// A binary search over a sorted table, so it is cheap enough for the tick paths that
/// ask it about every creature in range.
#[must_use]
pub fn body_type(body: u16) -> BodyType {
    match BODY_TYPES.binary_search_by_key(&body, |&(id, _)| id) {
        Ok(index) => BODY_TYPES[index].1,
        Err(_) => BodyType::Empty,
    }
}

/// Whether a body knows what a door handle is.
///
/// ServUO's `BaseCreature.CanOpenDoors`, exactly: `!Body.IsAnimal && !Body.IsSea`. So
/// an orc follows you through a door and a wolf does not, and a body the table does not
/// list is assumed to have hands — which is ServUO's answer too, since an unlisted body
/// is `BodyType.Empty` and neither of the two things the rule excludes.
///
/// This was a list of eight human body ids, described in its own comment as a stand-in
/// "without body-type tables yet". The whole monster half of Britannia was shut out by
/// a closed door it could have opened.
#[must_use]
pub fn body_opens_doors(body: u16) -> bool {
    !matches!(body_type(body), BodyType::Animal | BodyType::Sea)
}

/// The item graphic that draws a body as a mount on a rider, for the bodies that can be
/// ridden at all. `None` is "not rideable", which is what double-click checks first.
///
/// Ported from ServUO's `BaseMount` subclasses — the `base(name, bodyID, itemID, …)`
/// each one passes, plus the alternating body/item arrays a class that rolls between
/// several looks keeps (`Horse` is one of four). Thirty bodies, against the eight the
/// hand-kept list had.
#[must_use]
pub fn mount_item_for(body: u16) -> Option<u16> {
    MOUNTS
        .binary_search_by_key(&body, |&(id, _)| id)
        .ok()
        .map(|index| MOUNTS[index].1)
}

/// The creature body a mount-item graphic stands for — the inverse of
/// [`mount_item_for`]. Persistence saves the worn mount item, not the ridden
/// creature (which lives only while ridden), so restoring a saved ride rebuilds
/// the creature from the item it was drawn as. `None` is "not a mount item".
///
/// Derived from the one [`MOUNTS`] table rather than written out again: two
/// hand-kept halves of one mapping is how a saved ride comes back as the wrong
/// animal.
#[must_use]
pub fn mount_body_for(item_graphic: u16) -> Option<u16> {
    MOUNTS
        .iter()
        .find(|&&(_, item)| item == item_graphic)
        .map(|&(body, _)| body)
}

/// The default name a creature's body gives it — "a chicken", "a horse" —
/// shown on single-click and in the tooltip when a spawn did not name it.
///
/// Creature names are not in any client file the way item names are (those come
/// from tiledata); every emulator holds its own table, ServUO on each
/// `BaseCreature`, Sphere in its chardefs. This is the core default that pack
/// data overrides — the same "default in core, customise in pack" split item
/// names and spells have — so the common Britannia wildlife and dungeon monsters
/// read right out of the box and an unlisted body simply stays nameless rather
/// than wearing a wrong label. Body ids are ServUO's. Expand as needed.
#[must_use]
pub const fn creature_name(body: u16) -> Option<&'static str> {
    Some(match body {
        // Farm and forest animals.
        0x0006 => "a bird",
        0x00C9 => "a cat",
        0x00CA => "an alligator",
        0x00CB => "a pig",
        0x00CD => "a rabbit",
        0x00CF => "a sheep",
        0x00D0 => "a chicken",
        0x00D1 => "a goat",
        0x00D7 => "a giant rat",
        0x00D8 | 0x00E7 => "a cow",
        0x00D9 => "a dog",
        0x00DD => "a walrus",
        0x00EA => "a great hart",
        0x00ED => "a hind",
        0x00EE => "a rat",
        0x0097 => "a dolphin",
        0x0122 => "a boar",
        // Mounts — the stable of [`mount_item_for`].
        0x00C8 | 0x00CC | 0x00E2 | 0x00E4 => "a horse",
        0x00DC => "a llama",
        0x00DB => "a forest ostard",
        0x00D2 => "a desert ostard",
        0x00DA => "a frenzied ostard",
        0x0123 => "a pack horse",
        0x0124 => "a pack llama",
        // Common monsters.
        0x0003 => "a zombie",
        0x0004 => "a gargoyle",
        0x0011 => "an orc",
        0x0012 => "an ettin",
        0x0017 => "a dire wolf",
        0x0019 | 0x001B => "a grey wolf",
        0x001D => "a gorilla",
        0x0023 | 0x0024 => "a lizardman",
        0x002A => "a ratman",
        0x0030 => "a scorpion",
        0x0032 | 0x0038 => "a skeleton",
        0x0034 => "a snake",
        0x0035 | 0x0036 => "a troll",
        0x00A7 => "a brown bear",
        0x00D4 => "a grizzly bear",
        0x00D5 => "a polar bear",
        0x00E1 => "a timber wolf",
        // Undead.
        0x001A => "a spectre",
        0x0018 => "a lich",
        0x004F => "a lich lord",
        0x009A => "a mummy",
        0x0099 => "a ghoul",
        0x0039 => "a bone knight",
        0x0093 => "a skeletal knight",
        0x0094 => "a skeletal mage",
        // Dragons and reptiles.
        0x000C | 0x003B => "a dragon",
        0x003C | 0x003D => "a drake",
        0x003E => "a wyvern",
        0x00B4 | 0x0031 => "a white wyrm",
        0x0096 => "a sea serpent",
        0x0015 => "a giant serpent",
        0x00CE => "a lava lizard",
        // Daemons.
        0x0009 => "a daemon",
        0x004A => "an imp",
        // Elementals.
        0x000F => "a fire elemental",
        0x0010 => "a water elemental",
        0x000D => "an air elemental",
        0x000E => "an earth elemental",
        0x009F => "a blood elemental",
        0x00A3 => "a snow elemental",
        0x00A2 => "a poison elemental",
        // The rest of the common bestiary.
        0x0016 => "a gazer",
        0x001E => "a harpy",
        0x0049 => "a stone harpy",
        0x0001 => "an ogre",
        0x0053 => "an ogre lord",
        0x004B => "a cyclops",
        0x004C => "a titan",
        0x001C => "a giant spider",
        0x009D => "a giant black widow",
        0x002F => "a reaper",
        0x0033 => "a slime",
        0x0007 => "an orc captain",
        0x0046 => "a terathan warrior",
        _ => return None,
    })
}

/// A creature's base sound id — ServUO's `BaseSoundID`, keyed by body like
/// [`creature_name`]. Its attack, hurt and death sounds are fixed offsets from
/// it (`+2`, `+3`, `+4`), so an orc growls and a wolf howls instead of every
/// mobile making the human punch sound. `None` for a human body (which uses the
/// gendered death sounds) and for the passive fauna ServUO leaves silent (a
/// rabbit, a deer). Grow it alongside `creature_name` as bodies are added.
pub const fn creature_base_sound(body: u16) -> Option<u16> {
    Some(match body {
        // Farm and forest animals.
        0x0006 => 0x001B,          // bird
        0x00C9 => 0x0069,          // cat
        0x00CA => 0x0294,          // alligator
        0x00CB | 0x0122 => 0x00C4, // pig, boar
        0x00CF => 0x00D6,          // sheep
        0x00D0 => 0x006E,          // chicken
        0x00D1 => 0x0099,          // goat
        0x00D7 => 0x0188,          // giant rat
        0x00D8 | 0x00E7 => 0x0078, // cow
        0x00D9 => 0x0085,          // dog
        0x00DD => 0x00E0,          // walrus
        0x00EE => 0x00CC,          // rat
        0x0097 => 0x008A,          // dolphin
        // Mounts.
        0x00C8 | 0x00CC | 0x00E2 | 0x00E4 | 0x0123 => 0x00A8, // horse, pack horse
        0x00DC | 0x0124 => 0x03F3,                            // llama, pack llama
        0x00DB | 0x00D2 => 0x0270,                            // forest / desert ostard
        0x00DA => 0x0275,                                     // frenzied ostard
        // Monsters.
        0x0003 => 0x01D7,                            // zombie
        0x0004 => 0x0174,                            // gargoyle
        0x0011 => 0x045A,                            // orc
        0x0012 => 0x016F,                            // ettin
        0x0017 | 0x0019 | 0x001B | 0x00E1 => 0x00E5, // dire / grey / timber wolf
        0x001D => 0x009E,                            // gorilla
        0x0023 | 0x0024 => 0x01A1,                   // lizardman
        0x002A => 0x01B5,                            // ratman
        0x0030 => 0x018D,                            // scorpion
        0x0032 | 0x0038 => 0x048D,                   // skeleton
        0x0034 => 0x00DB,                            // snake
        0x0035 | 0x0036 => 0x01CD,                   // troll
        0x00A7 | 0x00D4 | 0x00D5 => 0x00A3,          // brown / grizzly / polar bear
        // Undead.
        0x001A | 0x0099 => 0x0482,          // spectre / wraith, ghoul
        0x0018 => 0x03E9,                   // lich
        0x004F => 0x019C,                   // lich lord
        0x009A => 0x01D7,                   // mummy
        0x0039 | 0x0093 | 0x0094 => 0x01C3, // bone / skeletal knight and mage
        // Dragons and reptiles — all share the dragon roar.
        0x000C | 0x003B | 0x003C | 0x003D | 0x003E | 0x00B4 | 0x0031 => 0x016A,
        0x0096 => 0x01BF, // sea serpent
        0x0015 => 0x00DB, // giant serpent
        0x00CE => 0x005A, // lava lizard
        // Daemons.
        0x0009 => 0x0165, // daemon
        0x004A => 0x01A6, // imp
        // Elementals.
        0x000F => 0x0346,          // fire
        0x0010 | 0x009F => 0x0116, // water, blood
        0x000D => 0x028F,          // air
        0x000E => 0x010C,          // earth
        0x00A3 | 0x00A2 => 0x0107, // snow, poison
        // The rest of the common bestiary.
        0x0016 => 0x0179,          // gazer
        0x001E | 0x0049 => 0x0192, // harpy, stone harpy
        0x0001 | 0x0053 => 0x01AB, // ogre, ogre lord
        0x004B => 0x025C,          // cyclops
        0x004C => 0x0261,          // titan
        0x001C | 0x009D => 0x0388, // giant spider, giant black widow
        0x002F => 0x01BA,          // reaper
        0x0033 => 0x01C8,          // slime
        0x0007 => 0x045A,          // orc captain (orc sound)
        0x0046 => 0x024D,          // terathan warrior
        _ => return None,
    })
}

/// Whether a body is female — the human death sound splits male from female,
/// ServUO's `m_Female`. The known female bodies: human, elf and gargoyle.
pub const fn body_is_female(body: u16) -> bool {
    matches!(body, 0x0191 | 0x025E | 0x02EF)
}

/// A creature that fights at distance — an archer's bow, a mage's bolt, a
/// dragon's breath, abstracted to what the tick needs: how far it reaches and
/// what kind of hurt it is. The damage amount is the creature's `MeleeDamage`;
/// a ranged creature caught in melee still bites with the same number.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RangedAttack {
    /// How far the attack reaches, in tiles.
    pub range: u8,
    /// The damage type's wire value (see [`DamageType::from_u8`]).
    pub kind: u8,
}

/// Marks a townsperson as a shopkeeper: it answers double-click with a buy
/// gump and "sell" with an offer list. Its goods live in a container worn on
/// its stock layer, priced item by item with [`Price`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Vendor;

/// What a vendor charges per unit for a stock item. Selling pays half.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Price(pub u32);

/// A mobile being ridden: off every screen and every sector, alive in the
/// registry, waiting for the dismount that puts it back on the ground.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Ridden {
    /// Who sits on it.
    pub rider: EntityId,
}

/// A mobile riding a mount.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Riding {
    /// The creature underneath, held out of the world until dismount.
    pub mount: EntityId,
    /// The mount item worn on the mount layer — what the client draws.
    pub item: EntityId,
}

/// The cached route of a chase, followed a step per beat.
///
/// Replanning A* from scratch every beat is what the old brain did, and it is
/// both wasteful and the direct cause of wall-hugging: a plan that fails one
/// beat was retried identically the next. A route is planned once, followed
/// until it goes stale — the quarry moved, the route ran out, or two seconds
/// passed (the references' repath cadence) — and replanned then.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChasePath {
    /// The remaining route, as wire directions (0–7).
    pub steps: Vec<u8>,
    /// The next step to take.
    pub next: usize,
    /// Where the route was aimed; a quarry that strays invalidates it.
    pub goal: Point,
    /// When it was planned, for the repath clock.
    pub planned_at: u64,
}

/// Marks a mobile whose brain is a script's `onTick`, not the built-in one.
///
/// The richer path [`Brain`] leaves room for, now real: the tick's built-in
/// thinking skips a mobile carrying this, and the server calls its `onTick`
/// every tick instead — the per-mobile hook the scripting benchmark sized. A
/// script takes control of a mobile it spawned, then drives it from JavaScript;
/// the built-in `ai` stays out of its way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scripted;

/// Marks a player who has died and walks as a ghost: greyed, silent to the
/// living, waiting on resurrection.
///
/// Only players become ghosts — a creature is reaped into a corpse and gone. The
/// world draws a ghost only to other ghosts and to staff
/// (`WorldState::can_see_mobile`), so the living see an empty tile where a dead
/// player stands. A ghost wears the [`ghost_body`] and a death shroud in place of
/// its living body; resurrection lifts the marker and restores both. The living
/// `body` it rose from is remembered here — the ghost body hides it, and without
/// it a raised player would rise the wrong colour or race, and a relogged one
/// could never be brought back at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ghost {
    /// The living body to restore on resurrection.
    pub body: Body,
}

/// Marks a mobile as a banker: a townsperson who opens your bank box when you ask,
/// and greets those who come near.
///
/// The service, not the person — the graphic, name and standing-still are ordinary
/// mobile data a spawn sets; this is the one bit that makes saying "bank" near it
/// do something. A player within reach of any banker gets their own bank box, the
/// same container the bank holds for them everywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Banker;

/// The trade a townsperson plies, in ServUO's form — "the blacksmith", "the
/// banker". The `Title` beside a `BaseVendor`'s `Name`.
///
/// # Why it is a component and not just part of the name
///
/// It is a *key*. Three separate rules look a townsperson's trade up: the outfit
/// generated at spawn, the personal name put in front of it, and — every time
/// anyone speaks nearby — the keyword table that decides what it answers. A trade
/// that lived only inside the `Name` string would have to be parsed back out of
/// it, and one that lived only in the spawn call that placed it would be lost at
/// the first restart, which is exactly how the quest givers went inert (see
/// `MobileRecord::quest_giver`). So it is saved with the mobile.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Title(pub String);

/// A vendor's shelf as it was first stocked, and when it next refills.
///
/// ServUO's `BaseVendor.Restock`, which tops each `IBuyItemInfo` back up to its
/// original amount on `OnRestock`, checked when the shop is opened
/// (`DelayRestock`, an hour). Without it a bought-out shelf stays bought out for the
/// life of the shard, which is what this engine did.
///
/// The original amounts have to be *remembered*, not recomputed: the crate's live
/// contents are what is left, and there is nothing else to compare them against. So
/// the list is kept whole on the vendor and saved with it — a restock timer that
/// forgot its shelf at every restart would be a slower version of the same bug.
///
/// `at` is a tick count, like [`Decays`] and every other timer here, so a shard's
/// economy replays.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Restock {
    /// The tick at or after which the next shop-open refills the shelf.
    pub at: u64,
    /// What the shelf holds when full.
    pub lines: Vec<StockRecord>,
}

/// One line of a vendor's full shelf, inside a [`Restock`].
///
/// The price and the label are part of it, not just the count: a line that sold out
/// entirely leaves no item behind to copy them from, so a restock that only
/// remembered graphics would put nameless goods back on the shelf at a price of one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StockRecord {
    /// The goods' graphic.
    pub graphic: u16,
    /// Their hue.
    pub hue: u16,
    /// How many the shelf holds when full.
    pub amount: u16,
    /// What one unit costs.
    pub price: u32,
    /// The label the client shows.
    pub name: String,
}

/// Where a townsperson sleeps, for the optional daily routine.
///
/// Read only when [`Gameplay::npc_schedule`](crate::Gameplay::npc_schedule) is on;
/// without it an NPC keeps to its post around the clock, which is what both
/// references do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NightHome(pub Point);

/// A townsperson's AI base — what makes a townsperson *live* rather than stand
/// frozen. The shared part every trade reuses; the trade itself is a [`Title`]
/// beside it, and a service a marker like [`Banker`].
///
/// It keeps to a home: the tile it was placed on, and how far it may drift. A
/// beat every so often lets it greet a passer-by, turn to face them, or take an
/// idle step back toward where it belongs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Npc {
    /// The tile it belongs at — a shop counter, a bank.
    pub home: Point,
    /// How many tiles it may stray from `home`; `0` stands perfectly still.
    pub wander: u8,
    /// The tick it next gets a beat.
    pub next_beat: u64,
    /// The earliest tick it may greet or bark again, so it welcomes rather than
    /// natters. It sat on [`Banker`] while bankers were the only townsfolk that
    /// spoke; every trade greets now, so it belongs on the base.
    pub next_greet: u64,
}

/// A mobile's fighting state: whether it is in war mode, whom it is attacking,
/// and when it may next swing.
///
/// Players carry it from the moment they enter; a creature gets one when it
/// starts fighting (which is an `ai` question, not here). `next_swing` is a tick
/// number, like [`Decays`], so the swing timer is checked against the tick
/// counter and never a clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Combat {
    /// Whether swings are allowed at all.
    pub warmode: bool,
    /// The mobile being attacked, if any.
    pub target: Option<Serial>,
    /// The tick at or after which the next swing may land.
    pub next_swing: u64,
}

/// How hard a mobile hits in melee — the base a swing deals before the target's
/// armour takes its cut.
///
/// A mobile-level number: a creature's natural blow, or a script's pin. A player
/// carries none and derives the blow from the weapon wielded (`combat::melee_blow`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MeleeDamage {
    /// The blow before resistance.
    pub amount: u16,
}

/// A per-item weapon override — the pack's magic sword. Placed on a *weapon item*,
/// its speed and damage replace what the core weapon table gives that graphic
/// (`combat::equipped_weapon` reads it first); the weapon's skill still comes from
/// the base graphic. Era-independent: the same numbers whichever combat era runs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Weapon {
    /// Ticks-formula speed base (Sphere's weapon `base`); higher swings faster.
    pub speed: u16,
    /// Minimum damage before resistance.
    pub min: u16,
    /// Maximum damage before resistance.
    pub max: u16,
}

/// How many steps a mobile has taken — ServUO's `PlayerMobile.StepsTaken`, and
/// only ever read modulo the stride between stamina points (`combat::spend_step_stamina`).
///
/// Not saved: a fresh count after a restart costs a player at most one point of
/// stamina, and a saved one would be a column that means nothing to anything else.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Steps(pub u32);

/// A per-item armour override — the pack's enchanted breastplate. Placed on a
/// *worn armour item*, its rating replaces what the core armour table gives that
/// graphic (`combat::armor` reads it first); where the piece sits on the body
/// still comes from the layer it is worn on. Era-independent, like [`Weapon`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Armor {
    /// The piece's rating before body coverage — ServUO's `ArmorBase`.
    pub rating: u16,
}

/// How many ticks a mobile waits between swings.
///
/// One number stands in for what UO derives from a weapon's speed and the
/// wielder's dexterity — neither of which exists yet (there are no stats, and a
/// weapon has no speed). Making it a component a script sets is the honest
/// halfway house: swing speed is data now, and the derivation slots in later
/// without moving where the number is read.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwingSpeed {
    /// Ticks between blows.
    pub ticks: u64,
}

/// What kind of harm a blow does. Melee is [`Physical`](Self::Physical); a spell
/// picks its element.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum DamageType {
    /// A weapon or a fist.
    #[default]
    Physical,
    /// Fire.
    Fire,
    /// Cold.
    Cold,
    /// Poison.
    Poison,
    /// Energy.
    Energy,
}

impl DamageType {
    /// Read a damage type from a wire byte; anything unknown is physical.
    pub const fn from_u8(byte: u8) -> Self {
        match byte {
            1 => Self::Fire,
            2 => Self::Cold,
            3 => Self::Poison,
            4 => Self::Energy,
            _ => Self::Physical,
        }
    }
}

/// A mobile's armour: how much of each kind of blow it shrugs off, as a
/// percentage. Zero everywhere is no protection.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Resistance {
    /// Percent of physical damage absorbed, 0–100.
    pub physical: u8,
    /// Percent of fire damage absorbed.
    pub fire: u8,
    /// Percent of cold damage absorbed.
    pub cold: u8,
    /// Percent of poison damage absorbed.
    pub poison: u8,
    /// Percent of energy damage absorbed.
    pub energy: u8,
}

impl Resistance {
    /// The percentage that resists `kind` of damage, capped at 100.
    pub fn against(&self, kind: DamageType) -> u8 {
        let value = match kind {
            DamageType::Physical => self.physical,
            DamageType::Fire => self.fire,
            DamageType::Cold => self.cold,
            DamageType::Poison => self.poison,
            DamageType::Energy => self.energy,
        };
        value.min(100)
    }
}

/// A mobile's mana: what casting spends, and how much it can hold.
///
/// The hit-points of magic. A spell that costs more than `current` fizzles; a
/// cast draws it down; it trickles back over time. Only mobiles that cast carry
/// it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Mana {
    /// What it has now.
    pub current: u16,
    /// The most it can have.
    pub max: u16,
}

/// A mobile's stamina: the pool the client reads run-eligibility from, and how
/// much it can hold.
///
/// `max` is dexterity — the UO identity, where the stamina bar *is* dexterity —
/// so a dexterity change re-caps it the way strength re-caps hit points. It
/// trickles back over time like [`Mana`]. Unencumbered foot movement does not
/// spend it in the classic (pre-AoS) era — running is free on open ground — so
/// the pool sits full in normal play; its consumers are combat, being struck,
/// and moving overweight or mounted, which land later. The client refuses to run
/// at zero, so a real pool is what a future push-through mechanic spends against.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Stamina {
    /// What it has now.
    pub current: u16,
    /// The most it can have — dexterity.
    pub max: u16,
}

/// A mobile that can walk: its position, facing, sequence and pace.
///
/// Wraps [`Walker`] rather than replacing [`Position`]: the walk state and the
/// coordinate are asked for by different code at different times, and the tick
/// keeps them in step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Movement(pub Walker);

/// The region a mobile was last seen in — the remembered half of the crossing
/// diff.
///
/// The world does not call "you have left Britain" beside every step. It keeps
/// this, and one pass compares it against the region under the mobile's feet; a
/// difference is the crossing. Same shape as the status bar's snapshot, and for
/// the same reason: a line beside every mutation is the thing that decays the
/// moment a new mover forgets to write it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InRegion {
    /// Which facet's list [`region`](Self::region) indexes.
    ///
    /// An id alone is not an answer. Each facet numbers its own regions from
    /// zero, so region 3 in Felucca and region 3 in Ilshenar compare equal —
    /// and a traveller crossing between them would look to the diff like
    /// somebody who had not moved: no `RegionChanged`, no music, no guards.
    pub facet: u8,
    /// The region's id on that facet, or `None` out in the wilds.
    pub region: Option<u16>,
}

/// A town guard, summoned to execute someone and gone soon after.
///
/// Not a creature with a life: ServUO's guard is a sentence, and this marker is
/// what says so — the tick it vanishes on, and nothing else. There is no target
/// on it because there is no pursuit; a guard strikes in the moment it arrives.
/// A mobile wearing it is also exempt from earning a murder count,
/// because killing the guilty is its whole purpose (ServUO clears the guard's
/// `Criminal`/`Kills` on every beat, which is the same statement).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Guard {
    /// The tick it despawns, its work done.
    pub until: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_table_is_sorted_so_the_search_finds_things() {
        // Both lookups binary-search. An out-of-order entry does not fail loudly — it
        // silently answers `Empty` for a body that is in the table, and an ogre stops
        // opening doors for no visible reason.
        assert!(
            BODY_TYPES.windows(2).all(|w| w[0].0 < w[1].0),
            "BODY_TYPES must be sorted and unique"
        );
        assert!(
            MOUNTS.windows(2).all(|w| w[0].0 < w[1].0),
            "MOUNTS must be sorted and unique"
        );
    }

    #[test]
    fn doors_open_to_hands_and_not_to_paws() {
        // ServUO's `CanOpenDoors`: `!Body.IsAnimal && !Body.IsSea`. The eight-body list
        // this replaced shut out every monster in Britannia — an orc could not follow
        // you through a door it plainly has hands for.
        assert!(body_opens_doors(0x0190), "a man");
        assert!(body_opens_doors(0x0191), "a woman");
        assert!(body_opens_doors(0x0011), "an orc");
        assert!(!body_opens_doors(0x00C9), "a cat");
        assert!(!body_opens_doors(0x00E2), "a horse");
        // An unlisted body is `BodyType::Empty` — neither animal nor sea — so it has
        // hands, which is ServUO's answer too.
        assert_eq!(body_type(0xFFFE), BodyType::Empty);
        assert!(body_opens_doors(0xFFFE));
    }

    #[test]
    fn the_body_types_are_servuos() {
        assert_eq!(body_type(0x0190), BodyType::Human);
        assert_eq!(body_type(0x00E2), BodyType::Animal);
        assert_eq!(body_type(0x0011), BodyType::Monster);
    }

    #[test]
    fn every_horse_colour_is_rideable_and_round_trips() {
        // The hand-kept list had eight mounts; ServUO has thirty, and four of them are
        // the one `Horse` class rolling between colours — which the first scrape missed
        // entirely, because the colours live in an array and not in the constructor.
        for (body, item) in [
            (0x00C8, 0x3E9F),
            (0x00CC, 0x3EA2),
            (0x00E2, 0x3EA0),
            (0x00E4, 0x3EA1),
            (0x00DC, 0x3EA6),
        ] {
            assert_eq!(mount_item_for(body), Some(item), "body {body:#06x}");
            assert_eq!(mount_body_for(item), Some(body), "item {item:#06x}");
        }
        assert_eq!(mount_item_for(0x0190), None, "a person is not a mount");
        assert!(MOUNTS.len() >= 25, "{} mounts", MOUNTS.len());
    }

    #[test]
    fn no_two_mounts_share_one_item_graphic() {
        // `mount_body_for` is the inverse of one table now, and an inverse only exists
        // if the mapping is one to one — otherwise a saved ride comes back as whichever
        // animal the search happened to reach first.
        let mut items: Vec<u16> = MOUNTS.iter().map(|&(_, item)| item).collect();
        items.sort_unstable();
        let before = items.len();
        items.dedup();
        assert_eq!(before, items.len(), "a mount item graphic is used twice");
    }

    use openshard_entities::{Registry, SerialKind};
    use openshard_protocol::Direction;

    #[test]
    fn a_player_and_an_npc_differ_only_by_a_component() {
        // The claim the whole ECS rests on. If this ever needs a `kind` field,
        // something has gone wrong.
        let mut registry = Registry::new();
        let (player, _) = registry.spawn_with_serial(SerialKind::Mobile).unwrap();
        let (npc, _) = registry.spawn_with_serial(SerialKind::Mobile).unwrap();

        for entity in [player, npc] {
            registry.insert(entity, Position(Point::new(100, 100, 0)));
            registry.insert(entity, Body { id: 0x0190, hue: 0 });
        }
        registry.insert(
            player,
            Client {
                connection: ConnectionId::from_raw(1),
                version: ClientVersion::TOL,
            },
        );

        assert!(registry.has::<Client>(player));
        assert!(!registry.has::<Client>(npc), "an NPC has no connection");
        assert_eq!(registry.count::<Position>(), 2, "both are somewhere");
    }

    #[test]
    fn every_sounded_creature_is_also_named() {
        // The two bestiary tables cover the same creatures: a body that growls has
        // a name to show on single-click too. Names may outrun sounds — passive
        // fauna (a rabbit, a deer) are named but silent — but never the reverse.
        for body in 0u16..=0x0400 {
            if creature_base_sound(body).is_some() {
                assert!(
                    creature_name(body).is_some(),
                    "body {body:#06x} sounds like a creature but has no name"
                );
            }
        }
        // Spot-checks of the extended table (ServUO's BaseSoundID), and that a
        // human body is in neither — it falls back to the fists/gendered sounds.
        assert_eq!(creature_base_sound(0x001A), Some(0x0482)); // spectre / wraith
        assert_eq!(creature_base_sound(0x000C), Some(0x016A)); // dragon
        assert_eq!(creature_name(0x0009), Some("a daemon"));
        assert_eq!(
            creature_base_sound(0x0190),
            None,
            "a human is not a creature-sound body"
        );
    }

    #[test]
    fn a_rock_has_a_position_and_no_walk_state() {
        // Most things that have a position never walk. Storing a sequence and a
        // pace budget on every tree would be storage for a question nobody asks.
        let mut registry = Registry::new();
        let (rock, _) = registry.spawn_with_serial(SerialKind::Item).unwrap();
        registry.insert(rock, Position(Point::new(50, 50, 10)));

        assert!(registry.has::<Position>(rock));
        assert!(!registry.has::<Movement>(rock));
    }

    #[test]
    fn a_query_finds_every_mobile_that_can_walk() {
        let mut registry = Registry::new();
        let mut walkers = 0;
        for index in 0..10u16 {
            let (entity, _) = registry.spawn_with_serial(SerialKind::Mobile).unwrap();
            registry.insert(entity, Position(Point::new(index, 0, 0)));
            // Only the even ones move.
            if index % 2 == 0 {
                registry.insert(
                    entity,
                    Movement(Walker::new(
                        Point::new(index, 0, 0),
                        Facing::walking(Direction::North),
                    )),
                );
                walkers += 1;
            }
        }
        assert_eq!(registry.count::<Movement>(), walkers);
        assert_eq!(registry.count::<Position>(), 10);
    }
}
