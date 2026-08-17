//! The one place an input is offered to the client's own windows, and the one
//! place a pane's answer is carried out.
//!
//! This is the plan's `Panes::deliver`. It replaces the `||` chains in
//! `event_loop.rs`, whose single `bool` answered "this event is mine" and "ask
//! for a redraw" at the same time — see [`Response`] for the defect that cost.
//!
//! Three steps, in this order, and the order is the whole design:
//!
//! 1. **The manager's own gestures.** Moving a window and letting go of it
//!    belong to nobody's pane (decision 2). They run whether a pane takes the
//!    event or not.
//! 2. **The panes, top down, first `taken` wins.** Painter's order is z-order,
//!    so the last window in the list is the one drawn over the others and the
//!    first one offered an event.
//! 3. **The kinds that have not moved in yet.** The `App` methods in
//!    [`crate::own_windows`], reached only when no pane answered — so they are
//!    never a second opinion about the same click.
//!
//! Step 3 is scaffolding and dies with step 7 of the plan. Steps 1 and 2 are
//! the shape that stays.

use std::time::Instant;

use crate::app::App;
use crate::panes::{Button, Effect, Input, Modifiers, Pane, PaneCtx, PaneFrame, Response};
use crate::windows::{ItemDragTransaction, WindowSubject};

impl App {
    /// Offer one input to the window layer, and carry out whatever it asked
    /// for.
    ///
    /// The answer's [`out`](Response::out) is always empty: the effects a pane
    /// asked for have already been performed by the time this returns, and what
    /// is left is the two questions the caller has to act on — whether the
    /// event was the window layer's ([`taken`](Response::taken)), and whether
    /// the frame has to be drawn again ([`redraw`](Response::redraw)).
    ///
    /// The caller acts on `taken` by *not* doing whatever it would have done
    /// with the event: no map zoom under a pointer that is over a shop, no tile
    /// selected behind a bag that was only being raised.
    pub(crate) fn deliver(&mut self, input: Input) -> Response {
        let mut response = self.manager_gestures(input);
        if !response.taken {
            response.absorb(self.offer_to_panes(input));
        }
        if !response.taken {
            response.absorb(self.legacy_window_input(input));
        }
        response
    }

    /// The gestures that are the manager's and no pane's: the window being
    /// dragged follows the pointer, letting the button go ends that, and a
    /// press while the hand is full does nothing at all.
    ///
    /// Only the last of the three is `taken`. A move is not an exclusive event
    /// (see [`Input::Move`]), and the release that let go of a frame is also the
    /// release that drops a held item into the bag under it — swallowing it here
    /// would strand the hand.
    fn manager_gestures(&mut self, input: Input) -> Response {
        match input {
            Input::Move => {
                if self.drag_own_window() {
                    Response::stale()
                } else {
                    Response::ignored()
                }
            }
            Input::Release(Button::Left) => {
                self.windows.dragging = None;
                Response::ignored()
            }
            // **Decision 7's precondition, and the manager's because the hand
            // is.** Once a lift has gone to the shard this transaction *is* the
            // cursor: a second press is choosing a destination for the item
            // already on it, and it must not reach a window, which would answer
            // it by starting a second source. Ahead of the panes rather than
            // inside each of them, because a pane that forgot to ask is a pane
            // that quietly overwrites the hand.
            Input::Press(Button::Left)
                if self
                    .windows
                    .item_drag
                    .is_some_and(ItemDragTransaction::owns_cursor) =>
            {
                self.windows.dragging = None;
                Response::changed()
            }
            Input::Press(_) | Input::Release(Button::Right) | Input::Wheel(_) => Response::ignored(),
        }
    }

    /// Offer the input to every pane from the top down, and perform what the
    /// answers asked for.
    ///
    /// A pane that declines is walked past, and its `redraw` is still kept: a
    /// window can go stale without the event being its — a hover tint leaving
    /// one as the pointer crosses onto the window above it, say. The walk stops
    /// at the first pane that says `taken`, which is what "under this one" means
    /// in [`Response::taken`]'s own doc.
    ///
    /// # A located input stops at the window it is on
    ///
    /// The walk also stops *after* the window the pointer is over, for the two
    /// inputs that are somewhere: a press and a notch. Nothing below the window
    /// under the pointer may answer either, which is the rule every one of the
    /// legacy handlers opens with (`window_under_pointer`) and would otherwise
    /// be a rule six panes each had to remember. It matters most while the
    /// migration is half done: a kind that has not moved in yet declines
    /// everything, so without this a moved-in pane two windows down would
    /// happily take a click that landed on a bag drawn over it.
    ///
    /// A release and a move are not bounded. A release finishes a press
    /// wherever the pointer has got to since — a paperdoll's button has to come
    /// back up even if the finger slid off the window — and a move is offered to
    /// every window so that a tint left behind can be cleared.
    fn offer_to_panes(&mut self, input: Input) -> Response {
        // Asked first, because it is a `&self` method and reads the whole of
        // `App`: the z-order out of `own_windows` and last frame's pictures out
        // of `drawn_windows`. It is the one question a pane cannot answer about
        // itself — see [`PaneCtx::under_pointer`].
        let owner = self.window_under_pointer();
        let located = matches!(input, Input::Press(_) | Input::Wheel(_));
        // No world, no windows: `sync_own_windows` has already emptied the list,
        // and there is no authoritative picture to build a context out of.
        //
        // Field borrows rather than method calls from here down, so that the
        // context can hold the view, the files and the last frame's layouts
        // while the pane list is borrowed mutably beside them. They are
        // disjoint fields of `App`, which is what makes a readonly context and
        // a `&mut` pane possible in one loop at all.
        let Some(view) = self.world.authoritative.view.as_ref() else {
            return Response::ignored();
        };
        let resources = &self.resources;
        let drawn_windows = &self.windows.drawn_windows;
        let cursor = self.input.pointer_gump;
        let modifiers = Modifiers {
            shift: self.input.shift_held,
            ctrl: self.input.ctrl_held,
        };
        let now = Instant::now();
        let hand = self.windows.item_drag.and_then(ItemDragTransaction::drag);

        let mut response = Response::ignored();
        // Collected rather than performed inside the loop: an effect needs
        // `&mut self`, and the loop above holds half of `self` borrowed.
        let mut asked: Vec<(WindowSubject, Effect)> = Vec::new();
        for open in self.windows.own_windows.iter_mut().rev() {
            let under_pointer = owner == Some(open.subject);
            let ctx = PaneCtx {
                frame: PaneFrame {
                    view,
                    resources,
                    at: open.at,
                    cursor,
                    hand,
                },
                drawn: drawn_windows
                    .iter()
                    .find(|(subject, _)| *subject == open.subject)
                    .map(|(_, drawn)| drawn),
                under_pointer,
                modifiers,
                now,
            };
            let answer = open.pane.handle(input, &ctx);
            let taken = answer.taken;
            let subject = open.subject;
            response.taken |= taken;
            response.redraw |= answer.redraw;
            asked.extend(answer.out.into_iter().map(|effect| (subject, effect)));
            if taken || (located && under_pointer) {
                break;
            }
        }
        for (subject, effect) in asked {
            self.perform(subject, effect);
        }
        response
    }

    /// Carry out one thing a pane asked for, on the window it asked from.
    ///
    /// The manager's half of decision 5: a pane names what it wants and this is
    /// the only code that does it, which is why no pane holds a
    /// [`Link`](crate::link::Link) or writes its own position.
    fn perform(&mut self, subject: WindowSubject, effect: Effect) {
        match effect {
            Effect::Raise => self.raise_window(subject),
            Effect::Close => {
                self.close_window(subject);
            }
            Effect::Grab(grab) => self.windows.dragging = Some((subject, grab)),
            // A closed channel is not an error here, the same as everywhere else
            // this client sends: the shard thread has already said why it went.
            Effect::Net(action) => {
                if let Some(link) = self.world.shard.link() {
                    link.act(action);
                }
            }
            // One arm for both local kinds, and idempotent — which is the point:
            // pressing Skills twice must not scroll the sheet back to the top.
            // See `open_local_window`, which leaves a window it finds alone.
            Effect::Open(local) => {
                crate::windows::open_local_window(&mut self.windows.own_windows, local.subject());
            }
        }
    }

    /// The six kinds' input as it is answered today, behind the one type the
    /// router speaks.
    ///
    /// Every chain here is the chain that was in `event_loop.rs`, in its order,
    /// with one difference: what it answers is a [`Response`] and not a `bool`,
    /// so the caller can tell "the window took it" from "the frame is stale".
    /// The conflation the wheel defect was made of now lives in exactly one
    /// function instead of five call sites — and this function is deleted by the
    /// plan's step 7, one arm at a time, as each kind's pane takes its input
    /// over.
    fn legacy_window_input(&mut self, input: Input) -> Response {
        match input {
            Input::Press(Button::Left) => {
                if self.press_on_own_window() {
                    Response::changed()
                } else {
                    Response::ignored()
                }
            }
            // Three questions on the way up, in this order: a held item is
            // committed to whatever is under the pointer, a press that never
            // became a drag is dropped, and a button that was pushed down is let
            // back up.
            Input::Release(Button::Left) => {
                if self.release_container_item()
                    || self.release_container_press()
                    || self.release_on_own_window()
                {
                    Response::changed()
                } else {
                    Response::ignored()
                }
            }
            // The right button over a window closes it — the reference client's
            // own gesture. `cancel_target_cursor` is asked before this, in the
            // event loop: a targeting cursor is put out before a window is taken
            // down under it.
            Input::Press(Button::Right) => {
                if self.close_window_under_pointer() {
                    Response::changed()
                } else {
                    Response::ignored()
                }
            }
            Input::Release(Button::Right) => Response::ignored(),
            // All of these run, and none of them is exclusive: an item leaving
            // a bag and two hover tints are three different windows' business
            // and the pointer moved past all of them. The thumb that used to be
            // the fourth is `SkillsPane`'s own, offered above.
            Input::Move => {
                let mut stale = self.drag_container_item();
                stale |= self.hover_container_item();
                stale |= self.hover_paperdoll_item();
                if stale {
                    Response::stale()
                } else {
                    Response::ignored()
                }
            }
            // **Empty, and that is the plan's own milestone.** This arm was
            // `scroll_skills() || scroll_vendor() || zoom()` — the `||` chain
            // the whole of `docs/window_components.md` was written about, whose
            // one `bool` answered "the notch was taken" and "the list moved" at
            // the same time. Both windows own their wheel now, each answering
            // the two questions as two fields, and the zoom is the caller's
            // business, reached only when nothing above said the notch was its.
            // No kind that is left has a wheel at all.
            Input::Wheel(_) => Response::ignored(),
        }
    }
}
