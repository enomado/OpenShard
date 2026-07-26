//! What each usable skill actually does.
//!
//! One module per family, and one dispatch — [`start`] for the button, [`on_target`]
//! for the cursor's answer. The split is ServUO's `SkillUseCallback` and the
//! `Target` it puts up: pressing the button rarely *does* anything by itself, it
//! asks a question, and the answer arrives a packet later.
//!
//! A skill missing from [`start`] is one whose core behaviour is not built yet:
//! it still passes every gate, still announces
//! [`SkillRequested`](crate::SkillRequested), and a pack can give it meaning. That
//! is the difference between "the core has no opinion" and "the client cannot do
//! this", which is cliloc 500014 and is decided a step earlier.

mod lore;

use openshard_entities::{EntityId, Serial};
use openshard_protocol::encode_target_cursor_object;
use openshard_state::components::{Client, Position};
use openshard_state::{in_range, Skill, TargetPurpose, WorldState};

/// How far a lore cursor reaches, in tiles — ServUO's `Target(8, …)`.
const LORE_RANGE: u32 = 8;

/// Run a skill's core behaviour. Returns whether the core knew what to do with it.
///
/// Called after the gates and after the event, so a pack that wants a skill to
/// mean something else has already seen it and the core is the fallback, not the
/// competition.
pub(crate) fn start(state: &mut WorldState, actor: EntityId, id: u8) -> bool {
    match Skill::from_id(id) {
        Some(Skill::Anatomy | Skill::EvalInt) => {
            raise_cursor(state, actor, id);
            true
        }
        _ => false,
    }
}

/// A skill's cursor came back with something. Runs the skill against it.
pub fn on_target(state: &mut WorldState, actor: EntityId, id: u8, target: u32) {
    let Some(target) = Serial::new(target).and_then(|s| state.registry.entity_of(s)) else {
        return; // the cursor picked bare ground, or something that has since gone
    };
    // Every one of these is a look, and you cannot look across a town. Checked
    // server-side even though the cursor was raised with a range: the range on a
    // `0x6C` is the client's own courtesy, and a client is never the judge of
    // reach — the same rule `ITEM_REACH` holds for a lift.
    if !within(state, actor, target, LORE_RANGE) {
        return;
    }
    match Skill::from_id(id) {
        Some(Skill::Anatomy) => lore::anatomy(state, actor, target),
        Some(Skill::EvalInt) => lore::eval_int(state, actor, target),
        _ => {}
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

/// Put up a cursor that must pick an object, remembering which skill asked.
fn raise_cursor(state: &mut WorldState, actor: EntityId, id: u8) {
    let Some(&Client { connection, .. }) = state.registry.get::<Client>(actor) else {
        return; // a creature has no cursor to raise
    };
    let Some(serial) = state.registry.serial_of(actor) else {
        return;
    };
    state
        .pending_targets
        .insert(actor, TargetPurpose::Skill { skill: id });
    state.send(connection, encode_target_cursor_object(serial.raw()));
}
