use openshard_events::Cursor;

use super::*;

/// Which character a connection is playing.
///
/// Kept beside the connection rather than asked of the world, because the
/// question it exists to answer is one the world cannot answer well: whether
/// *some other* connection is playing a given character. See
/// [`Sessions::is_playing`].
pub(crate) struct PlayedCharacter {
    /// Whose account it is.
    account: AccountName,
    /// The character's name, as the account lists it.
    name: CharacterName,
}

/// Where a connection stands with respect to the world.
///
/// # Why this is not a bool, and not the world's
///
/// It replaced `playing: Option<PlayedCharacter>`, which was set the instant
/// `Command::Enter` was *queued* and never cleared. That conflated three
/// different situations — asked to enter, in the world, and gone from it — and
/// the first of them is not a rounding error: `World::enter` can refuse, and did
/// so silently, leaving a connection whose session claimed a character the world
/// had never created. Every in-world packet from it was then queued into a tick
/// that dropped it on a `players` miss, with nothing anywhere saying why.
///
/// It lives here rather than in `WorldState` because the packet router has to
/// decide *now* whether a packet may reach the world, and the world answers no
/// synchronous question — only `queue(Command)` in and events out. So this is a
/// projection of the world's fact and never a second copy of it: nothing but a
/// world event moves it past [`Entering`](Self::Entering). See
/// `docs/connection_state.md`, D4.
pub(crate) enum WorldPhase {
    /// Not in the world: still logging in, or on the character screen. Which of
    /// those it is, is `LoginSession`'s to say and is deliberately not repeated
    /// here — a fact kept in two places is a fact that can disagree with itself.
    Outside,
    /// `Command::Enter` is queued and the tick has not applied it yet.
    ///
    /// In-world packets are still queued in this state, and must be: the command
    /// queue is ordered, so anything arriving now is applied *after* the `Enter`
    /// it follows. This is the one-tick gap between asking and arriving, and
    /// naming it is the whole point of the enum.
    Entering(PlayedCharacter),
    /// The world says this connection is driving a character — a `PlayerEntered`
    /// arrived for it.
    Playing(PlayedCharacter),
    /// It was in the world and is not any more: a logout, or an entry the world
    /// refused. Terminal — a connection never comes back from here to the
    /// character screen, because a `0xD1` is answered with an ack and the client
    /// closes the socket itself.
    Left,
}

impl WorldPhase {
    /// The character this connection stands for, whether or not the world has
    /// confirmed it yet.
    ///
    /// Both states count, and that is deliberate: the question this answers is
    /// "may somebody delete this character", and a character whose `Enter` is
    /// still in the queue must be as undeletable as one already standing in
    /// Britain.
    fn character(&self) -> Option<&PlayedCharacter> {
        match self {
            Self::Entering(played) | Self::Playing(played) => Some(played),
            Self::Outside | Self::Left => None,
        }
    }

    /// Whether a packet meant for the world may be queued from this connection.
    fn may_act(&self) -> bool {
        self.character().is_some()
    }
}

/// Per-connection state this loop owns.
pub(crate) struct Session {
    pub(crate) login: LoginSession,
    /// Where this connection stands with respect to the world.
    phase: WorldPhase,
    pub(crate) outbox: OutboxTx,
    /// Tells the gateway framer this connection's client version. A game
    /// connection sends no version of its own, so the framer defaults to the older
    /// dialect until this carries the real one across — needed for the packets
    /// whose length changed across eras (the drop packet). Sent at character
    /// select, well before any in-world packet that depends on it.
    pub(crate) control: VersionTx,
}

impl Session {
    /// A connection that has just been accepted: no seed yet, no login, no
    /// character, and uncompressed until a `0x91` says otherwise.
    pub(crate) fn new(outbox: OutboxTx, control: VersionTx) -> Self {
        Self {
            login: LoginSession::new(),
            phase: WorldPhase::Outside,
            outbox,
            control,
        }
    }

    /// Whether this connection may send the world a command.
    ///
    /// True from the moment `Enter` is queued — see [`WorldPhase::Entering`] for
    /// why the gap counts as in.
    pub(crate) fn in_world(&self) -> bool {
        self.phase.may_act()
    }

    /// Note which character this connection asked to play, as the
    /// `Command::Enter` that would put it there is queued.
    ///
    /// Only ever `Outside → Entering`. A second `0x5D` on a connection that is
    /// already in the world is a client contradicting itself, and honouring it
    /// here would overwrite the character the world knows this socket by; the
    /// world refuses the duplicate `Enter` too (`RefusedEntry::AlreadyInWorld`).
    pub(crate) fn enter_world(&mut self, account: AccountName, name: CharacterName) {
        if matches!(self.phase, WorldPhase::Outside) {
            self.phase = WorldPhase::Entering(PlayedCharacter { account, name });
        }
    }

    /// The world confirmed this connection is in — a `PlayerEntered`.
    ///
    /// Only `Entering → Playing`: an arrival for a connection this side never
    /// sent an `Enter` for is not something to invent a character from.
    pub(crate) fn entered_world(&mut self) {
        if let WorldPhase::Entering(played) = std::mem::replace(&mut self.phase, WorldPhase::Left) {
            self.phase = WorldPhase::Playing(played);
        }
    }

    /// The world let this connection's character go — a `PlayerLeft`, or an entry
    /// it refused.
    pub(crate) fn left_world(&mut self) {
        self.phase = WorldPhase::Left;
    }

    /// Whether this connection is playing exactly this character.
    ///
    /// Both halves are compared case-folded, the way every other account and
    /// character key in the shard is built — the client sends the name back as
    /// it was typed, and "Lord British" and "lord british" are one character.
    fn is_playing(&self, account: &AccountName, name: &CharacterName) -> bool {
        self.phase.character().is_some_and(|played| {
            played.account.normalized() == account.normalized()
                && played.name.normalized() == name.normalized()
        })
    }

    /// Act on a login response. Returns `false` if the connection should go.
    ///
    /// Dropping the outbox is what closes the socket: the gateway's write task
    /// ends when its channel does. There is no separate "close" to forget.
    pub(crate) fn apply(&self, response: Response, id: ConnectionId) -> bool {
        match response {
            Response::Idle => true,
            Response::Send(bytes) => self.send_packet(bytes),
            Response::SendThenClose(bytes) => {
                let _ = self.send_packet(bytes);
                false
            }
            Response::Close(reason) => {
                warn!(%id, %reason, "closing on a protocol error");
                false
            }
        }
    }

    /// Send one server-to-client packet, compressing it on a game connection.
    ///
    /// The login connection sends plain bytes; the game connection Huffman-
    /// compresses every packet, each one independently — terminator and all —
    /// exactly as Sphere's `CNetworkOutput` does for `CONNECT_GAME`. Skip this
    /// and ClassicUO, which decompresses the game stream unconditionally, decodes
    /// the raw bytes through its Huffman tree, produces plausible garbage for a
    /// while, and then desyncs on a fabricated packet id far downstream —
    /// surfacing as `need more data ID: 0E ...` hundreds of bytes in, looking
    /// nothing like a compression problem.
    ///
    /// Which kind of connection this is comes off the login state machine, which
    /// decided it when it read the `0x91` — see `LoginSession::is_game_login`.
    /// There is no flag here to keep in step with it, and so no way for the two
    /// to disagree about a socket that is already committed either way.
    pub(crate) fn send_packet(&self, bytes: Vec<u8>) -> bool {
        let bytes = if self.login.is_game_login() {
            huffman::compress(&bytes)
        } else {
            bytes
        };
        self.outbox.send(bytes).is_ok()
    }
}

/// Every connection the shard loop is holding, by id.
///
/// A table with one query on top of it — [`Sessions::is_playing`] — and that
/// query is why this is a type rather than the bare `HashMap` it replaces.
/// "Is anybody playing this character?" reads across *all* the connections, so
/// it cannot live on a `&mut Session` borrowed out of the map, and leaving it
/// to the caller to hand-roll the scan is how it ends up written twice.
pub(crate) struct Sessions(HashMap<ConnectionId, Session>);

impl Sessions {
    /// An empty table — a shard that has just come up.
    pub(crate) fn new() -> Self {
        Self(HashMap::new())
    }

    /// Take on a freshly accepted connection.
    pub(crate) fn open(&mut self, id: ConnectionId, session: Session) {
        self.0.insert(id, session);
    }

    /// Forget a connection: it disconnected, or a handler decided it should go.
    ///
    /// Deliberately takes no "should it close?" bool. Every call site already
    /// reads as `if !keep { sessions.close(id) }`, and a version that took the
    /// bool and decided in here would have to borrow the table on the `true`
    /// path too — which conflicts with the `&mut Session` the caller is still
    /// holding there. The reason was logged where the decision was made, so
    /// there is nothing left to do here but the removal.
    pub(crate) fn close(&mut self, id: ConnectionId) {
        self.0.remove(&id);
    }

    pub(crate) fn get(&self, id: ConnectionId) -> Option<&Session> {
        self.0.get(&id)
    }

    pub(crate) fn get_mut(&mut self, id: ConnectionId) -> Option<&mut Session> {
        self.0.get_mut(&id)
    }

    /// Whether any connection is playing this character right now.
    ///
    /// # Why this and not the world
    ///
    /// Character deletion (`0x83`) has to refuse a character somebody is
    /// playing, and the connection asking is never that somebody — it is on the
    /// character screen. It is a *second* connection on the same account, which
    /// is the only way the situation arises at all.
    ///
    /// This used to be `World::is_online(serial)`, and it had a hole: the caller
    /// could only produce a serial by looking the character up in the saved-record
    /// map, and a character created during this run has no saved record until it
    /// logs out. So for a brand-new character the check did not run — the
    /// character was dropped from the account while its entity kept playing, and
    /// the world was never even told to forget it, because there was no record to
    /// remove. Asked of the sessions, the question needs no serial and no saved
    /// row: a connection knows what it is playing from the moment it asks to.
    pub(crate) fn is_playing(&self, account: &AccountName, name: &CharacterName) -> bool {
        self.0.values().any(|session| session.is_playing(account, name))
    }
}

/// Keeps the sessions' phases in step with what the world did.
///
/// The other half of [`WorldPhase`]: this side may only ever ask to enter, and
/// what actually happened comes back as `PlayerEntered`, `PlayerLeft` or
/// `PlayerRefused`, drained here once a tick. Cursors rather than a drain method
/// on `World` because that is how every other reader of the world's events works
/// — see `Scripts` — and because a cursor is consumed by reading, so this one
/// cannot eat the script's copy of the same event.
pub(crate) struct PhaseSync {
    entered: Cursor<PlayerEntered>,
    left: Cursor<PlayerLeft>,
    refused: Cursor<PlayerRefused>,
}

impl PhaseSync {
    /// Cursors positioned at the world's present — nothing that happened before
    /// this loop started is a phase change it has to catch up on.
    pub(crate) fn new(world: &World) -> Self {
        Self {
            entered: world.bus().cursor(),
            left: world.bus().cursor(),
            refused: world.bus().cursor(),
        }
    }

    /// Move each session's phase to what the last tick says it is, and return the
    /// connections that have to go.
    ///
    /// The returned list is refusals only: a client whose entry the world turned
    /// down is waiting for a world it will never be in, and there is nothing to
    /// send it that its client would act on — the `0x5D` it sent has no refusal
    /// packet in the protocol. Dropping the socket is what turns an indefinite
    /// hang into a reconnect. A `PlayerLeft` needs no such treatment: the client
    /// asked to log out and closes the socket itself.
    pub(crate) fn apply(&mut self, world: &World, sessions: &mut Sessions) -> Vec<ConnectionId> {
        // Collected before touching the sessions: the reads borrow the world's
        // bus, and the writes below borrow the table.
        let bus = world.bus();
        let entered: Vec<ConnectionId> = bus.read(&mut self.entered).map(|e| e.connection).collect();
        let left: Vec<ConnectionId> = bus.read(&mut self.left).map(|e| e.connection).collect();
        let refused: Vec<PlayerRefused> = bus.read(&mut self.refused).copied().collect();

        for connection in entered {
            if let Some(session) = sessions.get_mut(connection) {
                session.entered_world();
            }
        }
        for connection in left {
            if let Some(session) = sessions.get_mut(connection) {
                session.left_world();
            }
        }
        refused
            .into_iter()
            .map(|event| {
                warn!(connection = %event.connection, reason = ?event.reason, "the world refused this entry");
                if let Some(session) = sessions.get_mut(event.connection) {
                    session.left_world();
                }
                event.connection
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use openshard_gateway::{OutboxRx, outbox_channel, version_channel};
    use openshard_protocol::version::ClientVersion;

    use super::*;
    use crate::testing::{admin, at_character_screen, login_server, lord_british};

    /// A connection that has said nothing yet: not a game socket, so not
    /// compressed. A game one has to be logged in for real — see
    /// [`at_character_screen`].
    fn fresh() -> (Session, OutboxRx) {
        let (outbox, wire) = outbox_channel();
        let (control, _control_rx) = version_channel();
        (Session::new(outbox, control), wire)
    }

    #[test]
    fn a_game_connection_compresses_and_a_login_one_does_not() {
        // The whole bug. ClassicUO Huffman-decodes every packet on the game
        // connection; send one raw and it decodes garbage and desyncs later on a
        // fabricated id ("need more data ID: 0E ..."). A character-list-shaped
        // packet, since 0xA9 is the first thing the game connection ever sends.
        let packet = vec![0xA9u8, 0x00, 0x08, 0x05, b'L', b'o', b'r', b'd'];

        let (game, mut wire) = at_character_screen(&mut login_server(), Instant::now());
        assert!(game.send_packet(packet.clone()));
        let on_wire = wire.try_recv().expect("a packet was sent");
        assert_ne!(on_wire, packet, "a game packet must not leave raw");
        assert_eq!(
            huffman::decompress(&on_wire).expect("valid stream"),
            packet,
            "and the client must get its bytes back"
        );

        let (login, mut wire) = fresh();
        assert!(login.send_packet(packet.clone()));
        assert_eq!(
            wire.try_recv().expect("a packet was sent"),
            packet,
            "the login connection is never compressed"
        );
    }

    #[test]
    fn a_played_character_is_found_from_another_connection() {
        // What deletion asks. The connection that would delete is not the one
        // playing, so the answer has to come from the table and not from the
        // asking session.
        let playing = ConnectionId::from_raw(1);
        let screen = ConnectionId::from_raw(2);

        let mut sessions = Sessions::new();
        let (mut in_world, _wire) = fresh();
        in_world.enter_world(AccountName::new("admin"), CharacterName::new("Lord British"));
        sessions.open(playing, in_world);
        let (at_screen, _screen_wire) = fresh();
        sessions.open(screen, at_screen);

        assert!(sessions.is_playing(&AccountName::new("admin"), &CharacterName::new("Lord British")));
        // Case-folded on both halves: the client sends the name back as typed.
        assert!(sessions.is_playing(&AccountName::new("ADMIN"), &CharacterName::new("lord british")));
        // And a character nobody picked is not in play, nor is one on another
        // account that happens to share the name.
        assert!(!sessions.is_playing(&AccountName::new("admin"), &CharacterName::new("Dupre")));
        assert!(!sessions.is_playing(&AccountName::new("other"), &CharacterName::new("Lord British")));
    }

    #[test]
    fn a_connection_on_the_character_screen_is_not_in_the_world() {
        let (session, _wire) = at_character_screen(&mut login_server(), Instant::now());
        assert!(!session.in_world(), "logged in, but nothing has been picked yet");
    }

    /// The `Enter` a test would queue for `connection`, entering `lord_british`
    /// fresh — the same command `dispatch_world_packet` builds for a `0x5D`.
    fn entering(connection: ConnectionId) -> Command {
        Command::Enter(Entering {
            connection,
            version: ClientVersion::TOL,
            account: admin(),
            name: lord_british(),
            access: AccessLevel::Player,
            character: Character::fresh(Facet(0)),
        })
    }

    #[test]
    fn the_phase_moves_on_only_when_the_world_says_it_did() {
        // The two halves of an entry, and the gap between them. Queuing the
        // command is a request: it makes this connection able to send the world
        // more commands (they queue behind the `Enter` and apply after it), but
        // it is emphatically not the arrival — `playing: Option<..>` used to say
        // both with one value, and could therefore never be wrong about the
        // second in a way anyone could see.
        let now = Instant::now();
        let mut world = World::new((1363, 1600));
        let mut phases = PhaseSync::new(&world);
        let mut sessions = Sessions::new();
        let id = ConnectionId::from_raw(1);
        let (session, _wire) = fresh();
        sessions.open(id, session);

        sessions.get_mut(id).unwrap().enter_world(admin(), lord_british());
        let session = sessions.get(id).unwrap();
        assert!(session.in_world(), "commands may be queued across the gap");
        assert!(
            matches!(session.phase, WorldPhase::Entering(_)),
            "but the world has not run yet"
        );

        world.queue(entering(id));
        world.tick(now);
        assert!(
            phases.apply(&world, &mut sessions).is_empty(),
            "an entry that worked closes nothing"
        );
        assert!(
            matches!(sessions.get(id).unwrap().phase, WorldPhase::Playing(_)),
            "and now the world has said so"
        );
    }

    #[test]
    fn an_entry_the_world_refuses_takes_the_connection_down() {
        // What used to happen in silence. `World::enter` returns early on three
        // paths; before this the session went on claiming a character that was
        // never created, every packet from it was queued into a tick that dropped
        // it, and the client sat on "logging into shard" with nothing anywhere
        // saying why. A second `Enter` on one connection is the reachable one of
        // the three.
        let now = Instant::now();
        let mut world = World::new((1363, 1600));
        let mut phases = PhaseSync::new(&world);
        let mut sessions = Sessions::new();
        let id = ConnectionId::from_raw(1);
        let (session, _wire) = fresh();
        sessions.open(id, session);

        sessions.get_mut(id).unwrap().enter_world(admin(), lord_british());
        world.queue(entering(id));
        world.tick(now);
        phases.apply(&world, &mut sessions);

        world.queue(entering(id));
        world.tick(now);
        assert_eq!(
            phases.apply(&world, &mut sessions),
            vec![id],
            "the refused connection is handed back to be closed"
        );
        assert!(
            matches!(sessions.get(id).unwrap().phase, WorldPhase::Left),
            "and it is not left claiming to play anything"
        );
    }

    #[test]
    fn a_character_the_world_let_go_of_is_no_longer_being_played() {
        // The other end of the same lie: `playing` was "set once and never
        // cleared", so a logged-out character stayed undeletable for as long as
        // its socket lingered. `PlayerLeft` is what clears it now.
        let now = Instant::now();
        let mut world = World::new((1363, 1600));
        let mut phases = PhaseSync::new(&world);
        let mut sessions = Sessions::new();
        let id = ConnectionId::from_raw(1);
        let (session, _wire) = fresh();
        sessions.open(id, session);

        sessions.get_mut(id).unwrap().enter_world(admin(), lord_british());
        world.queue(entering(id));
        world.tick(now);
        phases.apply(&world, &mut sessions);
        assert!(sessions.is_playing(&admin(), &lord_british()));

        world.queue(Command::Disconnect { connection: id });
        world.tick(now);
        phases.apply(&world, &mut sessions);
        assert!(
            !sessions.is_playing(&admin(), &lord_british()),
            "the world let the character go, so nothing is playing it"
        );
    }
}
