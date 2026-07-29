//! The five trades, and the constants that are not in their recipe lists.
//!
//! Each `Def*.cs` in ServUO is a recipe table plus a short header of overrides —
//! the main skill, the chance floor, the exceptional curve, the sound, and what
//! `CanCraft` demands. The tables are generated (`tools/gen-craft-tables`); this
//! file is the headers, hand-written, because they are five values apiece and
//! they are the ones worth arguing about.

pub mod alchemy;
pub mod blacksmithy;
pub mod carpentry;
pub mod tailoring;
pub mod tinkering;

use openshard_state::{Skill, TICKS_PER_SECOND};

use crate::system::{CraftSystemDef, Eca, Needs, SystemId, Text};

/// ServUO's `Delay`, 1.25 seconds, which every one of the five passes to
/// `base(1, 1, 1.25)`. In ticks, because a craft is beaten down by the tick
/// counter like decay and a swing timer.
const DELAY_TICKS: u64 = TICKS_PER_SECOND * 5 / 4;

/// "You must be near an anvil and a forge to smith items."
const NEEDS_SMITHY: u32 = 1_044_267;

/// The trades a shard can practise, in the order their ids are numbered.
///
/// The index into this table is a [`SystemId`] — it rides in a `Crafting`
/// component and, once crafting is saved mid-flight, in a record. **Append, never
/// reorder.**
pub const SYSTEMS: &[CraftSystemDef] = &[
    // Blacksmithy. The chance floor is zero, so a recipe you have only just
    // qualified for always fails at first; the exceptional curve is the sliding
    // one, which is what makes the last five points of the skill worth having.
    CraftSystemDef {
        skill: Skill::Blacksmith,
        title: Text::Cliloc(1_044_002),
        chance_at_min: 0,
        eca: Eca::ChanceMinusSixtyToFourtyFive,
        delay_ticks: DELAY_TICKS,
        min_beats: 1,
        max_beats: 1,
        craft_sound: 0x2A,
        needs: Needs::smithy(),
        needs_message: NEEDS_SMITHY,
        groups: blacksmithy::GROUPS,
        recipes: blacksmithy::RECIPES,
        sub_res: Some(blacksmithy::SUB_RES),
    },
    // Tailoring. Half of everything a tailor qualifies for works first time,
    // which is ServUO's `GetChanceAtMin` of 0.5 and the difference between a
    // trade that is pleasant to learn and one that is not.
    CraftSystemDef {
        skill: Skill::Tailoring,
        title: Text::Cliloc(1_044_005),
        chance_at_min: 500,
        eca: Eca::ChanceMinusSixtyToFourtyFive,
        delay_ticks: DELAY_TICKS,
        min_beats: 1,
        max_beats: 1,
        craft_sound: 0x248,
        needs: Needs::none(),
        needs_message: 0,
        groups: tailoring::GROUPS,
        recipes: tailoring::RECIPES,
        sub_res: Some(tailoring::SUB_RES),
    },
    // Carpentry, the same floor as tailoring.
    CraftSystemDef {
        skill: Skill::Carpentry,
        title: Text::Cliloc(1_044_004),
        chance_at_min: 500,
        eca: Eca::ChanceMinusSixtyToFourtyFive,
        delay_ticks: DELAY_TICKS,
        min_beats: 1,
        max_beats: 1,
        craft_sound: 0x23D,
        needs: Needs::none(),
        needs_message: 0,
        groups: carpentry::GROUPS,
        recipes: carpentry::RECIPES,
        sub_res: Some(carpentry::SUB_RES),
    },
    // Tinkering: a smith's floor with a carpenter's freedom to work anywhere.
    CraftSystemDef {
        skill: Skill::Tinkering,
        title: Text::Cliloc(1_044_007),
        chance_at_min: 0,
        eca: Eca::ChanceMinusSixtyToFourtyFive,
        delay_ticks: DELAY_TICKS,
        min_beats: 1,
        max_beats: 1,
        craft_sound: 0x23B,
        needs: Needs::none(),
        needs_message: 0,
        groups: tinkering::GROUPS,
        recipes: tinkering::RECIPES,
        sub_res: Some(tinkering::SUB_RES),
    },
    // Alchemy is the only one of the five that takes the *default* exceptional
    // curve — it overrides no `ECA` at all — which is why a flat sixty points
    // appears here and nowhere else.
    CraftSystemDef {
        skill: Skill::Alchemy,
        title: Text::Cliloc(1_044_001),
        chance_at_min: 0,
        eca: Eca::ChanceMinusSixty,
        delay_ticks: DELAY_TICKS,
        min_beats: 1,
        max_beats: 1,
        craft_sound: 0x242,
        needs: Needs::none(),
        needs_message: 0,
        groups: alchemy::GROUPS,
        recipes: alchemy::RECIPES,
        sub_res: None,
    },
];

/// One system by id.
#[must_use]
pub fn system(id: SystemId) -> Option<&'static CraftSystemDef> {
    SYSTEMS.get(usize::from(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_trade_has_exactly_one_system() {
        // `tool_system` finds a system by its main skill, so two systems sharing
        // one would make a tool open whichever came first — silently, and only
        // for one of the two trades.
        for (i, def) in SYSTEMS.iter().enumerate() {
            for other in &SYSTEMS[i + 1..] {
                assert_ne!(def.skill, other.skill, "{:?} appears twice", def.skill);
            }
        }
    }

    #[test]
    fn every_tool_on_the_shelf_opens_a_system() {
        // The vendors already stock all five trades' tools. A graphic in the tool
        // table with no system behind it is a tool that answers a double-click
        // with nothing at all, which is what every one of these was before this
        // slice.
        for graphic in 0..=u16::MAX {
            let Some(tool) = openshard_state::craft::craft_tool(graphic) else {
                continue;
            };
            assert!(
                SYSTEMS.iter().any(|def| def.skill == tool.skill),
                "{graphic:#06X} names {:?}, which no system practises",
                tool.skill
            );
        }
    }

    #[test]
    fn every_recipe_names_a_group_that_exists() {
        // Groups are referenced by index and a dropped recipe leaves its group in
        // place, so an out-of-range index would draw an empty category rather
        // than fail — worth an assertion because nothing at runtime notices.
        for def in SYSTEMS {
            for recipe in def.recipes {
                assert!(
                    usize::from(recipe.group) < def.groups.len(),
                    "{:?} recipe {:#06X} is in group {}",
                    def.skill,
                    recipe.graphic,
                    recipe.group
                );
            }
        }
    }

    #[test]
    fn every_recipe_leads_with_its_systems_own_skill() {
        // The success chance is interpolated over the *main* skill's band, and a
        // recipe with no line for it would read as chance zero — a whole trade
        // that silently refuses everything.
        for def in SYSTEMS {
            for recipe in def.recipes {
                assert!(
                    recipe.skills.iter().any(|want| want.skill == def.skill),
                    "{:?} recipe {:#06X} never names {:?}",
                    def.skill,
                    recipe.graphic,
                    def.skill
                );
            }
        }
    }

    #[test]
    fn the_material_axis_substitutes_into_a_line_that_wants_it() {
        // The axis is a hue swap onto one resource line. A system with an axis
        // whose recipes never name its graphic would offer a material picker that
        // changed nothing.
        for def in SYSTEMS {
            let Some(axis) = def.sub_res else { continue };
            assert!(!axis.entries.is_empty());
            assert_eq!(axis.entries[0].hue, 0, "{:?}'s plain grade", def.skill);
            let uses_axis = def
                .recipes
                .iter()
                .filter(|recipe| recipe.resources.iter().any(|res| res.from_axis))
                .count();
            assert!(uses_axis > 0, "{:?} has an axis nothing reads", def.skill);
            for recipe in def.recipes {
                for res in recipe.resources {
                    assert_eq!(
                        res.from_axis,
                        res.graphic == axis.graphic,
                        "{:?} recipe {:#06X}",
                        def.skill,
                        recipe.graphic
                    );
                }
            }
        }
    }

    #[test]
    fn the_metals_are_the_same_hues_the_ground_yields() {
        // Mining pays in ore hues and a smith spends ingot hues; if the two
        // tables ever disagree, valorite ore smelts into an ingot no recipe can
        // find. Both come from ServUO's one `CraftResourceInfo` table, and this
        // is what says so.
        let axis = blacksmithy::SUB_RES;
        let ores = openshard_state::harvest::ORES;
        assert_eq!(axis.entries.len(), ores.len());
        for (entry, ore) in axis.entries.iter().zip(ores) {
            assert_eq!(entry.hue, ore.hue);
        }
    }
}
