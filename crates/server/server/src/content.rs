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
/// # A verb may have more than one answer, and `populate:` does
///
/// The spawn *regions* are here; the 789 standing townsfolk that ride on the
/// same verb in the pack's data are not, and become in-tree spawn data of their
/// own later. Both sides answering the same verb is fine — the world applies
/// what each lays, and `register_spawner` de-duplicates by `SpawnArea`, so the
/// pack's copy of a region the tree already laid is dropped rather than stacked.
///
/// # Laying twice is safe *here*, and will not be everywhere
///
/// `Regions::set` replaces the facet's whole list and `register_spawner`
/// de-duplicates by area, so pressing either button twice leaves one of each.
/// Decoration does not have that property — it is additive and persisted — so
/// the dataset that lands in this function next brings the idempotency question
/// with it. Neither of these answers it.
#[must_use]
pub fn verb(action: &str) -> Vec<Command> {
    let regions = region::shipped()
        .into_iter()
        .filter(|set| set.verb == action)
        .map(|set| Command::RegisterRegions {
            facet: set.facet,
            regions: set.regions,
        });
    let spawners = openshard_world::spawner::shipped()
        .into_iter()
        .filter(|set| set.verb == action)
        .flat_map(|set| set.spawners)
        .map(|spawner| Command::RegisterSpawner { spawner });
    // Decoration, then the door generation that reads it. The order is the pack's
    // and it is load-bearing: a generated door goes in the gap between two static
    // frames, and some of those frames are laid by the batch above.
    let decor = openshard_world::decoration::shipped()
        .into_iter()
        .filter(|set| set.verb == action)
        .flat_map(|set| {
            let batch = Command::Decorate {
                facet: set.facet,
                statics: set.statics.to_vec(),
                doors: set.doors.to_vec(),
                containers: set.containers.to_vec(),
            };
            let scans = set
                .door_regions
                .iter()
                .map(move |&(x, y, width, height)| Command::GenerateDoors {
                    facet: set.facet,
                    x,
                    y,
                    width,
                    height,
                });
            std::iter::once(batch).chain(scans)
        });
    regions.chain(spawners).chain(decor).collect()
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
    fn the_staff_menus_populate_button_lays_every_spawn_region_the_tree_ships() {
        let commands = verb("populate:felucca");
        let spawners = only_spawners(commands);
        assert!(
            spawners.len() > 1000,
            "only {} spawn regions on Felucca",
            spawners.len()
        );
        // Every one comes out with the placeholder id and no timer: both belong to
        // the live spawner, and `register_spawner` sets them. A number written into
        // the data would be a second source for either.
        assert!(
            spawners.iter().all(|s| s.id == 0 && s.next_spawn == 0),
            "a shipped spawn region arrived with an id or a timer already set"
        );
        assert!(
            spawners
                .iter()
                .all(|s| !s.creatures.is_empty() && s.max_count > 0),
            "a shipped spawn region can never put anything down"
        );
    }

    #[test]
    fn the_staff_menus_decorate_button_lays_the_art_then_scans_for_doors() {
        // The order is the whole of it. A generated door goes in the gap between
        // two static frames, and some of those frames are in the batch above it —
        // so a scan that ran first would find a doorway that is not there yet.
        let commands = verb("decorate:felucca");
        let Some(Command::Decorate {
            statics,
            doors,
            containers,
            ..
        }) = commands.first()
        else {
            panic!(
                "the decorate verb does not lay decoration first: {:?}",
                commands.first()
            );
        };
        assert!(statics.len() > 10_000, "only {} statics", statics.len());
        assert!(!doors.is_empty() && !containers.is_empty());
        assert!(
            commands[1..]
                .iter()
                .all(|c| matches!(c, Command::GenerateDoors { .. })),
            "something other than a door scan followed the decoration"
        );
        assert!(commands.len() > 1, "no region is scanned for implied doors");
    }

    #[test]
    fn a_verb_the_tree_has_no_content_for_lays_nothing() {
        // Not an error and not a panic: an unknown verb is what a pack that
        // dropped a set would produce, and the engine has never treated it as a
        // failure.
        assert!(verb("").is_empty());
        assert!(verb("regions:trammel").is_empty());
        assert!(verb("populate:trammel").is_empty());
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

        let populate = "populate:felucca";
        openshard_scripting::ScriptEngine::deliver(
            &mut engine,
            &openshard_scripting::Event::AdminAction {
                serial: None,
                action: populate.to_owned(),
            },
        )
        .expect("the pack's onEvent refused the populate verb");
        let pack_populate: Vec<Command> = openshard_scripting::ScriptEngine::take_commands(&mut engine)
            .into_iter()
            .filter_map(crate::scripting::into_world)
            .collect();

        let (tree_spawners, pack_spawners) = (only_spawners(verb(populate)), only_spawners(pack_populate));
        assert!(
            !pack_spawners.is_empty(),
            "the pack at {pack:?} laid no spawn regions for {populate:?}"
        );
        compare_spawners(&tree_spawners, &pack_spawners);

        let decorate = "decorate:felucca";
        openshard_scripting::ScriptEngine::deliver(
            &mut engine,
            &openshard_scripting::Event::AdminAction {
                serial: None,
                action: decorate.to_owned(),
            },
        )
        .expect("the pack's onEvent refused the decorate verb");
        let pack_decorate: Vec<Command> = openshard_scripting::ScriptEngine::take_commands(&mut engine)
            .into_iter()
            .filter_map(crate::scripting::into_world)
            .collect();
        compare_decoration(&verb(decorate), &pack_decorate);
    }

    /// One `Command::Decorate`'s payload, pulled apart for comparison.
    type DecorBatch = (
        openshard_protocol::world::Facet,
        Vec<(
            openshard_protocol::wire::Graphic,
            openshard_protocol::wire::Hue,
            openshard_protocol::world::Point,
        )>,
        Vec<openshard_world::DecorDoor>,
        Vec<openshard_world::DecorContainer>,
    );

    /// Two decoration command streams, reported piece by piece.
    ///
    /// [`compare_spawners`]' argument one order of magnitude further along: this
    /// is twenty-five thousand rows, and an `assert_eq!` over it is a `Debug`
    /// string no terminal will show and no person will read.
    fn compare_decoration(tree: &[Command], pack: &[Command]) {
        fn batch(commands: &[Command]) -> DecorBatch {
            commands
                .iter()
                .find_map(|c| match c {
                    Command::Decorate {
                        facet,
                        statics,
                        doors,
                        containers,
                    } => Some((*facet, statics.clone(), doors.clone(), containers.clone())),
                    _ => None,
                })
                .expect("no decoration in the stream")
        }
        fn scans(commands: &[Command]) -> Vec<Command> {
            commands
                .iter()
                .filter(|c| matches!(c, Command::GenerateDoors { .. }))
                .cloned()
                .collect()
        }

        let (tree_facet, tree_statics, tree_doors, tree_containers) = batch(tree);
        let (pack_facet, pack_statics, pack_doors, pack_containers) = batch(pack);
        assert_eq!(tree_facet, pack_facet, "the two decorate different facets");
        assert_eq!(
            (tree_statics.len(), tree_doors.len(), tree_containers.len()),
            (pack_statics.len(), pack_doors.len(), pack_containers.len()),
            "the tree lays {} statics, {} doors and {} containers; the pack lays {}, {} and {}",
            tree_statics.len(),
            tree_doors.len(),
            tree_containers.len(),
            pack_statics.len(),
            pack_doors.len(),
            pack_containers.len(),
        );

        if let Some(i) = (0..tree_statics.len()).find(|&i| tree_statics[i] != pack_statics[i]) {
            panic!(
                "static {i} differs\n  tree: {:?}\n  pack: {:?}",
                tree_statics[i], pack_statics[i]
            );
        }
        if let Some(i) = (0..tree_doors.len()).find(|&i| tree_doors[i] != pack_doors[i]) {
            panic!(
                "door {i} differs\n  tree: {:?}\n  pack: {:?}",
                tree_doors[i], pack_doors[i]
            );
        }
        if let Some(i) = (0..tree_containers.len()).find(|&i| tree_containers[i] != pack_containers[i]) {
            panic!(
                "container {i} differs\n  tree: {:?}\n  pack: {:?}",
                tree_containers[i], pack_containers[i]
            );
        }
        // Small enough to compare whole, and worth it: a missing scan box is a
        // district's worth of shop doors that never appear.
        assert_eq!(scans(tree), scans(pack), "the door-generation boxes differ");
    }

    /// Two spawner lists, reported item by item.
    ///
    /// **Not `assert_eq!`.** A thousand spawn regions holding eight thousand
    /// creatures between them is a five-megabyte `Debug` dump on failure, which is
    /// not a diff — it is a wall. This says how many differ and shows the first,
    /// which is what a person actually reads. Decoration is twenty times this and
    /// will want the same.
    fn compare_spawners(
        tree: &[openshard_world::spawner::Spawner],
        pack: &[openshard_world::spawner::Spawner],
    ) {
        assert_eq!(
            tree.len(),
            pack.len(),
            "the tree lays {} spawn regions and the pack lays {}",
            tree.len(),
            pack.len()
        );
        let differing: Vec<usize> = (0..tree.len()).filter(|&i| tree[i] != pack[i]).collect();
        let Some(&first) = differing.first() else {
            return;
        };
        let (a, b) = (&tree[first], &pack[first]);
        assert_eq!(
            a.area,
            b.area,
            "spawn region {first} of {} covers a different box in the tree than in the pack",
            differing.len()
        );
        assert_eq!(
            a.creatures.len(),
            b.creatures.len(),
            "spawn region {first} at {:?} holds {} creatures in the tree and {} in the pack",
            a.area,
            a.creatures.len(),
            b.creatures.len()
        );
        let creature = (0..a.creatures.len())
            .find(|&i| a.creatures[i] != b.creatures[i])
            .map(|i| format!("\n  tree: {:?}\n  pack: {:?}", a.creatures[i], b.creatures[i]))
            .unwrap_or_default();
        panic!(
            "{} of {} spawn regions differ; the first is {first} at {:?}\
             \n  max_count {} vs {}, respawn_delay {} vs {}{creature}",
            differing.len(),
            tree.len(),
            a.area,
            a.max_count,
            b.max_count,
            a.respawn_delay,
            b.respawn_delay,
        );
    }

    /// The spawn regions out of a command stream, in the order they were laid.
    ///
    /// Order is kept for `only_regions`' reason turned around: it does *not*
    /// matter — `register_spawner` assigns the id and de-duplicates by
    /// `SpawnArea` — but the two sources build the list from the same file order
    /// anyway, so a difference here is a real one rather than noise.
    fn only_spawners(commands: Vec<Command>) -> Vec<openshard_world::spawner::Spawner> {
        commands
            .into_iter()
            .filter_map(|command| match command {
                Command::RegisterSpawner { spawner } => Some(spawner),
                _ => None,
            })
            .collect()
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
