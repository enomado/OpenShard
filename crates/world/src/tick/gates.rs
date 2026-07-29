//! Gates: the pair a spell opens, the eight that always stand, and the two ways
//! to step through either.
//!
//! The trigger lives here rather than in `items` for the reason `tick/traps.rs`
//! does: walking into a gate calls `magic`, and `items` may not depend on
//! `magic` without closing the `skills → items → magic → skills` loop. So the
//! click and the step are dispatched from the tick, and what they *do* is the
//! travel crate's.
//!
//! **Walking in is found, not announced.** There are two movement paths — the
//! player walk and the server-authoritative step — and a call beside each is one
//! to forget. Both already emit `MobileMoved`, so this reads the tick's own
//! events, the shape `guard_crossings` uses. Unlike a once-a-tick scan of every
//! position it cannot miss somebody who steps on and off a gate inside one
//! batch of commands, which is an ordinary thing for the inbox to hold.

use super::*;
use openshard_state::components::{Moongate, Position, MOONGATE_GRAPHIC, MOONGATE_REACH};

/// How long a Gate Travel pair stands — ServUO's thirty seconds.
const GATE_LIFETIME_SECONDS: u64 = 30;

/// ServUO's `GateTravel` sound, played at both ends as the pair opens.
const GATE_OPEN_SOUND: u16 = 0x020E;
/// And the one a traveller makes arriving through one.
const GATE_USE_SOUND: u16 = 0x01FE;

impl World {
    /// Open a gate at the caster and another at `destination`, each leading to
    /// the other.
    pub(super) fn open_gate_pair(&mut self, caster: EntityId, target_serial: u32) {
        let Some(rune) =
            Serial::new(target_serial).and_then(|serial| self.state.registry.entity_of(serial))
        else {
            return;
        };
        if !openshard_items::in_reach(&self.state, rune, caster) {
            self.notify_self(caster, "That is too far away.");
            return;
        }
        let Some((there_facet, there)) = magic::destination_of(&self.state, rune) else {
            self.notify_self(caster, "That rune is not yet marked.");
            return;
        };
        let Some((here_facet, here)) = magic::standing_at(&self.state, caster) else {
            return;
        };

        if there_facet != here_facet && !self.state.gameplay.cross_facet_travel {
            self.notify_self(caster, "You cannot gate to another facet.");
            return;
        }
        if !self.state.facets.contains_key(&there_facet) {
            self.notify_self(caster, magic::TravelKind::GateTo.refusal());
            return;
        }
        // Both ends, each against its own kind — the same pairing Recall makes,
        // and the reason `may_travel` takes one end at a time.
        if !magic::may_travel(
            &self.state,
            caster,
            magic::TravelKind::GateFrom,
            here_facet,
            here,
        ) {
            self.notify_self(caster, magic::TravelKind::GateFrom.refusal());
            return;
        }
        if !magic::may_travel(
            &self.state,
            caster,
            magic::TravelKind::GateTo,
            there_facet,
            there,
        ) {
            self.notify_self(caster, magic::TravelKind::GateTo.refusal());
            return;
        }
        // ServUO checks both ends for an existing gate, and so does Sphere for a
        // telepad. Two gates on one tile are two overlapping ways out of the same
        // spot, and closing one leaves the other looking broken.
        if self.gate_at(here_facet, here).is_some() || self.gate_at(there_facet, there).is_some() {
            self.notify_self(caster, "There is already a gate there.");
            return;
        }

        let expires_at = Some(self.state.ticks + GATE_LIFETIME_SECONDS * TICKS_PER_SECOND);
        // Each points at the other's tile. There is no link to keep honest —
        // the destination *is* the link.
        self.spawn_gate(
            here_facet,
            here,
            Moongate {
                facet: there_facet,
                destination: there,
                expires_at,
            },
        );
        self.spawn_gate(
            there_facet,
            there,
            Moongate {
                facet: here_facet,
                destination: here,
                expires_at,
            },
        );
        self.state.play_sound(caster, GATE_OPEN_SOUND);
        self.notify_self(caster, "You open a magical gate to another location.");
    }

    /// Put one gate on the ground — the drawn-item path, `spawn_field_tile`'s
    /// shape.
    ///
    /// Deliberately **not** `items::spawn_item`: that stamps a `Decays` (a second
    /// clock contradicting this one) and emits an `ItemSpawned` the pack would
    /// read as somebody dropping something. A gate owns its own lifetime, and a
    /// permanent one owns none at all.
    pub(super) fn spawn_gate(&mut self, facet: u8, at: Point, gate: Moongate) -> Option<EntityId> {
        let Ok((entity, _serial)) = self.state.registry.spawn_with_serial(SerialKind::Item) else {
            warn!("out of item serials; not opening a gate");
            return None;
        };
        self.state.registry.insert(
            entity,
            Graphic {
                id: MOONGATE_GRAPHIC,
                hue: 0,
            },
        );
        self.state.registry.insert(entity, Position(at));
        self.state.registry.insert(entity, Facet(facet));
        self.state.registry.insert(entity, gate);
        self.state.facet_state_mut(facet).sectors.insert(entity, at);
        // No obstruction, ever: a gate is walked *into*. Blocking the tile is how
        // the walk-in trigger becomes dead code that reads as a movement bug.
        self.state.reveal(entity);
        Some(entity)
    }

    /// The gate standing on a tile, if any.
    pub(super) fn gate_at(&self, facet: u8, at: Point) -> Option<EntityId> {
        self.state
            .registry
            .query::<Moongate>()
            .find(|(entity, _)| {
                self.state.facet_of(*entity) == facet
                    && self
                        .state
                        .registry
                        .get::<Position>(*entity)
                        .is_some_and(|pos| pos.0.x == at.x && pos.0.y == at.y)
            })
            .map(|(entity, _)| entity)
    }

    /// Close the gates whose time is up — the tick counter, like a field and a
    /// decaying item, so a gate replays.
    pub(super) fn expire_gates(&mut self) {
        let now = self.state.ticks;
        let done: Vec<EntityId> = self
            .state
            .registry
            .query::<Moongate>()
            .filter(|(_, gate)| gate.expires_at.is_some_and(|at| now >= at))
            .map(|(entity, _)| entity)
            .collect();
        for entity in done {
            self.close_gate(entity);
        }
    }

    /// Take a gate off the world: forget it on every screen, off the sector grid,
    /// out of the registry. `remove_field`'s tail — miss it and what is left is
    /// an invisible gate that still works.
    fn close_gate(&mut self, entity: EntityId) {
        let Some(serial) = self.state.registry.serial_of(entity) else {
            return;
        };
        let facet = self.state.facet_of(entity);
        for watcher in self.state.watchers_of(entity) {
            self.state.forget(watcher, entity, serial);
        }
        self.state.facet_state_mut(facet).sectors.remove(entity);
        self.state.registry.despawn(entity);
    }

    /// Take everyone who stepped onto a gate this tick through it.
    pub(super) fn gate_crossings(&mut self) {
        let moved: Vec<(EntityId, Point)> = self
            .state
            .bus
            .read(&mut self.gated)
            .map(|event| (event.entity, event.to))
            .collect();
        for (entity, to) in moved {
            let facet = self.state.facet_of(entity);
            if let Some(gate) = self.gate_at(facet, to) {
                self.use_gate(entity, gate);
            }
        }
    }

    /// A double-click on a gate within reach — ServUO's `Moongate.OnDoubleClick`.
    ///
    /// Returns whether the click was a gate's, so the tick can stop before the
    /// ordinary item dispatch sees it.
    pub(super) fn click_gate(&mut self, player: EntityId, target: EntityId) -> bool {
        if !self.state.registry.has::<Moongate>(target) {
            return false;
        }
        let near = match (
            self.state.registry.get::<Position>(player),
            self.state.registry.get::<Position>(target),
        ) {
            (Some(&Position(here)), Some(&Position(there))) => {
                self.state.facet_of(player) == self.state.facet_of(target)
                    && openshard_state::sectors::in_range(here, there, MOONGATE_REACH)
            }
            _ => false,
        };
        if near {
            self.use_gate(player, target);
        }
        true
    }

    /// Step through. One door for both triggers, so a gate cannot be safe to walk
    /// into and unsafe to click.
    pub(super) fn use_gate(&mut self, traveller: EntityId, gate: EntityId) {
        let Some(&gate) = self.state.registry.get::<Moongate>(gate) else {
            return;
        };
        // Mid-cast or holding something, you are too busy — ServUO's 1049616 and
        // 1071955. Both are about arriving somewhere with state that belonged to
        // where you left.
        if self
            .state
            .registry
            .has::<openshard_state::components::Casting>(traveller)
        {
            self.notify_self(traveller, "You are too busy to do that.");
            return;
        }
        if self
            .connection_of(traveller)
            .is_some_and(|connection| self.state.held.contains_key(&connection))
        {
            self.notify_self(traveller, "You cannot teleport while dragging an object.");
            return;
        }
        if !self.state.facets.contains_key(&gate.facet) {
            return;
        }
        self.state.move_to(traveller, gate.facet, gate.destination);
        self.state.play_sound(traveller, GATE_USE_SOUND);
    }
}
