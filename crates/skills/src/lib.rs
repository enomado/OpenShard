//! Skills and stats: the check, the gain curve, and the stat foundation.
//!
//! A gameplay system in its own crate, like `chat`. The functions here operate on
//! the shared [`WorldState`]: set a skill or a stat, use a skill against a
//! difficulty band, roll it. A use resolves the check, applies any gain, and emits
//! [`SkillUsed`] — what the use *accomplishes* (the ore, the turned lock) is
//! decided elsewhere, the same decoupling combat's `MobileDied` has.
//!
//! What a skill *is* — its name, its client id, the stats it leans on — is
//! [`openshard_state::skill`], data several crates read. What training one *does*
//! is here.
//!
//! [`roll_skill_band`] and [`roll_skill_chance`] are public on purpose: magic's
//! casting and combat's to-hit train through the very same call a mined ore does,
//! so there is one gain curve in the engine and not three.

mod button;
mod check;
mod stats;

pub use button::{set_skill_delay, use_skill_button, SkillRequested, DEFAULT_SKILL_DELAY_TICKS};
pub use check::{gain_chance, roll_skill_band, roll_skill_chance, skill_value};
pub use stats::gain_stat;

use openshard_entities::{EntityId, Serial};
use openshard_state::components::{Hitpoints, Mana, Skills, Stamina, Stats};
use openshard_state::WorldState;

/// A mobile's skill moved.
///
/// Every change passes through the one gain path, so this is the single place a
/// skill's value changes at all — up from training, or *down* when a skill set to
/// "down" gives ground so another can rise past the total cap. The world reads it
/// to send the owner the single-line `0x3A` that makes an open skill window follow
/// live; nothing else needs it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SkillChanged {
    /// The mobile.
    pub entity: EntityId,
    /// Its wire identity.
    pub serial: Serial,
    /// Which skill, by id.
    pub skill: u8,
    /// The skill's value now, in tenths.
    pub value: u16,
}

/// A mobile used a skill: the check resolved, and any gain is already applied.
///
/// What the *use* accomplishes is not decided here — whether the ore comes out of
/// the rock, whether the lockpick turns — only whether the roll passed and where
/// the skill stands now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SkillUsed {
    /// The mobile.
    pub entity: EntityId,
    /// Its wire identity.
    pub serial: Serial,
    /// Which skill, by id.
    pub skill: u8,
    /// Whether the check succeeded.
    pub success: bool,
    /// The skill's value now, in tenths, after any gain.
    pub value: u16,
}

/// Set a mobile's stats by serial, and re-cap its pools to match.
pub fn set_stats(
    state: &mut WorldState,
    serial: u32,
    strength: u16,
    dexterity: u16,
    intelligence: u16,
) {
    let Some(entity) = Serial::new(serial).and_then(|s| state.registry.entity_of(s)) else {
        return;
    };
    apply_stats(state, entity, strength, dexterity, intelligence);
}

/// Set a mobile's stats and re-cap its hit points, mana and stamina.
///
/// The one door stats change through, so the three derived pools can never drift
/// from them — a stat gain and a script's `op_set_stats` both land here.
pub fn apply_stats(
    state: &mut WorldState,
    entity: EntityId,
    strength: u16,
    dexterity: u16,
    intelligence: u16,
) {
    state.registry.insert(
        entity,
        Stats {
            strength,
            dexterity,
            intelligence,
        },
    );
    // Strength caps hit points, intelligence mana, dexterity stamina; a lowered
    // cap drags the current value down with it, a raised one leaves room to heal
    // into.
    if let Some(&Hitpoints { current, .. }) = state.registry.get::<Hitpoints>(entity) {
        state.registry.insert(
            entity,
            Hitpoints {
                current: current.min(strength),
                max: strength,
            },
        );
    }
    if let Some(&Mana { current, .. }) = state.registry.get::<Mana>(entity) {
        state.registry.insert(
            entity,
            Mana {
                current: current.min(intelligence),
                max: intelligence,
            },
        );
    }
    if let Some(&Stamina { current, .. }) = state.registry.get::<Stamina>(entity) {
        state.registry.insert(
            entity,
            Stamina {
                current: current.min(dexterity),
                max: dexterity,
            },
        );
    }
}

/// Set a mobile's skill value, in tenths, clamped to that skill's cap.
pub fn set_skill(state: &mut WorldState, serial: u32, skill: u8, value: u16) {
    let Some(entity) = Serial::new(serial).and_then(|s| state.registry.entity_of(s)) else {
        return;
    };
    let mut skills = state
        .registry
        .get::<Skills>(entity)
        .cloned()
        .unwrap_or_default();
    let cap = skills.cap(skill).min(state.gameplay.skill_cap);
    skills.set(skill, value.min(cap));
    state.registry.insert(entity, skills);
}

/// Set the ceiling on one of a mobile's skills, in tenths, dragging the value
/// down under it if it now sits above.
pub fn set_skill_cap(state: &mut WorldState, serial: u32, skill: u8, cap: u16) {
    let Some(entity) = Serial::new(serial).and_then(|s| state.registry.entity_of(s)) else {
        return;
    };
    let mut skills = state
        .registry
        .get::<Skills>(entity)
        .cloned()
        .unwrap_or_default();
    skills.set_cap(skill, cap);
    let value = skills.get(skill);
    if value > cap {
        skills.set(skill, cap);
    }
    state.registry.insert(entity, skills);
}

/// Use a skill against a difficulty band: roll it, teach from it, announce it.
///
/// The band is ServUO's — under `min_skill` the attempt is beyond the mobile,
/// at `max_skill` it is no challenge — and both are in tenths.
pub fn use_skill(state: &mut WorldState, serial: u32, skill: u8, min_skill: i32, max_skill: i32) {
    let Some(serial) = Serial::new(serial) else {
        return;
    };
    let Some(entity) = state.registry.entity_of(serial) else {
        return;
    };
    let success = roll_skill_band(state, entity, skill, min_skill, max_skill);
    let value = state
        .registry
        .get::<Skills>(entity)
        .map_or(0, |s| s.get(skill));
    state.bus.send(SkillUsed {
        entity,
        serial,
        skill,
        success,
        value,
    });
}
