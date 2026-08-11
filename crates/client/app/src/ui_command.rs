//! State mutation driven by the player's own input: movement and targeting.
//!
//! [`App::walk`] and [`App::walk_toward_cursor`] are the keyboard's and the
//! mouse's own shares of taking a step; [`App::use_under_cursor`] and
//! [`App::attack_under_cursor`] are the single- and double-click, aimed by
//! the same pick the highlight was drawn from. [`ask_between`] and
//! [`heading_between`] are the bearing arithmetic both movement methods
//! share, pulled free of `&self` so they can be checked against a drawn
//! picture rather than a running window — see their own docs.
//!
//! Deliberately apart from `own_windows.rs`, which is also player input: a
//! gump press and a walk order are different subsystems that happen to both
//! start at the mouse, and folding them into one file would make "what does
//! a click do" answerable only by reading the whole thing.

use std::time::Instant;

use openshard_client_render::camera::{self, Camera};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::{items, mobiles};
use openshard_movement::{Heading, Lean};
use openshard_protocol::direction::{Direction, Facing};
use openshard_protocol::world::Point;

use crate::app::App;
use crate::presentation::{cluttered, cluttered_with_doors_open, terrain};
use crate::{DEAD_ZONE, TURN_ZONE, steer};

impl App {
    /// Take a step, answering whether anything on screen changed.
    ///
    /// Movement is clamped to the map rather than wrapped: walking off the north
    /// edge in UO is impossible, and a camera that wrapped would draw a seam
    /// between two sides of the world.
    pub(crate) fn walk(&mut self, facing: Facing) -> bool {
        // A hand on the body outranks a scenario, the same way a hand on the
        // camera outranks the lock: the two would otherwise both write the
        // player's position and the picture would be neither.
        self.replay = None;

        // Connected, the keyboard moves nothing: it asks. The body goes where
        // the `0x22` says it went, which is the whole point of the walk
        // handshake — a client that stepped locally and corrected later would
        // be predicting, and the prediction lives in `Walk` where it can be
        // rolled back.
        if let Some(link) = self.world.link.as_ref() {
            link.step(facing);
            return false;
        }

        // Turning costs no ground here either, now decided by the same rule
        // the online handshake and the server share
        // (`openshard_movement::intend`) rather than the simplification this
        // used to be — every call moving the body, turn or not, because there
        // was no server round trip to tell the two apart. That was rarely
        // visible when a fresh direction changed once in a while; it stopped
        // being rare once `Steering::detour` started sending several
        // direction changes a hold's worth apart in real cadence, but one
        // right after another within a single event-loop wake — and moving
        // the body on every one of them was a real body covering twice the
        // ground its pace implied.
        let turn = matches!(
            openshard_movement::intend(
                self.world.player.at,
                Facing::walking(self.world.player.facing),
                facing
            ),
            openshard_movement::Intent::Turned { .. }
        );
        let (x, y) = match turn {
            true => (self.world.player.at.x, self.world.player.at.y),
            false => {
                let (dx, dy) = facing.direction.step();
                let x =
                    (i32::from(self.world.player.at.x) + dx).clamp(0, self.resources.map.width() as i32 - 1);
                let y =
                    (i32::from(self.world.player.at.y) + dy).clamp(0, self.resources.map.height() as i32 - 1);
                (x as u16, y as u16)
            }
        };
        // On the surface there — the ground's average, or the highest platform
        // static's deck a step reaches — not at some height of the camera's,
        // and not the land alone: a mobile below the terrain is correctly
        // hidden by it, which is what the depth buffer is for and what looks
        // exactly like a mobile that failed to draw, and the same held for a
        // pier or a bridge before their deck was weighed. `predict_step` rather
        // than `predict_z` because reaching from the surface underfoot is what
        // climbs a staircase; the nearest-height guess walks through it. See
        // `link.rs`'s online `Command::Step`, which wants the identical answer
        // once a server is involved.
        let terrain = terrain(&self.resources);
        let ground =
            i8::try_from(terrain.predict_step(self.world.player.at, x, y)).unwrap_or(self.world.player.at.z);
        // The crowd's clock first, before the step is folded in, and for the
        // same reason `App::user_event` does it for a step off the wire: a step
        // is timestamped with `Crowd`'s own `now`, and this is called from
        // `about_to_wait` — where that clock is as old as the last frame. A step
        // recorded up to a frame in the past starts its crossing there, and
        // `crowd::crossing` then measures the time it has left from the same
        // stale instant. This is the offline half of the walk and it had the
        // defect the online half was already fixed for.
        let now = Instant::now();
        self.world
            .crowd
            .advance(now.saturating_duration_since(self.last_advance));
        self.last_advance = now;
        // Through the crowd like anyone else, so the placeholder walks when it
        // walks and stands when it stops. `None` is who it is: no shard has
        // named it, so it has no serial.
        // `Crowd::see` starts a fresh `Mobile` with no equipment — nobody
        // sent this placeholder a `0x78` — so whatever it was already wearing
        // is carried across by hand, the way `WorldView` carries it across a
        // `0x77`/`0x20` that names none either.
        let equipment = std::mem::take(&mut self.world.player.equipment);
        self.world.player = self.world.crowd.see(
            None,
            Point::new(x, y, ground),
            self.world.player.body,
            facing,
            self.world.player.hue,
            // At peace, and not a placeholder for an unknown: this is the
            // offline viewer's body, and there is no shard to have put it in a
            // stance. See `App::walk`'s own docs.
            false,
        );
        self.world.player.equipment = equipment;
        // Offline there is no shard to refuse a step, so nothing here is
        // speculative the way an online prediction is — trusted outright,
        // same as a correction is.
        self.world.cutaway_at = self.world.player.at;
        // Offline the body is what the camera is locked to, exactly as the
        // server's is when there is a server. Unlocked, walking still walks and
        // the body may leave the screen — walking and looking are different
        // questions, and `Home` is the answer to the second.
        //
        // No time has passed: this is an input, not a frame. A rig that filters
        // integrates over the span it is given, and time passes in `App::draw`.
        self.follow_player(std::time::Duration::ZERO);
        true
    }

    /// Send the body to whatever tile the cursor is over, answering whether
    /// anything on screen changed.
    ///
    /// The mouse's whole share of walking: a click names a destination and a
    /// drag restates it, and `steer.rs` is what turns either into one step every
    /// step's length. A cursor that is off the map or outside the world's
    /// viewport names no tile and is left alone rather than treated as the
    /// nearest one — a move order nobody gave is worse than one that did
    /// nothing.
    /// The mouse's whole share of walking, one call for both of its idioms:
    /// `self.input.ctrl_held` says which. Without Ctrl this is a heading — no map
    /// touched, no route planned, the same "run toward the cursor" a strategy
    /// game's held mouse button means. With it, a move order: a route planned
    /// with `find_path` to the exact tile. See `steer.rs`'s module docs for why
    /// they are not the same thing wearing one name.
    pub(crate) fn walk_toward_cursor(&mut self) -> bool {
        // As above: between frames, what is on screen is what the last frame drew.
        let Some(tile) = self.pick_tile(*self.control.camera()) else {
            return false;
        };
        let opened = cluttered_with_doors_open(&self.world, &self.resources);
        let cluttered = cluttered(&self.world, &self.resources);
        let ground = steer::Ground {
            real: &cluttered,
            through_doors: &opened,
        };
        let facing = if self.input.ctrl_held {
            self.steer.go_to(
                (tile.at.x, tile.at.y),
                self.world.player.at,
                Instant::now(),
                self.world.player.facing,
                ground,
            )
        } else {
            self.steer.steer(
                self.ask_to_cursor(*self.control.camera()),
                self.world.player.at,
                Instant::now(),
                self.world.player.facing,
                ground,
            )
        };
        match facing {
            Some(facing) => {
                // The marker under the destination has moved even when the step
                // itself changes nothing on screen, so the redraw is not the
                // step's to decide.
                self.walk(facing);
                true
            }
            None => true,
        }
    }

    /// Which way the cursor is asking the body to walk — measured **on the
    /// screen**, from where the body is drawn, not in the world's grid.
    ///
    /// The two are not the same question, and the screen one is the only one
    /// the player is actually asking. A player pushes the mouse away from the
    /// character in the direction they want it to go; what "that direction"
    /// means is a bearing on a flat picture. The grid is where the answer has
    /// to land — one of eight tile steps — but it is not where the ask lives,
    /// and measuring in the grid quietly swaps the isometric projection for
    /// nothing. That the two happen to agree for the projection drawn today
    /// (`camera::project` is a rotation and a uniform scale, and rounding to a
    /// sector survives that) is a coincidence of the numbers in it, not a
    /// property of the idea — change the tile to a 2:1 diamond, which is what
    /// most isometric art is, and the grid answer starts naming a direction
    /// the cursor is nowhere near.
    ///
    /// The origin is the body's own projected pixel and not the middle of the
    /// viewport, which is what makes this survive a camera that is not locked
    /// to the body: with a free eye the character is off-centre, sometimes far
    /// off-centre, and "away from the middle of the screen" would be a
    /// different direction from "away from the character". Both are defensible
    /// idioms and a shard may one day want the other; this is the one that
    /// keeps meaning what it means while the eye wanders.
    ///
    /// The sector is picked by the largest dot product against the eight
    /// directions' *projected* steps — normalised, since a diagonal projects to
    /// a longer screen vector than a cardinal and the unnormalised comparison
    /// would hand it sectors it has not earned. Those steps come from
    /// `camera::project` itself rather than from constants copied out of it, so
    /// there is one projection in this client and this reads it.
    ///
    /// How far it is asking from is the other half, and it decides *what* is
    /// asked for rather than only which way: a cursor held close in turns the
    /// body and walks it nowhere. [`ask_between`] is the rings; [`TURN_ZONE`]
    /// is the one that matters here.
    ///
    /// `None` when the cursor is on the body: no bearing exists, and picking
    /// one would be inventing an ask.
    pub(crate) fn ask_to_cursor(&self, camera: Camera) -> Option<steer::Ask> {
        let cursor = self.control.cursor();
        // The body's *drawn* pixel, height and all: what a player aims relative
        // to is the sprite they can see, not the tile beneath it.
        ask_between(camera::project(self.world.player.at), camera.pick(cursor))
    }

    /// Double-click whatever the cursor is over: ask the shard to use it.
    ///
    /// **Picked against the picture, not against the tile.** A door's leaf is
    /// drawn two tiles up the screen from the tile it stands on, so the tile
    /// under the cursor is the one *behind* it — the answer
    /// [`App::pick_tile`] gives, which is right for the Tile panel and wrong for
    /// this. [`items::pick`] hits the sprite's own opaque texels instead, which
    /// is what the player thinks they clicked on.
    ///
    /// Entities only: the map's statics are not entities and have no serial to
    /// name. What this covers is doors, containers, everything else the shard
    /// has put on the ground — and now mobiles, whose double-click the shard
    /// answers with a `0x88` and which this client can finally draw (see
    /// [`paperdoll`] and `WindowSubject::Paperdoll`).
    ///
    /// Nothing is done locally on the way out. The door swings when the `0x1A`
    /// that redraws it arrives; a client that also opened it itself would show
    /// a door the shard may have refused (a lock, or reach) standing open.
    pub(crate) fn use_under_cursor(&self, camera: Camera) {
        // The same question the highlight is drawn from, so the two cannot
        // disagree about whether the world owns the mouse: a click that arrives
        // while a panel holds the pointer is the panel's.
        if !self.world_owns_pointer() {
            return;
        }
        // The atlas is the frame's, and it is where the art the click is tested
        // against lives — offline, or before the first frame, there is nothing
        // drawn to have clicked on.
        let Some(window) = self.window.as_ref() else {
            return;
        };
        // The same cutaway the frame was drawn with, computed the same way: a
        // barrel hidden under a roof this client is not drawing is not something
        // the player can have pointed at.
        let cutaway = Cutaway::at(
            &self.resources.map,
            &self.resources.tiledata,
            self.world.cutaway_at,
            true,
        );
        // A creature under the cursor takes the click, and no item is used: it
        // is what the highlight is telling the player they are pointing at, and
        // using the barrel *behind* the shopkeeper is the one answer that is
        // certainly wrong. What a mobile's double-click asks for is the
        // paperdoll — the same `0x06` an item gets, answered differently by the
        // shard (`DoubleClick::interpret`), which is why nothing here says
        // "paperdoll" on the way out.
        let drawn = self.drawn_now(&window.atlases.mobiles);
        let on_mobile = mobiles::pick(
            &drawn.iter().map(|(_, mobile)| mobile.clone()).collect::<Vec<_>>(),
            &camera,
            &window.atlases.mobiles,
            &cutaway,
            &self.resources.equip_conv,
            self.control.cursor(),
        );
        if let Some(index) = on_mobile {
            // A body with no serial is one this client is drawing without the
            // shard having named it — the offline viewer's placeholder — and
            // there is nothing to ask about.
            if let (Some(serial), Some(link)) = (drawn[index].0, self.world.link.as_ref()) {
                link.use_object(serial);
            }
            return;
        }
        let Some(index) = items::pick(
            &self.world.items,
            &camera,
            &self.resources.tiledata,
            &self.world.tile_animations,
            &window.atlases.statics,
            &cutaway,
            self.control.cursor(),
        ) else {
            return;
        };
        let serial = self.world.item_serials[index];
        match self.world.link.as_ref() {
            Some(link) => link.use_object(serial),
            None => tracing::info!(serial = serial.raw(), "nothing used: no shard is connected"),
        }
    }

    /// Aim at whatever body the cursor is on, if this character is at war.
    ///
    /// The single click's half of a fight, beside [`App::use_under_cursor`]'s
    /// double click. Three gates, and each is a different kind of "no":
    ///
    /// - **At peace, nothing is sent.** Not a refusal — a click at peace is a
    ///   selection, which the caller has already made.
    /// - **A ghost sends nothing.** `Player::dead` is `0x2C`'s own answer
    ///   (`docs/combat.md`, D9/P4) — a ghost that somehow still carries the war
    ///   flag still sends no attack, the same `!InWarMode || IsDead` the
    ///   reference gates a swing by.
    /// - **A body with no serial is the offline viewer's placeholder** and
    ///   there is nothing to name.
    ///
    /// Reads [`App::on_mobile`] — what the *last frame* found under the cursor,
    /// the same answer the highlight is drawn from — rather than picking again
    /// here. That is `use_under_cursor`'s own rule turned into the one it should
    /// always have been: what the player clicked is what they were shown they
    /// were pointing at.
    pub(crate) fn attack_under_cursor(&self) {
        let Some(view) = self.world.view.as_ref() else {
            return;
        };
        if !view.player.war || view.player.dead {
            return;
        }
        // `on_mobile` is a `Who` — `None` inside the `Some` is a body no shard
        // has named, which the offline viewer draws and nothing can be asked
        // about.
        let Some(Some(mobile)) = self.picking.on_mobile else {
            return;
        };
        // Attacking yourself is a packet the shard refuses (`combat::attack`
        // checks it) and a click a player never means. Stopped here so the
        // refusal is not a round trip.
        if mobile == view.player.serial {
            return;
        }
        match self.world.link.as_ref() {
            Some(link) => link.attack(mobile),
            None => tracing::info!(serial = mobile.raw(), "nothing attacked: no shard is connected"),
        }
    }
}

/// The heading from one point on the screen to another, as one of the eight
/// ways a body can walk plus which side of that way it actually points.
///
/// Split out of [`App::heading_to_cursor`] because it is the whole of the
/// arithmetic and none of the state — a thing that can be checked against a
/// drawn picture rather than against a running window.
///
/// The sector is the largest dot product against the eight directions'
/// *projected* steps, normalised: a diagonal projects to a longer screen vector
/// than a cardinal (44 pixels against 31), and comparing unnormalised would
/// hand the diagonals sectors they have not earned. Those steps come from
/// [`camera::project`] rather than from constants copied out of it, so there is
/// one projection in this client and this reads it.
///
/// Three rings, and the distance is what picks one.
///
/// `None` inside [`DEAD_ZONE`] of the body: a cursor that close is not naming a
/// direction, and answering one anyway is what makes a body with the button
/// held and the mouse sitting still walk at random — the vector is a couple of
/// pixels long, so which of the eight sectors it lands in is decided by the
/// hand's own jitter, and every twitch of the mouse re-rolls it.
///
/// [`steer::Ask::Turn`] out to [`TURN_ZONE`]: the bearing is real by then, and
/// what is not real is the *step* — from that close it lands past the cursor
/// that asked for it. So the body faces the way it was pointed and stays where
/// it is, which is also the only way a mouse can ask a character to turn.
///
/// [`steer::Ask::Walk`] beyond it.
pub(crate) fn ask_between(body: camera::WorldPixel, cursor: camera::WorldPixel) -> Option<steer::Ask> {
    let (dx, dy) = (cursor.x - body.x, cursor.y - body.y);
    let reach = f64::from(dx * dx + dy * dy);
    if reach <= DEAD_ZONE * DEAD_ZONE {
        return None;
    }
    let heading = heading_between(dx, dy)?;
    Some(match reach < TURN_ZONE * TURN_ZONE {
        true => steer::Ask::Turn(heading),
        false => steer::Ask::Walk(heading),
    })
}

/// Which of the eight ways the offset `(dx, dy)` points, and which side of it —
/// the whole of the arithmetic and none of the zones, so that
/// [`ask_between`]'s rings and this can be argued with one at a time.
pub(crate) fn heading_between(dx: i32, dy: i32) -> Option<Heading> {
    let direction = Direction::ALL.into_iter().max_by(|a, b| {
        let cosine = |direction| {
            let (sx, sy) = on_screen(direction);
            let dot = f64::from(dx) * f64::from(sx) + f64::from(dy) * f64::from(sy);
            dot / f64::from(sx * sx + sy * sy).sqrt()
        };
        cosine(*a).total_cmp(&cosine(*b))
    })?;
    let (sx, sy) = on_screen(direction);
    Some(Heading {
        direction,
        // A cross product needs no normalising, so the lean stays exact: a
        // cursor squarely on a direction's screen bearing leans neither way and
        // says so without a tolerance. The projection turns the plane without
        // flipping it, so "clockwise" means on the screen what it means on the
        // grid — see `Lean::of`.
        lean: Lean::of(sx, sy, dx, dy),
    })
}

/// One step's worth of the projection, taken from the projection.
///
/// The origin tile is arbitrary and cancels in the subtraction; it is away from
/// the map's edges only so that neither end of it has to clamp.
pub(crate) fn on_screen(direction: Direction) -> (i32, i32) {
    let origin = Point::new(1000, 1000, 0);
    let (sx, sy) = direction.step();
    let stepped = Point::new(
        (i32::from(origin.x) + sx) as u16,
        (i32::from(origin.y) + sy) as u16,
        0,
    );
    let (a, b) = (camera::project(origin), camera::project(stepped));
    (b.x - a.x, b.y - a.y)
}
