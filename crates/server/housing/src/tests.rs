//! Placing a house over a world with nothing in it but a terrain that knows one
//! multi.
//!
//! The multi is a fixture rather than a client file, and it is the right call
//! here for the reason `client_files.rs` is the wrong place for a rule: what is
//! under test is the *arithmetic* — an offset added to an origin, a height added
//! to a z, a flag deciding whether a component blocks — and a real villa would
//! test the same arithmetic 148 times while making the expected answer
//! impossible to write down.
//!
//! What a real file settles is the format, and `uofiles::multi` already gates
//! that against one.

use std::collections::{BTreeMap, HashMap};

use openshard_entities::Registry;
use openshard_events::EventBus;
use openshard_movement::{Terrain, Tile};
use openshard_protocol::serial::{Serial, SerialKind};
use openshard_protocol::wire::Graphic;
use openshard_protocol::world::{Facet, Point};
use openshard_state::harvest::Banks;
use openshard_state::rng::Rng;
use openshard_state::sectors::Sectors;
use openshard_uofiles::multi::Component;

use super::*;
use openshard_state::{Dialogue, FacetState, Gameplay, QuestDefs, Regions};

/// A small world, and the multi id everything here places.
const SIZE: u32 = 32;
const COTTAGE: u16 = 0x64;

/// A wall: impassable, twenty tall — the classic UO wall the door height was
/// taken from.
const WALL: u16 = 0x0006;
/// A floor: drawn, walked over, and *not* impassable. The component that must
/// not be folded into the footprint, because a house whose floor blocked would
/// be sealed shut from the inside.
const FLOOR: u16 = 0x0007;

/// A terrain that knows one multi and one impassable graphic.
///
/// `can_step` allows everything, so a refused step in a test below is the
/// obstruction index refusing it and never the ground.
struct Ground {
    components: Vec<Component>,
    /// What every tile's land id is. `0` is nothing in particular; a road id
    /// makes the whole facet a street.
    land: u16,
    /// Whether the ground will take a house at all — `can_fit`'s answer, which
    /// is ServUO's rules two and four asked as one question.
    fits: bool,
}

impl Terrain for Ground {
    fn can_step(&self, _from: Point, to: Point) -> Option<Point> {
        Some(to)
    }

    fn land_tile(&self, _tile: Tile) -> Option<openshard_movement::LandTile> {
        Some(openshard_movement::LandTile(self.land))
    }

    fn can_fit(&self, _tile: Tile, _z: i32, _height: i32) -> bool {
        self.fits
    }

    fn multi_components(&self, id: u16) -> &[Component] {
        if id == COTTAGE { &self.components } else { &[] }
    }

    fn item_blocks(&self, graphic: Graphic) -> bool {
        graphic.0 == WALL
    }

    fn item_height(&self, graphic: Graphic) -> u8 {
        if graphic.0 == WALL { 20 } else { 0 }
    }
}

fn component(graphic: u16, dx: i16, dy: i16, dz: i16, drawn: bool) -> Component {
    Component {
        graphic,
        dx,
        dy,
        dz,
        // `1` is the `.mul`'s "drawn" value and `0` its skip — the sense that
        // reads backwards, and the reason this helper takes a `bool`.
        flags: u64::from(drawn),
    }
}

/// A cottage: four walls in a ring, a floor in the middle, and one component the
/// client never draws.
fn cottage() -> Vec<Component> {
    vec![
        component(1, 0, 0, 0, false), // the signature tile every multi starts with
        component(WALL, -1, -1, 0, true),
        component(WALL, 1, -1, 0, true),
        component(WALL, -1, 1, 0, true),
        component(WALL, 1, 1, 0, true),
        component(FLOOR, 0, 0, 0, true),
        // Drawn nowhere, and far enough away that folding it in would be obvious.
        component(WALL, 10, 10, 0, false),
    ]
}

fn world_with(components: Vec<Component>) -> WorldState {
    ground_of(components, 0, true)
}

fn ground_of(components: Vec<Component>, land: u16, fits: bool) -> WorldState {
    let mut facets = BTreeMap::new();
    facets.insert(
        Facet(0),
        FacetState {
            terrain: Some(Box::new(Ground {
                components,
                land,
                fits,
            })),
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
        alliances: openshard_state::Alliances::default(),
        parties: openshard_state::Parties::default(),
        gameplay: Gameplay::default(),
        save_requested: false,
    }
}

fn an_owner(state: &mut WorldState) -> Serial {
    let (_, serial) = state.registry.spawn_with_serial(SerialKind::Mobile).unwrap();
    serial
}

#[test]
fn a_house_is_an_item_whose_graphic_is_the_multi() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");

    assert_eq!(
        state.registry.get::<Drawn>(house).map(|drawn| drawn.id),
        Some(Graphic(MULTI_FLAG | COTTAGE)),
        "the wire carries a house as 0x4000 above its id"
    );
    assert_eq!(state.registry.get::<Position>(house).map(|p| p.0), Some(at));
    assert_eq!(
        state.registry.get::<House>(house),
        Some(&House {
            multi: COTTAGE,
            owner
        })
    );
    // And it is an *item*, so everything that walks items reaches it.
    assert!(
        state.registry.serial_of(house).is_some_and(|s| s.is_item()),
        "a house took a mobile serial"
    );
}

/// The whole point of H1: the walls stop somebody and the doorway does not.
#[test]
fn the_walls_block_and_the_floor_does_not() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");

    let obstructions = &state.facet_state(Facet(0)).obstructions;
    for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
        let tile = Tile::new((10 + dx) as u16, (10 + dy) as u16);
        assert!(
            obstructions.blocker_at_z(tile.x, tile.y, 0).is_some(),
            "the wall at ({dx}, {dy}) does not stop anybody"
        );
    }
    assert!(
        obstructions.blocker_at_z(10, 10, 0).is_none(),
        "the floor was folded in, which seals the house shut from the inside"
    );
    assert!(
        !obstructions.is_blocked(20, 20),
        "an undrawn component was folded in"
    );
}

/// A wall on the second floor blocks the second floor and leaves the ground
/// open — the reason an obstacle carries a z-span, exercised through a house
/// rather than through the index directly.
#[test]
fn an_upper_storey_wall_leaves_the_ground_floor_open() {
    let mut components = cottage();
    components.push(component(WALL, -1, -1, 20, true));
    let mut state = world_with(components);
    let owner = an_owner(&mut state);
    place(&mut state, Point::new(10, 10, 0), Facet(0), COTTAGE, owner).expect("a legal spot");

    let obstructions = &state.facet_state(Facet(0)).obstructions;
    // One tile, one entity, two walls: both must be there. Keyed by the entity
    // alone the second would have overwritten the first.
    assert!(
        obstructions.blocker_at_z(9, 9, 0).is_some(),
        "the ground floor wall"
    );
    assert!(
        obstructions.blocker_at_z(9, 9, 25).is_some(),
        "the upper floor wall"
    );
    // And the storey above both is open sky.
    assert!(obstructions.blocker_at_z(9, 9, 60).is_none());
}

/// Two houses may not stand in each other.
#[test]
fn a_house_will_not_go_where_a_house_already_is() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    place(&mut state, Point::new(10, 10, 0), Facet(0), COTTAGE, owner).expect("a legal spot");
    assert_eq!(
        place(&mut state, Point::new(10, 10, 0), Facet(0), COTTAGE, owner),
        Err(Refusal::Occupied)
    );
    // One tile over, the rings overlap at a corner, so it is still refused.
    assert_eq!(
        place(&mut state, Point::new(12, 10, 0), Facet(0), COTTAGE, owner),
        Err(Refusal::Occupied)
    );
    // Well clear, and it goes up.
    assert!(place(&mut state, Point::new(20, 20, 0), Facet(0), COTTAGE, owner).is_ok());
}

/// The four ways a placement is refused before the ground is even looked at.
#[test]
fn a_multi_nobody_can_build_is_refused_by_name() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);

    assert_eq!(
        place(&mut state, at, Facet(0), 0x0999, owner),
        Err(Refusal::NoSuchMulti),
        "an id the client has never heard of"
    );
    assert_eq!(
        place(&mut state, at, Facet(0), FOUNDATION_IDS.start, owner),
        Err(Refusal::NeedsCustomisation),
        "a customisable foundation has no stairs without a design system"
    );
    assert_eq!(
        place(&mut state, at, Facet(0), FOUNDATION_IDS.end - 1, owner),
        Err(Refusal::NeedsCustomisation),
        "and the far end of the range"
    );

    // A multi that is in the table and blocks nothing — the treasure-site markers
    // a real file ships five of.
    let mut marker = world_with(vec![component(FLOOR, 0, 0, 0, true)]);
    let marker_owner = an_owner(&mut marker);
    assert_eq!(
        place(&mut marker, at, Facet(0), COTTAGE, marker_owner),
        Err(Refusal::DrawsNothing)
    );
}

/// The graphic and the id are the same thing said two ways, and either reaches
/// the same house.
#[test]
fn a_graphic_and_an_id_place_the_same_house() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let from_id = place(&mut state, Point::new(5, 5, 0), Facet(0), COTTAGE, owner).expect("by id");
    let from_graphic = place(
        &mut state,
        Point::new(20, 20, 0),
        Facet(0),
        MULTI_FLAG | COTTAGE,
        owner,
    )
    .expect("by graphic");
    assert_eq!(
        state.registry.get::<House>(from_id).map(|h| h.multi),
        state.registry.get::<House>(from_graphic).map(|h| h.multi)
    );
}

/// A footprint that would hang off the north-west corner is refused rather than
/// wrapping to the far side of the world.
#[test]
fn a_house_at_the_edge_does_not_wrap_around_the_world() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    assert_eq!(
        place(&mut state, Point::new(0, 0, 0), Facet(0), COTTAGE, owner),
        Err(Refusal::OffTheMap),
        "a wall one tile west of x=0 became a wall at 65535"
    );
}

/// Taking the walls back out leaves the ground walkable, which is what a
/// demolition and a moving crate will need.
#[test]
fn unblocking_gives_the_ground_back() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let footprint = footprint_of(&state, at, Facet(0), COTTAGE).expect("the same footprint");

    unblock(&mut state, house, Facet(0), &footprint);
    assert!(
        !state.facet_state(Facet(0)).obstructions.is_blocked(9, 9),
        "a wall outlived the house"
    );

    // The walls are gone and the spot is *still* refused, which is the half worth
    // asserting: a house is two facts — the obstructions it holds and the entity
    // that owns a yard — and `unblock` undoes only the first. A demolition that
    // called this and stopped would leave a plot nobody could ever build on.
    assert_eq!(
        place(&mut state, at, Facet(0), COTTAGE, owner),
        Err(Refusal::TooCloseToAHouse)
    );

    state.registry.despawn(house);
    assert!(
        place(&mut state, at, Facet(0), COTTAGE, owner).is_ok(),
        "with the house gone the plot is free again"
    );
}

/// The rule a player notices the absence of: without it, houses go up in the
/// middle of Britain's streets.
#[test]
fn a_house_may_not_be_built_on_a_road() {
    // The whole facet is cobbles, which is the cheapest way to put a road under
    // every footprint tile.
    let mut state = ground_of(cottage(), 0x0071, true);
    let owner = an_owner(&mut state);
    assert_eq!(
        place(&mut state, Point::new(10, 10, 0), Facet(0), COTTAGE, owner),
        Err(Refusal::OnARoad)
    );

    // The list is ranges, not ids, so both ends and the middle of one must read
    // as road and the tile below it must not.
    assert!(is_road(0x0071) && is_road(0x0075) && is_road(0x0078));
    assert!(!is_road(0x0070), "one below the first range read as a road");
    assert!(is_road(0x3FF4), "the single-id range");
    assert!(is_road(0x0150) && is_road(0x015C), "the second furrow range");
    assert!(!is_road(0x015D));
}

/// Rules two and four, which `can_fit` asks as one question: a solid wall in the
/// way and thin air with no floor are the same refusal.
#[test]
fn ground_that_will_not_take_a_house_refuses_one() {
    let mut state = ground_of(cottage(), 0, false);
    let owner = an_owner(&mut state);
    assert_eq!(
        place(&mut state, Point::new(10, 10, 0), Facet(0), COTTAGE, owner),
        Err(Refusal::BadGround)
    );
}

/// Every house keeps five tiles to itself, and the yard is measured against the
/// other house's *footprint* rather than a stored rectangle.
#[test]
fn a_house_keeps_a_yard_clear_of_other_houses() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    place(&mut state, Point::new(10, 10, 0), Facet(0), COTTAGE, owner).expect("the first house");

    // The yard is measured wall to wall, not origin to origin, and that is the
    // arithmetic worth pinning. The first cottage's east wall is at x=11; a
    // second at origin 17 puts its west wall at 16, five tiles away and so
    // *inside* the yard. Origin 18 puts it at 17, six away, and clear.
    assert_eq!(
        place(&mut state, Point::new(17, 10, 0), Facet(0), COTTAGE, owner),
        Err(Refusal::TooCloseToAHouse),
        "two walls five tiles apart is inside a yard of five"
    );
    assert!(
        place(&mut state, Point::new(18, 10, 0), Facet(0), COTTAGE, owner).is_ok(),
        "a house six tiles clear of another was refused"
    );
}

/// A shard with no client files places nothing rather than placing something
/// with no walls — the same bargain every other `Terrain` method makes.
#[test]
fn a_world_with_no_terrain_has_no_houses() {
    let mut state = world_with(cottage());
    state.facet_state_mut(Facet(0)).terrain = None;
    let owner = an_owner(&mut state);
    assert_eq!(
        place(&mut state, Point::new(10, 10, 0), Facet(0), COTTAGE, owner),
        Err(Refusal::NoSuchMulti)
    );
}
