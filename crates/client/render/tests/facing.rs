//! What a real install's walls say about which edge they stand on.
//!
//! [`facing::face_of`](openshard_client_render::facing::face_of) is unit-tested
//! on silhouettes drawn by hand, which says the arithmetic is right and nothing
//! about whether the client's own art matches the shapes it was drawn against.
//! This is the other half: every `WALL` graphic the install ships, offered to the
//! detector, with a count of how many it could read.
//!
//! **The count is the point.** A detector with no coverage number is a green
//! light for having checked nothing — it would pass this file just as quietly by
//! refusing every graphic there is. So both tests assert on a *share decided*,
//! and both print the breakdown, because a run of one face and none of the
//! others would be a detector reading its own bias back.
//!
//! Two shares, and the second is the one that decides whether the picture
//! changes: a share of the *graphic table* counts a wall nobody ever built with
//! the same as the one every house in Britain is made of, and the table is mostly
//! the former. So [`britain`] weights by what actually stands on the map.
//!
//! Ignored and gated on `OPENSHARD_CLIENT`: no client files live in this
//! repository, ever.
//!
//! ```sh
//! OPENSHARD_CLIENT=… cargo test -p openshard-client-render --test facing \
//!     -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;

use openshard_client_render::facing::{self, Face};
use openshard_protocol::wire::Graphic;
use openshard_uofiles::art::Art;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::{TileData, TileFlags};

/// How much of the install's wall *art* this has to be able to read.
///
/// Measured, not chosen: the sweep produced 37.3% the day it was written. The
/// floor is under it by a margin, because the number this catches is a gate
/// tightened until the feature stops applying — not a version of the art with a
/// few graphics more.
///
/// It is low and correctly so. `WALL` is a broad flag: it covers doors,
/// fireplaces, arches, posts, corner pieces and whole multi-tile buildings
/// shipped as one graphic, and none of those is a wall standing on one edge of
/// one tile. See [`britain`] for the share that decides how the frame looks.
const MUST_DECIDE_ART: f64 = 0.30;

/// And how much of what a city is actually built out of.
///
/// Measured at 75.7% over the tiles below. This is the number the step is for: it
/// says that a wall the player is looking at is a wall this can shade along the
/// axis it runs on.
const MUST_DECIDE_MAP: f64 = 0.70;

/// The tiles the map sweep reads: Britain, from the bank down to the docks.
/// Wide enough to hold several districts, so the answer is not one architect's.
const BRITAIN: (std::ops::Range<u16>, std::ops::Range<u16>) = (1350..1500, 1600..1750);

#[test]
#[ignore]
fn every_wall_graphic_the_client_ships_is_offered_to_the_detector() {
    let Some(dir) = client() else { return };
    let art = Art::open(&dir).expect("the client's art");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");

    // The graphics this was designed against, by name and by verdict. A share is
    // a summary and a summary cannot be wrong in a way anybody can point at: if
    // the detector starts calling `0x0100` a south face it will still read the
    // same 37% of the table, and only this says so. Every one of these was read
    // off the silhouette by hand before the code was written — see the module
    // header of `facing.rs`.
    for (id, want) in [
        (0x0100u16, Some(Face::East)), // marble wall, running along +y
        (0x0007, Some(Face::South)),   // the same shape on the other axis
        (0x0063, Some(Face::South)),   // a low garden wall, its top surface drawn
        (0x0104, None),                // a corner: two faces at once
        (0x0006, None),                // and another
        (0x0009, None),                // a post: no edge at all
        (0x0082, None),                // a pillar filling its whole tile
    ] {
        let image = art.static_art(Graphic(id)).expect("read").expect("art");
        assert_eq!(facing::face_of(&image), want, "{id:#06X}");
    }

    let mut walls = 0usize;
    let mut faces: BTreeMap<String, usize> = BTreeMap::new();
    let mut undecided: Vec<u16> = Vec::new();
    for id in 0..=u16::MAX {
        if !is_wall(&tiledata, id) {
            continue;
        }
        let Ok(Some(image)) = art.static_art(Graphic(id)) else {
            continue;
        };
        walls += 1;
        match facing::face_of(&image) {
            Some(face) => *faces.entry(format!("{face:?}")).or_default() += 1,
            None => undecided.push(id),
        }
    }

    assert!(walls > 100, "an install with {walls} wall graphics is not one");
    let decided = walls - undecided.len();
    let share = decided as f64 / walls as f64;
    println!("wall graphics with art: {walls}");
    println!("decided:                {decided}  ({:.1}%)", share * 100.0);
    for (face, count) in &faces {
        println!("  {face:<6} {count}");
    }
    println!(
        "undecided, the first few: {:04X?}",
        &undecided[..12.min(undecided.len())]
    );
    assert!(
        share >= MUST_DECIDE_ART,
        "the detector read {:.1}% of this install's wall art, under the {:.0}% floor — a gate has \
         been tightened until the feature stops applying",
        share * 100.0,
        MUST_DECIDE_ART * 100.0,
    );

    // **The art only ever draws two of the four.** Every wall graphic in the
    // install stands on its tile's `y1` or `x1` edge — the two an isometric
    // camera looking from the north-west can see the face of. A wall on the `y0`
    // or `x0` edge would be a surface turned away from the viewer, and there is
    // no picture of one to ship.
    //
    // So this is not "all four are represented", which would be the natural
    // assertion and is false. It is the pair that exists, plus the fact that the
    // other two are vanishingly rare rather than absent: a handful of lattices
    // and gates read as west, and the detector is allowed to say so.
    let count = |face: Face| faces.get(&format!("{face:?}")).copied().unwrap_or(0);
    assert!(count(Face::South) > 100, "no south faces: {faces:?}");
    assert!(count(Face::East) > 100, "no east faces: {faces:?}");
    assert!(
        count(Face::North) + count(Face::West) < decided / 20,
        "north and west are supposed to be the rarity the art makes them: {faces:?}",
    );
}

/// The share of the walls *standing in a city* that this can read.
///
/// A different question from the one above and the one that matters: the graphic
/// table is mostly things nobody built with, and a town is built out of a few
/// dozen graphics repeated thousands of times. A detector that read every rare
/// panel and none of the four graphics Britain's houses are made of would pass
/// the art sweep and change nothing on screen.
#[test]
#[ignore]
fn britain_s_walls_are_read_where_they_stand() {
    let Some(dir) = client() else { return };
    let art = Art::open(&dir).expect("the client's art");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    let map = Map::load_facet(&dir, 0).expect("Felucca");

    // Read once per graphic, the way the atlas does: the answer is a property of
    // the picture, and a city repeats its pictures thousands of times.
    let mut known: BTreeMap<u16, Option<Face>> = BTreeMap::new();
    let mut standing = 0usize;
    let mut decided = 0usize;
    let mut faces: BTreeMap<String, usize> = BTreeMap::new();
    let mut worst: BTreeMap<u16, usize> = BTreeMap::new();
    let (xs, ys) = BRITAIN;
    for y in ys {
        for x in xs.clone() {
            for item in map.statics_at(x, y) {
                if !is_wall(&tiledata, item.tile) {
                    continue;
                }
                standing += 1;
                let face = *known.entry(item.tile).or_insert_with(|| {
                    art.static_art(Graphic(item.tile))
                        .ok()
                        .flatten()
                        .and_then(|image| facing::face_of(&image))
                });
                match face {
                    Some(face) => {
                        decided += 1;
                        *faces.entry(format!("{face:?}")).or_default() += 1;
                    }
                    None => *worst.entry(item.tile).or_default() += 1,
                }
            }
        }
    }

    assert!(
        standing > 500,
        "a Britain with {standing} wall statics is not one"
    );
    let share = decided as f64 / standing as f64;
    println!("wall statics standing: {standing}  ({} graphics)", known.len());
    println!("decided:               {decided}  ({:.1}%)", share * 100.0);
    for (face, count) in &faces {
        println!("  {face:<6} {count}");
    }
    // Which unread graphic costs the most pixels, so that the next session
    // improving the detector knows where to point it rather than guessing.
    let mut worst: Vec<(u16, usize)> = worst.into_iter().collect();
    worst.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (id, count) in worst.iter().take(15) {
        let size = art
            .static_art(Graphic(*id))
            .ok()
            .flatten()
            .map(|i| format!("{}x{}", i.width(), i.height()))
            .unwrap_or_default();
        println!(
            "  unread {id:#06X} x{count:<5} {size:<9} {:?}",
            tiledata.static_tile(*id).flags
        );
    }

    assert!(
        share >= MUST_DECIDE_MAP,
        "only {:.1}% of the walls standing in Britain were read, under the {:.0}% floor",
        share * 100.0,
        MUST_DECIDE_MAP * 100.0,
    );
}

fn client() -> Option<std::path::PathBuf> {
    std::env::var_os("OPENSHARD_CLIENT").map(std::path::PathBuf::from)
}

/// A static the detector is meant to be offered: flagged `WALL`, and not a floor.
///
/// The floor test is not redundant with the flag — a few graphics carry both —
/// and it is the same order the renderer decides in: `Stance::of` reads the
/// client's own `FLOOR` bit before it looks at what the art said, so a graphic
/// that never reaches the detector must not be counted against it here either.
fn is_wall(tiledata: &TileData, id: u16) -> bool {
    let tile = tiledata.static_tile(id);
    tile.flags.has(TileFlags::WALL) && !tile.flags.is_background()
}
