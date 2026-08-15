//! The rules, over a world with nothing in it but mobiles.
//!
//! A bare [`WorldState`]: no map, no terrain, no gateway. That is enough for
//! every rule here, because none of them look at the ground — and it keeps the
//! assertions about guilds rather than about a world that had to be stood up
//! first. The window's tests add a session row and a `Client`, which is all a
//! gump needs; where those packets end up is the tick's business.

use std::collections::{BTreeMap, HashMap};

use openshard_entities::{EntityId, Registry};
use openshard_events::EventBus;
use openshard_gateway::ConnectionId;
use openshard_protocol::access::AccessLevel;
use openshard_protocol::gump::{ButtonId, GumpResponse, RawButtonId, RawGumpId, RawGumpKey};
use openshard_protocol::identity::AccountName;
use openshard_protocol::serial::SerialKind;
use openshard_protocol::version::ClientVersion;
use openshard_protocol::world::Facet;
use openshard_state::connection::Connection;
use openshard_state::harvest::Banks;
use openshard_state::rng::Rng;
use openshard_state::sectors::Sectors;
use openshard_state::{
    Client, Dialogue, FacetState, Gameplay, GuildCandidate, GuildGumpContext, GuildId, GuildMember,
    GuildPage, Obstructions, QuestDefs, Regions, Relation, TargetPurpose, WorldState,
};

use crate::{Outcome, Refusal, may_lead, roster};

/// One tile of nothing, which is all any rule here needs to stand on.
const SIZE: u32 = 8;

fn world() -> WorldState {
    let mut facets = BTreeMap::new();
    facets.insert(
        Facet(0),
        FacetState {
            terrain: None,
            coarse: None,
            width: SIZE,
            height: SIZE,
            sectors: Sectors::new(SIZE, SIZE),
            obstructions: Obstructions::default(),
            regions: Regions::new(SIZE, SIZE),
            banks: Banks::default(),
        },
    );
    WorldState {
        registry: Registry::new(),
        bus: EventBus::new(),
        facets,
        default_facet: Facet(0),
        players: HashMap::new(),
        connections: HashMap::new(),
        seen: HashMap::new(),
        start: (0, 0),
        rng: Rng::new(1),
        ticks: 0,
        hour: 0,
        worn: Default::default(),
        outbox: Vec::new(),
        open_containers: HashMap::new(),
        trades: Vec::new(),
        quests: QuestDefs::default(),
        dialogue: Dialogue::default(),
        guilds: openshard_state::Guilds::default(),
        gameplay: Gameplay::default(),
        save_requested: false,
    }
}

/// A mobile with a serial, which is all a guild records about a member.
fn mobile(state: &mut WorldState) -> EntityId {
    let (entity, _) = state
        .registry
        .spawn_with_serial(SerialKind::Mobile)
        .expect("the mobile pool is not exhausted");
    entity
}

/// Found a guild with one member, the common opening.
fn a_guild(state: &mut WorldState) -> (EntityId, GuildId) {
    let leader = mobile(state);
    let guild = crate::found(state, leader, "The Silver Serpent", "OSS").expect("a first guild");
    (leader, guild)
}

/// A mobile with a client behind it, which is what a window needs: a `Client`
/// component, a session row to hang the context on, and a `players` entry so a
/// reply can find who sent it.
fn player(state: &mut WorldState, id: u64) -> (EntityId, ConnectionId) {
    let entity = mobile(state);
    let connection = ConnectionId::from_raw(id);
    state.connections.insert(
        connection,
        Connection::new(
            ClientVersion::new(7, 0, 0, 0),
            AccountName::new("tester"),
            AccessLevel::Player,
        ),
    );
    state.players.insert(connection, entity);
    state.registry.insert(entity, Client { connection });
    (entity, connection)
}

/// The `0xB1` a client would send back: our gump, one button, and whatever was
/// typed into the fields.
fn reply(button: ButtonId, fields: &[(u16, &str)]) -> GumpResponse {
    GumpResponse {
        serial: RawGumpKey(0),
        gump_id: RawGumpId(crate::GUILD_GUMP.0),
        button: RawButtonId(button.0),
        switches: Vec::new(),
        text_entries: fields.iter().map(|&(id, text)| (id, text.to_owned())).collect(),
    }
}

/// What the window last drew for this player.
fn context(state: &WorldState, player: EntityId) -> Option<GuildGumpContext> {
    state.row_of(player).and_then(|row| row.guild_gump.clone())
}

#[test]
fn founding_a_guild_makes_the_founder_its_leader() {
    let mut state = world();
    let (leader, guild) = a_guild(&mut state);
    assert_eq!(may_lead(&state, leader), Ok(guild));
    assert_eq!(state.guild_of(leader).map(|g| g.id), Some(guild));
    assert_eq!(roster(&state, guild), vec![leader]);
}

#[test]
fn a_name_or_an_abbreviation_belongs_to_one_guild() {
    // The abbreviation is drawn in brackets beside a name. Two guilds sharing one
    // would make the bracket a lie, and there is nothing on screen to tell them
    // apart by.
    let mut state = world();
    a_guild(&mut state);
    let second = mobile(&mut state);
    assert_eq!(
        crate::found(&mut state, second, "the silver serpent", "TBR"),
        Err(Refusal::NameTaken),
        "case is not what makes two guilds different"
    );
    assert_eq!(
        crate::found(&mut state, second, "The Black Rose", "oss"),
        Err(Refusal::AbbreviationTaken)
    );
    assert_eq!(
        crate::found(&mut state, second, "   ", "TBR"),
        Err(Refusal::NoName)
    );
    // And the founder of one may not found another.
    let leader = state.guilds.iter().next().expect("the first guild").leader;
    let founder = state.registry.entity_of(leader).expect("its leader");
    assert_eq!(
        crate::found(&mut state, founder, "The Black Rose", "TBR"),
        Err(Refusal::AlreadyInAGuild)
    );
}

#[test]
fn a_guild_may_not_conscript() {
    // An invitation is a question, not a membership. The difference matters
    // because the answer is the player's, and a guild that could add people
    // without asking could turn a stranger orange to their own friends.
    let mut state = world();
    let (leader, guild) = a_guild(&mut state);
    let recruit = mobile(&mut state);

    crate::invite(&mut state, leader, recruit).expect("a leader may ask");
    assert!(state.guild_of(recruit).is_none(), "asking joined them");
    assert!(state.registry.has::<GuildCandidate>(recruit));

    assert_eq!(crate::accept_invitation(&mut state, recruit), Ok(guild));
    assert_eq!(state.guild_of(recruit).map(|g| g.id), Some(guild));
    assert!(
        !state.registry.has::<GuildCandidate>(recruit),
        "the invitation outlived the answer"
    );
}

#[test]
fn only_a_leader_asks() {
    let mut state = world();
    let (leader, _) = a_guild(&mut state);
    let member = mobile(&mut state);
    crate::invite(&mut state, leader, member).unwrap();
    crate::accept_invitation(&mut state, member).unwrap();

    let stranger = mobile(&mut state);
    assert_eq!(
        crate::invite(&mut state, member, stranger),
        Err(Refusal::NotTheLeader)
    );
    assert_eq!(
        crate::dismiss(&mut state, member, leader),
        Err(Refusal::NotTheLeader)
    );
    assert_eq!(
        crate::invite(&mut state, stranger, member),
        Err(Refusal::NotInAGuild)
    );
}

#[test]
fn an_invitation_does_not_outlive_the_guild() {
    // Disbanding does not walk the roster of people it merely *asked* — they are
    // not on it. So the stale invitation is caught when it is answered, and
    // cleared rather than left to be answered again.
    let mut state = world();
    let (leader, _) = a_guild(&mut state);
    let recruit = mobile(&mut state);
    crate::invite(&mut state, leader, recruit).unwrap();
    crate::disband(&mut state, leader).unwrap();

    assert_eq!(
        crate::accept_invitation(&mut state, recruit),
        Err(Refusal::NoSuchGuild)
    );
    assert!(
        !state.registry.has::<GuildCandidate>(recruit),
        "a question nobody is left to have asked"
    );
}

#[test]
fn a_leader_may_not_walk_out_on_a_guild_that_still_has_members() {
    // The guild would be left naming a leader who is not in it, and nothing could
    // appoint another.
    let mut state = world();
    let (leader, guild) = a_guild(&mut state);
    let member = mobile(&mut state);
    crate::invite(&mut state, leader, member).unwrap();
    crate::accept_invitation(&mut state, member).unwrap();

    assert_eq!(
        crate::leave(&mut state, leader),
        Err(Refusal::PassLeadershipFirst)
    );
    crate::pass_leadership(&mut state, leader, member).expect("handing it over");
    assert_eq!(may_lead(&state, member), Ok(guild));
    crate::leave(&mut state, leader).expect("no longer the leader");
    assert_eq!(roster(&state, guild), vec![member]);
}

#[test]
fn the_last_member_leaving_disbands_rather_than_orphans() {
    let mut state = world();
    let (leader, guild) = a_guild(&mut state);
    crate::leave(&mut state, leader).expect("the last one out");
    assert!(state.guilds.get(guild).is_none(), "a guild with nobody in it");
    assert!(state.guild_of(leader).is_none());
}

#[test]
fn a_title_is_clipped_and_may_be_taken_back() {
    let mut state = world();
    let (leader, _) = a_guild(&mut state);
    let long = "Grand Warlord of the Eastern Reaches";
    crate::set_title(&mut state, leader, leader, long).unwrap();
    let title = |state: &WorldState| {
        state
            .registry
            .get::<GuildMember>(leader)
            .map(|m| m.title.clone())
            .unwrap_or_default()
    };
    assert_eq!(title(&state), "Grand Warlord of the", "clipped to 20");
    assert_eq!(title(&state).chars().count(), crate::TITLE_LIMIT);

    // Clearing it is a thing a leader is allowed to say, not an error — refusing
    // it would leave no way to undo a title at all.
    crate::set_title(&mut state, leader, leader, "  ").unwrap();
    assert_eq!(title(&state), "");
}

#[test]
fn the_label_is_what_a_click_draws() {
    let mut state = world();
    let (leader, _) = a_guild(&mut state);
    assert_eq!(state.guild_label(leader).as_deref(), Some("[OSS]"));
    crate::set_title(&mut state, leader, leader, "Warlord").unwrap();
    assert_eq!(state.guild_label(leader).as_deref(), Some("[Warlord, OSS]"));
    assert_eq!(
        state.guild_title_of(leader),
        Some(("Warlord".to_owned(), "The Silver Serpent".to_owned()))
    );

    // And nothing at all for a mobile in no guild, which is almost every one.
    let stranger = mobile(&mut state);
    assert_eq!(state.guild_label(stranger), None);
}

#[test]
fn a_war_takes_two_declarations() {
    let mut state = world();
    let (ours, one) = a_guild(&mut state);
    let theirs = mobile(&mut state);
    let two = crate::found(&mut state, theirs, "The Black Rose", "TBR").unwrap();

    assert_eq!(
        crate::propose(&mut state, ours, two, Relation::War),
        Ok(Outcome::Offered)
    );
    assert_eq!(
        state.guilds.get(one).unwrap().toward(two),
        None,
        "one guild's word made a war"
    );
    assert_eq!(
        crate::propose(&mut state, theirs, one, Relation::War),
        Ok(Outcome::Declared)
    );
    assert_eq!(state.guilds.get(one).unwrap().toward(two), Some(Relation::War));

    // Peace is one guild's decision, not a second handshake: the alternative is a
    // guild that cannot stop being attacked because its attacker will not agree.
    crate::make_peace(&mut state, ours, two).expect("ending it");
    assert_eq!(state.guilds.get(one).unwrap().toward(two), None);
    assert_eq!(state.guilds.get(two).unwrap().toward(one), None);
}

#[test]
fn a_guild_declares_on_someone_else() {
    let mut state = world();
    let (ours, one) = a_guild(&mut state);
    assert_eq!(
        crate::propose(&mut state, ours, one, Relation::War),
        Err(Refusal::NoSuchGuild),
        "at war with itself"
    );
    assert_eq!(
        crate::make_peace(&mut state, ours, GuildId(999)),
        Err(Refusal::NoSuchGuild)
    );
}

#[test]
fn disbanding_clears_the_membership_it_leaves_behind() {
    // `guild_of` already reads a membership naming a dead guild as none, which is
    // what protects an offline member. This is the other half: the component goes
    // too, so nothing is left to be restored into a guild that is gone.
    let mut state = world();
    let (leader, guild) = a_guild(&mut state);
    let member = mobile(&mut state);
    crate::invite(&mut state, leader, member).unwrap();
    crate::accept_invitation(&mut state, member).unwrap();

    crate::disband(&mut state, leader).expect("the leader's to disband");
    assert!(state.guilds.get(guild).is_none());
    assert!(!state.registry.has::<GuildMember>(member));
    assert!(!state.registry.has::<GuildMember>(leader));
    assert_eq!(roster(&state, guild), Vec::<EntityId>::new());
}

#[test]
fn the_window_a_player_with_no_guild_gets_is_the_founding_form() {
    // And it is the only page they get: a stale button from a window drawn before
    // they left lands here rather than on an empty roster.
    let mut state = world();
    let (player, connection) = player(&mut state, 1);
    crate::open(&mut state, connection);
    let drawn = context(&state, player).expect("a window");
    assert_eq!(drawn.page, GuildPage::Main);

    crate::gump::show(&mut state, player, GuildPage::Diplomacy);
    assert_eq!(context(&state, player).expect("a window").page, GuildPage::Main);
}

#[test]
fn the_founding_form_founds_what_was_typed_into_it() {
    let mut state = world();
    let (player, connection) = player(&mut state, 1);
    crate::open(&mut state, connection);

    let typed = reply(
        crate::gump::button::FOUND,
        &[
            (crate::gump::FIELD_NAME as u16, "The Silver Serpent"),
            (crate::gump::FIELD_ABBREVIATION as u16, "OSS"),
        ],
    );
    assert!(crate::handle(&mut state, connection, &typed));
    let guild = state.guild_of(player).expect("a guild");
    assert_eq!(guild.name, "The Silver Serpent");
    assert_eq!(guild.abbreviation, "OSS");
}

#[test]
fn a_reply_to_a_window_this_side_never_opened_does_nothing() {
    // The gump id is not a secret and the button is whatever the client says. The
    // context is what makes the difference, and it is taken rather than read — so
    // the same reply twice is one action.
    let mut state = world();
    let (player, connection) = player(&mut state, 1);
    let typed = reply(
        crate::gump::button::FOUND,
        &[
            (crate::gump::FIELD_NAME as u16, "The Silver Serpent"),
            (crate::gump::FIELD_ABBREVIATION as u16, "OSS"),
        ],
    );
    assert!(
        crate::handle(&mut state, connection, &typed),
        "the reply is still ours to have refused"
    );
    assert!(state.guild_of(player).is_none(), "a guild from no window");

    crate::open(&mut state, connection);
    crate::handle(&mut state, connection, &typed);
    assert!(state.guild_of(player).is_some());
    // The second press finds no context. Without that, a name already taken would
    // be the only thing stopping it.
    crate::handle(&mut state, connection, &typed);
    assert_eq!(state.guilds.len(), 1);
}

#[test]
fn a_diplomacy_row_declares_on_the_guild_that_row_drew() {
    let mut state = world();
    let (leader, connection) = player(&mut state, 1);
    let own = crate::found(&mut state, leader, "The Silver Serpent", "OSS").unwrap();
    let other = mobile(&mut state);
    let theirs = crate::found(&mut state, other, "The Black Rose", "TBR").unwrap();

    crate::gump::show(&mut state, leader, GuildPage::Diplomacy);
    let drawn = context(&state, leader).expect("a window");
    assert_eq!(drawn.guilds, vec![theirs], "the page listed its own guild");

    let war = reply(crate::gump::row_button(crate::gump::DIPLOMACY_BASE, 0, 0), &[]);
    crate::handle(&mut state, connection, &war);
    assert_eq!(
        state.guilds.get(own).unwrap().offered(theirs),
        Some(Relation::War),
        "row zero declared on nobody"
    );
}

#[test]
fn a_row_the_window_never_drew_names_nobody() {
    let mut state = world();
    let (leader, connection) = player(&mut state, 1);
    crate::found(&mut state, leader, "The Silver Serpent", "OSS").unwrap();
    let other = mobile(&mut state);
    crate::found(&mut state, other, "The Black Rose", "TBR").unwrap();

    crate::gump::show(&mut state, leader, GuildPage::Diplomacy);
    // One guild was listed; row four is a number the client made up.
    let forged = reply(crate::gump::row_button(crate::gump::DIPLOMACY_BASE, 4, 0), &[]);
    crate::handle(&mut state, connection, &forged);
    assert!(
        state.guilds.iter().all(|guild| guild.proposals.is_empty()),
        "a forged row declared a war"
    );
}

#[test]
fn a_roster_row_sets_the_title_typed_beside_it() {
    let mut state = world();
    let (leader, connection) = player(&mut state, 1);
    crate::found(&mut state, leader, "The Silver Serpent", "OSS").unwrap();
    let member = mobile(&mut state);
    crate::invite(&mut state, leader, member).unwrap();
    crate::accept_invitation(&mut state, member).unwrap();

    crate::gump::show(&mut state, leader, GuildPage::Roster);
    let drawn = context(&state, leader).expect("a window");
    let row = drawn
        .members
        .iter()
        .position(|&serial| Some(serial) == state.registry.serial_of(member))
        .expect("the member was drawn");

    let set = reply(
        crate::gump::row_button(crate::gump::ROSTER_BASE, row, 0),
        &[(row as u16, "Warlord")],
    );
    crate::handle(&mut state, connection, &set);
    assert_eq!(
        state
            .registry
            .get::<GuildMember>(member)
            .map(|m| m.title.as_str()),
        Some("Warlord")
    );
}

#[test]
fn the_invite_button_raises_a_cursor_only_for_a_leader() {
    let mut state = world();
    let (leader, connection) = player(&mut state, 1);
    crate::found(&mut state, leader, "The Silver Serpent", "OSS").unwrap();
    crate::gump::show(&mut state, leader, GuildPage::Main);
    crate::handle(&mut state, connection, &reply(crate::gump::button::INVITE, &[]));
    assert_eq!(state.take_target(leader), Some(TargetPurpose::GuildInvite));

    // A plain member pressing the same button — the window can outlive the rank
    // that drew it, and hiding a button hides it on one screen only.
    let (member, member_connection) = player(&mut state, 2);
    crate::invite(&mut state, leader, member).unwrap();
    crate::accept_invitation(&mut state, member).unwrap();
    crate::gump::show(&mut state, member, GuildPage::Main);
    crate::handle(
        &mut state,
        member_connection,
        &reply(crate::gump::button::INVITE, &[]),
    );
    assert_eq!(state.take_target(member), None, "a member raised a cursor");
}
