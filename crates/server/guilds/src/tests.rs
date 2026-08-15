//! The rules, over a world with nothing in it but mobiles.
//!
//! A bare [`WorldState`]: no map, no clients, no packets going anywhere. That is
//! enough to test every rule here, because none of them look at the ground —
//! and it means the assertions are about guilds rather than about a world that
//! had to be stood up first. What the packets *do* is
//! `world/src/tick/tests.rs`' business, next to the notoriety tests they change.

use std::collections::{BTreeMap, HashMap};

use openshard_entities::{EntityId, Registry};
use openshard_events::EventBus;
use openshard_protocol::serial::SerialKind;
use openshard_protocol::world::Facet;
use openshard_state::harvest::Banks;
use openshard_state::rng::Rng;
use openshard_state::sectors::Sectors;
use openshard_state::{
    Dialogue, FacetState, Gameplay, GuildCandidate, GuildId, GuildMember, Obstructions, QuestDefs, Regions,
    Relation, WorldState,
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
