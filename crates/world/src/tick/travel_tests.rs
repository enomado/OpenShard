//! Moving a mobile between facets: what the traveller's client is told, and
//! what the world it left has to forget.
//!
//! A child module rather than more of `tests.rs`, for the reason `region_tests`
//! gives: these read private world state, so they stay inside the module, but
//! they need not pile into the same file.
//!
//! Every case here is a cache that no compiler checks. A facet change that
//! forgets one of them produces no error and no failing single-facet test — it
//! produces a client drawing mobiles from a world it is no longer in, at
//! coordinates that now mean somewhere else.

use super::tests::{
    add_empty_facet, add_empty_facet_sized, enter, enter_as, enter_on_facet, packets_for,
    serial_of, walk, world, START,
};
use super::*;
use openshard_state::components::{InRegion, Movement, Position};
use openshard_state::{Region, RegionFlags, RegionRect};

/// Ilshenar's shape, which is nothing like Britannia's — the whole reason the
/// client has to be told.
const ILSHENAR: (u32, u32) = (2304, 1600);

/// Where a traveller lands, inside every facet these tests register.
fn arrival() -> Point {
    Point::new(START.0, START.1, 0)
}

#[test]
fn a_traveller_leaves_the_old_facets_sector_grid() {
    // The removal `teleport` never had to do. Left out, the old facet keeps
    // handing this entity back to every `nearby` query on it forever.
    let now = Instant::now();
    let mut world = world();
    add_empty_facet(&mut world, 1);
    let connection = enter(&mut world, now);
    let traveller = world.state.players[&connection];

    assert!(
        world.state.facet_state(0).sectors.position_of(traveller) == Some(arrival()),
        "it starts on facet 0's grid"
    );

    world.state.move_to(traveller, 1, arrival());

    assert_eq!(
        world.state.facet_state(0).sectors.position_of(traveller),
        None,
        "and is gone from it"
    );
    assert_eq!(
        world.state.facet_state(1).sectors.position_of(traveller),
        Some(arrival()),
        "and on the new one"
    );
    assert_eq!(
        world.state.facet_of(traveller),
        1,
        "and the world agrees which facet it is on"
    );
}

#[test]
fn a_watcher_on_the_old_facet_is_told_to_forget_the_traveller() {
    // Two mobiles on different facets never see each other, so a watcher left
    // holding the traveller would hold it until one of them logged out.
    let now = Instant::now();
    let mut world = world();
    add_empty_facet(&mut world, 1);
    let watcher_connection = enter(&mut world, now);
    let traveller_connection = enter_as(&mut world, ConnectionId::from_raw(2), now);
    let traveller = world.state.players[&traveller_connection];
    let traveller_serial = serial_of(&world, traveller_connection);
    let watcher = world.state.players[&watcher_connection];
    assert!(
        world.state.seen[&watcher].contains(&traveller),
        "they can see each other to begin with"
    );
    let _ = packets_for(&mut world, watcher_connection);

    world.state.move_to(traveller, 1, arrival());

    assert!(
        !world.state.seen[&watcher].contains(&traveller),
        "the watcher no longer holds the traveller"
    );
    assert!(
        packets_for(&mut world, watcher_connection)
            .iter()
            .any(|p| p[0] == 0x1D
                && u32::from_be_bytes([p[1], p[2], p[3], p[4]]) == traveller_serial),
        "and was told to take it off the screen"
    );
}

#[test]
fn a_traveller_forgets_everything_on_the_old_facets_screen() {
    // ServUO's `ClearScreen`. Without it the client keeps drawing the mobiles of
    // a world it has left, and their serials go on meaning something.
    let now = Instant::now();
    let mut world = world();
    add_empty_facet(&mut world, 1);
    let traveller_connection = enter(&mut world, now);
    let stayer_connection = enter_as(&mut world, ConnectionId::from_raw(2), now);
    let traveller = world.state.players[&traveller_connection];
    let stayer_serial = serial_of(&world, stayer_connection);
    assert!(
        !world.state.seen[&traveller].is_empty(),
        "it has somebody on screen to begin with"
    );
    let _ = packets_for(&mut world, traveller_connection);

    world.state.move_to(traveller, 1, arrival());

    assert!(
        world.state.seen[&traveller].is_empty(),
        "the traveller's screen is empty on arrival"
    );
    assert!(
        packets_for(&mut world, traveller_connection)
            .iter()
            .any(|p| p[0] == 0x1D && u32::from_be_bytes([p[1], p[2], p[3], p[4]]) == stayer_serial),
        "and it was told to forget who it left behind"
    );
}

#[test]
fn a_facet_change_sends_the_new_facets_map_dimensions() {
    // `0xBF 0x08` says which map to draw; `0x76` says where on it and how big it
    // is. Sending Britannia's size for Ilshenar puts the edge of the world in
    // the wrong place.
    let now = Instant::now();
    let mut world = world();
    add_empty_facet_sized(&mut world, 1, ILSHENAR.0, ILSHENAR.1);
    let connection = enter(&mut world, now);
    let traveller = world.state.players[&connection];
    let _ = packets_for(&mut world, connection);

    world.state.move_to(traveller, 1, arrival());
    let sent = packets_for(&mut world, connection);

    let map_change = sent
        .iter()
        .find(|p| p[0] == 0xBF && u16::from_be_bytes([p[3], p[4]]) == 0x08)
        .expect("told which map to draw");
    assert_eq!(map_change[5], 1, "facet 1");

    let change = sent
        .iter()
        .find(|p| p[0] == 0x76)
        .expect("told how big the new world is");
    assert_eq!(
        u16::from_be_bytes([change[12], change[13]]),
        ILSHENAR.0 as u16,
        "the new facet's width, not the old one's"
    );
    assert_eq!(
        u16::from_be_bytes([change[14], change[15]]),
        ILSHENAR.1 as u16,
        "and its height"
    );

    // And no `0x1B`: that is the packet that starts a session, not one that
    // moves it.
    assert!(
        !sent.iter().any(|p| p[0] == 0x1B),
        "a facet change is not a login"
    );
}

#[test]
fn login_sends_the_dimensions_of_the_facet_the_character_is_on() {
    // The same fact, at the other end: `0x1B` carried Britannia's size for every
    // facet, so a character saved in Ilshenar woke to a map three times too big.
    let now = Instant::now();
    let mut world = world();
    add_empty_facet_sized(&mut world, 1, ILSHENAR.0, ILSHENAR.1);
    let connection = ConnectionId::from_raw(77);
    enter_on_facet(&mut world, connection, 1, now);

    let start = packets_for(&mut world, connection)
        .into_iter()
        .find(|p| p[0] == 0x1B)
        .expect("the world-entry packet");
    // 0x1B: id(1) serial(4) pad(4) body(2) x(2) y(2) pad(1) z(1) facing(1)
    // pad(1) 0xFFFFFFFF(4) pad(4) width(2) height(2) — width at offset 27.
    assert_eq!(
        u16::from_be_bytes([start[27], start[28]]),
        ILSHENAR.0 as u16,
        "the facet it logged in on"
    );
    assert_eq!(
        u16::from_be_bytes([start[29], start[30]]),
        ILSHENAR.1 as u16,
    );
}

#[test]
fn the_same_region_id_on_two_facets_is_still_a_crossing() {
    // Every facet numbers its regions from zero, so an id alone is not an
    // answer. Compared without the facet, a traveller between two regions that
    // happen to share a number looks like somebody who never moved: no
    // `RegionChanged`, no music, and no guards.
    let now = Instant::now();
    let mut world = world();
    add_empty_facet(&mut world, 1);
    let named = |name: &str| Region {
        id: 0,
        name: name.to_owned(),
        priority: 50,
        rects: vec![RegionRect::new(START.0 - 20, START.1 - 20, 40, 40)],
        flags: RegionFlags::default(),
        music: None,
        light: None,
    };
    for (facet, name) in [(0, "Britain"), (1, "Compassion")] {
        world.queue(Command::RegisterRegions {
            facet,
            regions: vec![named(name)],
        });
    }
    world.tick(now);

    let connection = enter(&mut world, now);
    let traveller = world.state.players[&connection];
    world.tick(now);
    assert_eq!(
        world.state.registry.get::<InRegion>(traveller),
        Some(&InRegion {
            facet: 0,
            region: Some(0)
        }),
        "it is in Britain, region zero of facet zero"
    );
    let mut crossings = world.state.bus.cursor::<crate::events::RegionChanged>();

    world.state.move_to(traveller, 1, arrival());
    world.tick(now);

    let names: Vec<String> = world
        .state
        .bus
        .read(&mut crossings)
        .map(|crossing| crossing.name.clone())
        .collect();
    assert!(
        names.iter().any(|name| name == "Compassion"),
        "arriving on another facet's region zero is a crossing, not a no-op: {names:?}"
    );
    assert_eq!(
        world.state.registry.get::<InRegion>(traveller),
        Some(&InRegion {
            facet: 1,
            region: Some(0)
        }),
        "and the memory now names the facet it is on"
    );
}

#[test]
fn a_facet_change_resets_the_walk_sequence() {
    // The client zeroes its own count on a jump it did not predict. A server
    // that keeps counting refuses the client's next step — which was correct —
    // and the two ends spend the rest of the session out of phase.
    let now = Instant::now();
    let mut world = world();
    add_empty_facet(&mut world, 1);
    let connection = enter(&mut world, now);
    let traveller = world.state.players[&connection];

    world.queue(Command::Walk {
        connection,
        request: walk(0, Direction::North),
    });
    world.tick(now);
    let walker = world.registry().get::<Movement>(traveller).unwrap().0;
    assert!(!walker.sequence.is_fresh(), "it has taken a step");

    world.state.move_to(traveller, 1, arrival());

    let walker = world.registry().get::<Movement>(traveller).unwrap().0;
    assert!(
        walker.sequence.is_fresh(),
        "and the jump put both ends back to zero"
    );
    assert_eq!(
        world.registry().get::<Position>(traveller).map(|p| p.0),
        Some(arrival()),
        "the walker's own copy of where it stands moved with it"
    );
}

#[test]
fn a_facet_the_shard_never_loaded_is_refused() {
    // A mobile there would have no ground, no neighbours and no way back.
    let now = Instant::now();
    let mut world = world();
    let connection = enter(&mut world, now);
    let traveller = world.state.players[&connection];

    world.state.move_to(traveller, 9, Point::new(100, 100, 0));

    assert_eq!(world.state.facet_of(traveller), 0, "it did not go anywhere");
    assert_eq!(
        world.state.facet_state(0).sectors.position_of(traveller),
        Some(arrival()),
        "and is still on the grid it started on"
    );
}
