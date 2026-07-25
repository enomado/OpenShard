//! The backpack: finding it, putting things in it, taking things out.
//!
//! Every "hand this to a player" rule needs the same two steps — locate the
//! container on the backpack layer, then merge or place into it — and every
//! "collect N of these" rule needs the same all-or-nothing draw against it. Both
//! were written inline where they were first wanted, each with its own local copy
//! of the layer number; the quest turn-in would have made a third. One copy is a
//! constant, two is a coincidence, and three is how the reward path and the
//! turn-in path start disagreeing about what a backpack is.

use super::*;

/// The paperdoll layer a backpack is worn on. ServUO's `Layer.Backpack`.
pub const BACKPACK_LAYER: u8 = 0x15;

/// The container a mobile wears as its backpack, if it has one.
///
/// A mobile without one is not an error: a creature has no pack, and a reward or
/// a turn-in aimed at it simply does nothing rather than dropping loot on the
/// floor of wherever it happened to be standing.
#[must_use]
pub fn backpack_of(state: &WorldState, mobile: Serial) -> Option<Serial> {
    state
        .registry
        .query::<Equipped>()
        .find(|(item, equipped)| {
            equipped.mobile == mobile
                && equipped.layer == BACKPACK_LAYER
                && state.registry.has::<Container>(*item)
        })
        .and_then(|(item, _)| state.registry.serial_of(item))
}

/// Put an item into a mobile's backpack: merged onto a like pile when
/// `stackable` (gold, reagents), else placed as a discrete piece.
///
/// Returns whether it landed. `false` means the mobile wears no backpack, and the
/// caller decides what that means — nothing is spilled on the ground here, because
/// a reward that quietly becomes litter at the giver's feet is worse than one that
/// visibly did not arrive.
pub fn give_to_backpack(
    state: &mut WorldState,
    mobile: Serial,
    graphic: u16,
    hue: u16,
    amount: u16,
    stackable: bool,
) -> bool {
    let Some(backpack) = backpack_of(state, mobile) else {
        return false;
    };
    if stackable {
        crate::give(state, backpack, graphic, hue, u32::from(amount));
    } else {
        crate::place_one(state, backpack, graphic, hue, amount);
    }
    true
}

/// Take `amount` of a graphic out of a mobile's backpack — **all or nothing**.
///
/// Returns what was taken: `amount` when the player had at least that many across
/// however many piles, otherwise `0` with nothing removed. The partial take is
/// refused on purpose: a hand-in that swallows four of the five items asked for
/// and then reports failure has destroyed four items for nothing, and the player
/// has no way to see where they went.
///
/// Piles are drawn down oldest first, which is only the registry's order — no
/// rule depends on which identical pile is emptied.
pub fn take_from_backpack(
    state: &mut WorldState,
    mobile: Serial,
    graphic: u16,
    amount: u16,
) -> u16 {
    let Some(backpack) = backpack_of(state, mobile) else {
        return 0;
    };
    let piles: Vec<(Serial, u16)> = state
        .registry
        .query::<Contained>()
        .filter(|(item, held)| {
            held.container == backpack
                && state
                    .registry
                    .get::<Graphic>(*item)
                    .is_some_and(|g| g.id == graphic)
        })
        .filter_map(|(item, _)| {
            state
                .registry
                .serial_of(item)
                .map(|serial| (serial, crate::amount_of(state, item)))
        })
        .collect();
    let total: u32 = piles.iter().map(|(_, held)| u32::from(*held)).sum();
    if total < u32::from(amount) {
        return 0;
    }
    let mut remaining = amount;
    for (pile, held) in &piles {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(*held);
        crate::consume(state, *pile, take);
        remaining -= take;
    }
    amount
}

/// How many of a graphic a mobile carries in its backpack, counting every pile.
///
/// A read, not a take: a collect objective needs to know how far along it is
/// without destroying the evidence. Only the backpack itself — a bag *inside* it
/// counts for weight (see [`carried_with`](crate::carried_with)) but not here, so
/// that "in your pack" means the one place a player can see at a glance.
///
/// Walks the containment column once. Callers asking about several graphics, or
/// about several players in a pass, should build a [`Contents`](crate::Contents)
/// and use [`carried_amount_with`] instead — otherwise it is a full column scan
/// per question.
#[must_use]
pub fn carried_amount(state: &WorldState, mobile: Serial, graphic: u16) -> u32 {
    carried_amount_with(state, &crate::contents_index(state), mobile, graphic)
}

/// [`carried_amount`], against an index already built.
#[must_use]
pub fn carried_amount_with(
    state: &WorldState,
    contents: &crate::Contents,
    mobile: Serial,
    graphic: u16,
) -> u32 {
    let Some(backpack) = backpack_of(state, mobile) else {
        return 0;
    };
    contents
        .get(&backpack)
        .into_iter()
        .flatten()
        .filter(|item| {
            state
                .registry
                .get::<Graphic>(**item)
                .is_some_and(|g| g.id == graphic)
        })
        .map(|item| u32::from(crate::amount_of(state, *item)))
        .sum()
}
