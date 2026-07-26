//! What each usable skill actually does.
//!
//! One module per family, and one dispatch — [`start`] for the button, [`on_target`]
//! for the cursor's answer. The split is ServUO's `SkillUseCallback` and the
//! `Target` it puts up: pressing the button rarely *does* anything by itself, it
//! asks a question, and the answer arrives a packet later.
//!
//! A skill missing from [`ASKS`] is one whose core behaviour is not built yet:
//! it still passes every gate, still announces
//! [`SkillRequested`](crate::SkillRequested), and a pack can give it meaning. That
//! is the difference between "the core has no opinion" and "the client cannot do
//! this", which is cliloc 500014 and is decided a step earlier.

mod appraise;
mod forensics;
mod lore;
mod mind;
mod poison;

pub use poison::PoisonedSelf;

use openshard_entities::{EntityId, Serial};
use openshard_protocol::encode_target_cursor_object;
use openshard_state::components::{Client, HearsGhosts, Position};
use openshard_state::{in_range, Skill, TargetPurpose, WorldState};

/// A skill that answers a question about something: the line it asks with, and how
/// far the cursor reaches.
///
/// Both numbers are ServUO's, per skill, out of each handler's `OnUse` and the
/// `Target(range, …)` it constructs — they are *not* one shared number. Arms Lore
/// wants the thing in your hands (2), Anatomy across a room (8), Forensic
/// Evaluation as far as you can see a body (10).
struct Ask {
    /// The cliloc the client is prompted with — "Whom shall I examine?".
    prompt: u32,
    /// How far the answer may be, in tiles. Re-checked server-side when it lands.
    range: u32,
}

/// The prompt and reach of every skill that raises an object cursor.
///
/// A skill absent from this table has no core cursor behaviour yet.
#[rustfmt::skip]
const ASKS: &[(Skill, Ask)] = &[
    (Skill::Anatomy,   Ask { prompt: 500_321, range: 8 }),  // Whom shall I examine?
    (Skill::EvalInt,   Ask { prompt: 500_906, range: 8 }),  // What do you wish to evaluate?
    (Skill::ArmsLore,  Ask { prompt: 500_349, range: 2 }),  // What item do you wish to get information about?
    (Skill::ItemId,    Ask { prompt: 500_343, range: 8 }),  // What do you wish to appraise and identify?
    (Skill::Forensics, Ask { prompt: 501_000, range: 10 }), // Show me the crime.
    (Skill::TasteId,   Ask { prompt: 502_807, range: 2 }),  // What would you like to taste?
    (Skill::Poisoning, Ask { prompt: 502_137, range: 2 }),  // Select the poison you wish to use
];

/// The ask for a skill id, if the core raises a cursor for it.
fn ask_for(id: u8) -> Option<&'static Ask> {
    let skill = Skill::from_id(id)?;
    ASKS.iter()
        .find(|(candidate, _)| *candidate == skill)
        .map(|(_, ask)| ask)
}

/// Run a skill's core behaviour. Returns whether the core knew what to do with it.
///
/// Called after the gates and after the event, so a pack that wants a skill to
/// mean something else has already seen it and the core is the fallback, not the
/// competition.
pub(crate) fn start(state: &mut WorldState, actor: EntityId, id: u8) -> bool {
    // A skill that asks a question puts up its cursor and waits; the answer arrives
    // in [`on_target`] a packet later.
    if let Some(ask) = ask_for(id) {
        return raise_cursor(state, actor, id, ask.prompt);
    }
    // And a skill a mobile turns on itself resolves here and now.
    match Skill::from_id(id) {
        Some(Skill::Meditation) => {
            mind::meditation(state, actor);
            true
        }
        Some(Skill::SpiritSpeak) => {
            mind::spirit_speak(state, actor);
            true
        }
        _ => false,
    }
}

/// Let a Spirit Speak contact with the netherworld lapse, telling whoever held it.
///
/// The counterpart of every other expiry the tick runs (`magic::expire_frozen`,
/// `expire_buffs`), on the tick counter for the same reason: a contact measured
/// against a wall clock would not replay.
pub fn expire_ghost_contact(state: &mut WorldState) {
    let now = state.ticks;
    let lapsed: Vec<EntityId> = state
        .registry
        .query::<HearsGhosts>()
        .filter(|(_, contact)| now >= contact.until)
        .map(|(entity, _)| entity)
        .collect();
    for entity in lapsed {
        state.registry.remove::<HearsGhosts>(entity);
        mind::contact_faded(state, entity);
    }
}

/// A skill's cursor came back with something. Runs the skill against it.
pub fn on_target(state: &mut WorldState, actor: EntityId, id: u8, target: u32) {
    let Some(ask) = ask_for(id) else {
        return;
    };
    let Some(target) = Serial::new(target).and_then(|s| state.registry.entity_of(s)) else {
        return; // the cursor picked bare ground, or something that has since gone
    };
    // Checked server-side even though the cursor was raised with a range: the range
    // on a `0x6C` is the client's own courtesy, and a client is never the judge of
    // reach — the same rule `ITEM_REACH` holds for a lift.
    if !within(state, actor, target, ask.range) {
        return;
    }
    match Skill::from_id(id) {
        Some(Skill::Anatomy) => lore::anatomy(state, actor, target),
        Some(Skill::EvalInt) => lore::eval_int(state, actor, target),
        Some(Skill::ArmsLore) => appraise::arms_lore(state, actor, target),
        Some(Skill::ItemId) => appraise::item_id(state, actor, target),
        Some(Skill::Forensics) => forensics::forensics(state, actor, target),
        Some(Skill::TasteId) => poison::taste_id(state, actor, target),
        // Poisoning is the one skill that asks twice: this was the potion, and the
        // next cursor asks what to put it on.
        Some(Skill::Poisoning) => poison::chose_potion(state, actor, target),
        _ => {}
    }
}

/// A skill's *second* cursor came back. Only Poisoning asks twice.
///
/// The reach is the same as the first ask's, and re-checked here for the same
/// reason: a `0x6C` range is the client's courtesy, never the judge.
pub fn on_second_target(
    state: &mut WorldState,
    actor: EntityId,
    id: u8,
    first: EntityId,
    target: u32,
) {
    let Some(ask) = ask_for(id) else {
        return;
    };
    let Some(target) = Serial::new(target).and_then(|s| state.registry.entity_of(s)) else {
        return;
    };
    if !within(state, actor, target, ask.range) {
        return;
    }
    if Skill::from_id(id) == Some(Skill::Poisoning) {
        poison::apply_to(state, actor, first, target);
    }
}

/// Whether two things are on the same facet and within `range` tiles.
fn within(state: &WorldState, a: EntityId, b: EntityId, range: u32) -> bool {
    let (Some(&Position(at)), Some(&Position(other))) = (
        state.registry.get::<Position>(a),
        state.registry.get::<Position>(b),
    ) else {
        return false;
    };
    state.facet_of(a) == state.facet_of(b) && in_range(at, other, range)
}

/// Put up a cursor that must pick an object, remembering which skill asked, and
/// prompt the asker with the skill's own line. Returns whether a cursor went up —
/// a creature has none, and a skill that cannot ask has not started.
fn raise_cursor(state: &mut WorldState, actor: EntityId, id: u8, prompt: u32) -> bool {
    let Some(&Client { connection, .. }) = state.registry.get::<Client>(actor) else {
        return false; // a creature has no cursor to raise
    };
    let Some(serial) = state.registry.serial_of(actor) else {
        return false;
    };
    state
        .pending_targets
        .insert(actor, TargetPurpose::Skill { skill: id });
    state.localized_message(actor, prompt, "");
    state.send(connection, encode_target_cursor_object(serial.raw()));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ask_is_a_skill_the_window_can_press() {
        // A cursor for a skill the button refuses would never be raised: the gate
        // runs first. So each row here must be a skill ServUO marks usable.
        for (skill, _) in ASKS {
            let info = openshard_state::skill::info(skill.id()).expect("a real skill");
            assert!(info.usable, "{} is not usable from the window", info.name);
        }
    }

    #[test]
    fn no_skill_asks_twice() {
        for (i, (skill, _)) in ASKS.iter().enumerate() {
            for (other, _) in &ASKS[i + 1..] {
                assert_ne!(skill.id(), other.id(), "{skill:?} appears twice");
            }
        }
    }
}
