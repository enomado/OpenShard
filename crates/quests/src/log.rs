//! The ways into the quest system from outside: the paperdoll button, the
//! "quest" keyword, and the two bindings the pack sets on an NPC.

use openshard_entities::{EntityId, Serial};
use openshard_gateway::ConnectionId;
use openshard_state::components::{Client, Escortable, QuestGiver};
use openshard_state::{QuestGumpContext, QuestSection, WorldState};

use crate::gump;
use crate::offer;

/// How close a giver has to be to hear "quest" — the banker's range, and the
/// same reason: a keyword answered from across the town square is a keyword
/// answered by the wrong NPC.
const SPEECH_RANGE: u32 = 4;

/// Open a player's quest log — where the paperdoll's Quest button lands.
///
/// Draws ServUO's `Section.Main`. An empty log still opens: a window that says
/// "nothing here" is an answer, and silence looks like a broken button.
pub fn open_log(state: &mut WorldState, connection: ConnectionId) {
    let Some(&player) = state.players.get(&connection) else {
        return;
    };
    open_log_for(state, player);
}

/// Open a mobile's own quest log, by entity — for the staff command and the
/// context menu, which already know who they mean.
pub fn open_log_for(state: &mut WorldState, player: EntityId) {
    if state.registry.get::<Client>(player).is_none() {
        return;
    }
    gump::show(
        state,
        player,
        QuestGumpContext {
            quest: String::new(),
            section: QuestSection::Main,
            offer: false,
            completed: false,
            giver: None,
        },
    );
}

/// Someone said something: if the words hold "quest" and a giver is standing
/// close by, treat it as if they had double-clicked that giver.
///
/// **Only a player's speech counts.** `MobileSpoke` fires for NPCs too — quite
/// correctly, since an NPC saying something is a thing that happened — and an
/// NPC's own line can hold the word "quest", so a filter that lives anywhere but
/// on the reader has a quest giver answering itself.
pub fn speech_offer(state: &mut WorldState, speaker: Serial, text: &str) {
    let Some(player) = state.registry.entity_of(speaker) else {
        return;
    };
    if state.registry.get::<Client>(player).is_none() {
        return;
    }
    if !text
        .to_ascii_lowercase()
        .split_whitespace()
        .any(|word| word.trim_matches(|c: char| !c.is_ascii_alphanumeric()) == "quest")
    {
        return;
    }
    let Some(&openshard_state::components::Position(at)) =
        state
            .registry
            .get::<openshard_state::components::Position>(player)
    else {
        return;
    };
    // Found through the spatial index, not by walking every giver on the shard —
    // the pack's version asked each of ~60 givers for its position on every line
    // anyone spoke.
    let facet = state.facet_of(player);
    let nearby: Vec<EntityId> = state
        .facet_state(facet)
        .sectors
        .nearby(at, SPEECH_RANGE)
        .map(|(entity, _)| entity)
        .collect();
    for candidate in nearby {
        if candidate != player && state.registry.has::<QuestGiver>(candidate) {
            offer::talk_to(state, player, candidate);
            return;
        }
    }
}

/// Mark an NPC as offering a set of quests. From the pack, and **saved with the
/// mobile** — that is the whole point.
pub fn bind_giver(state: &mut WorldState, mobile: Serial, keys: Vec<String>) {
    let Some(entity) = state.registry.entity_of(mobile) else {
        return;
    };
    if keys.is_empty() {
        state.registry.remove::<QuestGiver>(entity);
        return;
    }
    state.registry.insert(entity, QuestGiver { keys });
}

/// Mark an NPC as escortable, optionally to a fixed region. From the pack, and
/// saved with the mobile.
pub fn make_escortable(state: &mut WorldState, mobile: Serial, destination: String) {
    let Some(entity) = state.registry.entity_of(mobile) else {
        return;
    };
    state.registry.insert(
        entity,
        Escortable {
            destination,
            escorter: None,
            last_seen: state.ticks,
        },
    );
}

/// Put an escortable in someone's care. Refuses one that is already following
/// somebody — ServUO's escortable takes one charge at a time.
pub fn start_escort(state: &mut WorldState, npc: Serial, escorter: Serial) -> bool {
    let Some(entity) = state.registry.entity_of(npc) else {
        return false;
    };
    let Some(mut escort) = state.registry.get::<Escortable>(entity).cloned() else {
        return false;
    };
    if escort.escorter.is_some_and(|current| current != escorter) {
        return false;
    }
    escort.escorter = Some(escorter);
    escort.last_seen = state.ticks;
    state.registry.insert(entity, escort);
    true
}

/// A player double-clicked an escortable: take it into their care, and say where
/// it wants to go.
///
/// Returns whether the mobile was an escortable at all. One escortable follows
/// one person: a second player clicking it is told so rather than silently
/// stealing it, which is ServUO's rule and the only one that does not lose the
/// first player's walk.
pub(crate) fn ask_for_escort(state: &mut WorldState, player: EntityId, npc: EntityId) -> bool {
    let Some(escort) = state.registry.get::<Escortable>(npc).cloned() else {
        return false;
    };
    let (Some(player_serial), Some(npc_serial)) = (
        state.registry.serial_of(player),
        state.registry.serial_of(npc),
    ) else {
        return false;
    };
    if escort
        .escorter
        .is_some_and(|current| current != player_serial)
    {
        state.system_message(player, "That person is already being escorted.");
        return true;
    }
    // A destination the pack did not fix is chosen now, from the facet's own
    // named regions — ServUO's `PickRandomDestination`. On the world's seeded
    // generator, so the choice replays with the rest of the tick.
    let destination = if escort.destination.is_empty() {
        let Some(picked) = random_town(state, npc) else {
            return true; // a facet with no named regions has nowhere to go
        };
        picked
    } else {
        escort.destination.clone()
    };
    make_escortable(state, npc_serial, destination.clone());
    if !start_escort(state, npc_serial, player_serial) {
        return true;
    }
    let name = state
        .registry
        .get::<openshard_state::components::Name>(npc)
        .map_or_else(|| "The traveller".to_owned(), |n| n.0.clone());
    state.system_message(player, &format!("{name} will follow you to {destination}."));
    true
}

/// A named region on the mobile's facet, picked at random — where an escortable
/// with no fixed destination asks to be taken.
///
/// Skips the region it is standing in: "escort me to where we already are" is a
/// quest that completes on the spot.
fn random_town(state: &mut WorldState, npc: EntityId) -> Option<String> {
    let facet = state.facet_of(npc);
    let here = state
        .registry
        .get::<openshard_state::components::Position>(npc)
        .map(|position| position.0);
    let standing_in = here
        .and_then(|at| state.region_at(facet, at))
        .map(|region| region.name.clone());
    let names: Vec<String> = state
        .facet_state(facet)
        .regions
        .iter()
        .filter(|region| Some(&region.name) != standing_in.as_ref())
        .map(|region| region.name.clone())
        .collect();
    if names.is_empty() {
        return None;
    }
    let pick = state.rng.below(names.len() as u32) as usize;
    names.get(pick).cloned()
}
