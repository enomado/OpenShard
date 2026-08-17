//! Ships: putting one on the water, and what it does to the tiles it covers.
//!
//! A boat is a multi, exactly as a house is — one entity whose graphic is
//! `0x4000 | id`, drawn by every client out of its own `multi.mul`, with no
//! component ever on the wire. Everything in `openshard-housing`'s placement
//! path applies, and this crate is deliberately that path's shape rather than a
//! generalisation of it — and it does **not** depend on that crate. The two are
//! siblings: a shared "multi placement" abstraction would have to be designed
//! before either caller needed it, and the shape is cheaper to repeat than the
//! abstraction is to get wrong.
//!
//! # What is different from a house, and it is only two things
//!
//! **It goes on water, and a house may not.** [`place`] refuses a berth whose
//! tiles are not sea, through `Terrain::land_is_water` — the seam
//! `item_blocks`, `item_height` and `multi_components` all came through, rather
//! than a third notion of "water" beside the client's tile flags and fishing's
//! id ranges.
//!
//! **Its tiles go in [`Boats`] and not in `Obstructions`.** That index only
//! subtracts, and a deck is somewhere to stand over water that is otherwise not
//! ground at all — `docs/boats.md`'s B3, argued in `openshard_state::boat`.
//!
//! # And it sails
//!
//! [`step`] moves the hull a tile and takes whoever is standing on the deck with
//! it. What is still B2's is the **wire**: nothing tells a client the ship has
//! moved, so one that already has it on screen keeps drawing it where it was
//! until something else refreshes that screen. Everyone aboard is redrawn
//! correctly, because each of them is relocated through the ordinary move path.

use openshard_entities::EntityId;
use openshard_movement::Tile;
use openshard_protocol::serial::Serial;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{Facet, Point};
use openshard_state::WorldState;
use openshard_state::boat::Plank;
use openshard_state::components::{Boat, Drawn, Movement, Position};

/// The bit a multi's graphic carries on the wire.
///
/// The protocol's own, not a copy of housing's: a boat is a multi for the same
/// reason a house is, and both are reading the same wire fact. This crate does
/// **not** depend on `openshard-housing` — they are siblings, and the one thing
/// they share is a constant that belongs to neither.
pub const MULTI_FLAG: u16 = openshard_protocol::wire::MultiId::FLAG;

/// Why a boat could not be launched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No multi by that id, or one that draws nothing. A fact about the id.
    NoSuchMulti,
    /// A component would land off the edge of the world. Arithmetic, not
    /// judgement — there is no tile there to float on.
    OffTheMap,
    /// Some of the berth is not sea.
    NotOnWater,
    /// Another ship is already there.
    Occupied,
    /// The registry could not mint a serial.
    NoSerials,
}

impl Refusal {
    /// What to say to whoever tried.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoSuchMulti => "No ship by that name.",
            Self::OffTheMap => "That is off the edge of the world.",
            Self::NotOnWater => "A ship has to float.",
            Self::Occupied => "There is already a ship there.",
            Self::NoSerials => "The world is full.",
        }
    }
}

/// One tile of a berth: where it is, and what the ship puts there.
///
/// The pair [`Boats::moor`](openshard_state::Boats::moor) takes, named so that
/// deriving a berth and mooring one are visibly the same thing.
type Berth = ((u16, u16), Plank);

/// The tiles a boat standing at `at` would cover, and what each one is.
///
/// Public because the boot path needs it without going through [`place`]'s
/// refusals — a ship that was afloat when it was launched stays afloat, the way
/// a house that was legal when it was built stays built.
///
/// A component that blocks by its tiledata is hull; everything else is deck. The
/// split is the whole of what [`Boats`](openshard_state::Boats) needs, and it is
/// made once here rather than per step.
///
/// Undrawn components are skipped, the way a house's footprint skips them: the
/// signature tile every multi opens with is not part of the ship.
pub fn planks_of(
    state: &WorldState,
    boat: EntityId,
    at: Point,
    facet: Facet,
    multi: u16,
) -> Result<Vec<Berth>, Refusal> {
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
        let (Ok(x), Ok(y)) = (
            u16::try_from(i32::from(at.x) + i32::from(component.dx)),
            u16::try_from(i32::from(at.y) + i32::from(component.dy)),
        ) else {
            return Err(Refusal::OffTheMap);
        };
        let Ok(z) = i8::try_from(i32::from(at.z) + i32::from(component.dz)) else {
            return Err(Refusal::OffTheMap);
        };
        out.push((
            (x, y),
            Plank {
                boat,
                z,
                height: terrain.item_height(graphic),
                blocks: terrain.item_blocks(graphic),
            },
        ));
    }
    if out.is_empty() {
        return Err(Refusal::NoSuchMulti);
    }
    Ok(out)
}

/// Put a ship on the water.
///
/// `housing::place`'s shape, including the staff exemption: `actor`
/// is taken so the *judgements* about the berth can be skipped for a game
/// master, while the facts about the id — no such multi, off the edge of the
/// world — hold for everybody. Housing's D10 table is the specification and the
/// split is the same one, with the same reasoning.
pub fn place(
    state: &mut WorldState,
    actor: EntityId,
    at: Point,
    facet: Facet,
    multi: u16,
    owner: Serial,
) -> Result<EntityId, Refusal> {
    let staff = state.is_staff(actor);
    let multi = multi & !MULTI_FLAG;

    let Ok((entity, _)) = state
        .registry
        .spawn_with_serial(openshard_protocol::serial::SerialKind::Item)
    else {
        return Err(Refusal::NoSerials);
    };
    // The shape is derived before anything is written, so a refusal below leaves
    // nothing behind but the serial — which is the one thing that cannot be
    // taken back, and is why this is the first fallible step after it.
    let berth = match planks_of(state, entity, at, facet, multi) {
        Ok(berth) => berth,
        Err(refusal) => {
            state.registry.despawn(entity);
            return Err(refusal);
        }
    };

    if !staff {
        if let Err(refusal) = check_berth(state, facet, &berth) {
            state.registry.despawn(entity);
            return Err(refusal);
        }
    }

    state.registry.insert(
        entity,
        Drawn {
            id: Graphic(MULTI_FLAG | multi),
            hue: Hue(0),
        },
    );
    state.registry.insert(entity, Position(at));
    state.registry.insert(entity, Boat { multi, owner });
    state.registry.insert(entity, facet);
    // On the sector grid like any item, so a client sailing into view is told
    // about it by the ordinary interest sweep rather than by a path of its own.
    state.facet_state_mut(facet).sectors.insert(entity, at);
    state.facet_state_mut(facet).boats.moor(entity, berth);
    Ok(entity)
}

/// The two judgements about a berth: it is all sea, and nothing else is in it.
///
/// Both are staff-exempt, for housing's D10 reason — they are statements about
/// the *place* and a game master is allowed to put a ship in a fountain.
fn check_berth(state: &WorldState, facet: Facet, berth: &[((u16, u16), Plank)]) -> Result<(), Refusal> {
    let facet_state = state.facet_state(facet);
    let Some(terrain) = facet_state.terrain.as_deref() else {
        return Err(Refusal::NoSuchMulti);
    };
    for &((x, y), _) in berth {
        if !terrain.land_is_water(Tile::new(x, y)) {
            return Err(Refusal::NotOnWater);
        }
        if facet_state.boats.boat_at(x, y).is_some() {
            return Err(Refusal::Occupied);
        }
    }
    Ok(())
}

/// Sail one tile, carrying whoever is aboard.
///
/// `World::step`'s structure — decide, then apply — with the terrain check
/// replaced by *does the whole translated berth fit*. Nothing is written until
/// every question has been asked, so a refused course leaves the ship exactly
/// where it was with everybody still standing on it.
///
/// # The manifest is derived, never stored
///
/// Who moves with the ship is worked out here, per move, from the tiles it
/// covers and the sector grid — `docs/boats.md`'s B1a. An `OnDeck` component
/// would be a second copy of a fact [`Position`] already holds, and the two
/// would disagree the first time anything moved a body without going through
/// this function.
///
/// # And each of them is moved absolutely
///
/// B1's refusal of a parent transform, on the engine's own evidence: this world
/// has no way to carry one entity by moving another — mounting *deletes* the
/// mount rather than carrying it. So the deck's occupants are relocated one by
/// one through [`WorldState::move_to`], which is the **fourth** caller of the
/// `disrupt` → position → `refresh_around` → `broadcast_move` sequence after
/// `npc::live`, `quests::advance_escorts` and the tick's own `step`, and the
/// point `docs/boats.md:384` names as when that tail wants a name of its own.
///
/// # What this does not do yet
///
/// **The hull is not redrawn.** A client that already has the ship on its screen
/// keeps drawing it where it was until something else refreshes it; the
/// forget-then-reveal that fixes it is B2's fourth step, and the number of
/// packets a move costs is owed with it.
pub fn step(
    state: &mut WorldState,
    boat: EntityId,
    direction: openshard_protocol::direction::Direction,
) -> Result<Point, Refusal> {
    let facet = state.facet_of(boat);
    let (Some(&Position(at)), Some(&Boat { multi, .. })) = (
        state.registry.get::<Position>(boat),
        state.registry.get::<Boat>(boat),
    ) else {
        return Err(Refusal::NoSuchMulti);
    };
    let Some(to) = openshard_movement::step_from(at, direction) else {
        return Err(Refusal::OffTheMap);
    };

    // Decide. The new berth is derived before anything is written and checked
    // against the water and against every *other* ship — a hull is not in
    // `Obstructions`, so nothing else in this engine would notice two of them in
    // one tile.
    let berth = planks_of(state, boat, to, facet, multi)?;
    check_course(state, facet, boat, &berth)?;
    let manifest = aboard(state, boat, facet);

    // Apply. The index first, so a body relocated below lands on a deck that is
    // already where the ship is going rather than on the one it is leaving.
    state.facet_state_mut(facet).boats.moor(boat, berth);
    state.registry.insert(boat, Position(to));
    state.facet_state_mut(facet).sectors.insert(boat, to);

    let (dx, dy) = (
        i32::from(to.x) - i32::from(at.x),
        i32::from(to.y) - i32::from(at.y),
    );
    for (occupant, was) in manifest {
        let (Ok(x), Ok(y)) = (
            u16::try_from(i32::from(was.x) + dx),
            u16::try_from(i32::from(was.y) + dy),
        ) else {
            // The berth fits, so this cannot happen for anyone standing inside
            // it — but a body at the edge of the coordinate space is left where
            // it is rather than wrapped to the far side of the world.
            continue;
        };
        state.disrupt(occupant);
        state.move_to(occupant, facet, Point { x, y, z: was.z });
    }

    Ok(to)
}

/// The two questions a course has to answer: it is all sea, and no *other* ship
/// is in it.
///
/// Not [`check_berth`]: that one refuses any boat at all in the tile, which for
/// a move is the ship itself in the berth it is leaving. The difference is one
/// comparison and it is the whole of why a ship can sail forward.
fn check_course(state: &WorldState, facet: Facet, boat: EntityId, berth: &[Berth]) -> Result<(), Refusal> {
    let facet_state = state.facet_state(facet);
    let Some(terrain) = facet_state.terrain.as_deref() else {
        return Err(Refusal::NoSuchMulti);
    };
    for &((x, y), _) in berth {
        if !terrain.land_is_water(Tile::new(x, y)) {
            return Err(Refusal::NotOnWater);
        }
        if facet_state.boats.at(x, y).iter().any(|plank| plank.boat != boat) {
            return Err(Refusal::Occupied);
        }
    }
    Ok(())
}

/// Everyone standing on the deck right now, and the tile each is standing on.
///
/// A mobile is aboard when it is on a tile the ship covers **and its feet are on
/// a plank** — the second half matters, because a swimmer beside the hull and a
/// body on a pier the ship is moored against are both on a covered tile and
/// neither is a passenger.
///
/// Mobiles only. A crate lying on the deck is cargo and stays where it is until
/// B4 gives the ship a hold; carrying it would need the item move path rather
/// than [`WorldState::move_to`], which is a mobile's.
fn aboard(state: &WorldState, boat: EntityId, facet: Facet) -> Vec<(EntityId, Point)> {
    let facet_state = state.facet_state(facet);
    let covered = facet_state.boats.covered_by(boat);
    let Some(&first) = covered.first() else {
        return Vec::new();
    };
    // One sector query for the whole ship rather than one per tile: the berth is
    // a handful of tiles and `nearby` is a block sweep, so asking it four times
    // for a sloop would walk the same statics four times.
    let centre = Point::new(first.0, first.1, 0);
    let reach = covered
        .iter()
        .map(|&(x, y)| u32::from(x.abs_diff(first.0)).max(u32::from(y.abs_diff(first.1))))
        .max()
        .unwrap_or(0);

    facet_state
        .sectors
        .nearby(centre, reach)
        .filter(|&(entity, _)| entity != boat)
        .filter(|(entity, _)| state.registry.has::<Movement>(*entity))
        .filter(|&(_, at)| facet_state.boats.deck_at(at.x, at.y, i32::from(at.z)) == Some(i32::from(at.z)))
        .collect()
}

/// Take a ship off the water: out of the boat index, off the sector grid, and
/// out of the registry.
///
/// Separate from any decay path, which is B4's: this is what `.boat` needs to be
/// undoable and what a scuttled ship will need when there is one.
pub fn sink(state: &mut WorldState, boat: EntityId) {
    let facet = state.facet_of(boat);
    state.facet_state_mut(facet).boats.cast_off(boat);
    state.facet_state_mut(facet).sectors.remove(boat);
    state.registry.despawn(boat);
}

/// The ship covering `at`, if one does.
///
/// A lookup in the tile index rather than a scan over the boats, because unlike
/// `housing::house_at` this one is asked by the step path.
#[must_use]
pub fn boat_at(state: &WorldState, at: Point, facet: Facet) -> Option<EntityId> {
    state.facet_state(facet).boats.boat_at(at.x, at.y)
}

#[cfg(test)]
mod tests;
