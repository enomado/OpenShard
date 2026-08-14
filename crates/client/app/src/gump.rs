//! The shard's dialogs: the state a `0xB0` has that its layout does not.
//!
//! A gump is UO's windowing primitive, and the shard sends one as a *layout* —
//! a list of elements at pixel coordinates, keyed to art in the client's files.
//! `.admin` opens one; so does a shopkeeper, a quest and a book.
//!
//! # Nothing here draws
//!
//! This module held an egui rendering of a layout for as long as no gump art
//! could be read. That is over: `client/render`'s
//! [`gump`](openshard_client_render::gump) pass draws the pictures a layout
//! names, out of the client's own `gumpartLegacyMUL.uop`, and
//! [`letters`](openshard_client_render::gump::letters) draws its text out of
//! `fonts.mul`. What is left here is the part that is not a picture: which page
//! is showing, which switches the player has set, what has been typed into a
//! field, and which button the finger is on.
//!
//! The egui window went with the drawing, and that was the *layout* fix rather
//! than a tidy-up. A dialog drawn as an egui window was two windows: egui's
//! frame, title bar and close box on the outside, and the shard's own
//! background picture inside it — one of which the reference client has and the
//! other of which it does not. Worse, every widget over the art was sized by a
//! constant this module had invented (a 26 by 20 button, a 220-point label),
//! because egui needs a size *before* the art is packed and the art's real size
//! was only known afterwards. So the clickable rectangle and the picture under
//! it were two different rectangles, and the window's own extent was a third.
//! Now a window is exactly the list of pictures it drew, a click is an opaque
//! texel of one of them ([`pick`](openshard_client_render::gump::pick)), and
//! there is no second opinion to keep in step.
//!
//! # Three things the wire does not say
//!
//! 1. **A window closes when it is answered.** No packet says so — the server
//!    sends one `0xB0`, waits for one `0xB1`, and both ends assume the client
//!    took the window down. That is [`WorldView::gump_closed`], called by the
//!    caller once the reply is on its way.
//! 2. **A page button never reaches the server.** `{ button ... 0 N id }` flips
//!    to page `N` inside the client, and only a reply button answers. So the
//!    current page is state that lives here and nowhere else.
//! 3. **A button is pressed on the way down and answered on the way up.** The
//!    layout carries two pictures for every button and nothing that says when
//!    to draw the second one; it is the mouse, and [`Dialogs::held`] is where
//!    the mouse is remembered between the two events.
//!
//! [`WorldView::gump_closed`]: openshard_client_net::view::WorldView::gump_closed

use std::collections::{BTreeSet, HashMap};

use openshard_client_net::view::OpenGump;
use openshard_client_render::atlas::FontAtlas;
use openshard_client_render::gump::{self, CAPTION_FONT, CaptionSource, GumpAtlas, GumpPixel, Hit, Window};
use openshard_client_render::text::{self, GumpLabel};
use openshard_protocol::gump::layout::{Element, Flag};
use openshard_protocol::gump::{GumpId, RawButtonId, RawGumpId, RawGumpKey, RawSwitchId};
use openshard_uofiles::cliloc::Cliloc;

use crate::link::GumpReply;

/// What every open dialog is holding that no packet carries.
///
/// Keyed by dialog id rather than by index: the list of open windows is rebuilt
/// from the [`WorldView`](openshard_client_net::view::WorldView) every frame, so
/// a position in it is not an identity, and a redrawn dialog would otherwise
/// silently inherit the state of whatever now stands where it used to.
#[derive(Default)]
pub struct Dialogs {
    by_dialog: HashMap<GumpId, Sheet>,
    /// The button the mouse is down on, and whose window it belongs to.
    ///
    /// One, not one per window: there is one pointer. It is a
    /// [`Hit`] rather than a button id because that is what a press *is* —
    /// [`gump::window`] draws the pressed face by comparing this against the
    /// hit it computes, so what looks pressed and what the release will act on
    /// are one value.
    held: Option<(GumpId, Hit)>,
    /// The field taking keystrokes, and whose window it belongs to.
    ///
    /// `None` is "the keyboard belongs to the world", which is the ordinary
    /// state: a dialog with no field in it never takes one, and clicking off a
    /// field gives the keyboard back.
    focus: Option<(GumpId, TextEntryId)>,
}

/// A page inside one gump layout.
///
/// Page numbers are client-side state, distinct from gump ids and from the
/// layout's text-entry ids. The renderer's layout boundary stays raw because
/// that is where its parser declares the number.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct GumpPage(u32);

impl GumpPage {
    const fn new(page: u32) -> Self {
        Self(page)
    }

    const fn raw(self) -> u32 {
        self.0
    }
}

/// The identity of one editable field inside a gump layout.
///
/// It is not a page, a button, or a switch id. The wire/layout seam is where
/// it becomes the `u16` a `0xB1` text-entry list carries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct TextEntryId(u16);

impl TextEntryId {
    const fn new(id: u16) -> Self {
        Self(id)
    }

    const fn raw(self) -> u16 {
        self.0
    }
}

/// One window's answers-in-progress.
#[derive(Default)]
struct Sheet {
    /// The page being shown. Page `0` is drawn on every page, so this starts at
    /// zero and a layout with no pages at all never leaves it.
    page: GumpPage,
    /// Which switches are on, by their id. Seeded from the layout's own
    /// `initial` flags the first time an id is seen, and the player's after
    /// that — which is why absence and `false` are different here.
    switches: HashMap<RawSwitchId, bool>,
    /// What has been typed into each field, by its id. Seeded the same way,
    /// from the line the layout pointed the field at.
    entries: HashMap<TextEntryId, String>,
}

/// What the window flags in a layout ask for.
#[derive(Default, Clone, Copy)]
pub struct WindowFlags {
    /// `{ nomove }`: the player may not drag it.
    pub no_move: bool,
    /// `{ noclose }`: the right button does not take it down, and it has to be
    /// answered by one of its own buttons.
    pub no_close: bool,
}

/// Read the flags out of a layout. `{ nodispose }` and `{ noresize }` have
/// nothing to honour here: no window this client draws is resizable, and there
/// is no separate dismissal to suppress.
pub fn flags(gump: &OpenGump) -> WindowFlags {
    let mut flags = WindowFlags::default();
    for element in &gump.elements {
        match element {
            Element::Flag(Flag::NoMove) => flags.no_move = true,
            Element::Flag(Flag::NoClose) => flags.no_close = true,
            _ => {}
        }
    }
    flags
}

impl Dialogs {
    /// Forget the windows the shard has taken away, and give a new one's
    /// switches and fields the values its layout asked for.
    ///
    /// Seeding is the *first time only*: after that the maps hold what the
    /// player has done, and re-seeding would drag a checkbox back out from
    /// under their finger every frame. Forgetting is the other direction of the
    /// same idea — a dialog that comes back comes back as the server drew it.
    pub fn sync(&mut self, open: &[OpenGump]) {
        self.by_dialog
            .retain(|id, _| open.iter().any(|gump| gump.gump_id == *id));
        if let Some((id, _)) = self.held {
            if !open.iter().any(|gump| gump.gump_id == id) {
                self.held = None;
            }
        }
        if let Some((id, _)) = self.focus {
            if !open.iter().any(|gump| gump.gump_id == id) {
                self.focus = None;
            }
        }
        for gump in open {
            let sheet = self.by_dialog.entry(gump.gump_id).or_default();
            for element in &gump.elements {
                match element {
                    Element::Check(switch) | Element::Radio(switch) => {
                        sheet.switches.entry(switch.id).or_insert(switch.initial);
                    }
                    Element::TextEntry { entry_id, line, .. } => {
                        sheet
                            .entries
                            .entry(TextEntryId::new(*entry_id))
                            .or_insert_with(|| gump.line(*line).unwrap_or_default().to_owned());
                    }
                    _ => {}
                }
            }
        }
    }

    /// Lay a dialog out at `at`, in the state the player has put it in.
    ///
    /// Every argument [`gump::window`] takes beyond the layout comes from here,
    /// which is the point of this module: the page, the switches and the
    /// pressed button are the three things the wire does not carry.
    pub fn layout(&self, gump: &OpenGump, at: GumpPixel, atlas: &GumpAtlas) -> Window {
        let sheet = self.by_dialog.get(&gump.gump_id);
        let on: BTreeSet<RawSwitchId> = sheet
            .map(|sheet| {
                sheet
                    .switches
                    .iter()
                    .filter(|(_, set)| **set)
                    .map(|(&id, _)| id)
                    .collect()
            })
            .unwrap_or_default();
        let held = self
            .held
            .filter(|(id, _)| *id == gump.gump_id)
            .map(|(_, hit)| hit);
        let page = sheet.map_or(GumpPage::default(), |sheet| sheet.page).raw();
        gump::window(&gump.elements, at, page, &on, held, atlas)
    }

    /// Every line of text a laid-out dialog draws: its captions, and what is in
    /// its fields.
    ///
    /// The captions name rows of the text table that arrived beside the layout
    /// and this is where they are resolved — `client/render` has never heard of
    /// an [`OpenGump`] and should not. A row a layout names and the table does
    /// not hold draws nothing, which is a shard bug and not a reason to drop
    /// the window.
    ///
    /// A focused field is written with a caret after it. That is the only thing
    /// this client draws that the layout did not ask for, and it is the one
    /// thing a player cannot otherwise see: which of two boxes the keys are
    /// going into.
    pub fn lines<'a>(
        &'a self,
        gump: &'a OpenGump,
        window: &Window,
        fonts: &FontAtlas,
        cliloc: Option<&'a Cliloc>,
    ) -> Vec<GumpLabel<'a>> {
        let mut lines: Vec<GumpLabel<'a>> = window
            .captions
            .iter()
            .filter_map(|caption| {
                // A wire line is missing when the layout named a row past the
                // table's end — a bug on the sending side. A cliloc is missing
                // both when this client has no `Cliloc.enu` at all and when
                // the number simply is not one of the ~40,000 it holds; either
                // way there is nothing to draw and the caption is dropped
                // rather than shown as a placeholder, the same tolerance
                // `gump.line` already has.
                let text = match caption.source {
                    CaptionSource::Line(line) => gump.line(line)?,
                    CaptionSource::Cliloc(number) => cliloc?.get(number)?,
                };
                Some(GumpLabel {
                    at: caption.at,
                    hue: caption.hue,
                    clip: caption.clip,
                    text,
                    font: CAPTION_FONT,
                })
            })
            .collect();
        let sheet = self.by_dialog.get(&gump.gump_id);
        for field in &window.fields {
            let typed = sheet
                .and_then(|sheet| sheet.entries.get(&TextEntryId::new(field.id)))
                .map(String::as_str)
                .unwrap_or_default();
            lines.push(GumpLabel {
                at: field.at,
                hue: field.hue,
                clip: Some(field.size),
                text: typed,
                font: CAPTION_FONT,
            });
            if self.focus == Some((gump.gump_id, TextEntryId::new(field.id))) {
                // A second line rather than a character appended to the first:
                // the text is the player's and borrowed, and a caret glued onto
                // it would be a `String` this frame owns. Where it goes is the
                // one thing that has to be measured, which is what
                // `gump::width` is for — the same walk `letters` advances by,
                // so the caret lands where the next character will.
                lines.push(GumpLabel {
                    at: field
                        .at
                        .offset(GumpPixel::new(text::gump_width(typed, CAPTION_FONT, fonts), 0)),
                    hue: field.hue,
                    clip: None,
                    text: CARET,
                    font: CAPTION_FONT,
                });
            }
        }
        lines
    }

    /// A left press over a laid-out dialog.
    ///
    /// Answers whether the press was *taken* by something in the window. A
    /// press that lands on a picture that answers to nothing — the background,
    /// a `{ gumppic }`, a label — is not taken, and the caller drags the window
    /// with it, which is how a gump is moved without a title bar.
    ///
    /// A switch answers on the way *down*, and a button does not: that is the
    /// reference's own split. A checkbox is its own answer and there is nothing
    /// to wait for; a button has a pressed picture to show while the finger is
    /// on it, and what it means happens when the finger comes off.
    pub fn press(&mut self, gump: &OpenGump, window: &Window, cursor: GumpPixel, atlas: &GumpAtlas) -> bool {
        // A field is a box and not a picture, and it is tested first for that
        // reason: it lies over the background, which *is* a picture, and asking
        // the pictures first would answer "the background" for every click into
        // a field.
        if let Some(id) = gump::field(&window.fields, cursor) {
            self.focus = Some((gump.gump_id, TextEntryId::new(id)));
            return true;
        }
        self.focus = None;
        let Some(hit) = window.hit(cursor, atlas) else {
            return false;
        };
        match hit {
            Hit::Reply(_) | Hit::Page(_) => {
                self.held = Some((gump.gump_id, hit));
            }
            Hit::Check(id) => self.toggle(gump.gump_id, id),
            Hit::Radio(id) => self.choose(gump, id),
        }
        true
    }

    /// The release that finishes a press, and the answer it produced if it was
    /// a reply button.
    ///
    /// The pointer has to still be on the button it went down on, which is what
    /// the second [`gump::pick`] is for: a press dragged off its button is a
    /// press taken back, in this client as in every other.
    ///
    /// A page button never answers. It flips the page here and the server is
    /// never told — see the module docs.
    pub fn release(
        &mut self,
        gump: &OpenGump,
        window: &Window,
        cursor: GumpPixel,
        atlas: &GumpAtlas,
    ) -> Option<GumpReply> {
        let (id, held) = self.held.take()?;
        if id != gump.gump_id {
            return None;
        }
        let under = window.hit(cursor, atlas);
        if under != Some(held) {
            return None;
        }
        match held {
            Hit::Page(page) => {
                self.by_dialog.entry(gump.gump_id).or_default().page = GumpPage::new(page);
                None
            }
            Hit::Reply(button) => Some(self.reply(gump, button)),
            // A switch is never held — see `press`.
            Hit::Check(_) | Hit::Radio(_) => None,
        }
    }

    /// The button a press is waiting on, for whoever needs to know whether the
    /// mouse is busy.
    pub fn holding(&self) -> Option<GumpId> {
        self.held.map(|(id, _)| id)
    }

    /// Take a keystroke into the focused field, and say whether one was taken.
    ///
    /// `text` is what the keyboard produced — `winit`'s own `KeyEvent::text`,
    /// which is the layout's and the IME's answer rather than a key code, so
    /// this is the one place in the client where a character is a character.
    /// Control characters are refused: a newline in a `{ textentry }` is not a
    /// character the reference lets in either.
    pub fn typed(&mut self, text: &str) -> bool {
        let Some((id, field)) = self.focus else {
            return false;
        };
        let wanted: String = text.chars().filter(|c| !c.is_control()).collect();
        if wanted.is_empty() {
            return false;
        }
        self.by_dialog
            .entry(id)
            .or_default()
            .entries
            .entry(field)
            .or_default()
            .push_str(&wanted);
        true
    }

    /// Rub out the last character of the focused field. Answers whether there
    /// was a field to rub out of.
    pub fn backspace(&mut self) -> bool {
        let Some((id, field)) = self.focus else {
            return false;
        };
        if let Some(text) = self
            .by_dialog
            .get_mut(&id)
            .and_then(|sheet| sheet.entries.get_mut(&field))
        {
            text.pop();
        }
        true
    }

    /// Whether a field is taking keys. What the caller asks before it lets a
    /// key walk the character.
    pub fn typing(&self) -> bool {
        self.focus.is_some()
    }

    /// Give the keyboard back to the world.
    pub fn unfocus(&mut self) {
        self.focus = None;
    }

    /// The answer a window closed by the right button sends: button zero.
    ///
    /// The shard is waiting for it exactly as much as it is waiting for a real
    /// button — one `0xB0` out, one `0xB1` back — which is why closing a dialog
    /// is not the same as forgetting it. `None` for a `{ noclose }` layout,
    /// which has to be answered by one of its own buttons.
    pub fn dismiss(&mut self, gump: &OpenGump) -> Option<GumpReply> {
        if flags(gump).no_close {
            return None;
        }
        Some(self.reply(gump, RawButtonId(0)))
    }

    /// Turn a checkbox over.
    fn toggle(&mut self, gump_id: GumpId, switch: RawSwitchId) {
        let sheet = self.by_dialog.entry(gump_id).or_default();
        let set = sheet.switches.entry(switch).or_default();
        *set = !*set;
    }

    /// Turn a radio on and the rest of its group off.
    ///
    /// The client is what enforces that, not the server: the wire carries the
    /// ids that are left on and trusts that only one of a group is among them.
    /// Every other radio in the layout is the group, because the layout has no
    /// way to say otherwise.
    fn choose(&mut self, gump: &OpenGump, switch: RawSwitchId) {
        let sheet = self.by_dialog.entry(gump.gump_id).or_default();
        for element in &gump.elements {
            if let Element::Radio(other) = element {
                sheet.switches.insert(other.id, other.id == switch);
            }
        }
    }

    /// What travels back: the button, and everything the player set on the way
    /// to pressing it.
    ///
    /// The switches and the fields are sorted before they are sent. Nothing on
    /// the wire requires it — the server reads the ids as a set — but a
    /// `HashMap`'s order is not stable between runs, and a packet whose bytes
    /// depend on the iteration order of a hash map is one that cannot be
    /// compared against a recording.
    fn reply(&mut self, gump: &OpenGump, button: RawButtonId) -> GumpReply {
        let sheet = self.by_dialog.entry(gump.gump_id).or_default();
        let mut switches: Vec<RawSwitchId> = sheet
            .switches
            .iter()
            .filter(|(_, &set)| set)
            .map(|(&id, _)| id)
            .collect();
        switches.sort_unstable();

        let mut text_entries: Vec<(u16, String)> = sheet
            .entries
            .iter()
            .map(|(&id, text)| (id.raw(), text.clone()))
            .collect();
        text_entries.sort_unstable_by_key(|(id, _)| *id);

        GumpReply {
            key: RawGumpKey(gump.key.0),
            gump_id: RawGumpId(gump.gump_id.0),
            button,
            switches,
            text_entries,
        }
    }
}

/// What a focused field draws after what has been typed into it.
const CARET: &str = "|";

#[cfg(test)]
mod tests {
    use openshard_protocol::gump::layout::parse;
    use openshard_protocol::gump::{ButtonId, GumpButton, GumpId, GumpKey, GumpLayout, GumpPoint, SwitchId};

    use super::*;

    fn admin_menu() -> OpenGump {
        let mut layout = GumpLayout::new();
        layout.background(0, 0, 300, 270, 5054);
        layout.label(105, 14, 2100, "Admin");
        layout.button(30, 54, 4005, 4007, GumpButton::Reply, 0, ButtonId(13));
        layout.label(66, 56, 1153, "Populate Felucca");
        layout.check(30, 100, 210, 211, false, SwitchId(1));
        layout.radio(30, 130, 208, 209, false, SwitchId(2));
        layout.radio(30, 150, 208, 209, true, SwitchId(3));
        layout.text_entry(30, 180, 120, 20, 1153, 7, "Britain");
        let (string, lines) = layout.finish();
        OpenGump {
            key: GumpKey(0x2A),
            gump_id: GumpId(0x00AD_0001),
            at: GumpPoint::new(100, 100),
            elements: parse(string),
            lines: lines.to_vec(),
        }
    }

    /// A switch starts where the layout said it starts, and only the first
    /// time: re-seeding every frame would pull a checkbox back out from under
    /// the player's finger. A field is seeded the same way, from the line the
    /// layout pointed it at.
    #[test]
    fn a_window_is_seeded_once_and_then_belongs_to_the_player() {
        let gump = admin_menu();
        let mut dialogs = Dialogs::default();
        dialogs.sync(std::slice::from_ref(&gump));
        let sheet = &dialogs.by_dialog[&gump.gump_id];
        assert_eq!(sheet.switches.get(&RawSwitchId(1)), Some(&false));
        assert_eq!(
            sheet.entries.get(&TextEntryId::new(7)).map(String::as_str),
            Some("Britain"),
            "the field starts out holding the line the layout named"
        );

        dialogs.toggle(gump.gump_id, RawSwitchId(1));
        dialogs.sync(std::slice::from_ref(&gump));
        assert_eq!(
            dialogs.by_dialog[&gump.gump_id].switches.get(&RawSwitchId(1)),
            Some(&true),
            "and it stays where the player put it"
        );
    }

    /// A window the shard has taken away takes its state with it — and the
    /// mouse and the keyboard with that: a press held on a window that is gone
    /// would answer for whatever opened next under the same finger.
    #[test]
    fn a_closed_window_forgets_everything_including_the_finger_on_it() {
        let gump = admin_menu();
        let mut dialogs = Dialogs::default();
        dialogs.sync(std::slice::from_ref(&gump));
        dialogs.held = Some((gump.gump_id, Hit::Reply(RawButtonId(13))));
        dialogs.focus = Some((gump.gump_id, TextEntryId::new(7)));

        dialogs.sync(&[]);
        assert!(dialogs.by_dialog.is_empty());
        assert!(dialogs.held.is_none());
        assert!(!dialogs.typing());
    }

    /// A radio turns its neighbours off, and a checkbox does not. The whole
    /// difference between the two, and the client is what enforces it: the wire
    /// carries a set of ids and trusts that only one of a group is in it.
    #[test]
    fn a_radio_turns_its_group_off_and_a_checkbox_minds_its_own_business() {
        let gump = admin_menu();
        let mut dialogs = Dialogs::default();
        dialogs.sync(std::slice::from_ref(&gump));

        dialogs.choose(&gump, RawSwitchId(2));
        let sheet = &dialogs.by_dialog[&gump.gump_id];
        assert!(sheet.switches[&RawSwitchId(2)]);
        assert!(
            !sheet.switches[&RawSwitchId(3)],
            "the other radio went off, initial or not"
        );

        dialogs.toggle(gump.gump_id, RawSwitchId(1));
        let sheet = &dialogs.by_dialog[&gump.gump_id];
        assert!(sheet.switches[&RawSwitchId(1)]);
        assert!(
            sheet.switches[&RawSwitchId(2)],
            "a checkbox left the radios alone"
        );
    }

    /// What travels back. Three things worth pinning: only the switches that
    /// are *on* are sent, they are sent in a stable order, and what was typed
    /// goes with them under the id the layout gave the field.
    #[test]
    fn an_answer_carries_what_the_player_set_and_nothing_else() {
        let gump = admin_menu();
        let mut dialogs = Dialogs::default();
        dialogs.sync(std::slice::from_ref(&gump));
        dialogs.toggle(gump.gump_id, RawSwitchId(1));
        dialogs.choose(&gump, RawSwitchId(3));
        dialogs.focus = Some((gump.gump_id, TextEntryId::new(7)));
        assert!(dialogs.typed("!"));

        let reply = dialogs.reply(&gump, RawButtonId(13));
        assert_eq!(reply.gump_id, RawGumpId(0x00AD_0001));
        assert_eq!(reply.key, RawGumpKey(0x2A), "the key is echoed, not invented");
        assert_eq!(reply.button, RawButtonId(13));
        assert_eq!(reply.switches, vec![RawSwitchId(1), RawSwitchId(3)]);
        assert_eq!(reply.text_entries, vec![(7, "Britain!".to_owned())]);
    }

    /// Keys go to the field the player clicked into and nowhere else — and
    /// with nothing focused they are the world's, which is what lets a letter
    /// walk the character while a dialog is open.
    #[test]
    fn a_keystroke_needs_a_field_to_go_into() {
        let gump = admin_menu();
        let mut dialogs = Dialogs::default();
        dialogs.sync(std::slice::from_ref(&gump));
        assert!(!dialogs.typed("x"), "nothing focused, nothing taken");
        assert!(!dialogs.backspace());

        dialogs.focus = Some((gump.gump_id, TextEntryId::new(7)));
        assert!(dialogs.typed("x"));
        assert!(
            !dialogs.typed("\n"),
            "a control character is not a character a field takes"
        );
        assert!(dialogs.backspace());
        assert_eq!(
            dialogs.by_dialog[&gump.gump_id].entries[&TextEntryId::new(7)],
            "Britain"
        );
    }

    /// `{ noclose }` means the window cannot be walked away from: the right
    /// button gets no answer out of it, and the shard keeps waiting for one of
    /// its own buttons.
    #[test]
    fn a_noclose_window_is_not_dismissed_by_the_right_button() {
        let mut layout = GumpLayout::new();
        layout.no_close();
        layout.background(0, 0, 100, 100, 5054);
        let (string, lines) = layout.finish();
        let gump = OpenGump {
            key: GumpKey(1),
            gump_id: GumpId(2),
            at: GumpPoint::new(0, 0),
            elements: parse(string),
            lines: lines.to_vec(),
        };

        let mut dialogs = Dialogs::default();
        dialogs.sync(std::slice::from_ref(&gump));
        assert!(dialogs.dismiss(&gump).is_none());
    }
}
