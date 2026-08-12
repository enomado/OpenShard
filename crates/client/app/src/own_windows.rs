//! The player's own gump, doll and skill windows: what the mouse is doing to
//! them, kept apart from `ui_command.rs`'s walk and targeting even though
//! both answer to the same click — a press on a window and a press on the
//! ground are different subsystems that happen to share an input device.
//!
//! [`App::sync_own_windows`] is the once-a-frame fold from the
//! [`WorldView`](openshard_client_net::view::WorldView) the shard has sent;
//! everything below it answers a press, a drag or a release against
//! whatever that fold last laid out — see [`windows::Windows::drawn_windows`]
//! for why the picture a click is tested against is the *last frame's*.

use openshard_client_render::gump::{self as gump_art, GumpPixel};
use openshard_protocol::gump::GumpId;

use crate::app::App;
use crate::windows::{Drawn, WindowSubject};
use crate::{gump, link};

mod paperdoll;
mod skills;
mod sync;

impl App {
    /// Which window the cursor is over, topmost first, or `None`.
    ///
    /// Against **every picture the window drew**, and each against its own
    /// opaque texels rather than a bounding box: a bag's art has transparent
    /// corners, a paperdoll's frame has a large transparent middle, and a click
    /// in either belongs to whatever is behind it — which is usually the world.
    /// A hat that the doll wears past the edge of its frame is the window's, and
    /// a hole in the frame's own corner is not: both fall out of asking the
    /// list, and neither did when this asked the background alone.
    ///
    /// The list is the last frame's — see [`windows::Windows::drawn_windows`] for why it is
    /// remembered rather than laid out again here — and the z-order is
    /// [`windows::Windows::own_windows`]'s, which is current: raising a window on the press
    /// must not wait for a frame.
    pub(crate) fn window_under_pointer(&self) -> Option<WindowSubject> {
        let cursor = self.input.pointer_gump;
        self.windows.own_windows.iter().rev().find_map(|window| {
            let drawn = self.drawn(window.subject)?;
            // A dialog's fields are the one part of a window that is a box
            // rather than a picture — see `gump::Field` — and a click in one is
            // still a click on the window. It sits over the background, which is
            // a picture, so this only matters for a field the layout hung
            // outside its own frame; asking is cheaper than being wrong there.
            if let Drawn::Dialog(laid_out) = drawn {
                if gump_art::field(&laid_out.fields, cursor).is_some() {
                    return Some(window.subject);
                }
            }
            gump_art::pick(drawn.pictures(), cursor, &self.resources.gump_atlas).map(|_| window.subject)
        })
    }

    /// What the last frame drew for one window, or `None` for a window that has
    /// not been drawn yet — every window on the frame its packet arrived.
    pub(crate) fn drawn(&self, subject: WindowSubject) -> Option<&Drawn> {
        self.windows
            .drawn_windows
            .iter()
            .find(|(drawn, _)| *drawn == subject)
            .map(|(_, drawn)| drawn)
    }

    /// The dialog a subject names, out of the view, or `None` if the shard has
    /// taken it away since.
    pub(crate) fn open_gump(&self, gump_id: GumpId) -> Option<&openshard_client_net::view::OpenGump> {
        self.world
            .authoritative
            .view
            .as_ref()?
            .gumps
            .iter()
            .find(|gump| gump.gump_id == gump_id)
    }

    /// Raise a window to the top of the pile, so that the one just clicked is
    /// the one drawn over the others.
    pub(crate) fn raise_window(&mut self, subject: WindowSubject) {
        if let Some(index) = self
            .windows
            .own_windows
            .iter()
            .position(|window| window.subject == subject)
        {
            let window = self.windows.own_windows.remove(index);
            self.windows.own_windows.push(window);
        }
    }

    /// A left press over one of this client's windows: raise it and take hold
    /// of it.
    ///
    /// Answers whether the press belonged to a window, so the caller can leave
    /// the world's own click alone when it did — a press that raised a bag must
    /// not also select the tile behind it.
    ///
    /// A dialog's own widgets take the press first, and take it away from the
    /// drag: pressing a button must not also start moving the window under it.
    /// Everything else in a dialog — its background, a `{ gumppic }`, a label —
    /// drags it, which is how a gump is moved when it has no title bar to move
    /// it by. See `gump::Dialogs::press`.
    pub(crate) fn press_on_own_window(&mut self) -> bool {
        let Some(subject) = self.window_under_pointer() else {
            // A press that missed every window gives the keyboard back: a field
            // stays focused only while the player is still in the dialog.
            self.windows.dialogs.unfocus();
            return false;
        };
        self.raise_window(subject);
        if let WindowSubject::Dialog(gump_id) = subject {
            // Both halves of the question are last frame's: the window the
            // pointer is over and the layout it was drawn as. Laying the dialog
            // out again here would ask the atlas and the view a second time and
            // could answer differently from what is on the screen — the rule
            // `drawn_windows` exists for.
            let taken = match (self.open_gump(gump_id), self.drawn(subject)) {
                (Some(gump), Some(Drawn::Dialog(window))) => {
                    // Cloned because `press` needs the dialogs mutably and the
                    // window is borrowed out of `self`. A laid-out window is a
                    // few hundred bytes and this happens once per click.
                    let window = window.clone();
                    let cursor = self.input.pointer_gump;
                    let gump = gump.clone();
                    self.windows
                        .dialogs
                        .press(&gump, &window, cursor, &self.resources.gump_atlas)
                }
                _ => false,
            };
            if taken {
                self.windows.dragging = None;
                return true;
            }
            // `{ nomove }`: the press is still the window's — it must not reach
            // the world behind it — but it does not pick the window up. A shard
            // that pins a dialog somewhere means it.
            if self
                .open_gump(gump_id)
                .is_some_and(|gump| gump::flags(gump).no_move)
            {
                self.windows.dragging = None;
                return true;
            }
        }
        // A paperdoll's own furniture, which is the same gesture a dialog's
        // buttons have and none of the machinery: there is no layout to consult,
        // only the list this window drew and the `hits` beside it. Taking the
        // press away from the drag is the point — the column of buttons runs
        // down the middle of the frame, and pressing one used to pick the whole
        // doll up.
        if let WindowSubject::Paperdoll(_) = subject {
            if let Some(button) = self.doll_button_under_pointer(subject) {
                self.windows.held_doll = Some((subject, button));
                self.windows.dragging = None;
                return true;
            }
        }
        // The skill window's own furniture: a heading's arrow, the two ends of
        // the bar, the track and the thumb. The same gesture again, and the same
        // reason for taking the press away from the drag — the bar runs down the
        // inside of the scroll, and a thumb that also picked the window up would
        // move both at once.
        if subject == WindowSubject::Skills {
            if let Some(hit) = self.skill_hit_under_pointer() {
                self.windows.held_skill = Some(hit);
                self.windows.dragging = None;
                return true;
            }
        }
        let grab = self
            .windows
            .own_windows
            .last()
            .map(|window| {
                GumpPixel::new(
                    self.input.pointer_gump.x - window.at.x,
                    self.input.pointer_gump.y - window.at.y,
                )
            })
            .unwrap_or_default();
        self.windows.dragging = Some((subject, grab));
        true
    }

    /// The release that finishes a press on a dialog's button or a paperdoll's,
    /// and whatever it sent.
    ///
    /// Answers whether anything happened, so the caller can ask for a redraw:
    /// the button comes back up on the way out either way, and a page button
    /// changes what the window is showing without a packet leaving.
    pub(crate) fn release_on_own_window(&mut self) -> bool {
        if let Some(hit) = self.windows.held_skill.take() {
            // The same "still on the same picture" rule the doll's buttons
            // follow. The thumb is the exception that needs no arm: it has
            // already done its work, on every mouse move since the press.
            if self.skill_hit_under_pointer() == Some(hit) {
                self.skill_clicked(hit);
            }
            return true;
        }
        if let Some((subject, button)) = self.windows.held_doll.take() {
            // Only if the pointer is still on the same button. A press that
            // slid off one is not a click on it — the reference's own rule for
            // every control it draws — and it is not a click on whatever the
            // finger landed on either.
            if self.doll_button_under_pointer(subject) == Some(button) {
                self.doll_clicked(subject, button);
            }
            // True whatever it landed on: the button was drawn pressed and has
            // to come back up.
            return true;
        }
        let Some(gump_id) = self.windows.dialogs.holding() else {
            return false;
        };
        let subject = WindowSubject::Dialog(gump_id);
        let (Some(gump), Some(Drawn::Dialog(window))) = (self.open_gump(gump_id), self.drawn(subject)) else {
            return false;
        };
        let window = window.clone();
        let gump = gump.clone();
        let cursor = self.input.pointer_gump;
        let reply = self
            .windows
            .dialogs
            .release(&gump, &window, cursor, &self.resources.gump_atlas);
        if let Some(reply) = reply {
            // A reply takes the window down with it: the shard sends one `0xB0`
            // and waits for one `0xB1`, and nothing ever arrives to say the
            // dialog is gone. `answer_gump` is what tells the view.
            self.answer_gump(reply);
            self.windows
                .own_windows
                .retain(|window| window.subject != subject);
        }
        true
    }

    /// Move the window being dragged so that the point the player grabbed stays
    /// under the cursor. Answers whether anything moved.
    pub(crate) fn drag_own_window(&mut self) -> bool {
        let Some((subject, grab)) = self.windows.dragging else {
            return false;
        };
        let at = GumpPixel::new(
            self.input.pointer_gump.x - grab.x,
            self.input.pointer_gump.y - grab.y,
        );
        let Some(window) = self
            .windows
            .own_windows
            .iter_mut()
            .find(|window| window.subject == subject)
        else {
            return false;
        };
        let moved = window.at != at;
        window.at = at;
        moved
    }

    /// Close the window under the cursor, if there is one.
    ///
    /// The right button, which is what the reference client closes a gump with,
    /// and it is *not* a conflict with the right-hold that steers: a press over
    /// a window never reaches the world, the same way a press over a panel does
    /// not. Answers whether the press was the window's — see
    /// [`App::close_window`].
    pub(crate) fn close_window_under_pointer(&mut self) -> bool {
        let Some(subject) = self.window_under_pointer() else {
            return false;
        };
        self.close_window(subject)
    }

    /// The topmost of this client's own windows, closed from the keyboard.
    ///
    /// [`windows::Windows::own_windows`] is in painter's order, so its last entry is the one
    /// drawn over the others — which is what a player means by "this window"
    /// when they have not pointed at anything.
    ///
    /// **Why the keyboard needs a route of its own.** A gump window is drawn by
    /// this client's own pass and egui is painted *over* it, so a floating panel
    /// standing on one covers it and takes the mouse with it:
    /// `Shell::on_window_event` claims the click before any of `window_event`'s
    /// arms are reached, and the right button never gets as far as
    /// [`App::close_window_under_pointer`]. The skill window cascades to
    /// `CONTAINER_ORIGIN`, which is inside where the dev window opens — so for
    /// as long as Escape quit the client, it was a window with no way out.
    pub(crate) fn close_top_window(&mut self) -> bool {
        let Some(subject) = self.windows.own_windows.last().map(|window| window.subject) else {
            return false;
        };
        self.close_window(subject)
    }

    /// Take one window down, whichever gesture asked for it — the right button
    /// over it, or Escape on the topmost.
    ///
    /// Answers whether the window *took* the request rather than whether it
    /// closed: a `{ noclose }` dialog stays up and still answers true, because
    /// the press that asked was the window's and must not reach the world
    /// behind it.
    ///
    /// Nothing goes out on the wire, for either kind. There is no
    /// close-container packet and no close-paperdoll packet — the shard keeps
    /// its own list of who has what open — which is why this end predicts the
    /// close locally (see [`windows::Windows::locally_closed`]) rather than waiting for a
    /// packet that never comes.
    /// A dialog is the one kind that *does* send something: the shard is
    /// waiting for a `0xB1` and gets button zero, which is what the reference
    /// client's close box answers with. A `{ noclose }` layout has no such
    /// answer to give — `dismiss` refuses it — and the window stays up, which is
    /// what that flag is for.
    pub(crate) fn close_window(&mut self, subject: WindowSubject) -> bool {
        if let WindowSubject::Dialog(gump_id) = subject {
            let Some(gump) = self.open_gump(gump_id).cloned() else {
                return false;
            };
            let Some(reply) = self.windows.dialogs.dismiss(&gump) else {
                // Answered by its own buttons or not at all. The press is still
                // the window's — it must not steer the body — so this says the
                // window took it.
                return true;
            };
            self.answer_gump(reply);
            self.windows
                .own_windows
                .retain(|window| window.subject != subject);
            self.windows.dragging = None;
            return true;
        }
        if self.world.authoritative.view.is_none() {
            return false;
        }
        match subject {
            WindowSubject::Container(serial) => {
                // The overlay, not `self.world.authoritative.view`, is what says this is closed —
                // that copy is never authoritative, see D2 in
                // `docs/client_window_state.md`. The shard thread's own
                // `WorldView` is what every future snapshot is cloned whole
                // from, and telling it is what `link::Command::CloseWindow`
                // is for; the overlay is what keeps this end from drawing the
                // stale, still-open entry in the meantime.
                self.windows.locally_closed.insert(subject);
                self.apply_close_window(link::CloseTarget::Container(serial));
            }
            WindowSubject::Paperdoll(serial) => {
                self.windows.locally_closed.insert(subject);
                self.apply_close_window(link::CloseTarget::Paperdoll(serial));
            }
            // Nothing in the view to tell and so nothing to overlay: the
            // skills stay where they are, the way a paperdoll's equipment
            // does. What closing takes away is the tree — which headings were
            // shut and where the list was scrolled to — and that is
            // deliberate: the reference's window does not remember either,
            // and a window with no memory is the backlog entry both kinds
            // already share.
            WindowSubject::Skills => {
                self.windows.skills = None;
                self.windows.held_skill = None;
            }
            WindowSubject::Status => self.windows.status = false,
            WindowSubject::Dialog(_) => unreachable!("answered above"),
        }
        self.windows
            .own_windows
            .retain(|window| window.subject != subject);
        self.windows.dragging = None;
        true
    }

    /// Say a line out loud, if there is a shard to hear it.
    ///
    /// Nothing is echoed locally. A shard sends every speaker their own words
    /// back — that is what makes `0xAE` exist — so a client that also drew them
    /// itself would show everything twice, and a line that never reached the
    /// server would look exactly like one that did.
    ///
    /// Offline the line goes nowhere and says so in the log rather than
    /// silently: the map viewer has nobody to talk to, and a chat box that
    /// swallowed what was typed would read as a broken connection.
    pub(crate) fn say(&mut self, line: String) {
        match self.world.link.as_ref() {
            Some(link) => link.say(line),
            None => tracing::info!(%line, "nothing said: no shard is connected"),
        }
    }

    /// Answer an open dialog and take it off the screen.
    ///
    /// The close is this end's, and it is why the overlay is set here rather
    /// than waiting for a packet: the server sends one `0xB0` and waits for
    /// one `0xB1`, and nothing ever arrives to say the window is gone. See
    /// [`windows::Windows::locally_closed`].
    pub(crate) fn answer_gump(&mut self, reply: link::GumpReply) {
        let gump_id = openshard_protocol::gump::GumpId(reply.gump_id.0);
        if let Some(link) = self.world.link.as_ref() {
            link.answer_gump(reply);
            // The reply itself leaves on the wire, but nothing about it tells
            // the shard thread's own `WorldView` — which every future
            // snapshot is cloned whole from — that this window is done; see
            // `link::Command::CloseWindow`.
            self.apply_close_window(link::CloseTarget::Gump(gump_id));
        }
        if self.world.authoritative.view.is_some() {
            self.windows.locally_closed.insert(WindowSubject::Dialog(gump_id));
        }
    }
}
