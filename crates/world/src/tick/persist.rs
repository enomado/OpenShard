use super::*;
use openshard_persistence::{
    CorpseData, DoneQuestRecord, EffectRecord, PetData, QuestRecord, RestockRecord, RunebookData,
    RunebookEntryData,
};
use openshard_state::components::{
    body_opens_doors, effect, Aggression, Banker, BehaviourBuff, BehaviourBuffs, Corpse, CraftedBy,
    DoneQuest, Escortable, Field, Frozen, Moongate, NightHome, Npc, Pet, PetOrder, PoisonCharges,
    Poisoned, Price, Quality, QuestGiver, QuestLog, QuestState, RangedAttack, Restock, RuneMark,
    Runebook, RunebookEntry, Skills, Spellbook, StatMod, StatMods, StockRecord, SwingSpeed, Title,
    TradeWindow, Trap, TrapKind, Vendor,
};

impl World {
    // -- persistence -------------------------------------------------------

    /// Mark what changed, from what the tick said happened.
    ///
    /// # Why this reads the bus instead of being called from each mutation
    ///
    /// The obvious version is a `journal.touch(entity)` next to every
    /// `registry.insert`. It works, and it decays: the day someone adds a system
    /// that moves a mobile — a teleport, a knockback, a script — they have to
    /// know that persistence exists and remember a line that nothing will fail
    /// without. The bug is silent, it survives every test that does not restart
    /// the shard, and it looks like the disk lost something.
    ///
    /// Emitting the event *is* the touch. A system that moves a mobile already
    /// has to say so, because that is how the client hears about it, and the
    /// same event now also means "and write it down". There is nothing left to
    /// forget.
    pub(super) fn mark_dirty(&mut self) {
        // Collected first: `read` borrows the bus, and the journal is a
        // different field but the iterator holds the borrow across the loop.
        let mut changed: Vec<EntityId> = Vec::new();
        changed.extend(
            self.state
                .bus
                .read(&mut self.entered)
                .map(|event| event.entity),
        );
        changed.extend(
            self.state
                .bus
                .read(&mut self.moved)
                .map(|event| event.entity),
        );
        changed.extend(
            self.state
                .bus
                .read(&mut self.turned)
                .map(|event| event.entity),
        );
        for entity in changed {
            self.journal.touch(entity);
        }
    }

    /// Every `save_every` ticks, hand what changed to whoever is collecting.
    pub(super) fn offer_snapshot(&mut self) {
        if self.save_every == 0 || !self.state.ticks.is_multiple_of(self.save_every) {
            return;
        }
        self.take_snapshot();
    }

    /// End every secure trade in progress, returning both sides' offerings.
    ///
    /// The shutdown path calls this before its final snapshot: an escrow is not
    /// saved, so goods left in one when the sweep runs would be lost. Outside
    /// shutdown a trade ends by itself — see `items::validate_trades`.
    pub fn cancel_all_trades(&mut self) {
        items::cancel_all_trades(&mut self.state);
    }

    /// Take a snapshot now, whatever the cadence says.
    ///
    /// For shutdown, for a GM save command, and for tests that would rather not
    /// tick four hundred times to see one row.
    pub fn take_snapshot(&mut self) {
        let ticks = self.state.ticks;

        // Start from the journal's logged-out records, their kept inventories, and
        // deletions. Dirty *online*-character records are dropped (the `|_| None`)
        // because every online character is saved in full below regardless — an
        // item picked up without a step never marks the character dirty, so the
        // dirty set is not a safe basis for saving what a character holds.
        let mut snapshot = self.journal.drain(ticks, |_| None).unwrap_or(Snapshot {
            tick: ticks,
            schema: SCHEMA_VERSION,
            characters: Vec::new(),
            removed: Vec::new(),
            inventories: Vec::new(),
            ground: None,
            spawners: None,
            mobiles: None,
            decorations: None,
            regions: None,
            clock_minutes: None,
        });

        // Every online character, whole: its record and its entire carried
        // inventory — worn gear, backpack, bank box and everything nested. A save is
        // a complete picture of who is here and what they hold, so nothing of value
        // depends on whether its owner happened to move this tick.
        let online: Vec<EntityId> = self.state.players.values().copied().collect();
        for entity in online {
            if let Some(record) = Self::record_of(&self.state.registry, entity, self.state.ticks) {
                let owner = record.serial;
                snapshot.characters.push(record);
                snapshot.inventories.push(Inventory {
                    owner,
                    items: self.inventory_of(entity),
                });
            }
        }

        // The whole ground, every save — decoration excluded (it has its own
        // sweep below). Dropped loot and stray items persist whether or not
        // anyone was active this tick.
        snapshot.ground = Some(self.ground_items());
        // And every spawn region with its timer, so populated areas stay populated
        // across a restart and a rare spawn's wait is not reset.
        snapshot.spawners = Some(self.spawner_records());

        // Every NPC mobile, whole, each with its carried inventory — worn gear and
        // a vendor's stock crate alike, through the same walk a character's takes.
        // The Sphere/ServUO model: the save IS the world, so a restart restores it
        // exactly and a killed creature (absent here) stays dead.
        let mobiles = self.mobile_records();
        for record in &mobiles {
            if let Some(entity) =
                Serial::new(record.serial).and_then(|s| self.state.registry.entity_of(s))
            {
                snapshot.inventories.push(Inventory {
                    owner: record.serial,
                    items: self.inventory_of(entity),
                });
            }
        }
        snapshot.mobiles = Some(mobiles);
        // And every placed decoration, door state included.
        snapshot.decorations = Some(self.decoration_records());
        // And the named regions of every facet, and the hour of the day. Neither
        // is a thing a player changes, but both are things a restart would
        // silently lose: no guards, no music, daylight in the dungeons, and every
        // night starting over.
        snapshot.regions = Some(self.region_records());
        snapshot.clock_minutes = Some(self.clock_minutes());

        // Skip only a genuinely empty save, so a quiet, empty shard queues nothing.
        let ground_empty = snapshot.ground.as_ref().is_none_or(Vec::is_empty);
        let spawners_empty = snapshot.spawners.as_ref().is_none_or(Vec::is_empty);
        let mobiles_empty = snapshot.mobiles.as_ref().is_none_or(Vec::is_empty);
        let decorations_empty = snapshot.decorations.as_ref().is_none_or(Vec::is_empty);
        let regions_empty = snapshot.regions.as_ref().is_none_or(Vec::is_empty);
        if snapshot.characters.is_empty()
            && snapshot.removed.is_empty()
            && ground_empty
            && spawners_empty
            && mobiles_empty
            && decorations_empty
            && regions_empty
        {
            return;
        }
        debug!(tick = ticks, rows = snapshot.len(), "snapshot taken");
        self.saves.push(snapshot);
    }

    /// Every item a character is carrying — worn, and inside anything worn, at any
    /// depth — as saveable records owned by that character.
    ///
    /// A breadth-first walk: the worn items first, then the contents of every
    /// container found, and their containers in turn. `owner` is the character on
    /// every record however deep, because that is the key a store replaces a whole
    /// inventory by.
    pub(super) fn inventory_of(&self, entity: EntityId) -> Vec<ItemRecord> {
        let registry = &self.state.registry;
        let Some(owner) = registry.serial_of(entity) else {
            return Vec::new();
        };
        let owner_raw = owner.raw();
        let mut records = Vec::new();
        let mut containers: Vec<Serial> = Vec::new();

        for (item, worn) in registry.query::<Equipped>() {
            if worn.mobile != owner {
                continue;
            }
            // A secure trade escrow is worn, but it is not the character's — it
            // is the transient half of a conversation, and saving it (with the
            // goods inside, since the walk below recurses into every container)
            // would restore an escrow to a trade that no longer exists and can
            // never be closed. The argument `ground_items` makes for a spell
            // field and a moongate. The items are safe because every path that
            // ends a trade puts them back in the two packs first, including the
            // logout that reaches this function.
            if registry.has::<TradeWindow>(item) {
                continue;
            }
            // The saddle *is* saved, on the mount layer like any worn item: it
            // carries the mount's graphic, and [`restore_inventory`] rebuilds the
            // ridden creature from it, so the rider logs back in still mounted.
            let location = ItemLocation::Equipped {
                mobile: owner_raw,
                layer: worn.layer,
            };
            if let Some(record) = Self::item_record(registry, item, owner_raw, location) {
                if record.container_gump.is_some() {
                    if let Some(serial) = registry.serial_of(item) {
                        containers.push(serial);
                    }
                }
                records.push(record);
            }
        }

        while let Some(container) = containers.pop() {
            for (item, held) in registry.query::<Contained>() {
                if held.container != container {
                    continue;
                }
                let location = ItemLocation::Contained {
                    container: container.raw(),
                    x: held.x,
                    y: held.y,
                    grid: held.grid,
                };
                if let Some(record) = Self::item_record(registry, item, owner_raw, location) {
                    if record.container_gump.is_some() {
                        if let Some(serial) = registry.serial_of(item) {
                            containers.push(serial);
                        }
                    }
                    records.push(record);
                }
            }
        }
        records
    }

    /// Every loose item on the ground — the dropped and the spawned, but not the
    /// [`Decoration`] a pack re-places and not a mobile — as ownerless records.
    pub(super) fn ground_items(&self) -> Vec<ItemRecord> {
        let registry = &self.state.registry;
        let mut records = Vec::new();
        for (item, Position(at)) in registry.query::<Position>() {
            // A drawable thing on the ground: a graphic, not a mobile (which carries
            // a Body), not decoration (which the pack owns and re-lays), and not a
            // field tile (transient like a cast in flight — it does not persist, and
            // restoring one would leave an eternal static that no longer expires or
            // blocks).
            //
            // A spell's gate is excluded for exactly the same reason, and ServUO
            // deletes its own on deserialise saying so: restored, a half-minute
            // portal becomes a permanent one whose caster no longer exists. The
            // eight city moongates are `Decoration` and are already out by the
            // line above, so the two cases do not collide.
            if !registry.has::<Graphic>(item)
                || registry.has::<Body>(item)
                || registry.has::<Decoration>(item)
                || registry.has::<Field>(item)
                || registry.has::<Moongate>(item)
            {
                continue;
            }
            let facet = self.state.facet_of(item);
            let location = ItemLocation::Ground {
                facet,
                x: at.x,
                y: at.y,
                z: at.z,
            };
            if let Some(record) = Self::item_record(registry, item, 0, location) {
                records.push(record);
            }
        }
        records
    }

    /// Turn one item entity into a saveable record, or `None` if it is not a
    /// drawable item (no graphic or no serial).
    pub(super) fn item_record(
        registry: &Registry,
        item: EntityId,
        owner: u32,
        location: ItemLocation,
    ) -> Option<ItemRecord> {
        let serial = registry.serial_of(item)?;
        let graphic = registry.get::<Graphic>(item)?;
        let amount = registry.get::<Amount>(item).map_or(1, |a| a.0);
        let container_gump = registry.get::<Container>(item).map(|c| c.gump);
        Some(ItemRecord {
            serial: serial.raw(),
            owner,
            graphic: graphic.id,
            hue: graphic.hue,
            amount,
            stackable: registry.has::<Stackable>(item),
            container_gump,
            // Vendor stock carries a unit price and a label; without them a
            // restored shop would sell nameless wares for a single coin.
            price: registry.get::<Price>(item).map(|p| p.0),
            name: registry.get::<Name>(item).map(|n| n.0.clone()),
            // A spellbook carries its learned spells; without the mask a
            // restored book is a graphic that no longer opens.
            spellbook: registry.get::<Spellbook>(item).map(|b| b.0),
            // And a corpse carries how it came to be one, so a shard that restarts
            // inside the seven minutes a body lies does not hand the investigator
            // an anonymous one.
            corpse: registry.get::<Corpse>(item).map(|story| CorpseData {
                owner: story.owner.clone(),
                killer: story.killer.clone(),
                examined_by: story.examined_by.clone(),
                looters: story.looters.clone(),
            }),
            // And the poison on it, bottled or smeared: all four potions are the
            // same graphic, so an unsaved bottle comes back empty.
            poison: registry
                .get::<PoisonCharges>(item)
                .map(|poison| (poison.level, poison.charges)),
            // And the trap on it, so a restart does not quietly disarm every chest
            // on the shard.
            trap: registry
                .get::<Trap>(item)
                .map(|trap| (trap_kind_code(trap.kind), trap.power, trap.level)),
            // And how much is left in a thing that wears out — a tool's swings or
            // an instrument's tunes. One field for both, as they are one interface
            // in ServUO; without it a half-played lute comes back full.
            uses: items::uses_left(registry, item),
            // And whose work it is, if it is anybody's. Without this every
            // exceptional piece on the shard quietly becomes ordinary at the
            // next restart — the `Murders` bug, over property somebody spent an
            // hour earning.
            crafted: registry
                .get::<Quality>(item)
                .map(|quality| quality.exceptional)
                .or_else(|| registry.has::<CraftedBy>(item).then_some(false))
                .map(|fine| {
                    (
                        fine,
                        registry.get::<CraftedBy>(item).map(|maker| maker.0.clone()),
                    )
                }),
            // And where a rune points, which is the whole of what a rune is —
            // an unsaved one comes back a blank, and the walk that marked it was
            // for nothing.
            rune: registry.get::<RuneMark>(item).map(|mark| {
                (
                    mark.facet,
                    mark.destination.x,
                    mark.destination.y,
                    mark.destination.z,
                )
            }),
            // And a runebook's whole contents, for the same reason over sixteen
            // times the work.
            runebook: registry.get::<Runebook>(item).map(|book| RunebookData {
                entries: book
                    .entries
                    .iter()
                    .map(|entry| RunebookEntryData {
                        facet: entry.facet,
                        x: entry.destination.x,
                        y: entry.destination.y,
                        z: entry.destination.z,
                        description: entry.description.clone(),
                    })
                    .collect(),
                charges: book.charges,
                max_charges: book.max_charges,
                default_entry: book.default_entry,
            }),
            location,
        })
    }

    /// Put a saved item's craftsmanship back: the exceptional mark, and the name
    /// of whoever made it.
    ///
    /// One helper because both restore paths — a logged-out character's inventory
    /// and the ground sweep — want the same two components, and a second copy is
    /// the pair of hand-kept halves that lets a bank-boxed masterpiece come back
    /// ordinary while a ground one does not.
    fn restore_craftsmanship(&mut self, entity: EntityId, record: &ItemRecord) {
        let Some((exceptional, ref maker)) = record.crafted else {
            return;
        };
        if exceptional {
            self.state.registry.insert(entity, Quality { exceptional });
        }
        if let Some(maker) = maker {
            self.state.registry.insert(entity, CraftedBy(maker.clone()));
        }
    }

    /// Put a saved item's travel state back: where a rune points, and what a
    /// runebook holds.
    ///
    /// One helper for the same reason [`restore_craftsmanship`] is one: both
    /// restore paths want it, and a second copy is how a rune in a bank box
    /// comes back marked while one on the floor comes back blank — a difference
    /// nothing shows until somebody casts.
    ///
    /// [`restore_craftsmanship`]: Self::restore_craftsmanship
    fn restore_travel_state(&mut self, entity: EntityId, record: &ItemRecord) {
        if let Some((facet, x, y, z)) = record.rune {
            self.state.registry.insert(
                entity,
                RuneMark {
                    facet,
                    destination: Point::new(x, y, z),
                },
            );
        }
        if let Some(book) = record.runebook.as_ref() {
            self.state.registry.insert(
                entity,
                Runebook {
                    entries: book
                        .entries
                        .iter()
                        .map(|entry| RunebookEntry {
                            facet: entry.facet,
                            destination: Point::new(entry.x, entry.y, entry.z),
                            description: entry.description.clone(),
                        })
                        .collect(),
                    charges: book.charges,
                    max_charges: book.max_charges,
                    default_entry: book.default_entry,
                    // Not saved: a couple of seconds' cooldown that a restart
                    // re-arms at zero, which errs the player's way.
                    next_use: 0,
                },
            );
        }
    }

    /// Every NPC mobile — townsperson, vendor, creature — as a saveable record:
    /// the Sphere/ServUO whole-world sweep. Players are excluded (they are
    /// [`CharacterRecord`]s), and so is a ridden mount in limbo — it has no
    /// position, and its ride persists through the saddle item instead.
    pub(super) fn mobile_records(&self) -> Vec<MobileRecord> {
        let registry = &self.state.registry;
        let mut records = Vec::new();
        for (entity, body) in registry.query::<Body>() {
            if registry.has::<Client>(entity) {
                continue;
            }
            let Some(&Position(at)) = registry.get::<Position>(entity) else {
                continue;
            };
            let Some(serial) = registry.serial_of(entity) else {
                continue;
            };
            let hits = registry
                .get::<Hitpoints>(entity)
                .copied()
                .unwrap_or(Hitpoints { current: 1, max: 1 });
            // No brain reads back as the values `spawn` builds no brain from.
            let (sight, aggression, beat, wander) =
                registry
                    .get::<Brain>(entity)
                    .map_or((0, 2, 0, false), |brain| {
                        (
                            brain.sight,
                            brain.aggression.to_bits(),
                            brain.beat_ticks,
                            brain.wander,
                        )
                    });
            let (ranged, ranged_kind) = registry
                .get::<RangedAttack>(entity)
                .map_or((0, 0), |ranged| (ranged.range, ranged.kind));
            let npc = registry.get::<Npc>(entity).copied();
            records.push(MobileRecord {
                serial: serial.raw(),
                body: body.id,
                hue: body.hue,
                facet: self.state.facet_of(entity),
                x: at.x,
                y: at.y,
                z: at.z,
                facing: registry
                    .get::<Heading>(entity)
                    .map_or(0, |heading| heading.0.to_bits()),
                name: registry.get::<Name>(entity).map(|n| n.0.clone()),
                hits_current: hits.current,
                hits_max: hits.max,
                notoriety: registry
                    .get::<Notoriety>(entity)
                    .copied()
                    .unwrap_or(Notoriety::Neutral)
                    .to_bits(),
                damage: registry.get::<MeleeDamage>(entity).map_or(0, |d| d.amount),
                resistance: registry.get::<Resistance>(entity).map_or(0, |r| r.physical),
                swing: registry.get::<SwingSpeed>(entity).map_or(0, |s| s.ticks),
                sight,
                aggression,
                beat,
                ranged,
                ranged_kind,
                wander,
                banker: registry.has::<Banker>(entity),
                vendor: registry.has::<Vendor>(entity),
                title: registry.get::<Title>(entity).map(|t| t.0.clone()),
                npc_home: npc.map(|n| (n.home.x, n.home.y, n.home.z)),
                npc_wander: npc.map_or(0, |n| n.wander),
                night_home: registry
                    .get::<NightHome>(entity)
                    .map(|h| (h.0.x, h.0.y, h.0.z)),
                // A tamed creature is property: a restart that quietly released
                // every pet on the shard would be the `Murders` lesson again.
                pet: registry.get::<Pet>(entity).map(|pet| PetData {
                    owner: pet.owner.raw(),
                    slots: pet.slots,
                    order: pet_order_code(pet.order),
                    order_target: pet.order_target.map(Serial::raw),
                }),
                restock: registry.get::<Restock>(entity).map(|shelf| RestockRecord {
                    // Seconds, not the tick: a tick counter restarts at boot, so a
                    // saved tick comes back either already due or an hour early.
                    in_seconds: shelf.at.saturating_sub(self.state.ticks) / TICKS_PER_SECOND,
                    lines: shelf
                        .lines
                        .iter()
                        .map(|l| (l.graphic, l.hue, l.amount, l.price, l.name.clone()))
                        .collect(),
                }),
                spawned_by: registry.get::<SpawnedBy>(entity).map(|s| s.0),
                effects: Self::effects_of(registry, entity, self.state.ticks),
                skills: registry.get::<Skills>(entity).map_or_else(Vec::new, |s| {
                    s.entries().map(|(id, value, _)| (id, value)).collect()
                }),
                quest_giver: registry
                    .get::<QuestGiver>(entity)
                    .map_or_else(Vec::new, |giver| giver.keys.clone()),
                escort_destination: registry
                    .get::<Escortable>(entity)
                    .map(|escort| escort.destination.clone()),
            });
        }
        records
    }

    /// Every placed decoration as a saveable record, door state included.
    pub(super) fn decoration_records(&self) -> Vec<DecorationRecord> {
        let registry = &self.state.registry;
        let mut records = Vec::new();
        for (entity, _) in registry.query::<Decoration>() {
            let Some(serial) = registry.serial_of(entity) else {
                continue;
            };
            let Some(&Graphic { id, hue }) = registry.get::<Graphic>(entity) else {
                continue;
            };
            let Some(&Position(at)) = registry.get::<Position>(entity) else {
                continue;
            };
            let door = registry.get::<Door>(entity).map(|door| DoorState {
                closed_graphic: door.closed,
                open_graphic: door.open,
                offset_x: door.offset_x,
                offset_y: door.offset_y,
                is_open: door.is_open,
            });
            records.push(DecorationRecord {
                serial: serial.raw(),
                graphic: id,
                hue,
                facet: self.state.facet_of(entity),
                x: at.x,
                y: at.y,
                z: at.z,
                door,
                container_gump: registry.get::<Container>(entity).map(|c| c.gump),
                key_value: registry
                    .get::<openshard_state::components::Lock>(entity)
                    .map_or(0, |lock| lock.key_value),
            });
        }
        records
    }

    /// What a character looks like on disk.
    ///
    /// `None` for anything that is not a character, which is not an error: the
    /// journal tracks entities and the world will hold more than people.
    /// The active effects on a mobile, as they go to disk.
    ///
    /// Poison and the stat buffs (Bless/Curse and their kin) go to disk here, so
    /// a relog carries a debuff instead of washing it off. For poison the
    /// `remaining` is the pulse count; for a timed buff it is the ticks left until
    /// it lifts, measured from `now`. A buff's `amount` is its signed stat offset.
    pub(super) fn effects_of(registry: &Registry, entity: EntityId, now: u64) -> Vec<EffectRecord> {
        let mut effects = Vec::new();
        if let Some(poison) = registry.get::<Poisoned>(entity) {
            effects.push(EffectRecord {
                kind: effect::POISON,
                amount: i16::from(poison.level),
                remaining: u16::from(poison.pulses_left),
            });
        }
        if let Some(mods) = registry.get::<StatMods>(entity) {
            for m in &mods.active {
                effects.push(EffectRecord {
                    kind: m.kind,
                    amount: m.offset,
                    remaining: m.expires_at.saturating_sub(now).min(u64::from(u16::MAX)) as u16,
                });
            }
        }
        // The non-stat behaviour buffs ride the same list — kind, magnitude, and
        // the ticks left until it lifts. A Night Sight relights on login (see the
        // enter path); the rest are read straight off the restored component.
        if let Some(buffs) = registry.get::<BehaviourBuffs>(entity) {
            for b in &buffs.active {
                effects.push(EffectRecord {
                    kind: b.kind,
                    amount: b.amount,
                    remaining: b.expires_at.saturating_sub(now).min(u64::from(u16::MAX)) as u16,
                });
            }
        }
        // Paralysis rides the same list — a relog does not thaw it.
        if let Some(frozen) = registry.get::<Frozen>(entity) {
            effects.push(EffectRecord {
                kind: effect::PARALYZE,
                amount: 0,
                remaining: frozen.until.saturating_sub(now).min(u64::from(u16::MAX)) as u16,
            });
        }
        effects
    }

    /// Put saved effects back on a restored mobile.
    ///
    /// The mirror of [`effects_of`](Self::effects_of). A stored poison becomes a
    /// live [`Poisoned`] again, its next pulse a full interval out from `now`
    /// (boot, or the tick a character logs in). A stat buff's *shift* is already
    /// folded into the saved stats and maxima, so its ledger entry is restored
    /// **without** re-applying — it only records how much to give back, and when.
    /// A tick count throughout, so a restored effect replays like decay.
    pub(super) fn apply_effects(
        registry: &mut Registry,
        entity: EntityId,
        effects: &[EffectRecord],
        now: u64,
    ) {
        let mut mods = StatMods::default();
        let mut buffs = BehaviourBuffs::default();
        for record in effects {
            if record.kind == effect::POISON {
                registry.insert(
                    entity,
                    Poisoned {
                        level: record.amount.clamp(0, i16::from(u8::MAX)) as u8,
                        next_pulse: now + combat::POISON_INTERVAL,
                        pulses_left: record.remaining.min(u16::from(u8::MAX)) as u8,
                    },
                );
            } else if matches!(
                record.kind,
                effect::STRENGTH
                    | effect::AGILITY
                    | effect::CUNNING
                    | effect::BLESS
                    | effect::WEAKEN
                    | effect::CLUMSY
                    | effect::FEEBLEMIND
                    | effect::CURSE
            ) {
                mods.active.push(StatMod {
                    kind: record.kind,
                    offset: record.amount,
                    expires_at: now + u64::from(record.remaining),
                });
            } else if matches!(
                record.kind,
                effect::NIGHT_SIGHT
                    | effect::PROTECTION
                    | effect::REACTIVE_ARMOR
                    | effect::MAGIC_REFLECT
            ) {
                // A behaviour buff nothing is folded into — just restore the
                // ledger entry with its time remaining out from `now`.
                buffs.active.push(BehaviourBuff {
                    kind: record.kind,
                    amount: record.amount,
                    expires_at: now + u64::from(record.remaining),
                });
            } else if record.kind == effect::PARALYZE {
                registry.insert(
                    entity,
                    Frozen {
                        until: now + u64::from(record.remaining),
                    },
                );
            }
            // An unrecognised kind from a newer save is skipped, not a crash.
        }
        if !mods.active.is_empty() {
            registry.insert(entity, mods);
        }
        if !buffs.active.is_empty() {
            registry.insert(entity, buffs);
        }
    }

    pub(super) fn record_of(
        registry: &Registry,
        entity: EntityId,
        now: u64,
    ) -> Option<CharacterRecord> {
        let serial = registry.serial_of(entity)?;
        let position = registry.get::<Position>(entity)?.0;
        let heading = registry.get::<Heading>(entity)?.0;
        let live_body = registry.get::<Body>(entity)?;
        // A ghost is saved as *living*: its `Ghost` marker remembers the body it
        // died in, and that is what goes on the row — the grey ghost body is
        // re-derived on login. Saving the ghost body instead would lose the living
        // one and leave a relogged ghost with nothing to resurrect back to.
        let dead = registry.get::<Ghost>(entity);
        let body = dead.map_or(*live_body, |g| g.body);
        let name = registry.get::<Name>(entity)?;
        // No account means this is not a player character — an NPC, say — so it
        // is not a `CharacterRecord`. Returning `None` drops it from the save,
        // which is the honest answer.
        let account = registry.get::<Account>(entity)?;
        let facet = registry.get::<Facet>(entity).map_or(DEFAULT_FACET, |f| f.0);
        let stats = registry.get::<Stats>(entity).copied().unwrap_or(Stats {
            strength: 100,
            dexterity: 100,
            intelligence: 100,
        });
        let skills = registry.get::<Skills>(entity).map_or_else(Vec::new, |s| {
            s.entries()
                .map(|(id, value, lock)| openshard_persistence::SkillRecord {
                    id,
                    value,
                    lock: lock.to_bits(),
                    cap: s.cap(id),
                })
                .collect()
        });
        // The stat arrows, and how long ago each stat last rose. The age is
        // relative on purpose: the tick counter restarts with the shard, so an
        // absolute stamp from the last run would sit in this run's future and
        // freeze the stat until the counter caught up.
        let locks = registry
            .get::<openshard_state::components::StatLocks>(entity)
            .copied()
            .unwrap_or_default();
        let last = registry
            .get::<openshard_state::components::LastStatGain>(entity)
            .copied()
            .unwrap_or_default();
        let age = |then: u64| now.saturating_sub(then);
        let stat_locks = openshard_persistence::StatLockRecord {
            strength: locks.strength.to_bits(),
            dexterity: locks.dexterity.to_bits(),
            intelligence: locks.intelligence.to_bits(),
            strength_age: age(last.strength),
            dexterity_age: age(last.dexterity),
            intelligence_age: age(last.intelligence),
        };
        Some(CharacterRecord {
            serial: serial.raw(),
            account: account.0.clone(),
            name: name.0.clone(),
            body: body.id,
            hue: body.hue,
            facet,
            x: position.x,
            y: position.y,
            z: position.z,
            facing: heading.to_bits(),
            strength: stats.strength,
            dexterity: stats.dexterity,
            intelligence: stats.intelligence,
            skills,
            effects: Self::effects_of(registry, entity, now),
            dead: dead.is_some(),
            fame: registry
                .get::<openshard_state::components::Fame>(entity)
                .map_or(0, |f| f.0),
            karma: registry
                .get::<openshard_state::components::Karma>(entity)
                .map_or(0, |k| k.0),
            murders: registry
                .get::<openshard_state::components::Murders>(entity)
                .map_or(0, |m| m.0),
            quests: Self::quests_of(registry, entity),
            done_quests: Self::done_quests_of(registry, entity, now),
            stat_locks,
        })
    }

    /// The quests a character has in progress, as they go to disk.
    ///
    /// Progress is stored per objective, positionally against the pack's
    /// definition — see [`QuestRecord`]. A timed objective's clock is written as
    /// the seconds *remaining*, never as the tick it ends on: the tick counter
    /// starts again from zero at every boot, so a saved deadline would mean a
    /// different moment on each restart. Same rule as `effects_of`.
    pub(super) fn quests_of(registry: &Registry, entity: EntityId) -> Vec<QuestRecord> {
        let Some(log) = registry.get::<QuestLog>(entity) else {
            return Vec::new();
        };
        log.active
            .iter()
            .map(|quest| QuestRecord {
                key: quest.key.clone(),
                progress: quest.progress.clone(),
                seconds: quest.seconds_left.clone(),
                failed: quest.failed,
                giver: quest.giver.map(Serial::raw),
            })
            .collect()
    }

    /// The quests a character has finished, with the wait before each may be
    /// taken again — again a remaining span rather than a deadline.
    pub(super) fn done_quests_of(
        registry: &Registry,
        entity: EntityId,
        now: u64,
    ) -> Vec<DoneQuestRecord> {
        let Some(log) = registry.get::<QuestLog>(entity) else {
            return Vec::new();
        };
        log.done
            .iter()
            .map(|done| DoneQuestRecord {
                key: done.key.clone(),
                restart_in_secs: if done.restart_at == u64::MAX {
                    u32::MAX // never again
                } else {
                    let ticks = done.restart_at.saturating_sub(now);
                    u32::try_from(ticks / TICKS_PER_SECOND).unwrap_or(u32::MAX)
                },
            })
            .collect()
    }

    /// Put a character's saved quests back on them, with every clock re-based on
    /// the tick it is being restored at.
    pub(super) fn apply_quests(
        registry: &mut Registry,
        entity: EntityId,
        quests: &[QuestRecord],
        done: &[DoneQuestRecord],
        now: u64,
    ) {
        if quests.is_empty() && done.is_empty() {
            return;
        }
        let log = QuestLog {
            active: quests
                .iter()
                .map(|record| QuestState {
                    key: record.key.clone(),
                    progress: record.progress.clone(),
                    seconds_left: record.seconds.clone(),
                    failed: record.failed,
                    giver: record.giver.and_then(Serial::new),
                })
                .collect(),
            done: done
                .iter()
                .map(|record| DoneQuest {
                    key: record.key.clone(),
                    restart_at: if record.restart_in_secs == u32::MAX {
                        u64::MAX
                    } else {
                        now + u64::from(record.restart_in_secs) * TICKS_PER_SECOND
                    },
                })
                .collect(),
        };
        registry.insert(entity, log);
    }

    /// Reserve a serial read from persistence so a fresh spawn never takes it.
    ///
    /// A logged-out character is not in the world — it is a row in the database —
    /// but its serial is still spoken for. Call this at boot for every stored
    /// character, before anyone can create a new one. Values outside the serial
    /// range are ignored: a corrupt row should not stop the shard from starting.
    pub fn reserve_serial(&mut self, raw: u32) {
        if let Some(serial) = Serial::new(raw) {
            self.state.registry.reserve_serial(serial);
        }
    }

    /// Bring saved items back from the store at boot.
    ///
    /// Reserves every item's serial so a live spawn cannot take it, places the
    /// loose ground items now, and files each character's carried items away by
    /// owner for [`enter`](Self::enter) to equip when that character logs in. Call
    /// once, after the map is loaded and before anyone connects.
    pub fn restore_items(&mut self, records: Vec<ItemRecord>) {
        for record in &records {
            self.reserve_serial(record.serial);
        }
        for record in records {
            if record.owner == 0 {
                self.place_ground_item(&record);
            } else {
                self.pending_inventories
                    .entry(record.owner)
                    .or_default()
                    .push(record);
            }
        }
    }

    /// Put one restored item on the ground, bound to its saved serial.
    pub(super) fn place_ground_item(&mut self, record: &ItemRecord) {
        let ItemLocation::Ground { facet, x, y, z } = record.location else {
            return;
        };
        let Some(serial) = Serial::new(record.serial) else {
            return;
        };
        let facet = if self.state.facets.contains_key(&facet) {
            facet
        } else {
            self.state.default_facet
        };
        let entity = self.state.registry.spawn();
        if self.state.registry.bind_serial(entity, serial).is_err() {
            self.state.registry.despawn(entity);
            return;
        }
        let position = Point::new(x, y, z);
        self.state.registry.insert(
            entity,
            Graphic {
                id: record.graphic,
                hue: record.hue,
            },
        );
        self.state.registry.insert(entity, Position(position));
        self.state.registry.insert(entity, Facet(facet));
        if record.amount > 1 {
            self.state.registry.insert(entity, Amount(record.amount));
        }
        if record.stackable {
            self.state.registry.insert(entity, Stackable);
        }
        if let Some(gump) = record.container_gump {
            self.state.registry.insert(entity, Container { gump });
        }
        if let Some(price) = record.price {
            self.state.registry.insert(entity, Price(price));
        }
        if let Some(name) = &record.name {
            self.state.registry.insert(entity, Name(name.clone()));
        }
        if let Some(mask) = record.spellbook {
            self.state.registry.insert(entity, Spellbook(mask));
        }
        if let Some(story) = &record.corpse {
            self.state.registry.insert(entity, corpse_from(story));
        }
        if let Some((level, charges)) = record.poison {
            self.state
                .registry
                .insert(entity, PoisonCharges { level, charges });
        }
        if let Some((kind, power, level)) = record.trap {
            self.state.registry.insert(
                entity,
                Trap {
                    kind: trap_kind_from(kind),
                    power,
                    level,
                },
            );
        }
        // The graphic says whether a saved use count is an instrument's tunes or a
        // tool's swings, so `items` decides which component it comes back as.
        if let Some(uses) = record.uses {
            items::restore_uses(&mut self.state, entity, record.graphic, uses);
        }
        self.restore_craftsmanship(entity, record);
        self.restore_travel_state(entity, record);
        // Loose clutter resumes rotting; a container does not (mark_decay skips
        // it) — except a corpse, which is a container that *must* rot, so it gets
        // a fresh timer here (the decay tick is not itself saved, so a restored
        // corpse counts down from the restore, like any restored clutter does).
        items::mark_decay(&mut self.state, entity);
        if record.graphic == openshard_state::components::CORPSE_GRAPHIC {
            self.state.registry.insert(
                entity,
                openshard_state::components::Decays {
                    at_tick: self.state.ticks + 7 * 60 * TICKS_PER_SECOND,
                },
            );
        }
        self.state
            .facet_state_mut(facet)
            .sectors
            .insert(entity, position);
    }

    /// Equip a logging-in character's saved inventory, if any is waiting.
    ///
    /// Two passes so nesting resolves whatever order the records are in: first
    /// spawn every item bound to its saved serial with its graphic and container
    /// mark, then place each — worn on the mobile, or inside the container its
    /// record names, now that every container entity exists. Returns whether an
    /// inventory was restored, so [`enter`](Self::enter) knows not to hand out a
    /// starter backpack.
    pub(super) fn restore_inventory(&mut self, owner: u32) -> bool {
        let Some(records) = self.pending_inventories.remove(&owner) else {
            return false;
        };
        // Pass one: the entities, so a container exists before its contents point
        // at it.
        for record in &records {
            let Some(serial) = Serial::new(record.serial) else {
                continue;
            };
            let entity = self.state.registry.spawn();
            if self.state.registry.bind_serial(entity, serial).is_err() {
                self.state.registry.despawn(entity);
                continue;
            }
            self.state.registry.insert(
                entity,
                Graphic {
                    id: record.graphic,
                    hue: record.hue,
                },
            );
            if record.amount > 1 {
                self.state.registry.insert(entity, Amount(record.amount));
            }
            if record.stackable {
                self.state.registry.insert(entity, Stackable);
            }
            if let Some(gump) = record.container_gump {
                self.state.registry.insert(entity, Container { gump });
            }
            if let Some(price) = record.price {
                self.state.registry.insert(entity, Price(price));
            }
            if let Some(name) = &record.name {
                self.state.registry.insert(entity, Name(name.clone()));
            }
            if let Some(mask) = record.spellbook {
                self.state.registry.insert(entity, Spellbook(mask));
            }
            if let Some(story) = &record.corpse {
                self.state.registry.insert(entity, corpse_from(story));
            }
            if let Some((level, charges)) = record.poison {
                self.state
                    .registry
                    .insert(entity, PoisonCharges { level, charges });
            }
            if let Some((kind, power, level)) = record.trap {
                self.state.registry.insert(
                    entity,
                    Trap {
                        kind: trap_kind_from(kind),
                        power,
                        level,
                    },
                );
            }
            if let Some(uses) = record.uses {
                items::restore_uses(&mut self.state, entity, record.graphic, uses);
            }
            self.restore_craftsmanship(entity, record);
            self.restore_travel_state(entity, record);
        }
        // Pass two: where each item goes.
        for record in &records {
            let Some(entity) =
                Serial::new(record.serial).and_then(|s| self.state.registry.entity_of(s))
            else {
                continue;
            };
            match record.location {
                ItemLocation::Equipped { mobile, layer } => {
                    if let Some(mobile) = Serial::new(mobile) {
                        self.state
                            .registry
                            .insert(entity, Equipped { mobile, layer });
                        // A saved mount: rebuild the ridden creature the saddle
                        // stands for and put the rider back in the saddle.
                        if layer == items::MOUNT_LAYER {
                            self.remount_saved(mobile, entity, record.graphic, record.hue);
                        }
                    }
                }
                ItemLocation::Contained {
                    container,
                    x,
                    y,
                    grid,
                } => {
                    if let Some(container) = Serial::new(container) {
                        self.state.registry.insert(
                            entity,
                            Contained {
                                container,
                                x,
                                y,
                                grid,
                            },
                        );
                    }
                }
                // An owned item is never on the ground; ignore a stray one rather
                // than drop it into the world at 0,0.
                ItemLocation::Ground { .. } => {}
            }
        }
        true
    }

    /// Bring the world's NPC mobiles back from the store at boot — each exactly
    /// as saved, wounded or well, at the tile it stood on, with its gear and a
    /// vendor's stock equipped from the already-restored item records. Call after
    /// [`restore_items`](Self::restore_items), which filed each mobile's inventory
    /// under its serial, and before anyone connects.
    ///
    /// The component list mirrors `npc::spawn` — the record-to-component
    /// conversion is exactly the seam this module exists to hold (see
    /// `persistence::record`) — with the differences a restore wants: the saved
    /// z and facing are honoured verbatim (no `stand_z` re-drop), no
    /// `MobileSpawned` is announced (the pack stocked this vendor in its first
    /// life; the stock is in the save), and a fresh stock crate is not equipped
    /// (the saved one is restored with the rest of the inventory).
    pub fn restore_mobiles(&mut self, records: Vec<MobileRecord>) {
        for record in records {
            let Some(serial) = Serial::new(record.serial) else {
                continue;
            };
            let entity = self.state.registry.spawn();
            if self.state.registry.bind_serial(entity, serial).is_err() {
                self.state.registry.despawn(entity);
                continue;
            }
            let facet = if self.state.facets.contains_key(&record.facet) {
                record.facet
            } else {
                self.state.default_facet
            };
            let position = Point::new(record.x, record.y, record.z);
            let facing = Facing::from_bits(record.facing);
            // Read before the registry is borrowed: a saved timer counts from the
            // tick the world came back at.
            let boot_ticks = self.state.ticks;
            let brain_interval = if record.beat > 0 {
                record.beat
            } else {
                self.state.gameplay.creature_step_ticks.max(1)
            };
            // And the beats, for the same borrow reason. A beat is *not* saved —
            // it is a pacing detail, not world state — so it has to be re-rolled
            // here, and re-rolling it to zero is what put every restored mobile on
            // one tick. That mattered more than it looks: `spawn` jitters the first
            // beat, but a shard is populated once and restored on every boot
            // afterwards, so the jitter ran exactly once in a shard's life and the
            // save undid it. See `npc::first_beat`.
            let first_think =
                openshard_npc::first_beat(&mut self.state.rng, boot_ticks, brain_interval);
            let first_beat = openshard_npc::first_beat(
                &mut self.state.rng,
                boot_ticks,
                openshard_npc::BEAT_TICKS,
            );
            let registry = &mut self.state.registry;
            registry.insert(
                entity,
                Body {
                    id: record.body,
                    hue: record.hue,
                },
            );
            registry.insert(entity, Position(position));
            registry.insert(entity, Heading(facing));
            registry.insert(entity, Facet(facet));
            registry.insert(
                entity,
                Hitpoints {
                    current: record.hits_current.max(1),
                    max: record.hits_max.max(1),
                },
            );
            registry.insert(entity, Notoriety::from_bits(record.notoriety));
            registry.insert(
                entity,
                MeleeDamage {
                    amount: record.damage,
                },
            );
            registry.insert(
                entity,
                Resistance {
                    physical: record.resistance.min(100),
                    ..Default::default()
                },
            );
            if record.swing != 0 {
                registry.insert(
                    entity,
                    SwingSpeed {
                        ticks: record.swing,
                    },
                );
            }
            if record.ranged > 0 {
                registry.insert(
                    entity,
                    RangedAttack {
                        range: record.ranged,
                        kind: record.ranged_kind,
                    },
                );
            }
            // The same brain rule `spawn` applies: anything that hunts, drifts,
            // or must answer or flee a blow.
            let aggression = Aggression::from_bits(record.aggression);
            if record.sight > 0 || record.wander || aggression != Aggression::Aggressive {
                registry.insert(
                    entity,
                    Brain {
                        sight: record.sight,
                        wander: record.wander,
                        next_think: first_think,
                        guard_until: 0,
                        opens_doors: body_opens_doors(record.body),
                        aggression,
                        beat_ticks: record.beat,
                    },
                );
            }
            if let Some(name) = record.name {
                registry.insert(entity, Name(name));
            }
            if record.banker {
                registry.insert(entity, Banker);
            }
            if record.vendor {
                registry.insert(entity, Vendor);
            }
            // The trade, without which a restored NPC is a mute statue: every
            // keyword it answers is looked up by this string.
            if let Some(title) = record.title {
                registry.insert(entity, Title(title));
            }
            if let Some(pet) = &record.pet {
                if let Some(owner) = Serial::new(pet.owner) {
                    registry.insert(
                        entity,
                        Pet {
                            owner,
                            slots: pet.slots,
                            order: pet_order_from(pet.order),
                            order_target: pet.order_target.and_then(Serial::new),
                        },
                    );
                }
            }
            if let Some((x, y, z)) = record.night_home {
                registry.insert(entity, NightHome(Point::new(x, y, z)));
            }
            if let Some(shelf) = record.restock {
                registry.insert(
                    entity,
                    Restock {
                        at: boot_ticks + shelf.in_seconds * TICKS_PER_SECOND,
                        lines: shelf
                            .lines
                            .into_iter()
                            .map(|(graphic, hue, amount, price, name)| StockRecord {
                                graphic,
                                hue,
                                amount,
                                price,
                                name,
                            })
                            .collect(),
                    },
                );
            }
            if let Some((x, y, z)) = record.npc_home {
                registry.insert(
                    entity,
                    Npc {
                        home: Point::new(x, y, z),
                        wander: record.npc_wander,
                        next_beat: first_beat,
                        // Eligible to greet at once, which is not the same as
                        // greeting at once: `attend` rolls for it, and the beats
                        // above no longer arrive together anyway.
                        next_greet: 0,
                    },
                );
            }
            // Re-tie it to the region that maintains it, so the region counts it
            // and does not spawn over it.
            if let Some(region) = record.spawned_by {
                registry.insert(entity, SpawnedBy(region));
            }
            registry.insert(entity, Movement(Walker::new(position, facing)));
            // A wounded creature comes back wounded; a poisoned one comes back
            // poisoned. Its pulses resume at boot's tick.
            let now = self.state.ticks;
            Self::apply_effects(&mut self.state.registry, entity, &record.effects, now);
            // Its combat skills come back too, so a restored monster still rolls to
            // hit and scales damage rather than reverting to a skill-less brawler.
            if !record.skills.is_empty() {
                let mut sheet = Skills::default();
                for (id, value) in &record.skills {
                    sheet.set(*id, *value);
                }
                self.state.registry.insert(entity, sheet);
            }
            self.state
                .facet_state_mut(facet)
                .sectors
                .insert(entity, position);
            // Whether it gives quests, and whether it can be escorted. This is
            // the binding that used to live only in the script's memory and so
            // was lost at every restart.
            if !record.quest_giver.is_empty() {
                self.state.registry.insert(
                    entity,
                    QuestGiver {
                        keys: record.quest_giver.clone(),
                    },
                );
            }
            if let Some(destination) = record.escort_destination.clone() {
                self.state.registry.insert(
                    entity,
                    Escortable {
                        destination,
                        // Nobody is leading it at boot: an escort in progress ends
                        // with the session it was walked in, the way a cast in
                        // flight or a swing timer does.
                        escorter: None,
                        last_seen: 0,
                    },
                );
            }
            // Its gear and stock were filed by `restore_items` under this serial.
            self.restore_inventory(record.serial);
            // And say it is back. Not a `MobileSpawned` — see `MobileRestored` for
            // why the two must not be one event — but not silence either, or every
            // rule bound to this NPC by whoever placed it stays unbound for the
            // rest of the shard's life.
            self.state.bus.send(crate::events::MobileRestored {
                entity,
                serial,
                body: record.body,
                at: position,
                // Its post, which is what a pack keys a binding on — see the
                // event's own doc. `position` is wherever it had wandered to when
                // the save was taken, and with a daily routine that is somewhere
                // else entirely for a third of the day.
                home: record
                    .npc_home
                    .map_or(position, |(x, y, z)| Point::new(x, y, z)),
            });
        }
    }

    /// Re-lay the saved decoration at boot: statics, doors (their open/shut state
    /// honoured) and town containers, each re-registered with the obstruction
    /// index over its own z-span — the part [`place_ground_item`](Self::place_ground_item)
    /// never does, and why decoration cannot ride the ground-item path.
    pub fn restore_decorations(&mut self, records: Vec<DecorationRecord>) {
        for record in records {
            let Some(serial) = Serial::new(record.serial) else {
                continue;
            };
            let entity = self.state.registry.spawn();
            if self.state.registry.bind_serial(entity, serial).is_err() {
                self.state.registry.despawn(entity);
                continue;
            }
            let facet = if self.state.facets.contains_key(&record.facet) {
                record.facet
            } else {
                self.state.default_facet
            };
            let position = Point::new(record.x, record.y, record.z);
            self.state.registry.insert(
                entity,
                Graphic {
                    id: record.graphic,
                    hue: record.hue,
                },
            );
            self.state.registry.insert(entity, Position(position));
            self.state.registry.insert(entity, Facet(facet));
            self.state.registry.insert(entity, Decoration);
            // The lock, on either kind: a door that was locked at the save comes back
            // locked, or a shard's set-piece unbars itself at every reboot.
            if record.key_value != 0 {
                self.state.registry.insert(
                    entity,
                    openshard_state::components::Lock {
                        key_value: record.key_value,
                        ..Default::default()
                    },
                );
            }
            if let Some(gump) = record.container_gump {
                self.state.registry.insert(entity, Container { gump });
            }
            match record.door {
                Some(door) => {
                    self.state.registry.insert(
                        entity,
                        Door {
                            closed: door.closed_graphic,
                            open: door.open_graphic,
                            offset_x: door.offset_x,
                            offset_y: door.offset_y,
                            is_open: door.is_open,
                            close_at: 0,
                        },
                    );
                    // A shut door seals its doorway again; an open one blocks
                    // nobody until it swings shut.
                    if !door.is_open {
                        self.state.facet_state_mut(facet).obstructions.block(
                            position.x,
                            position.y,
                            entity,
                            true,
                            position.z,
                            openshard_state::DOOR_HEIGHT,
                        );
                    }
                }
                None => {
                    // Plain art blocks over its tiledata z-span, exactly as
                    // `place_decoration` registered it the first time.
                    let height = self
                        .state
                        .facet_state(facet)
                        .terrain
                        .as_deref()
                        .filter(|t| t.item_blocks(record.graphic))
                        .map(|t| t.item_height(record.graphic));
                    if let Some(height) = height {
                        self.state
                            .facet_state_mut(facet)
                            .obstructions
                            .block(position.x, position.y, entity, false, position.z, height);
                    }
                }
            }
            self.state
                .facet_state_mut(facet)
                .sectors
                .insert(entity, position);
        }
    }

    /// Rebuild a saved ride: recreate the ridden creature the mount item was
    /// drawn as, and put its rider back in the saddle, so a character that logged
    /// out mounted logs back in mounted. The creature lives only in limbo (no
    /// position) until the rider dismounts, exactly as a live mount does — its
    /// stats do not matter while ridden, so a fresh serial and the body the
    /// saddle names are all it needs.
    fn remount_saved(&mut self, rider_serial: Serial, item: EntityId, graphic: u16, hue: u16) {
        let Some(rider) = self.state.registry.entity_of(rider_serial) else {
            return;
        };
        let Some(body) = openshard_state::components::mount_body_for(graphic) else {
            return;
        };
        let Ok((mount, _)) = self.state.registry.spawn_with_serial(SerialKind::Mobile) else {
            return;
        };
        let facet = self.state.facet_of(rider);
        self.state.registry.insert(mount, Body { id: body, hue });
        self.state.registry.insert(mount, Facet(facet));
        self.state.registry.insert(mount, Ridden { rider });
        self.state.registry.insert(rider, Riding { mount, item });
    }
}

/// A saved corpse story back into the component. The two shapes are deliberately
/// separate types (see `persistence::record`), so one conversion, in one place.
fn corpse_from(story: &CorpseData) -> Corpse {
    Corpse {
        owner: story.owner.clone(),
        killer: story.killer.clone(),
        examined_by: story.examined_by.clone(),
        looters: story.looters.clone(),
    }
}

/// A trap kind as one saved byte, and back. Written out rather than derived so the
/// on-disk numbering cannot drift when the enum gains a variant — the same reason
/// the effect kinds are numbered by hand in `state::effect`.
const fn trap_kind_code(kind: TrapKind) -> u8 {
    match kind {
        TrapKind::Magic => 0,
        TrapKind::Explosion => 1,
        TrapKind::Dart => 2,
        TrapKind::Poison => 3,
    }
}

/// The inverse. An unknown code reads as a magic trap rather than dropping the
/// trap entirely: a chest that quietly stops being trapped is the failure this
/// column exists to prevent.
const fn trap_kind_from(code: u8) -> TrapKind {
    match code {
        1 => TrapKind::Explosion,
        2 => TrapKind::Dart,
        3 => TrapKind::Poison,
        _ => TrapKind::Magic,
    }
}

/// A pet's standing order as one saved byte, and back. Written out by hand for the
/// same reason the trap kinds and the effect kinds are: the on-disk numbering must
/// not drift when the enum gains a variant.
const fn pet_order_code(order: PetOrder) -> u8 {
    match order {
        PetOrder::Follow => 0,
        PetOrder::Come => 1,
        PetOrder::Stay => 2,
        PetOrder::Guard => 3,
        PetOrder::Attack => 4,
        PetOrder::Stop => 5,
    }
}

/// The inverse. An unknown code reads as "follow", the harmless order.
const fn pet_order_from(code: u8) -> PetOrder {
    match code {
        1 => PetOrder::Come,
        2 => PetOrder::Stay,
        3 => PetOrder::Guard,
        4 => PetOrder::Attack,
        5 => PetOrder::Stop,
        _ => PetOrder::Follow,
    }
}
