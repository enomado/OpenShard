//! The world's runtime state: the data every system reads and writes.
//!
//! # Why this crate exists
//!
//! A gameplay system — combat, chat, skills — is a function over the world's
//! state: it reads components, rolls the world's generator, asks who is near a
//! point, and writes the result back. For those functions to live in their own
//! crates (`combat`, `chat`, …) rather than piling into one file, the state they
//! operate on has to sit *below* them in the dependency graph, in a crate they
//! can all depend on without depending on each other or on the tick that
//! sequences them.
//!
//! That is this crate. It owns the vocabulary of world state and nothing about
//! *when* it changes:
//!
//! - [`components`] — what a thing in the world is made of. Position, hit points,
//!   a combat stance, a skill map; a thing's identity is which of these it
//!   carries.
//! - [`Sectors`] — the spatial index that answers "what is near this point",
//!   Chebyshev distance, the square region a UO client draws.
//! - [`Regions`] — the named areas of a facet: which town or dungeon a point is
//!   in, and what holds there (guards, light, music).
//! - [`skill`] — what the fifty-eight skills are: their client ids, their names,
//!   and the per-skill numbers the check and the gain read.
//! - [`weapon`], [`armor`], [`title`] — the same shape for gear and standing: data
//!   keyed by graphic (or by fame and karma) that more than one system above reads,
//!   so it sits below all of them. `combat` turns a weapon row into a swing and a
//!   blow; `skills` reads the same row to answer an Arms Lore question. The rules
//!   stay in the crate that owns them.
//! - [`Rng`] — the seeded generator behind every roll. Deterministic on purpose:
//!   advanced only by the tick, never the OS, so a world replays roll for roll.
//!
//! The tick that drives all this, and the systems that act on it, live above.

pub mod armor;
pub mod components;
pub mod dialogue;
pub mod obstruct;
pub mod quest;
pub mod region;
pub mod rng;
pub mod runtime;
pub mod sectors;
pub mod skill;
pub mod title;
pub mod weapon;

pub use components::{
    effect, is_debuff, stat_shift, Access, Account, Amount, Banker, BehaviourBuff, BehaviourBuffs,
    Body, BodyType, Brain, Client, Combat, Contained, Container, CriminalUntil, DamageType, Decays,
    Decoration, Door, Equipped, Facet, Fame, Field, FieldKind, Frozen, Ghost, Graphic, Guard,
    Heading, HearsGhosts, Hitpoints, InRegion, Karma, KeyValue, LastStatGain, Lock, Mana,
    Meditating, MeleeDamage, Movement, MurderDecay, Murders, Name, NightHome, Npc, PoisonCharges,
    Poisoned, Position, Resistance, Scripted, SkillCooldown, Skills, SpawnedBy, Stackable, Stamina,
    StatLock, StatLocks, StatMod, StatMods, Stats, SwingSpeed, Title, Trap, TrapKind,
    DEFAULT_SKILL_CAP, EMPTY_BOTTLE_GRAPHIC, FIELD_HEIGHT, POISON_POTION_GRAPHIC,
};
pub use dialogue::{Dialogue, SpeechEntry, SpeechTable};
pub use obstruct::{LiveTerrain, Obstacle, Obstructions, DOOR_HEIGHT};
pub use quest::{ObjectiveDef, ObjectiveKind, QuestDef, QuestDefs, RewardDef, RewardKind};
pub use region::{Region, RegionFlags, RegionRect, Regions};
pub use rng::Rng;
pub use runtime::{
    Action, CastStyle, FacetState, Gameplay, HeldItem, Origin, Outbound, QuestGumpContext,
    QuestSection, TargetPurpose, TooltipMode, WorldState, TICKS_PER_SECOND,
};
pub use sectors::{distance, in_range, Sectors, SECTOR_SIZE, VIEW_RANGE};
pub use skill::{Skill, SkillInfo, StatCode, SKILLS, SKILL_COUNT};
pub use title::{award_fame, award_karma, award_message, compute_title, titled_name};
