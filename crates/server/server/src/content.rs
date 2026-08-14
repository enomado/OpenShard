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
//! # Where a verb would go
//!
//! Three of the datasets still in the pack — regions, spawns, decoration — are
//! laid by an *admin verb* rather than at boot (`world::admin`'s
//! `populate:felucca` and friends), because an operator lays and clears them by
//! hand. Quests are not: the pack registers them at load, unconditionally, and
//! so do we. The verb-keyed path arrives with the first dataset that needs one,
//! and not before — an empty dispatch written now would be a guess at a shape
//! three datasets have not asked for yet.

use openshard_state::quest;
use openshard_world::Command;

/// Every command the shard's own content lays down, before the first tick.
///
/// Called after the world is restored, so it can never overwrite a save it has
/// not seen; and before the first tick, so a player entering on tick one finds a
/// world that is already furnished.
///
/// # The pack still wins, while there is one
///
/// A configured script pack registers its own quests on the tick after this, and
/// [`QuestDefs::set`](openshard_state::quest::QuestDefs::set) replaces
/// everything before it — so a pack that defines quests silently overrides these
/// for exactly as long as the pack exists. That is deliberate for the length of
/// the migration: nothing is deleted until the equivalence test says the two
/// agree.
#[must_use]
pub fn boot() -> Vec<Command> {
    vec![Command::RegisterQuests {
        quests: quest::shipped(),
    }]
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
    fn boot_hands_the_world_every_quest_the_tree_ships() {
        let shipped = quest::shipped();
        assert!(!shipped.is_empty(), "the shard ships no quests at all");

        let commands = boot();
        assert_eq!(
            commands,
            vec![Command::RegisterQuests { quests: shipped }],
            "the tree's content is not reaching the world intact"
        );
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
    async fn the_tree_registers_the_quests_the_pack_did() {
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
        let from_pack = only_quests(
            openshard_scripting::ScriptEngine::take_commands(&mut engine)
                .into_iter()
                .filter_map(crate::scripting::into_world)
                .collect(),
        );
        let from_tree = only_quests(boot());

        // Without this the test passes on two empty lists — a pack path that
        // points somewhere harmless, or a load that registered nothing, would
        // read as agreement.
        assert!(
            !from_pack.is_empty(),
            "the pack at {pack:?} registered no quests at all; \
             OPENSHARD_PACK should name the pack's directory"
        );
        assert_eq!(
            from_tree, from_pack,
            "in-tree quests and the pack's have diverged; the migration is not done"
        );
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
