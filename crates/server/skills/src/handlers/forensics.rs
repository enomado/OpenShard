//! Forensic Evaluation: reading a body for how it got that way.
//!
//! ServUO's `ForensicEvaluation`. The skill that needs the world to have kept
//! notes — who struck the killing blow, who has been through the pockets since —
//! which is why laying a corpse now writes a
//! [`Corpse`](openshard_state::components::Corpse) and lifting something out of one
//! adds a looter. Everything this reads was recorded by somebody else's rule at the
//! moment it happened; nothing here reconstructs anything.

use openshard_entities::EntityId;
use openshard_state::components::{body_type, Amount, BodyType, Corpse};
use openshard_state::{Skill, WorldState};

use crate::check::roll_skill_band;

/// "You notice nothing unusual." — the answer to a body read by someone with too
/// little skill to read one at all.
const NOTHING_UNUSUAL: u32 = 501_003;
/// "You cannot determine anything useful." — the failed roll.
const NOTHING_USEFUL: u32 = 501_001;
/// "The corpse has not been desecrated."
const NOT_DESECRATED: u32 = 501_002;
/// "The forensicist ~1_NAME~ has already discovered that:"
const ALREADY_READ: u32 = 1_042_750;
/// "This person was killed by ~1_KILLER_NAME~"
const KILLED_BY: u32 = 1_042_751;
/// "This body has been disturbed by ~1_PLAYER_NAMES~"
const DISTURBED_BY: u32 = 1_042_752;
/// What ServUO reads out when nobody is on record as the killer.
const NO_ONE: &str = "no one";

/// The skill a body says nothing at all below, in tenths — ServUO's `minSkill`.
const CORPSE_MIN: u16 = 300;
/// The band a corpse is read against, in tenths.
const CORPSE_BAND: (i32, i32) = (300, 550);
/// The skill a living body says nothing at all below, in tenths.
const LIVING_MIN: u16 = 360;
/// The band a living body is read against.
const LIVING_BAND: (i32, i32) = (360, 1000);

/// Read what a target has to say about a crime.
pub(super) fn forensics(state: &mut WorldState, actor: EntityId, target: EntityId) {
    let id = Skill::Forensics.id();
    let skill = crate::skill_value(state, actor, id);
    if state.registry.has::<Corpse>(target) {
        read_corpse(state, actor, target, skill);
    } else if state
        .registry
        .has::<openshard_state::components::Body>(target)
    {
        read_the_living(state, actor, skill);
    } else {
        // An item that is not a corpse. ServUO reads a picked lock here and a few
        // Stygian-Abyss trinkets; the lock's picker is recorded by Lockpicking,
        // which does not exist yet, so there is nothing on an ordinary crate to
        // find.
        state.localized_message(actor, NOTHING_UNUSUAL, "");
    }
}

/// A corpse: who it was killed by, and who has been through it since.
fn read_corpse(state: &mut WorldState, actor: EntityId, corpse: EntityId, skill: u16) {
    if skill < CORPSE_MIN {
        state.localized_message(actor, NOTHING_UNUSUAL, "");
        return;
    }
    let id = Skill::Forensics.id();
    if !roll_skill_band(state, actor, id, CORPSE_BAND.0, CORPSE_BAND.1) {
        state.localized_message(actor, NOTHING_USEFUL, "");
        return;
    }
    let Some(story) = state.registry.get::<Corpse>(corpse).cloned() else {
        return;
    };
    // A second reader is told whose work they are repeating; the first puts their
    // own name on it. ServUO sets `m_Forensicist` on the first success and never
    // clears it.
    match &story.examined_by {
        Some(first) => state.localized_message(actor, ALREADY_READ, first),
        None => {
            let name = state
                .registry
                .get::<openshard_state::components::Name>(actor)
                .map(|n| n.0.clone());
            if let Some(name) = name {
                let mut updated = story.clone();
                updated.examined_by = Some(name);
                state.registry.insert(corpse, updated);
            }
        }
    }
    // Only a person can be said to have been killed by somebody. A corpse draws as
    // the body it was — `Amount` on the corpse item is the dead body id, the same
    // field the client reads to draw it — so that is what decides.
    if corpse_was_human(state, corpse) {
        let killer = story.killer.clone().unwrap_or_else(|| NO_ONE.to_owned());
        state.localized_message(actor, KILLED_BY, &killer);
    }
    if story.looters.is_empty() {
        state.localized_message(actor, NOT_DESECRATED, "");
    } else {
        state.localized_message(actor, DISTURBED_BY, &story.looters.join(", "));
    }
}

/// A living body. ServUO's one finding here is that the target belongs to the
/// Thieves' Guild, and there are no guilds yet, so a successful read honestly finds
/// nothing — which is a different sentence from failing to read at all, and the
/// client has both.
fn read_the_living(state: &mut WorldState, actor: EntityId, skill: u16) {
    if skill < LIVING_MIN {
        state.localized_message(actor, NOTHING_UNUSUAL, "");
        return;
    }
    let id = Skill::Forensics.id();
    if roll_skill_band(state, actor, id, LIVING_BAND.0, LIVING_BAND.1) {
        state.localized_message(actor, NOTHING_UNUSUAL, "");
    } else {
        state.localized_message(actor, NOTHING_USEFUL, "");
    }
}

/// Whether the corpse is a person's — read off the body id the corpse draws as.
fn corpse_was_human(state: &WorldState, corpse: EntityId) -> bool {
    state
        .registry
        .get::<Amount>(corpse)
        .is_some_and(|body| body_type(body.0) == BodyType::Human)
}
