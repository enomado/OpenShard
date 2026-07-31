//! Turns `data/*.json` into the `&'static` recipe tables `defs::*` publishes.
//!
//! The five trades are 485 recipes of pure data, generated once from ServUO and
//! edited as data ever since. Written out as Rust source they were sixteen
//! thousand lines — a `Recipe` literal is thirteen fields, eleven of which are
//! the default in almost every row, and each one dragged two named `const`s
//! beside it for its skills and its ingredients. As JSON with the defaults left
//! out they are five thousand, and a row is one line per ingredient.
//!
//! It is a build script rather than a runtime load on purpose: the tables stay
//! `const`, so nothing is parsed or allocated when a shard starts and the gump
//! still reads `&'static [Recipe]`. What a runtime load would report as an error
//! on the first craft of the day, this reports before the crate compiles.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

/// One trade's file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    /// The gump's left-hand column.
    groups: Vec<Text>,
    /// The material axis, for the four trades that have one.
    #[serde(default)]
    sub_res: Option<Axis>,
    /// Everything the trade can make.
    recipes: Vec<Row>,
}

/// `system::Text`: a number is a cliloc, a string is a literal.
#[derive(Deserialize)]
#[serde(untagged)]
enum Text {
    /// A cliloc number the client localizes itself.
    Cliloc(u32),
    /// A literal, for the rows ServUO has no number for.
    Str(String),
}

impl Text {
    /// The Rust expression for this text.
    fn expr(&self) -> String {
        match self {
            // Fully qualified: the generated file is `include!`d into modules
            // whose imports it cannot see, and a bare `ClilocId` would make the
            // generator depend on every one of them importing it.
            Self::Cliloc(n) => format!("Text::Cliloc(openshard_protocol::wire::ClilocId({n}))"),
            Self::Str(s) => format!("Text::Str({s:?})"),
        }
    }
}

/// `recipe::SubResAxis`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Axis {
    /// The resource graphic the axis substitutes a hue into.
    graphic: String,
    /// The heading over the material row.
    name: Text,
    /// The grades, plain one first.
    entries: Vec<AxisEntry>,
}

/// `recipe::SubRes`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AxisEntry {
    /// The hue that *is* this material.
    hue: String,
    /// Its name, for the material row.
    name: Text,
    /// The base skill needed to work it, in tenths.
    req_skill: i32,
    /// What is said when the crafter is not good enough.
    message: Text,
}

/// `recipe::Recipe`. Every field with a `default` is one the overwhelming
/// majority of rows leave alone, and leaving it out of the JSON is what makes a
/// recipe readable at a glance.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    /// The item art produced.
    graphic: String,
    /// Its name, for the gump.
    name: Text,
    /// Which group it files under.
    group: u16,
    /// How many come out.
    #[serde(default = "one")]
    amount: u16,
    /// A fixed hue for the result; zero means it takes the material's.
    #[serde(default = "zero")]
    hue: String,
    /// Consume every material in the pack and make as many as it can.
    #[serde(default)]
    use_all_res: bool,
    /// Tenths knocked off the "can you attempt this at all" gate.
    #[serde(default)]
    min_skill_offset: i32,
    /// Whether an exceptional one carries its maker's name.
    #[serde(default)]
    markable: bool,
    /// Never exceptional, whatever the roll.
    #[serde(default)]
    never_exceptional: bool,
    /// Always exceptional.
    #[serde(default)]
    always_exceptional: bool,
    /// What has to be standing nearby, on top of the system's own.
    #[serde(default)]
    needs: NeedsRow,
    /// The skills wanted; the system's own leads.
    skills: Vec<SkillRow>,
    /// What it eats.
    resources: Vec<ResRow>,
}

/// `recipe::CraftSkillReq`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillRow {
    /// The `Skill` variant's name, spelled as the enum spells it.
    skill: String,
    /// The bottom of the band, in tenths.
    min: i32,
    /// The top, at which the craft always succeeds.
    max: i32,
}

/// `recipe::CraftRes`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResRow {
    /// The item art consumed.
    graphic: String,
    /// And its hue; absent is the plain grade.
    #[serde(default = "zero")]
    hue: String,
    /// How many, per craft.
    amount: u16,
    /// What it is called, for the detail page.
    name: Text,
    /// What is said when there are not enough.
    message: Text,
    /// Whether the material axis substitutes into this line.
    #[serde(default)]
    from_axis: bool,
}

/// `system::Needs`, with every requirement absent by default.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct NeedsRow {
    /// A forge.
    #[serde(default)]
    forge: bool,
    /// An anvil.
    #[serde(default)]
    anvil: bool,
    /// Any fire at all.
    #[serde(default)]
    heat: bool,
    /// An oven.
    #[serde(default)]
    oven: bool,
    /// A flour mill.
    #[serde(default)]
    mill: bool,
    /// Water.
    #[serde(default)]
    water: bool,
}

/// serde's `default` wants a function, and one is the only sane stack size.
fn one() -> u16 {
    1
}

/// Likewise for a hue nobody wrote down.
fn zero() -> String {
    "0x0000".to_owned()
}

/// A hex literal from the data, passed through verbatim so the generated source
/// reads the way the table does — but parsed first, because `Graphic(0xZZZZ)`
/// would otherwise be a compile error in a file nobody has open.
fn hex(field: &str, raw: &str) -> String {
    let digits = raw
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{field}: {raw} is not 0x-prefixed"));
    u16::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{field}: {raw} is not a u16 ({e})"));
    raw.to_owned()
}

impl NeedsRow {
    /// The Rust expression, short where nothing is wanted — which is 484 rows of
    /// 485.
    fn expr(&self) -> String {
        if !(self.forge || self.anvil || self.heat || self.oven || self.mill || self.water) {
            return "Needs::none()".to_owned();
        }
        format!(
            "Needs {{ forge: {}, anvil: {}, heat: {}, oven: {}, mill: {}, water: {} }}",
            self.forge, self.anvil, self.heat, self.oven, self.mill, self.water
        )
    }
}

/// One trade's generated source.
fn generate(table: &Table) -> String {
    let mut out = String::new();
    out.push_str("// @generated by build.rs from the trade's `data/*.json`. Edit the JSON.\n\n");

    out.push_str("/// The gump's left-hand column.\npub const GROUPS: &[Text] = &[\n");
    for group in &table.groups {
        writeln!(out, "    {},", group.expr()).unwrap();
    }
    out.push_str("];\n\n");

    out.push_str("/// Everything this trade can make.\npub const RECIPES: &[Recipe] = &[\n");
    for row in &table.recipes {
        writeln!(out, "    Recipe {{").unwrap();
        writeln!(out, "        graphic: Graphic({}),", hex("graphic", &row.graphic)).unwrap();
        writeln!(out, "        name: {},", row.name.expr()).unwrap();
        writeln!(out, "        group: {},", row.group).unwrap();
        out.push_str("        skills: &[\n");
        for want in &row.skills {
            writeln!(
                out,
                "            CraftSkillReq {{ skill: Skill::{}, min: {}, max: {} }},",
                want.skill, want.min, want.max
            )
            .unwrap();
        }
        out.push_str("        ],\n        resources: &[\n");
        for res in &row.resources {
            writeln!(
                out,
                "            CraftRes {{ graphic: Graphic({}), hue: Hue({}), amount: {}, name: {}, \
                 message: {}, from_axis: {} }},",
                hex("resource graphic", &res.graphic),
                hex("resource hue", &res.hue),
                res.amount,
                res.name.expr(),
                res.message.expr(),
                res.from_axis
            )
            .unwrap();
        }
        out.push_str("        ],\n");
        writeln!(out, "        amount: {},", row.amount).unwrap();
        writeln!(out, "        hue: Hue({}),", hex("hue", &row.hue)).unwrap();
        writeln!(out, "        use_all_res: {},", row.use_all_res).unwrap();
        writeln!(out, "        min_skill_offset: {},", row.min_skill_offset).unwrap();
        writeln!(out, "        markable: {},", row.markable).unwrap();
        writeln!(out, "        never_exceptional: {},", row.never_exceptional).unwrap();
        writeln!(out, "        always_exceptional: {},", row.always_exceptional).unwrap();
        writeln!(out, "        needs: {},", row.needs.expr()).unwrap();
        out.push_str("    },\n");
    }
    out.push_str("];\n");

    if let Some(axis) = &table.sub_res {
        out.push_str("\n/// The material grades this trade's gump offers.\n");
        out.push_str("pub const SUB_RES: SubResAxis = SubResAxis {\n");
        writeln!(
            out,
            "    graphic: Graphic({}),",
            hex("axis graphic", &axis.graphic)
        )
        .unwrap();
        writeln!(out, "    name: {},", axis.name.expr()).unwrap();
        out.push_str("    entries: &[\n");
        for entry in &axis.entries {
            writeln!(
                out,
                "        SubRes {{ hue: Hue({}), name: {}, req_skill: {}, message: {} }},",
                hex("axis hue", &entry.hue),
                entry.name.expr(),
                entry.req_skill,
                entry.message.expr()
            )
            .unwrap();
        }
        out.push_str("    ],\n};\n");
    }

    out
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let data = Path::new("data");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", data.display());

    // Sorted, so a rebuild on a different filesystem produces the same bytes.
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(data).expect("crafting/data exists") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let stem = path
                .file_stem()
                .expect("a .json has a stem")
                .to_string_lossy()
                .into_owned();
            files.insert(stem, path);
        }
    }
    assert!(!files.is_empty(), "crafting/data has no tables");

    for (trade, path) in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let table: Table = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let generated = generate(&table);
        std::fs::write(Path::new(&out_dir).join(format!("{trade}.rs")), generated)
            .unwrap_or_else(|e| panic!("writing {trade}.rs: {e}"));
    }
}
