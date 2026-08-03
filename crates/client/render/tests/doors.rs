//! The door table, against the client that has to agree with it.
//!
//! `data/doors.json` is thirteen numbers ported out of ServUO, and a ported table
//! with no oracle is thirteen chances to have mistyped a hex digit into something
//! that still compiles. The client is the oracle available: every graphic a
//! family claims has to be a door in `tiledata.mul`, which a base off by one or a
//! family length off by eight would break immediately.
//!
//! The second test is the module's own premise, and it is worth having as a test
//! rather than as a paragraph: `crate::doors` exists **only** because the client
//! does not distinguish an open leaf from a shut one. The day a client ships that
//! does, this fails, and the right answer is to delete the table rather than to
//! update it.
//!
//! Ignored and gated on `OPENSHARD_CLIENT`: no client files live in this
//! repository, ever.

use openshard_client_render::{doors, occlusion};
use openshard_protocol::wire::Graphic;
use openshard_uofiles::tiledata::{TileData, TileFlags};

/// How many graphics one family occupies: eight facings, shut and open.
const FAMILY: u16 = 16;

fn tiledata() -> Option<TileData> {
    let dir = std::env::var_os("OPENSHARD_CLIENT").map(std::path::PathBuf::from)?;
    Some(TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul"))
}

/// Every graphic the table claims is a door in the client's own table.
///
/// The check a mistyped base fails: `0x0675` is `MetalDoor` and `0x0765` is not
/// a door at all, and nothing else here would notice the difference.
#[test]
#[ignore]
fn every_graphic_the_table_claims_is_a_door_in_the_client_s() {
    let Some(tiledata) = tiledata() else { return };
    let mut checked = 0usize;
    let mut missing = Vec::new();
    for base in bases() {
        for offset in 0..FAMILY {
            let id = base + offset;
            checked += 1;
            if !tiledata.static_tile(id).flags.has(TileFlags::DOOR) {
                missing.push(id);
            }
        }
    }
    // A count, not just a verdict: the families are recovered by walking, and a
    // walk that found nothing would pass every assertion about what it found.
    assert_eq!(checked, 13 * usize::from(FAMILY), "the table lost a family");
    println!(
        "{checked} graphics across 13 families; {} not flagged DOOR: {missing:04X?}",
        missing.len()
    );
    // Not all of them, and that is the client's own doing rather than a mistyped
    // base: `tiledata.mul` leaves a handful of leaves inside otherwise solid
    // families unflagged. What a wrong base looks like is *most* of a family
    // missing, since a door family sits in a run of doors and nothing else does.
    assert!(
        missing.len() * 8 < checked,
        "an eighth of the claimed graphics are not doors — a base is mistyped",
    );
}

/// **The premise.** An open leaf and its shut twin carry the same flags, so
/// nothing but this table can tell them apart.
///
/// If it ever fails, `crate::doors` should be deleted and `occlusion::opacity`
/// should go back to reading the flags — which is a better world, and the reason
/// the assertion is written as the *presence* of the problem rather than as a
/// property to preserve.
#[test]
#[ignore]
fn the_client_still_cannot_tell_an_open_door_from_a_shut_one() {
    let Some(tiledata) = tiledata() else { return };
    let mut pairs = 0;
    let mut blind = 0;
    let mut opaque_open = 0;
    let mut differ: Vec<String> = Vec::new();
    for base in bases() {
        for facing in 0..8u16 {
            let (shut, open) = (base + 2 * facing, base + 2 * facing + 1);
            assert!(!doors::is_open(Graphic(shut)), "{shut:#06X} read as open");
            assert!(doors::is_open(Graphic(open)), "{open:#06X} read as shut");
            pairs += 1;
            // The two bits `occlusion::opacity` would decide on if this table did
            // not exist. Not every bit: an open leaf differs from its twin in
            // `0x0400_0000`, which is a naming flag and stops nothing. Asserting
            // on the whole word would make this fail for a reason that has no
            // bearing on light, which is the sort of test that gets deleted.
            let stopping = |id: u16| {
                let f = tiledata.static_tile(id).flags;
                (f.has(TileFlags::NO_SHOOT), f.has(TileFlags::WINDOW))
            };
            if stopping(shut) == stopping(open) {
                blind += 1;
            } else {
                differ.push(format!(
                    "{shut:#06X}/{open:#06X} {:?} vs {:?}",
                    stopping(shut),
                    stopping(open)
                ));
            }
            // What the band on screen was: the leaf swung out of the way and the
            // grid still holding a whole tile of wall.
            if occlusion::opacity(Graphic(open), tiledata.static_tile(open)) != occlusion::CLEAR {
                opaque_open += 1;
            }
        }
    }
    println!("{pairs} open/shut pairs, {blind} whose stopping flags are identical");
    println!("  the exceptions: {differ:?}");
    // One pair, and it is named rather than tolerated by a percentage: the last
    // `MetalDoor` facing has a `WINDOW` open leaf against a `NO_SHOOT` shut one,
    // which is a single graphic's worth of data slip in `tiledata.mul` and not a
    // rule anybody could read a door's state off. If this ever grows past a
    // couple, the client has started distinguishing them and this table should
    // go.
    assert!(
        pairs - blind <= 2,
        "{} pairs are told apart by the flags that stop light — the client may have started \
         distinguishing them, in which case delete this table and read the flags: {differ:?}",
        pairs - blind,
    );
    assert_eq!(opaque_open, 0, "an open leaf still stops light");
}

/// The thirteen bases, recovered rather than restated here.
///
/// Restating them would make the walk circular in the one way that matters —
/// a typo copied into both places. A base is the graphic `doors` calls facing 0,
/// shut, which is a property of the table and needs no second list. Not "the
/// first door after a gap": the eight wooden and metal families are *adjacent*,
/// `0x0675 + 16` being `BarredMetalDoor`'s own base, and looking for gaps found
/// six of the thirteen.
fn bases() -> Vec<u16> {
    let mut found = Vec::new();
    for id in 0..=u16::MAX {
        if let Some((_, 0, false)) = doors::family(Graphic(id)) {
            found.push(id);
        }
    }
    assert_eq!(
        found.len(),
        13,
        "the table has {} families, not thirteen",
        found.len()
    );
    found
}
