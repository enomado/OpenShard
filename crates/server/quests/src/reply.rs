//! Reading the quest window's answer.
//!
//! A port of `MondainQuestGump.OnResponse` and `MondainResignGump.OnResponse`.
//! The button ids mean what they mean because the layout said so, and the *page*
//! they were clicked on comes from what the server remembered opening — never
//! from the client, which is free to send any number it likes.

use openshard_entities::EntityId;
use openshard_gateway::ConnectionId;
use openshard_protocol::gump::GumpResponse;
use openshard_state::components::QuestLog;
use openshard_state::{QuestGumpContext, QuestSection, WorldState};

use crate::gump::{self, button, QUEST_GUMP, QUEST_RESIGN_GUMP, RESIGN_OK, RESIGN_SWITCH_YES};
use crate::{offer, turnin};

/// Whether a gump id belongs to the quest system.
#[must_use]
pub const fn owns(gump_id: u32) -> bool {
    gump_id == QUEST_GUMP || gump_id == QUEST_RESIGN_GUMP
}

/// Act on a quest dialog's reply.
///
/// Returns whether the reply was one of ours, so the router can fall through to
/// the pack for anything else.
pub fn handle(state: &mut WorldState, connection: ConnectionId, response: &GumpResponse) -> bool {
    if !owns(response.gump_id) {
        return false;
    }
    let Some(&player) = state.players.get(&connection) else {
        return true;
    };
    if response.gump_id == QUEST_RESIGN_GUMP {
        resign_reply(state, player, response);
        return true;
    }
    // The context is the server's memory of what it drew. A reply with none is a
    // reply to a window this side never opened — a stale click, or a crafted
    // packet — and does nothing.
    let Some(context) = state.open_quest_gumps.remove(&player) else {
        return true;
    };
    quest_reply(state, player, &context, response.button);
    true
}

/// The main window's buttons.
fn quest_reply(state: &mut WorldState, player: EntityId, context: &QuestGumpContext, pressed: u32) {
    match pressed {
        button::CLOSE => {}
        button::CLOSE_QUEST => {
            // Back to the log.
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
        button::ACCEPT_QUEST if context.offer => {
            offer::accept(state, player, &context.quest, context.giver);
        }
        button::REFUSE_QUEST if context.offer => {
            offer::refuse(state, player, &context.quest);
        }
        button::RESIGN_QUEST if !context.offer => {
            gump::show_resign(state, player, &context.quest);
            // Remembered so the confirmation's reply knows which quest it is
            // about: the resign dialog carries no quest of its own.
            state.open_quest_gumps.insert(player, context.clone());
        }
        button::COMPLETE if !context.offer => {
            hand_in(state, player, context);
        }
        button::ACCEPT_REWARD if context.completed => {
            turnin::complete(state, player, &context.quest);
        }
        button::PREVIOUS_PAGE | button::NEXT_PAGE => {
            let section = page(context.section, pressed == button::NEXT_PAGE);
            gump::show(
                state,
                player,
                QuestGumpContext {
                    section,
                    ..context.clone()
                },
            );
        }
        row => open_row(state, player, row),
    }
}

/// Hand a finished quest in. With rewards to show, the rewards page comes first
/// and pays on its own button; with none, it is paid outright — ServUO's
/// `Buttons.Complete`.
fn hand_in(state: &mut WorldState, player: EntityId, context: &QuestGumpContext) {
    if !turnin::is_complete(state, player, &context.quest) {
        state.system_message(player, "You do not have everything you need!");
        return;
    }
    let has_rewards = state
        .quests
        .get(&context.quest)
        .is_some_and(|quest| !quest.rewards.is_empty());
    if has_rewards {
        gump::show(
            state,
            player,
            QuestGumpContext {
                section: QuestSection::Rewards,
                completed: true,
                ..context.clone()
            },
        );
    } else {
        turnin::complete(state, player, &context.quest);
    }
}

/// A row in the quest log: open that quest's detail page.
fn open_row(state: &mut WorldState, player: EntityId, pressed: u32) {
    let Some(index) = pressed.checked_sub(gump::ROW_OFFSET) else {
        return;
    };
    let Some(entry) = state
        .registry
        .get::<QuestLog>(player)
        .and_then(|log| log.active.get(index as usize))
    else {
        return;
    };
    let (key, giver) = (entry.key.clone(), entry.giver);
    gump::show(
        state,
        player,
        gump::log_context(&key, QuestSection::Description, giver),
    );
}

/// The next or previous page of a quest, stopping at the ends.
const fn page(from: QuestSection, forward: bool) -> QuestSection {
    match (from, forward) {
        (QuestSection::Description, true) => QuestSection::Objectives,
        (QuestSection::Objectives, true) => QuestSection::Rewards,
        (QuestSection::Objectives, false) => QuestSection::Description,
        (QuestSection::Rewards, false) => QuestSection::Objectives,
        (section, _) => section,
    }
}

/// The resign confirmation: only the "yes" radio actually gives the quest up.
fn resign_reply(state: &mut WorldState, player: EntityId, response: &GumpResponse) {
    let context = state.open_quest_gumps.remove(&player);
    let Some(context) = context else {
        return;
    };
    if response.button != RESIGN_OK {
        return; // dismissed
    }
    if response.switches.contains(&RESIGN_SWITCH_YES) {
        offer::resign(state, player, &context.quest);
    } else {
        // Kept: back to the quest's page, where it was.
        gump::show(state, player, context);
    }
}
