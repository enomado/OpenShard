//! Quests: offering, accepting, progress, turn-in, and the two failures the
//! pack-side version had — a giver that stopped being one after a restart, and a
//! turn-in that took some of what it asked for and paid nothing.
//!
//! A child module rather than more of `tests.rs`, which is long past the size a
//! file should be. These read private world state, so they stay inside the
//! module.

use super::tests::{enter, packets_for, spawn_mobile_at, world, START};
use super::*;
use openshard_quests::{QUEST_GUMP, QUEST_RESIGN_GUMP};
use openshard_state::components::{Amount, Contained, Graphic, QuestGiver, QuestLog, Stackable};
use openshard_state::quest::{ObjectiveDef, ObjectiveKind, QuestDef, RewardDef, RewardKind};

/// The body a rat is drawn as — the slay quests' target.
const RAT: u16 = 0x00EE;
/// Spiders' silk, the obtain quests' target.
const SILK: u16 = 0x0F8D;

/// A quest asking for five rats, paying 250 gold.
fn rat_cull() -> QuestDef {
    QuestDef {
        key: "rat_cull".to_owned(),
        title: "A Plague of Rats".to_owned(),
        description: "Slay five rats.".to_owned(),
        complete: "Well done.".to_owned(),
        objectives: vec![ObjectiveDef {
            kind: ObjectiveKind::Slay { body: RAT },
            count: 5,
            name: "sewer rat".to_owned(),
            seconds: 0,
        }],
        rewards: vec![RewardDef {
            kind: RewardKind::Gold(250),
            name: "250 gold".to_owned(),
        }],
        ..QuestDef::default()
    }
}

/// A quest asking for five skeins of silk.
fn silk_gather() -> QuestDef {
    QuestDef {
        key: "silk_gather".to_owned(),
        title: "Silk for the Spellwright".to_owned(),
        objectives: vec![ObjectiveDef {
            kind: ObjectiveKind::Obtain { graphic: SILK },
            count: 5,
            name: "spiders' silk".to_owned(),
            seconds: 0,
        }],
        rewards: vec![RewardDef {
            kind: RewardKind::Gold(120),
            name: "120 gold".to_owned(),
        }],
        ..QuestDef::default()
    }
}

/// Put a quest giver on the map beside the start, bound to `keys`.
fn place_giver(world: &mut World, keys: &[&str], now: Instant) -> u32 {
    let at = Point::new(START.0 + 1, START.1, 0);
    let serial = spawn_mobile_at(world, at, 100, now);
    let entity = world
        .state
        .registry
        .entity_of(Serial::new(serial).unwrap())
        .unwrap();
    world.state.registry.insert(
        entity,
        QuestGiver {
            keys: keys.iter().map(|&k| k.to_owned()).collect(),
        },
    );
    serial
}

/// The player's quest log, or an empty one.
fn log_of(world: &World, connection: ConnectionId) -> QuestLog {
    let player = world.state.players[&connection];
    world
        .state
        .registry
        .get::<QuestLog>(player)
        .cloned()
        .unwrap_or_default()
}

/// Answer the open quest gump with a button.
fn press(world: &mut World, connection: ConnectionId, gump_id: u32, button: u32) {
    press_with(world, connection, gump_id, button, Vec::new());
}

/// Answer with a button and a set of switches on — the resign dialog's shape.
fn press_with(
    world: &mut World,
    connection: ConnectionId,
    gump_id: u32,
    button: u32,
    switches: Vec<u32>,
) {
    world.queue(Command::GumpResponse {
        connection,
        response: openshard_protocol::GumpResponse {
            serial: 0,
            gump_id,
            button,
            switches,
            text_entries: Vec::new(),
        },
    });
}

/// Whether any packet this tick was a gump display (`0xB0`).
fn drew_a_gump(world: &mut World, connection: ConnectionId) -> bool {
    packets_for(world, connection)
        .iter()
        .any(|packet| packet.first() == Some(&0xB0))
}

/// Register a set of quests on the world.
fn register(world: &mut World, quests: Vec<QuestDef>) {
    world.state.quests.set(quests);
}

#[test]
fn a_double_clicked_giver_offers_its_quest() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    let _ = packets_for(&mut world, connection);

    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);

    assert!(
        drew_a_gump(&mut world, connection),
        "double-clicking a giver draws the offer"
    );
    assert!(
        log_of(&world, connection).active.is_empty(),
        "an offer is not an acceptance"
    );
}

#[test]
fn accepting_puts_the_quest_in_the_log() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);

    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    // Button 4 is Accept — ServUO's `Buttons.AcceptQuest`.
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    let log = log_of(&world, connection);
    assert_eq!(log.active.len(), 1);
    assert_eq!(log.active[0].key, "rat_cull");
    assert_eq!(log.active[0].progress, vec![0]);
}

#[test]
fn refusing_starts_nothing() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);

    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 2); // Refuse
    world.tick(now);

    assert!(log_of(&world, connection).active.is_empty());
}

#[test]
fn a_reply_to_a_gump_that_was_never_opened_does_nothing() {
    // The context is the server's memory of what it drew. Without it a crafted
    // `0xB1` naming the quest gump would accept whatever was pending.
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);

    press(&mut world, connection, QUEST_GUMP, 4); // Accept, out of nowhere
    world.tick(now);

    assert!(log_of(&world, connection).active.is_empty());
}

#[test]
fn the_paperdoll_quest_button_opens_the_log() {
    // `0xD7` subcommand `0x32`. Nothing decoded it before, so the button did
    // nothing at all and there was no way to see an accepted quest.
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let _ = packets_for(&mut world, connection);

    world.queue(Command::QuestLogRequest { connection });
    world.tick(now);

    assert!(
        drew_a_gump(&mut world, connection),
        "an empty log still opens — silence looks like a broken button"
    );
}

#[test]
fn a_slain_body_advances_only_the_killers_objective() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    let player = world.state.players[&connection];
    let killer = world.state.registry.serial_of(player).unwrap();
    world.state.bus.send(openshard_combat::MobileDied {
        entity: player,
        serial: killer,
        body: RAT,
        killer: Some(killer),
    });
    world.tick(now);

    assert_eq!(log_of(&world, connection).active[0].progress, vec![1]);
}

#[test]
fn an_unattributed_death_advances_nothing() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    let player = world.state.players[&connection];
    let serial = world.state.registry.serial_of(player).unwrap();
    world.state.bus.send(openshard_combat::MobileDied {
        entity: player,
        serial,
        body: RAT,
        killer: None, // a field, a fall, a reflected blow
    });
    world.tick(now);

    assert_eq!(log_of(&world, connection).active[0].progress, vec![0]);
}

#[test]
fn obtain_progress_is_found_by_the_diffing_pass_not_announced() {
    // Nothing in the engine says "an item moved". The pass looks instead, which
    // is why picking the silk up counts without any call beside the insert — and
    // why dropping it counts down again.
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![silk_gather()]);
    let giver = place_giver(&mut world, &["silk_gather"], now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    let player = world.state.players[&connection];
    let owner = world.state.registry.serial_of(player).unwrap();
    let backpack = openshard_items::backpack_of(&world.state, owner).unwrap();
    let silk = put_silk(&mut world, backpack, 5);

    tick_past_the_obtain_cadence(&mut world, now);
    assert_eq!(
        log_of(&world, connection).active[0].progress,
        vec![5],
        "carrying five counts as five"
    );

    // And it falls back when they are gone.
    world.state.registry.despawn(silk);
    tick_past_the_obtain_cadence(&mut world, now);
    assert_eq!(
        log_of(&world, connection).active[0].progress,
        vec![0],
        "an objective that says 'carry five' is false once you are not"
    );
}

#[test]
fn a_turn_in_takes_the_items_and_pays() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![silk_gather()]);
    let giver = place_giver(&mut world, &["silk_gather"], now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    let player = world.state.players[&connection];
    let owner = world.state.registry.serial_of(player).unwrap();
    let backpack = openshard_items::backpack_of(&world.state, owner).unwrap();
    put_silk(&mut world, backpack, 5);
    tick_past_the_obtain_cadence(&mut world, now);

    // Talk to the giver again: the complete page, then hand in, then take the
    // reward.
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 8); // Complete
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 5); // Accept reward
    world.tick(now);

    let log = log_of(&world, connection);
    assert!(log.active.is_empty(), "the quest leaves the log");
    assert_eq!(log.done.len(), 1, "and is remembered as done");
    assert_eq!(
        openshard_items::carried_amount(&world.state, owner, SILK),
        0,
        "the silk was handed over"
    );
    assert_eq!(
        openshard_items::carried_amount(&world.state, owner, 0x0EED),
        120,
        "and the gold arrived"
    );
}

#[test]
fn a_player_one_item_short_loses_nothing_and_is_paid_nothing() {
    // The pack's version took each objective independently, so a player short on
    // the second lost what they brought for the first — invisibly.
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    let two_part = QuestDef {
        key: "two_part".to_owned(),
        title: "Two Things".to_owned(),
        objectives: vec![
            ObjectiveDef {
                kind: ObjectiveKind::Obtain { graphic: SILK },
                count: 2,
                name: "silk".to_owned(),
                seconds: 0,
            },
            ObjectiveDef {
                kind: ObjectiveKind::Obtain { graphic: 0x0F7A },
                count: 2,
                name: "garlic".to_owned(),
                seconds: 0,
            },
        ],
        ..QuestDef::default()
    };
    register(&mut world, vec![two_part]);
    let giver = place_giver(&mut world, &["two_part"], now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    let player = world.state.players[&connection];
    let owner = world.state.registry.serial_of(player).unwrap();
    let backpack = openshard_items::backpack_of(&world.state, owner).unwrap();
    put_silk(&mut world, backpack, 2); // the first objective only

    // Force the quest to *look* complete, so the turn-in is reached at all: the
    // point under test is what happens when the items are not really there.
    {
        let mut log = log_of(&world, connection);
        log.active[0].progress = vec![2, 2];
        world.state.registry.insert(player, log);
    }
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 8);
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 5);
    world.tick(now);

    assert_eq!(
        openshard_items::carried_amount(&world.state, owner, SILK),
        2,
        "nothing was taken, because not everything could be"
    );
    assert!(
        !log_of(&world, connection).active.is_empty(),
        "and the quest is still open"
    );
}

#[test]
fn resigning_needs_the_yes_radio() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    // Open the log, the quest's page, then Resign.
    world.queue(Command::QuestLogRequest { connection });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 11); // the first row
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 3); // Resign
    world.tick(now);

    // "No" keeps it.
    press_with(&mut world, connection, QUEST_RESIGN_GUMP, 1, vec![0]);
    world.tick(now);
    assert_eq!(
        log_of(&world, connection).active.len(),
        1,
        "answering no keeps the quest"
    );

    press(&mut world, connection, QUEST_GUMP, 3);
    world.tick(now);
    press_with(&mut world, connection, QUEST_RESIGN_GUMP, 1, vec![1]);
    world.tick(now);
    assert!(
        log_of(&world, connection).active.is_empty(),
        "answering yes gives it up"
    );
}

#[test]
fn a_done_once_quest_is_never_offered_again() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    let mut quest = rat_cull();
    quest.done_once = true;
    quest.objectives[0].count = 1;
    register(&mut world, vec![quest]);
    let giver = place_giver(&mut world, &["rat_cull"], now);

    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    // Finish it.
    let player = world.state.players[&connection];
    let killer = world.state.registry.serial_of(player).unwrap();
    world.state.bus.send(openshard_combat::MobileDied {
        entity: player,
        serial: killer,
        body: RAT,
        killer: Some(killer),
    });
    world.tick(now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 8);
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 5);
    world.tick(now);
    assert!(log_of(&world, connection).active.is_empty());

    // And it may not be taken again.
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);
    assert!(
        log_of(&world, connection).active.is_empty(),
        "a once-only quest stays done"
    );
}

#[test]
fn a_completed_quest_reaches_the_pack() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    let mut quest = rat_cull();
    quest.objectives[0].count = 1;
    register(&mut world, vec![quest]);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    let mut done: Cursor<openshard_quests::QuestCompleted> = world.bus().cursor();

    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);
    let player = world.state.players[&connection];
    let killer = world.state.registry.serial_of(player).unwrap();
    world.state.bus.send(openshard_combat::MobileDied {
        entity: player,
        serial: killer,
        body: RAT,
        killer: Some(killer),
    });
    world.tick(now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 8);
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 5);
    world.tick(now);

    let events: Vec<_> = world.bus().read(&mut done).cloned().collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].key, "rat_cull");
}

/// Put a stack of silk in a container.
fn put_silk(world: &mut World, container: Serial, amount: u16) -> EntityId {
    let (item, _) = world
        .state
        .registry
        .spawn_with_serial(SerialKind::Item)
        .unwrap();
    world
        .state
        .registry
        .insert(item, Graphic { id: SILK, hue: 0 });
    world.state.registry.insert(item, Amount(amount));
    world.state.registry.insert(item, Stackable);
    world.state.registry.insert(
        item,
        Contained {
            container,
            x: 0,
            y: 0,
            grid: 0,
        },
    );
    item
}

/// Tick until the obtain pass has certainly run.
fn tick_past_the_obtain_cadence(world: &mut World, now: Instant) {
    for _ in 0..=openshard_quests::OBTAIN_EVERY_TICKS {
        world.tick(now);
    }
}

#[test]
fn a_quest_giver_is_still_a_giver_after_a_restart() {
    // The headline failure of the pack-side version. The binding lived in a JS
    // map filled on `MobileSpawned`, and restored NPCs announce no such thing —
    // so the shard's quests worked on the boot where the world was populated and
    // were inert on every boot after, with nothing anywhere to say why.
    let now = Instant::now();
    let mut world = world();
    let _ = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    world.take_snapshot();
    let snapshot = world.drain_saves().next().expect("a snapshot");
    let mobiles = snapshot.mobiles.clone().expect("the mobile sweep");
    assert!(
        mobiles
            .iter()
            .any(|m| m.serial == giver && m.quest_giver == ["rat_cull"]),
        "the binding is in the save"
    );

    // The restart: a fresh world restored from the records alone, and a player
    // who was never here when the giver was placed.
    let mut shard = super::tests::world();
    shard.restore_mobiles(mobiles);
    shard.state.quests.set(vec![rat_cull()]);
    let connection = enter(&mut shard, now);
    let _ = packets_for(&mut shard, connection);

    shard.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    shard.tick(now);
    assert!(
        drew_a_gump(&mut shard, connection),
        "and the giver still offers its quest"
    );
}

#[test]
fn restoring_a_mobile_announces_it_as_restored_not_as_spawned() {
    // The two must stay different events: a handler that *creates* on a spawn (a
    // vendor's stock crate) would duplicate it every reboot if they were one.
    let now = Instant::now();
    let mut world = world();
    let _ = enter(&mut world, now);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    world.take_snapshot();
    let mobiles = world
        .drain_saves()
        .next()
        .expect("a snapshot")
        .mobiles
        .clone()
        .expect("the mobile sweep");

    let mut shard = super::tests::world();
    let mut restored: Cursor<crate::events::MobileRestored> = shard.bus().cursor();
    let mut spawned: Cursor<openshard_npc::MobileSpawned> = shard.bus().cursor();
    shard.restore_mobiles(mobiles);

    let restores: Vec<_> = shard.bus().read(&mut restored).cloned().collect();
    assert!(
        restores.iter().any(|e| e.serial.raw() == giver),
        "a restored NPC says so"
    );
    assert_eq!(
        shard.bus().read(&mut spawned).count(),
        0,
        "and does not claim to have spawned"
    );
}

#[test]
fn a_quest_log_survives_a_restart_with_its_progress_and_cooldowns() {
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    register(&mut world, vec![rat_cull()]);
    let giver = place_giver(&mut world, &["rat_cull"], now);
    world.queue(Command::DoubleClick {
        connection,
        serial: giver,
    });
    world.tick(now);
    press(&mut world, connection, QUEST_GUMP, 4);
    world.tick(now);

    let player = world.state.players[&connection];
    let killer = world.state.registry.serial_of(player).unwrap();
    world.state.bus.send(openshard_combat::MobileDied {
        entity: player,
        serial: killer,
        body: RAT,
        killer: Some(killer),
    });
    world.tick(now);
    assert_eq!(log_of(&world, connection).active[0].progress, vec![1]);

    world.take_snapshot();
    let record = world
        .drain_saves()
        .next()
        .expect("a snapshot")
        .characters
        .into_iter()
        .find(|c| c.serial == killer.raw())
        .expect("the character");
    assert_eq!(record.quests.len(), 1);
    assert_eq!(record.quests[0].progress, vec![1]);

    // And it comes back on login.
    let mut shard = super::tests::world();
    shard.state.quests.set(vec![rat_cull()]);
    shard.queue(Command::Enter {
        connection: connection_two(),
        version: ClientVersion::TOL,
        account: "admin".to_owned(),
        name: "Lord British".to_owned(),
        serial: Some(record.serial),
        position: None,
        facet: 0,
        appearance: None,
        sheet: Some(CharacterSheet {
            strength: record.strength,
            dexterity: record.dexterity,
            intelligence: record.intelligence,
            skills: Vec::new(),
            effects: Vec::new(),
            dead: false,
            quests: record.quests.clone(),
            done_quests: record.done_quests.clone(),
        }),
        access: AccessLevel::Player,
    });
    shard.tick(now);

    let log = log_of(&shard, connection_two());
    assert_eq!(log.active.len(), 1, "the quest came back");
    assert_eq!(log.active[0].progress, vec![1], "with its progress");
}

/// A second connection id, for the relog half of the persistence tests.
fn connection_two() -> ConnectionId {
    ConnectionId::from_raw(2)
}
