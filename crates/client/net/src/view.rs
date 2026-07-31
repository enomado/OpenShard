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
//!
//! Two of those packets can name the client's own serial, and neither means
//! what it means about anybody else: a `0x78` about ourselves is the paperdoll
//! a shard sends exactly once at world entry, and a `0x77` about ourselves is
//! not a move at all. Both are routed by serial in [`WorldView::apply`], so
//! [`WorldView::mobiles`] holds only *other* mobiles, as its docs promise.

use std::collections::HashMap;

use openshard_protocol::direction::Facing;
use openshard_protocol::mobile::{Equipment, Notoriety, StatusFlags};
use openshard_protocol::serial::Serial;
use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{MapSize, PlayerStart, Point};

/// The client's own character, as the server last described it.
///
/// Not `Copy`: it carries the equipment list, which is a `Vec`. That is the
/// price of the one packet a client hears about its own paperdoll from — see
/// [`Player::equipment`].
#[derive(Clone, PartialEq, Eq, Debug)]
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
    /// What it is wearing, including the backpack it must be able to open.
    ///
    /// `0x1B` carries no equipment and neither does `0x20`, so this is empty
    /// until the server sends this client a `0x78` naming *its own* serial —
    /// which a shard does exactly once, at world entry, because the pass that
    /// reveals a mobile sends it to everyone except itself.
    pub equipment: Vec<Equipment>,
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
                equipment: Vec::new(),
            },
            map: start.map,
            mobiles: HashMap::new(),
            items: HashMap::new(),
        }
    }

    /// Record a step of the player's own that the server has confirmed.
    ///
    /// The one thing that reaches the player from outside [`apply`](Self::apply),
    /// and it has to be: a `0x22` ack carries a sequence number and a health-bar
    /// colour and *no position*, so where the body now stands is the tile the
    /// acked step was asking for, and only [`Walk`](crate::walk::Walk) — which
    /// sent it — knows what that was.
    ///
    /// This is still a record of what the server said rather than a guess: the
    /// ack is the saying. The prediction that has not been acked yet stays where
    /// it belongs, in [`Walk::predicted`](crate::walk::Walk::predicted).
    ///
    /// Returns whether anything changed, the same as [`apply`](Self::apply).
    pub fn player_stepped(&mut self, position: Point, facing: Facing) -> bool {
        let changed = self.player.position != position || self.player.facing != facing;
        self.player.position = position;
        self.player.facing = facing;
        changed
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
                    // `0x20` is a position and an appearance, never a paperdoll:
                    // keeping what the `0x78` said is the difference between a
                    // client that still knows its backpack and one that forgets
                    // it the first time the server nudges the body.
                    equipment: self.player.equipment.clone(),
                };
                let changed = self.player != fresh;
                self.player = fresh;
                changed
            }
            // A `0x77` naming this client's own serial is not a move of it: the
            // client's body is moved by `0x20` and by its own acked steps, and
            // acting on this one would fight the prediction in `Walk`. See
            // `openshard_protocol::mobile::MobileMove`.
            ServerPacket::MobileMove(step) if step.serial == self.player.serial => false,
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
            // The one `0x78` a client is sent about itself, and the only place
            // it learns what it is wearing. It goes to the player rather than
            // into `mobiles`, which is never keyed by our own serial: a body in
            // both would be drawn twice, once at each end's idea of where it is.
            ServerPacket::MobileIncoming(incoming) if incoming.serial == self.player.serial => {
                let fresh = Player {
                    serial: self.player.serial,
                    body: incoming.body,
                    hue: incoming.hue,
                    flags: incoming.flags,
                    position: incoming.position,
                    facing: incoming.facing,
                    equipment: incoming.equipment.clone(),
                };
                let changed = self.player != fresh;
                self.player = fresh;
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
    fn a_confirmed_step_moves_the_player_and_says_when_it_did_not() {
        // What `Walk` hands back on a `0x22`. Turning is a step in UO and its
        // ack looks exactly like a move's, so "the position did not change"
        // must not read as "nothing happened": the facing did.
        let mut view = WorldView::entered(start());
        assert!(view.player_stepped(Point::new(1475, 1769, 20), Facing::walking(Direction::North)));
        assert_eq!(view.player.position, Point::new(1475, 1769, 20));
        assert!(
            !view.player_stepped(Point::new(1475, 1769, 20), Facing::walking(Direction::North)),
            "the same place, facing the same way, is not a change"
        );
        assert!(
            view.player_stepped(Point::new(1475, 1769, 20), Facing::walking(Direction::East)),
            "a turn moves nobody and is still a change"
        );
    }

    #[test]
    fn our_own_0x78_dresses_the_player_instead_of_adding_a_mobile() {
        // The shard sends this once, at world entry, and it is the only packet
        // that tells a client what it is wearing — the reveal pass shows a
        // mobile to everyone but itself. Filed under `mobiles` it would be a
        // second body at the player's own serial, drawn twice.
        let mut view = WorldView::entered(start());
        let mine = MobileIncoming {
            serial: view.player.serial,
            body: Graphic(0x0190),
            position: start().position,
            facing: start().facing,
            hue: Hue(0x83EA),
            flags: StatusFlags::NONE,
            notoriety: Notoriety::Innocent,
            equipment: vec![shirt()],
        };
        assert!(view.apply(&ServerPacket::MobileIncoming(mine.clone())));
        assert!(view.mobiles.is_empty(), "we are not one of the others");
        assert_eq!(view.player.equipment, vec![shirt()]);
        assert_eq!(view.player.hue, Hue(0x83EA));
        assert!(
            !view.apply(&ServerPacket::MobileIncoming(mine)),
            "the same packet twice changes nothing"
        );

        // And a 0x20 afterwards must not undress us: it carries a body and a
        // position, and no paperdoll at all.
        view.apply(&ServerPacket::PlayerUpdate(PlayerUpdate {
            serial: view.player.serial,
            body: Graphic(0x0190),
            hue: Hue(0x83EA),
            flags: StatusFlags::NONE,
            position: Point::new(1476, 1770, 20),
            facing: Facing::walking(Direction::North),
        }));
        assert_eq!(view.player.equipment, vec![shirt()]);
    }

    #[test]
    fn our_own_0x77_moves_nothing() {
        // Sphere's warning, from the client's side: 0x77 cannot move the body
        // the client is predicting for. Acting on one would fight `Walk`.
        let mut view = WorldView::entered(start());
        assert!(!view.apply(&ServerPacket::MobileMove(MobileMove {
            serial: view.player.serial,
            body: Graphic(0x0190),
            position: Point::new(1000, 1000, 0),
            facing: Facing::walking(Direction::North),
            hue: Hue::NONE,
            flags: StatusFlags::NONE,
            notoriety: Notoriety::Innocent,
        })));
        assert_eq!(view.player.position, start().position);
        assert!(view.mobiles.is_empty());
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
