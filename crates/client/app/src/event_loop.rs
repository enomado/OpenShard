//! `winit` glue: the `ApplicationHandler` impl that dispatches a platform
//! event into whichever subsystem answers to it — a step to `ui_command.rs`,
//! a packet to `net_command.rs`, a press on a window to `own_windows.rs`, a
//! redraw to `presentation.rs`. Nothing here decides gameplay on its own; it
//! is the seam between what winit reports and the method that already knows
//! what to do with it.

use std::time::Instant;

use openshard_client_render::camera::RealPixel;
use openshard_client_render::gump::GumpPixel;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::WindowId;

use crate::app::App;
use crate::picking::SelectedIdentity;
use crate::presentation::frame_dump_root;
use crate::world::{cluttered, cluttered_with_doors_open};
use crate::{DOUBLE_CLICK, GLIDE_INTERVAL, PAGE_PIXELS, desk, keys, link, shell, steer};

impl ApplicationHandler<link::Update> for App {
    /// The shard thread had something to say.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, update: link::Update) {
        // The crowd's clock first, and before the packet is folded in. A step is
        // timestamped with `Crowd`'s own `now` — that is what the *next* step's
        // crossing is measured against (`crowd::glide_time`) — and this handler
        // used to fold packets in between two `advance` calls, so every step was
        // recorded at the previous frame's instant: up to 16ms in the past
        // mid-walk and up to a whole `FRAME_DELAY` for a body that had stopped.
        // The measurement is a difference of two of those, so the error lands on
        // the crossing *length*: the walk oracle in `dst.rs` caught a tile after
        // a turn taking 416ms instead of 400, which is a body a frame behind
        // itself and then yanked forward.
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
            // The window stays open: whatever is on screen is still the last
            // thing the server said, and closing it would take the reason with
            // it. The map viewer is what is left, which is a fair description
            // of a client that has lost its shard.
            link::Update::Lost(reason) => {
                eprintln!("disconnected: {reason}");
                self.world.link = None;
                return;
            }
        }
        // A step that arrives while nobody was moving finds the animation clock
        // armed for the *standing* rate, up to a whole `FRAME_DELAY` away — so
        // the first 80ms of the glide would be drawn frozen at its start, once
        // per tile. Pulling the tick forward is what makes a walk continuous
        // from its first frame; it is a `min` rather than an assignment because
        // a clock already running at the glide rate is the earlier of the two.
        let soon = now + GLIDE_INTERVAL;
        if self.world.crowd.anyone_gliding() && self.next_tick > soon {
            self.next_tick = soon;
        }
        if let Some(window) = self.window.as_ref() {
            window.window.request_redraw();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        match self.create_window(event_loop) {
            Ok(window) => self.window = Some(window),
            Err(error) => {
                eprintln!("{error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // The UI sees everything first, and what it takes reaches neither the
        // camera nor the walk keys — otherwise a drag inside a panel pans the
        // world underneath it. egui never claims a close or a resize, so
        // returning here cannot swallow one.
        let consumed = match (self.shell.as_mut(), self.window.as_ref()) {
            (Some(shell), Some(screen)) => shell.on_window_event(&screen.window, &event),
            _ => false,
        };
        if consumed {
            // A key the UI took is a key this will never hear come up, and a
            // held direction that is never released walks for ever. Typing into
            // a panel should stop the character anyway, so letting go of
            // everything is both the fix and the behaviour.
            if matches!(event, WindowEvent::KeyboardInput { .. }) {
                self.steer.clear();
                self.input.aiming = false;
            }
            if let Some(window) = self.window.as_ref() {
                window.window.request_redraw();
            }
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(window) = self.window.as_mut() {
                    window.config.width = size.width.max(1);
                    window.config.height = size.height.max(1);
                    window.surface.configure(&window.device, &window.config);
                    self.control.resize(window.config.width, window.config.height);
                    // The world texture and the depth buffer follow the
                    // *camera's* size and not the window's, which are the same
                    // thing only at zoom 1. `draw` resizes them together.
                    window.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                // The speech line, ahead of every hotkey and every walk key:
                // once it has the keyboard, a letter is a letter typed and not
                // a body walked. Arrows move the caret here rather than the
                // player, which is why this comes before `direction_of` below
                // and not after it, and Escape closes the line rather than
                // reaching the `KeyCode::Escape` arm further down that quits
                // the client.
                if self.chat.focused {
                    if event.state == ElementState::Pressed {
                        match code {
                            KeyCode::Enter | KeyCode::NumpadEnter => {
                                if let Some(line) = self.chat.take() {
                                    self.say(line);
                                }
                            }
                            KeyCode::Escape => {
                                self.chat.typed.clear();
                                self.chat.cursor = 0;
                                self.chat.focused = false;
                            }
                            KeyCode::Backspace => self.chat.backspace(),
                            KeyCode::Delete => self.chat.delete(),
                            KeyCode::ArrowLeft => self.chat.left(),
                            KeyCode::ArrowRight => self.chat.right(),
                            KeyCode::Home => self.chat.cursor = 0,
                            KeyCode::End => self.chat.cursor = self.chat.typed.len(),
                            _ => {
                                if let Some(text) = event.text.as_deref() {
                                    self.chat.insert(text);
                                }
                            }
                        }
                        if let Some(window) = self.window.as_ref() {
                            window.window.request_redraw();
                        }
                    }
                    return;
                }
                // A `{ textentry }` the player has clicked into, on the same
                // terms and ahead of the same keys: while a field has the
                // keyboard, a letter is a letter typed. Below the speech line
                // rather than above it because the two cannot both be focused —
                // Enter opens one and a click opens the other — and the order
                // only decides which is asked first.
                //
                // Escape gives the keyboard back rather than quitting the
                // client, which is what the arm further down would do.
                if self.windows.dialogs.typing() {
                    if event.state == ElementState::Pressed {
                        match code {
                            KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter => {
                                self.windows.dialogs.unfocus();
                            }
                            KeyCode::Backspace => {
                                self.windows.dialogs.backspace();
                            }
                            _ => {
                                if let Some(text) = event.text.as_deref() {
                                    self.windows.dialogs.typed(text);
                                }
                            }
                        }
                        if let Some(window) = self.window.as_ref() {
                            window.window.request_redraw();
                        }
                    }
                    return;
                }
                // An arrow is *held*, not pressed: while it is down a step is
                // due every step's length, and that clock is ours rather than
                // the operating system's repeat rate. See `keys.rs`.
                if let Some(direction) = keys::Held::direction_of(code) {
                    let step = match event.state {
                        ElementState::Pressed => {
                            let opened = cluttered_with_doors_open(&self.world, &self.resources);
                            let cluttered = cluttered(&self.world, &self.resources);
                            self.steer.press(
                                direction,
                                self.world.player.at,
                                Instant::now(),
                                self.world.player.facing,
                                steer::Ground {
                                    real: &cluttered,
                                    through_doors: &opened,
                                },
                            )
                        }
                        ElementState::Released => {
                            self.steer.release(direction);
                            None
                        }
                    };
                    if let Some(facing) = step {
                        if self.walk(facing) {
                            if let Some(window) = self.window.as_ref() {
                                window.window.request_redraw();
                            }
                        }
                    }
                    return;
                }
                if event.state != ElementState::Pressed {
                    return;
                }
                // Escape takes the topmost window down and does *not* quit the
                // client. Quitting on it was this client's own invention — no
                // reference client does it — and it cost more than a keystroke:
                // a window whose right button is eaten by a panel over it (see
                // [`App::close_top_window`]) had no way out at all, because the
                // one key a player reaches for closed the whole session
                // instead. Leaving is `CloseRequested`, which is the window
                // manager's close box and every other application's answer.
                if code == KeyCode::Escape {
                    if self.close_top_window() {
                        if let Some(window) = self.window.as_ref() {
                            window.window.request_redraw();
                        }
                    }
                    return;
                }
                let changed = match code {
                    // The dev window, the same switch the status strip's `dev`
                    // toggle is. Two ways in rather than one because the state is
                    // remembered: a window closed once stays closed across
                    // launches, and the strip that reopens it is itself a thing
                    // you have to know is there. A key is what you reach for
                    // without knowing anything.
                    //
                    // F1 and not a letter: a letter reaching here means the
                    // chat line was not focused, and one that walked the body
                    // instead of opening it would be the bug — see the
                    // `self.chat.focused` branch above.
                    KeyCode::F1 => {
                        // The shell's `Desk` and not this one: see
                        // `Shell::toggle_dev`. Before there is a shell there is no
                        // window either, so this arm cannot be reached without one.
                        if let Some(shell) = self.shell.as_mut() {
                            shell.toggle_dev();
                        }
                        true
                    }
                    // Opens the speech line — the reference client's own
                    // gesture, and unbound until now: movement is arrows only
                    // (`keys::Held::direction_of`), so no key is taken from it.
                    // `steer.clear()` for the same reason the UI-consumed path
                    // above does: an arrow held down when this fires would
                    // otherwise never see its `Released` and walk forever,
                    // since arrows move the caret and not the body once
                    // `chat.focused` is true.
                    KeyCode::Enter | KeyCode::NumpadEnter => {
                        self.chat.focused = true;
                        self.steer.clear();
                        true
                    }
                    KeyCode::Home => {
                        self.relock();
                        true
                    }
                    // Page up and down lift the eye rather than the body,
                    // which is a pan: the map has no vertical axis to walk
                    // along, only a projection that folds `z` into `y`.
                    KeyCode::PageUp => self.control.pan(0, PAGE_PIXELS),
                    KeyCode::PageDown => self.control.pan(0, -PAGE_PIXELS),
                    // A diagnostic, not a feature: a fixed mixed-case ASCII
                    // line, sent without ever going through the keyboard —
                    // no xkb group, no IME, no `shell`'s `TextEdit`. Whatever
                    // shows up over the head from this key is exactly what
                    // `0xAD` → `0xAE` → `text::collect` do with known-good
                    // bytes, with typing entirely ruled out as a variable.
                    KeyCode::F9 => {
                        self.say("AbCdEfGh The Quick Brown Fox 123".to_owned());
                        false
                    }
                    // Night on and off. A key and not a setting because the
                    // only honest test of firelight is the two pictures side
                    // by side, and there is no time of day on the wire yet for
                    // it to follow — see `App::night`.
                    KeyCode::F10 => {
                        self.graphics.night = !self.graphics.night;
                        true
                    }
                    // The lighting's own values, one after another — see
                    // `crate::debug::View`. A key rather than a setting for the
                    // same reason F10 is one: what is being looked for is a
                    // difference between two pictures of the same instant, and
                    // anything that needed a restart would put a different world
                    // on either side of the comparison.
                    // The sun on and off, for the same reason F10 exists: the
                    // only honest test of a shadow is the two pictures of the
                    // same instant, one with it and one without.
                    KeyCode::F8 => {
                        self.graphics.sunlit = !self.graphics.sunlit;
                        true
                    }
                    // The sky field on and off — what a roof does to the light
                    // under it, against a flat ambient. A key for the third time
                    // for the same reason: the two pictures of one instant are the
                    // only way to see which of the two terms a dark room came from.
                    KeyCode::F6 => {
                        self.graphics.sky_field = !self.graphics.sky_field;
                        true
                    }
                    // The torch in the player's own hand, on and off — the same
                    // two-pictures-of-one-instant reason F10 and F8 are keys,
                    // and here it is also the only way to see what the map's own
                    // fires are doing without a beam swinging across them.
                    KeyCode::F7 => {
                        self.graphics.lantern = !self.graphics.lantern;
                        true
                    }
                    // The occlusion grid as solids — step 23.0. A key beside the
                    // checkbox for the reason F10 and F8 are keys: what is being
                    // read is the difference between two pictures of one
                    // instant, here the world with the geometry drawn over it
                    // and the world without, and a hand that has to find a
                    // checkbox has moved the camera by the time it is back.
                    KeyCode::F5 => {
                        self.graphics.show_solids = !self.graphics.show_solids;
                        true
                    }
                    // The world image, off underneath the solids — the opposite
                    // reading from F5's own (decision 39.2 draws the sprite on
                    // purpose, so the box can be checked against it): this is for
                    // looking at the box itself, with nothing behind it arguing
                    // about what shape it is.
                    KeyCode::F3 => {
                        self.graphics.solids_only = !self.graphics.solids_only;
                        true
                    }
                    // And how much of the grid either view draws — the second
                    // datum, and a key for the same reason F5 is one, only more
                    // so: the question it answers is "is that floor missing, or
                    // is it under my feet", and the two pictures that answer it
                    // differ in nothing but this.
                    KeyCode::F4 => {
                        self.graphics.solids_everything = !self.graphics.solids_everything;
                        true
                    }
                    KeyCode::F11 => {
                        self.graphics.light_view = self.graphics.light_view.next();
                        tracing::info!(view = self.graphics.light_view.name(), "lighting view");
                        true
                    }
                    // What a fragment whose ray met no box is answered with, one
                    // after another — `impostor::Fringe`, and a key for exactly
                    // the reason F10 and F11 are keys. Two of its three states
                    // were refused on measurements
                    // (`docs/lighting_state.md`'s fringe entry) and the
                    // instrument that refusal is *supposed* to answer to is a
                    // person looking at two pictures of one instant, which is
                    // what a key gives and a setting read at start-up does not.
                    KeyCode::F2 => {
                        self.graphics.fringe = self.graphics.fringe.next();
                        // **The state on a line of its own**, which is the whole
                        // difference between a knob and a knob that looks
                        // broken: `discard` cuts a stripe out of every course of
                        // every roof, so a person who sees no change at all is
                        // looking at a frame this key never reached, and the log
                        // is what tells those two apart.
                        //
                        // It does *not* need the lights on. Daylight builds an
                        // empty grid, not an absent one, and
                        // `statics::push_volumes` calls `boxes_of` regardless —
                        // so a sprite is met against its own per-tile box either
                        // way, and only the box's *name* (`SolidId::NOBODY`) and
                        // its merging are what a grid adds. Measured rather than
                        // reasoned: `isolated_scene` with `_IMPOSTOR=0` still
                        // moves 9.7% of the frame's pixels between two of these
                        // states.
                        tracing::info!(fringe = self.graphics.fringe.name(), "fringe");
                        true
                    }
                    // This frame, written out: every plane the blit can draw of
                    // it, plus the inputs it was assembled from. See
                    // [`App::frame_dump`] for why the client having none was the
                    // thing standing in front of `docs/parity.md`'s gate, and
                    // [`frame_dump_root`] for where it lands.
                    //
                    // A key and not a setting, for the reason F5, F8 and F10 are
                    // keys and more so: what is being dumped is the instant a
                    // person is looking at, and anything that had to be switched
                    // on beforehand would dump a different one.
                    KeyCode::F12 => {
                        self.graphics.frame_dump =
                            Some(frame_dump_root().join(format!("frame-{}", self.graphics.frame_dumps)));
                        self.graphics.frame_dumps += 1;
                        true
                    }
                    _ => false,
                };
                if changed {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            // Shift is the whole of "run", and it arrives here rather than as a
            // key: `ModifiersChanged` is what winit reports a held modifier
            // with, and a `KeyboardInput` for the shift itself would miss the
            // case of it going down between two steps.
            WindowEvent::ModifiersChanged(modifiers) => {
                self.steer.set_running(modifiers.state().shift_key());
                // Toggling Ctrl mid-drag switches the right-hold from a
                // heading to a move order (or back) on the next cursor move —
                // no special-casing needed, `walk_toward_cursor` reads this
                // fresh every call.
                self.input.ctrl_held = modifiers.state().control_key();
            }
            // A window that loses focus never hears the key come up, and a
            // character that keeps walking into a wall while its player is in
            // another window is not what the key meant. The destination goes
            // with it, for the same reason: nobody is watching it be walked to.
            //
            // It is also half of what paces the loop — see [`App::watched`] —
            // and regaining focus has to ask for a frame, because the redraw
            // that would have asked for the next one is the one that stopped
            // being drawn.
            WindowEvent::Focused(focused) => {
                self.input.focused = focused;
                if focused {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                } else {
                    self.steer.clear();
                    self.input.aiming = false;
                }
            }
            // Entirely covered by another window: the compositor will not show
            // anything drawn, so the loop stops drawing at the display's rate
            // and falls back to the animation clock. Uncovered, it restarts the
            // same way focus does.
            WindowEvent::Occluded(occluded) => {
                self.input.occluded = occluded;
                if !occluded {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            // A cursor that has left says so once and then goes quiet, so the
            // flag is what stands in for the positions that stop arriving. It
            // reaches here even when egui consumed the move that preceded it:
            // `on_window_event` does not claim these.
            WindowEvent::CursorEntered { .. } => {
                self.input.pointer_inside = true;
            }
            WindowEvent::CursorLeft { .. } => {
                self.input.pointer_inside = false;
                if let Some(window) = self.window.as_ref() {
                    window.window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input.pointer_inside = true;
                // Relative to the *viewport* and not the window: the camera's
                // own centre is the viewport's, so a cursor measured from the
                // window would zoom about a point half a panel away.
                let origin = self.shell.as_ref().map_or((0, 0), |shell| {
                    (shell.viewport().x as i32, shell.viewport().y as i32)
                });
                let cursor = RealPixel::new(position.x as i32 - origin.0, position.y as i32 - origin.1);
                // The interface's cursor is measured from the surface's own
                // corner and in gump pixels, which is what everything drawn by
                // the gump pass is placed in.
                let scale = self.gump_scale();
                self.input.pointer_gump = GumpPixel::new(
                    (position.x as f32 / scale) as i32,
                    (position.y as f32 / scale) as i32,
                );
                let mut changed = self.control.cursor_moved(cursor);
                changed |= self.drag_own_window();
                changed |= self.drag_thumb();
                // Held, the button steers: a heading toward wherever the cursor
                // is, by default, or a Ctrl-held move order — see
                // `walk_toward_cursor` and `steer.rs`'s module docs for why
                // those are two different things and not one idiom stated
                // twice.
                if self.input.aiming {
                    changed |= self.walk_toward_cursor();
                }
                if changed {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == winit::event::MouseButton::Middle {
                    self.control.set_panning(state == ElementState::Pressed);
                }
                // A left click selects the tile under the cursor for the Tile
                // panel — reached here and not through egui, because `consumed`
                // above already sent every click the UI wanted to it.
                if button == winit::event::MouseButton::Left && state == ElementState::Released {
                    self.windows.dragging = None;
                    // And a button that was pressed on a dialog is answered on
                    // the way up, if the pointer is still on it — see
                    // `gump::Dialogs::release`.
                    if self.release_on_own_window() {
                        if let Some(window) = self.window.as_ref() {
                            window.window.request_redraw();
                        }
                    }
                }
                // A container window takes the press before the world sees it,
                // the same way a panel does: the click that raises a bag must
                // not also select the tile behind it, and it must not start a
                // double-click pair that would use whatever is under there.
                if button == winit::event::MouseButton::Left
                    && state == ElementState::Pressed
                    && self.press_on_own_window()
                {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                } else if button == winit::event::MouseButton::Left && state == ElementState::Pressed {
                    // The camera as it stands, which between two frames is the
                    // one the last frame was drawn with — the picture the player
                    // is clicking on.
                    let camera = *self.control.camera();
                    // What the last frame found under the cursor — by identity,
                    // and in the same order the hover pick already answered
                    // "what is the cursor on" in: a creature first, then an item,
                    // then the map's own furniture, then bare ground. `None` all
                    // the way down is how a selection is put out: there is
                    // nothing to select where nothing is standing.
                    //
                    // **The tile a bare click names is the ground under the
                    // cursor; everything else's is its own.** Two different
                    // arithmetics, on purpose: a wall's picture stands up the
                    // screen from the cell it is built on, so the ground *under
                    // the cursor* is the cell behind the wall — two tiles behind
                    // it, for a wall of ordinary height. Selecting a wall and
                    // marking that other tile is the client saying "this one"
                    // about two places at once, which is what this arm used to
                    // do; a mobile or an item stands exactly where it says it
                    // does, so no such correction applies to either.
                    self.picking.selected = match (
                        self.picking.on_mobile,
                        self.picking.on_item,
                        self.picking.on_static,
                    ) {
                        (Some(who), _, _) => Some(SelectedIdentity::Mobile(who)),
                        (None, Some(serial), _) => Some(SelectedIdentity::Item(serial)),
                        (None, None, Some(picked)) => Some(SelectedIdentity::Static(picked)),
                        (None, None, None) => self.pick_tile(camera).map(|tile| SelectedIdentity::Tile {
                            x: tile.at.x,
                            y: tile.at.y,
                        }),
                    };
                    // **In war mode, a single click on a body is an attack.**
                    // The reference client's own gesture, and the reason the
                    // stance exists at all: at peace the same click selects and
                    // nothing more. Beside the selection rather than instead of
                    // it — the Tile panel still shows what was clicked, and a
                    // click that both selects and aims is one click with two
                    // readers, not two gestures fighting over one button.
                    //
                    // An aim and nothing else goes out; the shard's `swings`
                    // strikes on its own timer and answers with a `0xAA`. See
                    // `openshard_client_net::combat`.
                    self.attack_under_cursor();
                    // And the second click of a pair is a *use*: a door opens, a
                    // container opens, food is eaten. Which of those it is, is
                    // the shard's answer and not this end's — see
                    // `openshard_client_net::interact`.
                    let now = Instant::now();
                    let paired = self
                        .input
                        .last_click
                        .is_some_and(|last| now.duration_since(last) <= DOUBLE_CLICK);
                    // Cleared on a pair rather than restarted, so a third click
                    // starts a fresh one — ClassicUO's own reset.
                    self.input.last_click = (!paired).then_some(now);
                    if paired {
                        self.use_under_cursor(camera);
                    }
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
                // A right hold is a heading toward the cursor by default, or a
                // Ctrl-held move order — either way it stays under way while
                // the button is, driven from `CursorMoved`. Left is spoken for
                // by the Tile panel above, and the middle button pans.
                // Right over a window closes it — the reference client's own
                // gesture — and does not steer: a press that never reached the
                // world cannot be a heading into it.
                if button == winit::event::MouseButton::Right
                    && state == ElementState::Pressed
                    && self.close_window_under_pointer()
                {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                } else if button == winit::event::MouseButton::Right {
                    self.input.aiming = state == ElementState::Pressed;
                    if self.input.aiming {
                        if self.walk_toward_cursor() {
                            if let Some(window) = self.window.as_ref() {
                                window.window.request_redraw();
                            }
                        }
                    } else {
                        // A heading stops the instant the button does — unlike
                        // a move order, which keeps walking itself there after
                        // the button that gave it is gone. `mouse_up` only
                        // touches the heading; a Ctrl-held destination in
                        // flight is untouched.
                        self.steer.mouse_up();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // A notch is a line on a wheel and a fraction of one on a
                // touchpad, and only the sign is asked for here: the ladder is
                // what decides how far a notch goes.
                let notches = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(position) => position.y as f32,
                };
                // A list under the pointer takes the notch before the camera
                // does: rolling a wheel over a window is scrolling that window,
                // in this client as in every other.
                if notches != 0.0 && (self.scroll_skills(notches) || self.zoom(notches > 0.0)) {
                    if let Some(window) = self.window.as_ref() {
                        window.window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    /// Re-arm the animation clock and ask for a redraw when it has advanced.
    ///
    /// `winit`'s idiomatic timer: `ControlFlow::WaitUntil` sleeps the event
    /// loop rather than spinning it, and returning here every
    /// [`App::redraw_interval`] is what stands in for a real client's own
    /// `Mobile.ProcessAnimation` poll.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // A held arrow — or a tile the mouse sent the body to — asks for a step
        // every step's length. Here and not in the input event: the operating
        // system repeats a held key at a rate that is not a walking speed, a
        // mouse held over the ground reports a move a pixel, and the fast half
        // of either is refused by the shard as a speedhack — which reads as the
        // walk stuttering. See `steer.rs`.
        //
        // Twice at most, because a turn is a step that covers no ground and
        // costs no time against the shard's pace budget: the step it precedes is
        // due the same instant, and holding that back to the next wake would put
        // a frame of standing still exactly where the player asked for movement.
        // Two and not a loop — the second ask is the step the turn was for, and
        // anything past it is a rate, which is what the clock is for.
        let mut moved = false;
        for _ in 0..2 {
            // Both halves of the ground, because this is where a destination
            // replans: see `steer::Ground`. Built here rather than held, for the
            // reason the single terrain always was — they borrow the map and the
            // view, and the walk borrows `steer` mutably beside them.
            let opened = cluttered_with_doors_open(&self.world, &self.resources);
            let cluttered = cluttered(&self.world, &self.resources);
            let ground = steer::Ground {
                real: &cluttered,
                through_doors: &opened,
            };
            let Some(facing) = self
                .steer
                .due(now, self.world.player.at, self.world.player.facing, ground)
            else {
                break;
            };
            moved |= self.walk(facing);
        }
        if moved {
            if let Some(window) = self.window.as_ref() {
                window.window.request_redraw();
            }
        }
        // The animation clock. Watched, this is a safety net rather than the
        // pacer — `draw` asks for the next frame itself and the display answers
        // — and it is kept for the paths where that ask does not happen: `draw`
        // returns early with no window, with a swapchain it had to rebuild, and
        // on a compositor that refused to hand over a texture. Without it, one
        // of those would stop the loop dead until the next input event. The
        // redraw requests coalesce, so a net that fires while the display is
        // already pacing costs a wake and no frame.
        if now >= self.next_tick {
            self.next_tick = now + self.redraw_interval();
            if let Some(window) = self.window.as_ref() {
                window.window.request_redraw();
            }
        }
        // Three reasons to come back, so three terms: the animation clock,
        // whatever the UI is animating, and the next step a held key is owed.
        // The deadline is the earliest — a loop that slept past the step would
        // walk at whatever rate it happened to wake at.
        // `checked_add`, because a still UI asks for eternity
        // (`Duration::MAX`, see `Shell::repaint_after`) and `now + MAX`
        // overflows the instant rather than meaning "never". An overflow is
        // exactly the case where the UI wants no frame of its own, so it falls
        // back to the animation clock.
        let deadline = match self.shell.as_ref().map(shell::Shell::repaint_after) {
            Some(after) => match now.checked_add(after) {
                Some(ui) => self.next_tick.min(ui),
                None => self.next_tick,
            },
            None => self.next_tick,
        };
        let deadline = match self.steer.deadline() {
            Some(step) => deadline.min(step),
            None => deadline,
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }

    /// The loop is over: write down what the HUD looked like.
    ///
    /// Here and not on `CloseRequested`, because that is one of several ways out
    /// — `event_loop.exit()` is also called from a startup failure and from the
    /// link — and this is the one place all of them pass through. A client that
    /// is killed writes nothing, which is the honest behaviour: the file says
    /// where things were when the client was last *closed*.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // The HUD's half — tab, panel, scale — and then the platform's, which
        // only the window itself can answer for.
        if let Some(shell) = self.shell.as_ref() {
            self.desk = shell.desk();
        }
        if let Some(screen) = self.window.as_ref() {
            let size = screen.window.inner_size();
            // A window whose position the platform will not report — Wayland
            // does not, by design — keeps whatever the file already said rather
            // than being moved to the origin. Half a frame restored is better
            // than a window that walks to the top-left corner every launch.
            let position = screen.window.outer_position().ok();
            let previous = self.desk.window;
            self.desk.window = Some(desk::Frame {
                x: position.map_or_else(|| previous.map_or(0, |frame| frame.x), |at| at.x),
                y: position.map_or_else(|| previous.map_or(0, |frame| frame.y), |at| at.y),
                width: size.width.max(1),
                height: size.height.max(1),
                maximized: screen.window.is_maximized(),
            });
        }
        if let Err(error) = self.desk.save(std::path::Path::new(desk::PATH)) {
            eprintln!("{error}");
        }
    }
}
