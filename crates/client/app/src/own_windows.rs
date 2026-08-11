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

use std::time::Instant;

use openshard_client_render::gump::{self as gump_art, GumpPixel};
use openshard_client_render::{paperdoll, skills};
use openshard_protocol::gump::GumpId;
use openshard_protocol::mobile::Equipment;
use openshard_protocol::skill::SkillLock;
use openshard_protocol::wire::{Layer, RawSkillId};

use crate::app::App;
use crate::windows::{Drawn, WindowSubject, reconcile_own_windows};
use crate::{gump, link, scroll_pairs};

impl App {
    /// Open a window for everything the shard has opened and this client has
    /// not placed yet, and drop the windows whose subject is gone.
    ///
    /// Run once a frame rather than when the packet arrived, and idempotent for
    /// that reason: the `0x24` and the `0x88` are folded into the [`WorldView`]
    /// by `client/net`, which knows nothing about screens, so the window is
    /// this end noticing that the view has grown something it has nowhere to
    /// put.
    ///
    /// The drop is the other direction of the same idea: a container removed
    /// from the world — or a mobile destroyed — takes its entry in the view
    /// with it (see `WorldView::apply`'s `Remove` arm), and a window over
    /// nothing must not outlive it.
    pub(crate) fn sync_own_windows(&mut self) {
        let Some(view) = self.world.view.as_ref() else {
            // No world, no windows: a map viewer has no shard to have opened
            // one, and anything left over is from a session that has ended.
            self.windows.own_windows.clear();
            // Including the skill window, whose existence is this field: a tree
            // left standing here would reopen the window at the next login with
            // the last session's headings shut.
            self.windows.skills = None;
            self.windows.held_skill = None;
            return;
        };
        // The state a dialog holds that no packet does, kept in step with the
        // same list: a window the shard has taken away forgets its page, its
        // switches and the finger on it — see `gump::Dialogs::sync`.
        self.windows.dialogs.sync(&view.gumps);
        reconcile_own_windows(
            view,
            &mut self.windows.own_windows,
            &mut self.windows.locally_closed,
            self.windows.skills.is_some(),
        );
    }

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

    /// Which of a paperdoll's pictures the cursor is over is a button, if it is
    /// over one at all.
    ///
    /// Against the list the last frame drew and the atlas it was drawn from —
    /// [`windows::Windows::drawn_windows`]' rule — so this and the picture on the screen
    /// cannot answer differently. `hits` is what turns an index into a meaning;
    /// a picture that is not in it is the frame, the body or a garment, and a
    /// press there is a press on the window.
    pub(crate) fn doll_button_under_pointer(&self, subject: WindowSubject) -> Option<paperdoll::DollButton> {
        let Some(Drawn::Paperdoll(doll)) = self.drawn(subject) else {
            return None;
        };
        let index = gump_art::pick(
            &doll.pictures,
            self.input.pointer_gump,
            &self.resources.gump_atlas,
        )?;
        doll.hits.get(&index).copied()
    }

    /// Which of the skill window's pictures the cursor is over means something,
    /// if any of them does.
    ///
    /// [`App::doll_button_under_pointer`]'s twin, and the same rule: the list is
    /// the one the last frame drew, so a row cut away by the viewport is not
    /// pickable where it is not drawn — that box is on the pictures themselves,
    /// which is why nothing here has to know about it.
    pub(crate) fn skill_hit_under_pointer(&self) -> Option<skills::Hit> {
        let Some(Drawn::Skills(sheet)) = self.drawn(WindowSubject::Skills) else {
            return None;
        };
        sheet.hit(self.input.pointer_gump, &self.resources.gump_atlas)
    }

    /// How tall the list is right now — what every scroll is clamped against.
    ///
    /// Asked afresh rather than remembered: shutting a heading changes it, and a
    /// clamp against a stale height either refuses a scroll that is now legal or
    /// allows one that is not.
    pub(crate) fn skill_content(&self) -> i32 {
        match self.windows.skills.as_ref() {
            Some(tree) => {
                skills::content_height(&self.resources.skill_names, &self.resources.skill_groups, tree)
            }
            None => 0,
        }
    }

    /// What a click on the skill window does.
    ///
    /// Four of the five hits; the thumb is the fifth and does its work on the
    /// move rather than the release — see [`App::drag_thumb`].
    pub(crate) fn skill_clicked(&mut self, hit: skills::Hit) {
        let content = self.skill_content();
        // The point the *track* was clicked, taken before the tree is borrowed
        // mutably: it is read off the window the last frame drew, and that
        // borrow and this one cannot both stand.
        let jumped = match hit {
            skills::Hit::Track => match self.drawn(WindowSubject::Skills) {
                Some(Drawn::Skills(sheet)) => Some(sheet.offset_at(self.input.pointer_gump, content)),
                _ => None,
            },
            _ => None,
        };
        let Some(tree) = self.windows.skills.as_mut() else {
            return;
        };
        match hit {
            // Opening or shutting a heading leaves the scroll where it is,
            // which can be past the end of a list that has just got shorter —
            // so it is clamped against what the list is *now*.
            skills::Hit::Heading(group) => {
                tree.toggle(group);
                let content =
                    skills::content_height(&self.resources.skill_names, &self.resources.skill_groups, tree);
                tree.scroll_to(tree.offset(), content);
            }
            skills::Hit::Up => tree.scroll_by(-skills::STEP, content),
            skills::Hit::Down => tree.scroll_by(skills::STEP, content),
            skills::Hit::Track => {
                if let Some(offset) = jumped {
                    tree.scroll_to(offset, content);
                }
            }
            skills::Hit::Thumb => {}
            // Drawn back immediately rather than left to a reply that never
            // comes — see `skills::Tree::lock_of`'s doc for why the shard
            // sends nothing here.
            skills::Hit::Lock(id) => {
                let shard = self
                    .world
                    .view
                    .as_ref()
                    .and_then(|view| view.player.skills.get(&id.0))
                    .map(|line| line.lock)
                    .unwrap_or_default();
                let next = match tree.lock_of(id, shard) {
                    SkillLock::Up => SkillLock::Down,
                    SkillLock::Down => SkillLock::Locked,
                    SkillLock::Locked => SkillLock::Up,
                };
                tree.set_lock(id, next);
                if let Some(link) = self.world.link.as_ref() {
                    link.set_skill_lock(RawSkillId(id.0), next);
                }
            }
            skills::Hit::Use(id) => {
                if let Some(link) = self.world.link.as_ref() {
                    link.use_skill(RawSkillId(id.0));
                }
            }
        }
    }

    /// Follow the pointer with a thumb that is being dragged. Answers whether
    /// the list moved.
    ///
    /// Driven from the mouse's own movement, like [`App::drag_own_window`] and
    /// for the same reason: a drag is a gesture that is under way between a
    /// press and a release, and the release only ends it.
    pub(crate) fn drag_thumb(&mut self) -> bool {
        if self.windows.held_skill != Some(skills::Hit::Thumb) {
            return false;
        }
        let content = self.skill_content();
        let Some(Drawn::Skills(sheet)) = self.drawn(WindowSubject::Skills) else {
            return false;
        };
        let offset = sheet.offset_at(self.input.pointer_gump, content);
        let Some(tree) = self.windows.skills.as_mut() else {
            return false;
        };
        let before = tree.offset();
        tree.scroll_to(offset, content);
        tree.offset() != before
    }

    /// A wheel notch over the skill window scrolls it instead of zooming the
    /// world. Answers whether the window took the notch.
    ///
    /// Taken whenever the pointer is over the window, even when the list is
    /// already at its end: a wheel that fell through to the camera because the
    /// list could not move would zoom the world from inside a window, which is
    /// the one thing a player rolling a wheel over a list does not mean.
    pub(crate) fn scroll_skills(&mut self, notches: f32) -> bool {
        if self.window_under_pointer() != Some(WindowSubject::Skills) {
            return false;
        }
        let content = self.skill_content();
        if let Some(tree) = self.windows.skills.as_mut() {
            // A notch is a row, and up the wheel is up the list.
            let step = match notches > 0.0 {
                true => -skills::STEP,
                false => skills::STEP,
            };
            tree.scroll_by(step, content);
        }
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

    /// One of a paperdoll's buttons was clicked: send what it means.
    ///
    /// **Every one of these is a request and nothing else.** Not a window this
    /// client opens on its own, not a stance it flips locally: the shard answers
    /// the toggle with a `0x72`, the Quest button with a dialog, the Skills
    /// button with a `0x3A`, and what is drawn follows *those*. It is
    /// [`App::use_under_cursor`]'s rule for the interface — a client that acted
    /// on its own would show a state the server refused.
    ///
    /// # The three scrolls want a pair
    ///
    /// The seven buttons down the frame answer a single click (`Button` and
    /// `OnButtonClick` in `PaperDollGump`); the profile scroll, the party
    /// manifest and the virtue menu are `GumpPic`s with a
    /// `MouseDoubleClick` handler, and a single click on one does nothing at
    /// all. That difference is honoured here rather than in
    /// [`paperdoll::DollButton`], which says which picture was hit and nothing
    /// about what the mouse did to it.
    ///
    /// # What sends nothing, and why it is not a guess
    ///
    /// Help, Options and the party manifest have nowhere to go: `0x9B` and the
    /// party's `0xBF 0x06` are not in `openshard_protocol` yet, and the options
    /// window is a client's own and does not exist. Profile (`0xB8`) is the same
    /// gap. They press, they come back up, and they are written down in
    /// `docs/client.md` — a packet invented here so that a button "did
    /// something" would be a shard logging an unknown id for a window that is
    /// never going to open.
    pub(crate) fn doll_clicked(&mut self, subject: WindowSubject, button: paperdoll::DollButton) {
        let WindowSubject::Paperdoll(mobile) = subject else {
            return;
        };
        // A doll is filed under the serial it is a picture of, so *whose* it is
        // is this question and not a second field: only our own frame carries
        // the six buttons and the toggle, but a stranger's carries Status and
        // the profile scroll, and those name the body they were clicked on.
        let Some(view) = self.world.view.as_ref() else {
            return;
        };
        let own = view.player.serial == mobile;
        let war = view.player.war;
        // The equipment list this doll's serial actually is: our own carries it
        // on `Player`, everybody else on the `Mobile` the view filed them under.
        // Neither `EquipmentLayer` nor `paperdoll::Doll` carries a serial at
        // all — see the module doc on `paperdoll` — so the backpack's has to be
        // read back off the same list the `0x88`/`0x78` filled in.
        let equipment: &[Equipment] = match own {
            true => &view.player.equipment,
            false => view
                .mobiles
                .get(&mobile)
                .map_or(&[] as &[Equipment], |mobile| mobile.equipment.as_slice()),
        };
        let backpack = equipment
            .iter()
            .find(|item| item.layer == Layer::BACKPACK)
            .map(|item| item.serial);
        // Before the link is borrowed, and for all four of these rather than
        // only the ones with a packet: the pair is a fact about the gesture, and
        // a scroll that recorded no first click would let a *later* click on
        // another scroll pair with something older than it.
        let paired = match button {
            paperdoll::DollButton::Profile
            | paperdoll::DollButton::Party
            | paperdoll::DollButton::Virtue
            | paperdoll::DollButton::Backpack => self.scroll_paired(subject, button),
            _ => false,
        };
        let Some(link) = self.world.link.as_ref() else {
            return;
        };
        // Set inside the match and acted on after it: the link is borrowed out
        // of `self` for the length of it, and opening the window is a write.
        let mut opened_skills = false;
        match button {
            // The one picture whose *state* is on the frame: what is asked for
            // is the opposite of what the last packet about the stance said.
            paperdoll::DollButton::WarMode => link.war_mode(!war),
            paperdoll::DollButton::LogOut => link.log_out(),
            paperdoll::DollButton::Quests => link.quest_log(),
            paperdoll::DollButton::Guild => link.guild_menu(),
            // The two windows this client cannot draw yet. The request still
            // goes out: the shard answers a `0x34` with a `0x11` or a `0x3A`,
            // and the day either window is built it will be built against a
            // packet that is already arriving rather than against a guess.
            //
            // Only for our own doll, because that is all this shard answers:
            // `RequestStatus` is keyed on the connection and ignores the serial
            // in the packet (see `StatusQuery::serial`), so pressing Status on a
            // stranger's frame would send our own status back and open nothing
            // about them. A health bar over somebody else is a window of its
            // own — backlog, `docs/client.md`.
            paperdoll::DollButton::Status if own => link.status(mobile),
            // The window opens *here*, on the press, and the packet only fills
            // it: the shard sends the whole list at world entry as well, so a
            // window that opened when a `0x3A` arrived would open itself at
            // every login. Opened before the answer comes back, which is why a
            // skill with no line yet is a row with an empty column rather than
            // an empty window.
            //
            // Only for our own doll, `Status`'s reason: a `0x3A` has no serial
            // in it and is always about the body at this end.
            paperdoll::DollButton::Skills if own => {
                link.skills(mobile);
                opened_skills = true;
            }
            // The scrolls, which are a *double* click. `Virtue` is the only one
            // of the three with a packet — the reference's `0xB1` under a gump
            // id nobody opened, see `openshard_client_net::doll::virtue`.
            paperdoll::DollButton::Virtue if paired => link.virtue(mobile),
            // The backpack, the same double click again: `0x06` on its serial,
            // exactly what a bag on the ground gets from
            // [`App::use_under_cursor`]. Nothing is opened here — the `0x24`
            // that answers it is what does, the same rule that keeps the door
            // and the toggle from acting before the shard has.
            paperdoll::DollButton::Backpack if paired => {
                if let Some(serial) = backpack {
                    link.use_object(serial);
                }
            }
            // Everything else: a stranger's Status, the first click of a pair,
            // and the four buttons with nothing to send.
            _ => {}
        }
        if opened_skills {
            // Pressing it again with the window already open leaves the tree
            // alone — the headings the player shut stay shut — and asks the
            // shard for the list once more, which is what the reference's own
            // button does.
            self.windows.skills.get_or_insert_with(skills::Tree::default);
        }
    }

    /// Whether this click on a scroll is the second of a pair, on the same
    /// scroll of the same window.
    ///
    /// [`input::Input::last_click`]'s rule, applied to a picture instead of the world:
    /// cleared when a pair fires, so a third click starts a fresh one, and the
    /// subject and the button are both compared — two clicks on two different
    /// scrolls are two first clicks, not a double click on the second.
    ///
    /// Only ever asked about the three scrolls. The seven buttons act on the
    /// first click and never reach here.
    pub(crate) fn scroll_paired(&mut self, subject: WindowSubject, button: paperdoll::DollButton) -> bool {
        let now = Instant::now();
        let paired = scroll_pairs(self.windows.last_scroll, now, subject, button);
        self.windows.last_scroll = (!paired).then_some((now, subject, button));
        paired
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
        if self.world.view.is_none() {
            return false;
        }
        match subject {
            WindowSubject::Container(serial) => {
                // The overlay, not `self.world.view`, is what says this is closed —
                // that copy is never authoritative, see D2 in
                // `docs/client_window_state.md`. The shard thread's own
                // `WorldView` is what every future snapshot is cloned whole
                // from, and telling it is what `link::Command::CloseWindow`
                // is for; the overlay is what keeps this end from drawing the
                // stale, still-open entry in the meantime.
                self.windows.locally_closed.insert(subject);
                if let Some(link) = self.world.link.as_ref() {
                    link.close_window(link::CloseTarget::Container(serial));
                }
            }
            WindowSubject::Paperdoll(serial) => {
                self.windows.locally_closed.insert(subject);
                if let Some(link) = self.world.link.as_ref() {
                    link.close_window(link::CloseTarget::Paperdoll(serial));
                }
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
            link.close_window(link::CloseTarget::Gump(gump_id));
        }
        if self.world.view.is_some() {
            self.windows.locally_closed.insert(WindowSubject::Dialog(gump_id));
        }
    }
}
