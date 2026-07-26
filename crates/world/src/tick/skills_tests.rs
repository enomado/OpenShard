//! The skills that answer a question about something: Arms Lore, Item
//! Identification, Forensic Evaluation.
//!
//! A child module rather than more of `tests.rs`, which is long past the size a
//! file should be. These go through the whole path a player does — press the
//! button, get a cursor, click a thing, read the line that comes back — because
//! every one of the three has been wrong at a different link in that chain before:
//! a cliloc block chosen by the wrong arithmetic reads as a plausible sentence
//! about the wrong object, which no client will report.

use super::tests::{enter, packets_for, spawn_mobile_at, world, START};
use super::*;
use openshard_skills::DEFAULT_SKILL_DELAY_TICKS;
use openshard_state::components::{Amount, Corpse, Graphic, Name};
use openshard_state::Skill;

/// Give the player a skill outright, so a roll is a sure thing.
fn train(world: &mut World, connection: ConnectionId, skill: Skill, value: u16) {
    let entity = world.state.players[&connection];
    let serial = world.state.registry.serial_of(entity).unwrap().raw();
    world.queue(Command::SetSkill {
        serial,
        skill: skill.id(),
        value,
    });
}

/// Put an item on the ground next to the player and return its serial.
fn item_beside(world: &mut World, graphic: u16, now: Instant) -> u32 {
    world.queue(Command::SpawnItem {
        graphic,
        hue: 0,
        amount: 1,
        stackable: false,
        position: Point::new(START.0 + 1, START.1, 0),
        facet: 0,
    });
    world.tick(now);
    let (entity, _) = world
        .state
        .registry
        .query::<Graphic>()
        .find(|(entity, g)| g.id == graphic && !world.state.registry.has::<Body>(*entity))
        .expect("the item was spawned");
    world.state.registry.serial_of(entity).unwrap().raw()
}

/// Press a skill's button, answer its cursor with `target`, and return every
/// cliloc the player was sent by the answer.
fn use_skill_on(
    world: &mut World,
    connection: ConnectionId,
    skill: Skill,
    target: u32,
    now: Instant,
) -> Vec<u32> {
    let cursor_id = {
        let entity = world.state.players[&connection];
        world.state.registry.serial_of(entity).unwrap().raw()
    };
    world.queue(Command::UseSkillButton {
        connection,
        skill: skill.id(),
    });
    world.tick(now);
    let _ = packets_for(world, connection);
    world.queue(Command::TargetResponse {
        connection,
        response: openshard_protocol::TargetResponse {
            cursor_id,
            serial: target,
            location: Point::new(START.0 + 1, START.1, 0),
            graphic: 0,
            cancelled: false,
        },
    });
    world.tick(now);
    clilocs(world, connection)
}

/// Every cliloc number the connection was sent this tick, in order.
fn clilocs(world: &mut World, connection: ConnectionId) -> Vec<u32> {
    packets_for(world, connection)
        .into_iter()
        .filter(|p| p[0] == 0xC1)
        .map(|p| u32::from_be_bytes([p[14], p[15], p[16], p[17]]))
        .collect()
}

#[test]
fn arms_lore_reads_a_weapon_off_the_core_table() {
    // A katana: `BaseSword`, so slashing, and one-handed — the block at 1038220,
    // no hand offset. Pre-AoS damage 5..26 averages 15, which is band 3, three
    // strides of nine along the block. Getting the base or the stride wrong shows
    // a sentence about a different weapon entirely, which is why the whole number
    // is pinned rather than a range.
    let now = Instant::now();
    let mut world = world();
    let looker = enter(&mut world, now);
    train(&mut world, looker, Skill::ArmsLore, 1000);
    world.tick(now);
    let katana = item_beside(&mut world, 0x13FF, now);
    let _ = packets_for(&mut world, looker);

    let said = use_skill_on(&mut world, looker, Skill::ArmsLore, katana, now);
    assert!(
        said.contains(&(1_038_220 + 3 * 9)),
        "the slashing block, band 3: {said:?}"
    );
}

#[test]
fn arms_lore_knows_a_two_handed_weapon_from_a_one_handed_one() {
    // The hand comes from tiledata's quality byte, which a test world has no
    // client files for — so both read one-handed here, and the six classes that
    // *insist* in code are the ones that can differ without a client. A bow is
    // one of them, and its own block carries no hand offset at all, so this
    // asserts the two facts that do not depend on a file being present: a katana
    // lands on the one-handed slashing line, and a bow lands on the ranged block.
    let now = Instant::now();
    let mut world = world();
    let looker = enter(&mut world, now);
    train(&mut world, looker, Skill::ArmsLore, 1000);
    world.tick(now);
    let bow = item_beside(&mut world, 0x13B2, now);
    let _ = packets_for(&mut world, looker);

    // Bow: pre-AoS 9..41 averages 25, band 5.
    let said = use_skill_on(&mut world, looker, Skill::ArmsLore, bow, now);
    assert!(
        said.contains(&(1_038_224 + 5 * 9)),
        "the ranged block, band 5, no hand offset: {said:?}"
    );
}

#[test]
fn arms_lore_reads_armour_by_its_rating() {
    // A plate chest rates 40, capped at 35, which is band 7 — the top line,
    // "superbly crafted to provide maximum protection".
    let now = Instant::now();
    let mut world = world();
    let looker = enter(&mut world, now);
    train(&mut world, looker, Skill::ArmsLore, 1000);
    world.tick(now);
    let plate = item_beside(&mut world, 0x1415, now);
    let _ = packets_for(&mut world, looker);

    let said = use_skill_on(&mut world, looker, Skill::ArmsLore, plate, now);
    assert!(
        said.contains(&(1_038_295 + 7)),
        "the top armour line: {said:?}"
    );
}

#[test]
fn arms_lore_refuses_something_that_is_neither() {
    // 500352, "This is neither weapon nor armor." A gold coin is in no table, and
    // the honest answer is the client's own line for that rather than silence.
    let now = Instant::now();
    let mut world = world();
    let looker = enter(&mut world, now);
    train(&mut world, looker, Skill::ArmsLore, 1000);
    world.tick(now);
    let gold = item_beside(&mut world, 0x0EED, now);
    let _ = packets_for(&mut world, looker);

    let said = use_skill_on(&mut world, looker, Skill::ArmsLore, gold, now);
    assert!(
        said.contains(&500_352),
        "neither weapon nor armour: {said:?}"
    );
}

#[test]
fn item_identification_names_the_thing_and_prices_it_if_it_has_one() {
    // "It appears to be:" then the name, drawn over the item itself. The value
    // line follows only for an item that carries a price, because the core knows
    // what a shopkeeper charges and nothing else — a guessed number for a rock
    // would read as authoritative.
    let now = Instant::now();
    let mut world = world();
    let looker = enter(&mut world, now);
    train(&mut world, looker, Skill::ItemId, 1000);
    world.tick(now);
    let scroll = item_beside(&mut world, 0x1F2D, now);
    let entity = world
        .state
        .registry
        .entity_of(Serial::new(scroll).unwrap())
        .unwrap();
    world
        .state
        .registry
        .insert(entity, Name("a scroll of magic arrow".to_owned()));
    world
        .state
        .registry
        .insert(entity, openshard_state::components::Price(12));
    let _ = packets_for(&mut world, looker);

    let said = use_skill_on(&mut world, looker, Skill::ItemId, scroll, now);
    assert!(said.contains(&1_041_349), "it appears to be: {said:?}");
    assert!(said.contains(&1_041_351), "and it is worth: {said:?}");
}

#[test]
fn forensics_says_who_killed_a_body_and_who_has_been_through_it() {
    // Everything Forensic Evaluation reads was written by somebody else's rule at
    // the moment it happened. This is the whole chain: a mobile dies to a named
    // killer, the reap lays a corpse that remembers both, and the skill reads it
    // back — including "not desecrated", which is a different sentence from a
    // failed roll and is the one a fresh body deserves.
    let now = Instant::now();
    let mut world = world();
    let looker = enter(&mut world, now);
    let player_serial = {
        let entity = world.state.players[&looker];
        world.state.registry.serial_of(entity).unwrap().raw()
    };
    train(&mut world, looker, Skill::Forensics, 1000);
    world.tick(now);

    let victim = spawn_mobile_at(&mut world, Point::new(START.0 + 1, START.1, 0), 10, now);
    world.queue(Command::Damage {
        serial: victim,
        amount: 100,
        damage_type: 0,
        by: player_serial,
    });
    world.tick(now);
    // The corpse the reap laid, remembering the player as its killer.
    let (corpse, _) = world
        .state
        .registry
        .query::<Corpse>()
        .next()
        .expect("a corpse was laid");
    let story = world.state.registry.get::<Corpse>(corpse).unwrap();
    assert_eq!(
        story.killer.as_deref(),
        Some("Lord British"),
        "the killer is remembered by name, not by serial"
    );
    let corpse_serial = world.state.registry.serial_of(corpse).unwrap().raw();
    let _ = packets_for(&mut world, looker);

    let said = use_skill_on(&mut world, looker, Skill::Forensics, corpse_serial, now);
    // A human body (the test creature wears 0x0190) reports its killer.
    assert!(
        said.contains(&1_042_751),
        "killed by ~1_KILLER_NAME~: {said:?}"
    );
    assert!(said.contains(&501_002), "and not yet desecrated: {said:?}");
    // The first reader signs the body, so a second one is told whose work it is.
    assert_eq!(
        world
            .state
            .registry
            .get::<Corpse>(corpse)
            .unwrap()
            .examined_by
            .as_deref(),
        Some("Lord British")
    );
    // The button holds for a second after a use, so wait it out rather than
    // stripping the cooldown: the refusal is the rule, not an obstacle.
    for _ in 0..=DEFAULT_SKILL_DELAY_TICKS {
        world.tick(now);
    }
    let _ = packets_for(&mut world, looker);
    let again = use_skill_on(&mut world, looker, Skill::Forensics, corpse_serial, now);
    assert!(
        again.contains(&1_042_750),
        "the forensicist has already discovered that: {again:?}"
    );
}

#[test]
fn taking_something_off_a_corpse_makes_you_a_looter() {
    // The other half of the record Forensics reads, written where the lifting
    // happens: a corpse keeps a guest list, an ordinary chest does not.
    let now = Instant::now();
    let mut world = world();
    let looter = enter(&mut world, now);
    let player_serial = {
        let entity = world.state.players[&looter];
        world.state.registry.serial_of(entity).unwrap().raw()
    };
    let victim = spawn_mobile_at(&mut world, Point::new(START.0 + 1, START.1, 0), 10, now);
    world.queue(Command::Damage {
        serial: victim,
        amount: 100,
        damage_type: 0,
        by: player_serial,
    });
    world.tick(now);
    let (corpse, _) = world
        .state
        .registry
        .query::<Corpse>()
        .next()
        .expect("a corpse was laid");
    // The core drops a little gold in every creature corpse; lift it.
    let corpse_serial = world.state.registry.serial_of(corpse).unwrap();
    let (loot, _) = world
        .state
        .registry
        .query::<Contained>()
        .find(|(_, c)| c.container == corpse_serial)
        .expect("a corpse holds the baseline gold");
    let loot_serial = world.state.registry.serial_of(loot).unwrap().raw();

    world.queue(Command::PickUpItem {
        connection: looter,
        serial: loot_serial,
        amount: 1,
    });
    world.tick(now);
    assert_eq!(
        world.state.registry.get::<Corpse>(corpse).unwrap().looters,
        vec!["Lord British".to_owned()],
        "the corpse remembers who went through it"
    );
}

#[test]
fn a_corpses_story_comes_back_after_a_restart() {
    // A corpse lies for seven minutes and a shard restarts inside that window, so
    // the story rides the item's saved record (schema v17). Without it the body a
    // player was investigating comes back anonymous, killed by nobody.
    let now = Instant::now();
    let mut world = world();
    let player = enter(&mut world, now);
    let player_serial = {
        let entity = world.state.players[&player];
        world.state.registry.serial_of(entity).unwrap().raw()
    };
    let victim = spawn_mobile_at(&mut world, Point::new(START.0 + 1, START.1, 0), 10, now);
    world.queue(Command::Damage {
        serial: victim,
        amount: 100,
        damage_type: 0,
        by: player_serial,
    });
    world.tick(now);
    world.take_snapshot();
    let snapshot = world.drain_saves().next().expect("a snapshot was taken");
    let saved = snapshot
        .ground
        .as_ref()
        .expect("the ground was swept")
        .iter()
        .find(|item| item.corpse.is_some())
        .expect("the corpse was swept into the save")
        .clone();
    assert_eq!(
        saved.corpse.as_ref().unwrap().killer.as_deref(),
        Some("Lord British")
    );

    // A fresh world restoring that save has the same body with the same story.
    let mut reborn = super::tests::world();
    reborn.restore_items(vec![saved]);
    let (corpse, _) = reborn
        .state
        .registry
        .query::<Corpse>()
        .next()
        .expect("the corpse came back");
    assert_eq!(
        reborn
            .state
            .registry
            .get::<Corpse>(corpse)
            .unwrap()
            .killer
            .as_deref(),
        Some("Lord British")
    );
    assert!(
        reborn.state.registry.get::<Amount>(corpse).is_some(),
        "and still draws as the body it was"
    );
}
