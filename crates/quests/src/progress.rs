//! Moving an objective along.
//!
//! # Why nothing calls in here
//!
//! Each of these is driven by something the world already says, or by a pass that
//! *looks*. Nothing in combat, items or movement knows quests exist — a kill
//! objective reads the deaths combat announced, an escort reads where the escorted
//! NPC is standing, and an obtain objective counts the backpack because the engine
//! emits nothing at all when an item changes hands.
//!
//! That last one is deliberate rather than a gap. The obvious fix — an
//! `ItemMoved` event, or a `quests::notice()` beside every insert into a
//! container — is exactly the pattern the persistence rule warns decays: the first
//! system that moves an item without knowing quests exist breaks a quest silently,
//! and no test without a quest in it catches that. A pass that looks cannot be
//! forgotten. It costs one walk of the backpack per player who actually has an
//! obtain objective, half a second apart, which is the same bargain the status bar
//! already makes.

use openshard_combat::MobileDied;
use openshard_entities::{EntityId, Serial};
use openshard_items::Contents;
use openshard_state::components::{Escortable, QuestLog};
use openshard_state::quest::ObjectiveKind;
use openshard_state::{QuestSection, WorldState, TICKS_PER_SECOND};

use crate::events::{QuestFailed, QuestObjectiveUpdated};
use crate::gump::{self, sound};

/// How often the obtain pass looks, in ticks. Twice a second, the status bar's
/// cadence — fast enough that picking an item up feels immediate, slow enough
/// that a still player costs nothing.
///
/// Public because the caller has to build the containment index the pass reads,
/// and building it every tick to be thrown away nineteen times out of twenty is
/// the cost this cadence exists to avoid.
pub const OBTAIN_EVERY_TICKS: u64 = TICKS_PER_SECOND / 2;

/// How far an escorted NPC may fall behind before it gives up, in ticks.
/// ServUO's `BaseEscortable` waits about half a minute for a lost escorter.
const ESCORT_PATIENCE_TICKS: u64 = 30 * TICKS_PER_SECOND;

/// Credit this tick's kills against every slay objective they match.
///
/// Only the killer's, and only when the kill was attributed: an unattributed
/// death (a field, a fall, a reflected blow) advances nobody's quest, which is
/// the same rule that keeps it off a murder count.
pub fn advance_slay(state: &mut WorldState, deaths: &[MobileDied]) {
    for death in deaths {
        let Some(killer) = death.killer else {
            continue;
        };
        let Some(entity) = state.registry.entity_of(killer) else {
            continue;
        };
        advance(
            state,
            entity,
            |kind| matches!(*kind, ObjectiveKind::Slay { body } if body == death.body),
        );
    }
}

/// Count what every player with an obtain objective is carrying, and move the
/// objective to match.
///
/// Progress goes **down** as well as up: drop the items and the objective falls
/// back, which is ServUO's behaviour and the only honest reading of an objective
/// that says "carry five of these".
pub fn refresh_obtain(state: &mut WorldState, contents: &Contents) {
    if !state.ticks.is_multiple_of(OBTAIN_EVERY_TICKS) {
        return;
    }
    let players: Vec<EntityId> = state.players.values().copied().collect();
    for player in players {
        let Some(log) = state.registry.get::<QuestLog>(player).cloned() else {
            continue;
        };
        let Some(serial) = state.registry.serial_of(player) else {
            continue;
        };
        let mut changed = false;
        let mut updates: Vec<(String, usize, u16, u16)> = Vec::new();
        let mut log = log;
        for quest in &mut log.active {
            let Some(def) = state.quests.get(&quest.key) else {
                continue;
            };
            for (index, objective) in def.objectives.iter().enumerate() {
                let ObjectiveKind::Obtain { graphic } = objective.kind else {
                    continue;
                };
                let held = openshard_items::carried_amount_with(state, contents, serial, graphic);
                let held = u16::try_from(held.min(u32::from(objective.count))).unwrap_or(0);
                let Some(slot) = quest.progress.get_mut(index) else {
                    continue;
                };
                if *slot == held {
                    continue;
                }
                let rising = held > *slot;
                *slot = held;
                changed = true;
                if rising {
                    updates.push((quest.key.clone(), index, held, objective.count));
                }
            }
        }
        if changed {
            state.registry.insert(player, log);
            for (key, index, progress, goal) in updates {
                announce(state, player, serial, &key, index, progress, goal);
            }
        }
    }
}

/// Move every escortable that someone is leading, and pay when it arrives.
///
/// The arrival test is a point query — which region is this NPC standing in — and
/// not an event, which is what lets this crate stay below the one that owns
/// regions. An escortable whose leader has been out of sight too long gives up.
pub fn advance_escorts(state: &mut WorldState) -> Vec<(u32, u8)> {
    let escorting: Vec<(EntityId, Escortable)> = state
        .registry
        .query::<Escortable>()
        .filter(|(_, escort)| escort.escorter.is_some())
        .map(|(entity, escort)| (entity, escort.clone()))
        .collect();
    let mut steps = Vec::new();
    for (npc, escort) in escorting {
        let Some(escorter) = escort.escorter.and_then(|s| state.registry.entity_of(s)) else {
            abandon(state, npc);
            continue;
        };
        let (Some(here), Some(there)) = (position_of(state, npc), position_of(state, escorter))
        else {
            abandon(state, npc);
            continue;
        };
        // Out of sight for too long: the escorter logged out, died, or simply
        // walked off. Give up rather than trail a ghost across the facet.
        if openshard_state::in_range(here, there, ESCORT_RANGE) {
            if let Some(mut current) = state.registry.get::<Escortable>(npc).cloned() {
                current.last_seen = state.ticks;
                state.registry.insert(npc, current);
            }
        } else if state.ticks.saturating_sub(escort.last_seen) > ESCORT_PATIENCE_TICKS {
            abandon(state, npc);
            continue;
        }

        // Arrived? Checked before stepping, so the last step into the region
        // pays on the same tick it lands.
        let facet = state.facet_of(npc);
        let arrived = state
            .region_at(facet, here)
            .is_some_and(|region| region.name == escort.destination);
        if arrived {
            arrive(state, npc, escorter, &escort.destination);
            continue;
        }

        // Otherwise follow, on its own beat, planning around obstacles the way a
        // chasing creature does — the same `step_toward`, so an escortable walks
        // through a doorway rather than shuffling against the frame. The step is
        // *returned*, not taken: movement is the world's, and this is the
        // decide-then-apply split `ai::think_one` already uses.
        if !state.ticks.is_multiple_of(ESCORT_BEAT_TICKS)
            || openshard_state::in_range(here, there, FOLLOW_GAP)
        {
            continue;
        }
        let opens_doors = state
            .registry
            .get::<openshard_state::components::Body>(npc)
            .is_some_and(|body| openshard_state::components::body_opens_doors(body.id));
        if let Some(direction) = openshard_ai::step_toward(state, facet, here, there, opens_doors) {
            if let Some(serial) = state.registry.serial_of(npc) {
                steps.push((serial.raw(), direction));
            }
        }
    }
    steps
}

/// Ticks between an escortable's steps — a townsperson's amble, so a player does
/// not have to stand still and wait for it.
const ESCORT_BEAT_TICKS: u64 = 6;

/// How close is close enough to stop following. One tile back, so it does not
/// tread on the escorter's heels or block a doorway they are trying to use.
const FOLLOW_GAP: u32 = 1;

/// How close an escorted NPC must stay to whoever is leading it.
const ESCORT_RANGE: u32 = 12;

/// Count down every timed objective, and fail the quest when one runs out.
pub fn tick_timers(state: &mut WorldState) {
    if !state.ticks.is_multiple_of(TICKS_PER_SECOND) {
        return;
    }
    let players: Vec<EntityId> = state.players.values().copied().collect();
    for player in players {
        let Some(mut log) = state.registry.get::<QuestLog>(player).cloned() else {
            continue;
        };
        let mut newly_failed: Vec<String> = Vec::new();
        let mut changed = false;
        for quest in &mut log.active {
            if quest.failed {
                continue;
            }
            for seconds in &mut quest.seconds_left {
                if *seconds == 0 {
                    continue; // untimed, or already run out
                }
                *seconds -= 1;
                changed = true;
                if *seconds == 0 {
                    quest.failed = true;
                    newly_failed.push(quest.key.clone());
                }
            }
        }
        if !changed {
            continue;
        }
        state.registry.insert(player, log);
        for key in newly_failed {
            if let Some(serial) = state.registry.serial_of(player) {
                state.bus.send(QuestFailed {
                    player: serial,
                    key: key.clone(),
                });
            }
            gump::show(
                state,
                player,
                gump::log_context(&key, QuestSection::Failed, None),
            );
        }
    }
}

/// Bump every objective of a player's that `matches`, and say so.
///
/// The one place progress moves, so the "an objective went up" message, sound and
/// event are written once rather than at each call site.
pub(crate) fn advance(
    state: &mut WorldState,
    player: EntityId,
    matches: impl Fn(&ObjectiveKind) -> bool,
) {
    let Some(mut log) = state.registry.get::<QuestLog>(player).cloned() else {
        return;
    };
    let Some(serial) = state.registry.serial_of(player) else {
        return;
    };
    let mut updates: Vec<(String, usize, u16, u16)> = Vec::new();
    for quest in &mut log.active {
        if quest.failed {
            continue;
        }
        let Some(def) = state.quests.get(&quest.key) else {
            continue;
        };
        for (index, objective) in def.objectives.iter().enumerate() {
            if !matches(&objective.kind) {
                continue;
            }
            let Some(slot) = quest.progress.get_mut(index) else {
                continue;
            };
            if *slot >= objective.count {
                continue;
            }
            *slot += 1;
            updates.push((quest.key.clone(), index, *slot, objective.count));
        }
    }
    if updates.is_empty() {
        return;
    }
    state.registry.insert(player, log);
    for (key, index, progress, goal) in updates {
        announce(state, player, serial, &key, index, progress, goal);
    }
}

/// Tell the player an objective moved, and the pack that it did.
fn announce(
    state: &mut WorldState,
    player: EntityId,
    serial: Serial,
    key: &str,
    objective: usize,
    progress: u16,
    goal: u16,
) {
    state.bus.send(QuestObjectiveUpdated {
        player: serial,
        key: key.to_owned(),
        objective,
        progress,
        goal,
    });
    gump::play(state, player, sound::UPDATE);
    if progress >= goal && crate::turnin::is_complete(state, player, key) {
        gump::play(state, player, sound::COMPLETE);
        state.system_message(
            player,
            "You have completed a quest! Return to whoever gave it to you.",
        );
    }
}

/// An escortable that reached where it was going: the escort objective is met.
fn arrive(state: &mut WorldState, npc: EntityId, escorter: EntityId, destination: &str) {
    advance(
        state,
        escorter,
        |kind| matches!(kind, ObjectiveKind::Escort { region } if region == destination),
    );
    // The NPC's part is over: it stops being escortable and wanders off like any
    // other townsperson. Despawning it here would make a quest giver vanish under
    // the player who just walked it across the map.
    state.registry.remove::<Escortable>(npc);
    if let Some(name) = state
        .registry
        .get::<openshard_state::components::Name>(npc)
        .map(|n| n.0.clone())
    {
        state.system_message(escorter, &format!("{name} thanks you and departs."));
    }
}

/// Stop following: the escorter logged out, died, or simply walked away.
fn abandon(state: &mut WorldState, npc: EntityId) {
    if let Some(mut escort) = state.registry.get::<Escortable>(npc).cloned() {
        escort.escorter = None;
        escort.last_seen = state.ticks;
        state.registry.insert(npc, escort);
    }
}

/// Where a mobile stands, if it is anywhere.
fn position_of(state: &WorldState, entity: EntityId) -> Option<openshard_protocol::Point> {
    state
        .registry
        .get::<openshard_state::components::Position>(entity)
        .map(|position| position.0)
}
