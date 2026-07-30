use super::*;

/// Sender half of the tick's outbound-snapshot channel. Only the tick loop in
/// `run_shard` ever has a snapshot to hand off, so this stays private to the
/// module rather than a bare `UnboundedSender` some unrelated `Snapshot`
/// producer could be handed by mistake.
#[derive(Debug, Clone)]
pub(crate) struct SnapshotTx(mpsc::UnboundedSender<Snapshot>);

impl SnapshotTx {
    // Boxed: a bare `SendError<Snapshot>` carries a whole `Snapshot` by value,
    // which is the failure case's problem alone — the caller here only ever
    // checks `is_err()` and never wants that weight on the stack.
    fn send(&self, snapshot: Snapshot) -> Result<(), Box<mpsc::error::SendError<Snapshot>>> {
        self.0.send(snapshot).map_err(Box::new)
    }
}

/// Receiver half of [`SnapshotTx`], drained by [`save_loop`].
#[derive(Debug)]
pub(crate) struct SnapshotRx(mpsc::UnboundedReceiver<Snapshot>);

impl SnapshotRx {
    async fn recv(&mut self) -> Option<Snapshot> {
        self.0.recv().await
    }
}

fn snapshot_channel() -> (SnapshotTx, SnapshotRx) {
    let (tx, rx) = mpsc::unbounded_channel();
    (SnapshotTx(tx), SnapshotRx(rx))
}

/// Sender half of the save task's failure-signal channel — a save failed and
/// the tick loop should mark the world for a full resweep. The unit payload
/// makes it easy to reach for a bare `mpsc::unbounded_channel::<()>()` at a
/// call site that also happens to touch snapshots; the newtype keeps the two
/// from being confused for each other by the compiler as well as the reader.
#[derive(Debug, Clone)]
pub(crate) struct FailureTx(mpsc::UnboundedSender<()>);

impl FailureTx {
    fn send(&self) -> Result<(), mpsc::error::SendError<()>> {
        self.0.send(())
    }
}

/// Receiver half of [`FailureTx`], read by the tick loop in `run_shard`.
#[derive(Debug)]
struct FailureRx(mpsc::UnboundedReceiver<()>);

impl FailureRx {
    async fn recv(&mut self) -> Option<()> {
        self.0.recv().await
    }
}

fn failure_channel() -> (FailureTx, FailureRx) {
    let (tx, rx) = mpsc::unbounded_channel();
    (FailureTx(tx), FailureRx(rx))
}

/// Write snapshots, forever, on a task nothing waits for.
///
/// # This is the only place that touches a disk
///
/// And it is deliberately somewhere the tick cannot reach. The world hands over
/// owned values and moves on; whatever happens here — a slow disk, a lock, a
/// database in another country — happens to this task and to nothing else. A
/// shard whose store is wedged saves late. It does not lag, and it does not stop
/// letting people play.
///
/// # A failed write is reported, not retried here
///
/// Retrying from here would write the same stale snapshot at a world that has
/// moved on. The failure goes back to the shard loop, which asks the world for a
/// full sweep — see `World::resweep`. The cost of a failure is a fat save, and
/// the recovery reads the world as it is now rather than as it was.
async fn save_loop(store: Arc<dyn Store>, mut snapshots: SnapshotRx, failures: FailureTx) {
    while let Some(snapshot) = snapshots.recv().await {
        let rows = snapshot.len();
        let started = Instant::now();
        match store.save(&snapshot).await {
            Ok(()) => debug!(
                tick = snapshot.tick,
                rows,
                took = ?started.elapsed(),
                "saved"
            ),
            Err(error) => {
                error!(tick = snapshot.tick, rows, %error, "save failed; the next one will be a full sweep");
                // If the shard loop is gone there is nobody to sweep and nothing
                // to do about it. The `let _` is that, not carelessness.
                let _ = failures.send();
            }
        }
    }
}

/// Accounts come from the store first — their credentials are the argon2
/// hashes saved there — and config seeds the rest. The store is authoritative
/// for a password once it has one, so a config `[[accounts]]` line only
/// creates an account the store has never seen; changing a config password
/// after the first boot does nothing (the shard says as much in the docs).
async fn load_accounts(store: &dyn Store, config: &Config) -> DevAccounts {
    let mut accounts = DevAccounts::new();
    match store.accounts().await {
        Ok(stored) => {
            for record in stored {
                accounts = accounts.with_credential(&record.name, &record.credential);
            }
        }
        Err(error) => error!(%error, "could not read saved accounts; config seeds them instead"),
    }
    for account in &config.accounts {
        // Seed a config account only if the store has never seen it, hashing the
        // plaintext once and writing that same hash both in memory and to the
        // store — never the plaintext.
        if !accounts.contains(&account.name) {
            let credential = openshard_login::password::hash(&account.password);
            accounts = accounts.with_credential(&account.name, &credential);
            let record = AccountRecord {
                name: account.name.clone(),
                credential,
            };
            if let Err(error) = store.put_account(&record).await {
                warn!(%account.name, %error, "could not persist a configured account");
            }
        }
        // Characters and access come from config every boot regardless: access is
        // deliberately not persisted (re-derived each login), and config
        // characters are folded in beside the stored ones below, deduped.
        for character in &account.characters {
            accounts = accounts.with_character(&account.name, character);
        }
        // An unparseable access level is logged and left a player — authority is
        // never granted by a typo.
        match account.access.0.parse::<AccessLevel>() {
            Ok(AccessLevel::Player) => {}
            Ok(level) => accounts = accounts.with_access(&account.name, level),
            Err(error) => {
                warn!(%account.name, %error, "unknown access level; treating as player")
            }
        }
    }
    accounts
}

/// Bring the world's characters back from the database: reserve every stored
/// serial so a new character cannot take one, list the stored characters so
/// they show up to play, and keep their records so playing one restores it
/// where it was.
async fn restore_characters(
    store: &dyn Store,
    world: &mut World,
    mut accounts: DevAccounts,
) -> (DevAccounts, Roster) {
    let mut roster = Roster::new();
    match store.characters().await {
        Ok(characters) => {
            for record in characters {
                world.reserve_serial(record.serial);
                let listed = accounts
                    .characters(&record.account)
                    .iter()
                    .any(|entry| entry.name.0.eq_ignore_ascii_case(&record.name.0));
                if !listed {
                    accounts = accounts.with_character(&record.account, &record.name);
                }
                roster.remember(record);
            }
            if !roster.is_empty() {
                info!(characters = roster.len(), "restored the world from the database");
            }
        }
        Err(error) => error!(%error, "could not read saved characters; starting with none"),
    }
    (accounts, roster)
}

/// Bring back saved items: the world reserves their serials, drops the loose
/// ground clutter back where it lay, and files each character's carried
/// inventory to re-equip when it logs in. Called after `restore_characters`,
/// so their serials are already reserved and an item can point at the
/// container it was in.
async fn restore_items(store: &dyn Store, world: &mut World) {
    match store.items().await {
        Ok(items) => {
            if !items.is_empty() {
                info!(items = items.len(), "restored saved items");
            }
            world.restore_items(items);
        }
        Err(error) => error!(%error, "could not read saved items; starting with none"),
    }
}

/// Bring back the world's NPC mobiles — townsfolk, vendors, creatures — each
/// exactly as saved. Called after `restore_items`, so each mobile's gear and
/// stock is already filed under its serial for `World::restore_mobiles` to
/// equip. This is the whole-world model: the pack seeds a fresh world once (a
/// staff Populate), and from then on the save is the truth — nothing respawns
/// at boot.
async fn restore_mobiles(store: &dyn Store, world: &mut World) {
    match store.mobiles().await {
        Ok(mobiles) => {
            if !mobiles.is_empty() {
                info!(mobiles = mobiles.len(), "restored the world's mobiles");
            }
            world.restore_mobiles(mobiles);
        }
        Err(error) => error!(%error, "could not read saved mobiles; starting with none"),
    }
}

/// Bring back the placed decoration, door state and all.
async fn restore_decorations(store: &dyn Store, world: &mut World) {
    match store.decorations().await {
        Ok(decorations) => {
            if !decorations.is_empty() {
                info!(decorations = decorations.len(), "restored the world's decoration");
            }
            world.restore_decorations(decorations);
        }
        Err(error) => error!(%error, "could not read saved decorations; starting with none"),
    }
}

/// Bring back the spawn regions with their respawn timers, so a populated area
/// stays populated across a restart and a rare spawn keeps its remaining wait
/// rather than popping again the moment the shard comes up.
async fn restore_spawners(store: &dyn Store, world: &mut World) {
    match store.spawners().await {
        Ok(spawners) => {
            if !spawners.is_empty() {
                info!(spawners = spawners.len(), "restored spawn regions");
            }
            world.restore_spawners(spawners);
        }
        Err(error) => error!(%error, "could not read saved spawners; starting with none"),
    }
}

/// Bring back the named regions — towns, dungeons, guarded zones. Saved like
/// everything else, so a restart keeps its guards, its music and the dark in
/// its caves without waiting for a staff `.admin`.
async fn restore_regions(store: &dyn Store, world: &mut World) {
    match store.regions().await {
        Ok(regions) => {
            if !regions.is_empty() {
                info!(regions = regions.len(), "restored the world's regions");
            }
            world.restore_regions(regions);
        }
        Err(error) => error!(%error, "could not read saved regions; starting with none"),
    }
}

/// Bring back the two things only the world itself knew: the hour of the day, and
/// where its roll generator had got to.
///
/// The tick counter restarts at zero by design, so without the clock every restart
/// would be a fresh midnight. The generator is the same shape of loss with a
/// sharper edge — re-seeded, it does not roll *differently*, it rolls the previous
/// run's sequence again, which is a thing a player who dislikes a roll can arrange
/// by getting the shard restarted.
///
/// A store with no such row yet — a world nobody has saved — is left exactly as
/// built, which is what keeps a configured `world.seed` from being overwritten on
/// the first boot. A store that cannot be *read* is logged and treated the same
/// way: this is cosmetic-to-annoying, not corrupting, and refusing to boot over it
/// would be worse.
///
/// `pinned_seed` is only here to be *complained* about: a saved world resumes its
/// stream, so a `world.seed` set on a shard that has saved does nothing, and a knob
/// that silently does nothing is the failure this whole config crate exists to
/// prevent. The operator hears it once, at boot.
async fn restore_world(store: &dyn Store, world: World, pinned_seed: Option<u64>) -> World {
    match store.world().await {
        Ok(Some(record)) => {
            if pinned_seed.is_some() {
                warn!(
                    "world.seed is set but this world has been saved before, so it is ignored: \
                     the shard resumes the roll stream its save recorded. Start from a fresh \
                     database to use it."
                );
            }
            world
                .with_clock_minutes(record.clock_minutes)
                .with_rng_state(record.rng_state)
        }
        Ok(None) => world,
        Err(error) => {
            error!(%error, "could not read the saved world scalars; starting at midnight with a fresh roll stream");
            world
        }
    }
}

/// Build the login server: the character-creation screen needs somewhere to
/// start (without it the client refuses to create at all — "No city found.
/// Something wrong with the received cities." — because the list it was sent
/// is empty), and the AoS (0xB9) and character-list (0xA9) flags that tell a
/// modern client to turn on tooltips and context menus rather than staying on
/// single-click names.
fn build_login_server(
    config: &Config,
    accounts: DevAccounts,
    advertised: SocketAddrV4,
) -> LoginServer<DevAccounts> {
    let mut login = LoginServer::new(accounts, &config.server.name, advertised);
    // The list is filtered to the facets this shard loaded, so every city
    // offered is one a player can actually be placed in.
    login.starts = start_cities(&config.world.facets, (config.world.start.x, config.world.start.y));
    login.supported_features = crate::boot::supported_features_of(config);
    login.character_list_flags = crate::boot::character_list_flags_of(config);
    login
}

/// Run one tick: advance the world, flush its outbound packets, hand off
/// its snapshots and departed characters, pump the gameplay script, and
/// reclaim expired login keys.
///
/// The key reclaim rides along here rather than after every `select!` branch
/// because `AuthKeys::expire` is memory upkeep for abandoned relay keys —
/// `AuthKeys::redeem` already checks expiry on its own — and its own doc says
/// to call it "on a timer". The tick is that timer; a packet or a disconnect
/// is not.
fn world_tick(
    world: &mut World,
    sessions: &mut Sessions,
    phases: &mut PhaseSync,
    saves: &SnapshotTx,
    roster: &mut Roster,
    scripts: &mut Option<Scripts>,
    login_server: &mut LoginServer<DevAccounts>,
) {
    world.tick(Instant::now());
    // Before anything is sent: what the world did this tick decides which
    // connections are in it, and a refusal means there is nobody left to send to.
    for connection in phases.apply(world, sessions) {
        sessions.close(connection);
    }
    for out in world.drain_outbound() {
        if let Some(session) = sessions.get(out.connection) {
            // A connection reaches the world only after its game
            // login, so this is always a game connection and every
            // packet leaves compressed. `send_packet` gates on the
            // flag anyway, so it stays correct if that ever changes.
            let _ = session.send_packet(out.packet);
        }
    }
    // Handed off, not awaited. The tick's job here is to stop
    // holding the only copy.
    for snapshot in world.drain_saves() {
        let _ = saves.send(snapshot);
    }
    // Keep the in-memory character list current with logouts, so a
    // re-login this run finds a character where it left, not where it
    // was at boot. The store gets the same record via the snapshot
    // above; this is the copy a re-login can read before that lands.
    for record in world.drain_departed() {
        roster.remember(record);
    }
    // Feed the script this tick's events and queue its commands for
    // the next one. After the drains, so a command a script emits is
    // applied by a tick and leaves through this same path.
    if let Some(scripts) = scripts.as_mut() {
        scripts.pump(world);
    }
    // Reclaim keys from clients that selected a shard and never came back.
    login_server.keys.expire(Instant::now());
}

/// Drive login and the world until the gateway stops.
///
/// One task owns both. That is not a limitation: the world is deliberately
/// single-threaded — a deterministic tick is the whole point — and login is a
/// state machine that does no work worth parallelising. Async lives in the
/// gateway's tasks, on the far side of the channel.
pub async fn run_shard(mut events: ServerEventRx, config: &Config, mut world: World, store: Arc<dyn Store>) {
    // `Config::validate` (run by `Config::load`, which every `Config` reaching
    // here has been through) refuses an IPv6 `server.advertise`, so this is
    // always `Some` in practice.
    let advertised = config
        .advertise_v4()
        .expect("Config::validate rejects an IPv6 server.advertise");

    let (saves, snapshots) = snapshot_channel();
    let (failed, mut failures) = failure_channel();

    // Load and restore in turn: characters after accounts (a stored character
    // can add to the account's list), items after characters, mobiles after
    // items, and so on — see each function's doc for why. This all borrows the
    // store; the save task takes ownership after, so it has to come last.
    let accounts = load_accounts(store.as_ref(), config).await;
    let (accounts, mut roster) = restore_characters(store.as_ref(), &mut world, accounts).await;
    restore_items(store.as_ref(), &mut world).await;
    restore_mobiles(store.as_ref(), &mut world).await;
    restore_decorations(store.as_ref(), &mut world).await;
    restore_spawners(store.as_ref(), &mut world).await;
    restore_regions(store.as_ref(), &mut world).await;
    world = restore_world(store.as_ref(), world, config.world.seed).await;

    // this dont need to be here
    let mut login_server = build_login_server(config, accounts, advertised);

    // Kept, not detached: shutdown hands it a final snapshot, closes the channel,
    // and awaits this task so every queued write lands before the process exits.
    let save_task = tokio::spawn(save_loop(store, snapshots, failed));

    // The gameplay script, if one is configured. Constructed after the world is
    // built and restored, before the first tick, so its cursors start clean.
    let mut scripts = Scripts::load(&config.scripting.main, &world);

    let mut sessions = Sessions::new();
    // Built after the world is restored, so the arrivals and departures of the
    // restore are not read back as phase changes for connections that do not
    // exist yet.
    let mut phases = PhaseSync::new(&world);
    let mut ticker = tokio::time::interval(TICK_INTERVAL);
    // A tick that ran late must not try to catch up by running several in a row:
    // that turns a hiccup into a stall, and a fixed timestep into a variable one.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // Biased so the tick cannot be starved by a busy network. Without
            // this, a flood of packets would keep `recv` ready forever and the
            // world would stop simulating under exactly the load that needs it.
            biased;

            _ = ticker.tick() => {
                world_tick(
                    &mut world,
                    &mut sessions,
                    &mut phases,
                    &saves,
                    &mut roster,
                    &mut scripts,
                    &mut login_server,
                );
            }

            // Before `events`: a store that is failing is worth hearing about
            // ahead of the next packet, and there is never a queue of these.
            Some(()) = failures.recv() => {
                warn!("a save failed; marking the world for a full sweep");
                world.resweep();
            }

            // Ctrl-C: leave the loop and save the world on the way out, rather than
            // dying with the last save cadence's worth of play unwritten.
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown requested; saving the world");
                break;
            }

            event = events.recv() => {
                let Some(event) = event else {
                    error!("the gateway stopped; saving the world");
                    break;
                };
                world_handle_network(&mut sessions, &mut login_server, &mut world, advertised, &mut roster, event);
            }
        }
    }

    // Shutdown: one last full snapshot, then flush every queued write before the
    // process exits. This is the one moment a lost write costs a player real value,
    // so unlike the per-tick handoff it is *awaited*. Dropping the sender ends the
    // save task's receive loop once it has drained what is left.
    //
    // End every trade first. A trade escrow is deliberately not saved — a
    // restored one would be a window nobody can close — so the goods inside one
    // have to be back in the two packs *before* the sweep reads them, or a clean
    // stop taken mid-trade costs both parties whatever they had offered.
    world.cancel_all_trades();
    world.take_snapshot();
    for snapshot in world.drain_saves() {
        let _ = saves.send(snapshot);
    }
    drop(saves);
    if let Err(error) = save_task.await {
        error!(%error, "the save task did not finish cleanly on shutdown");
    }
    info!("world saved; shutting down");
}

/// Whether the relay is about to send this client somewhere it cannot get back
/// from.
///
/// True when `advertise` is loopback and the client is not on this machine: the
/// relay will tell it to dial `127.0.0.1`, it will reach its own loopback, find
/// nothing, and give up.
///
/// # Why this is worth catching here and not only at startup
///
/// The startup warning fires before anyone has connected, and it scrolls away.
/// By the time the mistake is *made* — the moment a client that is not on this
/// machine picks the shard — the warning is a hundred lines up, and what the
/// operator is looking at is a client stuck on "logging into shard" and a server
/// log that says nothing is wrong.
///
/// And nothing here *is* wrong, which is the whole difficulty. This end sends a
/// perfectly good packet and never sees a second connection, because the failure
/// happens somewhere it cannot observe. This is the last moment the shard can
/// still see both addresses at once and say what is about to happen.
pub(crate) fn relay_is_unreachable(client: SocketAddr, advertised: SocketAddrV4) -> bool {
    advertised.ip().is_loopback() && !client.ip().is_loopback()
}

/// Route a decoded login packet to either of the two things it can mean here:
/// character creation, or an ordinary login-state transition. Both arrive as
/// `Packet::Login` from `parse_packet`, and creation needs both `login` and
/// `world` in reach, which is why this crosses the login/world line instead of
/// living in `dispatch_world_packet`.
///
/// Deletion is the third `Packet::Login` case and is routed past this function,
/// because it needs the whole session table rather than this one session — see
/// [`delete_character`].
fn handle_login_packet(
    session: &mut Session,
    login: &mut LoginServer<DevAccounts>,
    world: &mut World,
    packet: LoginStagePacket,
    id: ConnectionId,
) -> bool {
    match packet {
        // Character creation crosses the login/world line: it writes the new
        // character onto the account and then enters the world with it.
        LoginStagePacket::CreateCharacter(create) => create_character(session, login, world, create, id),
        login_packet => {
            // The game login is the seam Sphere calls CONNECT_GAME: from here
            // on, this connection's every server->client packet is
            // Huffman-compressed — starting with the reply `handle` just built,
            // which is why `apply` follows the call rather than the state
            // machine sending anything itself. Nothing is copied out to say so:
            // `Session::send_packet` reads the state machine.
            let authenticated = session.login.account().is_some();
            let response = login.handle(&mut session.login, login_packet, Instant::now());
            // And it is the seam the *world* takes the connection over at. The
            // transition is one-way and happens once — `LoginSession` has no path
            // back out of `CharacterListSent` — so comparing the account across
            // the call is enough to catch it exactly once, with no flag here to
            // fall out of step with the state machine that owns the fact.
            if !authenticated && session.login.account().is_some() {
                world.queue(Command::Authenticated {
                    connection: id,
                    version: session.login.version(),
                });
            }
            session.apply(response, id)
        }
    }
}

/// Route a decoded world packet: pick the character screen's `0x5D` out of it,
/// then gate everything else on the connection's phase and queue what the
/// dispatcher makes of it.
///
/// # The one gate
///
/// This `if` is the whole of what thirty arms of `dispatch_world_packet` used to
/// repeat — see that function's doc, and `docs/connection_state.md` S3. It is
/// here rather than there because this is the last place that still holds the
/// session: past it, a packet is only a packet.
///
/// The dropped packet is not named in the log. `ClientPacket`'s `Debug` carries
/// bodies — a `0x03` would put whatever the player typed in the log — and a
/// per-variant name table would be a second list to keep in step with the enum.
/// The connection is what a reader needs; which packet it was, the client knows.
fn handle_world_packet(
    session: &mut Session,
    login: &LoginServer<DevAccounts>,
    world: &mut World,
    roster: &Roster,
    packet: ClientPacket,
    id: ConnectionId,
) -> bool {
    // `0x5D` is the one packet a connection outside the world may send, and it
    // is what puts it inside. It also needs the account's authority, looked up
    // here because this is where the store is in reach.
    let packet = match packet {
        ClientPacket::CharacterPlay(play) => {
            let access = session
                .login
                .account()
                .map_or(AccessLevel::Player, |a| login.accounts.access_level(a));
            return play_character(session, world, roster, play, id, access);
        }
        packet => packet,
    };

    if !session.in_world() {
        debug!(%id, "a world packet from a connection that is not in the world");
        return true;
    }
    if let Some(command) = dispatch_world_packet(packet, id) {
        world.queue(command);
    }
    true
}

pub(crate) fn world_handle_network(
    sessions: &mut Sessions,
    login: &mut LoginServer<DevAccounts>,
    world: &mut World,
    advertised: SocketAddrV4,
    roster: &mut Roster,
    event: ServerEvent,
) {
    match event {
        ServerEvent::Connected {
            id,
            address,
            outbox,
            control,
        } => {
            info!(%id, %address, "connected");
            if relay_is_unreachable(address, advertised) {
                error!(
                    client = %address,
                    %advertised,
                    "this client is not on this machine and server.advertise is loopback. \
                     When it picks the shard it will be told to dial {advertised} — its own \
                     loopback — and will hang on \"logging into shard\" until it times out. \
                     Set server.advertise to the address this client can reach."
                );
            }
            sessions.open(id, Session::new(outbox, control));
        }

        ServerEvent::Received { id, event } => {
            // Read what decoding needs and let the borrow go, rather than
            // holding the session across the routing below. `0x83` is routed
            // with the whole table in hand — see `delete_character` — and a
            // `&mut Session` taken here would still be alive at that point.
            let Some(version) = sessions.get(id).map(|session| session.login.version()) else {
                // Disconnected arrived first. Possible: the gateway's tasks and
                // this loop are not synchronised.
                return;
            };
            // Every arm below looked the session up a line ago and nothing
            // between here and there can remove it: this loop is the only thing
            // that touches the table, and it is not concurrent with itself.
            const PRESENT: &str = "the session was looked up at the top of this arm";
            match event {
                Event::Seeded(seed) => sessions.get_mut(id).expect(PRESENT).login.on_seed(seed),
                Event::Packet(packet) => match packet.parse_packet(version) {
                    // Ok: hand the decoded packet to whichever side it belongs
                    // to. Every handler returns the same "keep the connection?"
                    // bool, so there is one place that acts on it.
                    Ok(packet) => {
                        let keep = match packet {
                            // Deletion reads across every session, so it takes
                            // the table and is routed here rather than through
                            // `handle_login_packet`.
                            Packet::Login(LoginStagePacket::DeleteCharacter(delete)) => {
                                delete_character(sessions, login, world, roster, delete, id)
                            }
                            Packet::Login(login_packet) => {
                                let session = sessions.get_mut(id).expect(PRESENT);
                                handle_login_packet(session, login, world, login_packet, id)
                            }
                            Packet::World(world_packet) => {
                                let session = sessions.get_mut(id).expect(PRESENT);
                                handle_world_packet(session, login, world, roster, world_packet, id)
                            }
                        };
                        if !keep {
                            sessions.close(id);
                        }
                    }
                    // Err: every case here only logs and drops. Nothing decoded,
                    // so there is nothing to route.
                    Err(error) => {
                        match error {
                            PacketError::Login(ClientLoginDecodeError::CreateCharacter(error)) => {
                                warn!(%id, %error, "malformed create-character");
                            }
                            PacketError::Login(ClientLoginDecodeError::DeleteCharacter(error)) => {
                                warn!(%id, %error, "malformed delete-character");
                            }
                            PacketError::Login(other) => {
                                warn!(%id, ?other, "malformed login packet");
                            }
                            PacketError::World(error) => {
                                warn!(%id, ?error, "malformed packet");
                            }
                        }
                        sessions.close(id);
                    }
                },
            }
        }

        ServerEvent::Disconnected { id, reason } => {
            match reason {
                Some(reason) => warn!(%id, %reason, "disconnected"),
                None => info!(%id, "disconnected"),
            }
            // The world learns on its own schedule. It owns the entity and the
            // serial, and tearing them down from here would be a write to the
            // world from outside the tick.
            world.queue(Command::Disconnect { connection: id });
            sessions.close(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use openshard_protocol::direction::{Direction, Facing};
    use openshard_protocol::identity::RawCharacterName;
    use openshard_protocol::wire::{RawCharacterSlot, RawClientIp};
    use openshard_protocol::world::{RawFastwalkKey, RawStepSequence, WalkRequest};

    use super::*;
    use crate::testing::{admin, at_character_screen, login_server, lord_british};

    fn client(address: &str) -> SocketAddr {
        address.parse().expect("a client address")
    }

    fn advertise(address: &str) -> SocketAddrV4 {
        address.parse().expect("an advertised address")
    }

    #[test]
    fn a_loopback_advertise_is_unreachable_to_a_client_on_the_network() {
        // The bug this exists for: the client dials its own loopback and hangs
        // on "logging into shard" while this end sees one connection, a clean
        // disconnect, and nothing to explain either.
        assert!(relay_is_unreachable(
            client("192.168.11.163:51606"),
            advertise("127.0.0.1:2593")
        ));
    }

    #[test]
    fn a_loopback_advertise_is_fine_for_a_client_on_this_machine() {
        // And this is why the shard does not simply refuse to start on a
        // loopback advertise: a developer with the client on their own desk is
        // the common case, and it works.
        assert!(!relay_is_unreachable(
            client("127.0.0.1:51606"),
            advertise("127.0.0.1:2593")
        ));
    }

    #[test]
    fn a_real_advertise_is_fine_for_anyone() {
        for address in ["127.0.0.1:51606", "192.168.11.163:51606", "8.8.8.8:51606"] {
            assert!(
                !relay_is_unreachable(client(address), advertise("192.168.11.10:2593")),
                "{address} should be able to reach an advertised LAN address"
            );
        }
    }

    /// One step north — any in-world packet would do; this is the smallest.
    fn a_step() -> ClientPacket {
        ClientPacket::Walk(WalkRequest {
            facing: Facing::walking(Direction::North),
            sequence: RawStepSequence(0),
            fastwalk_key: RawFastwalkKey(0),
        })
    }

    /// The `0x5D` a client sends when it picks [`lord_british`] off the list.
    fn picking_lord_british() -> ClientPacket {
        ClientPacket::CharacterPlay(CharacterPlay {
            name: RawCharacterName(lord_british().0),
            slot: RawCharacterSlot(0),
            client_ip: RawClientIp(0),
        })
    }

    #[test]
    fn a_world_packet_from_outside_the_world_becomes_no_command() {
        // The gate, from the side it refuses. A connection sitting on the
        // character screen has no entity, so a `0x02` from it would be queued
        // into a tick that drops it on a `players` miss — work created out of a
        // packet nobody can act on. The queue length is the assertion because
        // "nothing happened" has nothing else to look at; see `World::queued`.
        let mut login = login_server();
        let mut world = World::new((1363, 1600));
        let roster = Roster::new();
        let id = ConnectionId::from_raw(1);
        let (mut session, _wire) = at_character_screen(&mut login, Instant::now());

        assert!(
            handle_world_packet(&mut session, &login, &mut world, &roster, a_step(), id),
            "a packet that is merely early is not a reason to close"
        );
        assert_eq!(world.queued(), 0, "and it did not become work");
    }

    #[test]
    fn the_same_packet_becomes_a_command_once_the_connection_is_in() {
        // The other direction, so the test above cannot pass by a gate that
        // refuses everything. Nothing changes but the phase — the same session,
        // the same packet.
        let mut login = login_server();
        let mut world = World::new((1363, 1600));
        let roster = Roster::new();
        let id = ConnectionId::from_raw(1);
        let (mut session, _wire) = at_character_screen(&mut login, Instant::now());
        session.enter_world(admin(), lord_british());

        assert!(handle_world_packet(
            &mut session,
            &login,
            &mut world,
            &roster,
            a_step(),
            id
        ));
        assert_eq!(world.queued(), 1, "the step is queued for the next tick");
    }

    #[test]
    fn the_character_screen_packet_is_the_one_the_gate_does_not_stop() {
        // `0x5D` arrives from a connection that is by definition outside the
        // world — it is what puts it in — so it is routed before the gate. Route
        // it after and a shard accepts no logins at all, silently: the client
        // waits on "logging into shard" and this end says nothing.
        let mut login = login_server();
        let mut world = World::new((1363, 1600));
        let roster = Roster::new();
        let id = ConnectionId::from_raw(1);
        let (mut session, _wire) = at_character_screen(&mut login, Instant::now());
        assert!(!session.in_world(), "the fixture is on the character screen");

        assert!(handle_world_packet(
            &mut session,
            &login,
            &mut world,
            &roster,
            picking_lord_british(),
            id
        ));
        assert_eq!(world.queued(), 1, "the entry is queued");
        assert!(session.in_world(), "and the gate is open for what follows");
    }

    #[test]
    fn an_ipv6_loopback_client_is_still_on_this_machine() {
        // `::1` is loopback and the obvious check — comparing against the string
        // "127.0.0.1", or against Ipv4Addr::LOCALHOST — misses it, and fires a
        // scary error at a developer whose client happens to have resolved
        // localhost to v6.
        assert!(!relay_is_unreachable(
            client("[::1]:51606"),
            advertise("127.0.0.1:2593")
        ));
    }
}
