//! What a worn suit does for its wearer: the rules over
//! [`openshard_state::armor`]'s data.
//!
//! The table itself — every armour class's rating keyed by graphic, the coverage
//! each layer lends, and which layer a blow lands on — is data and lives in
//! `state`, because `skills` reads the same ratings to answer an Arms Lore
//! question. Two numbers come out of it here.
//!
//! [`worn_armor_rating`] is the wearer's total, the `ArmorRating` a status bar shows
//! (ServUO's `PlayerMobile.ArmorRating`: each piece scaled by how much of a body it
//! covers). [`absorb_physical`] is what a swing loses to it pre-AoS — ServUO's
//! `BaseWeapon.AbsorbDamage`, which rolls a hit location, lets that piece and any
//! shield eat their share, and then takes a cut of the wearer's total. Both are
//! read-site derivations: nothing is mirrored onto the mobile, so armour coming off
//! needs no undoing.

use openshard_entities::EntityId;
use openshard_state::armor::{armor_data, hit_layer, layer_coverage, LAYER_SHIELD};
use openshard_state::components::{Armor, Equipped, Graphic};
use openshard_state::WorldState;

/// One worn piece's rating: the pack's [`Armor`] override if the item carries
/// one (an enchanted breastplate), else the core table's row for its graphic,
/// else nothing.
#[must_use]
pub fn piece_rating(state: &WorldState, item: EntityId) -> u16 {
    if let Some(&Armor { rating }) = state.registry.get::<Armor>(item) {
        return rating;
    }
    state
        .registry
        .get::<Graphic>(item)
        .and_then(|graphic| armor_data(graphic.id))
        .map_or(0, |armor| armor.rating)
}

/// The item a mobile wears on `layer`, if any.
#[must_use]
pub fn worn_on_layer(state: &WorldState, mobile: EntityId, layer: u8) -> Option<EntityId> {
    let serial = state.registry.serial_of(mobile)?;
    state
        .registry
        .query::<Equipped>()
        .find(|(_, worn)| worn.mobile == serial && worn.layer == layer)
        .map(|(entity, _)| entity)
}

/// A mobile's whole armour rating — every worn piece scaled by how much of the
/// body it covers, ServUO's `PlayerMobile.ArmorRating`.
///
/// This is the number the status bar carries (pre-AoS it is the armour rating
/// itself; from AoS the client labels the same field physical resistance). A
/// mobile in nothing rates zero, which is why every existing combat test — none
/// of which dresses anybody — is unchanged by armour landing.
#[must_use]
pub fn worn_armor_rating(state: &WorldState, mobile: EntityId) -> u16 {
    let Some(serial) = state.registry.serial_of(mobile) else {
        return 0;
    };
    let worn: Vec<(EntityId, u8)> = state
        .registry
        .query::<Equipped>()
        .filter(|(_, worn)| worn.mobile == serial)
        .map(|(entity, worn)| (entity, worn.layer))
        .collect();
    let hundredths: u32 = worn
        .into_iter()
        .map(|(item, layer)| u32::from(piece_rating(state, item)) * layer_coverage(layer))
        .sum();
    u16::try_from(hundredths / 100).unwrap_or(u16::MAX)
}

/// What a physical blow loses to the defender's armour, pre-AoS.
///
/// ServUO's `BaseWeapon.AbsorbDamage` outside AoS, in its three stages: a shield
/// eats its share first, then the piece on a rolled hit location eats its own
/// (`BaseArmor.OnHit`: half the piece's rating plus up to half again), and
/// finally the wearer's *total* rating gives up a slice sized by that same
/// location. Returns the damage that gets through.
///
/// Every roll spends the world's seeded `rng`, so a fight still replays.
pub fn absorb_physical(state: &mut WorldState, defender: EntityId, damage: u16) -> u16 {
    let total = worn_armor_rating(state, defender);
    let location = hit_layer(state.rng.below(100));
    let shield = worn_on_layer(state, defender, LAYER_SHIELD).map(|item| piece_rating(state, item));
    let piece = worn_on_layer(state, defender, location).map(|item| piece_rating(state, item));

    let mut left = u32::from(damage);
    for rating in [shield, piece].into_iter().flatten() {
        // `HalfAr + HalfAr * RandomDouble()` — half the rating always, up to half
        // again by luck. In integer terms: half, plus 0..=half.
        let half = u32::from(rating) / 2;
        let absorbed = half
            + if half == 0 {
                0
            } else {
                state.rng.below(half + 1)
            };
        left = left.saturating_sub(absorbed);
    }

    if total > 0 {
        // `from = (virtualArmor * scalar) / 2`, `to = virtualArmor * scalar`, and a
        // uniform roll between them.
        let to = u32::from(total) * layer_coverage(location) / 100;
        let from = to / 2;
        let absorbed = from
            + if to > from {
                state.rng.below(to - from + 1)
            } else {
                0
            };
        left = left.saturating_sub(absorbed);
    }
    u16::try_from(left).unwrap_or(u16::MAX)
}
