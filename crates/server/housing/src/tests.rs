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

/// An item on the ground, a container if asked for one.
fn an_item(state: &mut WorldState, at: Point, container: bool) -> EntityId {
    let (entity, _) = state.registry.spawn_with_serial(SerialKind::Item).unwrap();
    state.registry.insert(
        entity,
        Drawn {
            id: Graphic(0x0E3C),
            hue: openshard_protocol::wire::Hue(0),
        },
    );
    state.registry.insert(entity, Position(at));
    state.registry.insert(entity, Facet(0));
    if container {
        state.registry.insert(
            entity,
            openshard_state::components::Container {
                gump: Graphic(0x003C),
            },
        );
    }
    entity
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
            owner,
            co_owners: Default::default(),
            friends: Default::default(),
            bans: Default::default(),
            age: 0,
            // Five drawn tiles at four apiece — see `storage::LOCKDOWNS_PER_TILE`.
            lockdowns: 20,
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

/// A fresh house with one owner and nobody else in it.
fn a_house(owner: Serial) -> House {
    House {
        multi: COTTAGE,
        owner,
        co_owners: Default::default(),
        friends: Default::default(),
        bans: Default::default(),
        age: 0,
        lockdowns: 20,
    }
}

fn somebody(n: u32) -> Serial {
    Serial::new(n).expect("a serial")
}

/// The reference's rules are nested — a co-owner is a friend, an owner is a
/// co-owner — and asking them as one question is what stops the wrong one being
/// asked.
#[test]
fn standing_is_one_question_with_a_nested_answer() {
    let owner = somebody(1);
    let mut house = a_house(owner);
    let friend = somebody(2);
    let co_owner = somebody(3);
    let stranger = somebody(4);
    house.friends.insert(friend);
    house.co_owners.insert(co_owner);

    assert_eq!(house.standing_of(owner, false), Standing::Owner);
    assert_eq!(house.standing_of(co_owner, false), Standing::CoOwner);
    assert_eq!(house.standing_of(friend, false), Standing::Friend);
    assert_eq!(house.standing_of(stranger, false), Standing::Stranger);
    // The order is what makes "at least this trusted" a comparison.
    assert!(Standing::Owner > Standing::CoOwner);
    assert!(Standing::CoOwner > Standing::Friend);
    assert!(Standing::Friend > Standing::Stranger);
    assert!(Standing::Stranger > Standing::Banned);
}

/// Nobody bans the owner out of their own house, and staff are never turned
/// away — both are the reference's own first branches.
#[test]
fn the_owner_and_staff_cannot_be_banned() {
    let owner = somebody(1);
    let mut house = a_house(owner);
    let staffer = somebody(2);
    house.bans.insert(owner);
    house.bans.insert(staffer);

    assert_eq!(
        house.standing_of(owner, false),
        Standing::Owner,
        "the owner was banned from their own house"
    );
    assert_eq!(
        house.standing_of(staffer, true),
        Standing::CoOwner,
        "a game master was turned away"
    );
    // And the same mobile without the authority *is* banned.
    assert_eq!(house.standing_of(staffer, false), Standing::Banned);
}

/// Only the owner names a co-owner. A co-owner who could would be handing the
/// house to a crowd the owner never met.
#[test]
fn a_co_owner_may_name_friends_and_not_co_owners() {
    let owner = somebody(1);
    let mut house = a_house(owner);
    let co_owner = somebody(2);
    trust(&mut house, owner, co_owner, Standing::CoOwner, false).expect("the owner may");

    let newcomer = somebody(3);
    assert_eq!(
        trust(&mut house, co_owner, newcomer, Standing::Friend, false),
        Ok(()),
        "a co-owner may name a friend"
    );
    assert_eq!(
        trust(&mut house, co_owner, somebody(4), Standing::CoOwner, false),
        Err(ListRefusal::NotYours)
    );
    // And a friend may name nobody at all.
    assert_eq!(
        trust(&mut house, newcomer, somebody(5), Standing::Friend, false),
        Err(ListRefusal::NotYours)
    );
}

/// Promotion **moves** somebody rather than adding them twice: two lists holding
/// one person is two answers to one question.
#[test]
fn promoting_a_friend_leaves_them_in_one_list() {
    let owner = somebody(1);
    let mut house = a_house(owner);
    let who = somebody(2);
    trust(&mut house, owner, who, Standing::Friend, false).unwrap();
    trust(&mut house, owner, who, Standing::CoOwner, false).unwrap();

    assert!(house.co_owners.contains(&who));
    assert!(
        !house.friends.contains(&who),
        "they are in both lists, so which one answers depends on check order"
    );
    assert_eq!(house.standing_of(who, false), Standing::CoOwner);
}

/// A ban is the newer decision and it wins: "banned but still a co-owner" is a
/// state with no useful answer.
#[test]
fn banning_a_co_owner_takes_the_trust_with_it() {
    let owner = somebody(1);
    let mut house = a_house(owner);
    let turncoat = somebody(2);
    trust(&mut house, owner, turncoat, Standing::CoOwner, false).unwrap();

    ban(&mut house, owner, turncoat, false).expect("the owner may");
    assert_eq!(house.standing_of(turncoat, false), Standing::Banned);
    assert!(!house.co_owners.contains(&turncoat));

    // Lifting it gives back a stranger, not a co-owner: undoing a ban grants
    // nothing.
    unban(&mut house, owner, turncoat, false).expect("the owner may");
    assert_eq!(house.standing_of(turncoat, false), Standing::Stranger);
}

/// The owner is not a name on any list and cannot be dropped from one.
#[test]
fn nobody_evicts_the_owner() {
    let owner = somebody(1);
    let mut house = a_house(owner);
    let co_owner = somebody(2);
    trust(&mut house, owner, co_owner, Standing::CoOwner, false).unwrap();

    // `NotTheOwner` and not `NotYours`: a co-owner *may* drop friends, so the
    // refusal is about who was named rather than about who asked, and saying so
    // is the difference between a usable message and a puzzling one.
    assert_eq!(
        distrust(&mut house, co_owner, owner, false),
        Err(ListRefusal::NotTheOwner)
    );
    assert_eq!(
        ban(&mut house, co_owner, owner, false),
        Err(ListRefusal::NotTheOwner)
    );
    // Only the owner drops a co-owner.
    assert_eq!(
        distrust(&mut house, co_owner, co_owner, false),
        Err(ListRefusal::NotYours)
    );
    assert_eq!(distrust(&mut house, owner, co_owner, false), Ok(()));
    assert_eq!(house.standing_of(co_owner, false), Standing::Stranger);
}

/// The lists have ceilings, and re-adding somebody already on one is not a new
/// name.
#[test]
fn a_full_list_refuses_a_new_name_and_takes_an_old_one() {
    let owner = somebody(1);
    let mut house = a_house(owner);
    for n in 0..MAX_CO_OWNERS as u32 {
        trust(&mut house, owner, somebody(100 + n), Standing::CoOwner, false).expect("under the ceiling");
    }
    assert_eq!(
        trust(&mut house, owner, somebody(999), Standing::CoOwner, false),
        Err(ListRefusal::Full)
    );
    assert_eq!(
        trust(&mut house, owner, somebody(100), Standing::CoOwner, false),
        Ok(()),
        "somebody already on the list is not a new name"
    );
}

/// A door standing inside a house becomes the house's, and the house's rules
/// then decide who may work it.
///
/// The multi cannot be the source — three of a shipped file's 326 carry a door
/// component at all — so the rule is the one a player would state.
#[test]
fn a_house_adopts_the_doors_standing_inside_it() {
    use openshard_state::components::{Door, HouseDoor};

    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);

    // One door where the house will stand, and one well outside it.
    let inside = door_at(&mut state, Point::new(10, 10, 0));
    let outside = door_at(&mut state, Point::new(25, 25, 0));

    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let serial = state.registry.serial_of(house).unwrap();

    assert_eq!(
        state.registry.get::<HouseDoor>(inside).map(|d| d.house),
        Some(serial),
        "the door in the doorway is not the house's"
    );
    assert!(
        !state.registry.has::<HouseDoor>(outside),
        "a door in the next field was adopted"
    );
    // And it is still an ordinary door in every other respect.
    assert!(state.registry.has::<Door>(inside));
}

/// A door with no house opens for anyone, which is every door in Britannia.
fn door_at(state: &mut WorldState, at: Point) -> EntityId {
    use openshard_state::components::Door;
    let (entity, _) = state
        .registry
        .spawn_with_serial(openshard_protocol::serial::SerialKind::Item)
        .unwrap();
    state.registry.insert(entity, Position(at));
    state.registry.insert(entity, Facet(0));
    state.registry.insert(
        entity,
        Door {
            closed: Graphic(0x06A5),
            open: Graphic(0x06A6),
            offset_x: 1,
            offset_y: 0,
            is_open: false,
            close_at: 0,
        },
    );
    entity
}

/// A ban that only locked the door would leave whoever was already inside there
/// for good. This is the rule that makes one worth anything.
#[test]
fn a_ban_puts_out_whoever_is_already_inside() {
    use openshard_state::components::Body;

    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");

    // Three people standing in the doorway tile: the owner, a friend, and one
    // about to be banned.
    let inside = [owner, an_owner(&mut state), an_owner(&mut state)];
    for who in inside {
        let entity = state.registry.entity_of(who).expect("a mobile");
        state.registry.insert(entity, Position(at));
        state.registry.insert(entity, Facet(0));
        state.registry.insert(
            entity,
            Body {
                id: Graphic(0x0190),
                hue: openshard_protocol::wire::Hue(0),
            },
        );
    }
    let friend = inside[1];
    let unwelcome = inside[2];
    {
        let entry = state.registry.get_mut::<House>(house).expect("its component");
        trust(entry, owner, friend, Standing::Friend, false).unwrap();
        ban(entry, owner, unwelcome, false).unwrap();
    }

    let moved = evict_the_banned(&mut state, house);
    assert_eq!(
        moved.len(),
        1,
        "the wrong number of people were put out: {moved:?}"
    );

    let where_of = |serial: Serial| {
        let entity = state.registry.entity_of(serial).unwrap();
        state.registry.get::<Position>(entity).unwrap().0
    };
    assert_eq!(where_of(owner), at, "the owner was put out of their own house");
    assert_eq!(where_of(friend), at, "a friend was put out");
    assert_ne!(where_of(unwelcome), at, "the banned player stayed inside");
    // Just outside the box's west edge, which is where the doorstep is.
    assert_eq!(where_of(unwelcome), doorstep(&state, at, Facet(0), COTTAGE));
}

/// The sign hangs on the box's west-south corner, seven above the house's z.
///
/// The numbers rather than the rule, because the rule is one reduction away from
/// ServUO's `SetSign(Components.Min.X, Components.Height - 1 - Components.Center.Y, 7)`
/// and a reduction is exactly the kind of thing that is right on paper and off
/// by one in the tree. The cottage's box runs from -1 to +1 on both axes, so the
/// corner is one west and one south of the origin.
#[test]
fn a_house_hangs_its_sign_on_the_corner_of_its_box() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let serial = state.registry.serial_of(house).expect("the house's serial");

    assert_eq!(
        sign_spot(&state, at, Facet(0), COTTAGE),
        Some(Point::new(9, 11, 7))
    );
    let signs: Vec<_> = state
        .registry
        .query::<openshard_state::components::HouseSign>()
        .map(|(entity, sign)| (entity, sign.house))
        .collect();
    assert_eq!(signs.len(), 1, "a house got {} signs", signs.len());
    assert_eq!(signs[0].1, serial, "the sign names another house");
    assert_eq!(
        state.registry.get::<Position>(signs[0].0).map(|p| p.0),
        Some(Point::new(9, 11, 7))
    );
    assert_eq!(
        state.registry.get::<Drawn>(signs[0].0).map(|drawn| drawn.id),
        Some(Graphic(SIGN_GRAPHIC))
    );
    // And it is an item, so the interest sweep announces it like any other.
    assert!(
        state
            .registry
            .serial_of(signs[0].0)
            .is_some_and(|serial| serial.is_item()),
        "the sign took a mobile serial"
    );
}

/// A shard with no client files gets a house with no sign, rather than a sign
/// at the origin.
///
/// The same bargain the walls make. A sign hung at the house's own tile would be
/// *inside* it on every multi whose box is not centred, and a plaque a player
/// cannot reach is worse than no plaque.
#[test]
fn a_house_with_no_multi_table_hangs_no_sign() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    assert_eq!(
        sign_spot(&state, Point::new(10, 10, 0), Facet(0), COTTAGE + 1),
        None,
        "an id the table does not hold got a spot anyway"
    );
    let _ = owner;
}

/// The sign's window is a window over the five verbs, and it obeys them.
///
/// The row is the half a cursor cannot do — taking somebody *off* a list — so
/// this is the branch worth pinning: a friend pressing a co-owner's row changes
/// nothing, and the co-owner pressing it does.
#[test]
fn only_a_co_owner_may_drop_a_name_from_the_window() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");

    let friend = an_owner(&mut state);
    let co_owner = an_owner(&mut state);
    {
        let entry = state.registry.get_mut::<House>(house).expect("its component");
        trust(entry, owner, friend, Standing::Friend, false).unwrap();
        trust(entry, owner, co_owner, Standing::CoOwner, false).unwrap();
    }

    // A friend pressing the row: refused, because `distrust` asks for co-owner.
    let as_friend = state.registry.entity_of(friend).expect("a mobile");
    sign::apply(
        &mut state,
        as_friend,
        house,
        openshard_state::HouseChange::Drop,
        co_owner,
    );
    assert!(
        state
            .registry
            .get::<House>(house)
            .is_some_and(|entry| entry.co_owners.contains(&co_owner)),
        "a friend dropped a co-owner"
    );

    // The co-owner pressing the friend's row: done.
    let as_co_owner = state.registry.entity_of(co_owner).expect("a mobile");
    sign::apply(
        &mut state,
        as_co_owner,
        house,
        openshard_state::HouseChange::Drop,
        friend,
    );
    assert!(
        state
            .registry
            .get::<House>(house)
            .is_some_and(|entry| !entry.friends.contains(&friend)),
        "a co-owner could not drop a friend"
    );
}

/// A row button reads back as the row it was drawn for, and a number past the
/// end reads back as nothing.
#[test]
fn a_row_button_reads_back_as_the_row_it_was_drawn_for() {
    for row in 0..8 {
        assert_eq!(sign::row_of(sign::row_button(row), 8), Some(row));
    }
    assert_eq!(
        sign::row_of(sign::row_button(8), 8),
        None,
        "a reply naming row nine of an eight-row list resolved to something"
    );
    assert_eq!(
        sign::row_of(sign::button::BAN, 8),
        None,
        "an action button was read as a row"
    );
}

/// A house's ceiling is its own footprint at four apiece, computed once and
/// stored.
///
/// The cottage draws five tiles — four walls and a floor — so twenty lockdowns
/// and forty of storage. The number on the component rather than a recomputation
/// is the half worth pinning: the drop path reads it with no terrain in hand.
#[test]
fn a_house_gets_its_allowance_from_its_own_footprint() {
    use crate::storage::{LOCKDOWNS_PER_TILE, allowance, allowance_for};

    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");

    let tiles = tiles_of(&state, at, Facet(0), COTTAGE).len();
    assert_eq!(tiles, 5, "the cottage draws five tiles");
    assert_eq!(
        state.registry.get::<House>(house).map(|entry| entry.lockdowns),
        Some((tiles * LOCKDOWNS_PER_TILE) as u32)
    );
    assert_eq!(allowance(&state, house), allowance_for(tiles));
    assert_eq!(allowance(&state, house).lockdowns, 20);
    assert_eq!(allowance(&state, house).storage, 40);
}

/// Lock down, secure, release — and the three rules that decide each.
#[test]
fn only_a_co_owner_pins_and_only_inside_the_house() {
    use crate::storage::{StorageRefusal, lock_down, locked_down, release};

    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let master = state.registry.entity_of(owner).expect("a mobile");

    // A chest on the house's own floor, and a barrel two tiles outside it.
    let inside = an_item(&mut state, at, true);
    let outside = an_item(&mut state, Point::new(at.x + 8, at.y, at.z), true);

    let stranger = an_owner(&mut state);
    let stranger = state.registry.entity_of(stranger).expect("a mobile");
    assert_eq!(
        lock_down(&mut state, stranger, house, inside, None),
        Err(StorageRefusal::NotYours),
        "a stranger pinned something in somebody else's house"
    );
    assert_eq!(
        lock_down(&mut state, master, house, outside, None),
        Err(StorageRefusal::NotInThisHouse),
        "a thing on the grass was locked down in the house"
    );
    assert_eq!(lock_down(&mut state, master, house, inside, None), Ok(()));
    assert_eq!(locked_down(&state, house), vec![inside]);
    assert_eq!(
        lock_down(&mut state, master, house, inside, None),
        Err(StorageRefusal::NoChange),
        "pinning the same item twice counted twice"
    );

    // A secure has to be a container, and the same item becomes one for free —
    // it is already on the list.
    let plank = an_item(&mut state, at, false);
    assert_eq!(
        lock_down(&mut state, master, house, plank, Some(Standing::Friend)),
        Err(StorageRefusal::NotAContainer)
    );
    assert_eq!(
        lock_down(&mut state, master, house, inside, Some(Standing::Friend)),
        Ok(())
    );
    assert_eq!(
        locked_down(&state, house).len(),
        1,
        "making a lockdown secure spent a second slot"
    );

    assert_eq!(release(&mut state, master, house, inside), Ok(()));
    assert!(locked_down(&state, house).is_empty());
    assert_eq!(
        release(&mut state, master, house, inside),
        Err(StorageRefusal::NoChange)
    );
}

/// The allowance is a ceiling and the ceiling refuses.
#[test]
fn a_full_house_takes_no_more_lockdowns() {
    use crate::storage::{StorageRefusal, allowance, lock_down};

    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let master = state.registry.entity_of(owner).expect("a mobile");

    let ceiling = allowance(&state, house).lockdowns;
    for _ in 0..ceiling {
        let item = an_item(&mut state, at, false);
        assert_eq!(lock_down(&mut state, master, house, item, None), Ok(()));
    }
    let one_too_many = an_item(&mut state, at, false);
    assert_eq!(
        lock_down(&mut state, master, house, one_too_many, None),
        Err(StorageRefusal::NoRoom)
    );
}

/// A secure opens by standing, and every other container in Britannia opens for
/// anybody.
#[test]
fn a_secure_opens_for_the_standing_it_names() {
    use crate::storage::{lock_down, may_open};

    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let master = state.registry.entity_of(owner).expect("a mobile");

    let chest = an_item(&mut state, at, true);
    let plain = an_item(&mut state, at, true);
    lock_down(&mut state, master, house, chest, Some(Standing::CoOwner)).unwrap();

    let friend = an_owner(&mut state);
    let stranger = an_owner(&mut state);
    {
        let entry = state.registry.get_mut::<House>(house).expect("its component");
        trust(entry, owner, friend, Standing::Friend, false).unwrap();
    }
    let friend = state.registry.entity_of(friend).expect("a mobile");
    let stranger = state.registry.entity_of(stranger).expect("a mobile");

    assert!(may_open(&state, master, chest), "the owner was shut out");
    assert!(
        !may_open(&state, friend, chest),
        "a friend opened a co-owners' secure"
    );
    assert!(!may_open(&state, stranger, chest));
    assert!(
        may_open(&state, stranger, plain),
        "an ordinary chest refused a stranger"
    );

    // And "anyone" means the bottom of the ladder, not the absence of one: a
    // banned player is still below it.
    lock_down(&mut state, master, house, chest, Some(Standing::Stranger)).unwrap();
    assert!(may_open(&state, stranger, chest));
    let banned = state.registry.serial_of(stranger).unwrap();
    {
        let entry = state.registry.get_mut::<House>(house).expect("its component");
        ban(entry, owner, banned, false).unwrap();
    }
    assert!(
        !may_open(&state, stranger, chest),
        "a banned player opened a secure standing open to anyone"
    );
}

/// The storage ceiling counts what is in the secures, one level deep.
#[test]
fn the_storage_ceiling_counts_what_is_in_the_secures() {
    use crate::storage::{allowance, has_room_for, lock_down, stored};

    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let master = state.registry.entity_of(owner).expect("a mobile");

    let chest = an_item(&mut state, at, true);
    lock_down(&mut state, master, house, chest, Some(Standing::Friend)).unwrap();
    let chest_serial = state.registry.serial_of(chest).unwrap();

    assert_eq!(stored(&state, house), 0);
    for _ in 0..3 {
        let item = an_item(&mut state, at, false);
        state.registry.remove::<Position>(item);
        state.registry.insert(
            item,
            openshard_state::components::Contained {
                container: chest_serial,
                position: openshard_protocol::gump::GumpPoint::new(0, 0),
                grid: openshard_protocol::containers::GridSlot(0),
            },
        );
    }
    assert_eq!(stored(&state, house), 3);
    assert!(has_room_for(&state, house, allowance(&state, house).storage - 3));
    assert!(
        !has_room_for(&state, house, allowance(&state, house).storage - 2),
        "the ceiling let one past it"
    );
}

/// The six stages are the reference's thresholds, and the boundaries are where
/// it puts them.
///
/// The boundaries rather than a sample from the middle of each band: they are
/// not evenly spaced — the first stage is half a percent of the period and the
/// last is five — so a rounding slip shows up nowhere else.
#[test]
fn a_house_wears_through_the_reference_stages() {
    use crate::decay::Condition;

    for (per_mille, expected) in [
        (0, Condition::LikeNew),
        (4, Condition::LikeNew),
        (5, Condition::Slightly),
        (249, Condition::Slightly),
        (250, Condition::Somewhat),
        (499, Condition::Somewhat),
        (500, Condition::Fairly),
        (749, Condition::Fairly),
        (750, Condition::Greatly),
        (949, Condition::Greatly),
        (950, Condition::InDanger),
        (999, Condition::InDanger),
        (1000, Condition::Collapsed),
        (5000, Condition::Collapsed),
    ] {
        assert_eq!(Condition::at(per_mille), expected, "at {per_mille} per mille");
    }
}

/// The clock runs, the refresh stops it, and a period of zero turns it off.
#[test]
fn the_sweep_ages_a_house_and_a_refresh_undoes_it() {
    use crate::decay::{Condition, age_and_collect, condition, refresh};

    let mut state = world_with(cottage());
    state.gameplay.house_decay_ticks = 1000;
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");

    assert_eq!(condition(&state, house), Condition::LikeNew);
    for _ in 0..600 {
        assert!(age_and_collect(&mut state).is_empty());
    }
    assert_eq!(condition(&state, house), Condition::Fairly);
    refresh(&mut state, house);
    assert_eq!(condition(&state, house), Condition::LikeNew);

    // Decay off: nothing ages, so nothing ever collapses.
    state.gameplay.house_decay_ticks = 0;
    for _ in 0..5000 {
        assert!(age_and_collect(&mut state).is_empty());
    }
    assert_eq!(
        state.registry.get::<House>(house).map(|entry| entry.age),
        Some(0),
        "a shard with decay off still counted"
    );
}

/// The whole of H5 in one house: it comes down, the walls go with it, and what
/// it was holding is in the crate rather than gone.
#[test]
fn a_collapsed_house_leaves_a_crate_and_no_walls() {
    use crate::decay::{CRATE_GRAPHIC, age_and_collect, demolish};
    use crate::storage::lock_down;
    use openshard_state::components::{Contained, Container};

    let mut state = world_with(cottage());
    state.gameplay.house_decay_ticks = 10;
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");
    let master = state.registry.entity_of(owner).expect("a mobile");
    let house_serial = state.registry.serial_of(house).expect("its serial");

    // A locked-down plank, a secure chest, and something inside the chest.
    let plank = an_item(&mut state, at, false);
    let chest = an_item(&mut state, at, true);
    lock_down(&mut state, master, house, plank, None).unwrap();
    lock_down(&mut state, master, house, chest, Some(Standing::Friend)).unwrap();
    let chest_serial = state.registry.serial_of(chest).unwrap();
    let inside = an_item(&mut state, at, false);
    state.registry.remove::<Position>(inside);
    state.registry.insert(
        inside,
        Contained {
            container: chest_serial,
            position: openshard_protocol::gump::GumpPoint::new(0, 0),
            grid: openshard_protocol::containers::GridSlot(0),
        },
    );
    // And a loose barrel nobody pinned, which is not the house's to move.
    let loose = an_item(&mut state, at, false);

    // The walls are up before, and down after.
    assert!(
        state
            .facet_state(Facet(0))
            .obstructions
            .is_blocked(at.x - 1, at.y - 1),
        "the cottage never had walls"
    );
    let mut down = Vec::new();
    for _ in 0..11 {
        down = age_and_collect(&mut state);
        if !down.is_empty() {
            break;
        }
    }
    assert_eq!(down, vec![house], "the period ran out and nothing collapsed");
    demolish(&mut state, house);

    assert!(
        !state
            .facet_state(Facet(0))
            .obstructions
            .is_blocked(at.x - 1, at.y - 1),
        "the walls outlived the house"
    );
    assert!(
        state.registry.serial_of(house).is_none(),
        "the house is still there"
    );
    assert!(
        state
            .registry
            .query::<openshard_state::components::HouseSign>()
            .next()
            .is_none(),
        "the sign outlived its house"
    );

    // One crate, on the house's own tile, holding the plank and the chest — and
    // the chest still holding what was in it.
    let crates: Vec<_> = state
        .registry
        .query::<Container>()
        .filter(|(entity, _)| {
            state
                .registry
                .get::<Drawn>(*entity)
                .is_some_and(|drawn| drawn.id == Graphic(CRATE_GRAPHIC))
        })
        .map(|(entity, _)| entity)
        .collect();
    assert_eq!(crates.len(), 1, "the wrong number of crates");
    let crate_serial = state.registry.serial_of(crates[0]).unwrap();
    assert_eq!(state.registry.get::<Position>(crates[0]).map(|p| p.0), Some(at));

    let packed: Vec<EntityId> = state
        .registry
        .query::<Contained>()
        .filter(|(_, held)| held.container == crate_serial)
        .map(|(entity, _)| entity)
        .collect();
    assert_eq!(packed.len(), 2, "the crate holds {packed:?}");
    assert!(packed.contains(&plank) && packed.contains(&chest));
    assert_eq!(
        state.registry.get::<Contained>(inside).map(|held| held.container),
        Some(chest_serial),
        "the chest was emptied into the crate beside it"
    );
    assert!(
        state.registry.get::<Position>(loose).is_some(),
        "the loose barrel was swept up with the house's own things"
    );
    assert!(
        !state
            .registry
            .has::<openshard_state::components::LockedDown>(plank),
        "the plank came out of the house still pinned to it"
    );
    let _ = house_serial;
}

/// A house with nothing pinned in it leaves no crate.
#[test]
fn an_empty_house_leaves_no_crate() {
    let mut state = world_with(cottage());
    let owner = an_owner(&mut state);
    let at = Point::new(10, 10, 0);
    let house = place(&mut state, at, Facet(0), COTTAGE, owner).expect("a legal spot");

    assert_eq!(crate::decay::demolish(&mut state, house), None);
    assert!(
        state
            .registry
            .query::<openshard_state::components::Container>()
            .next()
            .is_none(),
        "an empty house left a crate to stand in the road"
    );
}
