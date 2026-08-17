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
//! # What this phase is not
//!
//! It does not move. B1 is a ship at a mooring you can walk the deck of; the
//! step, the manifest of who moves with it, and the wire for a hull that
//! changes tiles are B2, and none of them needs a decision taken here.

use openshard_entities::EntityId;
use openshard_movement::Tile;
use openshard_protocol::serial::Serial;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{Facet, Point};
use openshard_state::WorldState;
use openshard_state::boat::Plank;
use openshard_state::components::{Boat, Drawn, Position};

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
/// A component that blocks by its tiledata is hull; everything else is deck. The
/// split is the whole of what [`Boats`](openshard_state::Boats) needs, and it is
/// made once here rather than per step.
///
/// Undrawn components are skipped, the way a house's footprint skips them: the
/// signature tile every multi opens with is not part of the ship.
fn planks(
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
    let berth = match planks(state, entity, at, facet, multi) {
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
