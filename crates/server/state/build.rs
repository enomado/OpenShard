//! Turns `data/*.json` into the three lookup tables this crate keeps in `const`s.
//!
//! All three are ported reference data — ServUO's `Data/bodyTable.cfg`, its
//! `BaseMount` subclasses, and `SkillInfo.Table` — and between them they were
//! 1,428 lines of Rust that no one has ever read as code: 469 body ids, 31
//! mount pairs, and 58 skills of thirteen columns apiece. They are 258 lines of
//! JSON now.
//!
//! The generated tables stay `const`, because two of them are binary-searched on
//! the tick path and the third is indexed by a `Skill` discriminant. **Sorting
//! is this script's job, not the data's**: `body_type` and `mount_item_for`
//! search sorted slices, and a table sorted by hand decays the first time
//! somebody appends a row. The doc comments for the generated items live here
//! rather than in the JSON — a data file is a poor place for prose, and this is
//! the file that decides what the item means.

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

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let out_dir = Path::new(&out_dir);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=data");

    for (name, render) in [
        ("body_types", body_types as fn(&str) -> String),
        ("mounts", mounts),
        ("skills", skills),
    ] {
        let path = Path::new("data").join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        std::fs::write(out_dir.join(format!("{name}.rs")), render(&text))
            .unwrap_or_else(|e| panic!("writing {name}.rs: {e}"));
    }
}
