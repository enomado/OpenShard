//! The world's runtime state: the data a tick reads and writes.
//!
//! [`WorldState`] gathers everything a gameplay system touches — the registry,
//! the event bus, the spatial index, the seeded generator, who is on each
//! client's screen — into one value that lives *below* the systems that act on
//! it. That is what lets a system be a function in its own crate
//! (`combat::swings(&mut WorldState)`) rather than a method on a single
//! ever-growing world object.
//!
//! What is deliberately *not* here: the tick itself, the persistence journal,
//! and the client's map files. Those sit above, in `openshard-world`, which owns
//! a `WorldState` and drives it. This crate knows the shape of world state and
//! nothing about when it changes or how it is saved.

use std::collections::{BTreeMap, HashMap, HashSet};

use openshard_entities::{EntityId, Registry};
use openshard_events::EventBus;
use openshard_gateway::ConnectionId;
use openshard_movement::Terrain;
use openshard_protocol::combat::HealthBar;
use openshard_protocol::feedback::{Animation, NewAnimation, PlaySound};
use openshard_protocol::items::WorldItem;
use openshard_protocol::mobile::{Equipment, MobileIncoming, MobileMove, Notoriety, Remove};
use openshard_protocol::properties::{PropertyList, TooltipRevision};
use openshard_protocol::serial::Serial;
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::wire::SoundId;
use openshard_protocol::world::{PlayerUpdate, Point};
use openshard_protocol::{encode_message, AccessLevel, ClientVersion, Feature};

use crate::components::{
    body_opens_doors, Access, Amount, Body, Client, Contained, Equipped, Facet, Ghost, Graphic,
    Heading, HearsGhosts, Hidden, Hitpoints, Meditating, Movement, Name, Position, Staff,
    Stealthing,
};
use crate::dialogue::Dialogue;
use crate::obstruct::{LiveTerrain, Obstructions};
use crate::quest::QuestDefs;
use crate::region::{Region, Regions};
use crate::rng::Rng;
use crate::sectors::{Sectors, VIEW_RANGE};

/// A character's height above the ground when the facet has no map to ask.
const Z_WITHOUT_A_MAP: i8 = 0;

/// "You stop meditating." — the line a broken trance says, ServUO's 500134.
const STOP_MEDITATING: u32 = 500_134;

/// The hue and font a private system line is drawn in — the client's usual muted
/// grey, so it reads as the server talking rather than as a mobile speaking.
const SYSTEM_HUE: u16 = 0x03B2;
const SYSTEM_FONT: u16 = 3;

/// Ticks in one second — the reciprocal of the world's 50ms tick interval. The
/// world defines the interval; this is the whole-number rate config uses to turn
/// operator-facing seconds into the tick counts timers run on. If one moves, the
/// other must.
pub const TICKS_PER_SECOND: u64 = 20;

/// The gameplay rules an operator tuned, in the form the systems read them: the
/// [`GameplayConfig`](../../openshard_config) knobs, with the second-valued ones
/// already converted to ticks. A plain value the [`WorldState`] carries so any
/// system can reach the number it needs — combat the swing era, chat the speech
/// ranges, items the decay timer — without a config crate below them.
#[derive(Clone, Copy, Debug)]
pub struct Gameplay {
    /// Which swing-speed formula combat uses (Sphere's `CombatEra`, 0–4).
    pub combat_era: u8,
    /// The swing formula's numerator (Sphere's `SpeedScaleFactor`).
    pub speed_scale_factor: u64,
    /// The ceiling any one skill trains to, in tenths — the cap a character's
    /// skills are given when nothing raises one of them.
    pub skill_cap: u16,
    /// The ceiling on *all* skills added together, in tenths — ServUO's
    /// `PlayerCaps.TotalSkillCap`, the classic 700.0. What makes a character a
    /// build rather than a list: past it, one skill only rises if another gives
    /// ground.
    pub total_skill_cap: u32,
    /// The ceiling on the three stats added together — the classic 225.
    pub stat_cap: u16,
    /// The ceiling on any one stat — the classic 125.
    pub stat_cap_individual: u16,
    /// How long after a stat rises before it may rise again, in ticks. ServUO
    /// ships the long delay *off*, leaving half a second.
    pub stat_gain_ticks: u64,
    /// The chance, in per-mille, that a skill gain also tries for a stat — only
    /// the ML mechanic (`combat_era` 4) reads it; the older one rolls each stat's
    /// own weight from the skill table instead.
    pub stat_gain_chance: u32,
    /// How long an item lies on the ground before it rots, in ticks.
    pub decay_ticks: u64,
    /// How long a criminal flag lasts, in ticks.
    pub criminal_ticks: u64,
    /// How far normal speech carries, in tiles.
    pub distance_talk: u32,
    /// How far a whisper carries, in tiles.
    pub distance_whisper: u32,
    /// How far a yell carries, in tiles.
    pub distance_yell: u32,
    /// Ticks between a hunting creature's steps. 8 (0.4s) is the references'
    /// base-monster pace — slower than a running player on purpose; 5 (0.25s)
    /// matches a runner, for shards that want monsters to catch people. Idle
    /// creatures amble at twice this.
    pub creature_step_ticks: u64,
    /// How a spell is cast — Sphere's cast-while-walking, or the UO/ServUO
    /// stop-to-cast with the target after.
    pub cast_style: CastStyle,
    /// Whether taking damage while casting disturbs the spell (UO's fizzle). Only
    /// meaningful in [`CastStyle::Stop`], where there is a cast to disturb.
    pub spell_disturb: bool,
    /// How AoS object tooltips are served — Sphere's `TOOLTIPMODE`, plus an off
    /// gate. Read by the interest substrate to decide what to send when a thing is
    /// drawn, and by the world when the client asks for a full list.
    pub tooltip_mode: TooltipMode,
    /// Whether the server answers a context-menu request with a popup.
    pub context_menus: bool,
    /// Whether spells require and consume reagents at all (classic UO on; a
    /// no-reagent shard off).
    pub reagents: bool,
    /// Whether a failed cast still spends mana — Sphere's `ManaLossFail`. Spent at
    /// resolution once success is known; a successful cast always spends.
    pub mana_loss_on_fail: bool,
    /// Whether a failed cast still consumes reagents — Sphere's `ReagentLossFail`.
    pub reagent_loss_on_fail: bool,
    /// Whether the status bar's gold adds the bank box. Off is ServUO's truth (a
    /// virtual box, whose gold never reaches the character's total); on sums both.
    /// Never affects weight — banked goods are not carried either way.
    pub bank_gold_in_status: bool,
    /// Whether an NPC purchase falls back to the bank when the pack is short —
    /// ServUO's `BaseVendor`, which tries the pack, then the bank.
    pub vendor_bank_payment: bool,
    /// Level-of-detail: when on, a creature with no player within
    /// [`lod_radius`](Self::lod_radius) dozes at a stretched beat instead of
    /// paying for the full AI decision each beat. Off simulates every creature at
    /// full rate. Read by `World::think`.
    pub lod: bool,
    /// How close (tiles, Chebyshev) a player must be for a creature to think at
    /// full rate under [`lod`](Self::lod). Above the view range and the largest
    /// sight, so a visible creature is never dozed.
    pub lod_radius: u32,
    /// How many times its normal beat a dozing creature's next think is pushed
    /// out under [`lod`](Self::lod). At least 1.
    pub lod_idle_factor: u64,
    /// Ticks in one UO minute — how fast the world clock runs. ServUO's five real
    /// seconds to the minute puts a whole UO day in two real hours.
    pub uo_minute_ticks: u64,
    /// The season the client draws: 0 spring, 1 summer, 2 fall, 3 winter, 4
    /// desolation. Static for now; sent on world entry.
    pub season: u8,
    /// Whether guards answer at all in the regions marked guarded — ServUO's
    /// per-region `Disabled`, as one shard-wide switch.
    pub guards: bool,
    /// Whether townsfolk keep a daily routine: at its post inside working hours,
    /// at its `NightHome` outside them.
    ///
    /// Off by default, and deliberately marked as ours: neither reference ties an
    /// NPC to the clock. ServUO's nearest equivalent is a hand-placed `WayPoint`
    /// chain, which a builder walks an NPC along with no notion of the hour. With
    /// no `NightHome` in the pack's data the setting does nothing.
    pub npc_schedule: bool,
    /// The hour townsfolk arrive at their posts, with
    /// [`npc_schedule`](Self::npc_schedule) on.
    pub npc_work_hour: u8,
    /// The hour townsfolk leave for home, with
    /// [`npc_schedule`](Self::npc_schedule) on. Must be after
    /// [`npc_work_hour`](Self::npc_work_hour) — `config` rejects a working day that
    /// wraps midnight, so nothing downstream has to reason about one.
    pub npc_home_hour: u8,
}

/// How AoS object tooltips (the "cliloc" hover names) are served — Sphere's
/// `TOOLTIPMODE`, with an added off state.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TooltipMode {
    /// No tooltips, and AoS is not advertised — a modern client falls back to the
    /// classic single-click name label.
    Off,
    /// Send only a revision (`0xDC`) when a thing is drawn and wait for the client
    /// to request the full list (`0xD6`). Sphere's `TOOLTIPMODE_SENDVERSION`, the
    /// bandwidth-cheap standard.
    #[default]
    SendVersion,
    /// Send the whole tooltip (`0xD6`) up front. Sphere's `TOOLTIPMODE_SENDFULL`.
    SendFull,
}

impl TooltipMode {
    /// Parse the operator's `tooltips` string. `"off"` disables them, `"full"`
    /// sends the whole list up front; anything else is the send-version default.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "false" => Self::Off,
            "full" | "sendfull" => Self::SendFull,
            _ => Self::SendVersion,
        }
    }
}

/// How a spell is cast — the choice both reference emulators make differently.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum CastStyle {
    /// The UO/ServUO original: the caster stops, says the words over a cast
    /// delay, and only then does the target cursor appear (after which it may
    /// move again). Damage during the delay can disturb it.
    #[default]
    Stop,
    /// Sphere's feel: the spell resolves as it is cast, with no rooting delay —
    /// the caster keeps walking, and a target cursor (if any) comes up at once.
    Walk,
}

impl CastStyle {
    /// Parse the operator's `cast_style` string. `"sphere"`/`"walk"` is the
    /// walking cast; anything else is the stop-to-cast default.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "sphere" | "walk" | "walking" => Self::Walk,
            _ => Self::Stop,
        }
    }
}

impl Gameplay {
    /// Seconds, as a count of ticks.
    ///
    /// The operator writes seconds; every system counts ticks, because a tick
    /// count replays and a wall clock does not. One conversion, here, so no
    /// caller has to remember the tick rate.
    #[must_use]
    pub const fn ticks(seconds: u64) -> u64 {
        seconds * TICKS_PER_SECOND
    }

    /// Milliseconds, as a count of ticks — at least one, so a sub-tick interval
    /// still advances.
    #[must_use]
    pub const fn ticks_from_ms(milliseconds: u64) -> u64 {
        let ticks = milliseconds / (1000 / TICKS_PER_SECOND);
        if ticks == 0 {
            1
        } else {
            ticks
        }
    }
}

impl Default for Gameplay {
    /// The pre-AoS feel the systems were built with — the values that were
    /// compile-time constants before an operator could tune them.
    ///
    /// Written as a literal, and the one place the defaults live. This used to be
    /// a twenty-seven-argument `new`, which is how a config knob ends up
    /// positionally next to the wrong one; a caller now names each field it means
    /// to change and takes the rest from here.
    fn default() -> Self {
        Self {
            combat_era: 1,
            speed_scale_factor: 15000,
            skill_cap: 1000,
            total_skill_cap: 7000,
            stat_cap: 225,
            stat_cap_individual: 125,
            // ServUO ships the fifteen-minute delay switched off, which leaves
            // the half second its config falls back to.
            stat_gain_ticks: Self::ticks_from_ms(500),
            stat_gain_chance: 50, // 5%, ServUO's PlayerChanceToGainStats
            decay_ticks: Self::ticks(20 * 60),
            criminal_ticks: Self::ticks(2 * 60),
            distance_talk: 18,
            distance_whisper: 3,
            distance_yell: 31,
            creature_step_ticks: Self::ticks_from_ms(400),
            cast_style: CastStyle::Stop,
            spell_disturb: true,
            tooltip_mode: TooltipMode::SendVersion,
            context_menus: true,
            reagents: true,
            mana_loss_on_fail: true,
            reagent_loss_on_fail: true,
            // The bank is not a second pocket, so its gold is not on the bar.
            bank_gold_in_status: false,
            // But a vendor does fall back to it, as ServUO's does.
            vendor_bank_payment: true,
            lod: false, // opt-in
            lod_radius: 32,
            lod_idle_factor: 8,
            // ServUO's rate: a whole UO day in two real hours.
            uo_minute_ticks: Self::ticks(5),
            season: 0, // spring
            guards: true,
            // Ours, not the references'; opt-in, and inert without pack data.
            npc_schedule: false,
            npc_work_hour: 7,
            npc_home_hour: 21,
        }
    }
}

/// Bytes for a connection, produced by a tick.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outbound {
    /// Who to send to.
    pub connection: ConnectionId,
    /// What to send.
    pub packet: Vec<u8>,
}

/// One facet: its ground, and who is near what on it.
///
/// The world keeps one of these per loaded facet. Two mobiles on different
/// facets never share a sector grid, so they never see each other and never
/// block each other — the isolation is a property of the data structure, not a
/// check anyone has to remember to write.
///
/// The ground is a [`Terrain`] trait object, not a concrete map: this crate sits
/// below the client-file parsers, so it holds the *abstraction* of terrain and
/// the world hands it the real thing (a `MapTerrain`) boxed. A facet with no map
/// carries `None` and every step is allowed.
pub struct FacetState {
    /// The floor, if this facet has a map loaded.
    pub terrain: Option<Box<dyn Terrain + Send + Sync>>,
    /// Who is near what, on this facet.
    pub sectors: Sectors,
    /// What the live world has put in the way: closed doors, placed decoration.
    pub obstructions: Obstructions,
    /// The named areas of this facet — towns, dungeons, guarded zones.
    pub regions: Regions,
}

impl FacetState {
    /// The terrain every movement decision actually checks: the map with the
    /// live obstacles laid over it. Works with no map too — an open world with
    /// doors in it still has doors.
    #[must_use]
    pub fn live_terrain(&self) -> LiveTerrain<'_> {
        LiveTerrain::new(self.terrain.as_deref(), &self.obstructions, false)
    }

    /// The same terrain as a door-opener plans over: closed doors do not block,
    /// because the mobile walking the route opens them on arrival.
    #[must_use]
    pub fn planning_terrain(&self, through_doors: bool) -> LiveTerrain<'_> {
        LiveTerrain::new(self.terrain.as_deref(), &self.obstructions, through_doors)
    }
}

/// An item on a cursor: the entity, and where it was lifted from.
///
/// The origin is the whole reason to remember more than the entity. A drag that
/// is refused — dropped out of reach, into nothing — has to put the item back
/// exactly where it was, and by then it is off the ground (and out of any
/// container) with no place of its own to return to.
#[derive(Clone, Copy, Debug)]
pub struct HeldItem {
    /// The lifted item.
    pub entity: EntityId,
    /// Where it was, so a cancelled drag can undo cleanly.
    pub origin: Origin,
}

impl std::fmt::Debug for FacetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FacetState")
            .field("has_terrain", &self.terrain.is_some())
            .field("sectors", &self.sectors.len())
            .finish()
    }
}

/// Where a held item came from, so a cancelled drag can put it back.
#[derive(Clone, Copy, Debug)]
pub enum Origin {
    /// It was on the ground.
    Ground {
        /// Where it lay.
        position: Point,
        /// On which facet.
        facet: u8,
    },
    /// It was inside a container.
    Container(Contained),
    /// It was worn by a mobile.
    Worn(Equipped),
}

/// The world's runtime state — the data every gameplay system operates on.
///
/// A plain value with public fields: it is a data carrier, not an encapsulation
/// boundary. The boundary that matters is the event bus (systems emit, they do
/// not call), not field privacy. Nothing here is a static; a test builds as many
/// as it likes.
/// The worn-items index behind [`WorldState::equipment_of`] — a cache of which
/// items each mobile has on, and the `Equipped` column version it was built from.
///
/// Not a mirror: nothing maintains it. It is rebuilt whole the first time it is
/// read after the column changes, which the column reports for itself.
#[derive(Debug, Default)]
pub struct WornIndex {
    /// The `Registry::column_version::<Equipped>` this was built from.
    version: u64,
    /// Mobile serial -> the item entities it is wearing.
    by_mobile: HashMap<Serial, Vec<EntityId>>,
}

pub struct WorldState {
    /// Everything in the world.
    pub registry: Registry,
    /// What happened, for anyone to read: the client, persistence, scripts.
    pub bus: EventBus,
    /// The loaded facets, each with its own ground and interest grid, keyed by
    /// facet number. There is always at least the default one.
    pub facets: BTreeMap<u8, FacetState>,
    /// The facet a new character spawns on, and the one anything asking for a
    /// facet it does not have falls back to.
    pub default_facet: u8,
    /// Which entity a connection is driving.
    pub players: HashMap<ConnectionId, EntityId>,
    /// What each player's client currently has on screen.
    ///
    /// The server has to remember, because the client never says. There is no
    /// "what can you see" packet — only "draw this" and "forget that" — so the
    /// only way to send a mobile exactly once is to know what was sent before.
    pub seen: HashMap<EntityId, HashSet<EntityId>>,
    /// The item each connection is dragging on its cursor, and where it was so a
    /// cancelled drag can put it back. An item here is off the ground and out of
    /// everyone's [`seen`](Self::seen) — in limbo until a `0x08` lands it.
    pub held: HashMap<ConnectionId, HeldItem>,
    /// Where new characters appear. The height comes from the map.
    pub start: (u16, u16),
    /// The generator behind every roll — a swing landing, a skill gaining. Part
    /// of the state so replay is exact; advanced only inside the tick.
    pub rng: Rng,
    /// How many ticks have run.
    pub ticks: u64,
    /// Who is wearing what, rebuilt from the `Equipped` column when it changes.
    /// A cache with no contents of its own — read it through
    /// [`equipment_of`](WorldState::equipment_of), never directly.
    pub worn: WornIndex,
    /// The world's hour, 0–23, refreshed once per tick from the tick counter.
    ///
    /// Derived, not stored — `world/tick/ambient.rs` computes it and drops it
    /// here at the top of every tick, the same way `ticks` is the one clock every
    /// system reads. It is state rather than a parameter because more than one
    /// system now asks what time it is (a townsperson's routine, a shop's opening
    /// hours, its greeting), and threading an `hour` argument through each of them
    /// is a signature to keep in step for a value that has exactly one source.
    pub hour: u64,
    /// Packets the last tick produced.
    pub outbox: Vec<Outbound>,
    /// Which connections have each container open, so a change to its contents —
    /// an item consumed as a reagent, one decaying inside — can be pushed to the
    /// clients looking at it. A connection's opens are cleared on logout.
    pub open_containers: HashMap<Serial, HashSet<ConnectionId>>,
    /// Mobiles that have a targeting cursor up, and what the click is for. A `.tele`
    /// raises one; the `0x6C` answer looks here to know what to do with the spot.
    pub pending_targets: HashMap<EntityId, TargetPurpose>,
    /// Every quest this shard knows, as the script pack defined them. Replaced
    /// wholesale on a pack reload, and never persisted — the pack is the truth
    /// about what a quest *is*, every boot; only a player's progress is saved.
    pub quests: QuestDefs,
    /// What every trade says, as the script pack registered it. Replaced wholesale
    /// on a reload and never persisted, for the same reason as
    /// [`quests`](Self::quests): the pack is the truth about content.
    pub dialogue: Dialogue,
    /// Which quest dialog each player has open, and on which page.
    ///
    /// Session state, like [`pending_targets`](Self::pending_targets): a gump
    /// exists only while someone is looking at it, and a reply that arrives for a
    /// window this side never opened is a reply to nothing. Cleared on logout.
    pub open_quest_gumps: HashMap<EntityId, QuestGumpContext>,
    /// The tunable rules — swing era, speech ranges, timers — the systems read.
    pub gameplay: Gameplay,
    /// Set by a staff `.save` to ask the tick for an immediate snapshot. The world
    /// clears it once taken — a request, not the save itself, because taking the
    /// snapshot is the `World`'s to do, not a system's.
    pub save_requested: bool,
}

/// Which page of the quest dialog a player is looking at.
///
/// ServUO's `MondainQuestGump.Section`, and the same one window for all of it: a
/// quest log, an offer, an objectives page and a rewards page are the same frame
/// with a different middle, so they share an id and a reply handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuestSection {
    /// The quest log: every quest in progress, one row each.
    Main,
    /// A quest's prose — the offer's first page, and the log's detail page.
    Description,
    /// What it asks for, with progress.
    Objectives,
    /// What it pays.
    Rewards,
    /// What the giver says when the offer is turned down.
    Refuse,
    /// What the giver says at turn-in.
    Complete,
    /// What the giver says when it is not finished yet.
    InProgress,
    /// What is said when a timer ran out.
    Failed,
}

/// What a player's open quest dialog is showing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestGumpContext {
    /// Which quest, by the pack's key. Empty on the log page, which is about no
    /// single quest.
    pub quest: String,
    /// Which page.
    pub section: QuestSection,
    /// Whether this is an *offer* (Accept/Refuse) rather than the log's view of a
    /// quest already taken (Resign/Close). The same pages, different buttons — and
    /// the difference decides whether a button id means "accept" or "resign", so it
    /// is remembered here rather than trusted from the reply.
    pub offer: bool,
    /// Whether the quest is finished, which is what lets the rewards page pay out.
    pub completed: bool,
    /// The giver the dialog was opened at, so a turn-in knows who to thank.
    pub giver: Option<Serial>,
}

/// What a raised targeting cursor is waiting to do with the click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetPurpose {
    /// Teleport the targeter to the clicked spot — the cursor `.tele`.
    Teleport,
    /// A targeted spell waiting for its aim — the cursor a spell puts up once
    /// the cast resolves. `success` is the skill roll already made, carried here
    /// so a fumbled cast that still raises a cursor simply lands no effect.
    Spell {
        /// Which spell, by id.
        spell: u16,
        /// Whether the cast's skill roll passed.
        success: bool,
    },
    /// A skill waiting for the thing it was pointed at — "whom shall I examine?".
    /// Which skill asked is all that needs remembering; the rest is the skill's own.
    Skill {
        /// Which skill, by id.
        skill: u8,
    },
    /// A skill's *second* cursor: it has one answer and wants another.
    ///
    /// Poisoning is the reason — ServUO asks for the potion, then for the blade —
    /// and it is a separate variant rather than an `Option` on
    /// [`Skill`](Self::Skill) so that the common case stays a skill and one click.
    /// The first answer is carried as an entity and re-checked when the second
    /// lands: a potion drunk or dropped while the cursor was up poisons nothing.
    SkillSecond {
        /// Which skill, by id.
        skill: u8,
        /// What its first cursor came back with.
        first: EntityId,
    },
    /// A staff `.trap` waiting for the container to put a trap on.
    SetTrap {
        /// What the trap will do.
        kind: crate::components::TrapKind,
        /// How hard it hits, and how hard it is to take off.
        power: u16,
    },
    /// A key waiting to be turned on something — ServUO's `Key.OnDoubleClick`, which
    /// raises a cursor rather than guessing which of several nearby doors was meant.
    TurnKey {
        /// The key, by entity. Checked to still exist when the click lands: a key
        /// dropped or consumed while the cursor was up opens nothing.
        key: EntityId,
    },
}

impl WorldState {
    /// Which facet an entity is on: its [`Facet`] component, or the world default
    /// so callers can index [`facets`](Self::facets) with the result.
    #[must_use]
    pub fn facet_of(&self, entity: EntityId) -> u8 {
        self.registry
            .get::<Facet>(entity)
            .map_or(self.default_facet, |facet| facet.0)
    }

    /// The state of a facet the world is known to have.
    #[must_use]
    pub fn facet_state(&self, facet: u8) -> &FacetState {
        &self.facets[&facet]
    }

    /// The same, mutably. Panics only on a facet no entity should carry —
    /// `facet_of` and `enter` keep every live entity on a loaded facet.
    pub fn facet_state_mut(&mut self, facet: u8) -> &mut FacetState {
        self.facets
            .get_mut(&facet)
            .expect("an entity's facet is always loaded")
    }

    /// The region a point on `facet` falls in, if any.
    #[must_use]
    pub fn region_at(&self, facet: u8, point: Point) -> Option<&Region> {
        self.facets.get(&facet)?.regions.at(point)
    }

    /// The region an entity is standing in, if any. The lookup every rule that
    /// asks "is this allowed here" goes through.
    #[must_use]
    pub fn region_of(&self, entity: EntityId) -> Option<&Region> {
        let position = self.registry.get::<Position>(entity)?;
        self.region_at(self.facet_of(entity), position.0)
    }

    /// Where a character appears on `facet`: the configured x and y, at that
    /// facet's height.
    ///
    /// The `z` is read from the map rather than configured. A second source of
    /// truth that disagrees by three units leaves a character unable to take a
    /// single step — every one is more than a two-unit climb — with nothing in
    /// the log to explain it.
    #[must_use]
    pub fn start_position(&self, facet: u8) -> Point {
        let (x, y) = self.start;
        let z = self
            .facets
            .get(&facet)
            .and_then(|state| state.terrain.as_ref())
            .and_then(|terrain| terrain.ground_z(x, y))
            .unwrap_or(Z_WITHOUT_A_MAP);
        Point::new(x, y, z)
    }

    /// Is any connected player within `range` tiles (Chebyshev) of `centre` on
    /// `facet`? Cheap: players are few, so this walks the player table rather than
    /// the sector grid, and stops at the first hit. The primitive level-of-detail
    /// gates a creature's AI on — a creature no player is near need not think.
    #[must_use]
    pub fn any_player_near(&self, centre: Point, range: u32, facet: u8) -> bool {
        self.players.values().any(|&entity| {
            self.facet_of(entity) == facet
                && self
                    .registry
                    .get::<Position>(entity)
                    .is_some_and(|pos| crate::sectors::in_range(pos.0, centre, range))
        })
    }

    /// Take a mobile out of the world: forget it from every screen, drop it from
    /// the sector grid, despawn it.
    ///
    /// The counterpart of the spawn path, and the one place that order is
    /// written down — forgetting *after* the despawn would leave the serial
    /// unresolvable and the mobile drawn on every screen that had it, which is
    /// exactly the "ghost that never leaves" bug.
    pub fn despawn_mobile(&mut self, entity: EntityId) {
        let Some(serial) = self.registry.serial_of(entity) else {
            return;
        };
        let facet = self.facet_of(entity);
        for watcher in self.watchers_of(entity) {
            self.forget(watcher, entity, serial);
        }
        self.seen.remove(&entity);
        self.facet_state_mut(facet).sectors.remove(entity);
        self.registry.despawn(entity);
    }

    /// Everyone who currently has `entity` on their screen — the mobiles whose
    /// `seen` set holds it. The audience for a redraw: a health bar, a change of
    /// colour.
    #[must_use]
    pub fn watchers_of(&self, entity: EntityId) -> Vec<EntityId> {
        self.seen
            .iter()
            .filter(|(watcher, seen)| **watcher != entity && seen.contains(&entity))
            .map(|(watcher, _)| *watcher)
            .collect()
    }

    /// Redraw `entity`'s health bar: the real numbers to itself, a 0–100 scale to
    /// everyone watching. The `0xA1` a blow or a heal sends.
    pub fn broadcast_health(&mut self, entity: EntityId) {
        let Some(&Hitpoints { current, max }) = self.registry.get::<Hitpoints>(entity) else {
            return;
        };
        let Some(serial) = self.registry.serial_of(entity) else {
            return;
        };
        if let Some(&Client {
            connection,
            version,
        }) = self.registry.get::<Client>(entity)
        {
            let exact = ServerPacket::Health(HealthBar::exact(serial, max, current));
            self.outbox.push(Outbound {
                connection,
                packet: exact.encode(version),
            });
        }
        let scaled = ServerPacket::Health(HealthBar::scaled(serial, max, current));
        for watcher in self.watchers_of(entity) {
            if let Some(&Client {
                connection,
                version,
            }) = self.registry.get::<Client>(watcher)
            {
                self.outbox.push(Outbound {
                    connection,
                    packet: scaled.encode(version),
                });
            }
        }
    }

    /// Send one prebuilt, version-independent packet to every player within
    /// view range of `source` — its own client included.
    ///
    /// The audience for a sound or a graphical effect is who is *near*, not the
    /// `seen` set a health redraw uses: a door never enters anyone's `seen` (it is
    /// decoration, redrawn by `reveal`, not tracked as an interest), yet its creak
    /// must still be heard — so this asks the spatial index for neighbours the way
    /// `reveal` does, and keeps the ones with a client. There is no self-vs-others
    /// split: a sound and an effect are the same bytes for everyone, so a caller
    /// builds the packet once and this fans it out. The feedback seam every
    /// gameplay system reaches for — a swing, a spell, a door — so the world is
    /// *felt*, not merely correct.
    pub fn broadcast_from(&mut self, source: EntityId, packet: Vec<u8>) {
        let facet = self.facet_of(source);
        let sectors = &self.facet_state(facet).sectors;
        let Some(centre) = sectors.position_of(source) else {
            return;
        };
        // Collected before the mutation so the sectors borrow is dropped.
        let audience: Vec<EntityId> = sectors
            .nearby(centre, VIEW_RANGE)
            .map(|(id, _)| id)
            .collect();
        for entity in audience {
            if let Some(&Client { connection, .. }) = self.registry.get::<Client>(entity) {
                self.outbox.push(Outbound {
                    connection,
                    packet: packet.clone(),
                });
            }
        }
    }

    /// Play `sound` at `source`'s position, heard by everyone who can see it.
    ///
    /// A no-op for a source with no `Position` (a contained item) — its holder's
    /// tile is where such a sound belongs, and that is the caller's to place. The
    /// `0x54` is placed in 3D so the client attenuates it by distance.
    /// `sound` is still a bare id here, not a [`SoundId`]: the sound *tables* —
    /// spell definitions, creature voices, instrument notes — carry raw numbers
    /// out of config, and the newtype starts where the packet is built. Converting
    /// those tables is its own sweep and would drag serde into the protocol
    /// newtypes; nothing here unwraps a `SoundId`, which is the rule that matters.
    pub fn play_sound(&mut self, source: EntityId, sound: u16) {
        let Some(&Position(at)) = self.registry.get::<Position>(source) else {
            return;
        };
        let packet = ServerPacket::PlaySound(PlaySound {
            sound: SoundId(sound),
            at,
        });
        self.broadcast_packet(source, &packet);
    }

    /// Send `mobile` a private system line — seen by that client and no one else.
    ///
    /// The server talking, not a mobile: it goes out under the system serial in
    /// the client's usual grey, so it reads as feedback rather than as somebody
    /// speaking. A mobile with no client (an NPC, a scripted actor) simply hears
    /// nothing.
    pub fn system_message(&mut self, mobile: EntityId, text: &str) {
        let Some(&Client { connection, .. }) = self.registry.get::<Client>(mobile) else {
            return;
        };
        let packet = encode_message(
            openshard_protocol::SYSTEM_SERIAL,
            openshard_protocol::NO_GRAPHIC,
            0, // regular mode
            SYSTEM_HUE,
            SYSTEM_FONT,
            "System",
            text,
        );
        self.send(connection, packet);
    }

    /// Send `mobile` a private **localized** line — a cliloc the client looks up
    /// in its own translation file and draws.
    ///
    /// The form nearly every stock message takes: a number travels, the player
    /// reads it in their own language, and the shard ships no English. `arguments`
    /// fills the cliloc's `~1_val~` slots, tab-separated, and is usually empty.
    /// A mobile with no client hears nothing, like [`system_message`](Self::system_message).
    pub fn localized_message(&mut self, mobile: EntityId, cliloc: u32, arguments: &str) {
        let Some(&Client { connection, .. }) = self.registry.get::<Client>(mobile) else {
            return;
        };
        let packet = openshard_protocol::encode_localized_message(
            openshard_protocol::SYSTEM_SERIAL,
            openshard_protocol::NO_GRAPHIC,
            0, // regular mode
            SYSTEM_HUE,
            SYSTEM_FONT,
            cliloc,
            "System",
            arguments,
        );
        self.send(connection, packet);
    }

    /// Draw a localized line over `source`'s head, for `watcher` alone.
    ///
    /// ServUO's `PrivateOverheadMessage`: the same `0xC1`, but addressed with the
    /// looked-at thing's serial and graphic rather than the system's, so the text
    /// floats over *it* — and sent to one connection, so a crowded street does not
    /// read everybody's Anatomy check. The whole lore family answers this way.
    pub fn private_overhead_cliloc(
        &mut self,
        watcher: EntityId,
        source: EntityId,
        cliloc: u32,
        arguments: &str,
    ) {
        let Some(&Client { connection, .. }) = self.registry.get::<Client>(watcher) else {
            return;
        };
        let serial = self.registry.serial_of(source).map_or(0, |s| s.raw());
        let graphic = self
            .registry
            .get::<Body>(source)
            .map(|body| body.id)
            .or_else(|| self.registry.get::<Graphic>(source).map(|g| g.id))
            .unwrap_or(openshard_protocol::NO_GRAPHIC);
        let packet = openshard_protocol::encode_localized_message(
            serial,
            graphic,
            0, // regular mode
            SYSTEM_HUE,
            SYSTEM_FONT,
            cliloc,
            "",
            arguments,
        );
        self.send(connection, packet);
    }

    /// Draw `text` over `source` for `watcher` alone — ServUO's
    /// `PrivateOverheadMessage` with a plain string rather than a cliloc.
    ///
    /// The cliloc form ([`private_overhead_cliloc`](Self::private_overhead_cliloc))
    /// is what nearly everything should use, and this is for the one case it cannot
    /// serve: a line whose *content* is a name the client has no number for — Item
    /// Identification saying what an item turned out to be. Ships no English of its
    /// own; the text is a name already in the world.
    pub fn private_overhead_text(&mut self, watcher: EntityId, source: EntityId, text: &str) {
        let Some(&Client { connection, .. }) = self.registry.get::<Client>(watcher) else {
            return;
        };
        let serial = self.registry.serial_of(source).map_or(0, |s| s.raw());
        let graphic = self
            .registry
            .get::<Body>(source)
            .map(|body| body.id)
            .or_else(|| self.registry.get::<Graphic>(source).map(|g| g.id))
            .unwrap_or(openshard_protocol::NO_GRAPHIC);
        let packet = encode_message(serial, graphic, 0, SYSTEM_HUE, SYSTEM_FONT, "", text);
        self.send(connection, packet);
    }

    /// Play `sound` for `mobile` alone — a sound about the player, not about the
    /// world.
    ///
    /// The quest sounds are the reason this exists beside [`play_sound`]: ServUO's
    /// accept, resign, complete and objective-update chimes are feedback on a
    /// dialog only one person is looking at, and broadcasting them would have a
    /// whole street hear a stranger take a quest. The packet is still placed at the
    /// mobile's own tile, so the client does not attenuate it away.
    ///
    /// A no-op for a mobile with no client (an NPC) or no position.
    pub fn play_sound_to(&mut self, mobile: EntityId, sound: u16) {
        let Some(&Client { connection, .. }) = self.registry.get::<Client>(mobile) else {
            return;
        };
        let Some(&Position(at)) = self.registry.get::<Position>(mobile) else {
            return;
        };
        let packet = ServerPacket::PlaySound(PlaySound {
            sound: SoundId(sound),
            at,
        });
        self.send_packet(connection, &packet);
    }

    /// Turn `mobile` to look at `other`, and tell everyone watching.
    ///
    /// Two people talking face each other; ServUO does it with `GetDirectionTo`
    /// before a greeting or a beg. A no-op if either has no position, or if the
    /// mobile is already facing that way — the broadcast is not free.
    pub fn face_toward(&mut self, mobile: EntityId, other: EntityId) {
        let (Some(&Position(from)), Some(&Position(to))) = (
            self.registry.get::<Position>(mobile),
            self.registry.get::<Position>(other),
        ) else {
            return;
        };
        let Some(direction) = openshard_movement::direction_toward(from, to) else {
            return; // standing on the same tile: no way to face
        };
        let facing = openshard_protocol::Facing::walking(direction);
        if self.registry.get::<Heading>(mobile).map(|h| h.0) == Some(facing) {
            return;
        }
        self.registry.insert(mobile, Heading(facing));
        if let Some(Movement(mut walker)) = self.registry.get::<Movement>(mobile).copied() {
            walker.facing = facing;
            self.registry.insert(mobile, Movement(walker));
        }
        self.broadcast_move(mobile);
    }

    /// Animate `mobile` performing `action` — a swing, a death throe, a cast
    /// gesture — for everyone who can see it.
    ///
    /// The wire is per-client, not per-packet: a modern client (7.0.0.0+) gets the
    /// `0xE2` new-animation packet, where the server names a body-agnostic
    /// [`AnimationType`](Action) and the client picks the frames for that body —
    /// which is why a swing needs no body table there. An older client gets the
    /// `0x6E` classic packet, whose action id *is* body-specific, so it is chosen
    /// off a coarse humanoid-vs-creature split (the same `body_opens_doors` line
    /// the door AI uses). The split is deliberately rough: exact per-weapon,
    /// per-body actions want the animation tables the references key off body id,
    /// and the modern path — the one the test clients take — does not need them.
    pub fn animate(&mut self, mobile: EntityId, action: Action) {
        let Some(serial) = self.registry.serial_of(mobile) else {
            return;
        };
        let humanoid = self
            .registry
            .get::<Body>(mobile)
            .is_some_and(|body| body_opens_doors(body.id));
        // Built once each; the per-recipient choice is only which to send.
        let new_packet = ServerPacket::NewAnimation(NewAnimation {
            serial,
            animation_type: action.animation_type(),
            action: 0,
            delay: 0,
        });
        let (old_action, frames) = action.classic_action(humanoid);
        let old_packet = ServerPacket::Animation(Animation {
            serial,
            action: old_action,
            frame_count: frames,
            repeat_count: 1,
            forward: true,
            repeat: false,
            delay: 0,
        });

        for (connection, version) in self.audience_of(mobile) {
            let packet = if version.supports(Feature::NewMobileAnimation) {
                &new_packet
            } else {
                &old_packet
            };
            self.outbox.push(Outbound {
                connection,
                packet: packet.encode(version),
            });
        }
    }
}

/// A mobile action worth animating — the semantic the caller names, which
/// [`WorldState::animate`] turns into the wire animation each client understands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// A melee or ranged swing.
    Attack,
    /// A death throe.
    Die,
    /// A spellcasting gesture.
    Cast,
    /// A bow — what a beggar does before asking, and the one action here that is a
    /// courtesy rather than a blow.
    Bow,
}

impl Action {
    /// The `0xE2` [`AnimationType`](Action) — ServUO's enum: Attack 0, Die 3,
    /// Spell 11, Bow 9. The client maps it to the right frames for whatever body it
    /// is, so no body table is needed on this path.
    const fn animation_type(self) -> u16 {
        match self {
            Self::Attack => 0, // Attack
            Self::Die => 3,    // Die
            Self::Cast => 11,  // Spell
            Self::Bow => 9,    // Bow
        }
    }

    /// The `0x6E` classic action id and frame count, which *are* body-specific.
    /// The humanoid ids are ServUO's people-animation values (Wrestle 31, human
    /// die 21, human directed-cast 16); the creature ids its monster-group values
    /// (attack 4, die 2, cast 12). A coarse split until weapon and body tables
    /// land — good enough for the old 2D client, which is the minority path.
    const fn classic_action(self, humanoid: bool) -> (u16, u16) {
        match (self, humanoid) {
            (Self::Attack, true) => (31, 7), // WeaponAnimation.Wrestle
            (Self::Attack, false) => (4, 4), // monster attack1
            (Self::Die, true) => (21, 6),    // human die
            (Self::Die, false) => (2, 4),    // monster die
            (Self::Cast, true) => (16, 7),   // human directed-cast
            (Self::Cast, false) => (12, 7),  // monster cast
            // Only a person bows; a creature that is asked for money simply looks
            // at you, so the classic path animates nothing body-specific for it.
            (Self::Bow, true) => (32, 5), // human bow
            (Self::Bow, false) => (4, 4), // nothing better on a monster
        }
    }
}

/// Interest management: the machinery that keeps each client's screen in sync
/// with the world — who to draw, who to forget, who to redraw on a move. Shared
/// by every system that changes what a mobile looks like or where it stands.
impl WorldState {
    /// Move a mobile to `to` at once — a teleport, not a walk. Sets its position
    /// everywhere the world tracks it, tells its own client to jump there, and
    /// refreshes what it and everyone around it can see.
    ///
    /// The own-client `0x20` is the part a plain position write forgets: without
    /// it the client keeps drawing its character at the old tile while the new
    /// neighbours appear around where it used to stand — the "teleport did not
    /// refresh" bug. A walk does not need this because the client predicts its own
    /// step; a decree does, because the client was not expecting to move.
    pub fn teleport(&mut self, entity: EntityId, to: Point) {
        let facet = self.facet_of(entity);
        self.registry.insert(entity, Position(to));
        // Keep the walker's own copy in step, or the next walk starts from the old
        // tile.
        if let Some(Movement(mut walker)) = self.registry.get::<Movement>(entity).copied() {
            walker.position = to;
            self.registry.insert(entity, Movement(walker));
        }
        self.facet_state_mut(facet).sectors.insert(entity, to);

        if let Some(&Client { connection, .. }) = self.registry.get::<Client>(entity) {
            let serial = self.registry.serial_of(entity).map_or(0, |s| s.raw());
            let body = self.registry.get::<Body>(entity);
            let facing = self.registry.get::<Heading>(entity).map(|h| h.0);
            if let (Some(body), Some(facing)) = (body, facing) {
                self.send_packet(
                    connection,
                    &ServerPacket::PlayerUpdate(PlayerUpdate {
                        serial,
                        body: body.id,
                        hue: body.hue,
                        flags: 0,
                        position: to,
                        facing,
                    }),
                );
            }
        }
        self.refresh_around(entity);
    }

    /// Bring `entity`'s neighbourhood up to date, both ways.
    ///
    /// Whoever it can see, and whoever can see it. Both, because visibility is
    /// symmetric here and doing one direction leaves the other end with a mobile
    /// that walked away and never left the screen.
    pub fn refresh_around(&mut self, entity: EntityId) {
        // Only this entity's facet: two mobiles on different facets share no
        // sector grid, so a lookup here never turns up anyone on another one.
        let facet = self.facet_of(entity);
        let sectors = &self.facet_state(facet).sectors;
        let Some(centre) = sectors.position_of(entity) else {
            return;
        };

        // A mobile with no client has no screen, and `show` says so on its first
        // line — so for an NPC every one of the two directions below but one is
        // work done to be thrown away. Only "who can see *me*" means anything, and
        // the answer to that is a walk of the players, of whom there are a
        // handful, rather than a sweep of the sector block, which in a decorated
        // town hands back several hundred statics to sift for a few neighbours.
        //
        // This is the difference between an NPC step costing O(everything nearby)
        // and O(players), and almost every step taken in a populated shard is an
        // NPC's.
        if !self.registry.has::<Client>(entity) {
            self.refresh_watchers(entity, centre, facet);
            self.broadcast_move(entity);
            return;
        }

        // Collect first. The lookup borrows the index and the sends borrow `self`
        // mutably, and more importantly a snapshot here is what keeps the set of
        // neighbours from shifting while it is walked. A set and not a `Vec`:
        // it is membership-tested once per remembered entity and once per watcher
        // below, which on a `Vec` is a linear scan inside two loops.
        let neighbours: HashSet<EntityId> = sectors
            .nearby(centre, VIEW_RANGE)
            .map(|(id, _)| id)
            .filter(|id| *id != entity)
            .collect();

        for other in &neighbours {
            self.show(entity, *other);
            self.show(*other, entity);
        }

        // Anything this one used to see and no longer can. `nearby` says who is
        // close; only the remembered set says who *was*.
        let gone: Vec<EntityId> = self
            .seen
            .get(&entity)
            .map(|seen| {
                seen.iter()
                    .filter(|id| !neighbours.contains(id))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for other in gone {
            if let Some(serial) = self.registry.serial_of(other) {
                self.forget(entity, other, serial);
            }
        }

        // And anyone who used to see this one and no longer can.
        for watcher in self.watchers_of(entity) {
            if !neighbours.contains(&watcher) {
                if let Some(serial) = self.registry.serial_of(entity) {
                    self.forget(watcher, entity, serial);
                }
            }
        }

        self.broadcast_move(entity);
    }

    /// The half of [`refresh_around`](Self::refresh_around) that matters for a
    /// mobile with no screen of its own: draw it for the players who can now see
    /// it, and take it off the screens of those who cannot.
    ///
    /// Reached by walking the players, not the sector index — see the caller.
    fn refresh_watchers(&mut self, entity: EntityId, centre: Point, facet: u8) {
        let players: Vec<EntityId> = self.players.values().copied().collect();
        for player in players {
            if player == entity {
                continue;
            }
            let near = self.facet_of(player) == facet
                && self
                    .registry
                    .get::<Position>(player)
                    .is_some_and(|at| crate::sectors::in_range(at.0, centre, VIEW_RANGE));
            if near {
                self.show(player, entity);
            } else if let Some(serial) = self.registry.serial_of(entity) {
                self.forget(player, entity, serial);
            }
        }
    }

    /// Tell everyone already watching `entity` that it moved.
    ///
    /// Only those who already have it: someone seeing it for the first time gets
    /// a `0x78` from [`show`](Self::show), and a `0x77` for a mobile the client
    /// has never heard of is ignored.
    pub fn broadcast_move(&mut self, entity: EntityId) {
        let Some(packet) = self.mobile_move(entity) else {
            return;
        };
        for watcher in self.watchers_of(entity) {
            let Some(&Client {
                connection,
                version,
            }) = self.registry.get::<Client>(watcher)
            else {
                continue;
            };
            self.outbox.push(Outbound {
                connection,
                packet: ServerPacket::MobileMove(packet).encode(version),
            });
        }
    }

    /// Draw `other` for `watcher`, if it is not already on screen.
    pub fn show(&mut self, watcher: EntityId, other: EntityId) {
        // Only players have screens. An NPC "seeing" someone is an AI question,
        // and it does not belong in the packet path.
        let Some(&Client {
            connection,
            version,
        }) = self.registry.get::<Client>(watcher)
        else {
            return;
        };
        if self
            .seen
            .get(&watcher)
            .is_some_and(|seen| seen.contains(&other))
        {
            return;
        }
        // The living cannot see the dead: a ghost is drawn only to another ghost
        // or to staff. Skip it here, before it ever enters `seen`, so a living
        // watcher never has it on screen to move or forget.
        if !self.can_see_mobile(watcher, other) {
            return;
        }
        let Some(packet) = self.draw_packet(other, version) else {
            return;
        };
        self.seen.entry(watcher).or_default().insert(other);
        self.outbox.push(Outbound { connection, packet });
        // The health bar rides along with the draw. There is no "what is its
        // health" packet the client can count on us answering — it opens the bar
        // from what it was last told — so a mobile whose bar is never sent shows an
        // empty frame until the first blow moves it. Send the scaled bar on sight
        // and it reads full from the moment you see it, like every other client.
        if let Some(&Hitpoints { current, max }) = self.registry.get::<Hitpoints>(other) {
            if let Some(serial) = self.registry.serial_of(other) {
                let bar = ServerPacket::Health(HealthBar::scaled(serial, max, current));
                self.outbox.push(Outbound {
                    connection,
                    packet: bar.encode(version),
                });
            }
        }
        // AoS tooltip: the drawn thing's property revision rides along, so the
        // client knows its cached tooltip is stale and can ask for a fresh one.
        if let Some(tooltip) = self.tooltip_packet(other, version) {
            self.outbox.push(Outbound {
                connection,
                packet: tooltip,
            });
        }
    }

    /// The tooltip packet to send *alongside* a draw, or `None` when tooltips are
    /// off, the client is too old for them, or the object has no properties.
    ///
    /// In send-version mode a client new enough for revision hashes ([`0xDC`],
    /// [`Feature::TooltipHash`]) gets just the revision and asks for the list on
    /// hover; an older AoS client, or send-full mode, gets the whole list up front
    /// — it cannot request one it was never told a revision for. Sphere's
    /// `TOOLTIPMODE`.
    fn tooltip_packet(&self, entity: EntityId, version: ClientVersion) -> Option<Vec<u8>> {
        if self.gameplay.tooltip_mode == TooltipMode::Off || !version.supports(Feature::Tooltips) {
            return None;
        }
        let (full, hash) = self.object_properties(entity)?;
        let send_version = self.gameplay.tooltip_mode == TooltipMode::SendVersion
            && version.supports(Feature::TooltipHash);
        if send_version {
            let serial = self.registry.serial_of(entity)?.raw();
            Some(ServerPacket::TooltipRevision(TooltipRevision { serial, hash }).encode(version))
        } else {
            Some(full)
        }
    }

    /// The `0xD6` property list for an object and its revision hash, or `None` for
    /// something with no name to show. Name-only for now: a mobile is cliloc
    /// `1050045` (`~1_PREFIX~~2_NAME~~3_SUFFIX~`) with its [`Name`]; an item is
    /// cliloc `1020000 + graphic` — the client's own tiledata-name range, so no
    /// string is sent — pluralised through cliloc `1050039` when it is a stack.
    /// The item-vs-mobile split is [`draw_packet`](Self::draw_packet)'s, read for
    /// a tooltip rather than a draw. Ported from ServUO's `AddNameProperties` /
    /// `Item.AddNameProperty`.
    #[must_use]
    pub fn object_properties(&self, entity: EntityId) -> Option<(Vec<u8>, u32)> {
        let serial = self.registry.serial_of(entity)?.raw();
        let mut list = PropertyList::new(serial);
        if let Some(Name(name)) = self.registry.get::<Name>(entity) {
            // The earned name — a fame title once the mobile is famous enough for an
            // onlooker to have heard of it. The cliloc is `~1_PREFIX~~2_NAME~~3_SUFFIX~`
            // and ServUO fills the three separately; the title table interleaves a
            // prefix and a suffix around the name in one string, so it goes in the name
            // slot whole and the other two stay empty.
            let name = crate::title::titled_name(self, entity, name);
            list.add_args(1_050_045, &format!(" \t{name}\t "));
        } else if let Some(&Graphic { id, .. }) = self.registry.get::<Graphic>(entity) {
            let cliloc = 1_020_000 + u32::from(id);
            match self.registry.get::<Amount>(entity) {
                Some(Amount(amount)) if *amount > 1 => {
                    list.add_args(1_050_039, &format!("{amount}\t#{cliloc}"));
                }
                _ => list.add(cliloc),
            }
        } else {
            return None;
        }
        Some(list.finish())
    }

    /// Send `entity`'s full `0xD6` property list to one connection — the answer to
    /// a client's tooltip request. Nothing is sent for an object with no name.
    pub fn send_property_list(&mut self, connection: ConnectionId, entity: EntityId) {
        if let Some((packet, _)) = self.object_properties(entity) {
            self.outbox.push(Outbound { connection, packet });
        }
    }

    /// The packet that draws `entity` on a client, or `None` for something not
    /// drawable. A mobile is a `0x78`, an item a `0x1A` — the interest system does
    /// not care which, only that there is one packet per thing on screen.
    #[must_use]
    pub fn draw_packet(&mut self, entity: EntityId, version: ClientVersion) -> Option<Vec<u8>> {
        if self.registry.has::<Body>(entity) {
            let incoming = self.mobile_incoming(entity)?;
            Some(ServerPacket::MobileIncoming(incoming).encode(version))
        } else if self.registry.has::<Graphic>(entity) {
            Some(ServerPacket::WorldItem(self.world_item(entity)?).encode(version))
        } else {
            None
        }
    }

    /// Build a `0x1A` for an entity, if it is a drawable item.
    #[must_use]
    pub fn world_item(&self, entity: EntityId) -> Option<WorldItem> {
        let serial = self.registry.serial_of(entity)?;
        let Graphic { id, hue } = *self.registry.get::<Graphic>(entity)?;
        let Position(position) = *self.registry.get::<Position>(entity)?;
        // No `Amount` means a single. The encoder treats 1 and absent the same.
        let amount = self.registry.get::<Amount>(entity).map_or(1, |a| a.0);
        Some(WorldItem {
            serial: serial.raw(),
            graphic: id,
            amount,
            position,
            hue,
        })
    }

    /// Take `other` off `watcher`'s screen.
    pub fn forget(&mut self, watcher: EntityId, other: EntityId, serial: Serial) {
        if let Some(seen) = self.seen.get_mut(&watcher) {
            if !seen.remove(&other) {
                return;
            }
        } else {
            return;
        }
        if let Some(&Client { connection, .. }) = self.registry.get::<Client>(watcher) {
            self.send_packet(
                connection,
                &ServerPacket::Remove(Remove {
                    serial: serial.raw(),
                }),
            );
        }
    }

    /// Whether `watcher`'s *account* may command — a GameMaster or above.
    ///
    /// The authority half of Sphere's split: `PLEVEL` says who may run a staff
    /// command, and it never moves within a session. Every `.`-command gate reads
    /// this, which is what lets a game master who has turned their staff mode
    /// *off* turn it back on again.
    #[must_use]
    pub fn staff_authority(&self, watcher: EntityId) -> bool {
        self.registry
            .get::<Access>(watcher)
            .is_some_and(|access| access.0 >= AccessLevel::GameMaster)
    }

    /// Whether `watcher` is *acting* as staff right now — the exemptions half.
    ///
    /// Sphere's `PRIV_GM`, which its `.GM` toggles and every in-game rule reads
    /// (`IsPriv(PRIV_GM)`), never the level. Here it is the [`Staff`] marker,
    /// given at login to an account with [`staff_authority`](Self::staff_authority)
    /// and taken off by `.gm`. Staff see the dead and do not tire; a game master
    /// with the mode off walks the world under exactly the rules a player does,
    /// which is the only way to test them from a staff account.
    #[must_use]
    pub fn is_staff(&self, watcher: EntityId) -> bool {
        self.registry.has::<Staff>(watcher)
    }

    /// Whether a mobile may teleport to a point — the `no_teleport` region flag.
    ///
    /// Both ends are checked, not just the destination: a region that bars
    /// teleporting bars it *out* as well as *in*, or a jail is a jail only until
    /// someone inside casts. ServUO's `SpellHelper.CheckTravel` makes the same
    /// two checks for the same reason.
    ///
    /// Staff pass. Every in-game exemption goes through [`is_staff`](Self::is_staff),
    /// so `.gm off` puts a game master under this rule with everyone else.
    #[must_use]
    pub fn may_teleport(&self, mobile: EntityId, to: Point) -> bool {
        if self.is_staff(mobile) {
            return true;
        }
        let facet = self.facet_of(mobile);
        let barred = |point: Option<Point>| {
            point.is_some_and(|point| {
                self.region_at(facet, point)
                    .is_some_and(|region| region.flags.no_teleport)
            })
        };
        let from = self.registry.get::<Position>(mobile).map(|p| p.0);
        !barred(from) && !barred(Some(to))
    }

    /// Whether `watcher` may see mobile `other`. The living cannot see the dead: a
    /// ghost is drawn only to itself, to another ghost, or to staff — ServUO's
    /// `CanSee(Mobile)` (`this == m || m.Alive || !Alive || IsStaff`). Every other
    /// mobile in range is visible to everyone; an item is never a ghost, so this
    /// bites only mobiles.
    ///
    /// It gates *hearing* as well as drawing (`chat::speak`), because ServUO's
    /// speech runs through the same `CanSee`: a ghost nobody can see should not be
    /// a disembodied voice either.
    #[must_use]
    pub fn can_see_mobile(&self, watcher: EntityId, other: EntityId) -> bool {
        if watcher == other {
            return true; // you always see yourself, hidden or dead
        }
        // Hidden is the stricter of the two: nobody sees you but staff.
        if self.registry.has::<Hidden>(other) && !self.is_staff(watcher) {
            return false;
        }
        if !self.registry.has::<Ghost>(other) {
            return true;
        }
        self.registry.has::<Ghost>(watcher) || self.is_staff(watcher)
    }

    /// A mobile did something that gives away where it is — ServUO's
    /// `Mobile.RevealingAction`.
    ///
    /// Attacking, speaking, casting, lifting, dying: the list is ServUO's, and it
    /// also disrupts (`DisruptiveAction` is the last line of `RevealingAction`,
    /// with the comment "anything that unhides you will also disrupt meditation"),
    /// so the two are one call here as they are there.
    ///
    /// Substrate, not a rule, for the same reason [`disrupt`](Self::disrupt) is:
    /// every crate that does something revealing has to be able to say so, and none
    /// of them can depend on the crate that owns Hiding.
    pub fn break_cover(&mut self, mobile: EntityId) {
        self.registry.remove::<Stealthing>(mobile);
        if self.registry.remove::<Hidden>(mobile).is_some() {
            // Back onto every screen in range. `reveal` is the one draw path, so
            // this is the only line that has to know a mobile just became visible.
            self.refresh_around(mobile);
        }
        self.disrupt(mobile);
    }

    /// Take a mobile off every screen but its own — the mirror of
    /// [`break_cover`](Self::break_cover), and the only place a mobile becomes
    /// hidden.
    ///
    /// The marker alone would be enough for anything drawn *after* it, since
    /// `can_see_mobile` gates every draw; what this adds is telling the clients that
    /// already have it on screen to forget it, which is the same `0x1D` a mobile
    /// walking out of range gets.
    pub fn conceal(&mut self, mobile: EntityId) {
        self.registry.insert(mobile, Hidden);
        let Some(serial) = self.registry.serial_of(mobile) else {
            return;
        };
        for watcher in self.watchers_of(mobile) {
            if watcher != mobile && !self.is_staff(watcher) {
                self.forget(watcher, mobile, serial);
            }
        }
    }

    /// A hidden mobile took a step. Spends a stealth step, or gives it away.
    ///
    /// ServUO's `Mobile.OnMove`: running or riding breaks cover outright, and so
    /// does a step past the budget Stealth bought. Called from both movement paths —
    /// there is no shared step, which is why it is called twice and lives here once.
    pub fn step_while_hidden(&mut self, mobile: EntityId, running: bool, mounted: bool) {
        if !self.registry.has::<Hidden>(mobile) || self.is_staff(mobile) {
            return;
        }
        let budget = self
            .registry
            .get::<Stealthing>(mobile)
            .map_or(0, |s| s.steps_left);
        if running || mounted || budget == 0 {
            self.break_cover(mobile);
            return;
        }
        self.registry.insert(
            mobile,
            Stealthing {
                steps_left: budget - 1,
            },
        );
    }

    /// A mobile did something that breaks concentration — ServUO's
    /// `Mobile.DisruptiveAction`.
    ///
    /// Today that means one thing: a meditative trance ends and the mobile is told
    /// so. It is substrate rather than a rule for the same reason `can_see_mobile`
    /// is — every crate that *does* something disruptive has to be able to say so
    /// (a step, a blow taken, a word spoken, an item lifted), and none of them can
    /// depend on the crate that owns Meditation. ServUO calls it from exactly those
    /// places, and this is called from their counterparts here.
    pub fn disrupt(&mut self, mobile: EntityId) {
        if self.registry.remove::<Meditating>(mobile).is_some() {
            self.localized_message(mobile, STOP_MEDITATING, "");
        }
    }

    /// Whether `listener` may *hear* mobile `other` speak.
    ///
    /// Everything anyone can see, they can hear — and one thing more: a living
    /// mobile under Spirit Speak catches what the dead are saying, which is the
    /// whole point of the classic skill. The two questions are deliberately two
    /// predicates: a ghost stays *invisible* to that listener, so `can_see_mobile`
    /// must not be relaxed to cover it, or contacting the netherworld would make
    /// the dead walk visibly among the living.
    #[must_use]
    pub fn can_hear_mobile(&self, listener: EntityId, other: EntityId) -> bool {
        if self.can_see_mobile(listener, other) {
            return true;
        }
        self.registry.has::<Ghost>(other) && self.registry.has::<HearsGhosts>(listener)
    }

    /// A mobile's standing — the colour of its health bar. Absent reads as
    /// [`Notoriety::Innocent`], a blue bar, the safe default.
    #[must_use]
    pub fn notoriety_of(&self, entity: EntityId) -> Notoriety {
        self.registry
            .get::<Notoriety>(entity)
            .copied()
            .unwrap_or(Notoriety::Innocent)
    }

    /// Build a `0x78` for an entity, if it is a drawable mobile.
    #[must_use]
    pub fn mobile_incoming(&mut self, entity: EntityId) -> Option<MobileIncoming> {
        let serial = self.registry.serial_of(entity)?;
        let Position(position) = *self.registry.get::<Position>(entity)?;
        let Heading(facing) = *self.registry.get::<Heading>(entity)?;
        let body = *self.registry.get::<Body>(entity)?;
        Some(MobileIncoming {
            serial: serial.raw(),
            body: body.id,
            position,
            facing,
            hue: body.hue,
            flags: 0,
            notoriety: self.notoriety_of(entity),
            equipment: self.equipment_of(serial),
        })
    }

    /// What a mobile is wearing, as the `0x78` equipment list.
    ///
    /// # Why this keeps an index
    ///
    /// This is called on every *first sight* of a mobile — each `0x78` carries
    /// what its subject is wearing — and the honest version of it filters the
    /// whole `Equipped` column by owner. That is fine until a shard is populated:
    /// 726 dressed townsfolk is a column of ~3,800 rows, scanned in full to find
    /// the five a single NPC has on, once per NPC as a player walks past. One
    /// walk across a market square is millions of comparisons.
    ///
    /// The index is a *cache*, not a mirror. It is keyed on
    /// [`Registry::column_version`], which the column bumps for itself whenever
    /// an entity gains or loses the component, so it rebuilds when it is stale
    /// and nothing anywhere has to remember to invalidate it. That distinction is
    /// the whole design: a hand-maintained "what is worn by whom" map is a
    /// `touch` beside every equip, and the first system that equips something
    /// without knowing the map exists breaks it silently.
    ///
    /// It holds *entities*, not the finished list, so a re-dyed or re-graphicked
    /// item still reads its current `Graphic` here — only membership is cached,
    /// and only membership is what the version tracks.
    #[must_use]
    pub fn equipment_of(&mut self, mobile: Serial) -> Vec<Equipment> {
        let version = self.registry.column_version::<Equipped>();
        if self.worn.version != version {
            self.worn.by_mobile.clear();
            for (item, worn) in self.registry.query::<Equipped>() {
                self.worn
                    .by_mobile
                    .entry(worn.mobile)
                    .or_default()
                    .push(item);
            }
            self.worn.version = version;
        }
        let Some(items) = self.worn.by_mobile.get(&mobile) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|&item| {
                let serial = self.registry.serial_of(item)?;
                let worn = self.registry.get::<Equipped>(item)?;
                let Graphic { id, hue } = *self.registry.get::<Graphic>(item)?;
                Some(Equipment {
                    serial: serial.raw(),
                    graphic: id,
                    layer: worn.layer,
                    hue,
                })
            })
            .collect()
    }

    /// Build a `0x77` for an entity.
    #[must_use]
    pub fn mobile_move(&self, entity: EntityId) -> Option<MobileMove> {
        let serial = self.registry.serial_of(entity)?;
        let Position(position) = *self.registry.get::<Position>(entity)?;
        let Heading(facing) = *self.registry.get::<Heading>(entity)?;
        let body = *self.registry.get::<Body>(entity)?;
        Some(MobileMove {
            serial: serial.raw(),
            body: body.id,
            position,
            facing,
            hue: body.hue,
            flags: 0,
            notoriety: self.notoriety_of(entity),
        })
    }

    /// Queue a raw packet for a connection.
    pub fn send(&mut self, connection: ConnectionId, packet: Vec<u8>) {
        self.outbox.push(Outbound { connection, packet });
    }

    /// The client version negotiated on `connection`.
    ///
    /// `None` for a connection with no player in the world. That is not a stopgap
    /// for "don't know yet": every in-world packet is addressed to a mobile, and a
    /// connection without one has nothing such a packet could tell it — it left,
    /// or it has not entered.
    #[must_use]
    pub fn version_of(&self, connection: ConnectionId) -> Option<ClientVersion> {
        let &player = self.players.get(&connection)?;
        self.registry
            .get::<Client>(player)
            .map(|client| client.version)
    }

    /// Queue `packet` for a connection, framed for the version that connection
    /// negotiated.
    ///
    /// The seam every server-to-client packet should go through: the caller names
    /// *what* to say and this decides *how* to say it to this particular client.
    /// A connection with no player is skipped — see [`version_of`](Self::version_of).
    /// Encoding for a guessed version instead is how a client silently drops a
    /// packet it cannot parse, which is the failure mode that is hardest to see.
    pub fn send_packet(&mut self, connection: ConnectionId, packet: &ServerPacket) {
        let Some(version) = self.version_of(connection) else {
            return;
        };
        let bytes = packet.encode(version);
        self.outbox.push(Outbound {
            connection,
            packet: bytes,
        });
    }

    /// Send `packet` to every player within view range of `source` — its own
    /// client included — each encoded for their own client version.
    ///
    /// The audience for a sound or an effect is who is *near*, not the `seen` set
    /// a health redraw uses: a door never enters anyone's `seen` (it is decoration,
    /// redrawn by `reveal`, not tracked as an interest), yet its creak must still
    /// be heard — so this asks the spatial index for neighbours the way `reveal`
    /// does, and keeps the ones with a client. The feedback seam every gameplay
    /// system reaches for — a swing, a spell, a door — so the world is *felt*, not
    /// merely correct.
    ///
    /// Unlike [`broadcast_from`](Self::broadcast_from) this encodes per recipient
    /// rather than fanning out one buffer, so a packet that grows a
    /// version-conditional tail needs no new call shape: the caller never learns
    /// that the bytes differ.
    pub fn broadcast_packet(&mut self, source: EntityId, packet: &ServerPacket) {
        for (connection, version) in self.audience_of(source) {
            let bytes = packet.encode(version);
            self.outbox.push(Outbound {
                connection,
                packet: bytes,
            });
        }
    }

    /// The clients within view range of `source`, with the version each speaks.
    ///
    /// Collected up front so the sectors borrow is dropped before anything is
    /// queued.
    fn audience_of(&self, source: EntityId) -> Vec<(ConnectionId, ClientVersion)> {
        let facet = self.facet_of(source);
        let sectors = &self.facet_state(facet).sectors;
        let Some(centre) = sectors.position_of(source) else {
            return Vec::new();
        };
        sectors
            .nearby(centre, VIEW_RANGE)
            .filter_map(|(entity, _)| self.registry.get::<Client>(entity))
            .map(|client| (client.connection, client.version))
            .collect()
    }

    /// Draw a newly placed or changed `entity` to everyone in range who does not
    /// already have it — a fresh item, a spawned creature, an equipped mobile.
    pub fn reveal(&mut self, entity: EntityId) {
        let facet = self.facet_of(entity);
        let sectors = &self.facet_state(facet).sectors;
        let Some(centre) = sectors.position_of(entity) else {
            return;
        };
        let watchers: Vec<EntityId> = sectors
            .nearby(centre, VIEW_RANGE)
            .map(|(id, _)| id)
            .filter(|id| *id != entity)
            .collect();
        for watcher in watchers {
            self.show(watcher, entity);
        }
    }
}

impl std::fmt::Debug for WorldState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldState")
            .field("ticks", &self.ticks)
            .field("entities", &self.registry.len())
            .field("players", &self.players.len())
            .field("facets", &self.facets.len())
            .finish()
    }
}
