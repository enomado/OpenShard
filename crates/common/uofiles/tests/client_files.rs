//! What the readers do on a real client install, rather than on a fixture.
//!
//! Every test here skips unless `OPENSHARD_CLIENT` points at a UO client
//! directory. No client files enter this repository — they are copyrighted, and
//! a path that is right on one machine is wrong on every other.
//!
//! # Why a synthetic fixture is not enough
//!
//! A fixture is written by the same understanding that wrote the parser, so the
//! two agree by construction. Every mistake this suite exists to catch — a facet
//! whose shape the block count cannot name, a tiledata layout guessed from a
//! size, a container whose entries are not in the order they look like they are
//! in — is a mistake a fixture reproduces faithfully. These are the assertions
//! only a shipped file can settle.
//!
//! The install these numbers were taken from is client 7.0.116.0.

use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::{LAND_TILE_COUNT, TileData, TileDataFormat};

/// The client directory, or `None` to skip.
///
/// Read at runtime rather than compile time so that setting the variable does
/// not need a rebuild.
fn client_dir() -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var_os("OPENSHARD_CLIENT")?);
    dir.join("tiledata.mul").exists().then_some(dir)
}

fn tiledata() -> Option<TileData> {
    let dir = client_dir()?;
    Some(TileData::load(dir.join("tiledata.mul")).expect("a client ships a readable tiledata.mul"))
}

/// Every facet a client ships, and what it must come out as.
const FACETS: [(u8, u32, u32, &str); 6] = [
    (0, 7168, 4096, "Felucca/Trammel (post-ML)"),
    (1, 7168, 4096, "Felucca/Trammel (post-ML)"),
    (2, 2304, 1600, "Ilshenar"),
    (3, 2560, 2048, "Malas"),
    (4, 1448, 1448, "Tokuno"),
    (5, 1280, 4096, "Ter Mur"),
];

#[test]
fn every_facet_loads_as_the_facet_it_actually_is() {
    // The regression this suite was written for. Malas and Ter Mur are both
    // 81,920 blocks, so before the facet number reached the size decision,
    // `load_facet(dir, 5)` returned a 2560x2048 map named "Malas" — 256 blocks
    // per column instead of 512, which transposes everything past the first
    // column and reports no error at all.
    //
    // Nothing inside either file can catch this: the maps are the same length,
    // the staidx files are the same length, and every block in both is a
    // well-formed 196 bytes. Only the number the caller asked for can.
    let Some(dir) = client_dir() else {
        return;
    };
    for (facet, width, height, name) in FACETS {
        let map = Map::load_facet(&dir, facet).unwrap_or_else(|e| panic!("map{facet} should load: {e}"));
        assert_eq!(
            (map.width(), map.height()),
            (width, height),
            "map{facet} came out the wrong shape"
        );
        assert_eq!(map.facet_name(), name);
        assert!(map.static_count() > 1000, "map{facet} has almost no statics");
    }
}

#[test]
fn a_facets_far_corner_is_on_the_map_and_one_past_it_is_not() {
    // The bounds follow from the shape, so this is the shape again from the
    // other side: on a facet loaded as the wrong one, the corner it claims and
    // the corner it has would disagree.
    let Some(dir) = client_dir() else {
        return;
    };
    for (facet, width, height, _) in FACETS {
        let map = Map::load_facet(&dir, facet).unwrap();
        let (far_x, far_y) = ((width - 1) as u16, (height - 1) as u16);
        assert!(
            map.land(far_x, far_y).is_some(),
            "map{facet} is short of its corner"
        );
        assert!(
            !map.contains(width as u16, far_y),
            "map{facet} is wider than it says"
        );
        assert!(
            !map.contains(far_x, height as u16),
            "map{facet} is taller than it says"
        );
    }
}

#[test]
fn the_statics_a_facet_reports_are_the_statics_its_index_describes() {
    // An independent count, from the raw `staidx`/`statics` pair rather than
    // through the loader: 12-byte index entries, 7-byte statics, and the two
    // sentinels the loader treats as "nothing here". If the loader ever dropped
    // a block — a wrong block count, an off-by-one in the index walk — the two
    // numbers part company. It is a re-derivation rather than a second source,
    // which is worth saying out loud, but it is a different walk over the same
    // bytes and it catches a loader that silently loses blocks.
    let Some(dir) = client_dir() else {
        return;
    };
    let facet = 0u8;
    let index = std::fs::read(dir.join(format!("staidx{facet}.mul"))).unwrap();
    let data = std::fs::read(dir.join(format!("statics{facet}.mul"))).unwrap();

    let mut expected = 0usize;
    for entry in index.chunks_exact(12) {
        let offset = u32::from_le_bytes(entry[0..4].try_into().unwrap());
        let length = u32::from_le_bytes(entry[4..8].try_into().unwrap());
        if offset == u32::MAX || length == u32::MAX || length == 0 {
            continue;
        }
        let (offset, length) = (offset as usize, length as usize);
        if offset + length > data.len() {
            continue;
        }
        expected += length / 7;
    }

    let map = Map::load_facet(&dir, facet).unwrap();
    assert_eq!(map.static_count(), expected, "the loader and the index disagree");
    assert!(
        expected > 1_000_000,
        "Felucca holds millions of statics, not {expected}"
    );
}

#[test]
fn a_statics_coordinates_land_inside_its_own_block() {
    // `load_statics` recovers a block's world origin by inverting the
    // column-major block formula, and `block_index` applies that formula
    // forward. They are written out twice, in two places, and nothing makes
    // them agree. If they ever disagree, a static asks to be drawn in a
    // different block from the one it is stored in — and `statics_at` filters
    // by exact coordinates, so the symptom is furniture that quietly vanishes.
    let Some(dir) = client_dir() else {
        return;
    };
    let map = Map::load_facet(&dir, 0).unwrap();

    let mut found = 0;
    // Britain: dense enough that a sweep this small finds thousands.
    for y in 1500..1560u16 {
        for x in 1450..1510u16 {
            for item in map.statics_at(x, y) {
                assert_eq!((item.x, item.y), (x, y));
                found += 1;
            }
        }
    }
    // A sweep over empty ocean would satisfy every assertion above without
    // examining a single static.
    assert!(found > 500, "only {found} statics in the middle of Britain");
}

#[test]
fn the_shipped_tiledata_is_the_high_seas_layout() {
    // There is no version field. The layout is decided by which of the two
    // divides the file exactly, and this is that arithmetic against a file
    // rather than against a constant someone typed.
    let Some(dir) = client_dir() else {
        return;
    };
    let bytes = std::fs::read(dir.join("tiledata.mul")).unwrap();
    let data = TileData::parse(&bytes).expect("a shipped tiledata.mul parses");
    assert_eq!(
        data.format(),
        TileDataFormat::HighSeas,
        "a 7.0.x client is High Seas"
    );

    // The detection has to be exact, not merely satisfied. Drop one byte and
    // neither layout divides the file any more, so the answer must become "this
    // is not tiledata.mul" rather than the other layout — a detection that
    // rounded would pick the wrong stride and read every tile's flags from the
    // middle of its neighbour.
    assert!(
        TileData::parse(&bytes[..bytes.len() - 1]).is_none(),
        "a file one byte short still parsed, so the layout was not decided by arithmetic"
    );
}

#[test]
fn the_land_table_reads_as_names_and_not_as_bytes_from_the_next_field() {
    // The whole entry stride, checked by its consequence. One byte out and
    // every name picks up the tail of the field before it — which is exactly
    // how a wrong stride announces itself, and exactly what a synthetic fixture
    // cannot show, because a fixture is laid out by the same arithmetic.
    let Some(data) = tiledata() else {
        return;
    };
    for (id, name) in [
        (0x0003u16, "grass"),
        (0x0006, "grass"),
        (0x0016, "sand"),
        (0x00A8, "water"),
        (0x03A4, "snow"),
    ] {
        assert_eq!(data.land(id).name, name, "land {id:#06X}");
    }

    let named = (0..LAND_TILE_COUNT)
        .filter(|id| !data.land(*id as u16).name.is_empty())
        .count();
    // Roughly a quarter of the table is named on 7.0.116.0; the rest is
    // genuinely blank. A stride that had slipped would leave far fewer names
    // intact, and a count of zero would mean this test read nothing.
    assert!(named > 3000, "only {named} land tiles have a name");
}

#[test]
fn water_is_water_across_the_run_the_client_ships() {
    // Movement asks exactly this question, on every step, of every tile. The
    // four ids are one contiguous run because that is how the file stores the
    // ocean, and a flags field read at the wrong width or offset would not give
    // four in a row.
    let Some(data) = tiledata() else {
        return;
    };
    for id in 0x00A8u16..=0x00AB {
        let tile = data.land(id);
        assert_eq!(tile.name, "water", "land {id:#06X}");
        assert!(tile.flags.is_water(), "land {id:#06X} is not water");
        assert!(tile.flags.is_blocking(), "water blocks a walker");
        assert!(!tile.flags.is_platform(), "water is not something to stand on");
    }

    // And the ground beside it is not water, so the assertion above is not
    // simply true of everything.
    assert!(!data.land(0x0003).flags.is_water(), "grass is not water");
    assert!(!data.land(0x0003).flags.is_blocking(), "grass is walkable");
}

#[test]
fn the_static_entry_layout_lands_on_weight_layer_and_height() {
    // Height at 20 and name at 21, from Sphere's `CUOItemTypeRec_HS`. One byte
    // out and the height byte becomes the first character of the name — which
    // is the tell, and which needs a real entry to show, because a fixture puts
    // the bytes wherever the parser expects them.
    let Some(data) = tiledata() else {
        return;
    };

    let crate_tile = data.static_tile(0x0E3D);
    assert_eq!(crate_tile.name, "crate");
    assert_eq!(crate_tile.height, 3, "a crate is three units tall");
    assert_eq!(
        crate_tile.weight, 255,
        "255 is immovable, and a crate is furniture"
    );
    assert!(crate_tile.flags.is_blocking());

    // Stairs: the run at 1006 is what the climb rule is built on. Their height
    // is the full height; the terrain code halves it.
    let stairs = data.static_tile(1006);
    assert_eq!(stairs.name, "stone stairs");
    assert!(stairs.flags.is_climbable(), "1006 is the first climbable tile");
    assert_eq!(stairs.height, 10);
}

#[test]
fn a_real_tiledata_name_carries_the_plural_marker_the_client_resolves() {
    // `pluralize_name` was written for a bug report — "bolt%s% of cloth"
    // reaching the client verbatim — and tested on that string. This is the
    // marker in a shipped file, which is what says the parser preserves it
    // rather than eating the `%` as a name terminator.
    let Some(data) = tiledata() else {
        return;
    };
    let board = data.static_tile(0x1BD7);
    assert_eq!(board.name, "board%s", "the marker survives the name field");
    assert_eq!(
        openshard_uofiles::tiledata::pluralize_name(&board.name, true),
        "boards"
    );
    assert_eq!(
        openshard_uofiles::tiledata::pluralize_name(&board.name, false),
        "board"
    );
}

#[test]
fn land_tile_zero_is_a_dummy_and_sets_no_movement_bit() {
    // A quirk of the shipped file, pinned so it is not mistaken for a parser
    // bug the next time somebody prints the land table. Record 0 is the only
    // one written in the pre-High-Seas 26-byte shape: its name sits six bytes
    // into the entry, so read at the modern offsets it comes out as flags
    // 0x4E55_0000_0000_0000 and the name "ED" — the tail of "UNUSED".
    //
    // It is left alone rather than special-cased because the bits that land in
    // that flag word are all above bit 32, and every flag movement asks about
    // is below it. What matters is that this junk cannot make tile 0 walkable,
    // water, or a floor — and that is what is asserted.
    let Some(data) = tiledata() else {
        return;
    };
    let dummy = data.land(0);
    assert!(!dummy.flags.is_water());
    assert!(!dummy.flags.is_blocking());
    assert!(!dummy.flags.is_platform());
    assert!(!dummy.flags.is_climbable());
    assert!(
        !dummy.flags.has(openshard_uofiles::tiledata::TileFlags::FLOOR),
        "the dummy record must not read as a floor"
    );
    // The neighbours are ordinary records, which is what makes record 0 a quirk
    // of the file rather than a stride that is wrong everywhere.
    assert_eq!(data.land(2).name, "NODRAW");
    assert_eq!(data.land(3).name, "grass");
}

#[test]
fn the_corner_of_felucca_is_ocean_and_britain_is_not() {
    // Block 0 and a block deep inside the file, so that "the map loaded" means
    // more than "the first block parsed". A container concatenated in offset
    // order passes at (0,0) and fails here.
    let Some(dir) = client_dir() else {
        return;
    };
    let map = Map::load_facet(&dir, 0).unwrap();
    let data = tiledata().unwrap();

    let corner = map.land(0, 0).expect("(0,0) is on the map");
    assert!(
        data.land(corner.tile).flags.is_water(),
        "the north-west corner of Felucca is ocean, not tile {}",
        corner.tile
    );

    let britain = map.land(1495, 1629).expect("Britain is on the map");
    assert!(
        !data.land(britain.tile).flags.is_water(),
        "the middle of Britain came out as water, so the blocks are misplaced"
    );
}
