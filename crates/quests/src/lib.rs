//! Quests: offering them, tracking them, turning them in, and the window that
//! shows them.
//!
//! # What is here and what is the pack's
//!
//! The *model* is here — what an objective is, how progress moves, when a quest
//! may be offered again, what the log looks like on a client. The *content* is
//! the script pack's: a quest is registered as data (`op_register_quests`) and
//! bound to an NPC as data (`op_bind_quest_giver`), and the engine does the rest.
//! That is the same split `magic::spells` and `combat::weapons` use, and it is
//! why this crate exists at all rather than the pack owning everything.
//!
//! It moved down here for three reasons that only showed up in play:
//!
//! - **There was no quest log.** The paperdoll's Quest button is a packet
//!   (`0xD7`/`0x32`), not a gump reply, and nothing on the pack side can answer
//!   a packet. A player could accept a quest and then had no way to see it.
//! - **A giver bound in script memory stopped being a giver at the first
//!   restart.** Restored NPCs were announced to nobody, so every binding was lost
//!   and the shard's quests worked exactly once. The binding is a saved component
//!   now (`QuestGiver`), so the engine knows without asking.
//! - **The pack could not build the right window even in principle**: no
//!   switches on a gump reply, no way to close a gump, no private message, no
//!   per-player sound. All four now exist; the first three were needed here.
//!
//! # The shape
//!
//! Systems over [`WorldState`], like every other gameplay crate. Nothing calls in
//! here: [`advance_slay`] reads the deaths combat announced, [`refresh_obtain`]
//! *looks* rather than being told (there is no inventory event, and adding one
//! beside every item move is the pattern the persistence rule warns decays), and
//! [`advance_escorts`] asks where an NPC is standing. The gump is drawn from
//! components and answered from what the server remembers drawing.
//!
//! Faithful to ServUO's Mondain's Legacy quest system —
//! `BaseQuest`/`BaseObjective`/`BaseReward`, `MondainQuester`,
//! `MondainQuestGump` — field for field and button for button, so the two can be
//! read side by side.
//!
//! Deferred, on purpose: quest chains, `ApprenticeObjective`, the question-and-
//! answer objective, reward *choice*, and the staff force-complete button.

mod events;
mod gump;
mod log;
mod offer;
mod progress;
mod reply;
mod turnin;

pub use events::{
    QuestAccepted, QuestCompleted, QuestFailed, QuestObjectiveUpdated, QuestRefused, QuestResigned,
};
pub use gump::{QUEST_GUMP, QUEST_RESIGN_GUMP};
pub use log::{
    bind_giver, escort_destination, make_escortable, open_log, open_log_for, speech_offer,
    start_escort,
};
pub use offer::{accept, can_offer, offer, refuse, resign, talk_to, QUEST_LIMIT};
pub use progress::{
    advance_escorts, advance_slay, refresh_obtain, tick_timers, OBTAIN_EVERY_TICKS,
};
pub use reply::{handle, owns};
pub use turnin::{complete, is_complete};
