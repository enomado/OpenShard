//! What a quest *is*: the definition a shard writes, and the progress a player
//! makes against it.
//!
//! Shared substrate, not rules. The types here are read by the quest system that
//! offers and turns in a quest, by the gump that draws it, and by the persistence
//! that saves it — so they live below all three, the way [`Region`](crate::Region)
//! lives below the guards that read it.
//!
//! # Definitions come from the pack; progress belongs to the player
//!
//! A [`QuestDef`] is content: a title, some objectives, some rewards. It arrives
//! from the script pack at load time and is replaced wholesale on a reload, so it
//! is never persisted — the pack is the source of truth for what a quest *is*,
//! every boot. A [`QuestState`] is the opposite: it is what one character has
//! done, it is saved with them, and it must survive the pack being edited.
//!
//! That is why a quest is keyed by the pack's **string**, never by its index.
//! Indices are how a saved "you have killed 3 of 5 rats" silently becomes progress
//! on a different quest the day someone reorders the list.
//!
//! The model is ServUO's `BaseQuest`/`BaseObjective`/`BaseReward`, field for
//! field, so that converting real quests later is transcription rather than
//! design.

use std::collections::HashMap;

/// What an objective asks for.
///
/// ServUO's objective classes, as one enum: the concrete list is small, closed,
/// and each variant is read in exactly one place, which is a worse fit for a trait
/// than for a match.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ObjectiveKind {
    /// Kill `count` creatures of a body. ServUO's `SlayObjective`.
    Slay {
        /// The body that counts. Matched against the victim's, so any creature
        /// drawn as this counts.
        body: u16,
    },
    /// Carry `count` of an item at once. ServUO's `ObtainObjective`.
    ///
    /// Counted from the backpack rather than announced, because nothing in the
    /// engine emits an event when an item changes hands — see the diffing pass in
    /// the quest system. Progress therefore goes *down* when the items are dropped,
    /// which is ServUO's behaviour too.
    Obtain {
        /// The item graphic that counts.
        graphic: u16,
    },
    /// Take `count` of an item to a named NPC. ServUO's `DeliverObjective`.
    Deliver {
        /// What to carry.
        graphic: u16,
        /// Who to take it to, by name. A name and not a serial: the destination is
        /// written by the pack before anything has been spawned, and a name still
        /// means the same thing after a restart.
        to: String,
    },
    /// Walk someone to a named region. ServUO's `EscortObjective`.
    Escort {
        /// The destination region's name, as `Regions` knows it.
        region: String,
    },
}

/// One thing a quest asks for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ObjectiveDef {
    /// What it asks.
    pub kind: ObjectiveKind,
    /// How many. At least 1; an objective asking for none is complete on sight.
    pub count: u16,
    /// What to call the thing, in the gump — "sewer rat", "spiders' silk".
    pub name: String,
    /// How long the player has, in seconds. `0` is untimed, which is the norm.
    pub seconds: u32,
}

impl ObjectiveDef {
    /// Whether this objective runs against a clock.
    #[must_use]
    pub const fn timed(&self) -> bool {
        self.seconds > 0
    }
}

/// What a quest pays.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RewardKind {
    /// Coins into the backpack.
    Gold(u32),
    /// An item into the backpack.
    Item {
        /// Its graphic.
        graphic: u16,
        /// Its hue, or 0.
        hue: u16,
        /// How many.
        amount: u16,
        /// Whether it merges onto a like pile.
        stackable: bool,
    },
}

/// One reward, with the name the gump shows for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RewardDef {
    /// What the player gets.
    pub kind: RewardKind,
    /// What to call it in the rewards page.
    pub name: String,
}

/// A quest, as the pack defines it.
///
/// The text fields are ServUO's, and each is shown at exactly one moment:
/// `description` when the quest is offered and in the log, `refuse` when it is
/// turned down, `uncomplete` when the giver is talked to before it is finished,
/// `complete` at turn-in, `failed` when a timer runs out.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QuestDef {
    /// The pack's id for it, and the key a player's progress is saved under.
    pub key: String,
    /// The quest's name, in the log and the offer.
    pub title: String,
    /// What it asks, in prose.
    pub description: String,
    /// What the giver says when the offer is refused.
    pub refuse: String,
    /// What the giver says when the quest is in progress but not done.
    pub uncomplete: String,
    /// What the giver says on turn-in.
    pub complete: String,
    /// What is said when a timed objective runs out.
    pub failed: String,
    /// What it asks for.
    pub objectives: Vec<ObjectiveDef>,
    /// What it pays.
    pub rewards: Vec<RewardDef>,
    /// Whether *every* objective must be met (ServUO's `AllObjectives`), or any
    /// one of them is enough.
    pub all_objectives: bool,
    /// Whether it can only ever be done once by a character.
    pub done_once: bool,
    /// How long before it may be taken again, in seconds. `0` is immediately —
    /// unless [`done_once`](Self::done_once), which outranks it.
    pub restart_delay_secs: u32,
}

impl Default for QuestDef {
    fn default() -> Self {
        Self {
            key: String::new(),
            title: String::new(),
            description: String::new(),
            refuse: String::new(),
            uncomplete: String::new(),
            complete: String::new(),
            failed: String::new(),
            objectives: Vec::new(),
            rewards: Vec::new(),
            // ServUO's default: a quest asks for everything on its list.
            all_objectives: true,
            done_once: false,
            restart_delay_secs: 0,
        }
    }
}

/// Every quest this shard knows, by key.
///
/// Replaced wholesale by the pack — there is no "add one" — because a hot reload
/// re-runs the pack's registration from the top, and merging would leave a quest
/// the pack has deleted still on offer.
#[derive(Clone, Default, Debug)]
pub struct QuestDefs {
    defs: Vec<QuestDef>,
    by_key: HashMap<String, usize>,
}

impl QuestDefs {
    /// Replace every definition with `defs`.
    ///
    /// A duplicate key keeps the *last* one, so a pack that redefines a quest
    /// later in its load order wins — the same rule a redefined function follows
    /// in the script itself.
    pub fn set(&mut self, defs: Vec<QuestDef>) {
        self.by_key = defs
            .iter()
            .enumerate()
            .map(|(index, def)| (def.key.clone(), index))
            .collect();
        self.defs = defs;
    }

    /// The definition for a key, if the pack still defines it.
    ///
    /// `None` is an ordinary answer, not a fault: a saved quest whose definition
    /// the pack has since removed reads as `None`, and every caller treats that as
    /// "this quest no longer exists" rather than failing.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&QuestDef> {
        self.by_key.get(key).and_then(|&index| self.defs.get(index))
    }

    /// Whether any quest is defined at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// How many are defined.
    #[must_use]
    pub fn len(&self) -> usize {
        self.defs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quest(key: &str, title: &str) -> QuestDef {
        QuestDef {
            key: key.to_owned(),
            title: title.to_owned(),
            ..QuestDef::default()
        }
    }

    #[test]
    fn a_registration_replaces_everything_before_it() {
        let mut defs = QuestDefs::default();
        defs.set(vec![quest("rat_cull", "A Plague of Rats")]);
        defs.set(vec![quest("silk_gather", "Silk for the Spellwright")]);
        assert!(
            defs.get("rat_cull").is_none(),
            "a quest the pack no longer defines must stop being offered"
        );
        assert_eq!(defs.len(), 1);
    }

    #[test]
    fn a_repeated_key_keeps_the_last_definition() {
        let mut defs = QuestDefs::default();
        defs.set(vec![quest("rat_cull", "Old"), quest("rat_cull", "New")]);
        assert_eq!(defs.get("rat_cull").unwrap().title, "New");
    }

    #[test]
    fn an_unknown_key_is_an_answer_not_a_fault() {
        let defs = QuestDefs::default();
        assert!(defs.get("no_such_quest").is_none());
    }
}
