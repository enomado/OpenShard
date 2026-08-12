//! State mutation driven by the shard: folding a `WorldView` snapshot into
//! [`App::entered`], and keeping the eye on the body it names in
//! [`App::follow_player`].
//!
//! Both read the wire's own words as ground truth — a correction is trusted
//! outright, same as an ordinary update is — which is the property that
//! keeps this file apart from `ui_command.rs`: everything here answers to a
//! packet, and everything there answers to a key or a click.

use std::time::Instant;

use openshard_client_net::view::{Heard, WorldView};
use openshard_client_render::control::Follow;
use openshard_client_render::items::GroundItem;
use openshard_client_render::mobiles;
use openshard_movement::Terrain;
use openshard_protocol::server_packet::ServerPacket;
use openshard_uofiles::anim::is_ghost;

use crate::app::App;
use crate::world::cluttered;
use crate::{clutter, crowd, link};

impl App {
    /// Reduce one cross-thread update at the event-loop boundary.
    pub(crate) fn on_update(&mut self, update: link::Update) -> bool {
        let now = Instant::now();
        self.world
            .crowd
            .advance(now.saturating_duration_since(self.last_advance));
        self.last_advance = now;
        match update {
            link::Update::World { view, body } => self.entered(*view, body, None),
            link::Update::Mutation { packet, body } => self.apply_mutation(&packet, body),
            link::Update::Prediction(body) => self.apply_prediction(body),
            link::Update::Animation(animation) => {
                self.world.crowd.play(
                    Some(animation.serial),
                    animation.action,
                    animation.frame_count,
                    animation.repeat_count,
                    animation.forward,
                    animation.repeat,
                    animation.delay,
                );
            }
            link::Update::Lost(reason) => {
                eprintln!("disconnected: {reason}");
                self.world.link = None;
                return false;
            }
        }
        let soon = now + crate::GLIDE_INTERVAL;
        if self.world.crowd.anyone_gliding() && self.next_tick > soon {
            self.next_tick = soon;
        }
        true
    }

    /// Apply a local UI mutation on the same thread that owns `WorldView`.
    pub(crate) fn apply_close_window(&mut self, target: link::CloseTarget) {
        let Some(view) = self.world.authoritative.view.as_mut() else {
            return;
        };
        match target {
            link::CloseTarget::Paperdoll(serial) => view.paperdoll_closed(serial),
            link::CloseTarget::Container(serial) => view.container_closed(serial),
            link::CloseTarget::Gump(gump_id) => view.gump_closed(gump_id),
        };
    }

    /// Apply one network mutation on the event-loop thread. This is the only
    /// place after connection setup that mutates the client-owned view.
    pub(crate) fn apply_mutation(&mut self, packet: &ServerPacket, body: link::Body) {
        let Some(mut view) = self.world.authoritative.view.take() else {
            return;
        };
        let previous_latest = view.journal.back().cloned();
        view.apply(packet);
        view.player_stepped(body.predicted.position, body.predicted.facing);
        self.entered(*view, body, previous_latest);
    }

    /// Apply prediction without changing authoritative server state.
    pub(crate) fn apply_prediction(&mut self, body: link::Body) {
        self.world.player.at = body.predicted.position;
        self.world.player.facing = body.predicted.facing.direction;
        self.world.player.drawn = self
            .world
            .crowd
            .drawn_for(self.world.me())
            .unwrap_or_else(|| openshard_client_render::follow::Gaze::on(body.predicted.position));
        self.follow_player(std::time::Duration::ZERO);
    }

    /// Redraw from what the server has shown us.
    ///
    /// A projection of the whole [`WorldView`], rebuilt each time rather than
    /// patched: the view is the record of what arrived, and anything kept in
    /// step with it by hand would be a second record that could disagree.
    pub(crate) fn entered(&mut self, view: WorldView, body: link::Body, previous_latest: Option<Heard>) {
        // The facet is chosen at startup and `0x1B` names only its size, so a
        // shard serving a different one draws this client the wrong ground with
        // no complaint from either end. Said once, because it is a
        // misconfiguration and not an event.
        if !self.world.authoritative.facet_checked {
            self.world.authoritative.facet_checked = true;
            if u32::from(view.map.width) != self.resources.map.width()
                || u32::from(view.map.height) != self.resources.map.height()
            {
                eprintln!(
                    "the shard's facet is {}x{} and {} is {}x{}: the ground drawn is not the ground you are standing on",
                    view.map.width,
                    view.map.height,
                    self.resources.map.facet_name(),
                    self.resources.map.width(),
                    self.resources.map.height(),
                );
            }
        }

        // Our own body is drawn where this end *predicted* it, not where the
        // last ack put it: the step leaves the moment the player asks for it and
        // the `0x22` confirming it arrives a round trip later, so a body drawn
        // from the view stands still for the latency and then crosses its tile
        // in a hurry. See `link::Body`.
        //
        // A correction is the one thing that is not walked into: the tile it
        // puts the body back on was never crossed.
        let me = Some(view.player.serial);
        // Ours is the one body whose pace is not guessed at: we send its steps.
        // Said every update rather than once, because the serial is the shard's
        // to name and nothing here is told when it does.
        self.world.crowd.commanding(me);
        // A rollback is also the one thing that makes `steer.rs`'s idea of which
        // way this body was last sent a lie — it is a step ahead of the shard on
        // purpose, and a refusal is the shard saying that step never happened.
        // Left uncorrected, the step after a `0x21` is decided against a facing
        // nobody has: it is timed as a turn when it is a step, or as a step when
        // it is a turn, and either is a beat of the walk in the wrong place.
        if body.corrected {
            self.steer.corrected(body.predicted.facing.direction);
        }
        self.world.player = match body.corrected {
            true => self.world.crowd.snap(
                me,
                body.predicted.position,
                view.player.body,
                body.predicted.facing,
                view.player.hue,
                // A ghost stands with no sword drawn even if `war` is still
                // set — D9's `!InWarMode || IsDead`.
                view.player.war && !view.player.dead,
            ),
            false => self.world.crowd.see(
                me,
                body.predicted.position,
                view.player.body,
                body.predicted.facing,
                view.player.hue,
                // Our own stance is the `0x72`'s and the `0x88`'s, not a bit of
                // a `0x77` — no `0x77` ever describes this body. See
                // `view::Player::war` beside `view::Mobile::war`. Gated on
                // death for the same reason as the branch above.
                view.player.war && !view.player.dead,
            ),
        };
        self.world.player.equipment = crowd::worn(&view.player.equipment, &self.resources.tiledata).into();
        // Sorted by serial for the same reason, and for one more: two items on
        // one tile at one height are drawn in the order they arrive here, so an
        // order that changed every frame would flicker.
        //
        // Before the cutaway guard below, and not with the other projections
        // further down, because that guard asks what this client can already see
        // in its way — and a barrel it was told about in the very packet being
        // folded in is part of that.
        let mut items: Vec<_> = view.items.iter().collect();
        items.sort_unstable_by_key(|(serial, _)| serial.raw());
        self.world.items.clear();
        self.world.item_serials.clear();
        for (serial, item) in items {
            self.world.items.push(GroundItem {
                at: item.position,
                graphic: item.graphic,
                hue: item.hue,
            });
            self.world.item_serials.push(*serial);
        }
        // The same list read for a second question — not what to draw, but what
        // a step cannot go through. Rebuilt here rather than per decision: one
        // click plans a route over hundreds of tiles, and each of them would
        // otherwise rescan everything on screen. See `clutter.rs`.
        self.world.clutter = clutter::Clutter::of(&self.world.items, &self.resources.tiledata);
        // `cutaway_at` follows the same prediction `player.at` does, with one
        // guard: it only ever advances to a tile the client's own static map
        // agrees is reachable from the one it already held. A correction is
        // the server's own word and is trusted outright, same as `player.at`
        // is; an optimistic step is only trusted here when it is not one
        // `Steering::detour` is going to have offered into a wall this end
        // can already see — see the field's own doc for why.
        self.world.cutaway_at = match body.corrected {
            true => body.predicted.position,
            false => {
                let terrain = cluttered(&self.world, &self.resources);
                match terrain.can_step(self.world.cutaway_at, body.predicted.position) {
                    Some(_) => body.predicted.position,
                    None => self.world.cutaway_at,
                }
            }
        };
        // Sorted by serial: a `HashMap`'s order is not one, and an atlas built
        // in a different order every frame is a rebuild every frame.
        let mut others: Vec<_> = view.mobiles.iter().collect();
        others.sort_unstable_by_key(|(serial, _)| serial.raw());
        self.world.others = others
            .into_iter()
            .map(|(serial, mobile)| {
                let who = Some(*serial);
                // Their stance is a bit of the flag byte the same packet
                // carried — `view::Mobile::war` — so a shopkeeper who draws a
                // sword changes how they stand on the next `0x77` about them.
                // A ghost is drawn no sword regardless: nothing on the wire
                // says a stranger died, but their body id does — see
                // `is_ghost` — the same D9 gate the player's own body gets.
                let mut drawn = self.world.crowd.see(
                    who,
                    mobile.position,
                    mobile.body,
                    mobile.facing,
                    mobile.hue,
                    mobile.war() && !is_ghost(mobile.body),
                );
                drawn.equipment = crowd::worn(&mobile.equipment, &self.resources.tiledata).into();
                (who, drawn)
            })
            .collect();
        // Whoever the view no longer holds walked out of range, and their clock
        // goes with them. Our own body is kept by its serial like anyone else's;
        // the placeholder's `None` is gone the moment a shard names us, which is
        // right — it was never a mobile.
        self.world.crowd.retain(|who| {
            who.is_some_and(|serial| serial == view.player.serial || view.mobiles.contains_key(&serial))
        });
        self.world.connection = format!("in world as 0x{:08X}", view.player.serial.raw());
        // The newest line in the journal, heard once and hung over its
        // speaker's head for a while — compared against the old view, still
        // in `self.world.authoritative.view` at this point, so a redraw that changed nothing else
        // does not restart the hold on the same sentence. A system line
        // (`serial: None`) has no mobile to hang over and is left for the
        // HUD's world window instead, which is not built yet.
        if let Some(latest) = view.journal.back() {
            let already_heard = previous_latest.as_ref() == Some(latest);
            if !already_heard {
                if let Some(serial) = latest.serial {
                    self.world
                        .crowd
                        .hear(Some(serial), latest.text.clone(), latest.font, latest.hue);
                }
            }
        }
        // Whole, for the HUD's world window: the three projections above are
        // what the renderer wants, and none of them keeps a serial.
        self.world.authoritative.view = Some(Box::new(view));
        // The camera follows the body, which is what `0x20` is for — unless it
        // has been unlocked, in which case the eye is the mouse's and the body
        // is free to walk off the screen. `Home` puts it back. After the view is
        // stored, because that is what says who we are, and the glide is keyed
        // by it.
        //
        // Zero, for the reason `App::walk_offline` says: a packet is not a
        // frame. The crowd's clock was brought up to date before this fold, so
        // there is no elapsed time left to hand a rig anyway.
        self.follow_player(std::time::Duration::ZERO);
    }

    /// Point the eye at our own body, wherever the glide has it this instant.
    ///
    /// Called every frame and not only when a step arrives: the glide moves the
    /// body a few pixels per frame, and an eye that moved a tile at a time would
    /// jerk the whole world under it. Reads the crowd's clock straight, so it is
    /// also what keeps the eye and the sprite from disagreeing by a frame.
    ///
    /// `elapsed` is the same span the crowd's clock was just advanced by, and
    /// deliberately the same value: a rig that filters is integrating over it,
    /// and a camera integrating a different amount of time than the body moved
    /// through lags by whatever the difference was — which varies frame to
    /// frame, and varying lag is what an eye reads as a stutter.
    pub(crate) fn follow_player(&mut self, elapsed: std::time::Duration) {
        self.world.player.drawn = self.world.drawn_player();
        let gaze = mobiles::gaze(&self.world.player);
        self.control.follow_body(gaze, elapsed);
        // What the eye was asked for, what the screen was given, and what the
        // filter had before the quantiser — the three the bench records, from
        // the one place the camera is advanced.
        //
        // Only while the eye is the body's: unlocked, the camera is wherever a
        // hand left it and a lag against a body it is not following is not a
        // number about the rig.
        if let Some(state) = self.control.eye_exact() {
            if self.control.follow() == Follow::Body {
                self.scope
                    .record(elapsed, gaze, self.control.camera().eye(), state);
            }
        }
    }
}
