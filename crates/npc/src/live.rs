//! The townsfolk beat: greeting, facing, barking and wandering.
//!
//! # The AI, and its seam
//!
//! [`live`] is the per-tick beat. It does everything it can directly on the world
//! — greet, turn to face, bark — and returns the one thing it cannot: the *steps*
//! it wants to take, because stepping is bound to the terrain and the walk
//! machinery the tick owns. That is the same decide-then-apply split the creature
//! brain uses (`ai::think_one` returns a direction, the tick calls `step`).
//!
//! # Why a random heading does not make an NPC walk
//!
//! The motion path implements turn-as-step: a step in a direction you are not
//! already facing only *turns* you (`world/tick/motion.rs`). So an idle NPC that
//! picks a fresh random heading every beat spends seven beats in eight pirouetting
//! and one actually moving, which reads exactly like standing still — and that is
//! what this did. `ai::think_one` already guards against it for creatures; the
//! fix here is the reference's own: ServUO's `BaseAI.WalkRandom(iChanceToNotMove,
//! iChanceToDir, iSteps)`, called as `WalkRandomInHome(2, 2, 1)`, which keeps the
//! current heading unless a one-in-`iChanceToDir` roll says otherwise. Most beats
//! it walks on, so it walks.
//!
//! # And a shopkeeper serving a customer stands still
//!
//! ServUO's `VendorAI.DoActionInteract` turns the vendor to face whoever it is
//! dealing with and takes no step at all. Without that the shopkeeper wanders off
//! mid-transaction, which is the other half of "the vendors feel dead": they were
//! not only silent, they were walking away.

use openshard_entities::EntityId;
use openshard_protocol::{Direction, Facing, Point};
use openshard_state::components::{Heading, Npc, Position};
use openshard_state::sectors::in_range;
use openshard_state::WorldState;

use crate::speech::{bark_line, greeting_for};

/// How long between an NPC's beats, in ticks (~2s at 20Hz). `spawn` also jitters the
/// first one across this span, so a whole facet's townsfolk do not beat in lockstep.
pub(crate) const BEAT_TICKS: u64 = 40;
/// How near a player has to come for a townsperson to greet them. ServUO's
/// `VendorAI.HandlesOnSpeech` uses the same four tiles.
pub(crate) const GREET_RANGE: u32 = 4;
/// How long a townsperson waits between greetings — long enough not to natter at
/// someone standing at the counter.
const GREET_COOLDOWN: u64 = 15 * 20;
/// How long between two of an NPC's own idle remarks. Much longer than a greeting:
/// a bark is atmosphere, and a street of shopkeepers each shouting every fifteen
/// seconds is worse than silence.
const BARK_COOLDOWN: u64 = 60 * 20;
/// The chance, in a hundred, that an idle NPC with nobody near says something to
/// itself this beat.
const BARK_CHANCE: u32 = 6;

/// One tick of townsfolk life. Returns the steps the NPCs want —
/// `(serial, direction)` — for the tick to apply through its own terrain-checked
/// `step`. Everything else is done here on the world.
///
/// `hour` is the world's hour (0–23), from the tick counter — see
/// `world/tick/ambient.rs`. It is only read when `gameplay.npc_schedule` is on.
#[must_use]
pub fn live(state: &mut WorldState, hour: u64) -> Vec<(u32, u8)> {
    let now = state.ticks;
    let due: Vec<EntityId> = state
        .registry
        .query::<Npc>()
        .filter(|(_, npc)| now >= npc.next_beat)
        .map(|(entity, _)| entity)
        .collect();

    let mut steps = Vec::new();
    for npc in due {
        // Space out the next beat first, so an early return below still paces it.
        if let Some(mut n) = state.registry.get::<Npc>(npc).copied() {
            n.next_beat = now + BEAT_TICKS;
            state.registry.insert(npc, n);
        }
        let Some(&Position(at)) = state.registry.get::<Position>(npc) else {
            continue;
        };
        let facet = state.facet_of(npc);

        // An NPC nobody could see or hear need not think. The same `lod` gate the
        // creature brains sit behind (`world/tick.rs`), and for the same reason:
        // a full Felucca is thousands of mobiles, and the ones alone in a field
        // are exactly the ones whose beat nobody can tell was skipped.
        if state.gameplay.lod && !state.any_player_near(at, state.gameplay.lod_radius, facet) {
            if let Some(mut n) = state.registry.get::<Npc>(npc).copied() {
                n.next_beat = now + BEAT_TICKS * state.gameplay.lod_idle_factor.max(1);
                state.registry.insert(npc, n);
            }
            continue;
        }

        // Someone close? Face them, greet them if it is time, and stand still this
        // beat — you do not wander off mid-hello, and you certainly do not wander
        // off mid-sale.
        if let Some((visitor, visitor_at)) = nearest_player(state, facet, at, GREET_RANGE) {
            attend(state, npc, at, visitor, visitor_at, now);
            continue;
        }

        // Nobody near: a remark to itself now and then, and a drift near home.
        bark(state, npc, now);
        if let Some(dir) = wander_step(state, npc, at, hour) {
            if let Some(serial) = state.registry.serial_of(npc) {
                steps.push((serial.raw(), dir));
            }
        }
    }
    steps
}

/// Attend to a visitor: turn to face them (ServUO's `VendorAI.DoActionInteract`)
/// and greet them if the cooldown has passed. Every trade greets, not only the
/// bankers — the greeting line itself comes from the trade's speech table.
fn attend(
    state: &mut WorldState,
    npc: EntityId,
    at: Point,
    visitor: EntityId,
    visitor_at: Point,
    now: u64,
) {
    // Turn to face them, and let watchers see the turn.
    if let Some(dir) = openshard_ai::direction_toward(at, visitor_at) {
        let facing = Facing::walking(dir);
        if state.registry.get::<Heading>(npc).map(|h| h.0) != Some(facing) {
            state.registry.insert(npc, Heading(facing));
            state.broadcast_move(npc);
        }
    }

    let Some(npc_state) = state.registry.get::<Npc>(npc).copied() else {
        return;
    };
    if now < npc_state.next_greet {
        return;
    }
    let Some(line) = greeting_for(state, npc, visitor) else {
        return;
    };
    crate::say(state, npc, &line);
    state.registry.insert(
        npc,
        Npc {
            next_greet: now + GREET_COOLDOWN,
            ..npc_state
        },
    );
}

/// An idle remark, when nobody is within greeting range. Silent unless the trade's
/// table supplies a line, so a bare shard's townsfolk do not chatter nonsense.
fn bark(state: &mut WorldState, npc: EntityId, now: u64) {
    let Some(npc_state) = state.registry.get::<Npc>(npc).copied() else {
        return;
    };
    if now < npc_state.next_greet || state.rng.below(100) >= BARK_CHANCE {
        return;
    }
    let Some(line) = bark_line(state, npc) else {
        return;
    };
    crate::say(state, npc, &line);
    state.registry.insert(
        npc,
        Npc {
            next_greet: now + BARK_COOLDOWN,
            ..npc_state
        },
    );
}

/// An idle step for an NPC: head to its post when it has strayed, else drift.
/// `None` means stand still this beat. The tile is not checked here — the tick's
/// `step` validates it against the terrain, and a step into a wall simply turns
/// the NPC.
fn wander_step(state: &mut WorldState, npc: EntityId, at: Point, hour: u64) -> Option<u8> {
    let Npc { home, wander, .. } = *state.registry.get::<Npc>(npc)?;
    if wander == 0 {
        return None;
    }
    let post = post_at_hour(state, npc, home, hour);

    // ServUO's `WalkRandomInHome`: past the home range, walk back; inside it,
    // `WalkRandom`.
    if chebyshev(at, post) > u32::from(wander) {
        return walk_home(state, npc, at, post);
    }

    // `WalkRandom(2, 2, 1)`: one chance in two of not moving at all, and one in
    // two of picking a new heading rather than continuing on the current one.
    // Reusing the heading is what makes the step translate instead of turn.
    if state.rng.below(2) == 0 {
        return None;
    }
    if state.rng.below(2) == 0 {
        return Some(state.rng.below(8) as u8);
    }
    state
        .registry
        .get::<Heading>(npc)
        .map(|h| h.0.direction.to_bits())
        .or_else(|| Some(state.rng.below(8) as u8))
}

/// A step back toward the post — pathed around the counter, not into it. A
/// townsperson is human: a shut door on the way is opened, not an obstacle (the
/// auto-close swings it shut again behind them).
fn walk_home(state: &mut WorldState, npc: EntityId, at: Point, post: Point) -> Option<u8> {
    let facet = state.facet_of(npc);
    let dir = openshard_ai::step_toward(state, facet, at, post, true)?;
    if let Some(tile) = openshard_movement::step_from(at, Direction::from_bits(dir)) {
        let door = state
            .facet_state(facet)
            .live_terrain()
            .blocker_at(tile.x, tile.y)
            .filter(|o| o.door)
            .map(|o| o.entity);
        if let Some(door) = door {
            openshard_items::open_door(state, door);
            return None;
        }
    }
    Some(dir)
}

/// Where this NPC should be at this hour.
///
/// Off by default and beyond both references — ServUO's nearest equivalent is a
/// hand-placed `WayPoint` chain, which is not tied to the clock at all. With
/// `gameplay.npc_schedule` on, a townsperson with a `night_home` walks to it
/// outside working hours and back to its post inside them. Without the setting, or
/// without a `night_home` in the pack's data, this is the post and nothing changes.
fn post_at_hour(state: &WorldState, npc: EntityId, home: Point, hour: u64) -> Point {
    if !state.gameplay.npc_schedule {
        return home;
    }
    let Some(night_home) = state.registry.get::<Npc>(npc).and_then(|_| {
        state
            .registry
            .get::<openshard_state::components::NightHome>(npc)
            .map(|h| h.0)
    }) else {
        return home;
    };
    let work = state.gameplay.npc_work_hour;
    let rest = state.gameplay.npc_home_hour;
    // A working day that does not wrap midnight is the only shape the setting
    // allows; `config` rejects the rest, so this comparison is enough.
    if hour >= u64::from(work) && hour < u64::from(rest) {
        home
    } else {
        night_home
    }
}

/// The nearest player to `at` within `range` on `facet`, and where it stands.
fn nearest_player(
    state: &WorldState,
    facet: u8,
    at: Point,
    range: u32,
) -> Option<(EntityId, Point)> {
    state
        .players
        .values()
        .filter_map(|&entity| {
            let pos = state.registry.get::<Position>(entity)?.0;
            (state.facet_of(entity) == facet && in_range(pos, at, range)).then_some((entity, pos))
        })
        .min_by_key(|(_, pos)| squared_distance(*pos, at))
}

/// Chebyshev distance — the square UO measures range in.
pub(crate) fn chebyshev(a: Point, b: Point) -> u32 {
    let dx = i32::from(a.x).abs_diff(i32::from(b.x));
    let dy = i32::from(a.y).abs_diff(i32::from(b.y));
    dx.max(dy)
}

/// Squared Euclidean distance, for picking the *nearest* of several in range.
fn squared_distance(a: Point, b: Point) -> i64 {
    let dx = i64::from(a.x) - i64::from(b.x);
    let dy = i64::from(a.y) - i64::from(b.y);
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chebyshev_is_the_square_uo_measures() {
        assert_eq!(chebyshev(Point::new(0, 0, 0), Point::new(3, 1, 0)), 3);
        assert_eq!(chebyshev(Point::new(5, 5, 0), Point::new(5, 5, 0)), 0);
    }
}
