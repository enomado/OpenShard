//! The guild window: one dialog, three pages, reached from the paperdoll's
//! Guild button.
//!
//! # One window, not four
//!
//! ServUO draws a gump per question — `GuildmasterGump`, `GuildRosterGump`,
//! `GuildDeclareWarGump`, `GrantGuildTitleGump`, a dozen more — because each is a
//! subclass with its own `OnResponse`. One id and three pages is the same
//! information with one reply handler, which is the shape `openshard-quests`
//! settled on for the same reason: four handlers that must agree about button
//! numbering are four chances to disagree.
//!
//! # The rows are the server's memory
//!
//! A reply names a *row*, and a row means whatever was drawn in it. The client is
//! free to send any number, so which member or which guild row three was comes
//! from [`GuildGumpContext`] — what this side remembers drawing — and never from
//! the packet. A reply to a window this side never opened resolves to nothing.
//!
//! # What it does not draw yet
//!
//! Paging. The lists are capped at [`MAX_ROWS`] and say so on the last line when
//! they are cut, rather than quietly showing the first twelve of a hundred.

use openshard_entities::EntityId;
use openshard_protocol::gump::{
    ButtonId, CloseGump, GUMP_WHITE, GumpButton, GumpDisplay, GumpId, GumpKey, GumpLayout, GumpPoint,
};
use openshard_protocol::server_packet::ServerPacket;
use openshard_state::{Client, GuildGumpContext, GuildMember, GuildPage, Name, Relation, WorldState};

use crate::{may_lead, roster};

/// The gump id the guild window answers under. Distinct from the quest window's
/// `0x0051_0001` and the admin menu's `0x00AD_0001`, so a reply is never
/// mistaken for either.
pub const GUILD_GUMP: GumpId = openshard_protocol::gump::id::GUILD;

/// Where the window opens.
const WINDOW: (i32, i32) = (100, 100);
/// How wide and tall the frame is. Tall enough for [`MAX_ROWS`] rows.
const FRAME: (i32, i32) = (420, 400);
/// The most rows a list draws before it says it has been cut.
pub const MAX_ROWS: usize = 12;

/// The first row's top edge, and how far apart rows sit.
const ROW_TOP: i32 = 90;
const ROW_HEIGHT: i32 = 24;

/// Hues: one for what a guild is, one for what it is at war with.
const HUE_HEADING: u32 = 1153;
const HUE_WAR: u32 = 33;
const HUE_ALLY: u32 = 68;

/// The buttons that mean one thing wherever they appear.
pub(crate) mod button {
    use openshard_protocol::gump::ButtonId;

    /// Dismiss. The layout draws no `X` of its own, so this is only ever the
    /// client's close box.
    pub const CLOSE: ButtonId = ButtonId::CLOSE_BOX;
    /// Found the guild named in the two fields.
    pub const FOUND: ButtonId = ButtonId(1);
    /// Say yes to an invitation.
    pub const ACCEPT: ButtonId = ButtonId(2);
    /// Say no to one.
    pub const DECLINE: ButtonId = ButtonId(3);
    /// To the roster page.
    pub const ROSTER: ButtonId = ButtonId(4);
    /// To the diplomacy page.
    pub const DIPLOMACY: ButtonId = ButtonId(5);
    /// Back to the front page.
    pub const MAIN: ButtonId = ButtonId(6);
    /// Leave the guild.
    pub const LEAVE: ButtonId = ButtonId(7);
    /// Raise a cursor to ask someone to join.
    pub const INVITE: ButtonId = ButtonId(8);
    /// Disband it.
    pub const DISBAND: ButtonId = ButtonId(9);
}

/// The two text fields on the founding form.
pub(crate) const FIELD_NAME: u32 = 1;
pub(crate) const FIELD_ABBREVIATION: u32 = 2;

/// A row button is its list's base plus the row index times the number of things
/// a row can ask for.
///
/// Both directions have a name — [`row_button`] draws and [`row_of`] reads — so
/// the arithmetic is written once and neither side re-derives it. That is what
/// went wrong in the admin menu's hand-written layout, and the note there says so.
pub(crate) const ROSTER_BASE: u32 = 100;
pub(crate) const DIPLOMACY_BASE: u32 = 1000;
/// How many buttons a row of either list draws.
pub(crate) const ROW_ACTIONS: u32 = 3;

/// The button id for one action on one row.
pub(crate) const fn row_button(base: u32, index: usize, action: u32) -> ButtonId {
    ButtonId(base + (index as u32) * ROW_ACTIONS + action)
}

/// Which row and which action a button id names, or `None` if it is not one of
/// this list's.
pub(crate) fn row_of(base: u32, button: ButtonId, rows: usize) -> Option<(usize, u32)> {
    let offset = button.0.checked_sub(base)?;
    let index = (offset / ROW_ACTIONS) as usize;
    if index >= rows {
        return None;
    }
    Some((index, offset % ROW_ACTIONS))
}

/// Draw the guild window for a player, and remember what it drew.
///
/// Closes the window already open first. The pages replace each other under one
/// id, and a client told to draw the same id twice draws two windows — the same
/// close-then-draw every other dialog here opens with.
pub fn show(state: &mut WorldState, player: EntityId, page: GuildPage) {
    let Some(&Client { connection, .. }) = state.registry.get::<Client>(player) else {
        return;
    };
    let Some(serial) = state.registry.serial_of(player) else {
        return;
    };
    // A player who is not in a guild has one page. Asking for another is not an
    // error — it is a stale button on a window drawn before they left — so it
    // lands on the page they do have.
    let page = if state.guild_of(player).is_some() {
        page
    } else {
        GuildPage::Main
    };
    let (layout, context) = build(state, player, page);
    let (string, lines) = layout.finish();

    let close = ServerPacket::CloseGump(CloseGump {
        gump_id: GUILD_GUMP,
        button: ButtonId::CLOSE_BOX,
    });
    let draw = ServerPacket::GumpDisplay(GumpDisplay {
        serial: GumpKey::on(serial),
        gump_id: GUILD_GUMP,
        at: GumpPoint::new(WINDOW.0, WINDOW.1),
        layout: string.to_owned(),
        lines: lines.to_vec(),
    });
    state.send_packet(connection, &close);
    state.send_packet(connection, &draw);
    if let Some(row) = state.row_of_mut(player) {
        row.guild_gump = Some(context);
    }
}

/// Build one page, and the record of what its rows meant.
fn build(state: &WorldState, player: EntityId, page: GuildPage) -> (GumpLayout, GuildGumpContext) {
    let mut layout = GumpLayout::new();
    layout.no_resize();
    layout.page(0);
    layout.background(0, 0, FRAME.0, FRAME.1, 5054);

    let mut context = GuildGumpContext {
        page,
        guilds: Vec::new(),
        members: Vec::new(),
    };
    match page {
        GuildPage::Main => main_page(&mut layout, state, player),
        GuildPage::Roster => roster_page(&mut layout, state, player, &mut context),
        GuildPage::Diplomacy => diplomacy_page(&mut layout, state, player, &mut context),
    }
    (layout, context)
}

/// A plain reply button with the menu's art, and its label.
fn action(layout: &mut GumpLayout, x: i32, y: i32, id: ButtonId, hue: u32, label: &str) {
    layout.button(x, y, 4005, 4007, GumpButton::Reply, 0, id);
    layout.label(x + 36, y + 2, hue, label);
}

/// The front page: found a guild, or what yours is.
fn main_page(layout: &mut GumpLayout, state: &WorldState, player: EntityId) {
    let Some(guild) = state.guild_of(player) else {
        return no_guild_page(layout, state, player);
    };
    layout.label(
        20,
        20,
        HUE_HEADING,
        format!("{} [{}]", guild.name, guild.abbreviation),
    );
    let title = state
        .registry
        .get::<GuildMember>(player)
        .map_or("", |member| member.title.as_str());
    let title = if title.is_empty() { "no title" } else { title };
    layout.label(20, 44, GUMP_WHITE, format!("You hold {title}."));

    let mut y = ROW_TOP;
    action(layout, 20, y, button::ROSTER, GUMP_WHITE, "Members");
    y += ROW_HEIGHT + 8;
    if may_lead(state, player).is_ok() {
        action(layout, 20, y, button::INVITE, GUMP_WHITE, "Ask someone to join");
        y += ROW_HEIGHT + 8;
        action(layout, 20, y, button::DIPLOMACY, GUMP_WHITE, "Wars and alliances");
        y += ROW_HEIGHT + 8;
        action(layout, 20, y, button::DISBAND, HUE_WAR, "Disband this guild");
    } else {
        action(layout, 20, y, button::LEAVE, HUE_WAR, "Leave this guild");
    }
}

/// The front page for someone in no guild: the invitation, if there is one, and
/// the form for founding.
fn no_guild_page(layout: &mut GumpLayout, state: &WorldState, player: EntityId) {
    layout.label(20, 20, HUE_HEADING, "Guild");

    let invitation = state
        .registry
        .get::<openshard_state::GuildCandidate>(player)
        .and_then(|asked| state.guilds.get(asked.guild));
    let mut y = 50;
    if let Some(guild) = invitation {
        layout.label(
            20,
            y,
            GUMP_WHITE,
            format!("{} has asked you to join.", guild.name),
        );
        y += ROW_HEIGHT;
        action(layout, 20, y, button::ACCEPT, HUE_ALLY, "Accept");
        action(layout, 200, y, button::DECLINE, HUE_WAR, "Decline");
        y += ROW_HEIGHT + 16;
    }

    layout.label(20, y, GUMP_WHITE, "Found a guild of your own:");
    y += ROW_HEIGHT;
    layout.label(20, y, GUMP_WHITE, "Name");
    layout.text_entry(90, y, 260, 20, GUMP_WHITE, FIELD_NAME, "");
    y += ROW_HEIGHT + 4;
    layout.label(20, y, GUMP_WHITE, "Abbrev.");
    layout.text_entry(90, y, 60, 20, GUMP_WHITE, FIELD_ABBREVIATION, "");
    y += ROW_HEIGHT + 12;
    action(layout, 20, y, button::FOUND, HUE_ALLY, "Found it");
}

/// The roster. A leader gets a field and three buttons per row; everyone else
/// gets the list.
fn roster_page(
    layout: &mut GumpLayout,
    state: &WorldState,
    player: EntityId,
    context: &mut GuildGumpContext,
) {
    let Some(guild) = state.guild_of(player).map(|g| g.id) else {
        return;
    };
    layout.label(20, 20, HUE_HEADING, "Members");
    action(layout, 20, 46, button::MAIN, GUMP_WHITE, "Back");

    let leads = may_lead(state, player).is_ok();
    let members = roster(state, guild);
    let shown = members.len().min(MAX_ROWS);
    for &member in members.iter().take(shown) {
        let Some(serial) = state.registry.serial_of(member) else {
            continue;
        };
        context.members.push(serial);
        let row = context.members.len() - 1;
        let y = ROW_TOP + (row as i32) * ROW_HEIGHT;
        let name = state
            .registry
            .get::<Name>(member)
            .map_or("someone", |name| name.0.as_str());
        layout.label(20, y, GUMP_WHITE, name);
        let title = state
            .registry
            .get::<GuildMember>(member)
            .map_or("", |entry| entry.title.as_str());
        if leads {
            layout.text_entry(140, y, 110, 20, GUMP_WHITE, row as u32, title);
            layout.button(
                258,
                y,
                4005,
                4007,
                GumpButton::Reply,
                0,
                row_button(ROSTER_BASE, row, 0),
            );
            layout.button(
                288,
                y,
                4017,
                4019,
                GumpButton::Reply,
                0,
                row_button(ROSTER_BASE, row, 1),
            );
            layout.button(
                318,
                y,
                4011,
                4013,
                GumpButton::Reply,
                0,
                row_button(ROSTER_BASE, row, 2),
            );
        } else {
            layout.label(140, y, GUMP_WHITE, title);
        }
    }
    if leads {
        layout.label(258, ROW_TOP - 22, GUMP_WHITE, "title  out  lead");
    }
    cut_notice(layout, members.len(), shown);
}

/// Every other guild, and where this one stands with it.
fn diplomacy_page(
    layout: &mut GumpLayout,
    state: &WorldState,
    player: EntityId,
    context: &mut GuildGumpContext,
) {
    let Ok(own) = may_lead(state, player) else {
        // Not the leader any more — a stale button on a window drawn while they
        // were. Draw the page with nothing on it rather than a list they cannot
        // act on.
        layout.label(20, 20, HUE_HEADING, "Wars and alliances");
        action(layout, 20, 46, button::MAIN, GUMP_WHITE, "Back");
        return;
    };
    layout.label(20, 20, HUE_HEADING, "Wars and alliances");
    action(layout, 20, 46, button::MAIN, GUMP_WHITE, "Back");
    layout.label(258, ROW_TOP - 22, GUMP_WHITE, "war  ally  peace");

    let others: Vec<_> = state.guilds.iter().filter(|guild| guild.id != own).collect();
    let shown = others.len().min(MAX_ROWS);
    for (row, guild) in others.iter().take(shown).enumerate() {
        context.guilds.push(guild.id);
        let y = ROW_TOP + (row as i32) * ROW_HEIGHT;
        let ours = state.guilds.get(own);
        let (standing, hue) = match ours.and_then(|ours| ours.toward(guild.id)) {
            Some(Relation::War) => ("at war", HUE_WAR),
            Some(Relation::Ally) => ("allied", HUE_ALLY),
            None => match ours.and_then(|ours| ours.offered(guild.id)) {
                Some(Relation::War) => ("war declared", HUE_WAR),
                Some(Relation::Ally) => ("alliance offered", HUE_ALLY),
                None => ("", GUMP_WHITE),
            },
        };
        layout.label(
            20,
            y,
            GUMP_WHITE,
            format!("{} [{}]", guild.name, guild.abbreviation),
        );
        layout.label(150, y, hue, standing);
        layout.button(
            258,
            y,
            4017,
            4019,
            GumpButton::Reply,
            0,
            row_button(DIPLOMACY_BASE, row, 0),
        );
        layout.button(
            288,
            y,
            4011,
            4013,
            GumpButton::Reply,
            0,
            row_button(DIPLOMACY_BASE, row, 1),
        );
        layout.button(
            318,
            y,
            4005,
            4007,
            GumpButton::Reply,
            0,
            row_button(DIPLOMACY_BASE, row, 2),
        );
    }
    cut_notice(layout, others.len(), shown);
}

/// Say so when a list was cut, rather than showing the first twelve of a hundred
/// and looking complete.
fn cut_notice(layout: &mut GumpLayout, total: usize, shown: usize) {
    if total > shown {
        let y = ROW_TOP + (shown as i32) * ROW_HEIGHT;
        layout.label(20, y, HUE_HEADING, format!("...and {} more.", total - shown));
    }
}

#[cfg(test)]
mod tests {
    use super::{DIPLOMACY_BASE, ROSTER_BASE, row_button, row_of};

    #[test]
    fn a_row_button_reads_back_as_the_row_it_was_drawn_for() {
        for row in 0..8 {
            for action in 0..3 {
                let id = row_button(ROSTER_BASE, row, action);
                assert_eq!(row_of(ROSTER_BASE, id, 8), Some((row, action)));
            }
        }
    }

    #[test]
    fn a_button_past_the_end_of_the_list_names_no_row() {
        // The client sends whatever it likes. A row the window never drew has to
        // resolve to nothing rather than to the row arithmetic's opinion.
        assert_eq!(row_of(ROSTER_BASE, row_button(ROSTER_BASE, 9, 0), 4), None);
        assert_eq!(
            row_of(ROSTER_BASE, openshard_protocol::gump::ButtonId(1), 4),
            None
        );
        // And the two lists do not overlap, which is what keeps a diplomacy
        // button from dismissing a member.
        assert!(row_of(ROSTER_BASE, row_button(DIPLOMACY_BASE, 0, 0), 12).is_none());
    }
}
