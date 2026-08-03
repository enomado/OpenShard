//! How well the [`Prism`] model fits the pictures the client actually ships.
//!
//! [`openshard_client_render::facing::prism_silhouette`] is the forward
//! direction: a shape in, the drawing the projection makes of it out. This is the
//! measurement that says whether that shape is the shape the artist drew — every
//! candidate prism is scored against a real sprite by how much of the two
//! silhouettes agree, and the best one is printed with its score.
//!
//! **It is the non-circular check the whole model rests on.** Everything else
//! about a stair — its normals, its occluder, where a pixel of it stands — is
//! derived from the prism, and a prism derived from nothing but our own
//! projection would agree with itself perfectly while describing a shape no
//! client ever drew.
//!
//! Ignored and gated on `OPENSHARD_CLIENT`, like every other test that reads an
//! install:
//!
//! ```sh
//! OPENSHARD_CLIENT=… cargo test -p openshard-client-render --test prism -- \
//!     --ignored --nocapture
//! ```
//!
//! `OPENSHARD_ART=1822,0x0736` picks the graphics; the default is the staircase
//! of `docs/lighting.md`'s backlog entry, a plain wall for contrast, and the
//! floor lid that stands over both.

use openshard_client_render::facing::{Face, Prism, prism_silhouette};
use openshard_protocol::wire::Graphic;
use openshard_uofiles::art::Art;
use openshard_uofiles::color::Color16;
use openshard_uofiles::image::Image;
use openshard_uofiles::tiledata::TileData;

use std::path::PathBuf;

/// What to fit, and why each one is in the list:
///
/// - `1822` and `1846` are the two statics a flight of stairs in Britain is made
///   of — the report this model came from.
/// - `200` is a plain wall, which is *not* a prism, and its score is what says
///   the fit means anything: a measure that likes everything measures nothing.
const DEFAULT: &[u16] = &[1822, 1846, 200];

/// The tallest prism considered, in `z`. Twenty is a wall's height and taller
/// than any climbable static in the client's files.
const MAX_HEIGHT: u8 = 20;

/// How many treads a candidate stair may have. One is a box; six is finer than
/// any tread a 44-pixel sprite can show.
const MAX_TREADS: usize = 6;

fn client_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("OPENSHARD_CLIENT")?))
}

fn wanted() -> Vec<u16> {
    let Some(text) = std::env::var_os("OPENSHARD_ART") else {
        return DEFAULT.to_vec();
    };
    text.to_string_lossy()
        .split(',')
        .map(|part| {
            let part = part.trim();
            match part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                Some(hex) => u16::from_str_radix(hex, 16).expect("a hex graphic id"),
                None => part.parse().expect("a decimal graphic id"),
            }
        })
        .collect()
}

#[test]
#[ignore = "reads a real install and prints for a person"]
fn which_prism_the_art_is_a_picture_of() {
    let Some(dir) = client_dir() else {
        return;
    };
    let art = Art::open(&dir).expect("artLegacyMUL.uop");
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");

    for id in wanted() {
        let Some(picture) = art.static_art(Graphic(id)).expect("the art reads") else {
            println!("{id} (0x{id:04X}): no picture");
            continue;
        };
        let tile = tiledata.static_tile(id);
        let (best, score) = fit(&picture);
        println!(
            "\n{id} (0x{id:04X})  {}x{}  tiledata height {}  climbable {}",
            picture.width(),
            picture.height(),
            tile.height,
            tile.flags.is_climbable(),
        );
        println!("  best fit: {best:?}");
        println!("  agreement: {:.3}", score);
        // What the height *would* be if it came from tiledata, so that the two
        // numbers can be compared without arithmetic in the reader's head. A
        // climbable static's stated height is the full one and a walker stands
        // half way up it — see `movement::scene::stair`.
        println!(
            "  tiledata says {} z, or {} climbed; the art says {} z",
            tile.height,
            tile.height / 2,
            best.top(),
        );
        check(id, &best, score);
    }
}

/// How closely a shape has to match before it is that shape.
///
/// Measured rather than chosen: the two stair statics fit at 0.977 and 0.975,
/// and a plain wall — which is not a prism at all — fits its best candidate at
/// 0.812. Nine tenths sits in the gap with room on both sides.
///
/// The gap is why a fit alone is not the gate. A wall scoring 0.81 means the
/// measure *likes* walls, so what admits a prism is `CLIMBABLE` first and the
/// score second — the same order-of-policy `Stance::of` uses for a floor.
const FITS: f32 = 0.9;

/// What the fit is expected to say about the graphics in [`DEFAULT`].
///
/// Only for those: a run with `OPENSHARD_ART` set is a person looking at
/// something, and asserting about whatever they typed would be asserting about a
/// number nobody has seen yet.
fn check(id: u16, best: &Prism, score: f32) {
    if std::env::var_os("OPENSHARD_ART").is_some() {
        return;
    }
    match id {
        // The landing: a plain box five `z` tall. Its tiledata height is ten,
        // which is the *full* height a climbable static states and twice what the
        // artist drew — so a model that took its height from the flags would be
        // twice as tall as the picture. The art is what the height comes from.
        1822 => {
            assert!(score > FITS, "the landing fits its box: {score}");
            assert_eq!(best.treads, vec![5], "a box and not a stair");
        }
        // The flight: three treads climbing west, five `z` in all. Here the
        // tiledata height *is* what was drawn, which is the other half of the
        // reason the number cannot be trusted: the same field means two things.
        1846 => {
            assert!(score > FITS, "the flight fits its stair: {score}");
            assert_eq!(best.top(), 5, "five z of climb");
            assert_eq!(best.treads.len(), 3, "three treads");
            assert_eq!(best.up, Face::West, "climbing west");
        }
        // And the wall, which is the control: no prism is a good picture of it.
        200 => assert!(score < FITS, "a wall is not a prism: {score}"),
        _ => {}
    }
}

/// The best prism for a picture, and how much of the two silhouettes agree.
///
/// The score is intersection over union of the drawn pixels: 1.0 is the same
/// shape, and it falls whether the model covers pixels the art leaves empty or
/// the other way round. Both failures matter — a model that is too small leaves
/// lit pixels with no surface, and one that is too big puts surface where the
/// artist drew air.
fn fit(picture: &Image) -> (Prism, f32) {
    let mut best = (Prism::box_of(0), 0.0f32);
    for height in 0..=MAX_HEIGHT {
        let candidate = Prism::box_of(height);
        let score = agreement(picture, &prism_silhouette(&candidate));
        if score > best.1 {
            best = (candidate, score);
        }
    }
    for up in [Face::North, Face::East, Face::South, Face::West] {
        for treads in 2..=MAX_TREADS {
            for top in 1..=MAX_HEIGHT {
                // An even climb: the treads rise in equal steps to `top`, which
                // is the stair every one of the client's own is drawn as. A
                // profile with unequal treads is a search this does not need
                // until a graphic is found that wants one.
                let profile: Vec<u8> = (1..=treads)
                    .map(|i| (u16::from(top) * i as u16 / treads as u16) as u8)
                    .collect();
                let candidate = Prism { up, treads: profile };
                let score = agreement(picture, &prism_silhouette(&candidate));
                if score > best.1 {
                    best = (candidate, score);
                }
            }
        }
    }
    best
}

/// Intersection over union of two silhouettes, lined up by the bottom row and
/// the centre column.
///
/// The bottom row is where `statics::stand_on` puts every static whatever its art
/// holds, and both pictures are 44 wide, so the alignment is not a fit parameter
/// — which is the point: a model that has to be slid into place is a model with a
/// free variable nobody stated.
fn agreement(art: &Image, model: &Image) -> f32 {
    let rows = art.height().max(model.height());
    let (mut both, mut either) = (0u32, 0u32);
    for column in 0..44u16 {
        for row in 0..rows {
            let art_pixel = drawn(art, column, row);
            let model_pixel = drawn(model, column, row);
            if art_pixel && model_pixel {
                both += 1;
            }
            if art_pixel || model_pixel {
                either += 1;
            }
        }
    }
    match either {
        0 => 0.0,
        _ => both as f32 / either as f32,
    }
}

/// Whether a picture draws anything `row` rows up from its own bottom edge, in
/// the column `column` counted from the left of a 44-wide sprite.
fn drawn(image: &Image, column: u16, row: u16) -> bool {
    if row >= image.height() {
        return false;
    }
    // Centre the sprite's columns on 44, since a graphic may be narrower.
    let offset = (44 - i32::from(image.width())) / 2;
    let x = i32::from(column) - offset;
    if x < 0 || x >= i32::from(image.width()) {
        return false;
    }
    let y = image.height() - 1 - row;
    !image
        .pixel(x as u16, y)
        .unwrap_or(Color16::TRANSPARENT)
        .is_transparent()
}
