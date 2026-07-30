//! N8 — the sweep's last stage: `docs/protocol_newtypes.md`'s N10 says every
//! bare integer field remaining in the crate is either wrapped or on an
//! explicit, reasoned allowlist, and that the count is asserted, not assumed.
//! This test is the assertion — "no violations found" from a detector that
//! examined nothing has been green here before, so this one records what it
//! looked at and fails loudly if the allowlist and the crate disagree, in
//! either direction.
//!
//! The scan is a plain text walk, not a syntax tree: `syn` is not a
//! dependency anywhere in this workspace, and every existing self-check in
//! this crate (`feature.rs`'s `all_lists_every_feature_exactly_once`,
//! `direction.rs`'s `every_byte_the_client_can_send_names_a_direction`) is an
//! in-process assertion over program data, not a source-file reader — this is
//! the first of that kind, so it stays as small as the job allows.
//!
//! Two shapes a naive `pub name: u16` grep would miss are the reason N7's own
//! backlog forced this stage: a tuple (`StartLocation::position` was a bare
//! `(i32, i32, i32)`, fixed in N8) and a `Vec` of one
//! (`PropertyQueryRequest::serials` was a bare `Vec<u32>`, fixed in N7). Both
//! are gone now, but the *type* of gap is not — a later field could
//! reintroduce either — so the scan matches on whether the type *expression*
//! names a primitive integer anywhere in it, not on the field being one on
//! its own; that is what catches `GumpResponse::text_entries: Vec<(u16,
//! String)>` below without a second rule.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Every bare-integer field the sweep leaves in place, and why — `(file,
/// field, reason)`. This is what the sweep actually enforces; the table in
/// `docs/protocol_newtypes.md`'s N10 section is the narrative for the same
/// list, and the two must agree by hand. A field appears here once per
/// struct that has it: `vendor.rs`'s `amount` is on three different structs
/// and is on this list three times, one per struct, because the scan below
/// counts occurrences of `(file, field)`, not distinct names.
const ALLOWLIST: &[(&str, &str, &str)] = &[
    // -- geometric components: N1 amendment 2, N2 amendment 2 --------------
    (
        "world.rs",
        "x",
        "Point: one geometric quantity's own axis, added to and compared on every step",
    ),
    ("world.rs", "y", "Point: same"),
    ("world.rs", "z", "Point: same"),
    ("world.rs", "width", "MapSize: same argument as Point"),
    ("world.rs", "height", "MapSize: same"),
    (
        "gump.rs",
        "x",
        "GumpPoint: the same argument in gump-space pixels, signed for negative layout offsets",
    ),
    ("gump.rs", "y", "GumpPoint: same"),
    // -- current/max bars: N2 amendment 2 -----------------------------------
    (
        "mobile.rs",
        "current",
        "Vitals: half a bar is not a smaller number, it is the wrong ratio",
    ),
    ("mobile.rs", "max", "Vitals: same"),
    // -- status-bar quantities: N2 amendment 3 ------------------------------
    (
        "mobile.rs",
        "strength",
        "MobileStatus: a quantity clamped by [gameplay], not a protocol rule",
    ),
    ("mobile.rs", "dexterity", "MobileStatus: same"),
    ("mobile.rs", "intelligence", "MobileStatus: same"),
    ("mobile.rs", "gold", "MobileStatus: same"),
    ("mobile.rs", "armor", "MobileStatus: same"),
    ("mobile.rs", "weight", "MobileStatus: same"),
    ("mobile.rs", "max_weight", "MobileStatus: same"),
    ("mobile.rs", "stat_cap", "MobileStatus: same"),
    ("mobile.rs", "followers", "MobileStatus: same"),
    ("mobile.rs", "followers_max", "MobileStatus: same"),
    // -- stack sizes and gold: N4 amendments 8/9, N5 amendment 1 -----------
    (
        "containers.rs",
        "amount",
        "ContainedItem: a stack size, the MobileStatus quantity argument",
    ),
    ("items.rs", "amount", "WorldItem: the same stack size, outbound"),
    ("items.rs", "amount", "PickUpItem: the same stack size, inbound"),
    (
        "vendor.rs",
        "price",
        "BuyLine: gold, the MobileStatus::gold argument",
    ),
    ("vendor.rs", "amount", "Purchase: a stack size"),
    ("vendor.rs", "amount", "SellLine: same"),
    ("vendor.rs", "price", "SellLine: gold, same as BuyLine::price"),
    ("vendor.rs", "amount", "Sale: same stack size"),
    // -- login's own quantities, and a type that is not a packet struct -----
    // N6 amendments 7/8
    // `percent_full` was on this list and is not any more: 100 is a ceiling
    // the client imposes, so it became `PercentFull` with the clamp in its
    // constructor. `timezone` has no such rule and stays.
    (
        "login.rs",
        "timezone",
        "ShardEntry: a quantity, the MobileStatus argument",
    ),
    (
        "version.rs",
        "major",
        "ClientVersion is not a packet struct; narrowed from a seed dword or a 0xBD string",
    ),
    ("version.rs", "minor", "ClientVersion: same"),
    ("version.rs", "revision", "ClientVersion: same"),
    ("version.rs", "patch", "ClientVersion: same"),
    // -- animation/effect numbers whose domain lives above protocol ---------
    // module doc in feedback.rs
    (
        "feedback.rs",
        "action",
        "Animation: body-specific index; openshard_state::Action lives above protocol",
    ),
    (
        "feedback.rs",
        "frame_count",
        "Animation: a quantity, a per-effect literal at every call site",
    ),
    ("feedback.rs", "repeat_count", "Animation: same"),
    ("feedback.rs", "delay", "Animation: same"),
    (
        "feedback.rs",
        "animation_type",
        "NewAnimation: category index; domain above protocol, same as Animation::action",
    ),
    ("feedback.rs", "action", "NewAnimation: same"),
    ("feedback.rs", "delay", "NewAnimation: a quantity"),
    (
        "feedback.rs",
        "speed",
        "GraphicalEffect: a quantity, a per-effect literal",
    ),
    ("feedback.rs", "duration", "GraphicalEffect: same"),
    (
        "feedback.rs",
        "render_mode",
        "HuedEffect: no non-test caller constructs one, nothing to classify against",
    ),
    // -- skill numbers whose domain lives above protocol: N7 amendment ------
    (
        "skill.rs",
        "id",
        "SkillEntry: openshard_state::Skill lives above protocol, same argument as feedback.rs",
    ),
    ("skill.rs", "value", "SkillEntry: a quantity"),
    ("skill.rs", "base", "SkillEntry: same"),
    ("skill.rs", "cap", "SkillEntry: same"),
    // -- one spell school ever wired, and its bitmask: N7 amendment 8, N8 ---
    (
        "spellbook.rs",
        "offset",
        "SpellbookContent: nothing branches on the byte while no second school exists",
    ),
    (
        "spellbook.rs",
        "content",
        "SpellbookContent: a membership bitmask over spell ids; which ids exist is Community \
         Pack content, the feedback.rs argument again",
    ),
    // -- computed by the server, read only by the client: N7 amendment 6 ----
    (
        "properties.rs",
        "hash",
        "TooltipRevision: server-computed, client-only reader; none of N3's four classes fit",
    ),
    // -- text field ids the pack script owns, above the engine: N5 ----------
    (
        "gump.rs",
        "text_entries",
        "GumpResponse: a Vec of (id, text); which id a pack drew is the pack's business",
    ),
    // -- diagnostic fields on typed errors, not wire data: N8 ---------------
    (
        "error.rs",
        "expected",
        "WrongPacket: the id the decoder wanted, chosen by the dispatcher, not client input",
    ),
    (
        "error.rs",
        "found",
        "WrongPacket: the packet's own header id, already matched before this fires",
    ),
    (
        "gump.rs",
        "id",
        "InvalidSwitchId: the rejected value, carried for Display, not a second wire field",
    ),
    (
        "context.rs",
        "tag",
        "InvalidContextMenuIndex: same — the rejected value, carried for Display",
    ),
    ("wire.rs", "slot", "InvalidCharacterSlot: same"),
];

/// Whether a field's type expression names a primitive wire integer anywhere
/// in it — `u16`, `(i32, i32, i32)`, `Vec<u32>` and `Vec<(u16, String)>` all
/// do; `Point`, `Serial`, `Vec<RawSerial>` do not. Matching the whole type
/// text rather than requiring the field's own type to *be* one of these
/// tokens is what closes N7's tuple and collection blind spots without a
/// second rule for each.
fn names_a_primitive_int(ty: &str) -> bool {
    let mut token = String::new();
    let mut hit = false;
    for c in ty.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() || c == '_' {
            token.push(c);
            continue;
        }
        if matches!(
            token.as_str(),
            "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64"
        ) {
            hit = true;
        }
        token.clear();
    }
    hit
}

/// Every `pub name: <type containing a primitive int>` field declaration in
/// one source file, as `(field name, type text)`.
///
/// A field declaration is recognised as "`pub `, then an identifier with no
/// space or `(` in it, then `:`" — which a struct field satisfies and a
/// method (`pub fn foo(x: u16)`), an associated const (`pub const ID: u8`) or
/// a tuple struct's own definition (`pub struct RawSerial(pub u32);`, no
/// colon at all) do not, so none of those needs a separate exclusion.
fn scan_file(text: &str) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("pub ") else {
            continue;
        };
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let name = &rest[..colon];
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let ty = rest[colon + 1..].trim().trim_end_matches(',').trim();
        if names_a_primitive_int(ty) {
            hits.push((name.to_owned(), ty.to_owned()));
        }
    }
    hits
}

/// `a` minus `b` as multisets, keyed by `(file, field)` — what is in `a` more
/// times than in `b`. Called both ways: found-minus-allowed is a violation to
/// fix, allowed-minus-found is a stale entry the field's own fix should have
/// deleted.
fn multiset_extra(a: &[(String, String)], b: &[(String, String)]) -> Vec<(String, String)> {
    let mut remaining: HashMap<(String, String), i32> = HashMap::new();
    for key in b {
        *remaining.entry(key.clone()).or_insert(0) += 1;
    }
    let mut extra = Vec::new();
    for key in a {
        let count = remaining.entry(key.clone()).or_insert(0);
        if *count > 0 {
            *count -= 1;
        } else {
            extra.push(key.clone());
        }
    }
    extra.sort();
    extra
}

#[test]
fn every_bare_integer_field_is_wrapped_or_allowlisted() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut entries: Vec<_> = fs::read_dir(&src_dir)
        .expect("protocol crate must have a src directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    entries.sort();
    assert!(
        entries.len() > 20,
        "found only {} .rs files under {src_dir:?} — the scan is reading the wrong directory",
        entries.len(),
    );

    let mut found: Vec<(String, String)> = Vec::new();
    for path in &entries {
        let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let text = fs::read_to_string(path).expect("protocol source file must be readable");
        for (field, _ty) in scan_file(&text) {
            found.push((file_name.clone(), field));
        }
    }

    let allowed: Vec<(String, String)> = ALLOWLIST
        .iter()
        .map(|(file, field, _reason)| ((*file).to_owned(), (*field).to_owned()))
        .collect();

    let unallowed = multiset_extra(&found, &allowed);
    let stale = multiset_extra(&allowed, &found);

    assert!(
        unallowed.is_empty() && stale.is_empty(),
        "N10's coverage check disagrees with the allowlist in `tests/bare_integer_fields.rs`.\n\
         \n\
         Bare integer fields found in `src/` but NOT on the allowlist — give the field a real \
         type, or add a reasoned entry:\n{unallowed:#?}\n\
         \n\
         Allowlist entries with no matching field left in `src/` — the field was fixed and the \
         entry should have been deleted with it:\n{stale:#?}",
    );
}
