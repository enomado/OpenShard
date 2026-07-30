//! What the server has shown us.
//!
//! The client's side of `World::seen`. The server remembers what is on each
//! client's screen because there is no "what can you see" packet; this is the
//! other end of that arrangement — a record of what arrived, never a guess
//! about what is there.
//!
//! It grows with the decoders. `0x1B`, `0x20`, `0x1D`, `0x77`, `0x78` and
//! `0x1A` are decoded, so the player, every other mobile and every ground item
//! this client has been shown are held here. `0x11` (a mobile's paperdoll
//! numbers) decodes too, but is not folded in below: it is status-bar data, not
//! a position or an appearance, and belongs with whatever eventually models the
//! status bar rather than with a record of what is on screen.

use std::collections::HashMap;

use openshard_protocol::direction::Facing;
use openshard_protocol::mobile::{Equipment, Notoriety, StatusFlags};
use openshard_protocol::serial::Serial;
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{MapSize, PlayerStart, Point};

/// The client's own character, as the server last described it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Player {
    /// The serial everything else addresses this character by.
    pub serial: Serial,
    /// The body graphic.
    pub body: Graphic,
    /// Its hue. `0x1B` never carries one — see [`WorldView::entered`] — so this
    /// reads [`Hue::NONE`] until the first `0x20`.
    pub hue: Hue,
    /// Poisoned, invisible, war mode. Same absence as `hue` until the first
    /// `0x20`.
    pub flags: StatusFlags,
    /// Where it stands.
    pub position: Point,
    /// Which way it faces, and whether it is running.
    pub facing: Facing,
}

/// Another mobile, as `0x77` or `0x78` last described it.
///
/// Not the client's own character — see [`Player`] for that, and
/// [`WorldView::player`] for why the two are not the same type: `0x77`/`0x78`
/// cannot move the client's own body (Sphere's warning, kept in
/// `openshard_protocol::mobile::MobileMove`'s docs), so nothing here is ever
/// keyed by the player's serial.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mobile {
    /// Its body graphic.
    pub body: Graphic,
    /// Where it stands.
    pub position: Point,
    /// Which way it faces, and whether it is running.
    pub facing: Facing,
    /// Its hue.
    pub hue: Hue,
    /// Poisoned, invisible, war mode.
    pub flags: StatusFlags,
    /// How to colour its health bar.
    pub notoriety: Notoriety,
    /// What it is wearing.
    ///
    /// Only `0x78` carries this; a `0x77` move leaves it as it was; see
    /// [`WorldView::apply`].
    pub equipment: Vec<Equipment>,
}

/// An item on the ground, as `0x1A` last described it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Item {
    /// Its graphic.
    pub graphic: Graphic,
    /// How many are in the stack.
    pub amount: u16,
    /// Where it lies.
    pub position: Point,
    /// Its hue, or [`Hue::NONE`] for none.
    pub hue: Hue,
}

/// Everything this client has been told about the world.
///
/// # There is no such thing as an empty one
///
/// A `WorldView` is built from the `0x1B` that puts a body in the world, so it
/// cannot exist before the client is in one. That is why nothing here is an
/// `Option`: "we are not in the world yet" is the absence of this whole value,
/// not a field inside it saying so.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorldView {
    /// This client's character.
    pub player: Player,
    /// How big the facet is. The client needs it to bound the map it draws.
    pub map: MapSize,
    /// Every other mobile this client has been shown, by serial.
    pub mobiles: HashMap<Serial, Mobile>,
    /// Every ground item this client has been shown, by serial.
    pub items: HashMap<Serial, Item>,
}

impl WorldView {
    /// The world as the entry packet described it: nobody else on screen yet.
    #[must_use]
    pub fn entered(start: PlayerStart) -> Self {
        Self {
            player: Player {
                serial: start.serial,
                body: start.body,
                hue: Hue::NONE,
                flags: StatusFlags::NONE,
                position: start.position,
                facing: start.facing,
            },
            map: start.map,
            mobiles: HashMap::new(),
            items: HashMap::new(),
        }
    }

    /// Fold in what a packet says.
    ///
    /// Returns whether anything changed, which is what a renderer wants to know
    /// and what a test can assert on.
    ///
    /// Most packets are still `false`: their decoders do not exist yet, so they
    /// never reach here as anything but
    /// [`Undecoded`](crate::connection::Event::Undecoded). The list grows one
    /// decoder at a time and this is where each one lands.
    pub fn apply(&mut self, packet: &ServerPacket) -> bool {
        match packet {
            // A second `0x1B` restarts the session on a real client — it is not
            // a move — so taking it wholesale is right: whatever the server
            // says now replaces what we thought, everyone else included.
            ServerPacket::PlayerStart(start) => {
                let fresh = Self::entered(*start);
                let changed = *self != fresh;
                *self = fresh;
                changed
            }
            ServerPacket::PlayerUpdate(update) => {
                let fresh = Player {
                    serial: self.player.serial,
                    body: update.body,
                    hue: update.hue,
                    flags: update.flags,
                    position: update.position,
                    facing: update.facing,
                };
                let changed = self.player != fresh;
                self.player = fresh;
                changed
            }
            ServerPacket::MobileMove(step) => {
                // A move never touches what a mobile is wearing; keep whatever
                // 0x78 last said, or nothing if this is the first we have seen
                // of it — a naked arrival is exactly what an empty list means.
                let equipment = self
                    .mobiles
                    .get(&step.serial)
                    .map(|mobile| mobile.equipment.clone())
                    .unwrap_or_default();
                let fresh = Mobile {
                    body: step.body,
                    position: step.position,
                    facing: step.facing,
                    hue: step.hue,
                    flags: step.flags,
                    notoriety: step.notoriety,
                    equipment,
                };
                let changed = self.mobiles.get(&step.serial) != Some(&fresh);
                self.mobiles.insert(step.serial, fresh);
                changed
            }
            ServerPacket::MobileIncoming(incoming) => {
                let fresh = Mobile {
                    body: incoming.body,
                    position: incoming.position,
                    facing: incoming.facing,
                    hue: incoming.hue,
                    flags: incoming.flags,
                    notoriety: incoming.notoriety,
                    equipment: incoming.equipment.clone(),
                };
                let changed = self.mobiles.get(&incoming.serial) != Some(&fresh);
                self.mobiles.insert(incoming.serial, fresh);
                changed
            }
            ServerPacket::WorldItem(item) => {
                let fresh = Item {
                    graphic: item.graphic,
                    amount: item.amount,
                    position: item.position,
                    hue: item.hue,
                };
                let changed = self.items.get(&item.serial) != Some(&fresh);
                self.items.insert(item.serial, fresh);
                changed
            }
            // Mobiles walking out of range and items being picked up arrive the
            // same way — the client does not distinguish, it just forgets the
            // serial. Only one of the two maps can ever hold it.
            ServerPacket::Remove(remove) => {
                let had_mobile = self.mobiles.remove(&remove.serial).is_some();
                let had_item = self.items.remove(&remove.serial).is_some();
                had_mobile || had_item
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use openshard_protocol::direction::Direction;
    use openshard_protocol::items::WorldItem;
    use openshard_protocol::mobile::{MobileIncoming, MobileMove, Remove};
    use openshard_protocol::world::PlayerUpdate;

    use super::*;

    fn start() -> PlayerStart {
        PlayerStart {
            serial: Serial::new(0x0000_002A).unwrap(),
            body: Graphic(0x0190),
            position: Point::new(1475, 1770, 20),
            facing: Facing::walking(Direction::South),
            map: MapSize::BRITANNIA,
        }
    }

    fn other() -> Serial {
        Serial::new(0x0000_0002).unwrap()
    }

    fn shirt() -> Equipment {
        Equipment {
            serial: Serial::new(0x4000_0001).unwrap(),
            graphic: Graphic(0x1517),
            layer: openshard_protocol::wire::Layer(0x05),
            hue: Hue(0x0021),
        }
    }

    #[test]
    fn entering_records_what_the_server_said() {
        let view = WorldView::entered(start());
        assert_eq!(view.player.position, Point::new(1475, 1770, 20));
        assert_eq!(view.map, MapSize::BRITANNIA);
        assert!(view.mobiles.is_empty());
        assert!(view.items.is_empty());
    }

    #[test]
    fn a_repeated_entry_packet_replaces_the_view() {
        // The server sends 0x1B to *restart* a session, not to nudge a
        // position. Merging it field by field would leave a client half in the
        // old world.
        let mut view = WorldView::entered(start());
        let moved = PlayerStart {
            position: Point::new(1000, 1000, -10),
            ..start()
        };
        assert!(view.apply(&ServerPacket::PlayerStart(moved)));
        assert_eq!(view.player.position, Point::new(1000, 1000, -10));
        assert!(
            !view.apply(&ServerPacket::PlayerStart(moved)),
            "the same packet twice changes nothing"
        );
    }

    #[test]
    fn a_player_update_moves_the_players_own_body() {
        let mut view = WorldView::entered(start());
        let update = PlayerUpdate {
            serial: view.player.serial,
            body: Graphic(0x0191),
            hue: Hue(0x0021),
            flags: StatusFlags::NONE,
            position: Point::new(1480, 1770, 20),
            facing: Facing::running(Direction::East),
        };
        assert!(view.apply(&ServerPacket::PlayerUpdate(update)));
        assert_eq!(view.player.body, Graphic(0x0191));
        assert_eq!(view.player.hue, Hue(0x0021));
        assert_eq!(view.player.position, Point::new(1480, 1770, 20));
        assert_eq!(view.player.facing, Facing::running(Direction::East));
        assert!(
            !view.apply(&ServerPacket::PlayerUpdate(update)),
            "the same packet twice changes nothing"
        );
    }

    #[test]
    fn a_mobile_incoming_is_recorded_with_its_equipment() {
        let mut view = WorldView::entered(start());
        let incoming = MobileIncoming {
            serial: other(),
            body: Graphic(0x0190),
            position: Point::new(1476, 1770, 20),
            facing: Facing::walking(Direction::South),
            hue: Hue(0x83EA),
            flags: StatusFlags::NONE,
            notoriety: Notoriety::Innocent,
            equipment: vec![shirt()],
        };
        assert!(view.apply(&ServerPacket::MobileIncoming(incoming.clone())));
        let mobile = view.mobiles.get(&other()).expect("the mobile was recorded");
        assert_eq!(mobile.position, Point::new(1476, 1770, 20));
        assert_eq!(mobile.equipment, vec![shirt()]);
        assert!(
            !view.apply(&ServerPacket::MobileIncoming(incoming)),
            "the same packet twice changes nothing"
        );
    }

    #[test]
    fn a_mobile_move_keeps_the_equipment_a_0x78_already_set() {
        // 0x77 never carries an equipment list — it is a move, not a redraw —
        // so a mobile already on screen must not be stripped naked by one.
        let mut view = WorldView::entered(start());
        view.apply(&ServerPacket::MobileIncoming(MobileIncoming {
            serial: other(),
            body: Graphic(0x0190),
            position: Point::new(1476, 1770, 20),
            facing: Facing::walking(Direction::South),
            hue: Hue(0x83EA),
            flags: StatusFlags::NONE,
            notoriety: Notoriety::Innocent,
            equipment: vec![shirt()],
        }));

        assert!(view.apply(&ServerPacket::MobileMove(MobileMove {
            serial: other(),
            body: Graphic(0x0190),
            position: Point::new(1477, 1770, 20),
            facing: Facing::walking(Direction::South),
            hue: Hue(0x83EA),
            flags: StatusFlags::NONE,
            notoriety: Notoriety::Innocent,
        })));

        let mobile = view.mobiles.get(&other()).unwrap();
        assert_eq!(mobile.position, Point::new(1477, 1770, 20));
        assert_eq!(mobile.equipment, vec![shirt()], "the move must not undress it");
    }

    #[test]
    fn a_world_item_is_recorded_by_serial() {
        let mut view = WorldView::entered(start());
        let item = WorldItem {
            serial: Serial::new(0x4000_00AB).unwrap(),
            graphic: Graphic(0x0EED),
            amount: 500,
            position: Point::new(1000, 2000, 5),
            hue: Hue(0x0021),
        };
        assert!(view.apply(&ServerPacket::WorldItem(item)));
        assert_eq!(view.items.get(&item.serial).unwrap().amount, 500);
    }

    #[test]
    fn a_remove_forgets_whichever_map_actually_holds_the_serial() {
        // The client does not distinguish a mobile walking out of range from an
        // item being picked up; neither does Remove — it just tries both maps.
        let mut view = WorldView::entered(start());
        let item = WorldItem {
            serial: Serial::new(0x4000_00AB).unwrap(),
            graphic: Graphic(0x0EED),
            amount: 1,
            position: Point::new(1000, 2000, 5),
            hue: Hue::NONE,
        };
        view.apply(&ServerPacket::WorldItem(item));

        assert!(view.apply(&ServerPacket::Remove(Remove { serial: item.serial })));
        assert!(!view.items.contains_key(&item.serial));
        assert!(
            !view.apply(&ServerPacket::Remove(Remove { serial: item.serial })),
            "forgetting something already gone changes nothing"
        );
    }
}
