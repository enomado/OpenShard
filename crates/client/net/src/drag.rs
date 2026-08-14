//! Client-side encoders for lifting an item and putting it into a container.

use openshard_protocol::feature::Feature;
use openshard_protocol::gump::GumpPoint;
use openshard_protocol::items::{DROP_TO_GROUND, DropItem, PickUpItem};
use openshard_protocol::packet::{DecodePacket, PacketLength, frame_body};
use openshard_protocol::serial::Serial;
use openshard_protocol::version::ClientVersion;
use openshard_protocol::world::Point;

/// Ask the shard to put `item` on the cursor.
#[must_use]
pub fn pick_up(item: Serial, amount: u16) -> Vec<u8> {
    frame_body(PickUpItem::ID, PacketLength::Fixed(7), |out| {
        out.u32(item.raw());
        out.u16(amount);
    })
}

/// Put the cursor item in a container at its gump-local location.
#[must_use]
pub fn drop_into(item: Serial, container: Serial, at: GumpPoint, version: ClientVersion) -> Vec<u8> {
    let grid = version.supports(Feature::ItemGrid);
    frame_body(
        DropItem::ID,
        PacketLength::Fixed(if grid { 15 } else { 14 }),
        |out| {
            out.u32(item.raw());
            out.u16(at.x.clamp(0, u16::MAX.into()) as u16);
            out.u16(at.y.clamp(0, u16::MAX.into()) as u16);
            out.u8(0);
            if grid {
                out.u8(0);
            }
            out.u32(container.raw());
        },
    )
}

/// Put the cursor item onto a world tile.
#[must_use]
pub fn drop_on_ground(item: Serial, at: Point, version: ClientVersion) -> Vec<u8> {
    let grid = version.supports(Feature::ItemGrid);
    frame_body(
        DropItem::ID,
        PacketLength::Fixed(if grid { 15 } else { 14 }),
        |out| {
            out.u32(item.raw());
            out.u16(at.x);
            out.u16(at.y);
            out.u8(at.z as u8);
            if grid {
                out.u8(0);
            }
            out.u32(DROP_TO_GROUND.0);
        },
    )
}

#[cfg(test)]
mod tests {
    use openshard_protocol::client_packet::ClientPacket;

    use super::*;

    #[test]
    fn container_drag_packets_round_trip_through_the_shards_decoder() {
        let version = ClientVersion::new(7, 0, 45, 65);
        let item = Serial::new(0x4000_002A).unwrap();
        let bag = Serial::new(0x4000_002B).unwrap();

        let ClientPacket::PickUpItem(pickup) = ClientPacket::decode(&pick_up(item, 7), version).unwrap()
        else {
            panic!("lift was not a 0x07");
        };
        assert_eq!(pickup.serial.validate(), Some(item));
        assert_eq!(pickup.amount, 7);

        let ClientPacket::DropItem(drop) =
            ClientPacket::decode(&drop_into(item, bag, GumpPoint::new(42, 73), version), version).unwrap()
        else {
            panic!("drop was not a 0x08");
        };
        assert_eq!(drop.serial.validate(), Some(item));
        assert_eq!(
            drop.destination(),
            openshard_protocol::items::DropDestination::Item {
                item: bag,
                at: GumpPoint::new(42, 73),
            }
        );
        let ground = Point::new(100, 200, 7);
        let ClientPacket::DropItem(drop) =
            ClientPacket::decode(&drop_on_ground(item, ground, version), version).unwrap()
        else {
            panic!("ground drop was not a 0x08");
        };
        assert_eq!(
            drop.destination(),
            openshard_protocol::items::DropDestination::Ground(ground)
        );
    }
}
