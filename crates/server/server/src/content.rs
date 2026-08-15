//! The shard's own gameplay content, laid down at boot.
//!
//! Content — quests, regions, spawns, decoration — is data in the domain crates,
//! compiled by their `build.rs` (see `docs/architecture.md` § "A big table is
//! data"). Somebody still has to hand it to the world, and that is a wiring job
//! between a crate that holds the data and a crate that owns the tick, which is
//! what the server is for. So this module is the counterpart of
//! [`scripting`](crate::scripting): the same `Command`s, from the tree instead of
//! from a script.
//!
//! # Why the commands are returned rather than applied
//!
//! [`boot`] hands back a `Vec<Command>` and queues nothing itself. That is what
//! makes the equivalence test below possible at all — the migration off the
//! script pack is only finished when both sources produce the *same* commands,
//! and comparing two worlds after the fact would compare everything that ever
//! happened to them instead. One list against another is the check that means
//! something.
//!
//! # Two ways in, and which dataset takes which
//!
//! [`boot`] is for content that is simply *true* of the shard: quests and
//! townsfolk speech are registered unconditionally, before the first tick.
//!
//! [`verb`] is for content an operator lays and clears by hand — the staff
//! menu's `regions:felucca` and the `--seed` argument that sends the same string
//! without a client attached. `world::admin` owns the buttons; this owns what
//! each one means, now that the answer is in the tree rather than in a pack's
//! `onEvent`.
//!
//! Both return commands and queue nothing, for the reason above. Spawns and
//! decoration are verbs too and will land in [`verb`] beside regions.

use openshard_state::{dialogue, quest, region};
use openshard_world::Command;

/// Every command the shard's own content lays down, before the first tick.
///
/// Called after the world is restored, so it can never overwrite a save it has
/// not seen; and before the first tick, so a player entering on tick one finds a
/// world that is already furnished.
///
/// # The pack still wins, while there is one
///
/// A configured script pack registers its own quests and speech on the tick after
/// this, and both destinations replace wholesale —
/// [`QuestDefs::set`](openshard_state::quest::QuestDefs::set) and
/// [`Dialogue::set_tables`](openshard_state::Dialogue::set_tables) — so a pack
/// that defines either silently overrides these for exactly as long as the pack
/// exists. That is deliberate for the length of the migration: nothing is deleted
/// until the equivalence test says the two agree.
#[must_use]
pub fn boot() -> Vec<Command> {
    vec![
        Command::RegisterQuests {
            quests: quest::shipped(),
        },
        Command::RegisterNpcSpeech {
            trades: dialogue::shipped(),
        },
    ]
}

/// What an admin verb lays down — the staff menu's buttons, and `--seed`.
///
/// Empty for a verb the tree has no data for, which is not an error: the menu
/// still offers `populate:felucca` and `decorate:felucca`, and until those
/// datasets move a configured pack is what answers them. An unknown string is
/// the same case and needs no separate arm.
///
/// # Why the verb is in the data
///
/// The set carries its own verb ([`RegionSet::verb`](openshard_state::region::RegionSet)),
/// so this is a lookup rather than a `match`. A `match` here would be a second
/// list to keep level with `world::admin`'s `ROWS`, and the failure when they
/// drifted would be a button that silently does nothing.
///
/// # Laying twice is safe *here*, and will not be everywhere
///
/// `Regions::set` replaces the facet's whole list, so pressing the button twice
/// leaves one set of regions. Decoration does not have that property — it is
/// additive and persisted — so the dataset that lands in this function next
/// brings the idempotency question with it. This one does not answer it.
#[must_use]
pub fn verb(action: &str) -> Vec<Command> {
    region::shipped()
        .into_iter()
        .filter(|set| set.verb == action)
        .map(|set| Command::RegisterRegions {
            facet: set.facet,
            regions: set.regions,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What CI gets, since the equivalence test below skips without the pack:
    /// the shard's content reaches the world as one registration carrying every
    /// quest the tree ships.
    ///
    /// Where it stops is deliberate. `WorldState` is not reachable from this
    /// crate — the server drives the world through commands and reads it through
    /// events, and widening that for a test would cost more than the test is
    /// worth. What `RegisterQuests` then *does* is `world`'s own to prove, and it
    /// does (`tick/quest_tests.rs`); what is unproven anywhere else, and proven
    /// here, is that `boot` emits it and emits all of it.
    #[test]
    fn boot_hands_the_world_every_dataset_the_tree_ships() {
        let quests = quest::shipped();
        let trades = dialogue::shipped();
        assert!(!quests.is_empty(), "the shard ships no quests at all");
        assert!(!trades.is_empty(), "the shard ships no trade speech at all");

        assert_eq!(
            boot(),
            vec![
                Command::RegisterQuests { quests },
                Command::RegisterNpcSpeech { trades },
            ],
            "the tree's content is not reaching the world intact"
        );
    }

    #[test]
    fn the_staff_menus_region_button_lays_the_regions_the_tree_ships() {
        // The verb string is the whole of the contract between `world::admin`'s
        // button and this module, and neither side can check the other at compile
        // time. If they drift, the button silently lays nothing.
        let commands = verb("regions:felucca");
        assert_eq!(commands.len(), 1, "the region button lays nothing");
        let Command::RegisterRegions { facet, regions } = &commands[0] else {
            panic!("the region verb laid something other than regions: {commands:?}");
        };
        assert_eq!(*facet, openshard_protocol::world::Facet(0));
        assert!(regions.len() > 100, "only {} regions on Felucca", regions.len());
        assert!(
            regions.iter().any(|region| region.flags.guarded),
            "no region on Felucca is guarded, so no guard will ever answer"
        );
    }

    #[test]
    fn a_verb_the_tree_has_no_content_for_lays_nothing() {
        // Not an error and not a panic: the menu still offers the two verbs whose
        // data is in the pack, and a shard with a pack configured is how they get
        // answered until PR4 and PR5.
        assert!(verb("populate:felucca").is_empty());
        assert!(verb("decorate:felucca").is_empty());
        assert!(verb("").is_empty());
        assert!(verb("regions:trammel").is_empty());
    }

    /// The migration's one load-bearing test: what the tree lays down and what
    /// the community pack lays down have to be the same commands.
    ///
    /// It needs the pack, which is a separate repository and not a dependency, so
    /// it strikes unless `OPENSHARD_PACK` names it — the same bargain the
    /// client-file tests make with `OPENSHARD_CLIENT`. Every dataset that moves
    /// in-tree extends this test; when the last one has, the pack is deletable
    /// and this test goes with it.
    ///
    /// ```sh
    /// OPENSHARD_PACK=../OpenShard-Community-Pack cargo test -p openshard-server content
    /// ```
    ///
    /// Async only to have a tokio runtime in scope, which is where the shard
    /// builds its engine too — `run_shard` is async, so `Scripts::load` always
    /// has one. V8 aborts the process rather than degrading when it posts a
    /// delayed task and finds no runtime, and a pack this size posts one.
    #[tokio::test]
    async fn the_tree_registers_what_the_pack_did() {
        let Some(pack) = std::env::var_os("OPENSHARD_PACK") else {
            return;
        };

        // The pack registers at *load*: `op_register_quests` is a top-level call,
        // so the commands are in the engine's outbox before any tick has run.
        let mut engine = openshard_scripting::DenoEngine::new();
        engine
            .load_file(&pack)
            .unwrap_or_else(|e| panic!("reading the pack at {pack:?}: {e}"))
            .unwrap_or_else(|e| panic!("loading the pack at {pack:?}: {e}"));

        // Through the same bridge the running shard uses, so this compares the
        // two sources and not two spellings of the conversion.
        let from_pack: Vec<Command> = openshard_scripting::ScriptEngine::take_commands(&mut engine)
            .into_iter()
            .filter_map(crate::scripting::into_world)
            .collect();
        let from_tree = boot();

        // A dataset at a time, so a failure names the one that diverged. Each
        // emptiness check earns its place: a pack path that points somewhere
        // harmless, or a load that registered nothing, would otherwise pass on two
        // empty lists and read as agreement. They are separate blocks rather than
        // a loop because the two comparisons are over different types, and giving
        // them a common one would mean comparing debug strings.
        let (tree_quests, pack_quests) = (only_quests(from_tree.clone()), only_quests(from_pack.clone()));
        assert!(
            !pack_quests.is_empty(),
            "the pack at {pack:?} registered no quests at all; \
             OPENSHARD_PACK should name the pack's directory"
        );
        assert_eq!(
            tree_quests, pack_quests,
            "in-tree quests and the pack's have diverged; the migration is not done"
        );

        let (tree_speech, pack_speech) = (only_speech(from_tree), only_speech(from_pack));
        assert!(
            !pack_speech.is_empty(),
            "the pack at {pack:?} registered no trade speech at all"
        );
        assert_eq!(
            tree_speech, pack_speech,
            "in-tree speech and the pack's have diverged; the migration is not done"
        );

        // Regions are the first dataset that does not arrive at load: both sides
        // answer a *verb*, so the pack has to be sent one. This is the shape every
        // later verb-keyed dataset repeats — press the button on the pack, press
        // it on the tree, compare.
        let verb_pressed = "regions:felucca";
        openshard_scripting::ScriptEngine::deliver(
            &mut engine,
            &openshard_scripting::Event::AdminAction {
                serial: None,
                action: verb_pressed.to_owned(),
            },
        )
        .expect("the pack's onEvent refused the region verb");
        let pack_verb: Vec<Command> = openshard_scripting::ScriptEngine::take_commands(&mut engine)
            .into_iter()
            .filter_map(crate::scripting::into_world)
            .collect();

        let (tree_regions, pack_regions) = (only_regions(verb(verb_pressed)), only_regions(pack_verb));
        assert!(
            !pack_regions.is_empty(),
            "the pack at {pack:?} laid no regions for {verb_pressed:?}"
        );
        assert_eq!(
            tree_regions, pack_regions,
            "in-tree regions and the pack's have diverged; the migration is not done"
        );
    }

    /// The region registrations out of a command stream, by facet.
    ///
    /// **Not sorted, unlike the other two.** A region's position in the list *is*
    /// its id — `Regions::set` numbers them by index, and that number is what a
    /// save and the wire carry — so two orders are two different worlds, and a
    /// sort here would hide the one difference that matters most.
    fn only_regions(
        commands: Vec<Command>,
    ) -> Vec<(openshard_protocol::world::Facet, Vec<openshard_state::Region>)> {
        commands
            .into_iter()
            .filter_map(|command| match command {
                Command::RegisterRegions { facet, regions } => Some((facet, regions)),
                _ => None,
            })
            .collect()
    }

    /// The speech registrations out of a command stream, each one's trades sorted
    /// by title.
    ///
    /// Sorted for [`only_quests`]' reason — the destination is a `HashMap` and the
    /// two sources owe each other no order. **Only the outer list.** A table's
    /// `entries` are in precedence order, the first match wins, and sorting them
    /// would hide exactly the difference that changes what an NPC answers.
    fn only_speech(commands: Vec<Command>) -> Vec<Vec<(String, openshard_state::SpeechTable)>> {
        commands
            .into_iter()
            .filter_map(|command| match command {
                Command::RegisterNpcSpeech { mut trades } => {
                    trades.sort_by(|a, b| a.0.cmp(&b.0));
                    Some(trades)
                }
                _ => None,
            })
            .collect()
    }

    /// The quest registrations out of a command stream, each one's quests sorted
    /// by key.
    ///
    /// Sorted because the two sources are under no obligation to agree on order
    /// and nothing downstream reads one: `QuestDefs` is a lookup by key. An order
    /// difference reported as a failure here would be a false alarm that stops
    /// the migration on a fact about the file, not about the shard.
    fn only_quests(commands: Vec<Command>) -> Vec<Vec<openshard_state::quest::QuestDef>> {
        commands
            .into_iter()
            .filter_map(|command| match command {
                Command::RegisterQuests { mut quests } => {
                    quests.sort_by(|a, b| a.key.cmp(&b.key));
                    Some(quests)
                }
                _ => None,
            })
            .collect()
    }
}
