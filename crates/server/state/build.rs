//! Turns `data/*.json` into the lookup tables this crate keeps in `const`s.
//!
//! Every one is ported reference data — ServUO's `Data/bodyTable.cfg`, its
//! `BaseMount` subclasses, `SkillInfo.Table`, the names and `BaseSoundID`s off
//! its `BaseCreature`s, and the tile sets its harvest definitions scan. Between
//! them they were 1,799 lines of Rust that no one has ever read as code: 469
//! body ids one per line, 58 skills of thirteen columns apiece, 271 lines of
//! `match` arms keyed by body, and a hundred lines of bare tile ids. They are
//! 557 lines of JSON now.
//!
//! Two shapes come out of here, and which one a table gets is not a style
//! choice:
//!
//! - **A `const` slice**, for the tables that are searched. `body_type` and
//!   `mount_item_for` binary-search theirs on the tick path; `SKILLS` is indexed
//!   by a `Skill` discriminant.
//! - **A `const fn` over a `match`**, for `creature_name` and
//!   `creature_base_sound`. The compiler turns a dense integer `match` into a
//!   jump, and a search over a slice could not be `const fn` at all — so the
//!   generated code keeps the shape the hand-written code had.
//!
//! **Invariants are this script's job, not the data's.** It sorts what is
//! binary-searched, because a table sorted by hand decays the first time
//! somebody appends a row; it rejects a duplicate id, which a binary search
//! would answer arbitrarily and a `match` would answer with whichever arm came
//! first. The doc comments for the generated items live here rather than in the
//! JSON — a data file is a poor place for prose, and this is the file that
//! decides what the item means.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

/// The doc over the generated `BODY_TYPES`.
const BODY_TYPES_DOC: &str = "\
/// Every body ServUO's `Data/bodyTable.cfg` gives a type, sorted by id.
///
/// `Equipment` entries are dropped: they are item art, never a mobile. What is left
/// is what a creature can be.
///
/// The source is `data/body_types.json`, which groups the ids by type because
/// that is how a person reads them; the sort into id order happens here, in
/// `build.rs`, because that is what [`body_type`]'s binary search needs and a
/// hand-sorted table decays the first time a row is appended.";

/// The doc over the generated `MOUNTS`.
const MOUNTS_DOC: &str = "\
/// The mount-item graphic each rideable body is drawn as, sorted by body id.
///
/// Ported from ServUO's `BaseMount` subclasses — the `base(name, bodyID, itemID, …)`
/// each one passes, plus the alternating body/item arrays a class that rolls between
/// several looks keeps (`Horse` is one of four).
///
/// The source is `data/mounts.json`. Both directions of the mapping are derived
/// from this one table: two hand-kept halves is how a saved ride comes back as
/// the wrong animal.";

/// The doc over the generated `SKILLS`.
const SKILLS_DOC: &str = "\
/// ServUO's `SkillInfo.Table`, verbatim, indexed by skill id.
///
/// The source is `data/skills.json`. The length is checked by the type: a row
/// added there without a matching [`Skill`] variant will not compile.";

/// One row of `data/skills.json`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillRow {
    /// What it is called: "Alchemy", "Item Identification".
    name: String,
    /// The title a grandmaster earns.
    title: String,
    /// The stat the ML gain mechanic tries first — a `StatCode` variant's name.
    primary: String,
    /// The stat it falls back to.
    secondary: String,
    /// How much strength lends to the effective value, in hundredths.
    str_scale: u32,
    /// How much dexterity lends.
    dex_scale: u32,
    /// How much intelligence lends.
    int_scale: u32,
    /// The ceiling on the whole stat bonus. ServUO sums the *undivided* scales
    /// here; see `SkillInfo::stat_total` for why that is not a slip.
    stat_total: u32,
    /// The chance weight that training nudges strength, in thousandths.
    str_gain: u32,
    /// The same for dexterity.
    dex_gain: u32,
    /// The same for intelligence.
    int_gain: u32,
    /// A multiplier on how readily the skill trains, in per-mille.
    gain_factor: u32,
    /// Whether the skill can be used straight from the window's button. False on
    /// thirty-five of the fifty-eight, so it is left out of the data there.
    #[serde(default)]
    usable: bool,
    /// Whether it may be used with a spell in flight. Spirit Speak alone.
    #[serde(default)]
    use_while_casting: bool,
}

/// A hex id from the data. Parsed to sort by, and re-emitted verbatim so the
/// generated source reads the way the table does.
fn id(raw: &str) -> u16 {
    let digits = raw
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{raw} is not 0x-prefixed"));
    u16::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{raw} is not a u16 ({e})"))
}

/// `data/body_types.json`, grouped by type name, into a table sorted by id.
fn body_types(text: &str) -> String {
    let grouped: BTreeMap<String, Vec<String>> = serde_json::from_str(text).expect("body_types.json");

    // Sorted here rather than in the data, and checked for the duplicate that a
    // binary search would answer arbitrarily.
    let mut rows: Vec<(u16, String, String)> = grouped
        .iter()
        .flat_map(|(kind, ids)| ids.iter().map(move |raw| (id(raw), raw.clone(), kind.clone())))
        .collect();
    rows.sort_by_key(|(id, _, _)| *id);
    for pair in rows.windows(2) {
        assert_ne!(
            pair[0].0, pair[1].0,
            "body {:#06x} is listed as both {} and {}",
            pair[0].0, pair[0].2, pair[1].2
        );
    }

    let mut out = String::from("// @generated by build.rs from data/body_types.json.\n\n");
    out.push_str(BODY_TYPES_DOC);
    out.push_str("\nconst BODY_TYPES: &[(u16, BodyType)] = &[\n");
    for (_, raw, kind) in &rows {
        writeln!(out, "    ({raw}, BodyType::{kind}),").unwrap();
    }
    out.push_str("];\n");
    out
}

/// `data/mounts.json`, a body/item pair per row, into a table sorted by body.
fn mounts(text: &str) -> String {
    let pairs: Vec<(String, String)> = serde_json::from_str(text).expect("mounts.json");

    let mut rows: Vec<(u16, String, String)> = pairs
        .into_iter()
        .map(|(body, item)| (id(&body), body, item))
        .collect();
    rows.sort_by_key(|(body, _, _)| *body);
    for pair in rows.windows(2) {
        assert_ne!(pair[0].0, pair[1].0, "body {:#06x} is ridden twice", pair[0].0);
    }

    let mut out = String::from("// @generated by build.rs from data/mounts.json.\n\n");
    out.push_str(MOUNTS_DOC);
    out.push_str("\nconst MOUNTS: &[(u16, u16)] = &[\n");
    for (_, body, item) in &rows {
        let _ = id(item);
        writeln!(out, "    ({body}, {item}),").unwrap();
    }
    out.push_str("];\n");
    out
}

/// `data/skills.json`, in the order the client numbers the skills.
fn skills(text: &str) -> String {
    let rows: Vec<SkillRow> = serde_json::from_str(text).expect("skills.json");

    let mut out = String::from("// @generated by build.rs from data/skills.json.\n\n");
    out.push_str(SKILLS_DOC);
    out.push_str("\npub const SKILLS: [SkillInfo; SKILL_COUNT] = [\n");
    for row in &rows {
        writeln!(out, "    SkillInfo {{").unwrap();
        writeln!(out, "        name: {:?},", row.name).unwrap();
        writeln!(out, "        title: {:?},", row.title).unwrap();
        writeln!(out, "        str_scale: {},", row.str_scale).unwrap();
        writeln!(out, "        dex_scale: {},", row.dex_scale).unwrap();
        writeln!(out, "        int_scale: {},", row.int_scale).unwrap();
        writeln!(out, "        stat_total: {},", row.stat_total).unwrap();
        writeln!(out, "        str_gain: {},", row.str_gain).unwrap();
        writeln!(out, "        dex_gain: {},", row.dex_gain).unwrap();
        writeln!(out, "        int_gain: {},", row.int_gain).unwrap();
        writeln!(out, "        gain_factor: {},", row.gain_factor).unwrap();
        writeln!(out, "        primary: StatCode::{},", row.primary).unwrap();
        writeln!(out, "        secondary: StatCode::{},", row.secondary).unwrap();
        writeln!(out, "        usable: {},", row.usable).unwrap();
        writeln!(out, "        use_while_casting: {},", row.use_while_casting).unwrap();
        out.push_str("    },\n");
    }
    out.push_str("];\n");
    out
}

/// One `group:` of the creature files — a heading and the rows under it. The
/// heading becomes the comment it already was, so the generated `match` reads
/// the way the hand-written one did.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatureGroup {
    /// The heading: "Farm and forest animals", "Undead".
    group: String,
    /// The rows under it, in the order they are matched.
    rows: Vec<CreatureRow>,
}

/// One row: the bodies that share a value, and the value.
///
/// `ids` is a list because several bodies are the same creature — four horse
/// bodies, two cow bodies — and the arm they generate is the `|` pattern that
/// was there before. Which bodies share a row differs between the two files:
/// the dire, grey and timber wolves have three names and one howl.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreatureRow {
    /// The body ids this row answers for.
    ids: Vec<String>,
    /// The default name, in `creature_names.json`.
    #[serde(default)]
    name: Option<String>,
    /// The base sound id, in `creature_sounds.json`.
    #[serde(default)]
    sound: Option<String>,
    /// What the sound belongs to, kept as the trailing comment it was: a sound
    /// id says nothing on its own, and `0x00E5` is a wolf only if it says so.
    #[serde(default)]
    note: Option<String>,
}

/// The doc over the generated `creature_name`.
const CREATURE_NAME_DOC: &str = "\
/// The default name a creature's body gives it — \"a chicken\", \"a horse\" —
/// shown on single-click and in the tooltip when a spawn did not name it.
///
/// Creature names are not in any client file the way item names are (those come
/// from tiledata); every emulator holds its own table, ServUO on each
/// `BaseCreature`, Sphere in its chardefs. This is the core default that pack
/// data overrides — the same \"default in core, customise in pack\" split item
/// names and spells have — so the common Britannia wildlife and dungeon monsters
/// read right out of the box and an unlisted body simply stays nameless rather
/// than wearing a wrong label. Body ids are ServUO's.
///
/// The table is `data/creature_names.json`. Expand it there.";

/// The doc over the generated `creature_base_sound`.
const CREATURE_SOUND_DOC: &str = "\
/// A creature's base sound id — ServUO's `BaseSoundID`, keyed by body like
/// [`creature_name`]. Its attack, hurt and death sounds are fixed offsets from
/// it (`+2`, `+3`, `+4`), so an orc growls and a wolf howls instead of every
/// mobile making the human punch sound. `None` for a human body (which uses the
/// gendered death sounds) and for the passive fauna ServUO leaves silent (a
/// rabbit, a deer).
///
/// The table is `data/creature_sounds.json`. Grow it alongside
/// `data/creature_names.json` as bodies are added — the two are keyed the same
/// and neither is complete.";

/// A `match` over body ids, generated from one of the two creature files.
///
/// The shape is deliberately the one that was written by hand: a `match` rather
/// than a sorted slice and a binary search, because the compiler turns a dense
/// integer `match` into a jump and because both functions are `const fn`, which
/// a search over a slice could not be.
fn creatures(text: &str, file: &str, doc: &str, signature: &str, open: &str, close: &str) -> String {
    let groups: Vec<CreatureGroup> = serde_json::from_str(text).unwrap_or_else(|e| panic!("{file}: {e}"));

    let mut out = format!("// @generated by build.rs from data/{file}.\n\n");
    out.push_str(doc);
    out.push('\n');
    out.push_str("#[must_use]\n");
    writeln!(out, "{signature} {{").unwrap();
    writeln!(out, "    {open}").unwrap();

    let mut seen: BTreeMap<u16, String> = BTreeMap::new();
    for group in &groups {
        writeln!(out, "        // {}.", group.group).unwrap();
        for row in &group.rows {
            assert!(!row.ids.is_empty(), "{file}: a row with no body ids");
            // An id in two arms is unreachable in the second — the `match` would
            // compile and quietly answer with the first, which is how a creature
            // ends up wearing another one's name.
            for raw in &row.ids {
                let parsed = id(raw);
                if let Some(first) = seen.insert(parsed, group.group.clone()) {
                    panic!(
                        "{file}: body {raw} appears twice, under {first} and {}",
                        group.group
                    );
                }
            }
            let pattern = row.ids.join(" | ");
            let value = match (&row.name, &row.sound) {
                (Some(name), None) => format!("{name:?}"),
                (None, Some(sound)) => {
                    let _ = id(sound);
                    sound.clone()
                }
                _ => panic!("{file}: a row must carry exactly one of `name` and `sound`"),
            };
            match &row.note {
                Some(note) => writeln!(out, "        {pattern} => {value}, // {note}").unwrap(),
                None => writeln!(out, "        {pattern} => {value},").unwrap(),
            }
        }
    }

    out.push_str("        _ => return None,\n");
    writeln!(out, "    {close}").unwrap();
    out.push_str("}\n");
    out
}

/// `data/harvest_tiles.json` — the four tile sets a harvest definition scans for.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HarvestTiles {
    /// ServUO's `Mining.m_MountainAndCaveTiles`, land ids and Ter Mur statics.
    mountain_and_cave: Vec<u16>,
    /// ServUO's sand tiles.
    sand: Vec<u16>,
    /// ServUO's `Lumberjacking.m_TreeTiles` — all statics.
    tree: Vec<String>,
    /// Water, as inclusive `(from, to)` ranges rather than every id.
    water: Vec<(String, String)>,
}

/// The four tile tables, emitted the way they were written: mining's and sand's
/// as decimal (they are land ids, and ServUO lists them decimal), the tree and
/// water statics as hex.
///
/// Nothing here is sorted or deduplicated. These are `contains` tables, so
/// neither would change an answer, and the order is ServUO's — keeping it is
/// what lets the two be diffed against each other.
fn harvest_tiles(text: &str) -> String {
    let tiles: HarvestTiles = serde_json::from_str(text).expect("harvest_tiles.json");

    let mut out = String::from("// @generated by build.rs from data/harvest_tiles.json.\n\n");

    for (ident, doc, values) in [
        (
            "MOUNTAIN_AND_CAVE_TILES",
            "ServUO's `Mining.m_MountainAndCaveTiles`, verbatim. Land ids below 0x4000, and\n\
             /// the Ter Mur cave statics above it.",
            &tiles.mountain_and_cave,
        ),
        ("SAND_TILES", "ServUO's sand tiles, verbatim.", &tiles.sand),
    ] {
        writeln!(out, "/// {doc}").unwrap();
        writeln!(out, "static {ident}: &[u16] = &[").unwrap();
        for chunk in values.chunks(10) {
            let row: Vec<String> = chunk.iter().map(u16::to_string).collect();
            writeln!(out, "    {},", row.join(", ")).unwrap();
        }
        out.push_str("];\n\n");
    }

    out.push_str(
        "/// ServUO's `Lumberjacking.m_TreeTiles`, verbatim — all statics, so every id is\n\
         /// matched through [`tile_key`]'s `| 0x4000`.\n",
    );
    out.push_str("static TREE_TILES: &[u16] = &[\n");
    for chunk in tiles.tree.chunks(8) {
        let row: Vec<String> = chunk
            .iter()
            .map(|t| {
                let _ = id(t);
                t.clone()
            })
            .collect();
        writeln!(out, "    {},", row.join(", ")).unwrap();
    }
    out.push_str("];\n\n");

    out.push_str(
        "/// Water, as inclusive `(from, to)` ranges: the sets are contiguous runs and\n\
         /// listing every id would be four hundred rows for no gain.\n",
    );
    out.push_str("static WATER_TILES: &[(u16, u16)] = &[\n");
    for (from, to) in &tiles.water {
        assert!(id(from) <= id(to), "water range {from}..{to} runs backwards");
        writeln!(out, "    ({from}, {to}),").unwrap();
    }
    out.push_str("];\n");
    out
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let out_dir = Path::new(&out_dir);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=data");

    let names = std::fs::read_to_string("data/creature_names.json").expect("creature_names.json");
    std::fs::write(
        out_dir.join("creature_names.rs"),
        creatures(
            &names,
            "creature_names.json",
            CREATURE_NAME_DOC,
            "pub const fn creature_name(body: Graphic) -> Option<&'static str>",
            "Some(match body.0 {",
            "})",
        ),
    )
    .expect("writing creature_names.rs");

    let sounds = std::fs::read_to_string("data/creature_sounds.json").expect("creature_sounds.json");
    std::fs::write(
        out_dir.join("creature_sounds.rs"),
        creatures(
            &sounds,
            "creature_sounds.json",
            CREATURE_SOUND_DOC,
            "pub const fn creature_base_sound(body: Graphic) -> Option<SoundId>",
            "Some(SoundId(match body.0 {",
            "}))",
        ),
    )
    .expect("writing creature_sounds.rs");

    for (name, render) in [
        ("body_types", body_types as fn(&str) -> String),
        ("mounts", mounts),
        ("skills", skills),
        ("harvest_tiles", harvest_tiles),
    ] {
        let path = Path::new("data").join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        std::fs::write(out_dir.join(format!("{name}.rs")), render(&text))
            .unwrap_or_else(|e| panic!("writing {name}.rs: {e}"));
    }
}
