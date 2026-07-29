//! What an item *is*, by its graphic, the moment one is made.
//!
//! A few core graphics carry state that nothing else can supply: an instrument has
//! a number of tunes left in it, a pickaxe a number of swings, and a bottle of
//! poison a poison. None can come from a table at read time — a half-used lute and
//! a fresh one share a graphic — so it has to be put on the item when the item is
//! created, and this is the one place that happens.
//!
//! Without it a shop's instruments are silent props and its poison potions are
//! empty glass: the Community Pack's vendors already stock both (the converter
//! reads ServUO's own `SB*.cs`), and the skills that use them would refuse every
//! one.

use openshard_state::WorldState;
use openshard_state::components::{
    Instrument, Name, POISON_POTION_GRAPHIC, PoisonCharges, RUNEBOOK_GRAPHIC, Runebook, Tool,
};
use openshard_state::craft::craft_tool;
use openshard_state::harvest::tool_data;
use openshard_state::instrument::{INSTRUMENT_MAX_USES, INSTRUMENT_MIN_USES, instrument_data};

use openshard_entities::{EntityId, Registry};

/// Give a freshly made item whatever its graphic implies.
///
/// Called from every place an item is created from a graphic — a vendor's shelf, a
/// script's spawn, a staff `.add` — so there is no path that makes a lute nobody
/// can play.
pub fn apply_core_defaults(state: &mut WorldState, item: EntityId, graphic: u16) {
    if instrument_data(graphic).is_some() {
        // Rolled between ServUO's two bounds on the world's own generator, so a
        // shelf of lutes replays and no two are identical.
        let span = u32::from(INSTRUMENT_MAX_USES - INSTRUMENT_MIN_USES) + 1;
        let uses_left = INSTRUMENT_MIN_USES + u16::try_from(state.rng.below(span)).unwrap_or(0);
        state.registry.insert(item, Instrument { uses_left });
        return;
    }
    if let Some(data) = tool_data(graphic) {
        // A pickaxe off the shelf holds a fixed number of swings, rolled the same
        // way — without it a bought pick has no `Tool` at all and the first swing
        // never wears it down.
        let span = u32::from(data.max_uses - data.min_uses) + 1;
        let uses_left = data.min_uses + u16::try_from(state.rng.below(span)).unwrap_or(0);
        state.registry.insert(item, Tool { uses_left });
        return;
    }
    if let Some(data) = craft_tool(graphic) {
        // The same for a smith's tongs or a tailor's sewing kit. The two tables
        // are separate because a pickaxe and a saw answer different questions —
        // which ground, which trade — and one of them ends up on `Skill` either
        // way, so merging them would only hide the difference.
        let span = u32::from(data.max_uses - data.min_uses) + 1;
        let uses_left = data.min_uses + u16::try_from(state.rng.below(span)).unwrap_or(0);
        state.registry.insert(item, Tool { uses_left });
        return;
    }
    if graphic == POISON_POTION_GRAPHIC {
        let level = poison_level_of(state, item);
        state.registry.insert(item, PoisonCharges { level, charges: 1 });
        return;
    }
    if graphic == RUNEBOOK_GRAPHIC {
        // An empty book with its charges. A bought one is blank — the
        // destinations are the owner's to bind — but it has to *be* a runebook
        // from the moment it exists, or a book off a shelf is a graphic that
        // refuses to open, which is the bug the spellbook mask once had.
        state.registry.insert(
            item,
            Runebook {
                charges: SHELF_RUNEBOOK_CHARGES,
                max_charges: SHELF_RUNEBOOK_CHARGES,
                ..Runebook::default()
            },
        );
    }
}

/// The charges a runebook that nobody crafted comes with — ServUO's own
/// constructor default, which its vendors and its loot both use. A crafted one
/// gets `5 + quality + Inscribe/30` instead, so the scribe's work shows.
const SHELF_RUNEBOOK_CHARGES: u8 = 6;

/// Put a saved use count back on an item, as whichever of the two kinds its
/// graphic says it is.
///
/// Both an instrument and a harvesting tool are ServUO's `IUsesRemaining` and both
/// ride the same saved column, so the graphic is what decides which component it
/// comes back as. Restoring the wrong one would leave a pickaxe that plays music.
pub fn restore_uses(state: &mut WorldState, item: EntityId, graphic: u16, uses_left: u16) {
    if instrument_data(graphic).is_some() {
        state.registry.insert(item, Instrument { uses_left });
    } else if tool_data(graphic).is_some() || craft_tool(graphic).is_some() {
        state.registry.insert(item, Tool { uses_left });
    }
}

/// How many uses an item has left, whichever of the two kinds it is — what the
/// save sweep writes.
///
/// Takes the registry rather than the world because that is all the save sweep
/// has: `item_record` runs over a borrowed `&Registry` while the rest of the world
/// is being read beside it.
#[must_use]
pub fn uses_left(registry: &Registry, item: EntityId) -> Option<u16> {
    if let Some(instrument) = registry.get::<Instrument>(item) {
        return Some(instrument.uses_left);
    }
    registry.get::<Tool>(item).map(|tool| tool.uses_left)
}

/// Which poison a bottle holds, read off its label.
///
/// All four strengths are the same graphic (`0x0F0A`), so the graphic cannot say —
/// but the *name* can, and a stocked bottle has one, because the converter carries
/// ServUO's own item labels through. A bottle with no label is the middling one,
/// which is what "a poison potion" means when nobody says otherwise.
fn poison_level_of(state: &WorldState, item: EntityId) -> u8 {
    let Some(name) = state.registry.get::<Name>(item) else {
        return 1;
    };
    let name = name.0.to_lowercase();
    if name.contains("lesser") {
        0
    } else if name.contains("greater") {
        2
    } else if name.contains("deadly") {
        3
    } else if name.contains("lethal") {
        4
    } else {
        1
    }
}
