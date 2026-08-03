//! Which door graphics are a leaf that has swung open.
//!
//! # Why this cannot be read out of the client
//!
//! An open door must not occlude: decision 3 of `docs/lighting.md` makes an
//! occluder a whole tile, so a door left in the grid lays a tile of wall across
//! its own doorway — a band of shadow with nothing visible casting it, and the
//! more obvious for the leaf beside it being brightly lit. Decision 11 said the
//! tile leaves the grid on its own, because an open leaf's graphic would not be
//! `NO_SHOOT`. That is false, and the three places the fact might have been were
//! each measured and each came back empty:
//!
//! - **`tiledata.mul` does not distinguish the two.** Over the 104 open/shut
//!   pairs below, the flags are *identical* in every one: the wooden and metal
//!   doors are `NO_SHOOT` both ways, the gates are clear both ways, the barred
//!   doors are `WINDOW` both ways. 55 of the open leaves stop everything.
//! - **The layout has no parity to read.** The `DOOR` flag comes in runs of 1,
//!   2, 4, 6, 7, 8, 11, 12, 13, 16, 20, 24, 29, 32, 80 and 98 graphics, so
//!   "odd offset within a run" names nothing.
//! - **The art does not say either.** "An open leaf's picture is wider than a
//!   tile" holds for 46 of the 104 pairs and no better, because a door swung to
//!   four of the eight facings is still 44 across.
//!
//! So the fact belongs to whoever changes the graphic, which is the shard, and
//! the table is the reference emulator's.
//!
//! # The port
//!
//! ServUO, `Scripts/Items/Functional/Doors.cs`: every `BaseDoor` subclass is
//! `base(closed + 2 * facing, closed + 1 + 2 * facing, …)` over the eight
//! `DoorFacing` values. A family is therefore sixteen consecutive graphics with
//! the shut leaves on the even offsets and the open ones on the odd — and the
//! thirteen family bases are `data/doors.json`.
//!
//! **A graphic this does not know keeps today's behaviour exactly**, which is the
//! safe direction: a shard's own door goes on occluding as it always did, rather
//! than a wrong guess opening a room to the street. That is the same refusal
//! [`crate::facing`] makes and for the same reason.
//!
//! # And why the server's copy is not this one
//!
//! `server/world/src/doorgen.rs` ports the same `+ 2 * facing` rule for the doors
//! a shard *generates*, and knows one family. This crate cannot reach it and must
//! not: `client/*` and `server/*` never depend on each other, and what both ends
//! agree on lives in `common/protocol`, which a table of art indices is not. Two
//! copies of one arithmetic rule is the price of that boundary; if a third reader
//! ever appears, the table is what moves down, not the boundary that moves.

use openshard_protocol::wire::Graphic;

include!(concat!(env!("OUT_DIR"), "/doors.rs"));

/// How many graphics one family occupies: eight facings, shut and open.
///
/// The same 16 `build.rs` checks the families do not overlap by. Stated in both
/// because the check has to run before this file compiles.
const FAMILY: u16 = 16;

/// Whether this graphic is a door leaf that has swung open.
///
/// `false` for a shut door, for anything that is not a door, and for a door this
/// does not know — see the module header for why the last of those is the answer
/// and not a failure.
pub fn is_open(graphic: Graphic) -> bool {
    let Some(offset) = offset_in_family(graphic) else {
        return false;
    };
    offset % 2 == 1
}

/// How far into its family a graphic is, or `None` if it is in none of them.
///
/// A binary search over thirteen sorted bases: `partition_point` gives the last
/// family that starts at or below the graphic, and the graphic is in it when it
/// has not run past its sixteen.
fn offset_in_family(graphic: Graphic) -> Option<u16> {
    let at = FAMILIES.partition_point(|(base, _)| *base <= graphic.0);
    let (base, _) = *FAMILIES.get(at.checked_sub(1)?)?;
    let offset = graphic.0 - base;
    (offset < FAMILY).then_some(offset)
}

/// Which family a graphic belongs to, by ServUO's name for it, or `None`.
///
/// Only a failing test prints this — a verdict about a door is a lot easier to
/// argue with when it says `MetalDoor facing 3, open` than when it says `0x067C`.
pub fn family(graphic: Graphic) -> Option<(&'static str, u16, bool)> {
    let offset = offset_in_family(graphic)?;
    let at = FAMILIES.partition_point(|(base, _)| *base <= graphic.0) - 1;
    Some((FAMILIES[at].1, offset / 2, offset % 2 == 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule, on the family the port was read off: `MetalDoor` at `0x0675`.
    ///
    /// Written out rather than looped, because the thing that fails here is an
    /// off-by-one in the parity and a loop over `is_open` would be the same
    /// off-by-one twice.
    #[test]
    fn a_metal_door_is_shut_on_the_even_graphic_and_open_on_the_odd() {
        assert!(!is_open(Graphic(0x0675)), "facing 0, shut");
        assert!(is_open(Graphic(0x0676)), "facing 0, open");
        assert!(!is_open(Graphic(0x0677)), "facing 1, shut");
        assert!(
            is_open(Graphic(0x0684)),
            "facing 7, open — the last of the family"
        );
        // Facing 3 is `base + 2 * 3`, so `0x067B` shut and `0x067C` open — and
        // getting that pair the wrong way round on the first try is why the
        // arithmetic is spelled out in an assertion rather than in a loop.
        assert_eq!(family(Graphic(0x067B)), Some(("MetalDoor", 3, false)));
        assert_eq!(family(Graphic(0x067C)), Some(("MetalDoor", 3, true)));
    }

    /// Every family is sixteen graphics and the sixteenth is the last.
    ///
    /// The bound is what keeps a lookup from claiming the graphics *after* a
    /// family — which for `MetalDoor` is `BarredMetalDoor`'s own base, so the
    /// error would be invisible there and wrong everywhere the next family is
    /// not a door at all.
    #[test]
    fn a_family_ends_after_sixteen_graphics() {
        for (base, name) in FAMILIES {
            assert_eq!(offset_in_family(Graphic(base)), Some(0), "{name}");
            assert_eq!(offset_in_family(Graphic(base + 15)), Some(15), "{name}");
            let after = Graphic(base + 16);
            // Either nothing, or the *next* family's own first graphic — never
            // this one's seventeenth.
            assert!(
                matches!(offset_in_family(after), None | Some(0)),
                "{name} claims a seventeenth graphic",
            );
        }
    }

    /// Nothing outside a family is a door, including the graphic just below one.
    #[test]
    fn a_graphic_in_no_family_is_not_an_open_door() {
        assert!(!is_open(Graphic(0)), "graphic zero is below every base");
        assert!(!is_open(Graphic(0x0674)), "one below MetalDoor");
        assert!(!is_open(Graphic(0x0100)), "a marble wall");
        assert!(!is_open(Graphic(u16::MAX)), "above every family");
        assert_eq!(family(Graphic(0x0674)), None);
    }

    /// The bases are sorted and disjoint, which the bisect above assumes and
    /// `build.rs` enforces — asserted here too, because the two are different
    /// programs and only this one runs in CI.
    #[test]
    fn the_families_are_sorted_and_do_not_overlap() {
        for pair in FAMILIES.windows(2) {
            assert!(
                pair[0].0 + FAMILY <= pair[1].0,
                "{} at {:#06X} runs into {} at {:#06X}",
                pair[0].1,
                pair[0].0,
                pair[1].1,
                pair[1].0,
            );
        }
    }
}
