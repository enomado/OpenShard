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

/// Per-connection state this loop owns.
pub(crate) struct Session {
    pub(crate) login: LoginSession,
    /// The character this connection asked to play, or `None` while it is still
    /// on the login or character screen.
    ///
    /// The world owns the entity; this is only the connection's own note of what
    /// it stands for — enough to know a `0x02` is worth queueing, and enough for
    /// another connection to be told the character is in use.
    ///
    /// Set once and never cleared. A `0xD1` logout is answered with an ack and
    /// the client closes the socket itself — see `Command::LogoutRequest` — so a
    /// connection never comes back from the world to the character screen, and
    /// there is no path where this would have to be unset.
    playing: Option<PlayedCharacter>,
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
            playing: None,
            outbox,
            control,
        }
    }

    /// Whether this connection has entered the world with a character.
    pub(crate) fn in_world(&self) -> bool {
        self.playing.is_some()
    }

    /// Note which character this connection is playing, as the `Command::Enter`
    /// that puts it there is queued.
    pub(crate) fn enter_world(&mut self, account: AccountName, name: CharacterName) {
        self.playing = Some(PlayedCharacter { account, name });
    }

    /// Whether this connection is playing exactly this character.
    ///
    /// Both halves are compared case-folded, the way every other account and
    /// character key in the shard is built — the client sends the name back as
    /// it was typed, and "Lord British" and "lord british" are one character.
    fn is_playing(&self, account: &AccountName, name: &CharacterName) -> bool {
        self.playing.as_ref().is_some_and(|played| {
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

#[cfg(test)]
mod tests {
    use openshard_gateway::{OutboxRx, outbox_channel, version_channel};

    use super::*;
    use crate::testing::{at_character_screen, login_server};

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
}
