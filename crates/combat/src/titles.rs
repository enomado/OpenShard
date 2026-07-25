//! Fame, karma, and the title they earn — ServUO's `Scripts/Misc/Titles.cs`.
//!
//! # Why this lives in `combat`
//!
//! Standing is already this crate's business: the murder count, the criminal flag and
//! the notoriety a health bar is drawn in all live here, and all three are decided by
//! who hit whom. Fame and karma are the same question asked over a longer span — what
//! a character *is*, rather than what they just did — and they are awarded from
//! `MobileDied`, which this crate emits. A crate of their own would depend on combat
//! for its only input.
//!
//! # The curve is the interesting part
//!
//! Awarding is not addition. ServUO subtracts `current / 100` from every offset, in
//! *both* directions, so the same kill is worth less to a famous character than to an
//! unknown one and a fall from grace slows as it deepens. That is what stops fame
//! being a counter of monsters killed. It is ported exactly, including the detail that
//! the reduction applies to a loss as well as a gain — read quickly it looks like a
//! bug, and "fixing" it would make infamy accelerate.

use openshard_entities::EntityId;
use openshard_state::components::{Fame, Karma};
use openshard_state::WorldState;

/// ServUO's `Titles.MinFame`/`MaxFame`.
pub const MIN_FAME: i32 = 0;
/// The most fame a character may hold.
pub const MAX_FAME: i32 = 32_000;
/// ServUO's `Titles.MinKarma`/`MaxKarma`.
pub const MIN_KARMA: i32 = -32_000;
/// The most karma a character may hold.
pub const MAX_KARMA: i32 = 32_000;

/// How much of `offset` actually lands, given what is already held.
///
/// ServUO's `AwardFame`/`AwardKarma` share this shape: the offset is reduced by
/// `current / 100` **whichever way it points**, then clamped into range. So a famous
/// character gains little and loses little, and an unknown one swings freely.
fn diminish(current: i32, offset: i32, min: i32, max: i32) -> i32 {
    if offset > 0 {
        if current >= max {
            return 0;
        }
        // Note the sign: ServUO subtracts in both branches. A gain shrinks toward zero.
        (offset - current / 100).max(0).min(max - current)
    } else if offset < 0 {
        if current <= min {
            return 0;
        }
        // And a loss also has `current / 100` subtracted, which for positive `current`
        // makes the loss *bigger* and for negative `current` smaller — so infamy slows
        // as it deepens. Clamped at zero the other way.
        (offset - current / 100).min(0).max(min - current)
    } else {
        0
    }
}

/// Award (or take) fame. Returns what actually landed, for the message.
pub fn award_fame(state: &mut WorldState, mobile: EntityId, offset: i32) -> i32 {
    let current = state.registry.get::<Fame>(mobile).map_or(0, |f| f.0);
    let landed = diminish(current, offset, MIN_FAME, MAX_FAME);
    if landed != 0 {
        state.registry.insert(mobile, Fame(current + landed));
    }
    landed
}

/// Award (or take) karma. Returns what actually landed.
pub fn award_karma(state: &mut WorldState, mobile: EntityId, offset: i32) -> i32 {
    let current = state.registry.get::<Karma>(mobile).map_or(0, |k| k.0);
    let landed = diminish(current, offset, MIN_KARMA, MAX_KARMA);
    if landed != 0 {
        state.registry.insert(mobile, Karma(current + landed));
    }
    landed
}

/// What ServUO tells a player about a change in standing — `1019051..1019066`, as
/// plain text. `None` when nothing landed, so nothing is said.
#[must_use]
pub fn award_message(landed: i32, karma: bool) -> Option<&'static str> {
    let (kind, band) = (if karma { "karma" } else { "fame" }, landed);
    Some(match (kind, band) {
        (_, 0) => return None,
        ("fame", n) if n > 40 => "You have gained a lot of fame.",
        ("fame", n) if n > 20 => "You have gained a good amount of fame.",
        ("fame", n) if n > 10 => "You have gained some fame.",
        ("fame", n) if n > 0 => "You have gained a little fame.",
        ("fame", n) if n < -40 => "You have lost a lot of fame.",
        ("fame", n) if n < -20 => "You have lost a good amount of fame.",
        ("fame", n) if n < -10 => "You have lost some fame.",
        ("fame", _) => "You have lost a little fame.",
        (_, n) if n > 40 => "You have gained a lot of karma.",
        (_, n) if n > 20 => "You have gained a good amount of karma.",
        (_, n) if n > 10 => "You have gained some karma.",
        (_, n) if n > 0 => "You have gained a little karma.",
        (_, n) if n < -40 => "You have lost a lot of karma.",
        (_, n) if n < -20 => "You have lost a good amount of karma.",
        (_, n) if n < -10 => "You have lost some karma.",
        (_, _) => "You have lost a little karma.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_kill_is_worth_less_to_a_famous_character() {
        // The whole point of the curve: without it fame is a counter of monsters killed.
        assert_eq!(diminish(0, 100, MIN_FAME, MAX_FAME), 100);
        assert_eq!(diminish(5000, 100, MIN_FAME, MAX_FAME), 50);
        assert_eq!(diminish(10000, 100, MIN_FAME, MAX_FAME), 0);
    }

    #[test]
    fn a_fall_from_grace_slows_as_it_deepens() {
        // ServUO subtracts `current / 100` from a *loss* too. Read quickly that looks
        // like a bug — it makes a loss bigger while karma is positive — and the reason
        // it is not is the other half: once karma is negative it makes each further
        // loss smaller, so infamy decelerates. Fixing the "bug" would make it run away.
        assert_eq!(diminish(0, -100, MIN_KARMA, MAX_KARMA), -100);
        assert_eq!(diminish(5000, -100, MIN_KARMA, MAX_KARMA), -150);
        assert_eq!(diminish(-5000, -100, MIN_KARMA, MAX_KARMA), -50);
        assert_eq!(diminish(-10000, -100, MIN_KARMA, MAX_KARMA), 0);
    }

    #[test]
    fn nothing_lands_past_the_bounds() {
        assert_eq!(diminish(MAX_FAME, 500, MIN_FAME, MAX_FAME), 0);
        assert_eq!(diminish(MIN_FAME, -500, MIN_FAME, MAX_FAME), 0);
        assert_eq!(diminish(MIN_KARMA, -500, MIN_KARMA, MAX_KARMA), 0);
        // And a partial award never overshoots the ceiling.
        assert_eq!(diminish(MAX_FAME - 10, 5000, MIN_FAME, MAX_FAME), 10);
    }

    #[test]
    fn a_message_is_only_sent_when_something_landed() {
        assert_eq!(award_message(0, false), None);
        assert_eq!(
            award_message(50, false),
            Some("You have gained a lot of fame.")
        );
        assert_eq!(
            award_message(-5, true),
            Some("You have lost a little karma.")
        );
    }
}
