//! A sea with one island in it, and a table that knows one ship.
//!
//! The multi is a fixture rather than a client file, for `openshard-housing`'s
//! own reason: what is under test is the arithmetic — an offset added to an
//! origin, a tiledata flag deciding hull from deck — and a real carrack would
//! test the same arithmetic a hundred times over while making the expected
//! answer impossible to write down.

use std::collections::{BTreeMap, HashMap};

use openshard_entities::Registry;
use openshard_events::EventBus;
use openshard_movement::{LandTile, Terrain};
use openshard_protocol::serial::SerialKind;
use openshard_state::harvest::Banks;
use openshard_state::rng::Rng;
use openshard_state::sectors::Sectors;
use openshard_state::{Boats, Dialogue, FacetState, Gameplay, Obstructions, QuestDefs, Regions};
use openshard_uofiles::multi::Component;

use super::*;

/// A small sea.
const SIZE: u32 = 64;
/// The one ship the fixture terrain knows.
const SLOOP: u16 = 0x0C;
/// A hull plank: impassable, ten tall.
const HULL: u16 = 0x3E4E;
/// A deck plank: walked on, three tall, and *not* impassable — the component
/// that must not be folded into the hull, because a ship whose deck blocked
/// would be a solid block of wood.
const DECK: u16 = 0x3E4A;

/// A sea with one strip of shore along y = 0.
struct Sea {
    components: Vec<Component>,
}

impl Terrain for Sea {
    fn can_step(&self, _from: Point, to: Point) -> Option<Point> {
        (to.y == 0).then_some(to)
    }

    fn land_tile(&self, _tile: Tile) -> Option<LandTile> {
        Some(LandTile(0))
    }

    fn land_is_water(&self, tile: Tile) -> bool {
        tile.y != 0
    }

    fn can_fit(&self, tile: Tile, _z: i32, _height: i32) -> bool {
        tile.y == 0
    }

    fn multi_components(&self, id: u16) -> &[Component] {
        if id == SLOOP { &self.components } else { &[] }
    }

    fn item_blocks(&self, graphic: Graphic) -> bool {
        graphic.0 == HULL
    }

    fn item_height(&self, graphic: Graphic) -> u8 {
        match graphic.0 {
            HULL => 10,
            DECK => 3,
            _ => 0,
        }
    }
}

fn component(graphic: u16, dx: i16, dy: i16, dz: i16, drawn: bool) -> Component {
    Component {
        graphic,
        dx,
        dy,
        dz,
        // `1` is the `.mul`'s drawn value and `0` its skip — the sense that
        // reads backwards from its name.
        flags: u64::from(drawn),
    }
}

/// A sloop: two deck tiles with a hull tile either side, and the signature tile
/// no client draws.
fn sloop() -> Vec<Component> {
    vec![
        component(1, 0, 0, 0, false),
        component(HULL, -1, 0, 0, true),
        component(DECK, 0, 0, 0, true),
        component(DECK, 0, 1, 0, true),
        component(HULL, 1, 0, 0, true),
    ]
}

fn a_sea() -> WorldState {
    let mut facets = BTreeMap::new();
    facets.insert(
        Facet(0),
        FacetState {
            terrain: Some(Box::new(Sea { components: sloop() })),
            coarse: None,
            width: SIZE,
            height: SIZE,
            sectors: Sectors::new(SIZE, SIZE),
            obstructions: Obstructions::default(),
            boats: Boats::default(),
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
        alliances: openshard_state::Alliances::default(),
        parties: openshard_state::Parties::default(),
        gameplay: Gameplay::default(),
        save_requested: false,
    }
}

fn a_captain(state: &mut WorldState) -> (EntityId, Serial) {
    state.registry.spawn_with_serial(SerialKind::Mobile).unwrap()
}

/// A ship is an item whose graphic is the multi, exactly as a house is — so
/// everything that already walks items draws it with no change.
#[test]
fn a_boat_is_an_item_whose_graphic_is_the_multi() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    let at = Point::new(20, 20, 0);
    let boat = place(&mut state, actor, at, Facet(0), SLOOP, owner).expect("open water");

    assert_eq!(
        state.registry.get::<Drawn>(boat).map(|drawn| drawn.id),
        Some(Graphic(MULTI_FLAG | SLOOP)),
        "the wire carries a ship as 0x4000 above its id",
    );
    assert_eq!(state.registry.get::<Position>(boat).map(|p| p.0), Some(at));
    assert_eq!(
        state.registry.get::<Boat>(boat),
        Some(&Boat { multi: SLOOP, owner }),
    );
    assert_eq!(boat_at(&state, at, Facet(0)), Some(boat));
}

/// **The split that makes a ship a ship.** The tiledata flag decides, so a deck
/// carries a body and a hull stops one — folding either into the other gives a
/// solid block of wood or a ghost ship.
#[test]
fn the_hull_blocks_and_the_deck_carries() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    place(&mut state, actor, Point::new(20, 20, 0), Facet(0), SLOOP, owner).expect("open water");

    let boats = &state.facet_state(Facet(0)).boats;
    assert_eq!(boats.deck_at(20, 20, 0), Some(3), "the deck plank's own top");
    assert_eq!(boats.deck_at(20, 21, 0), Some(3), "and the tile behind it");
    assert!(boats.hull_blocks(19, 20, 0), "the port hull");
    assert!(boats.hull_blocks(21, 20, 0), "the starboard hull");
    assert_eq!(boats.deck_at(19, 20, 0), None, "a hull is not a floor");
}

/// The undrawn signature tile is not part of the ship, the same way it is not
/// part of a house's footprint.
#[test]
fn the_signature_tile_is_not_a_plank() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    place(&mut state, actor, Point::new(20, 20, 0), Facet(0), SLOOP, owner).expect("open water");

    // Four drawn components, so four planks — not five.
    let boats = &state.facet_state(Facet(0)).boats;
    let covered = [(19, 20), (20, 20), (20, 21), (21, 20)];
    let total: usize = covered.iter().map(|&(x, y)| boats.at(x, y).len()).sum();
    assert_eq!(total, 4, "the signature tile was launched as part of the ship");
}

/// Half on the beach is not afloat. Every tile of the berth is checked, not
/// just the origin — the same reason a house's region check walks its whole
/// footprint.
#[test]
fn a_ship_half_on_the_beach_is_refused() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);

    // At y = 0 the sloop's port, starboard and midships planks are all on the
    // shore and only the aft one is afloat.
    assert_eq!(
        place(&mut state, actor, Point::new(20, 0, 0), Facet(0), SLOOP, owner),
        Err(Refusal::NotOnWater),
        "the shore runs along y = 0",
    );
    assert!(
        state.facet_state(Facet(0)).boats.is_empty(),
        "a refused launch left a ship in the index",
    );

    // One tile further out and every plank is over water. The contrast is the
    // point: the check is per tile, so it is the *beached* plank that refuses
    // and not the ship's proximity to land.
    assert!(
        place(&mut state, actor, Point::new(20, 1, 0), Facet(0), SLOOP, owner).is_ok(),
        "a ship moored against the shore is still afloat",
    );
}

/// **The one the index exists to make possible.** Two hulls are not in
/// `Obstructions`, so they do not see each other through the mechanism that
/// stops everything else; the berth check is what stops them, and this is the
/// test that fails if it is dropped.
#[test]
fn two_boats_do_not_occupy_one_tile() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    let at = Point::new(20, 20, 0);
    place(&mut state, actor, at, Facet(0), SLOOP, owner).expect("open water");

    assert_eq!(
        place(&mut state, actor, at, Facet(0), SLOOP, owner),
        Err(Refusal::Occupied),
    );
    assert_eq!(
        place(&mut state, actor, Point::new(21, 20, 0), Facet(0), SLOOP, owner),
        Err(Refusal::Occupied),
        "and an overlap of one tile is still an overlap",
    );
    assert_eq!(state.facet_state(Facet(0)).boats.len(), 1);
}

/// Far enough apart is fine, which is the other half of the same check.
#[test]
fn two_boats_moor_side_by_side_when_they_do_not_touch() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    place(&mut state, actor, Point::new(20, 20, 0), Facet(0), SLOOP, owner).expect("open water");
    place(&mut state, actor, Point::new(30, 20, 0), Facet(0), SLOOP, owner).expect("open water");

    assert_eq!(state.facet_state(Facet(0)).boats.len(), 2);
}

/// Staff skip the *judgements* about the berth and nothing else — housing's D10
/// split, with the same reasoning. A game master may put a ship in a fountain.
#[test]
fn staff_may_launch_a_ship_onto_dry_land() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    state.registry.insert(actor, openshard_state::components::Staff);

    assert!(
        place(&mut state, actor, Point::new(20, 0, 0), Facet(0), SLOOP, owner).is_ok(),
        "the exemption did not reach the water check",
    );
}

/// And they are not exempt from arithmetic: there is no tile off the edge of the
/// world to float on, whoever is asking.
#[test]
fn staff_are_still_refused_a_ship_off_the_edge_of_the_world() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    state.registry.insert(actor, openshard_state::components::Staff);

    assert_eq!(
        place(&mut state, actor, Point::new(0, 20, 0), Facet(0), SLOOP, owner),
        Err(Refusal::OffTheMap),
        "the port hull would stand at x -1",
    );
}

/// A multi no client knows is a fact about the id, so it is refused for staff
/// too — and it does not leave an entity behind.
#[test]
fn a_multi_that_is_not_a_ship_is_refused_and_leaves_nothing() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    let before = state.registry.query::<Position>().count();

    assert_eq!(
        place(
            &mut state,
            actor,
            Point::new(20, 20, 0),
            Facet(0),
            SLOOP + 1,
            owner
        ),
        Err(Refusal::NoSuchMulti),
    );
    assert_eq!(
        state.registry.query::<Position>().count(),
        before,
        "a refused launch left an entity on the water",
    );
}

/// Sinking one takes it out of all three places it was put.
#[test]
fn sinking_a_ship_clears_the_index_the_grid_and_the_registry() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    let at = Point::new(20, 20, 0);
    let boat = place(&mut state, actor, at, Facet(0), SLOOP, owner).expect("open water");

    sink(&mut state, boat);

    assert!(state.facet_state(Facet(0)).boats.is_empty());
    assert_eq!(boat_at(&state, at, Facet(0)), None);
    assert!(state.registry.get::<Position>(boat).is_none());
}

/// A shard with no client files has no sea, so it has nowhere to moor — the
/// same bargain every other client-file question on the terrain makes.
#[test]
fn a_shard_with_no_client_files_launches_nothing() {
    let mut state = a_sea();
    state.facet_state_mut(Facet(0)).terrain = None;
    let (actor, owner) = a_captain(&mut state);

    assert_eq!(
        place(&mut state, actor, Point::new(20, 20, 0), Facet(0), SLOOP, owner),
        Err(Refusal::NoSuchMulti),
    );
}

/// **The step check, end to end.** The map refuses the sea, the deck overturns
/// it, and the hull refuses again — which is the whole of what B1 promises a
/// player.
#[test]
fn a_body_walks_from_the_shore_onto_the_deck_and_not_through_the_hull() {
    let mut state = a_sea();
    let (actor, owner) = a_captain(&mut state);
    // Bow against the shore: the deck at (20, 1), hulls at (19, 1) and (21, 1).
    place(&mut state, actor, Point::new(20, 1, 0), Facet(0), SLOOP, owner).expect("staff-free water");

    let live = state.facet_state(Facet(0)).live_terrain();
    assert_eq!(
        live.can_step(Point::new(20, 0, 0), Point::new(20, 1, 0)),
        Some(Point::new(20, 1, 3)),
        "stepping aboard lands on the deck and not in the water",
    );
    assert_eq!(
        live.can_step(Point::new(20, 1, 3), Point::new(20, 2, 3)),
        Some(Point::new(20, 2, 3)),
        "and walking aft stays on it",
    );
    assert!(
        live.can_step(Point::new(20, 1, 3), Point::new(21, 1, 3))
            .is_none(),
        "walked straight through the hull",
    );
    assert!(
        live.can_step(Point::new(20, 0, 0), Point::new(30, 1, 0))
            .is_none(),
        "open water with no ship on it is still not walkable",
    );
}
