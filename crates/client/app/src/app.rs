//! [`App`]: the composition root. Every subsystem file has an `impl App`
//! block of its own — window/GPU setup in `window.rs`, packet-driven state
//! in `net_command.rs`, input-driven state in `ui_command.rs` and
//! `own_windows.rs`, read-only queries in `picking_query.rs`, the drawing
//! pipeline in `presentation.rs`, winit glue in `event_loop.rs` — and this
//! file is deliberately the thinnest of them: the struct's fields, and the
//! handful of accessors small enough that giving them a subsystem of their
//! own would be a file for one function.
//!
//! **Not a deeper decomposition.** `App` stays one struct with ~20 fields
//! rather than a struct-per-subsystem, because nearly every method above
//! reaches across more than one of those fields — `advance_replay` alone
//! touches the world, the crowd and the scope — and splitting the storage
//! along the same lines as the files would just move the borrow conflicts
//! the free functions in `presentation.rs` already dodge by staying free.
//! The split here is *where the code that touches a field lives*, not *which
//! struct the field is on*.

use std::sync::Arc;
use std::time::{Duration, Instant};

use openshard_client_render::animation::FRAME_DELAY;
use openshard_client_render::bench::{Scope, Script};
use openshard_client_render::camera::{Camera, TileBounds};
use openshard_client_render::control::Control;
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::mobiles;
use openshard_movement::Tile;
use openshard_protocol::direction::Facing;
use openshard_protocol::world::Point;

use crate::chat::Chat;
use crate::diagnostics::{OccluderSurface, Route, TerrainOverlay};
use crate::window::Screen;
use crate::{
    GLIDE_INTERVAL, desk, frames, graphics, input, picking, replay, resources, shell, steer, windows, world,
};

pub(crate) struct App {
    /// The client's own asset files, read once and held for the run — see
    /// [`resources::Resources`].
    pub(crate) resources: resources::Resources,
    /// The debug-view and lighting switches a person has set on this run —
    /// see [`graphics::GraphicsSettings`].
    pub(crate) graphics: graphics::GraphicsSettings,
    /// What the shard, or its absence, has said the world looks like — see
    /// [`world::WorldState`].
    pub(crate) world: world::WorldState,
    /// The shard thread's staged delivery into this event-loop-owned model.
    /// Mutations are drained in order; superseded frame predictions are not.
    pub(crate) updates: crate::link::Updates,
    /// One opt-in pause after entering the world, used only by a diagnostic
    /// harness to make mailbox backpressure observable.
    pub(crate) stall_on_update: Option<Duration>,
    /// The camera, who is allowed to move it, and what a drag has not yet spent.
    ///
    /// All of it arithmetic, and all of it in `client/render` where it can be
    /// reached by a test: this crate owns a window, a GPU and a `Map`, and none
    /// of the three has anything to say about a wheel notch.
    pub(crate) control: Control,
    /// Whether the device's refusal to hold a zoom's image has been said out
    /// loud. A silently truncated target draws a smaller world into a larger
    /// rect, which looks exactly like a bug in the projection — so it is
    /// reported, and once.
    pub(crate) zoom_limit_reported: bool,
    /// The dev HUD, once there is a window to put it on.
    pub(crate) shell: Option<shell::Shell>,
    /// What the HUD looked like when the client last closed: which tab, where
    /// the dev window and the operating system's window sat, and at what scale.
    ///
    /// Read once at startup and handed to the [`shell::Shell`] when there is a
    /// window; written back in [`App::exiting`]. Held here rather than in the
    /// shell because half of it — the frame — is the *platform's* window, which
    /// the HUD does not own and cannot ask about.
    pub(crate) desk: desk::Desk,
    /// Where the player is asking to walk — the arrows, and the tile the mouse
    /// last sent the body to.
    ///
    /// A step is not sent from the input event: the operating system's
    /// auto-repeat is not a walking speed, a shard refuses a flood of steps as a
    /// speedhack, and a mouse held over the ground reports a move a pixel. One
    /// clock paces all of them. See `steer.rs`.
    pub(crate) steer: steer::Steering,
    /// The last route assembled for the development HUD.
    ///
    /// A path search is considerably more expensive than drawing its line,
    /// especially when zooming out.  The cache is keyed by the two inputs that
    /// change as the body walks, and is cleared whenever a fresh world view
    /// changes the terrain it was planned over.
    pub(crate) route_cache: Option<RouteCache>,
    /// The terrain wash for an unchanged world and camera.
    pub(crate) terrain_cache: Option<TerrainCache>,
    /// The HUD's separate occlusion grid for an unchanged world/camera view.
    pub(crate) occluder_cache: Option<OccluderCache>,
    /// What the window system and the mouse have last said — see
    /// [`input::Input`].
    pub(crate) input: input::Input,
    /// When the clock next advances a frame.
    pub(crate) next_tick: Instant,
    /// When it last did.
    ///
    /// Presentation clocks are moved by *measured* time and not by the interval
    /// that was waited for: `WaitUntil` is a floor and the compositor overshoots
    /// it, so a clock fed the nominal step would run slow by however much it did
    /// — which a stepping animation hides and a glide does not.
    pub(crate) last_advance: Instant,
    /// When the last frame was *drawn*, for the frame panel's interval.
    ///
    /// Not [`App::last_advance`], which is the clock the world is advanced on
    /// and is moved by an arriving packet as well as by a frame. Measured
    /// against that, a frame that followed a packet by a millisecond would be
    /// reported as a thousand a second, and the one number the panel exists to
    /// show — the gap between two pictures — would be the one it does not.
    pub(crate) last_frame: Instant,
    pub(crate) window: Option<Screen>,
    /// What the last frame's HUD asked for, waiting to be applied at the top of
    /// the next one.
    ///
    /// **The shell's output is the next frame's input, and that is the rule the
    /// frame's ordering rests on.** A request is laid out from a snapshot and
    /// therefore only exists after that snapshot has been taken; applying it
    /// straight away — which is what this used to do — mutates the world and the
    /// camera *between* the readers of one frame, so the overlay egui had already
    /// laid out was drawn against a camera the world pass no longer had. Held for
    /// a frame instead, every writer runs before the snapshot and there is
    /// nothing left in a frame that can move underneath it.
    ///
    /// The delay is a frame on a button press, which is the same latency every
    /// keyboard and mouse event here already has: they arrive between frames and
    /// land on the next one.
    pub(crate) pending: shell::Request,
    /// What is under the cursor, and what the last click named — see
    /// [`picking::Picking`].
    pub(crate) picking: picking::Picking,
    /// The player's own windows, and what the mouse is doing to them — see
    /// [`windows::Windows`].
    pub(crate) windows: windows::Windows,
    /// The speech line — see [`Chat`].
    pub(crate) chat: Chat,
    /// The last few seconds of the eye, for the scope in the HUD.
    ///
    /// Recorded every frame the camera is advanced, from the same three values
    /// the offline bench records, so the panel's numbers and the table's are one
    /// arithmetic. See [`Scope`].
    pub(crate) scope: Scope,
    /// The last few seconds of the event loop, for the frame panel.
    ///
    /// Recorded every frame that is actually drawn, locked or not: this is a
    /// number about the loop and not about the camera. See [`frames::Frames`]
    /// for why it is not the scope.
    pub(crate) frames: frames::Frames,
    /// How many full atlas repacks this session has paid for — the eviction
    /// `AtlasError::Full` triggers, named in `docs/camera.md`: "costly and
    /// rare" was a claim nothing counted, and each one's cost otherwise reads
    /// as an ordinary heavy frame. See [`Frame::repacked`](frames::Frame) for
    /// which frame paid it.
    pub(crate) repacks: u64,
    /// The flamegraph socket, held open for as long as the client runs.
    ///
    /// Never read after it is built — dropping it is what closes the port, so
    /// holding it *is* the subscription. `None` unless `OPENSHARD_PUFFIN` asked
    /// for one; see [`profile::serve`], and [`profile`]'s docs for why the
    /// flamegraph is a separate viewer rather than a tab in this window.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) _puffin: Option<puffin_http::Server>,
    /// The bench's scenarios, built once.
    ///
    /// Held rather than rebuilt per frame because the HUD lists their names, and
    /// a scenario is a `Vec` of knots: building nine of them to print nine
    /// strings would be a small allocation storm on every frame that draws.
    pub(crate) scripts: Vec<Script>,
    /// The one being walked in the window, while it is.
    pub(crate) replay: Option<replay::Replay>,
}

/// A route snapshot and the world positions that make it valid.
pub(crate) struct RouteCache {
    pub(crate) from: Point,
    pub(crate) goal: Tile,
    pub(crate) route: Option<Arc<Route>>,
}

/// A terrain wash is independent of time; rebuilding it while the camera is
/// still only repeats per-tile walkability queries.
pub(crate) struct TerrainCache {
    pub(crate) camera: Camera,
    pub(crate) from: Point,
    pub(crate) overlay: Arc<TerrainOverlay>,
}

/// The wireframe grid is only a different rendering of the same static
/// occlusion data while its bounds, cutaway and atlas geometry stay unchanged.
pub(crate) struct OccluderCache {
    pub(crate) bounds: TileBounds,
    pub(crate) cutaway: Cutaway,
    pub(crate) atlas_revision: Option<u64>,
    pub(crate) surfaces: Arc<[OccluderSurface]>,
}

impl App {
    /// Real pixels per gump pixel, which is egui's own scale.
    ///
    /// Not the window's scale factor: the interface's art is placed at
    /// coordinates egui laid out in points, so any other number here slides a
    /// window's pictures off whatever egui drew beside them — and the cursor,
    /// which arrives from `winit` in real pixels, has to come back the same way
    /// or a click lands where the picture is not.
    pub(crate) fn gump_scale(&self) -> f32 {
        self.shell
            .as_ref()
            .map(|shell| shell.pixels_per_point())
            .unwrap_or(1.0)
    }

    /// Put the eye back on the body and lock it there.
    ///
    /// Where the body is *drawn* this frame, not the tile it is nominally on:
    /// a relock mid-step would otherwise land up to half a tile from the sprite
    /// and be corrected on the frame after.
    pub(crate) fn relock(&mut self) {
        self.world.presentation.player.drawn = self.world.drawn_player();
        self.control
            .relock(mobiles::gaze(&self.world.presentation.player));
    }

    /// Where our own body is drawn this instant, off the crowd's clock.
    ///
    /// Read rather than stored, and this is the one place that reads it: the
    /// position is a function of a clock and an ease's state, so one read once a
    /// frame is what keeps the sprite, the camera and the scope on the same
    /// number. A crowd that has never heard of us — before a shard names the
    /// body, and for the frame a placeholder is created on — answers with the
    /// tile, which is where a body nobody is easing stands.
    /// Whether there is anybody to show a frame to: the window has the keyboard
    /// and is not covered.
    ///
    /// What the loop's pacing hangs on, and the whole of what this client does
    /// about power. A window in the background still ages its animations — the
    /// crowd has to be where it would have been when the player comes back —
    /// but it does it on the animation clock rather than at the display's rate.
    pub(crate) fn watched(&self) -> bool {
        self.input.focused && !self.input.occluded
    }

    /// What is deciding when the next frame is drawn.
    ///
    /// Watched, it is the display and nothing else: [`App::draw`] asks for the
    /// next frame the moment it has queued one, and `PresentMode::Fifo` blocks
    /// the frame after that until the display has taken it. That is the loop
    /// every other real-time client runs, and it is what makes a still screen
    /// cost the same sixty frames a second as a moving one — which is the point,
    /// because "the frame rate drops when I stand still" was true here and read
    /// as a stall no matter how correct the reason was.
    ///
    /// Unwatched, there is nobody to show a frame to, and the timer below is
    /// what the loop falls back to. Two rates there, because there are two
    /// reasons for a frame and they are an order of magnitude apart: a body's
    /// animation steps once every [`FRAME_DELAY`] and nothing between two of
    /// those changes a pixel, while a *glide* moves a body a couple of pixels at
    /// a time and drawn on the animation clock would arrive in five visible
    /// jumps — the teleport it exists to remove, in instalments. Three reasons
    /// for the fast one and not one, because they are three independent things
    /// that move a pixel: a body mid-step, an eye still converging on one that
    /// has stopped, and a scenario waiting to deliver its next knot.
    ///
    /// The eye is the one that was missing. A rig that filters is still settling
    /// on frames where nothing else moved, and a loop that only woke for gliding
    /// bodies delivered the tail of every ease 80ms late and whole — the stutter
    /// the filter exists to remove, arriving just after it.
    pub(crate) fn pacing(&self) -> frames::Pacing {
        if self.watched() {
            return frames::Pacing::Display;
        }
        frames::Pacing::Timer(self.redraw_interval())
    }

    /// The fallback timer's interval. See [`App::pacing`] for when it is the one
    /// that decides.
    pub(crate) fn redraw_interval(&self) -> std::time::Duration {
        let moving = self.world.presentation.crowd.anyone_gliding()
            || self.control.settling()
            || self.replay.is_some();
        if moving { GLIDE_INTERVAL } else { FRAME_DELAY }
    }

    /// Start walking one of the bench's scenarios in the window.
    ///
    /// Offline only: with a shard connected the body goes where the `0x22` says
    /// it went, and a second writer would be two clients fighting over one
    /// character. The panel does not offer the buttons in that state and this
    /// refuses anyway, because a guard that only lives in a widget is a guard
    /// until somebody adds a keybinding.
    pub(crate) fn start_replay(&mut self, name: &str) {
        if self.world.link.is_some() {
            return;
        }
        let Some(script) = self.scripts.iter().find(|script| script.name == name).cloned() else {
            return;
        };
        // The height the script's own `z = 0` means here. Read once, from the
        // tile it starts on — see `Replay`'s docs on why not per tile.
        let ground = script
            .knots()
            .first()
            .map_or(self.world.presentation.player.at.z, |knot| {
                Self::in_bounds(
                    i32::from(knot.from.x),
                    i32::from(knot.from.y),
                    &self.resources.map,
                )
                .and_then(|tile| self.resources.map.land(tile.x, tile.y))
                .map_or(self.world.presentation.player.at.z, |cell| cell.z)
            });
        let replay = replay::Replay::new(script, ground);
        if let Some(start) = replay.start() {
            // Put down rather than walked, and the camera cut to it: a body
            // that strolled to the start of a scenario would be measured on the
            // way there, and an eye that eased across a facet is a second
            // motion on top of the one being looked at.
            let (body, hue) = (
                self.world.presentation.player.body,
                self.world.presentation.player.hue,
            );
            let equipment = std::mem::take(&mut self.world.presentation.player.equipment);
            let war = self
                .world
                .authoritative
                .view
                .as_ref()
                .is_some_and(|view| view.player.war);
            self.world.presentation.player = self.world.presentation.crowd.snap(
                self.world.me(),
                start,
                body,
                Facing::walking(self.world.presentation.player.facing),
                hue,
                war,
            );
            self.world.presentation.player.equipment = equipment;
            self.world.prediction.set(
                self.world.presentation.player.at,
                Facing::walking(self.world.presentation.player.facing),
            );
            self.world.presentation.cutaway_at = self.world.presentation.player.at;
            self.control
                .relock(mobiles::gaze(&self.world.presentation.player));
        }
        // The frames either side of a start are two different runs, and a metric
        // over both is a number about nothing.
        self.scope.clear();
        self.replay = Some(replay);
    }

    /// One frame of whatever scenario is being walked.
    ///
    /// Every knot the span covered, in order, each handed to the crowd as the
    /// packet it stands for: a crossing is glided and a jump is put down.
    pub(crate) fn advance_replay(&mut self, elapsed: std::time::Duration) {
        let Some(replay) = self.replay.as_mut() else {
            return;
        };
        let moves = replay.advance(elapsed);
        let finished = replay.finished();
        for step in moves {
            let (body, hue) = (
                self.world.presentation.player.body,
                self.world.presentation.player.hue,
            );
            let equipment = std::mem::take(&mut self.world.presentation.player.equipment);
            // The stance the session is actually in: a replay walks this body
            // through a recorded route, and what it is wearing or holding is
            // not part of the recording — so a scenario replayed while at war
            // is drawn at war, exactly as the same walk would be live.
            let war = self
                .world
                .authoritative
                .view
                .as_ref()
                .is_some_and(|view| view.player.war);
            self.world.presentation.player = match step.glided {
                true => {
                    self.world
                        .presentation
                        .crowd
                        .see(self.world.me(), step.to, body, step.facing, hue, war)
                }
                false => {
                    self.world
                        .presentation
                        .crowd
                        .snap(self.world.me(), step.to, body, step.facing, hue, war)
                }
            };
            self.world.presentation.player.equipment = equipment;
            self.world
                .prediction
                .set(self.world.presentation.player.at, step.facing);
            self.world.presentation.cutaway_at = self.world.presentation.player.at;
        }
        if finished {
            self.replay = None;
        }
    }

    /// A viewport that grew may have taken the world texture past what the
    /// device allows, which no zoom step asked for.
    pub(crate) fn fit_zoom_to_device(&mut self) {
        if let Some(refusal) = self.control.fit_to_device() {
            self.report_limit(format_args!(
                "a {}x{} world texture at {} is more than this GPU's {}: zooming in to {}",
                refusal.width, refusal.height, refusal.wanted, refusal.max, refusal.settled,
            ));
        }
    }

    /// One notch of the wheel, answering whether anything changed.
    ///
    /// At either end of the ladder nothing does, and zooming out can be refused
    /// by the device — which is said out loud rather than truncated.
    pub(crate) fn zoom(&mut self, inwards: bool) -> bool {
        match self.control.zoom(inwards) {
            Ok(changed) => changed,
            Err(refusal) => {
                self.report_limit(format_args!(
                    "{} would want a {}x{} world texture and this GPU allows {}: staying at {}",
                    refusal.wanted, refusal.width, refusal.height, refusal.max, refusal.settled,
                ));
                false
            }
        }
    }

    /// Say what the device refused, once.
    ///
    /// Once, because the wheel is held down and a line per notch is a wall of
    /// the same sentence — and because the second one tells nobody anything the
    /// first did not.
    pub(crate) fn report_limit(&mut self, message: std::fmt::Arguments<'_>) {
        if !self.zoom_limit_reported {
            self.zoom_limit_reported = true;
            eprintln!("{message}");
        }
    }
}
