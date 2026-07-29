//! Marking a rune, and recalling to what it remembers.
//!
//! The two halves of one fact, and the reason they sit together: what Mark
//! writes is exactly what Recall reads, and a disagreement between them is a
//! rune that says Britain and lands you in a swamp.
//!
//! Both are effects of a spell that has already been paid for, so neither
//! charges anything and neither rolls anything — by the time these run the mana
//! is spent, the reagents are gone and the skill check has passed. What is left
//! is whether the *world* allows it, which is `magic::may_travel` plus the
//! handful of things ServUO checks at the moment of arrival.

use super::*;
use openshard_state::components::{
    Combat, Contained, CriminalUntil, Equipped, Position, RuneMark, RECALL_RUNE_GRAPHIC,
};

/// The layer a backpack rides on. Mark wants the rune *in* it, which is stricter
/// than being able to reach it.
const BACKPACK_LAYER: u8 = 0x15;

impl World {
    /// The travel family's pre-cast refusals — ServUO's `Recall.CheckCast` and
    /// the identical block in `GateTravel`.
    ///
    /// Run before a single point of mana is spent, which is the whole reason
    /// they are here and not in the effect: a criminal who cannot escape should
    /// find that out for free, not for fourteen mana and three reagents.
    ///
    /// Mark is exempt from all of it — writing a rune is not fleeing.
    pub(super) fn travel_check_cast(
        &self,
        caster: EntityId,
        effect: magic::SpellEffect,
    ) -> Option<&'static str> {
        if !matches!(
            effect,
            magic::SpellEffect::Recall | magic::SpellEffect::GateTravel
        ) {
            return None;
        }
        if self.state.is_staff(caster) {
            return None;
        }
        // A thief does not vanish out of the town they just robbed.
        if self.state.registry.has::<CriminalUntil>(caster) {
            return Some("Thou'rt a criminal and cannot escape so easily.");
        }
        // Nor does anyone mid-fight. ServUO asks whether combat was *recent*; we
        // ask whether it is happening, which is the same question this engine can
        // answer — a `Combat` with a live target is a fight in progress.
        if self
            .state
            .registry
            .get::<Combat>(caster)
            .is_some_and(|combat| combat.target.is_some())
        {
            return Some("Wouldst thou flee during the heat of battle??");
        }
        if openshard_items::is_overloaded(&self.state, caster) {
            return Some("You are too encumbered to move.");
        }
        // Something on the cursor is in neither world once you leave.
        if self
            .connection_of(caster)
            .is_some_and(|connection| self.state.held.contains_key(&connection))
        {
            return Some("You cannot teleport while dragging an object.");
        }
        None
    }

    /// Write the caster's own spot onto a recall rune — the Mark spell.
    pub(super) fn mark_rune(&mut self, caster: EntityId, target_serial: u32) {
        let Some(rune) =
            Serial::new(target_serial).and_then(|serial| self.state.registry.entity_of(serial))
        else {
            return;
        };
        // Only a rune. ServUO says so with 502357 for anything else, and the
        // graphic is not the test: an unmarked rune has no `RuneMark`, so what
        // makes a thing markable is that the engine gave it the graphic.
        if self
            .state
            .registry
            .get::<Graphic>(rune)
            .is_none_or(|graphic| graphic.id != RECALL_RUNE_GRAPHIC)
        {
            self.notify_self(caster, "You cannot mark that.");
            return;
        }
        // In your own pack, not merely within arm's reach — cliloc 1062422. A
        // rune on a shop floor is somebody else's.
        if !self.in_own_backpack(caster, rune) {
            self.notify_self(
                caster,
                "You must have this rune in your backpack in order to mark it.",
            );
            return;
        }
        let facet = self.state.facet_of(caster);
        let Some(&Position(at)) = self.state.registry.get::<Position>(caster) else {
            return;
        };
        // Mark has one end — where you are standing is where the rune points.
        if !magic::may_travel(&self.state, caster, magic::TravelKind::Mark, facet, at) {
            self.notify_self(caster, magic::TravelKind::Mark.refusal());
            return;
        }

        self.state.registry.insert(
            rune,
            RuneMark {
                facet,
                destination: at,
            },
        );
        // The rune's name *is* its description — ServUO keeps a string on the
        // item and so does this, rather than a second field saying the same
        // thing. A rune that reads "1495, 1629" in a pack of sixteen is a rune
        // nobody can find twice.
        //
        // It is also the first thing in the world whose name *changes*. Nothing
        // re-sends a tooltip mid-life yet (the roadmap records that as a gap), so
        // the client picks the new one up the next time it asks, and the spell's
        // own line says what was marked meanwhile.
        let name = magic::describe(&self.state, facet, at);
        self.state
            .registry
            .insert(rune, Name(format!("a recall rune ({name})")));
        self.notify_self(caster, &format!("You mark the rune: {name}."));
    }

    /// Take the caster to where a marked rune points — the Recall spell.
    pub(super) fn recall(&mut self, caster: EntityId, target_serial: u32) {
        let Some(rune) =
            Serial::new(target_serial).and_then(|serial| self.state.registry.entity_of(serial))
        else {
            return;
        };
        // Recall is the one of the pair that does *not* want the rune in your
        // pack — ServUO's target accepts any rune you can see, and a rune held
        // out by a friend is a classic way to be fetched. Reach is still the
        // server's to judge.
        if !openshard_items::in_reach(&self.state, rune, caster) {
            self.notify_self(caster, "That is too far away.");
            return;
        }
        let Some((facet, destination)) = magic::destination_of(&self.state, rune) else {
            self.notify_self(caster, "That rune is not yet marked.");
            return;
        };
        self.travel_to(caster, facet, destination, magic::TravelKind::RecallFrom);
    }

    /// The arrival half, shared by Recall and by anything else that takes
    /// somebody to a remembered spot.
    ///
    /// `kind` names the *departure*; the destination is checked as the matching
    /// arrival, so one call site cannot check one end and forget the other.
    pub(super) fn travel_to(
        &mut self,
        traveller: EntityId,
        facet: u8,
        destination: Point,
        kind: magic::TravelKind,
    ) {
        let arriving = match kind {
            magic::TravelKind::RecallFrom => magic::TravelKind::RecallTo,
            magic::TravelKind::GateFrom => magic::TravelKind::GateTo,
            other => other,
        };
        let here = self.state.facet_of(traveller);

        // The classic rule, pre-AoS: a rune marked on another facet is a rune
        // you walk to. `cross_facet_travel` turns it into the behaviour from AoS
        // on. The machinery underneath works either way — this is a rule, not a
        // missing feature.
        if facet != here && !self.state.gameplay.cross_facet_travel {
            self.notify_self(traveller, "You cannot recall to another facet.");
            return;
        }
        if !self.state.facets.contains_key(&facet) {
            self.notify_self(traveller, arriving.refusal());
            return;
        }
        // Both ends, each against its own kind. Leaving is checked where the
        // traveller stands and arriving where they are going — the two are
        // different questions, and ServUO's `RecallFrom` row is the permissive
        // one precisely so a place you may not enter is still one you may leave.
        if let Some((here_facet, here)) = magic::standing_at(&self.state, traveller) {
            if !magic::may_travel(&self.state, traveller, kind, here_facet, here) {
                self.notify_self(traveller, kind.refusal());
                return;
            }
        }
        if !magic::may_travel(&self.state, traveller, arriving, facet, destination) {
            self.notify_self(traveller, arriving.refusal());
            return;
        }
        // Somewhere you could not have walked to is somewhere you may not arrive
        // — ServUO's `CanSpawnMobile`, cliloc 501025. Without it a rune marked
        // in a doorway that later grew a wall is a rune that buries you in it.
        if !self.can_stand_at(facet, destination) {
            self.notify_self(traveller, "Something is blocking the location.");
            return;
        }

        // ServUO plays the same sound at both ends: one for leaving, one for
        // arriving, and they are the whole of what an onlooker at either end
        // sees of a recall.
        self.state.play_sound(traveller, RECALL_SOUND);
        self.state.move_to(traveller, facet, destination);
        self.state.play_sound(traveller, RECALL_SOUND);
    }

    /// Whether a mobile could stand on this tile — the arrival test.
    fn can_stand_at(&self, facet: u8, at: Point) -> bool {
        let state = self.state.facet_state(facet);
        // A facet with no map is development mode, where every tile is allowed;
        // the same convention the step check uses.
        if state.terrain.is_none() {
            return true;
        }
        // The live floor, so a wall the world put there since the rune was
        // marked counts as much as one the map has always had.
        let live = state.live_terrain();
        // Reached from its own height, which is what asking "is this tile
        // standable" means when nobody is walking onto it from anywhere.
        openshard_movement::Terrain::stand_z(&live, at.x, at.y, i32::from(at.z)).is_some()
    }

    /// Whether `item` is inside the mobile's own backpack, at any depth.
    fn in_own_backpack(&self, mobile: EntityId, item: EntityId) -> bool {
        let Some(owner) = self.state.registry.serial_of(mobile) else {
            return false;
        };
        let Some(pack) = self
            .state
            .registry
            .query::<Equipped>()
            .find(|(_, worn)| worn.mobile == owner && worn.layer == BACKPACK_LAYER)
            .and_then(|(pack, _)| self.state.registry.serial_of(pack))
        else {
            return false;
        };
        // Walk out through the nesting: a rune in a pouch in the pack counts,
        // which is where anyone who owns sixteen of them keeps them.
        let mut container = self
            .state
            .registry
            .get::<Contained>(item)
            .map(|held| held.container);
        for _ in 0..MAX_CONTAINER_DEPTH {
            match container {
                Some(serial) if serial == pack => return true,
                Some(serial) => {
                    container = self
                        .state
                        .registry
                        .entity_of(serial)
                        .and_then(|outer| self.state.registry.get::<Contained>(outer))
                        .map(|held| held.container);
                }
                None => return false,
            }
        }
        false
    }
}

/// How deep the containment walk will go before giving up. A bound rather than a
/// cycle check: containment is a tree by construction, and a bound costs nothing
/// while a corrupted save that made a loop would otherwise hang the tick.
const MAX_CONTAINER_DEPTH: usize = 16;

/// ServUO's `Recall.cs` sound, played on departure and again on arrival.
const RECALL_SOUND: u16 = 0x01FC;
