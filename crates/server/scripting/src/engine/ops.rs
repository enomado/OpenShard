use super::*;
use openshard_protocol::mobile::Notoriety;
use openshard_protocol::world::{
    Aggression, DamageType, PhysicalResistance, PoisonLevel, RangedRange, Sight,
};

/// serial the engine has never been told about.
///
/// A direct read — the "look at the world" half of the contract. Not a fast op
/// because it returns a structured value; that cost is measured in the
/// benchmark and is the honest cost of a hook that reads state.
#[op2]
#[serde]
fn op_position(state: &mut OpState, serial: u32) -> Option<[i32; 3]> {
    state
        .borrow::<Host>()
        .entities
        .get(&serial)
        .map(|v| [v.x as i32, v.y as i32, v.z as i32])
}

/// Enqueue a move for the world to apply on its next tick.
///
/// The "change it only by asking" half. A fast op: no allocation, no return
/// value, just a push onto the outbox the engine drains after the hooks run.
#[op2(fast)]
fn op_move(state: &mut OpState, serial: u32, direction: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::Move {
        serial,
        direction: direction as u8,
    });
}

/// What a script passes to spawn an item — a plain object, so the JS reads
/// `op_spawn_item({ graphic, x, y })` rather than seven positional arguments,
/// most of which have sensible defaults.
#[derive(serde::Deserialize)]
struct SpawnSpec {
    graphic: u16,
    #[serde(default)]
    hue: u16,
    #[serde(default = "one")]
    amount: u16,
    #[serde(default)]
    stackable: bool,
    x: u16,
    y: u16,
    #[serde(default)]
    z: i8,
    #[serde(default)]
    facet: Facet,
}

/// The serde default for a spec's `aggression`: aggressive, the old behaviour.
/// The default stack size: a single item.
fn one() -> u16 {
    1
}

/// The serde default for a townsperson's `shoe`: plain shoes, `BaseVendor`'s own
/// default `ShoeType`.
fn shoes() -> u8 {
    1
}

/// Put an item on the ground. Enqueues a command; the world creates the entity
/// and draws it on the tick that applies it.
#[op2]
fn op_spawn_item(state: &mut OpState, #[serde] spec: SpawnSpec) {
    state.borrow_mut::<Host>().outbox.push(Command::SpawnItem {
        graphic: spec.graphic,
        hue: spec.hue,
        amount: spec.amount,
        stackable: spec.stackable,
        x: spec.x,
        y: spec.y,
        z: spec.z,
        facet: spec.facet,
    });
}

/// What a script passes to spawn a container: a [`SpawnSpec`] plus the gump the
/// client opens for it.
#[derive(serde::Deserialize)]
struct ContainerSpec {
    graphic: u16,
    gump: u16,
    #[serde(default)]
    hue: u16,
    x: u16,
    y: u16,
    #[serde(default)]
    z: i8,
    #[serde(default)]
    facet: Facet,
}

/// Put a container on the ground.
#[op2]
fn op_spawn_container(state: &mut OpState, #[serde] spec: ContainerSpec) {
    state.borrow_mut::<Host>().outbox.push(Command::SpawnContainer {
        graphic: spec.graphic,
        gump: spec.gump,
        hue: spec.hue,
        x: spec.x,
        y: spec.y,
        z: spec.z,
        facet: spec.facet,
    });
}

/// What a script passes to spawn a mobile.
#[derive(serde::Deserialize)]
struct MobileSpec {
    body: u16,
    #[serde(default)]
    hue: u16,
    #[serde(default = "one")]
    hits: u16,
    #[serde(default)]
    notoriety: Notoriety,
    #[serde(default)]
    damage: u16,
    #[serde(default)]
    resistance: PhysicalResistance,
    #[serde(default)]
    swing: u64,
    #[serde(default)]
    sight: Sight,
    #[serde(default)]
    aggression: Aggression,
    #[serde(default)]
    beat: u64,
    #[serde(default, with = "openshard_protocol::world::ranged")]
    ranged: Option<RangedRange>,
    #[serde(default)]
    ranged_kind: DamageType,
    #[serde(default)]
    wander: bool,
    x: u16,
    y: u16,
    #[serde(default)]
    z: i8,
    #[serde(default)]
    facet: Facet,
    #[serde(default)]
    name: String,
    /// The trade, ServUO-style ("the blacksmith"). The key its dress, its name and
    /// its speech all hang off.
    #[serde(default)]
    title: String,
    /// The trade's footwear, `ShoeType`'s wire byte; defaults to plain shoes.
    #[serde(default = "shoes")]
    shoe: u8,
    /// How widely known it is; a killer inherits it.
    #[serde(default)]
    fame: i32,
    /// Which way it is known. Negative is evil.
    #[serde(default)]
    karma: i32,
    /// Where it sleeps, `[x, y, z]`. Only read with `gameplay.npc_schedule` on.
    #[serde(default)]
    night_home: Option<(u16, u16, i8)>,
    #[serde(default)]
    banker: bool,
    #[serde(default)]
    vendor: bool,
    #[serde(default)]
    healer: bool,
    #[serde(default)]
    equipment: Vec<WornItemSpec>,
    #[serde(default)]
    skills: Vec<SkillSpec>,
}

/// One worn item in a [`MobileSpec`]: `{ graphic, layer, hue }`.
#[derive(serde::Deserialize)]
struct WornItemSpec {
    graphic: u16,
    layer: u8,
    #[serde(default)]
    hue: u16,
}

/// One trained skill in a [`MobileSpec`]: `{ id, value }`, value in tenths.
#[derive(serde::Deserialize)]
struct SkillSpec {
    id: u8,
    value: u16,
}

/// Put a creature or NPC in the world.
#[op2]
fn op_spawn_mobile(state: &mut OpState, #[serde] spec: MobileSpec) {
    let equipment = spec
        .equipment
        .into_iter()
        .map(|w| crate::WornItem {
            graphic: w.graphic,
            layer: w.layer,
            hue: w.hue,
        })
        .collect();
    let skills = spec.skills.into_iter().map(|s| (s.id, s.value)).collect();
    state.borrow_mut::<Host>().outbox.push(Command::SpawnMobile {
        body: spec.body,
        hue: spec.hue,
        hits: spec.hits,
        notoriety: spec.notoriety,
        damage: spec.damage,
        resistance: spec.resistance,
        swing: spec.swing,
        sight: spec.sight,
        aggression: spec.aggression,
        beat: spec.beat,
        ranged: spec.ranged,
        ranged_kind: spec.ranged_kind,
        wander: spec.wander,
        x: spec.x,
        y: spec.y,
        z: spec.z,
        facet: spec.facet,
        name: spec.name,
        title: spec.title,
        shoe: spec.shoe,
        fame: spec.fame,
        karma: spec.karma,
        night_home: spec.night_home,
        banker: spec.banker,
        vendor: spec.vendor,
        healer: spec.healer,
        equipment,
        skills,
    });
}

/// Deal damage to a mobile, of a kind (0 physical, 1 fire, …).
#[op2(fast)]
fn op_damage(state: &mut OpState, serial: u32, amount: u32, damage_type: u32, by: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::Damage {
        serial,
        amount: amount.min(u32::from(u16::MAX)) as u16,
        damage_type: damage_type.min(u32::from(u8::MAX)) as u8,
        by,
    });
}

/// Heal a mobile, up to its maximum.
#[op2(fast)]
fn op_heal(state: &mut OpState, serial: u32, amount: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::Heal {
        serial,
        amount: amount.min(u32::from(u16::MAX)) as u16,
    });
}

/// What a script passes to cast a spell — a plain object, so reagents can be a
/// list and the fields that default (target, the skill band, pack, reagents) can
/// be left off.
#[derive(serde::Deserialize)]
struct CastSpec {
    serial: u32,
    spell: u16,
    #[serde(default)]
    target: u32,
    mana: u16,
    /// The skill band, in tenths. Both default to zero, which is a band with no
    /// width: the cast always succeeds and never trains — right for a script that
    /// only wants the mana and reagent gate.
    #[serde(default)]
    min_skill: i32,
    #[serde(default)]
    max_skill: i32,
    skill: u8,
    #[serde(default)]
    pack: u32,
    /// `(graphic, count)` pairs the spell consumes from `pack`.
    #[serde(default)]
    reagents: Vec<(u16, u16)>,
}

/// Cast a spell. The outcome comes back as a `SpellCast` event, not a return —
/// the mana, reagents and skill roll happen on the tick.
#[op2]
fn op_cast_spell(state: &mut OpState, #[serde] spec: CastSpec) {
    state.borrow_mut::<Host>().outbox.push(Command::CastSpell {
        serial: spec.serial,
        spell: spec.spell,
        target: spec.target,
        mana: spec.mana,
        min_skill: spec.min_skill,
        max_skill: spec.max_skill,
        skill: spec.skill,
        pack: spec.pack,
        reagents: spec.reagents,
    });
}

/// Set a mobile's skill value, in tenths.
#[op2(fast)]
fn op_set_skill(state: &mut OpState, serial: u32, skill: u32, value: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::SetSkill {
        serial,
        skill: skill as u8,
        value: value.min(u32::from(u16::MAX)) as u16,
    });
}

/// Override a weapon item's speed and damage — the pack's magic sword.
#[op2(fast)]
fn op_set_weapon(state: &mut OpState, serial: u32, speed: u32, min: u32, max: u32) {
    let clamp = |v: u32| v.min(u32::from(u16::MAX)) as u16;
    state.borrow_mut::<Host>().outbox.push(Command::SetWeapon {
        serial,
        speed: clamp(speed),
        min: clamp(min),
        max: clamp(max),
    });
}

/// Put poison on an item — a bottled dose, or a coating on a blade.
#[op2(fast)]
fn op_set_poison(state: &mut OpState, serial: u32, level: u32, charges: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::SetPoison {
        serial,
        level: PoisonLevel::new(level.min(u32::from(u8::MAX)) as u8),
        charges: charges.min(u32::from(u16::MAX)) as u16,
    });
}

/// Set a mobile's stats; strength re-caps hits, intelligence re-caps mana.
#[op2(fast)]
fn op_set_stats(state: &mut OpState, serial: u32, strength: u32, dexterity: u32, intelligence: u32) {
    let clamp = |v: u32| v.min(u32::from(u16::MAX)) as u16;
    state.borrow_mut::<Host>().outbox.push(Command::SetStats {
        serial,
        strength: clamp(strength),
        dexterity: clamp(dexterity),
        intelligence: clamp(intelligence),
    });
}

/// Use a skill against a difficulty band, both edges in tenths. The result comes
/// back as a `SkillUsed` event, not a return value: the roll and any gain happen
/// on the tick, not in the op.
#[op2(fast)]
fn op_use_skill(state: &mut OpState, serial: u32, skill: u32, min_skill: i32, max_skill: i32) {
    state.borrow_mut::<Host>().outbox.push(Command::UseSkill {
        serial,
        skill: skill as u8,
        min_skill,
        max_skill,
    });
}

/// Make a mobile speak — an NPC's line, a keyword answer.
#[op2(fast)]
fn op_say(state: &mut OpState, serial: u32, #[string] text: String, hue: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::Speak {
        serial,
        hue: hue.min(u32::from(u16::MAX)) as u16,
        text,
    });
}

/// Take control of a mobile: from now on its `onTick` runs it, not the built-in
/// brain. The world starts handing it to this script each tick.
#[op2(fast)]
fn op_control(state: &mut OpState, serial: u32) {
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::Control { serial });
}

/// Show a gump (a dialog window) to a mobile's client:
/// `op_gump({ serial, gumpId, x, y, layout, lines })`. The reply returns as a
/// `GumpAnswered` event keyed on `gumpId`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GumpSpec {
    serial: u32,
    gump_id: u32,
    #[serde(default)]
    x: u16,
    #[serde(default)]
    y: u16,
    layout: String,
    #[serde(default)]
    lines: Vec<String>,
}

/// Send a pack-built gump to a mobile's client.
#[op2]
fn op_gump(state: &mut OpState, #[serde] spec: GumpSpec) {
    state.borrow_mut::<Host>().outbox.push(Command::ShowGump {
        serial: spec.serial,
        gump_id: spec.gump_id,
        x: spec.x,
        y: spec.y,
        layout: spec.layout,
        lines: spec.lines,
    });
}

/// One objective in an [`op_register_quests`] batch.
#[derive(serde::Deserialize)]
struct ObjectiveSpec {
    kind: String,
    #[serde(default)]
    target: u16,
    #[serde(default = "one")]
    count: u16,
    #[serde(default)]
    name: String,
    #[serde(default)]
    destination: String,
    #[serde(default)]
    seconds: u32,
}

/// One reward in an [`op_register_quests`] batch. Gold when `gold` is set,
/// otherwise the item the rest describes.
#[derive(serde::Deserialize)]
struct RewardSpec {
    #[serde(default)]
    name: String,
    #[serde(default)]
    gold: u32,
    #[serde(default)]
    graphic: u16,
    #[serde(default)]
    hue: u16,
    #[serde(default = "one")]
    amount: u16,
    #[serde(default)]
    stackable: bool,
}

/// One quest in an [`op_register_quests`] batch.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuestSpec {
    key: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    refuse: String,
    #[serde(default)]
    uncomplete: String,
    #[serde(default)]
    complete: String,
    #[serde(default)]
    failed: String,
    #[serde(default)]
    objectives: Vec<ObjectiveSpec>,
    #[serde(default)]
    rewards: Vec<RewardSpec>,
    #[serde(default = "yes")]
    all_objectives: bool,
    #[serde(default)]
    done_once: bool,
    #[serde(default)]
    restart_delay_secs: u32,
}

/// The whole quest list.
#[derive(serde::Deserialize)]
struct QuestsSpec {
    quests: Vec<QuestSpec>,
}

const fn yes() -> bool {
    true
}

/// What a script passes to register townsfolk speech.
#[derive(serde::Deserialize)]
struct NpcSpeechSpec {
    /// One entry per trade, keyed by the title its NPCs wear.
    #[serde(default)]
    trades: Vec<TradeSpeechSpec>,
    /// Personal names for male NPCs; empty keeps the core's list.
    #[serde(default)]
    male_names: Vec<String>,
    /// Personal names for female NPCs; empty keeps the core's list.
    #[serde(default)]
    female_names: Vec<String>,
}

/// One trade's speech in a [`NpcSpeechSpec`].
#[derive(serde::Deserialize)]
struct TradeSpeechSpec {
    title: String,
    #[serde(default)]
    greetings: Vec<String>,
    #[serde(default)]
    barks: Vec<String>,
    #[serde(default)]
    entries: Vec<SpeechEntrySpec>,
    #[serde(default)]
    fallback: String,
}

/// One keyword group in a [`TradeSpeechSpec`].
#[derive(serde::Deserialize)]
struct SpeechEntrySpec {
    keywords: Vec<String>,
    lines: Vec<String>,
}

/// Register what every trade says, replacing whatever was registered before:
/// `op_register_npc_speech({ trades: [...], male_names: [...], female_names: [...] })`.
///
/// Called at pack load time, like `op_register_quests`; a hot reload re-registers
/// cleanly. The keys are the titles NPCs are spawned with.
#[op2]
fn op_register_npc_speech(state: &mut OpState, #[serde] spec: NpcSpeechSpec) {
    let trades = spec
        .trades
        .into_iter()
        .map(|trade| crate::ScriptTradeSpeech {
            title: trade.title,
            greetings: trade.greetings,
            barks: trade.barks,
            entries: trade
                .entries
                .into_iter()
                .map(|entry| crate::ScriptSpeechEntry {
                    keywords: entry.keywords,
                    lines: entry.lines,
                })
                .collect(),
            fallback: trade.fallback,
        })
        .collect();
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::RegisterNpcSpeech {
            trades,
            male_names: spec.male_names,
            female_names: spec.female_names,
        });
}

/// Register the shard's quests, replacing whatever was registered before:
/// `op_register_quests({ quests: [...] })`. Called at pack load time; a hot
/// reload re-registers cleanly.
#[op2]
fn op_register_quests(state: &mut OpState, #[serde] spec: QuestsSpec) {
    let quests = spec
        .quests
        .into_iter()
        .map(|quest| crate::ScriptQuest {
            key: quest.key,
            title: quest.title,
            description: quest.description,
            refuse: quest.refuse,
            uncomplete: quest.uncomplete,
            complete: quest.complete,
            failed: quest.failed,
            objectives: quest
                .objectives
                .into_iter()
                .map(|objective| crate::ScriptObjective {
                    kind: objective.kind,
                    target: objective.target,
                    count: objective.count,
                    name: objective.name,
                    destination: objective.destination,
                    seconds: objective.seconds,
                })
                .collect(),
            rewards: quest
                .rewards
                .into_iter()
                .map(|reward| crate::ScriptReward {
                    name: reward.name,
                    gold: reward.gold,
                    graphic: reward.graphic,
                    hue: reward.hue,
                    amount: reward.amount,
                    stackable: reward.stackable,
                })
                .collect(),
            all_objectives: quest.all_objectives,
            done_once: quest.done_once,
            restart_delay_secs: quest.restart_delay_secs,
        })
        .collect();
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::RegisterQuests { quests });
}

/// Make an NPC a quest giver: `op_bind_quest_giver(serial, ["rat_cull"])`.
/// The binding is saved with the mobile, so it survives a restart — which is
/// exactly what a binding held in script memory did not.
#[op2]
fn op_bind_quest_giver(state: &mut OpState, serial: u32, #[serde] keys: Vec<String>) {
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::BindQuestGiver { serial, keys });
}

/// Make an NPC escortable: `op_make_escortable(serial, "Britain")`. An empty
/// destination lets the quest that starts the escort choose one. Saved with the
/// mobile.
#[op2(fast)]
fn op_make_escortable(state: &mut OpState, serial: u32, #[string] destination: String) {
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::MakeEscortable { serial, destination });
}

/// Close an open gump on a player's client: `op_close_gump(serial, gumpId)`.
/// A dialog that reopens on another page must close the old window first, or the
/// client stacks them.
#[op2(fast)]
fn op_close_gump(state: &mut OpState, serial: u32, gump_id: u32) {
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::CloseGump { serial, gump_id });
}

/// Send a player a private system line: `op_message(serial, text)`. The server
/// talking to one person — unlike `op_say`, which puts words over a mobile's head
/// for everyone in earshot.
#[op2(fast)]
fn op_message(state: &mut OpState, serial: u32, #[string] text: String) {
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::Message { serial, text });
}

/// Play a sound for one player: `op_play_sound(serial, sound)` — feedback on
/// something only they did.
#[op2(fast)]
fn op_play_sound(state: &mut OpState, serial: u32, sound: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::PlaySound {
        serial,
        sound: sound.min(u32::from(u16::MAX)) as u16,
    });
}

/// A quest reward or handout: `op_give_item({ serial, graphic, hue, amount,
/// stackable })` — dropped into the player's backpack.
#[derive(serde::Deserialize)]
struct GiveSpec {
    serial: u32,
    graphic: u16,
    #[serde(default)]
    hue: u16,
    #[serde(default = "one")]
    amount: u16,
    #[serde(default)]
    stackable: bool,
}

/// Put an item into a player's backpack.
#[op2]
fn op_give_item(state: &mut OpState, #[serde] spec: GiveSpec) {
    state.borrow_mut::<Host>().outbox.push(Command::GiveItem {
        serial: spec.serial,
        graphic: spec.graphic,
        hue: spec.hue,
        amount: spec.amount,
        stackable: spec.stackable,
    });
}

/// Take up to `amount` of a graphic from a player's backpack — a quest's "collect
/// N" turn-in. All-or-nothing; the result returns as an `ItemsTaken` event.
#[op2(fast)]
fn op_take_item(state: &mut OpState, serial: u32, graphic: u32, amount: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::TakeItem {
        serial,
        graphic: graphic.min(u32::from(u16::MAX)) as u16,
        amount: amount.min(u32::from(u16::MAX)) as u16,
    });
}

/// One creature template inside a [`SpawnerSpec`]. Mirrors [`MobileSpec`] minus
/// the position, which the region supplies.
#[derive(serde::Deserialize)]
struct CreatureSpec {
    body: u16,
    #[serde(default)]
    hue: u16,
    #[serde(default = "one")]
    hits: u16,
    #[serde(default)]
    notoriety: Notoriety,
    #[serde(default)]
    damage: u16,
    #[serde(default)]
    resistance: PhysicalResistance,
    #[serde(default)]
    fame: i32,
    #[serde(default)]
    karma: i32,
    #[serde(default)]
    swing: u64,
    #[serde(default)]
    sight: Sight,
    #[serde(default)]
    aggression: Aggression,
    #[serde(default)]
    beat: u64,
    #[serde(default, with = "openshard_protocol::world::ranged")]
    ranged: Option<RangedRange>,
    #[serde(default)]
    ranged_kind: DamageType,
    #[serde(default)]
    wander: bool,
    #[serde(default)]
    skills: Vec<SkillSpec>,
}

/// A spawn region, from the script: `op_register_spawner({ x, y, width, height,
/// maxCount, respawnDelay, creatures: [...] })`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpawnerSpec {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    #[serde(default)]
    facet: Facet,
    max_count: u16,
    respawn_delay: u64,
    creatures: Vec<CreatureSpec>,
}

/// Register a spawn region the world will keep populated.
#[op2]
fn op_register_spawner(state: &mut OpState, #[serde] spec: SpawnerSpec) {
    let creatures = spec
        .creatures
        .into_iter()
        .map(|c| crate::SpawnCreature {
            body: c.body,
            hue: c.hue,
            hits: c.hits,
            notoriety: c.notoriety,
            damage: c.damage,
            resistance: c.resistance,
            fame: c.fame,
            karma: c.karma,
            swing: c.swing,
            sight: c.sight,
            aggression: c.aggression,
            beat: c.beat,
            ranged: c.ranged,
            ranged_kind: c.ranged_kind,
            wander: c.wander,
            skills: c.skills.into_iter().map(|s| (s.id, s.value)).collect(),
        })
        .collect();
    state.borrow_mut::<Host>().outbox.push(Command::RegisterSpawner {
        x: spec.x,
        y: spec.y,
        width: spec.width,
        height: spec.height,
        facet: spec.facet,
        max_count: spec.max_count,
        respawn_delay: spec.respawn_delay,
        creatures,
    });
}

/// Remove every spawn region and the creatures they were maintaining.
#[op2(fast)]
fn op_clear_spawners(state: &mut OpState) {
    state.borrow_mut::<Host>().outbox.push(Command::ClearSpawners);
}

/// One rectangle of a [`RegionSpec`]. `zMin`/`zMax` default to the whole column,
/// which is what a `<rect>` with no `zrange` means in the data this comes from.
#[derive(serde::Deserialize)]
struct RegionRectSpec {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    #[serde(default = "z_floor", rename = "zMin")]
    z_min: i8,
    #[serde(default = "z_ceiling", rename = "zMax")]
    z_max: i8,
}

const fn z_floor() -> i8 {
    i8::MIN
}
const fn z_ceiling() -> i8 {
    i8::MAX
}

/// One named area in an [`op_register_regions`] batch.
#[derive(serde::Deserialize)]
struct RegionSpec {
    name: String,
    #[serde(default)]
    priority: u8,
    rects: Vec<RegionRectSpec>,
    #[serde(default)]
    guarded: bool,
    #[serde(default, rename = "noTeleport")]
    no_teleport: bool,
    #[serde(default, rename = "noRecall")]
    no_recall: bool,
    #[serde(default, rename = "noHousing")]
    no_housing: bool,
    #[serde(default)]
    safe: bool,
    #[serde(default)]
    music: Option<u16>,
    #[serde(default)]
    light: Option<u8>,
}

/// A whole facet's regions.
#[derive(serde::Deserialize)]
struct RegionsSpec {
    #[serde(default)]
    facet: Facet,
    regions: Vec<RegionSpec>,
}

/// Give a facet its named areas, replacing whatever it had.
#[op2]
fn op_register_regions(state: &mut OpState, #[serde] spec: RegionsSpec) {
    let regions = spec
        .regions
        .into_iter()
        .map(|region| crate::ScriptRegion {
            name: region.name,
            priority: region.priority,
            rects: region
                .rects
                .into_iter()
                .map(|r| (r.x, r.y, r.width, r.height, r.z_min, r.z_max))
                .collect(),
            guarded: region.guarded,
            no_teleport: region.no_teleport,
            no_recall: region.no_recall,
            no_housing: region.no_housing,
            safe: region.safe,
            music: region.music,
            light: region.light,
        })
        .collect();
    state.borrow_mut::<Host>().outbox.push(Command::RegisterRegions {
        facet: spec.facet,
        regions,
    });
}

/// Forget a facet's regions.
#[op2(fast)]
fn op_clear_regions(state: &mut OpState, facet: u8) {
    state
        .borrow_mut::<Host>()
        .outbox
        .push(Command::ClearRegions { facet: Facet(facet) });
}

/// One placed decoration in a [`DecorSpec`].
#[derive(serde::Deserialize)]
struct DecorStaticSpec {
    graphic: u16,
    #[serde(default)]
    hue: u16,
    x: u16,
    y: u16,
    #[serde(default)]
    z: i8,
}

/// One placed door in a [`DecorSpec`].
#[derive(serde::Deserialize)]
struct DecorDoorSpec {
    /// Which key opens it; absent or 0 is unlocked.
    #[serde(default)]
    key_value: u32,
    closed: u16,
    open: u16,
    #[serde(default)]
    offset_x: i16,
    #[serde(default)]
    offset_y: i16,
    x: u16,
    y: u16,
    #[serde(default)]
    z: i8,
}

/// One placed container in a [`DecorSpec`].
#[derive(serde::Deserialize)]
struct DecorContainerSpec {
    /// Which key opens it; absent or 0 is unlocked.
    #[serde(default)]
    key_value: u32,
    graphic: u16,
    gump: u16,
    #[serde(default)]
    hue: u16,
    x: u16,
    y: u16,
    #[serde(default)]
    z: i8,
}

/// A batch of decoration:
/// `op_decorate({ facet, statics: [...], doors: [...], containers: [...] })`.
#[derive(serde::Deserialize)]
struct DecorSpec {
    #[serde(default)]
    facet: Facet,
    #[serde(default)]
    statics: Vec<DecorStaticSpec>,
    #[serde(default)]
    doors: Vec<DecorDoorSpec>,
    #[serde(default)]
    containers: Vec<DecorContainerSpec>,
}

/// Place a batch of decoration the shard adds on top of the map's art: plain
/// statics, openable doors, and openable containers.
#[op2]
fn op_decorate(state: &mut OpState, #[serde] spec: DecorSpec) {
    let statics = spec
        .statics
        .into_iter()
        .map(|s| crate::DecorStatic {
            graphic: s.graphic,
            hue: s.hue,
            x: s.x,
            y: s.y,
            z: s.z,
        })
        .collect();
    let doors = spec
        .doors
        .into_iter()
        .map(|d| crate::DecorDoor {
            key_value: d.key_value,
            closed: d.closed,
            open: d.open,
            offset_x: d.offset_x,
            offset_y: d.offset_y,
            x: d.x,
            y: d.y,
            z: d.z,
        })
        .collect();
    let containers = spec
        .containers
        .into_iter()
        .map(|c| crate::DecorContainer {
            key_value: c.key_value,
            graphic: c.graphic,
            gump: c.gump,
            hue: c.hue,
            x: c.x,
            y: c.y,
            z: c.z,
        })
        .collect();
    state.borrow_mut::<Host>().outbox.push(Command::Decorate {
        facet: spec.facet,
        statics,
        doors,
        containers,
    });
}

/// Remove every script-placed decoration.
#[op2(fast)]
fn op_clear_decorations(state: &mut OpState) {
    state.borrow_mut::<Host>().outbox.push(Command::ClearDecorations);
}

/// A region to generate doors in:
/// `op_generate_doors({ facet, x, y, width, height })`.
#[derive(serde::Deserialize)]
struct DoorRegionSpec {
    #[serde(default)]
    facet: Facet,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

/// Generate functional doors from the map's static frames in a region — the shop
/// doors a building's static art only implies.
#[op2]
fn op_generate_doors(state: &mut OpState, #[serde] region: DoorRegionSpec) {
    state.borrow_mut::<Host>().outbox.push(Command::GenerateDoors {
        facet: region.facet,
        x: region.x,
        y: region.y,
        width: region.width,
        height: region.height,
    });
}

/// One stock line for `op_stock`, from the script.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StockLineSpec {
    graphic: u16,
    #[serde(default)]
    hue: u16,
    #[serde(default = "one")]
    amount: u16,
    #[serde(default = "one_price")]
    price: u32,
    #[serde(default)]
    name: String,
}

/// The serde default for a stock line's price: one coin.
fn one_price() -> u32 {
    1
}

/// Everything `op_stock` takes: the vendor and its goods.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StockSpec {
    serial: u32,
    items: Vec<StockLineSpec>,
}

/// Fill a vendor's stock crate with priced goods.
#[op2]
fn op_stock(state: &mut OpState, #[serde] spec: StockSpec) {
    let stock = spec
        .items
        .into_iter()
        .map(|line| crate::StockItem {
            graphic: line.graphic,
            hue: line.hue,
            amount: line.amount,
            price: line.price,
            name: line.name,
        })
        .collect();
    state.borrow_mut::<Host>().outbox.push(Command::StockVendor {
        serial: spec.serial,
        stock,
    });
}

/// Put an item into a container by serial — a pack dropping loot into a corpse
/// off a `CorpseCreated` event. `stackable` merges gold/reagents onto a like
/// pile; a discrete piece (a weapon) is placed whole.
#[op2(fast)]
fn op_add_loot(state: &mut OpState, container: u32, graphic: u32, hue: u32, amount: u32, stackable: bool) {
    state.borrow_mut::<Host>().outbox.push(Command::AddLoot {
        container,
        graphic: graphic.min(u32::from(u16::MAX)) as u16,
        hue: hue.min(u32::from(u16::MAX)) as u16,
        amount: amount.min(u32::from(u16::MAX)) as u16,
        stackable,
    });
}

/// Remove an item by serial, wherever it is — the one-shot primitive a used item
/// needs to vanish (a drunk potion, a read-once scroll). `amount == 0` removes
/// the whole item; a smaller amount takes that many off a stackable pile.
#[op2(fast)]
fn op_consume_item(state: &mut OpState, serial: u32, amount: u32) {
    state.borrow_mut::<Host>().outbox.push(Command::ConsumeItem {
        serial,
        amount: amount.min(u32::from(u16::MAX)) as u16,
    });
}

extension!(
    openshard_ops,
    ops = [
        op_position,
        op_move,
        op_spawn_item,
        op_spawn_container,
        op_spawn_mobile,
        op_stock,
        op_add_loot,
        op_consume_item,
        op_damage,
        op_heal,
        op_cast_spell,
        op_control,
        op_set_stats,
        op_set_skill,
        op_set_weapon,
        op_set_poison,
        op_use_skill,
        op_say,
        op_register_spawner,
        op_clear_spawners,
        op_register_regions,
        op_clear_regions,
        op_decorate,
        op_clear_decorations,
        op_generate_doors,
        op_gump,
        op_register_npc_speech,
        op_register_quests,
        op_bind_quest_giver,
        op_make_escortable,
        op_close_gump,
        op_message,
        op_play_sound,
        op_give_item,
        op_take_item
    ],
    docs = "OpenShard's script-facing ops: read entity state, enqueue commands.",
);
