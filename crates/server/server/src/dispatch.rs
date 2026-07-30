use super::*;

/// Turn a packet the world cares about into a command. `false` closes.
///
/// Nothing here answers the client. Every reply comes out of a tick, which is
/// what keeps the two ends in one order.
///
/// `packet` is already decoded — `parse_packet` in `shard.rs` does that once,
/// before routing here, so a malformed packet never reaches this function at
/// all; it closes the connection at the routing step instead.
pub(crate) fn dispatch_world_packet(
    session: &mut Session,
    world: &mut World,
    packet: ClientPacket,
    id: ConnectionId,
    roster: &Roster,
    access: AccessLevel,
) -> bool {
    match packet {
        ClientPacket::CharacterPlay(play) => {
            // Same guard as `create_character`/`delete_character`: no game login
            // behind this connection means no account to enter with, and a
            // default would enter the character on an account that never
            // authenticated it.
            let Some(account) = session.login.account().cloned() else {
                warn!(%id, "character-play before a game login");
                return false;
            };
            let name = CharacterName(play.name.0);
            // A stored character enters on its saved serial, spot and look; one
            // the roster has never heard of — a config-only character on a fresh
            // shard, or one created this run and not yet saved — enters fresh at
            // the start. Unpacking the row is `StoredCharacter::from_record`'s
            // job, not this one's.
            let character = roster
                .get(&account, &name)
                .and_then(StoredCharacter::from_record)
                .map_or_else(|| Character::fresh(Facet(0)), Character::Stored);
            // The connection's own note of what it is playing, taken as the
            // `Enter` is queued. It is what tells another connection on this
            // account that the character is in use — see `Sessions::is_playing`.
            session.enter_world(account.clone(), name.clone());
            // Tell the gateway framer this client's version now, before any
            // in-world packet whose length depends on it (the drop packet). The
            // game connection never stated its version; this is the auth-key-linked
            // one the login carried across. Character select is the last quiet
            // moment before world traffic starts.
            let _ = session.control.send(session.login.version());
            world.queue(Command::Enter(Entering {
                connection: id,
                version: session.login.version(),
                account,
                name,
                access,
                character,
            }));
            true
        }
        ClientPacket::Walk(request) => {
            if !session.in_world() {
                debug!(%id, "0x02 before entering the world");
                return true;
            }
            world.queue(Command::Walk {
                connection: id,
                request,
            });
            true
        }
        ClientPacket::LogoutRequest => {
            // "Log Out" on the paperdoll. The client tells the server it is
            // leaving and then waits to be told it may — see `world::LogoutAck`.
            // Queued like everything else, so the answer comes out of a tick.
            if session.in_world() {
                world.queue(Command::LogoutRequest { connection: id });
            }
            true
        }
        ClientPacket::StatusQuery(query) => {
            if session.in_world() {
                match query.kind {
                    StatusQueryKind::Skills => {
                        world.queue(Command::RequestSkills { connection: id });
                    }
                    StatusQueryKind::Status => {
                        world.queue(Command::RequestStatus { connection: id });
                    }
                }
            }
            true
        }
        ClientPacket::Encoded(command) => {
            // The AoS "encoded command": the paperdoll's own buttons, which are
            // not gump replies — the paperdoll is drawn client-side and has no
            // server layout to answer. Without this the Quest button does nothing
            // at all, with nothing anywhere to say why.
            if !session.in_world() {
                return true;
            }
            match command.subcommand.interpret() {
                EncodedSubcommand::QuestGumpRequest => {
                    world.queue(Command::QuestLogRequest { connection: id });
                }
                // Named, not routed: combat has no weapon abilities and `guilds`
                // is a stub. Naming them means the byte layout is not re-derived
                // the day either lands.
                EncodedSubcommand::SetAbility | EncodedSubcommand::GuildGumpRequest => {}
                EncodedSubcommand::Other(other) => {
                    debug!(subcommand = format!("0x{other:02X}"), "unhandled 0xD7");
                }
            }
            true
        }
        ClientPacket::GumpResponse(response) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::GumpResponse {
                connection: id,
                response,
            });
            true
        }
        ClientPacket::TargetResponse(response) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::TargetResponse {
                connection: id,
                response,
            });
            true
        }
        ClientPacket::PickUpItem(pickup) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::PickUpItem {
                connection: id,
                serial: pickup.serial,
                amount: pickup.amount,
            });
            true
        }
        ClientPacket::DropItem(drop) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::DropItem {
                connection: id,
                serial: drop.serial,
                position: drop.position,
                container: drop.container,
            });
            true
        }
        ClientPacket::SecureTrade(action) => {
            if !session.in_world() {
                return true;
            }
            match action {
                SecureTradeAction::Cancel { container } => {
                    world.queue(Command::TradeCancel {
                        connection: id,
                        container,
                    });
                }
                SecureTradeAction::Accept { container, accepted } => {
                    world.queue(Command::TradeAction {
                        connection: id,
                        container,
                        accepted,
                    });
                }
                // Virtual gold and platinum: an account balance this shard does
                // not keep. Gold is an item, and it trades by being dragged into
                // the window like anything else.
                SecureTradeAction::UpdateGold { .. } => {}
            }
            true
        }
        ClientPacket::DoubleClick(click) => {
            if !session.in_world() {
                return true;
            }
            // The paperdoll bit comes off here, where the packet is read: it is
            // framing the client owns, and `interpret` is total, so nothing
            // downstream has to know which bit it was.
            world.queue(Command::DoubleClick {
                connection: id,
                request: click.interpret(),
            });
            true
        }
        ClientPacket::Buy(reply) => {
            // A vendor purchase, answered out of the tick like everything else.
            if !session.in_world() {
                return true;
            }
            world.queue(Command::Buy {
                connection: id,
                vendor: reply.vendor,
                purchases: reply.purchases,
            });
            true
        }
        ClientPacket::Sell(reply) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::Sell {
                connection: id,
                vendor: reply.vendor,
                sales: reply.sales,
            });
            true
        }
        ClientPacket::Look(look) => {
            if !session.in_world() {
                return true;
            }
            // A `0x09` naming nothing — zero, or `0xFFFF_FFFF` — is a click that
            // hit no object, which is an answer and not a reason to queue work.
            let Some(serial) = look.serial.validate() else {
                return true;
            };
            world.queue(Command::SingleClick {
                connection: id,
                serial,
            });
            true
        }
        ClientPacket::PropertyQuery(query) => {
            // The AoS tooltip batch query: a client hovering wants these objects'
            // property lists. Answered out of the tick like every other reply.
            if !session.in_world() {
                return true;
            }
            debug!(%id, count = query.serials.len(), "0xD6 tooltip query");
            world.queue(Command::QueryProperties {
                connection: id,
                serials: query.serials,
            });
            true
        }
        ClientPacket::Equip(equip) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::EquipItem {
                connection: id,
                item: equip.item,
                layer: equip.layer,
                mobile: equip.mobile,
            });
            true
        }
        ClientPacket::WarMode(request) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::WarMode {
                connection: id,
                war: request.war,
            });
            true
        }
        ClientPacket::Attack(request) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::Attack {
                connection: id,
                target: request.target,
            });
            true
        }
        ClientPacket::Talk(talk) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::Say {
                connection: id,
                mode: talk.mode,
                hue: talk.hue,
                font: talk.font,
                text: talk.text,
            });
            true
        }
        ClientPacket::UnicodeTalk(talk) => {
            // What a modern client actually sends when you type. Same `Say` as the
            // ASCII 0x03 once the words are out.
            if !session.in_world() {
                return true;
            }
            world.queue(Command::Say {
                connection: id,
                mode: talk.mode,
                hue: talk.hue,
                font: talk.font,
                text: talk.text,
            });
            true
        }
        ClientPacket::Extended(request) => {
            // `0xBF` is a whole family of extended commands; `ExtendedRequest`
            // has already picked the one subcommand this packet carries.
            if !session.in_world() {
                return true;
            }
            match request {
                ExtendedRequest::Cast(cast) => {
                    // interpret() is total, so it may run right here rather than
                    // waiting for a tick system to have the domain in hand — see
                    // `docs/protocol_newtypes.md`'s N4 containers amendment 2. A
                    // wire 0 is never a legitimate spell id and queues nothing.
                    if let Some(spell) = cast.spell.interpret() {
                        world.queue(Command::RequestCast {
                            connection: id,
                            spell,
                        });
                    }
                }
                ExtendedRequest::ContextMenuRequest(request) => {
                    debug!(%id, serial = request.serial.0, "0xBF context-menu request");
                    world.queue(Command::ContextMenuRequest {
                        connection: id,
                        serial: request.serial,
                    });
                }
                ExtendedRequest::ContextMenuSelect(select) => {
                    world.queue(Command::ContextMenuSelect {
                        connection: id,
                        serial: select.serial,
                        index: select.index,
                    });
                }
                ExtendedRequest::StatLock(request) => {
                    // The seam is where a client's byte becomes a stat: the
                    // status bar has three arrows, and a packet naming a fourth
                    // is dropped here rather than travelling into the tick to be
                    // ignored by a `_ =>` arm nobody can see from the packet.
                    match request.stat.validate() {
                        Ok(stat) => world.queue(Command::SetStatLock {
                            connection: id,
                            stat,
                            lock: StatLock::from_wire(request.lock.interpret()),
                        }),
                        Err(invalid) => debug!(%id, %invalid, "0xBF 0x1A named no stat"),
                    }
                }
                ExtendedRequest::Unknown(subcommand) => {
                    debug!(%id, subcommand = format!("0x{subcommand:02X}"), "unhandled 0xBF");
                }
                // `ExtendedRequest` is `#[non_exhaustive]` for callers outside
                // this workspace; every variant that exists today is matched
                // above.
                _ => unreachable!("every ExtendedRequest variant is matched above"),
            }
            true
        }
        ClientPacket::UseSkill(request) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::UseSkillButton {
                connection: id,
                skill: request.skill,
            });
            true
        }
        ClientPacket::SkillLock(request) => {
            if !session.in_world() {
                return true;
            }
            world.queue(Command::SetSkillLock {
                connection: id,
                skill: request.skill,
                lock: request.lock,
            });
            true
        }
        // A `0x12` text command that is not "use skill" reaches here as
        // Unknown, not an error — see `ClientPacket::decode`.
        ClientPacket::Unknown { id: 0x12, .. } => {
            debug!(%id, "0x12 text command we do not act on");
            true
        }
        ClientPacket::Unknown { .. } => true,
        // `ClientPacket` is `#[non_exhaustive]` for callers outside this
        // workspace; every variant that exists today is matched above.
        _ => unreachable!("every ClientPacket variant is matched above"),
    }
}

/// The starting cities offered on the character-creation screen.
///
/// The nine classic towns a new character could wake up in on the original
/// Felucca map — the same list, inns and coordinates RunUO and ServUO have
/// shipped for two decades. Their order is what matters as much as their
/// contents: `start_location` in the create packet is a raw index into this
/// list, so position N here is the city the player picked when they clicked the
/// Nth entry. `create_character` reads the same list back to place the spawn, so
/// the two agree by construction.
///
/// All nine are on facet 0, the only facet a new character starts on, so the
/// list is filtered to the facets this shard actually loaded: offering a city on
/// a facet with no terrain would spawn the player in nowhere. If that leaves it
/// empty — a shard that loaded no facet carrying a starting city — one city at
/// the configured start is kept, because the client refuses an empty list and
/// says so: "No city found. Something wrong with the received cities."
///
/// The description cliloc is left 0: a client older than 7.0.13.0 ignores the
/// field, and a newer one shows the city and inn names either way.
pub(crate) fn start_cities(facets: &[u8], start: (u16, u16)) -> Vec<StartLocation> {
    fn city(area: &str, name: &str, x: i32, y: i32, z: i32) -> StartLocation {
        StartLocation {
            area: area.to_owned(),
            name: name.to_owned(),
            position: (x, y, z),
            map: MapId(0),
            description_cliloc: ClilocId(0),
        }
    }

    let mut cities: Vec<StartLocation> = [
        city("Yew", "The Empath Abbey", 633, 858, 0),
        city("Minoc", "The Barnacle", 2476, 413, 15),
        city("Britain", "Sweet Dreams Inn", 1496, 1628, 10),
        city("Moonglow", "The Scholars Inn", 4408, 1168, 0),
        city("Trinsic", "The Traveler's Inn", 1845, 2745, 0),
        city("Magincia", "The Great Horns Tavern", 3734, 2222, 20),
        city("Jhelom", "The Mercenary Inn", 1374, 3826, 0),
        city("Skara Brae", "The Falconer's Inn", 618, 2234, 0),
        city("Vesper", "The Ironwood Inn", 2771, 976, 0),
    ]
    .into_iter()
    .filter(|city| facets.contains(&city.map.0))
    .collect();

    if cities.is_empty() {
        cities.push(StartLocation {
            area: "Britannia".to_owned(),
            name: "Britain".to_owned(),
            position: (i32::from(start.0), i32::from(start.1), 0),
            map: MapId(facets.first().copied().unwrap_or(0)),
            description_cliloc: ClilocId(0),
        });
    }
    cities
}

/// Create a character on the authenticated account, then enter the world with
/// it — the two halves of what a `0x00`/`0xF8` packet asks for.
///
/// `create` is already decoded — `parse_packet` in `shard.rs` does that once,
/// before routing here, so a malformed `0x00`/`0xF8` never reaches this
/// function at all.
///
/// Returns `false` only to drop the connection: no game login behind this
/// connection to say whose character this is. A *refused* creation — a full
/// account, an empty or duplicate name — keeps the connection. Sphere answers
/// that with the same `0x82` a login error uses, and the client stays on the
/// creation screen to try again.
pub(crate) fn create_character(
    session: &mut Session,
    login: &mut LoginServer<DevAccounts>,
    world: &mut World,
    create: CreateCharacter,
    id: ConnectionId,
) -> bool {
    let Some(account) = session.login.account().cloned() else {
        warn!(%id, "create-character before a game login");
        return false;
    };

    // `create.name` stays a `RawCharacterName` until `create_character` — the
    // only place that turns one into a real `CharacterName` — validates it;
    // no premature `.0` unwrap here, and no `impl Into<...>` sugar at the call
    // site either: the trait takes the concrete raw type, so the clones below
    // are the explicit, visible cost of needing both the raw and validated
    // forms afterwards.
    let name = match login
        .accounts
        .create_character(account.clone(), create.name.clone())
    {
        Ok((_slot, name)) => {
            info!(%id, %account, %name, "character created");
            name
        }
        Err(reason) => {
            warn!(%id, %account, %create.name, ?reason, "character creation refused");
            let _ = session.send_packet(
                ServerPacket::LoginDenied(LoginDenied { reason }).encode(session.login.version()),
            );
            return true;
        }
    };

    // Place the character in the city they picked. `start_location` indexes the
    // very list `start_cities` built and the character-list packet offered, so a
    // valid pick names a real city; only a client sending an out-of-range index
    // falls back to the default facet and a fresh spawn.
    let (facet, start) = match login.starts.get(create.start_location.0 as usize) {
        Some(city) => (
            Facet(city.map.0),
            Some(Point::new(
                city.position.0 as u16,
                city.position.1 as u16,
                city.position.2 as i8,
            )),
        ),
        None => (Facet(0), None),
    };

    session.enter_world(account.clone(), name.clone());
    let access = login.accounts.access_level(&account);
    // A brand-new character: a fresh serial, spawned in the chosen city. The tick
    // will journal it, so it is in the database — and in the character list — by
    // the next time the player logs in.
    let character = Character::Fresh(FreshCharacter {
        facet,
        start,
        appearance: Some(Appearance {
            // No promotion exists yet for either raw value — see
            // `docs/protocol_newtypes.md` — so this is still an unchecked
            // pass-through, now visible at the call site as `.0` rather than
            // hidden behind a bare integer.
            body: {
                let (sex, race) = create.sex_race.interpret();
                Graphic(CreateCharacter::body(sex, race))
            },
            hue: Hue(create.skin_hue.0),
        }),
        // The stats and skills the player chose on the creation screen. The
        // client sends whole points; skills are stored in tenths, so a chosen 50
        // becomes 500. New skills start unlocked (training up).
        //
        // None of `strength`/`dexterity`/`intelligence`/`value` is validated
        // here — no promotion exists yet for `RawStatValue`/`RawSkillValue`,
        // so `.0` below is an unchecked pass-through of client input. See
        // `docs/protocol_newtypes.md`'s pilot notes.
        sheet: Some(CharacterSheet {
            strength: u16::from(create.strength.0),
            dexterity: u16::from(create.dexterity.0),
            intelligence: u16::from(create.intelligence.0),
            skills: create
                .skills
                .iter()
                .filter(|choice| choice.value.0 > 0)
                // A cap of zero means "whatever this shard caps a skill at" —
                // `enter` fills it in from `[gameplay] skill_cap`, so the knob is
                // read in one place and this end needs to know nothing about it.
                .map(|choice| (choice.skill.0, u16::from(choice.value.0) * 10, SkillLock::Up, 0))
                .collect(),
            // A new character's arrows all point up, and no stat has ever risen.
            stat_locks: openshard_persistence::StatLockRecord::default(),
            // A new character is clean, and unknown.
            effects: Vec::new(),
            dead: false,
            fame: 0,
            karma: 0,
            murders: 0,
            quests: Vec::new(),
            done_quests: Vec::new(),
        }),
    });
    world.queue(Command::Enter(Entering {
        connection: id,
        version: session.login.version(),
        account,
        name,
        access,
        character,
    }));
    true
}

/// Delete a character from the character-select screen (`0x83`).
///
/// `delete` is already decoded — see [`create_character`]'s doc for why.
///
/// Like create, this crosses the login/world line: it drops the character from
/// the account's list *and* tells the world to forget its saved row. Returns
/// `false` only to drop the connection (no game login behind this connection);
/// a refused delete — bad slot, or a character being played — keeps the
/// connection and answers with `0x85`, and a good one resends the list with
/// `0x86`.
///
/// # Why the whole session table and not one session
///
/// The refusal this owes the client is "that character is being played", and
/// the connection asking is never the one playing it — it is sitting on the
/// character screen. It is a *second* connection on the same account, so the
/// question can only be answered by reading across all of them. Taking the
/// table by `&` rather than one session by `&mut` is also what makes that
/// possible at all; see [`Sessions::is_playing`] for what this replaced and the
/// hole it had.
pub(crate) fn delete_character(
    sessions: &Sessions,
    login: &mut LoginServer<DevAccounts>,
    world: &mut World,
    roster: &mut Roster,
    delete: DeleteCharacter,
    id: ConnectionId,
) -> bool {
    let session = sessions
        .get(id)
        .expect("world_handle_network looks the session up before routing a packet to here");
    let Some(account) = session.login.account().cloned() else {
        warn!(%id, "delete-character before a game login");
        return false;
    };

    // The slot indexes the very list the client was last sent, which is the
    // account's in-memory character list — so that list's length is the whole
    // domain, and a slot outside it names no character to look up.
    let characters = login.accounts.characters(&account);
    let slot = match delete.slot.validate(characters.len()) {
        Ok(slot) => slot,
        Err(error) => {
            warn!(%id, %account, %error, "delete refused");
            let _ = session.send_packet(
                ServerPacket::DeleteReject(DeleteReject {
                    result: DeleteResult::CharNotExist,
                })
                .encode(session.login.version()),
            );
            return true;
        }
    };
    let name = characters
        .into_iter()
        .nth(slot.0 as usize)
        .expect("validate proved the slot is inside the list this was built from")
        .name;

    // A character being played cannot be deleted out from under its session.
    // Asked of the connections and not of the world: the world knows a serial,
    // and the only way to a serial from here is the roster — which a character
    // created during this run is not in until it logs out.
    if sessions.is_playing(&account, &name) {
        let _ = session.send_packet(
            ServerPacket::DeleteReject(DeleteReject {
                result: DeleteResult::CharBeingPlayed,
            })
            .encode(session.login.version()),
        );
        return true;
    }

    // Drop it from the authoritative in-memory list. The store checks the slot
    // against its own list rather than trusting the one checked above — two
    // lists, two checks — so a failure here is still a bad slot.
    if let Err(reason) = login.accounts.delete_character(&account, delete.slot) {
        warn!(%id, %account, %name, ?reason, "delete refused");
        let _ = session.send_packet(
            ServerPacket::DeleteReject(DeleteReject {
                result: DeleteResult::CharNotExist,
            })
            .encode(session.login.version()),
        );
        return true;
    }

    // Forget the roster entry so a re-login this run does not restore it, and
    // tell the world to forget the store row and inventory on the next save. The
    // serial stays reserved — a packet in flight may still name it.
    if let Some(record) = roster.forget(&account, &name) {
        world.queue(Command::DeleteCharacter {
            serial: record.serial,
        });
    }
    info!(%id, %account, %name, "character deleted");

    // Resend the updated list so the select screen redraws.
    let _ = session.send_packet(
        ServerPacket::CharacterListUpdate(CharacterListUpdate {
            characters: login.accounts.characters(&account),
        })
        .encode(session.login.version()),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{admin, at_character_screen, login_server, lord_british};
    use openshard_protocol::wire::RawCharacterSlot;

    #[test]
    fn a_character_being_played_cannot_be_deleted_from_another_connection() {
        // The hole this closes. The check used to read `World::is_online(serial)`,
        // and the only serial available came from the roster — which a
        // character created during this run does not appear in until it logs out.
        // So for a fresh character the check did not run at all: the character
        // left the account's list while its entity kept playing, and
        // `Command::DeleteCharacter` was never queued either, because there was no
        // record to remove. The roster is empty here on purpose — that is the
        // case.
        let now = Instant::now();
        let mut login = login_server();
        let mut world = World::new((1363, 1600));
        let mut roster = Roster::new();

        let mut sessions = Sessions::new();
        let (mut playing, _playing_wire) = at_character_screen(&mut login, now);
        playing.enter_world(admin(), lord_british());
        sessions.open(ConnectionId::from_raw(1), playing);

        // A second connection on the same account, sitting on the character
        // screen. This is the only way the situation arises at all.
        let screen = ConnectionId::from_raw(2);
        let (session, mut wire) = at_character_screen(&mut login, now);
        sessions.open(screen, session);

        assert!(
            delete_character(
                &sessions,
                &mut login,
                &mut world,
                &mut roster,
                DeleteCharacter {
                    slot: RawCharacterSlot(0),
                },
                screen,
            ),
            "a refused delete keeps the connection"
        );

        // Decompressed, because the fixture is a real game socket and everything
        // that leaves one is Huffman-compressed — see `Session::send_packet`.
        let reply = huffman::decompress(&wire.try_recv().expect("the client was answered"))
            .expect("a valid Huffman stream");
        assert_eq!(
            reply,
            vec![0x85, DeleteResult::CharBeingPlayed as u8],
            "0x85, and for the right reason"
        );
        assert_eq!(
            login.accounts.characters(&admin()).len(),
            1,
            "and the character is still on the account"
        );
    }

    #[test]
    fn a_character_nobody_is_playing_is_deleted() {
        // The other direction, so the check above cannot pass by refusing
        // everything: the same connection, the same slot, with nobody in world.
        let now = Instant::now();
        let mut login = login_server();
        let mut world = World::new((1363, 1600));
        let mut roster = Roster::new();

        let screen = ConnectionId::from_raw(1);
        let mut sessions = Sessions::new();
        let (session, mut wire) = at_character_screen(&mut login, now);
        sessions.open(screen, session);

        assert!(delete_character(
            &sessions,
            &mut login,
            &mut world,
            &mut roster,
            DeleteCharacter {
                slot: RawCharacterSlot(0),
            },
            screen,
        ));

        let reply = huffman::decompress(&wire.try_recv().expect("the client was answered"))
            .expect("a valid Huffman stream");
        assert_eq!(reply[0], 0x86, "the select screen is redrawn from the new list");
        assert!(
            login.accounts.characters(&admin()).is_empty(),
            "and the character is gone"
        );
    }

    #[test]
    fn a_facet_zero_shard_offers_the_classic_towns() {
        // Facet 0 loaded — the normal case — offers the nine classic Felucca
        // cities, every one of them on map 0 with a real, non-origin position.
        let cities = start_cities(&[0], (1363, 1600));
        assert_eq!(cities.len(), 9, "the nine classic starting cities");
        assert!(
            cities.iter().any(|city| city.area == "Britain"),
            "Britain is one of them"
        );
        for city in &cities {
            assert_eq!(city.map, MapId(0), "every classic city is on Felucca");
            assert!(
                city.position.0 > 0 && city.position.1 > 0,
                "a real spot, not the origin"
            );
        }
    }

    #[test]
    fn a_shard_without_facet_zero_still_offers_one_city() {
        // An empty list is what makes ClassicUO refuse to open the creation
        // screen. No classic city lives on a non-zero facet, so a shard that
        // loaded only facet 1 keeps a single fallback at the configured start —
        // on a facet it actually loaded, not facet 0 it did not.
        let cities = start_cities(&[1], (1363, 1600));
        assert_eq!(cities.len(), 1, "never empty");
        assert_eq!(cities[0].position, (1363, 1600, 0));
        assert_eq!(cities[0].map, MapId(1), "on a loaded facet");
    }

    #[test]
    fn start_location_indexes_the_offered_list() {
        // The contract create_character depends on: the byte the client sends is
        // a raw index into exactly this list, so the Nth city is the one picked
        // by clicking the Nth entry. If this order ever shifts, spawns land in
        // the wrong town silently.
        let cities = start_cities(&[0], (1363, 1600));
        assert_eq!(cities[0].area, "Yew");
        assert_eq!(cities[2].area, "Britain");
        assert_eq!(cities[8].area, "Vesper");
    }
}
