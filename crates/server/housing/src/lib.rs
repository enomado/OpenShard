//! Player houses: placing one, and the ground it is allowed to stand on.
//!
//! A **multi** is one item that draws as many. The wire carries a house as an
//! ordinary world item whose graphic is `0x4000 | id`; the client looks the id up
//! in its own files and draws the hundred and forty-eight statics a villa is made
//! of. **This crate sends none of them.**
//!
//! That is what makes a house tractable: the picture is free, because every
//! client already owns every house. What the shard owes is the half the picture
//! does not carry — where a wall is for the purpose of *stopping* somebody, and
//! whether this patch of Britannia was somewhere a house may go at all.
//!
//! See [`docs/housing.md`](../../../../docs/housing.md) for the five phases and
//! the decisions; this is H1.
//!
//! # Where the components come from
//!
//! [`Terrain::multi_components`](openshard_movement::Terrain::multi_components),
//! the same seam `item_blocks` and `item_height` already reach gameplay through.
//! A multi's shape is a client-file fact, and this crate reads it the way every
//! other gameplay crate reads one: by asking the terrain, which answers nothing
//! at all on a shard with no client files.
//!
//! # The footprint is stored, not recomputed
//!
//! Placement folds the blocking components into
//! [`Obstructions`](openshard_state::Obstructions) once. A step is ten a second
//! and a house does not move, so asking `multi.mul` per step would be paying a
//! hundred lookups for an answer that cannot have changed.

#[cfg(test)]
mod tests;

use openshard_entities::EntityId;
use openshard_movement::Tile;
use openshard_protocol::serial::Serial;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{Facet, Point};
use openshard_state::components::{Drawn, House, Position};
use openshard_state::{Obstructions, WorldState};

/// The bit that turns a multi id into the graphic the wire carries.
///
/// A mask rather than an addition on the way back: a caller may hold either
/// spelling, and `graphic & !MULTI_FLAG` is the id whichever it had.
pub const MULTI_FLAG: u16 = 0x4000;

/// The first customisable-house foundation id, and the last.
///
/// ServUO's `HousePlacement.Check` adds stairs to any multi in this range,
/// because a foundation's own component list has none — the stairs are part of
/// the *design*, which is a system this engine does not have. A foundation placed
/// without them is a house nobody can get into, so the range is refused by name
/// rather than placed and wondered about. See `docs/housing.md`'s D7.
pub const FOUNDATION_IDS: std::ops::Range<u16> = 0x13EC..0x1D00;

/// Why a house could not go there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No client files, or an id no client knows: there is nothing to place.
    NoSuchMulti,
    /// A customisable foundation, which needs a design system to have stairs.
    /// See [`FOUNDATION_IDS`].
    NeedsCustomisation,
    /// The multi is in the table and draws nothing — a treasure-site marker
    /// rather than a building. See `findings.md`.
    DrawsNothing,
    /// Part of the footprint is off the edge of the world.
    OffTheMap,
    /// Something already stands where the house would.
    Occupied,
    /// A footprint tile is over a road, a furrow or sand stones. ServUO's fifth
    /// rule, and the one a player notices the absence of: without it houses go
    /// up across Britain's streets.
    OnARoad,
    /// The ground will not take the house — a wall in the way, or thin air with
    /// no surface under it. ServUO's rules two and four, which `can_fit` asks as
    /// one question.
    BadGround,
    /// Another house's yard. Every house keeps five tiles to itself.
    TooCloseToAHouse,
    /// The serial pool is dry, which is a shard in trouble rather than a bad spot.
    NoSerials,
}

impl Refusal {
    /// What to say to whoever tried.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoSuchMulti => "No house has that number.",
            Self::NeedsCustomisation => "That is a customisable foundation, which this shard cannot build.",
            Self::DrawsNothing => "That multi is a marker, not a building.",
            Self::OffTheMap => "The house would hang off the edge of the world.",
            Self::Occupied => "Something is in the way.",
            Self::OnARoad => "A house may not be built on a road.",
            Self::BadGround => "The ground will not take a house here.",
            Self::TooCloseToAHouse => "That is too close to another house.",
            Self::NoSerials => "The shard is out of item serials.",
        }
    }
}

/// One tile of a house's footprint, already in world coordinates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Footprint {
    /// Where.
    pub tile: Tile,
    /// The z the component stands at, the house's own z included.
    pub z: i8,
    /// How tall it blocks, from its tiledata.
    pub height: u8,
}

/// Put a house on the ground and make its walls stop people.
///
/// `at` is the multi's **origin**, which is not the corner of its box — see
/// [`Multi::center`](openshard_uofiles::multi::Multi::center). It is the tile the
/// player clicked, and matching the reference's arithmetic for it is what keeps a
/// house from landing one tile off the spot they picked.
///
/// Returns the house's entity. The caller announces it; this does not, for
/// `spawn_item`'s reason — what a placement *is* to the world (a staff command, a
/// deed being consumed) is the caller's business.
pub fn place(
    state: &mut WorldState,
    at: Point,
    facet: Facet,
    multi: u16,
    owner: Serial,
) -> Result<EntityId, Refusal> {
    let multi = multi & !MULTI_FLAG;
    if FOUNDATION_IDS.contains(&multi) {
        return Err(Refusal::NeedsCustomisation);
    }
    let footprint = footprint_of(state, at, facet, multi)?;
    if footprint.is_empty() {
        return Err(Refusal::DrawsNothing);
    }
    if occupied_tile(state, facet, &footprint).is_some() {
        return Err(Refusal::Occupied);
    }
    check_ground(state, facet, &footprint)?;
    check_yard(state, facet, &footprint)?;

    let Ok((entity, _)) = state
        .registry
        .spawn_with_serial(openshard_protocol::serial::SerialKind::Item)
    else {
        return Err(Refusal::NoSerials);
    };
    state.registry.insert(
        entity,
        Drawn {
            id: Graphic(MULTI_FLAG | multi),
            hue: Hue(0),
        },
    );
    state.registry.insert(entity, Position(at));
    state.registry.insert(
        entity,
        House {
            multi,
            owner,
            co_owners: Default::default(),
            friends: Default::default(),
            bans: Default::default(),
        },
    );
    state.registry.insert(entity, facet);
    // On the sector grid like any item, so a client entering the area is told
    // about it by the ordinary interest sweep rather than by a path of its own.
    state.facet_state_mut(facet).sectors.insert(entity, at);
    let obstructions = &mut state.facet_state_mut(facet).obstructions;
    block_footprint(obstructions, entity, &footprint);
    adopt_doors(state, entity, facet, at, multi);
    Ok(entity)
}

/// How many co-owners a house may have — ServUO's `MaxCoOwners`.
pub const MAX_CO_OWNERS: usize = 15;
/// How many friends — ServUO's AoS `MaxFriends`. It is 50 before AoS, and this
/// engine's floor is AoS.
pub const MAX_FRIENDS: usize = 140;
/// How many bans, on the same terms.
pub const MAX_BANS: usize = 140;

/// Where somebody stands with a house — re-exported from `openshard-state`.
///
/// The type lives beside the data because a *door* has to ask it and the
/// double-click dispatch is `openshard-items`', which does not depend on this
/// crate. See [`Standing`](openshard_state::Standing)'s own docs: it is
/// [`Guild`](openshard_state::Guild)'s split, where the rules are the system
/// crate's and the question a wire path asks lives on the component.
pub use openshard_state::Standing;

/// Why a change to a house's lists was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListRefusal {
    /// The actor is not trusted enough to make this change.
    NotYours,
    /// That list is full.
    Full,
    /// The owner cannot be made a friend of, banned from, or evicted from their
    /// own house.
    NotTheOwner,
}

impl ListRefusal {
    /// What to say to whoever tried.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NotYours => "That is not your house to change.",
            Self::Full => "That list is full.",
            Self::NotTheOwner => "That cannot be done to the owner.",
        }
    }
}

/// Trust somebody with the house, at `standing`.
///
/// Only [`Standing::Friend`] and [`Standing::CoOwner`] can be granted: an owner
/// is made by transferring the house and a ban is [`ban`]. Granting **moves**
/// somebody between the two lists rather than adding them to both, so a friend
/// promoted to co-owner is in one place and the question has one answer.
///
/// A co-owner may add friends; only the owner may add co-owners. ServUO's own
/// split, and the reason it is not "whoever is trusted may share the trust they
/// have": a co-owner who could name another co-owner could hand the house to a
/// crowd the owner never met.
pub fn trust(
    house: &mut House,
    actor: Serial,
    who: Serial,
    standing: Standing,
    staff: bool,
) -> Result<(), ListRefusal> {
    let actor_standing = house.standing_of(actor, staff);
    let needed = match standing {
        Standing::CoOwner => Standing::Owner,
        _ => Standing::CoOwner,
    };
    if actor_standing < needed {
        return Err(ListRefusal::NotYours);
    }
    if who == house.owner {
        return Err(ListRefusal::NotTheOwner);
    }
    let (list, limit) = match standing {
        Standing::CoOwner => (&mut house.co_owners, MAX_CO_OWNERS),
        _ => (&mut house.friends, MAX_FRIENDS),
    };
    if !list.contains(&who) && list.len() >= limit {
        return Err(ListRefusal::Full);
    }
    list.insert(who);
    // Out of the other one: two lists holding the same person is two answers to
    // one question, and `standing_of` would silently prefer whichever it checked
    // first.
    match standing {
        Standing::CoOwner => house.friends.remove(&who),
        _ => house.co_owners.remove(&who),
    };
    house.bans.remove(&who);
    Ok(())
}

/// Take somebody off both trusted lists. A co-owner may drop a friend; only the
/// owner may drop a co-owner.
pub fn distrust(house: &mut House, actor: Serial, who: Serial, staff: bool) -> Result<(), ListRefusal> {
    let actor_standing = house.standing_of(actor, staff);
    let needed = if house.co_owners.contains(&who) {
        Standing::Owner
    } else {
        Standing::CoOwner
    };
    if actor_standing < needed {
        return Err(ListRefusal::NotYours);
    }
    if who == house.owner {
        return Err(ListRefusal::NotTheOwner);
    }
    house.co_owners.remove(&who);
    house.friends.remove(&who);
    Ok(())
}

/// Turn somebody away from the house.
///
/// A ban is the newer decision and it wins over the trusted lists: banning a
/// co-owner takes them off it, because "banned but still a co-owner" is a state
/// with no useful answer and the ban is the thing that was just decided.
pub fn ban(house: &mut House, actor: Serial, who: Serial, staff: bool) -> Result<(), ListRefusal> {
    if house.standing_of(actor, staff) < Standing::CoOwner {
        return Err(ListRefusal::NotYours);
    }
    if who == house.owner {
        return Err(ListRefusal::NotTheOwner);
    }
    if !house.bans.contains(&who) && house.bans.len() >= MAX_BANS {
        return Err(ListRefusal::Full);
    }
    house.bans.insert(who);
    house.co_owners.remove(&who);
    house.friends.remove(&who);
    Ok(())
}

/// Let a banned player back to the door. They come back a stranger, not a
/// friend: undoing a ban is not the same as granting anything.
pub fn unban(house: &mut House, actor: Serial, who: Serial, staff: bool) -> Result<(), ListRefusal> {
    if house.standing_of(actor, staff) < Standing::CoOwner {
        return Err(ListRefusal::NotYours);
    }
    house.bans.remove(&who);
    Ok(())
}

/// Hand every door standing inside a footprint to the house.
///
/// # Why a house adopts its doors rather than placing them
///
/// The obvious source is the multi itself, and it is not one: of the 326 multis
/// a shipped `multi.mul` holds, **three** carry a door component. The reference
/// agrees — ServUO's houses call `AddDoor` from each house class with an explicit
/// graphic and position, which is a per-house-type table of *content* this engine
/// does not have and should not invent.
///
/// So the rule is the one a player would state: a door standing inside your house
/// is your house's door. It is derivable from what is already on the ground, it
/// needs no table, and it is right for a door added by a pack, by a staff command
/// or by a later customisation system without any of them knowing about it.
///
/// Called at placement, and again whenever a door is put down — a house cannot
/// adopt a door that does not exist yet.
pub fn adopt_doors(state: &mut WorldState, house: EntityId, facet: Facet, at: Point, multi: u16) {
    let Some(serial) = state.registry.serial_of(house) else {
        return;
    };
    // **Every drawn tile, not the blocking footprint.** A door stands in a
    // *doorway*, which is by construction a gap in the walls — the one place the
    // footprint does not reach. Using it here adopted nothing, which a test
    // caught rather than a player.
    let area = tiles_of(state, at, facet, multi);
    let inside: Vec<EntityId> = state
        .registry
        .query::<openshard_state::components::Door>()
        .map(|(entity, _)| entity)
        .filter(|&entity| state.facet_of(entity) == facet)
        .filter(|&entity| {
            state
                .registry
                .get::<Position>(entity)
                .is_some_and(|&Position(at)| area.contains(&Tile::new(at.x, at.y)))
        })
        .collect();
    for door in inside {
        state
            .registry
            .insert(door, openshard_state::components::HouseDoor { house: serial });
    }
}

/// Where a banned player is put out to.
///
/// One tile west of the house's box, at the ground the house stands on. ServUO
/// moves them to the sign's own spot, which this engine has no sign for yet — and
/// "just outside, on the side the box ends" is the same intent with data that
/// exists.
#[must_use]
pub fn doorstep(state: &WorldState, at: Point, facet: Facet, multi: u16) -> Point {
    let tiles = tiles_of(state, at, facet, multi);
    let west = tiles.iter().map(|tile| tile.x).min().unwrap_or(at.x);
    Point::new(west.saturating_sub(1), at.y, at.z)
}

/// Put every banned player standing inside a house out of it.
///
/// The one rule in H3 that *acts* on somebody rather than refusing them, and the
/// reason a ban is worth anything at all: a ban that only locked the door would
/// leave whoever was already inside there for good.
///
/// Returns who was moved, so the caller can tell them — this crate does not send
/// packets, for `place`'s reason.
pub fn evict_the_banned(state: &mut WorldState, house: EntityId) -> Vec<EntityId> {
    let Some(entry) = state.registry.get::<House>(house).cloned() else {
        return Vec::new();
    };
    let Some(&Position(at)) = state.registry.get::<Position>(house) else {
        return Vec::new();
    };
    let facet = state.facet_of(house);
    let area = tiles_of(state, at, facet, entry.multi);
    let out = doorstep(state, at, facet, entry.multi);

    let caught: Vec<EntityId> = state
        .registry
        .query::<Position>()
        .filter(|(entity, _)| state.registry.has::<openshard_state::components::Body>(*entity))
        .filter(|(entity, _)| state.facet_of(*entity) == facet)
        .filter(|(_, Position(where_they_are))| area.contains(&Tile::new(where_they_are.x, where_they_are.y)))
        .filter(|(entity, _)| {
            state
                .registry
                .serial_of(*entity)
                .is_some_and(|who| entry.standing_of(who, state.is_staff(*entity)) == Standing::Banned)
        })
        .map(|(entity, _)| entity)
        .collect();
    for who in &caught {
        state.registry.insert(*who, Position(out));
    }
    caught
}

/// Every tile a house covers — its drawn components, blocking or not.
///
/// The footprint's counterpart, and the difference matters: a footprint is what
/// *stops* somebody and a doorway is a gap in it, so "does this house cover this
/// tile" and "does this house block this tile" are two questions with two
/// answers.
#[must_use]
pub fn tiles_of(state: &WorldState, at: Point, facet: Facet, multi: u16) -> Vec<Tile> {
    let multi = multi & !MULTI_FLAG;
    let Some(terrain) = state.facet_state(facet).terrain.as_deref() else {
        return Vec::new();
    };
    let mut out: Vec<Tile> = terrain
        .multi_components(multi)
        .iter()
        .filter(|component| component.drawn())
        .filter_map(|component| {
            let x = u16::try_from(i32::from(at.x) + i32::from(component.dx)).ok()?;
            let y = u16::try_from(i32::from(at.y) + i32::from(component.dy)).ok()?;
            Some(Tile::new(x, y))
        })
        .collect();
    out.sort_unstable_by_key(|tile| (tile.x, tile.y));
    out.dedup();
    out
}

/// Put a house's walls into the obstruction index.
///
/// [`place`]'s last step, public because the boot path takes it on its own: a
/// saved house is not re-placed — that would ask whether it *may* stand there,
/// and a house legal when it was built stays built even if the rules have since
/// tightened — so restoring one is the registry half by hand and this.
pub fn block(state: &mut WorldState, entity: EntityId, facet: Facet, footprint: &[Footprint]) {
    let obstructions = &mut state.facet_state_mut(facet).obstructions;
    block_footprint(obstructions, entity, footprint);
}

/// Take a house's walls back out of the obstruction index.
///
/// The entity itself is the caller's to despawn: this is the half that has to
/// happen *before* it goes, because the footprint is derived from where it stood.
pub fn unblock(state: &mut WorldState, entity: EntityId, facet: Facet, footprint: &[Footprint]) {
    let obstructions = &mut state.facet_state_mut(facet).obstructions;
    for spot in footprint {
        obstructions.unblock(spot.tile.x, spot.tile.y, entity);
    }
}

/// Where a house standing at `at` would block, and how tall at each tile.
///
/// Public because the boot path needs it to rebuild the index from a saved house
/// without going through [`place`]'s refusals — a house that was legal when it
/// was placed stays placed, even if the rules have since tightened.
pub fn footprint_of(
    state: &WorldState,
    at: Point,
    facet: Facet,
    multi: u16,
) -> Result<Vec<Footprint>, Refusal> {
    let multi = multi & !MULTI_FLAG;
    let Some(terrain) = state.facet_state(facet).terrain.as_deref() else {
        return Err(Refusal::NoSuchMulti);
    };
    let components = terrain.multi_components(multi);
    if components.is_empty() {
        return Err(Refusal::NoSuchMulti);
    }
    let mut out = Vec::new();
    for component in components.iter().filter(|c| c.drawn()) {
        let graphic = Graphic(component.graphic);
        // Only what actually stops somebody. A floor tile and a roof are drawn and
        // walked over; folding them in would seal a house shut from the inside.
        if !terrain.item_blocks(graphic) {
            continue;
        }
        let x = i32::from(at.x) + i32::from(component.dx);
        let y = i32::from(at.y) + i32::from(component.dy);
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
            return Err(Refusal::OffTheMap);
        };
        let z = i32::from(at.z) + i32::from(component.dz);
        let Ok(z) = i8::try_from(z) else {
            return Err(Refusal::OffTheMap);
        };
        out.push(Footprint {
            tile: Tile::new(x, y),
            z,
            height: terrain.item_height(graphic).max(1),
        });
    }
    Ok(out)
}

/// The land tile ranges a house may not stand on — ServUO's `RoadIDs`, inclusive
/// pairs.
///
/// Roads, cobbles, sand stones and ploughed furrows. A furrow is in the list for
/// the same reason a road is: it is somebody's field, and Britannia's farms are
/// as much scenery as its streets.
const ROAD_TILES: [(u16, u16); 8] = [
    (0x0071, 0x0078),
    (0x00E8, 0x00EB),
    (0x07AE, 0x07B1),
    (0x3FF4, 0x3FF4),
    (0x3FF8, 0x3FFB),
    (0x0442, 0x0479), // sand stones
    (0x0501, 0x0510), // sand stones
    (0x0009, 0x0015), // furrows
];

/// A second range of furrows, kept apart only because the array above is a
/// fixed-size literal and this is the ninth pair.
const MORE_FURROWS: (u16, u16) = (0x0150, 0x015C);

/// Whether a land tile is one a house may not stand on.
#[must_use]
pub fn is_road(land: u16) -> bool {
    ROAD_TILES
        .iter()
        .chain(std::iter::once(&MORE_FURROWS))
        .any(|&(low, high)| (low..=high).contains(&land))
}

/// How many tiles of yard a house keeps to itself, in every direction.
///
/// ServUO's `YardSize`, applied as a square rather than as its front-and-back
/// strip. The reference's rule is directional because a *foundation* has a front
/// and a back; a classic multi does not carry which way it faces, so a square is
/// the honest reading of "five tiles clear" for the shape this engine places.
/// Written down because it is a divergence, not an oversight.
pub const YARD: u16 = 5;

/// ServUO's rules two and four: nothing solid in the way, and something to stand
/// on.
///
/// `can_fit` asks both at once — it is "an open gap with a floor", so a solid
/// wall and thin air are the same refusal from its point of view — and it asks
/// them against the *map's* statics, which is the half `occupied_tile` cannot
/// see. And rule five, the road, which is a land-tile id rather than a shape.
fn check_ground(state: &WorldState, facet: Facet, footprint: &[Footprint]) -> Result<(), Refusal> {
    let Some(terrain) = state.facet_state(facet).terrain.as_deref() else {
        return Ok(()); // no map, no opinion — every other check here says the same
    };
    for spot in footprint {
        if terrain.land_tile(spot.tile).is_some_and(|land| is_road(land.0)) {
            return Err(Refusal::OnARoad);
        }
        if !terrain.can_fit(spot.tile, i32::from(spot.z), i32::from(spot.height).max(1)) {
            return Err(Refusal::BadGround);
        }
    }
    Ok(())
}

/// ServUO's rule three: a house keeps [`YARD`] tiles to itself.
///
/// Asked against the other houses' own footprints rather than against a stored
/// yard rectangle, because a footprint is what a house *is* and a rectangle would
/// be a second copy of it to keep in step. There are a handful of houses within
/// a few tiles of anywhere, so the scan is over the ones near enough to matter.
fn check_yard(state: &WorldState, facet: Facet, footprint: &[Footprint]) -> Result<(), Refusal> {
    let mine: Vec<Tile> = footprint.iter().map(|spot| spot.tile).collect();
    for (entity, house) in state.registry.query::<House>() {
        if state.registry.get::<Facet>(entity) != Some(&facet) {
            continue;
        }
        let Some(&Position(at)) = state.registry.get::<Position>(entity) else {
            continue;
        };
        let Ok(theirs) = footprint_of(state, at, facet, house.multi) else {
            continue;
        };
        for other in &theirs {
            if mine.iter().any(|tile| within_yard(*tile, other.tile)) {
                return Err(Refusal::TooCloseToAHouse);
            }
        }
    }
    Ok(())
}

/// Whether two tiles are inside one yard of each other.
fn within_yard(one: Tile, other: Tile) -> bool {
    one.x.abs_diff(other.x) <= YARD && one.y.abs_diff(other.y) <= YARD
}

/// Register every wall of a footprint against one entity.
fn block_footprint(obstructions: &mut Obstructions, entity: EntityId, footprint: &[Footprint]) {
    for spot in footprint {
        // Not a door: a house's own doors are entities of their own, placed on
        // top of it, and a wall a mobile could ask to open is a wall that stops
        // nobody who knows how.
        obstructions.block(spot.tile.x, spot.tile.y, entity, false, spot.z, spot.height);
    }
}

/// The first tile of `footprint` something already stands on, if any.
///
/// The narrow half of ServUO's five rules: this is "no impassable object may come
/// in direct contact with any part of the house", asked of the *dynamic* index
/// only. The map's own statics, the yard clearance, the flat foundation and the
/// road are the rest of D3 and are not here yet — see `docs/housing.md`, which
/// says so rather than letting a reader assume the check is complete.
fn occupied_tile(state: &WorldState, facet: Facet, footprint: &[Footprint]) -> Option<Tile> {
    let obstructions = &state.facet_state(facet).obstructions;
    footprint
        .iter()
        .find(|spot| {
            obstructions
                .blocker_at_z(spot.tile.x, spot.tile.y, i32::from(spot.z))
                .is_some()
        })
        .map(|spot| spot.tile)
}
