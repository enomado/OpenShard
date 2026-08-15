//! Turns `data/spawns.json` into the spawn regions this crate ships.
//!
//! The first `build.rs` in `world`, and it follows the conventions
//! `state/build.rs` sets out: serde structs live here rather than in the crate,
//! every one `deny_unknown_fields`, the doc comments for generated items are
//! `const`s in this file, and the invariants are checked here so a failure names
//! the JSON rather than surfacing as a quiet oddity in a running shard.
//!
//! # Why the creatures are a named table and not written where they are used
//!
//! Felucca's 1,430 spawn regions reference 8,338 creatures, and there are **193
//! distinct ones**. Written inline that is 8,338 copies of an eight-field struct
//! in the data and something like 150,000 lines in the generated source — a
//! compile-time cost and a diff nobody can read, for a file that is 97.7%
//! repetition.
//!
//! So `data/spawns.json` has two halves: a `creatures` table keyed by name, and
//! spawners that list the names they may put down. This script resolves the
//! references, and a name with no entry is a build failure that says which
//! spawner asked for it. The generated source is one `CreatureTemplate` literal
//! per reference — the resolution happens here, so nothing is looked up at
//! runtime and `Spawner` is unchanged.
//!
//! The names are authored, not derived. Most come from the same creature the
//! engine's own `creature_name` table knows; the rest are `body 0x00e8` and say
//! so, because that body genuinely has no name in the tree yet — which is also
//! why those creatures single-click with no label in game.
//!
//! # Ten regions used to start off the map, and nobody could have known
//!
//! The converter's output had ten regions with a negative `x` or `y`. The engine
//! never saw one: the script boundary turned it into `0` on the way in, keeping
//! the width and height, so the box was quietly shifted onto the map rather than
//! clipped. This data is what the engine *received*, so those ten carry the
//! zeroes, and the `u16` here means the case cannot come back.
//!
//! It is worth knowing which of the two a fix would be. Clipping instead —
//! keeping the far edge and taking the overhang off the size — is the more
//! defensible geometry, and it is a **change to where creatures spawn**, so it
//! belongs in a commit that says so rather than riding in on a data move.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

/// The doc over the generated `shipped`.
const SPAWNS_DOC: &str = "\
/// Every spawn set the shard ships, built fresh from `data/spawns.json`.
///
/// Ported from ServUO's `Spawner` placements, one region per `spawner.map`
/// entry, with each creature's stats read off the `BaseCreature` subclass it
/// names.
///
/// **Keyed by an admin verb**, like `region::shipped`: `populate:felucca` is a
/// button in the staff menu and a `--seed` argument, because an operator lays and
/// clears a facet's population by hand. The verb travels with the data rather
/// than being spelled into the server.
///
/// The `Spawner`s come out with `id: 0` and `next_spawn: 0`, which is not a
/// value — it is the same placeholder the script bridge passes.
/// [`World::register_spawner`](crate::World) assigns the real id and jitters the
/// first spawn across the respawn window, and it does that for *any* caller, so
/// this side has neither to give.
///
/// Written in the file's order. Nothing reads a spawner by position — the id is
/// assigned on registration and the de-duplication is by `SpawnArea` — so the
/// order is free, and keeping the file's makes a diff against the JSON legible.
";

/// One set in `data/spawns.json`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnFile {
    /// The admin verb that lays it — `world::admin`'s `ROWS` is the other half.
    verb: String,
    /// Which facet every region in the set belongs to.
    ///
    /// One facet per set, not per region: a set *is* a facet's population, and
    /// the verb says which ("populate:felucca"). A second facet is a second set.
    facet: u8,
    /// The distinct creatures, by the name the spawners refer to them by.
    ///
    /// A `BTreeMap` so the generated source does not reorder when serde's map
    /// iteration does — the emitted code is diffed by people.
    creatures: BTreeMap<String, Creature>,
    /// The regions, each listing creature names from the table above.
    spawners: Vec<SpawnerDef>,
}

/// One creature a region may put down.
///
/// Mirrors `CreatureTemplate`, with everything ServUO leaves at a default left
/// out of the data. The two that are *not* plain `Default` are called out below.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Creature {
    /// The body graphic.
    body: u16,
    /// Its hue. Zero — no hue — for every creature so far.
    #[serde(default)]
    hue: u16,
    /// Starting and maximum hit points.
    #[serde(default = "one")]
    hits: u16,
    /// The health-bar colour, as the wire value.
    #[serde(default = "innocent")]
    notoriety: u8,
    /// Melee damage before resistance.
    #[serde(default)]
    damage: u16,
    /// Physical resistance, a percentage.
    #[serde(default)]
    resistance: u8,
    /// How widely known it is.
    #[serde(default)]
    fame: i32,
    /// Which way. Negative is evil.
    #[serde(default)]
    karma: i32,
    /// Swing cadence in ticks; `0` derives it from dexterity.
    #[serde(default)]
    swing: u64,
    /// How far it notices a target; `0` for a placid animal.
    #[serde(default)]
    sight: u8,
    /// Whether it starts fights. Left out, it is [`natural_aggression`]'s answer
    /// for the body — the rule the script bridge applied, moved here with it.
    #[serde(default)]
    aggression: Option<u8>,
    /// Ticks between beats while hunting; `0` takes the shard default.
    #[serde(default)]
    beat: u64,
    /// Its ranged reach, if it has one.
    #[serde(default)]
    ranged: Option<u8>,
    /// The ranged attack's damage type, as the wire value.
    #[serde(default)]
    ranged_kind: u8,
    /// Whether it drifts when idle.
    #[serde(default)]
    wander: bool,
    /// Trained combat skills, `(skill id, value in tenths)`.
    #[serde(default)]
    skills: Vec<(u8, u16)>,
}

/// A creature's default hit points: one. `u16`'s own default is zero, which is a
/// creature that is dead where it stands.
const fn one() -> u16 {
    1
}

/// A creature's default notoriety: `Innocent`, the wire value 1.
const fn innocent() -> u8 {
    1
}

/// The posture a creature gets when the data does not set one.
///
/// Carried over verbatim from the script bridge's `default_aggression`, which is
/// where this rule lived while the spawn data was a pack's. Ordinary horses are
/// tameable mounts, not monsters: a body that is one must not hunt nearby players
/// merely because nobody wrote a field. Every other body keeps the historic
/// aggressive default, and the data can always say otherwise.
const fn natural_aggression(body: u16) -> u8 {
    match body {
        // Aggression::Passive.
        0x00C8 | 0x00CC | 0x00E2 | 0x00E4 => 0,
        // Aggression::Aggressive.
        _ => 2,
    }
}

/// One spawn region.
///
/// No `id` and no `next_spawn`: both belong to the live spawner rather than to
/// the content. `register_spawner` assigns the id and jitters the first spawn,
/// and a number written here would be a second source for either.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnerDef {
    /// West edge.
    x: u16,
    /// North edge.
    y: u16,
    /// Width in tiles.
    width: u16,
    /// Height in tiles.
    height: u16,
    /// The most live creatures it keeps.
    max_count: u16,
    /// Ticks to wait after a spawn before the next.
    respawn_delay: u64,
    /// Which creatures, by name into the file's table.
    creatures: Vec<String>,
}

/// The Rust expression for one creature, fully qualified — the generated file is
/// `include!`d into a module whose imports it cannot see.
///
/// The wire-value conversions are the total ones (`from_bits`, `from_u8`), which
/// fold anything unrecognised to a safe default rather than failing. That would
/// turn a typo into a blue health bar, so the *range* checks are asserts in
/// [`spawns`] instead, where the message can name the creature.
fn creature_expr(name: &str, c: &Creature, indent: &str) -> String {
    let aggression = c.aggression.unwrap_or_else(|| natural_aggression(c.body));
    let ranged = match c.ranged {
        Some(range) => format!(
            "Some(openshard_protocol::world::RangedRange::new({range}).expect({:?}))",
            format!("{name} has a ranged reach of zero, which is no reach at all")
        ),
        None => "None".to_owned(),
    };
    let skills: Vec<String> = c
        .skills
        .iter()
        .map(|(id, value)| {
            format!(
                "(openshard_state::Skill::from_id({id}).expect({:?}), {value})",
                format!("{name} names skill id {id}, which is not a skill")
            )
        })
        .collect();
    let skills = if skills.is_empty() {
        "Vec::new()".to_owned()
    } else {
        format!("vec![{}]", skills.join(", "))
    };

    let mut out = String::new();
    // The name is a comment rather than a field: `CreatureTemplate` has no room
    // for one, and a table of 193 anonymous stat blocks is unreadable.
    writeln!(out, "{indent}// {name}").unwrap();
    writeln!(out, "{indent}crate::spawner::CreatureTemplate {{").unwrap();
    writeln!(
        out,
        "{indent}    body: openshard_protocol::wire::Graphic({}),",
        c.body
    )
    .unwrap();
    writeln!(out, "{indent}    hue: openshard_protocol::wire::Hue({}),", c.hue).unwrap();
    writeln!(out, "{indent}    hits: {},", c.hits).unwrap();
    writeln!(
        out,
        "{indent}    notoriety: openshard_protocol::mobile::Notoriety::from_bits({}),",
        c.notoriety
    )
    .unwrap();
    writeln!(out, "{indent}    damage: {},", c.damage).unwrap();
    writeln!(
        out,
        "{indent}    resistance: openshard_protocol::world::PhysicalResistance::new({}),",
        c.resistance
    )
    .unwrap();
    writeln!(out, "{indent}    fame: {},", c.fame).unwrap();
    writeln!(out, "{indent}    karma: {},", c.karma).unwrap();
    writeln!(out, "{indent}    swing: {},", c.swing).unwrap();
    writeln!(
        out,
        "{indent}    sight: openshard_protocol::world::Sight({}),",
        c.sight
    )
    .unwrap();
    writeln!(
        out,
        "{indent}    aggression: openshard_protocol::world::Aggression::from_bits({aggression}),"
    )
    .unwrap();
    writeln!(out, "{indent}    beat: {},", c.beat).unwrap();
    writeln!(out, "{indent}    ranged: {ranged},").unwrap();
    writeln!(
        out,
        "{indent}    ranged_kind: openshard_protocol::world::DamageType::from_u8({}),",
        c.ranged_kind
    )
    .unwrap();
    writeln!(out, "{indent}    wander: {},", c.wander).unwrap();
    writeln!(out, "{indent}    skills: {skills},").unwrap();
    write!(out, "{indent}}}").unwrap();
    out
}

/// `data/spawns.json` into the `shipped` constructor.
fn spawns(text: &str) -> String {
    let file: SpawnFile = serde_json::from_str(text).expect("spawns.json");

    assert!(
        !file.verb.is_empty(),
        "a spawn set with no verb can never be laid"
    );
    assert!(
        !file.spawners.is_empty(),
        "spawn set {:?} lays no regions at all",
        file.verb
    );

    // A creature nothing spawns is dead weight that reads as content. Collected
    // before the emit so the message can name all of them at once.
    let referenced: std::collections::BTreeSet<&str> = file
        .spawners
        .iter()
        .flat_map(|s| s.creatures.iter().map(String::as_str))
        .collect();
    let orphans: Vec<&str> = file
        .creatures
        .keys()
        .map(String::as_str)
        .filter(|name| !referenced.contains(name))
        .collect();
    assert!(
        orphans.is_empty(),
        "spawns.json defines creatures no region spawns: {}",
        orphans.join(", ")
    );

    // Each creature's range checks, once per definition rather than once per
    // reference — the conversions the emitted code uses are total and would fold
    // a typo into a safe default instead of failing.
    for (name, c) in &file.creatures {
        assert!(
            c.hits > 0,
            "{name} has no hit points, so it is dead where it stands"
        );
        assert!(
            (1..=7).contains(&c.notoriety),
            "{name} has notoriety {}, which is not a wire value — the health bar would \
             silently read innocent",
            c.notoriety
        );
        assert!(
            c.aggression.is_none_or(|a| a <= 2),
            "{name} has an aggression that is not 0, 1 or 2, and anything else reads as \
             aggressive"
        );
        assert!(
            c.ranged_kind <= 4,
            "{name} has a ranged damage kind that is not a wire value, and anything else \
             reads as physical"
        );
        assert!(
            c.resistance <= 100,
            "{name} resists {}% of physical damage",
            c.resistance
        );
    }

    // The table, once. Emitting a literal per *reference* would be 8,338 of them
    // and something like 150,000 lines of source for rustc to chew through; the
    // spawners index into this instead and clone what they name.
    let index: BTreeMap<&str, usize> = file
        .creatures
        .keys()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();

    let mut out = String::from("// @generated by build.rs from data/spawns.json.\n\n");
    out.push_str(
        "/// Every distinct creature `data/spawns.json` defines, in the order the file\n\
         /// lists them. Built once per call and indexed into by the spawners below —\n\
         /// 1,430 regions name 8,338 creatures between them and there are 193 of them,\n\
         /// so a literal per reference would be almost all repetition.\n",
    );
    out.push_str("fn creature_table() -> Vec<crate::spawner::CreatureTemplate> {\n    vec![\n");
    for (name, creature) in &file.creatures {
        out.push_str(&creature_expr(name, creature, "        "));
        out.push_str(",\n");
    }
    out.push_str("    ]\n}\n\n");

    out.push_str(SPAWNS_DOC);
    out.push_str("#[must_use]\npub fn shipped() -> Vec<SpawnSet> {\n");
    out.push_str("    let c = creature_table();\n    vec![SpawnSet {\n");
    writeln!(out, "        verb: {:?}.to_owned(),", file.verb).unwrap();
    out.push_str("        spawners: vec![\n");

    for spawner in &file.spawners {
        assert!(
            spawner.width > 0 && spawner.height > 0,
            "a spawn region at {},{} is {}x{}, which contains no tile to spawn on",
            spawner.x,
            spawner.y,
            spawner.width,
            spawner.height
        );
        assert!(
            spawner.max_count > 0,
            "the spawn region at {},{} keeps no creatures alive, so it does nothing",
            spawner.x,
            spawner.y
        );
        assert!(
            !spawner.creatures.is_empty(),
            "the spawn region at {},{} has nothing to spawn",
            spawner.x,
            spawner.y
        );

        let picks: Vec<String> = spawner
            .creatures
            .iter()
            .map(|name| {
                let at = index.get(name.as_str()).unwrap_or_else(|| {
                    panic!(
                        "the spawn region at {},{} spawns {name:?}, which no creature in \
                         spawns.json is called",
                        spawner.x, spawner.y
                    )
                });
                format!("c[{at}].clone()")
            })
            .collect();

        out.push_str("            crate::spawner::Spawner::new(\n");
        // The placeholder id, overwritten by `register_spawner`.
        out.push_str("                0,\n");
        writeln!(
            out,
            "                crate::spawner::SpawnArea {{ x: {}, y: {}, width: {}, height: {}, \
             facet: openshard_protocol::world::Facet({}) }},",
            spawner.x, spawner.y, spawner.width, spawner.height, file.facet
        )
        .unwrap();
        // Wrapped rather than one index per line: these are 8,338 short
        // expressions, and a line each is the file size this table exists to avoid.
        out.push_str("                vec![");
        let mut column = 0;
        for (i, pick) in picks.iter().enumerate() {
            if column > 0 && column + pick.len() > 88 {
                out.push_str("\n                     ");
                column = 21;
            }
            out.push_str(pick);
            column += pick.len();
            if i + 1 < picks.len() {
                out.push_str(", ");
                column += 2;
            }
        }
        out.push_str("],\n");
        writeln!(out, "                {},", spawner.max_count).unwrap();
        writeln!(out, "                {},", spawner.respawn_delay).unwrap();
        out.push_str("            ),\n");
    }

    out.push_str("        ],\n    }]\n}\n");
    out
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=data");

    let path = Path::new("data").join("spawns.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    std::fs::write(Path::new(&out_dir).join("spawns.rs"), spawns(&text))
        .unwrap_or_else(|e| panic!("writing spawns.rs: {e}"));
}
