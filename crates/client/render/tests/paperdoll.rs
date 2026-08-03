//! A paperdoll against the files it is drawn out of.
//!
//! What the unit tests beside `paperdoll.rs` cannot say: they pin the *order*,
//! which is a table and needs nothing, and stop at the point where a picture has
//! to exist. Everything here is about the third index space — that an `AnimID`
//! plus 50000 is a picture the client actually ships, that the female half of
//! the file is sparse and the fallback is what makes it work, and that the list
//! a window is laid out as begins with the body and ends with the backpack.
//!
//! Ignored and gated on `OPENSHARD_CLIENT`: no client files live in this
//! repository, ever.

use openshard_client_render::gump::{GumpArt, GumpPixel};
use openshard_client_render::mobiles::EquipmentLayer;
use openshard_client_render::paperdoll::{self, FEMALE_GUMP_OFFSET, MALE_GUMP_OFFSET, Wearer};
use openshard_protocol::wire::{Graphic, Hue, Layer};
use openshard_uofiles::equipconv::EquipConv;
use openshard_uofiles::gumpart::Gumps;
use openshard_uofiles::tiledata::TileData;

/// The three files a paperdoll needs, or `None` where no client is installed.
fn client() -> Option<(Gumps, EquipConv, TileData)> {
    let dir = std::env::var_os("OPENSHARD_CLIENT").map(std::path::PathBuf::from)?;
    let gumps = Gumps::open(&dir).expect("gumpartLegacyMUL.uop");
    // The table is optional in a client install and its absence is not a
    // failure — an empty one resolves nothing, which is the ordinary case for
    // most rows anyway.
    let equip_conv = EquipConv::load(dir.join("Equipconv.def")).unwrap_or_default();
    let tiledata = TileData::load(dir.join("tiledata.mul")).expect("tiledata.mul");
    Some((gumps, equip_conv, tiledata))
}

/// A male human, and a female one — the two bodies every claim below is made
/// about.
const MALE: u16 = 0x0190;
const FEMALE: u16 = 0x0191;

fn worn(layer: Layer, graphic: u16) -> EquipmentLayer {
    EquipmentLayer {
        graphic,
        hue: Hue::NONE,
        layer,
    }
}

/// The `AnimID` of a shipped item, which is what a worn layer carries: the wire
/// graphic is `tiledata`'s key and never reaches a paperdoll.
fn anim_id(tiledata: &TileData, graphic: u16) -> u16 {
    tiledata.static_tile(graphic).anim_id
}

/// The window is the body first, then what is worn, then the backpack — and
/// every one of them is a gump the client ships.
///
/// The backpack's position in the list is the claim worth the test: it is drawn
/// *outside* the ordering, last, because nothing is ever drawn over it.
#[test]
#[ignore]
fn a_dressed_body_draws_its_gump_first_and_its_backpack_last() {
    let Some((gumps, equip_conv, tiledata)) = client() else {
        return;
    };
    // A plain shirt, a pair of pants and the backpack every character wears.
    let equipment = [
        worn(Layer::SHIRT, anim_id(&tiledata, 0x1517)),
        worn(Layer::PANTS, anim_id(&tiledata, 0x152E)),
        worn(Layer::BACKPACK, anim_id(&tiledata, 0x0E75)),
    ];
    let wearer = Wearer {
        body: MALE,
        hue: Hue::NONE,
        equipment: &equipment,
    };
    let pictures = paperdoll::window(&wearer, &equip_conv, &gumps, GumpPixel::new(100, 50));

    assert_eq!(pictures.len(), 4, "a body, two garments and a bag");
    assert_eq!(
        pictures[0].graphic,
        GumpArt::Gump(Graphic(0x000C)),
        "the male body is the first picture"
    );
    for picture in &pictures {
        assert_eq!(
            picture.at,
            GumpPixel::new(100, 50),
            "every layer is at the origin"
        );
        let GumpArt::Gump(graphic) = picture.graphic else {
            panic!("a paperdoll draws gump art and nothing else");
        };
        assert!(
            gumps.has(graphic).expect("the container reads"),
            "the client ships gump 0x{:04X}",
            graphic.0
        );
    }
    let backpack = paperdoll::gump_of(MALE, anim_id(&tiledata, 0x0E75), false, &equip_conv, &gumps);
    assert_eq!(
        pictures[3].graphic,
        GumpArt::Gump(backpack),
        "the backpack is drawn last, outside the order"
    );
}

/// A layer with no `AnimID` draws nothing — a ring, an earring, anything the
/// paperdoll has no picture of. `docs/client.md`'s "done when", in one
/// assertion.
#[test]
#[ignore]
fn a_layer_with_no_anim_id_draws_nothing() {
    let Some((gumps, equip_conv, _)) = client() else {
        return;
    };
    let equipment = [worn(Layer::RING, 0)];
    let wearer = Wearer {
        body: MALE,
        hue: Hue::NONE,
        equipment: &equipment,
    };
    let pictures = paperdoll::window(&wearer, &equip_conv, &gumps, GumpPixel::new(0, 0));
    assert_eq!(pictures.len(), 1, "the body, and nothing for the ring");
}

/// The two offsets, and the fallback between them.
///
/// The claim: a garment's female picture is `AnimID + 60000` **where the client
/// ships one**, and the male picture where it does not — which is not an edge
/// case but the ordinary state of the file. The test walks the shipped range
/// and asserts the rule held for every garment in it, and counts both outcomes
/// so that a run where the fallback never fired cannot pass silently pretending
/// it was exercised.
#[test]
#[ignore]
fn a_female_gump_falls_back_to_the_male_one_where_the_file_has_none() {
    let Some((gumps, equip_conv, tiledata)) = client() else {
        return;
    };
    let (mut own, mut fell_back, mut checked) = (0usize, 0usize, 0usize);
    for graphic in 0..u16::MAX {
        let anim = anim_id(&tiledata, graphic);
        if anim == 0 {
            continue;
        }
        // A pair the table has an opinion about is a different claim — the
        // override happens *before* the offset, so the picture is no longer
        // "this anim plus 60000" and this test would be asserting the table's
        // contents rather than the fallback. `Equipconv.def` really does carry
        // such rows for a female body (anim 0x02A8 among them, which is how
        // this line came to exist).
        if equip_conv.resolve(FEMALE, anim).is_some() {
            continue;
        }
        let male = Graphic(anim.wrapping_add(MALE_GUMP_OFFSET));
        if !gumps.has(male).expect("the container reads") {
            continue;
        }
        checked += 1;
        let female = Graphic(anim.wrapping_add(FEMALE_GUMP_OFFSET));
        let drawn = paperdoll::gump_of(FEMALE, anim, true, &equip_conv, &gumps);
        match gumps.has(female).expect("the container reads") {
            true => {
                own += 1;
                assert_eq!(drawn, female, "anim 0x{anim:04X} has a female picture of its own");
            }
            false => {
                fell_back += 1;
                assert_eq!(drawn, male, "anim 0x{anim:04X} has none, and takes the male one");
            }
        }
    }
    assert!(checked > 100, "the client ships equipment gumps: {checked}");
    assert!(own > 0, "some garments have a female picture: {own}");
    assert!(
        fell_back > 0,
        "and some do not — a run that never fell back has not tested the fallback: {fell_back}"
    );
}
