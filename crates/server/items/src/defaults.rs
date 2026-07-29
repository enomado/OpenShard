//! What an item *is*, by its graphic, the moment one is made.
//!
//! A few core graphics carry state that nothing else can supply: an instrument has
//! a number of tunes left in it, and a bottle of poison has a poison in it. Neither
//! can come from a table at read time — a half-used lute and a fresh one share a
//! graphic — so it has to be put on the item when the item is created, and this is
//! the one place that happens.
//!
//! Without it a shop's instruments are silent props and its poison potions are
//! empty glass: the Community Pack's vendors already stock both (the converter
//! reads ServUO's own `SB*.cs`), and the skills that use them would refuse every
//! one.

use openshard_state::components::{Instrument, Name, PoisonCharges, POISON_POTION_GRAPHIC};
use openshard_state::instrument::{instrument_data, INSTRUMENT_MAX_USES, INSTRUMENT_MIN_USES};
use openshard_state::WorldState;

use openshard_entities::EntityId;

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
    if graphic == POISON_POTION_GRAPHIC {
        let level = poison_level_of(state, item);
        state
            .registry
            .insert(item, PoisonCharges { level, charges: 1 });
    }
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
