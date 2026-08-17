//! A body's paperdoll as a component: the fifth window kind to own its state,
//! its layout and its input.
//!
//! Step 5 of `docs/window_components.md`, and the first kind whose effects are
//! mostly [`Effect::Open`] and [`Effect::Net`]: nearly every control on the
//! frame asks for another window — the shard's, through a packet, or one of
//! the two local kinds, through [`Effect::Open`], which had been waiting under
//! `#[expect(dead_code)]` since step 2 for exactly this pane. The enum the
//! buttons come from says so in its own doc: [`paperdoll::DollButton`] "names
//! windows rather than actions".
//!
//! # What this pane does not own: the hand
//!
//! A press on a worn item of this client's own body starts an item transfer,
//! and the transfer machinery — the press that may become a lift, the slop
//! that decides, the split prompt — is the hand's, which decision 7 gives to
//! the manager and step 6 moves. This pane **declines** that one press
//! (`Response::ignored`), so the legacy chain still answers it; every other
//! press on the window is this pane's. The two halves meet at step 6, where
//! the container — "the one the hand runs through" — moves in.

use std::time::Instant;

use openshard_client_net::action::Outgoing;
use openshard_client_render::gump::{self as gump_art, GumpArt, GumpPixel};
use openshard_client_render::mobiles::EquipmentLayer;
use openshard_client_render::paperdoll;
use openshard_protocol::mobile::Equipment;
use openshard_protocol::serial::Serial;
use openshard_protocol::speech::Font;
use openshard_protocol::wire::{Hue, Layer};

use crate::DOUBLE_CLICK;
use crate::crowd;
use crate::panes::{Button, Effect, Input, Line, LocalWindow, Pane, PaneCtx, PaneFrame, Response};
use crate::windows::{DragOrigin, Drawn};

/// One open paperdoll window.
#[derive(Debug)]
pub struct PaperdollPane {
    /// Whose body this is a window over.
    ///
    /// A shop's serial, one kind over: the subject carries a key and the pane
    /// is handed it when the window opens, because whether this doll is the
    /// player's own — which decides the frame, the buttons and what a press on
    /// a worn item means — is a comparison against this.
    mobile: Serial,
    /// The button the mouse went down on.
    ///
    /// [`DialogPane::held`](crate::panes::dialog)'s counterpart, and the same
    /// three things it buys: the pressed picture is drawn while the finger is
    /// down, the release acts only if the pointer is still on the *same*
    /// button, and a press on a button does not also drag the frame under it.
    ///
    /// A [`paperdoll::DollButton`] and not a picture index: the doll is laid
    /// out afresh every frame — a hat coming off changes how many pictures are
    /// in front of the buttons — so an index taken at the press would name a
    /// different picture by the time the button comes up. The button itself is
    /// stable.
    held: Option<paperdoll::DollButton>,
    /// The last completed click on one of the three scrolls, and when.
    ///
    /// The scrolls answer a *double* click where the seven buttons answer a
    /// single one (`GumpPic.MouseDoubleClick` against `Button`'s
    /// `OnButtonClick`), and this is that pair. Separate from
    /// [`last_click`](crate::input::Input::last_click), which pairs clicks on
    /// the *world*: a click on a window never reaches that one. It used to be
    /// `Windows::last_scroll`, keyed by subject; the subject key is gone
    /// because the state is this window's own now, so two clicks on two dolls
    /// cannot pair **by construction** rather than by a comparison.
    last_scroll: Option<(Instant, paperdoll::DollButton)>,
    /// The worn layer under the pointer, tinted on the next frame.
    ///
    /// Remembered so that the *move* that changes it can say
    /// [`Response::stale`]: the tint is decided in the layout, and what draws
    /// a frame is the animation clock, not the move — a picture that depends
    /// on the pointer owes the ask itself. `Windows::hovered_equipment` kept
    /// this for every doll at once, keyed by serial; per-pane, the key is
    /// gone.
    hovered: Option<Layer>,
    /// The cursor, with the hand full, is over this doll — and the doll is
    /// this client's own body, so the held wearable is drawn on it
    /// translucently as a preview.
    ///
    /// Only the *whereabouts* are remembered; the item itself is read out of
    /// [`PaneFrame::hand`] at layout time, so a hand that has emptied takes
    /// the preview with it on the next frame whether or not the pointer moves
    /// again. `Windows::preview_equipment` kept a copy of the item instead,
    /// which could outlive the drag it described.
    hand_over: bool,
}

/// A paperdoll, laid out for one frame: the doll, and what is written over it.
///
/// [`dialog::Window`](crate::panes::dialog::Window)'s shape for the same
/// reason: the name on the plate comes out of the view and the hover label out
/// of `tiledata`, neither of which `client/render` has heard of, so the text
/// is resolved here — in the pane that owns the hover — and the pass draws it
/// out of what the layout produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Window {
    /// The frame, its furniture and the doll — everything the pointer is
    /// tested against.
    pub doll: paperdoll::Doll,
    /// The text over them: the name on the plate, and the worn item the
    /// pointer is resting on.
    pub lines: Vec<Line>,
}

/// Whether a click on `button` at `now` is the second of a pair.
///
/// A pair is two clicks on one scroll of one window. The window half is the
/// pane itself now — `last` never held another window's click — so what is
/// left to compare is the picture and the clock: the profile scroll and the
/// party manifest sit fourteen pixels apart on the same frame, and a rule that
/// only looked at the time would open a profile because the hand slipped
/// between two clicks.
fn pairs(
    last: Option<(Instant, paperdoll::DollButton)>,
    now: Instant,
    button: paperdoll::DollButton,
) -> bool {
    last.is_some_and(|(at, picture)| picture == button && now.duration_since(at) <= DOUBLE_CLICK)
}

/// What one completed click on a doll control asks for.
///
/// The whole of what was `App::doll_clicked`, as effects instead of link
/// calls — and a free function of what it actually reads, so the mapping can
/// be pinned without a view. Always [`Response::changed`]: the button was
/// drawn pressed and has to come back up, whether or not its arm asks for
/// anything.
fn button_effects(
    button: paperdoll::DollButton,
    mobile: Serial,
    own: bool,
    war: bool,
    backpack: Option<Serial>,
    paired: bool,
) -> Response {
    let answer = Response::changed();
    match button {
        paperdoll::DollButton::WarMode => answer.with(Effect::Net(Outgoing::WarMode(!war))),
        paperdoll::DollButton::LogOut => answer.with(Effect::Net(Outgoing::LogOut)),
        paperdoll::DollButton::Quests => answer.with(Effect::Net(Outgoing::QuestLog)),
        paperdoll::DollButton::Guild => answer.with(Effect::Net(Outgoing::GuildMenu)),
        // Status packets describe this connection's player, so a stranger's
        // Status control must not open a misleading local window — and it
        // asks the shard nothing either, which is what the old handler did.
        paperdoll::DollButton::Status if own => answer
            .with(Effect::Net(Outgoing::Status(mobile)))
            .with(Effect::Open(LocalWindow::Status)),
        // Window visibility is local intent; the `0x3A` merely refreshes
        // data.
        paperdoll::DollButton::Skills if own => answer
            .with(Effect::Net(Outgoing::Skills(mobile)))
            .with(Effect::Open(LocalWindow::Skills)),
        paperdoll::DollButton::Virtue if paired => answer.with(Effect::Net(Outgoing::Virtue(mobile))),
        // Opening the backpack is a use of the worn bag, which needs its
        // serial — a body wearing none has nothing to open.
        paperdoll::DollButton::Backpack if paired => match backpack {
            Some(serial) => answer.with(Effect::Net(Outgoing::Use(serial))),
            None => answer,
        },
        // Help, Options, and the first click of every scroll pair: drawn,
        // pressed, released — and wired to nothing yet, exactly as they were
        // before this pane existed.
        _ => answer,
    }
}

impl PaperdollPane {
    /// The pane for the doll of `mobile`.
    pub const fn new(mobile: Serial) -> Self {
        Self {
            mobile,
            held: None,
            last_scroll: None,
            hovered: None,
            hand_over: false,
        }
    }

    /// Whether this doll is the player's own body.
    fn own(&self, frame: &PaneFrame<'_>) -> bool {
        frame.view.player.serial == self.mobile
    }

    /// What the mobile is wearing, as the wire said it — the serials and wire
    /// graphics a click and a label need, as opposed to the `AnimID`s the
    /// doll is drawn from.
    fn equipment<'view>(&self, frame: &PaneFrame<'view>) -> &'view [Equipment] {
        match self.own(frame) {
            true => &frame.view.player.equipment,
            false => frame
                .view
                .mobiles
                .get(&self.mobile)
                .map_or(&[] as &[Equipment], |mobile| mobile.equipment.as_slice()),
        }
    }

    /// Whether this was the second click on the same scroll, and the
    /// bookkeeping either way: a completed pair arms nothing, a first click
    /// arms the next.
    fn scroll_paired(&mut self, button: paperdoll::DollButton, now: Instant) -> bool {
        let paired = pairs(self.last_scroll, now, button);
        self.last_scroll = (!paired).then_some((now, button));
        paired
    }

    /// A left press somewhere in the window, against the layout it was drawn
    /// as.
    ///
    /// Every arm that answers raises — a press on a window is what puts it on
    /// top. A button takes the press away from the drag: the column runs down
    /// the middle of the frame, and pressing one used to pick the whole doll
    /// up. The press this pane does *not* answer is the one on a worn item of
    /// this client's own body — that is the hand's (see the module docs).
    fn press(&mut self, window: &Window, ctx: &PaneCtx<'_>) -> Response {
        let raised = Response::changed().with(Effect::Raise);
        if let Some(index) = gump_art::pick(
            &window.doll.pictures,
            ctx.frame.cursor,
            &ctx.frame.resources.gump_atlas,
        ) {
            if let Some(button) = window.doll.hits.get(&index) {
                self.held = Some(*button);
                return raised;
            }
            // A worn item on our own body: the press that starts an item
            // transfer, declined so the legacy chain answers it until step 6.
            // A stranger's worn item is not liftable and falls through to the
            // grab below, as it always has.
            if window.doll.equipment_hits.contains_key(&index) && self.own(&ctx.frame) {
                return Response::ignored();
            }
        }
        // The frame, the body, a stranger's shirt: nothing that answers, so
        // the press moves the window.
        raised.with(Effect::Grab(GumpPixel::new(
            ctx.frame.cursor.x - ctx.frame.at.x,
            ctx.frame.cursor.y - ctx.frame.at.y,
        )))
    }

    /// The release that finishes a press on one of this window's buttons.
    ///
    /// The pointer has to still be on the button it went down on: a press
    /// dragged off its button is a press taken back, in this client as in
    /// every other. Either way the answer is [`Response::changed`] once a
    /// press is being held — the button was drawn pressed and has to come
    /// back up.
    fn release(&mut self, ctx: &PaneCtx<'_>) -> Response {
        let Some(held) = self.held.take() else {
            return Response::ignored();
        };
        let Some(Drawn::Paperdoll(window)) = ctx.drawn else {
            return Response::changed();
        };
        let on = gump_art::pick(
            &window.doll.pictures,
            ctx.frame.cursor,
            &ctx.frame.resources.gump_atlas,
        )
        .and_then(|index| window.doll.hits.get(&index).copied());
        if on != Some(held) {
            return Response::changed();
        }
        self.clicked(held, ctx)
    }

    /// What one completed click means: gather what the effects are about —
    /// whose doll, what stance, which bag — and ask [`button_effects`].
    fn clicked(&mut self, button: paperdoll::DollButton, ctx: &PaneCtx<'_>) -> Response {
        let own = self.own(&ctx.frame);
        let backpack = self
            .equipment(&ctx.frame)
            .iter()
            .find(|item| item.layer == Layer::BACKPACK)
            .map(|item| item.serial);
        let paired = match button {
            paperdoll::DollButton::Profile
            | paperdoll::DollButton::Party
            | paperdoll::DollButton::Virtue
            | paperdoll::DollButton::Backpack => self.scroll_paired(button, ctx.now),
            _ => false,
        };
        button_effects(
            button,
            self.mobile,
            own,
            ctx.frame.view.player.war,
            backpack,
            paired,
        )
    }

    /// Follow the pointer with the two answers the layout will read: which
    /// worn layer is under it, and whether a full hand is over this doll.
    ///
    /// Not `taken` — a move is not an exclusive event — and
    /// [`Response::stale`] only when an answer changed: this is the Backlog's
    /// "a picture that depends on the pointer owes a frame", paid by
    /// remembering what the picture was.
    fn hover(&mut self, ctx: &PaneCtx<'_>) -> Response {
        let hovered = match (ctx.under_pointer, ctx.drawn) {
            (true, Some(Drawn::Paperdoll(window))) => gump_art::pick(
                &window.doll.pictures,
                ctx.frame.cursor,
                &ctx.frame.resources.gump_atlas,
            )
            .and_then(|index| window.doll.equipment_hits.get(&index).copied()),
            _ => None,
        };
        let hand_over = ctx.under_pointer && self.own(&ctx.frame) && ctx.frame.hand.is_some();
        if self.hovered == hovered && self.hand_over == hand_over {
            return Response::ignored();
        }
        self.hovered = hovered;
        self.hand_over = hand_over;
        Response::stale()
    }

    /// The text over the doll: the name on the plate, and the worn item the
    /// pointer is resting on, named at the cursor.
    ///
    /// The name comes out of the view — the `0x88` carries it and nothing
    /// else does — and the hover label out of `tiledata`, which is why both
    /// are resolved here rather than in the render crate's layout.
    fn lines(&self, frame: &PaneFrame<'_>) -> Vec<Line> {
        let mut lines = Vec::new();
        if let Some(doll) = frame.view.paperdolls.get(&self.mobile) {
            let name = paperdoll::name(&doll.name, frame.at);
            lines.push(Line {
                at: name.at,
                font: name.font,
                hue: name.hue,
                clip: name.clip,
                text: name.text.to_owned(),
            });
        }
        if let Some(layer) = self.hovered {
            if let Some(item) = self.equipment(frame).iter().find(|item| item.layer == layer) {
                lines.push(Line {
                    // The same step down-right the container hover uses, so
                    // the two hovers read as one behaviour.
                    at: frame.cursor.offset(GumpPixel::new(14, 18)),
                    font: Font(1),
                    hue: Hue::LABEL,
                    clip: None,
                    text: frame.resources.tiledata.static_tile(item.graphic.0).name.clone(),
                });
            }
        }
        lines
    }
}

impl Pane for PaperdollPane {
    /// Nothing of its own, and it could not be otherwise: which picture a
    /// worn item is, is the answer to [`paperdoll::gump_of`], so asking for
    /// the list *is* laying the window out — [`paperdoll::art_of`]'s own doc.
    /// The sweep over every laid-out window at the end of
    /// `render_passes::draw_gump_windows` packs what the layout produced.
    fn art(&self, _frame: &PaneFrame<'_>) -> Vec<GumpArt> {
        Vec::new()
    }

    /// The frame, the body, every worn layer over it, and the text over the
    /// lot.
    ///
    /// `None` only for a client with no gump art at all, in which case no
    /// window is drawn anyway. A mobile the view has never revealed still
    /// draws the *frame* — the window is open, and a window with no picture
    /// is one the pointer cannot find and the player cannot close; the doll
    /// appears on the frame the `0x77` arrives.
    fn layout(&self, frame: &PaneFrame<'_>) -> Option<Drawn> {
        let files = frame.resources.gumps.as_ref()?;
        let view = frame.view;
        let own = self.own(frame);
        let body = match own {
            true => Some((view.player.body, view.player.hue)),
            false => view
                .mobiles
                .get(&self.mobile)
                .map(|mobile| (mobile.body, mobile.hue)),
        };
        // The `0x88` carries no equipment — see `WorldView::paperdolls` — so
        // it is read off the body the window names, resolved once through
        // tiledata for the sprite and the doll alike (`crowd::worn`). An item
        // the hand has lifted off this body is subtracted until the shard
        // answers, exactly as its bag subtracts a lifted icon.
        let equipment: Vec<EquipmentLayer> = crowd::worn(self.equipment(frame), &frame.resources.tiledata)
            .into_iter()
            .filter(|item| {
                frame.hand.is_none_or(|drag| {
                    drag.origin
                        != DragOrigin::Equipment {
                            mobile: self.mobile,
                            layer: item.layer,
                        }
                })
            })
            .collect();
        let wearer = body.map(|(body, hue)| paperdoll::Wearer {
            body,
            hue,
            equipment: &equipment,
        });
        // The stance, off the player and not off the `0x88` the window opened
        // on: a `0x72` moves it while that packet stands still, and the
        // toggle is the one picture on the frame that has to follow.
        let whose = match own {
            true => paperdoll::Whose::Own { war: view.player.war },
            false => paperdoll::Whose::Another,
        };
        // The held wearable, projected onto our own doll while the hand is
        // over it. The item is the hand's, read now rather than remembered at
        // the move (see `hand_over`), and only a layer the body has empty
        // takes a preview.
        let preview = match self.hand_over {
            true => frame.hand.and_then(|drag| {
                let tile = frame.resources.tiledata.static_tile(drag.item.graphic.0);
                let layer = Layer(tile.layer);
                (own && layer.0 > 0 && layer.0 <= 25 && !equipment.iter().any(|worn| worn.layer == layer))
                    .then_some(EquipmentLayer {
                        graphic: tile.anim_id,
                        hue: drag.item.hue,
                        layer,
                    })
            }),
            false => None,
        };
        let doll = paperdoll::window(
            wearer.as_ref(),
            whose,
            self.held,
            self.hovered,
            preview,
            &frame.resources.equip_conv,
            files,
            frame.at,
        );
        let lines = self.lines(frame);
        Some(Drawn::Paperdoll(Window { doll, lines }))
    }

    /// A press on the window's furniture, the release that completes it, and
    /// the move that keeps the hover honest.
    ///
    /// The press is *located*, so it begins by asking whether this window is
    /// the one the pointer is on — see [`PaneCtx::under_pointer`]. The
    /// release is not: it finishes a press this pane started, wherever the
    /// pointer has got to. A move is offered to every window, which is what
    /// lets a tint be cleared on the doll the pointer just left.
    ///
    /// **No wheel.** Nothing on a doll scrolls, so a notch over one goes past
    /// it to the camera, exactly as it did. The right button is the manager's
    /// close, and there is nothing here to type into.
    fn handle(&mut self, input: Input, ctx: &PaneCtx<'_>) -> Response {
        match input {
            Input::Press(Button::Left) => {
                if !ctx.under_pointer {
                    return Response::ignored();
                }
                // Never drawn, so there is nothing to hit-test against and no
                // pixels the player can have meant. The window is still
                // raised and moved by the manager's own path.
                let Some(Drawn::Paperdoll(window)) = ctx.drawn else {
                    return Response::ignored();
                };
                self.press(window, ctx)
            }
            Input::Release(Button::Left) => self.release(ctx),
            Input::Move => self.hover(ctx),
            Input::Wheel(_) | Input::Press(Button::Right) | Input::Release(Button::Right) | Input::Key(_) => {
                Response::ignored()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial(raw: u32) -> Serial {
        Serial::new(raw).expect("a mobile serial")
    }

    /// The pairing rule, minus the half the shape now answers: two clicks on
    /// two different dolls cannot pair because each pane keeps its own
    /// `last_scroll` — there is no cross-window state left to compare.
    #[test]
    fn two_clicks_on_one_scroll_are_a_pair_and_two_scrolls_are_not() {
        let first = Instant::now();
        let soon = first + DOUBLE_CLICK / 2;
        let profile = paperdoll::DollButton::Profile;

        assert!(
            !pairs(None, first, profile),
            "the first click of all has nothing to pair with"
        );
        assert!(pairs(Some((first, profile)), soon, profile));
        assert!(
            !pairs(Some((first, profile)), soon, paperdoll::DollButton::Party),
            "the manifest sits fourteen pixels along, and is not the same scroll"
        );
        assert!(
            !pairs(
                Some((first, profile)),
                first + DOUBLE_CLICK + std::time::Duration::from_millis(1),
                profile
            ),
            "and a click a breath too late is a first click"
        );
    }

    /// A completed pair disarms the state and a first click arms it, so three
    /// quick clicks are a pair and a single, not a pair and a half.
    #[test]
    fn a_completed_pair_starts_over() {
        let mut pane = PaperdollPane::new(serial(0x2A));
        let first = Instant::now();
        let step = DOUBLE_CLICK / 4;
        let virtue = paperdoll::DollButton::Virtue;

        assert!(!pane.scroll_paired(virtue, first), "armed");
        assert!(pane.scroll_paired(virtue, first + step), "paired");
        assert!(
            !pane.scroll_paired(virtue, first + step * 2),
            "the third click starts a new pair rather than riding the old one"
        );
    }

    /// The buttons, as the packets and windows they ask for. WarMode toggles
    /// the stance it was drawn in, and the two window buttons ask the shard
    /// for fresh numbers *and* open the local window, in that order.
    #[test]
    fn a_button_asks_for_the_window_it_is_named_after() {
        let me = serial(0x2A);

        let answer = button_effects(paperdoll::DollButton::WarMode, me, true, false, None, false);
        assert!(matches!(
            answer.out.as_slice(),
            [Effect::Net(Outgoing::WarMode(true))]
        ));
        let answer = button_effects(paperdoll::DollButton::WarMode, me, true, true, None, false);
        assert!(
            matches!(answer.out.as_slice(), [Effect::Net(Outgoing::WarMode(false))]),
            "the toggle asks for the stance it is not in"
        );

        let answer = button_effects(paperdoll::DollButton::Skills, me, true, false, None, false);
        assert!(matches!(
            answer.out.as_slice(),
            [
                Effect::Net(Outgoing::Skills(who)),
                Effect::Open(LocalWindow::Skills)
            ] if *who == me
        ));
        let answer = button_effects(paperdoll::DollButton::Status, me, true, false, None, false);
        assert!(matches!(
            answer.out.as_slice(),
            [
                Effect::Net(Outgoing::Status(who)),
                Effect::Open(LocalWindow::Status)
            ] if *who == me
        ));

        let answer = button_effects(paperdoll::DollButton::LogOut, me, true, false, None, false);
        assert!(matches!(answer.out.as_slice(), [Effect::Net(Outgoing::LogOut)]));
        let answer = button_effects(paperdoll::DollButton::Quests, me, true, false, None, false);
        assert!(matches!(answer.out.as_slice(), [Effect::Net(Outgoing::QuestLog)]));
        let answer = button_effects(paperdoll::DollButton::Guild, me, true, false, None, false);
        assert!(matches!(
            answer.out.as_slice(),
            [Effect::Net(Outgoing::GuildMenu)]
        ));
    }

    /// A stranger's Status control asks for nothing at all: the packets it
    /// would send describe this connection's player, so both the request and
    /// the local window would be about the wrong body.
    #[test]
    fn a_strangers_status_button_neither_asks_nor_opens() {
        let stranger = serial(0xFF);
        let answer = button_effects(paperdoll::DollButton::Status, stranger, false, false, None, false);
        assert!(answer.redraw, "the button still comes back up");
        assert!(answer.out.is_empty(), "and nothing travels");
        let answer = button_effects(paperdoll::DollButton::Skills, stranger, false, false, None, false);
        assert!(answer.out.is_empty());
    }

    /// The scrolls act only on the second click of a pair: the first click
    /// asks for nothing, the paired one asks for the virtue menu or opens the
    /// worn backpack — and a body wearing no backpack has nothing to open.
    #[test]
    fn a_scroll_acts_on_the_paired_click_and_not_the_first() {
        let me = serial(0x2A);
        let pack = serial(0x4000_0001);

        let answer = button_effects(paperdoll::DollButton::Virtue, me, true, false, None, false);
        assert!(answer.out.is_empty(), "the first click only arms the pair");
        let answer = button_effects(paperdoll::DollButton::Virtue, me, true, false, None, true);
        assert!(matches!(
            answer.out.as_slice(),
            [Effect::Net(Outgoing::Virtue(who))] if *who == me
        ));

        let answer = button_effects(paperdoll::DollButton::Backpack, me, true, false, Some(pack), true);
        assert!(matches!(
            answer.out.as_slice(),
            [Effect::Net(Outgoing::Use(bag))] if *bag == pack
        ));
        let answer = button_effects(paperdoll::DollButton::Backpack, me, true, false, None, true);
        assert!(answer.out.is_empty(), "no worn backpack, nothing to use");
    }
}
