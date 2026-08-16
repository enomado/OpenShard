//! Houses across the save: swept out, and rebuilt on the way back in.
//!
//! # What is saved is where it stands, not what it is made of
//!
//! A multi's shape is a pure function of its id and it lives in the client's own
//! files. Saving the components would be saving a copy of a file every client
//! already has — one that goes stale the day the operator updates their install,
//! and then the shard's walls and the client's picture disagree with no way to
//! tell which is right. So the record is the id and the position, and the
//! footprint is recomputed at boot from the same table placement read it from.
//!
//! The consequence to accept: a shard booted **without** client files restores
//! the house entities and gives them no walls. That is the same bargain every
//! other `Terrain` method makes, and it is better than the alternative, which is
//! a house whose walls came from a file the client no longer has.

use openshard_persistence::record::HouseRecord;
use openshard_protocol::wire::{Graphic, Hue};
use openshard_protocol::world::{Facet, Point};
use openshard_state::components::{Drawn, House, Position};
use tracing::{info, warn};

use super::World;

impl World {
    /// Every house as a saveable record.
    pub(super) fn house_records(&self) -> Vec<HouseRecord> {
        self.state
            .registry
            .query::<House>()
            .filter_map(|(entity, house)| {
                let serial = self.state.registry.serial_of(entity)?;
                let &Position(at) = self.state.registry.get::<Position>(entity)?;
                Some(HouseRecord {
                    serial,
                    multi: house.multi,
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    facet: self.state.facet_of(entity).0,
                    owner: house.owner,
                })
            })
            .collect()
    }

    /// Put the houses back at boot, walls and all.
    ///
    /// Call once, before anyone connects. Not through `openshard_housing::place`:
    /// that decides whether a house *may* go somewhere, and a house that was
    /// legal when it was built stays built even if the rules have since tightened
    /// — otherwise a shard that changed its yard size would silently demolish
    /// half of Britannia at the next restart.
    pub fn restore_houses(&mut self, records: Vec<HouseRecord>) {
        let mut restored = 0;
        let mut wall_less = 0;
        for record in records {
            let facet = Facet(record.facet);
            let at = Point::new(record.x, record.y, record.z);
            let entity = self.state.registry.spawn();
            if self.state.registry.bind_serial(entity, record.serial).is_err() {
                warn!(serial = %record.serial, "a saved house's serial was already taken");
                self.state.registry.despawn(entity);
                continue;
            }
            self.state.registry.insert(
                entity,
                Drawn {
                    id: Graphic(openshard_housing::MULTI_FLAG | record.multi),
                    hue: Hue(0),
                },
            );
            self.state.registry.insert(entity, Position(at));
            self.state.registry.insert(
                entity,
                House {
                    multi: record.multi,
                    owner: record.owner,
                },
            );
            self.state.registry.insert(entity, facet);
            self.state.facet_state_mut(facet).sectors.insert(entity, at);

            match openshard_housing::footprint_of(&self.state, at, facet, record.multi) {
                Ok(footprint) => {
                    openshard_housing::block(&mut self.state, entity, facet, &footprint);
                }
                // No client files, or an id this install does not know. The house
                // is still there and still owned; it simply stops nobody, which
                // is said out loud rather than left to be found by walking
                // through a wall.
                Err(_) => wall_less += 1,
            }
            restored += 1;
        }
        if restored > 0 {
            info!(houses = restored, wall_less, "restored the shard's houses");
        }
    }
}
