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

use openshard_client_render::{
    container,
    gump::{self as gump_art, GumpPixel},
};
use openshard_protocol::containers::ContainedItem;
use openshard_protocol::gump::GumpId;
use openshard_protocol::gump::GumpPoint;
use openshard_protocol::mobile::Equipment;
use openshard_protocol::serial::Serial;
use openshard_protocol::speech::TalkMode;
use openshard_protocol::wire::{Graphic, Layer};

use crate::app::App;
use crate::windows::{Drawn, WindowSubject};
use crate::{DOUBLE_CLICK, chat, link};

mod paperdoll;
mod sync;

const MAX_STACK: u16 = 60_000;
const GOLD_GRAPHIC: Graphic = Graphic(0x0EED);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct StackStep {
    source: Serial,
    target: Serial,
    amount: u16,
}

fn split_amount(total: u16, requested: u16) -> Option<u16> {
    (total > 1).then(|| requested.clamp(1, total - 1))
}

/// Pick one safe merge for this pass. Whole donor piles are preferred when
/// they fit; a partial donor is used only to finish a target at 60,000. That
/// keeps serial churn low, while recomputing after every server refresh makes
/// split remainders available to the following pass.
fn next_stack_step(items: &[ContainedItem]) -> Option<StackStep> {
    for (target_index, target) in items.iter().enumerate() {
        let room = MAX_STACK.saturating_sub(target.amount.0);
        if room == 0 {
            continue;
        }
        let matches = |item: &&ContainedItem| {
            item.graphic == target.graphic
                && item.hue == target.hue
                && (target.graphic == GOLD_GRAPHIC || target.amount.0 > 1 || item.amount.0 > 1)
        };
        let donors: Vec<_> = items[target_index + 1..].iter().filter(matches).collect();
        let Some(source) = donors
            .iter()
            .rev()
            .find(|item| item.amount.0 <= room)
            .copied()
            .or_else(|| donors.last().copied())
        else {
            continue;
        };
        return Some(StackStep {
            source: source.serial,
            target: target.serial,
            amount: room.min(source.amount.0),
        });
    }
    None
}

impl App {
    fn stack_all_button_under_pointer(&self) -> Option<Serial> {
        let view = self.world.authoritative.view.as_ref()?;
        let backpack = view
            .player
            .equipment
            .iter()
            .find(|item| item.layer == Layer::BACKPACK)
            .map(|item| item.serial)?;
        for open in self.windows.own_windows.iter().rev() {
            if let WindowSubject::Container(serial) = open.subject {
                if serial == backpack && !view.vendor_buys.contains_key(&serial) {
                    if let Some(gump) = view.containers.get(&serial) {
                        if container::stack_all_button(&self.resources.gump_atlas, *gump, open.at)
                            .is_some_and(|button| button.contains(self.input.pointer_gump))
                        {
                            return Some(serial);
                        }
                    }
                }
            }
            if self.window_under_pointer() == Some(open.subject) {
                return None;
            }
        }
        None
    }

    /// The visible loot action under the pointer, if it is not covered by a
    /// higher window. The player's own backpack and vendor catalogues never
    /// offer it: neither is loot to sweep into the backpack.
    fn take_all_button_under_pointer(&self) -> Option<Serial> {
        let view = self.world.authoritative.view.as_ref()?;
        let backpack = view
            .player
            .equipment
            .iter()
            .find(|item| item.layer == Layer::BACKPACK)
            .map(|item| item.serial);
        for open in self.windows.own_windows.iter().rev() {
            if let WindowSubject::Container(serial) = open.subject {
                if Some(serial) != backpack && !view.vendor_buys.contains_key(&serial) {
                    if let Some(gump) = view.containers.get(&serial) {
                        if container::take_all_button(&self.resources.gump_atlas, *gump, open.at)
                            .is_some_and(|button| button.contains(self.input.pointer_gump))
                        {
                            return Some(serial);
                        }
                    }
                }
            }
            // A real window above the action owns this press; do not let a
            // control on a covered chest answer through its pixels.
            if self.window_under_pointer() == Some(open.subject) {
                return None;
            }
        }
        None
    }

    /// Move every currently listed item from `container` into the player's
    /// backpack. Each lift/drop pair stays on the ordinary, server-authoritative
    /// drag path, preserving reach, ownership and weight checks.
    fn take_all_from_container(&mut self, container: Serial) {
        let Some(view) = self.world.authoritative.view.as_ref() else {
            return;
        };
        let Some(backpack) = view
            .player
            .equipment
            .iter()
            .find(|item| item.layer == Layer::BACKPACK)
            .map(|item| item.serial)
        else {
            return;
        };
        if container == backpack || view.vendor_buys.contains_key(&container) {
            return;
        }
        let items = view.contents.get(&container).cloned().unwrap_or_default();
        let Some(link) = self.world.shard.link() else {
            return;
        };
        for (index, item) in items.into_iter().enumerate() {
            // Keep the dropped icons apart. The backpack remains authoritative
            // about their final grid slots and any stack merge.
            let column = (index % 6) as i32;
            let row = (index / 6) as i32;
            link.pick_up_item(item.serial, item.amount);
            link.drop_into(
                item.serial,
                backpack,
                GumpPoint::new(20 + column * 18, 20 + row * 18),
            );
        }
    }

    fn start_stack_pass(&mut self, container: Serial) {
        if self.windows.item_drag.is_some() {
            return;
        }
        self.windows.stack_pass = Some(crate::windows::StackPass {
            container,
            awaiting: None,
        });
        self.advance_stack_pass();
    }

    /// Send one merge from the latest authoritative container snapshot.
    /// A refresh follows every drag because a successful merge consumes the
    /// source serial without echoing a Remove packet to the client that lifted
    /// it. The next pass therefore never plans against a stale split remainder.
    pub(crate) fn advance_stack_pass(&mut self) {
        const RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
        let Some(pass) = self.windows.stack_pass.as_ref() else {
            return;
        };
        if self.windows.item_drag.is_some() {
            self.windows.stack_pass = None;
            return;
        }
        let container_serial = pass.container;
        let Some(items) = self
            .world
            .authoritative
            .view
            .as_ref()
            .and_then(|view| view.contents.get(&container_serial))
        else {
            self.windows.stack_pass = None;
            return;
        };
        if let Some((source, sent_at)) = &pass.awaiting {
            if items.iter().any(|item| item.serial == *source) {
                if sent_at.elapsed() >= RESPONSE_TIMEOUT {
                    self.windows.stack_pass = None;
                }
                return;
            }
        }
        let Some(step) = next_stack_step(items) else {
            self.windows.stack_pass = None;
            return;
        };
        let Some(link) = self.world.shard.link() else {
            self.windows.stack_pass = None;
            return;
        };
        link.pick_up_item(step.source, openshard_protocol::items::ItemAmount(step.amount));
        link.drop_onto_item(step.source, step.target);
        // Reopening requests a complete 0x3C list and repairs the intentionally
        // suppressed Remove-to-lifter packet before the following pass.
        link.use_object(container_serial);
        if let Some(pass) = self.windows.stack_pass.as_mut() {
            pass.awaiting = Some((step.source, Instant::now()));
        }
    }

    /// Remember a world item's press so a following pointer move can lift it.
    /// A plain click remains available to the normal selection/double-click
    /// use path in the event loop.
    pub(crate) fn press_world_item(&mut self) -> bool {
        let Some(serial) = self.picking.on_item else {
            return false;
        };
        let Some(item) = self
            .world
            .authoritative
            .view
            .as_ref()
            .and_then(|view| view.items.get(&serial))
        else {
            return false;
        };
        let openshard_protocol::items::WorldItemPayload::Stack(amount) = item.payload else {
            return false;
        };
        // A ground sprite has no gump-local grab point. Once it becomes a
        // cursor icon, anchor its visual centre to the pointer; that same
        // offset is used if it is released into a container.
        let grab = self
            .resources
            .art
            .static_art(item.graphic)
            .ok()
            .flatten()
            .map(|art| GumpPixel::new(i32::from(art.width()) / 2, i32::from(art.height()) / 2))
            .unwrap_or_default();
        self.windows.item_drag = Some(crate::windows::ItemDragTransaction::Pressed(
            crate::windows::ItemPress {
                item: ContainedItem {
                    serial,
                    graphic: item.graphic,
                    amount,
                    // A ground item has no gump position. It becomes relevant only
                    // after a drop, which supplies a real one.
                    at: GumpPoint::new(0, 0),
                    grid: Default::default(),
                    hue: item.hue,
                },
                origin: crate::windows::DragOrigin::Ground,
                at: self.input.pointer_gump,
                grab,
            },
        ));
        true
    }

    /// The container icon the pointer is directly over in the last drawn frame.
    pub(crate) fn container_item_under_pointer(&self) -> Option<ContainedItem> {
        let WindowSubject::Container(container) = self.window_under_pointer()? else {
            return None;
        };
        // Stock icons are visual entries in a vendor window, not ordinary
        // container items: lifting one would send `0x07`, whereas buying it
        // must send `0x3B`.  The shop controls own those clicks.
        if self
            .world
            .authoritative
            .view
            .as_ref()?
            .vendor_buys
            .contains_key(&container)
        {
            return None;
        }
        let Drawn::Container(pictures) = self.drawn(WindowSubject::Container(container))? else {
            return None;
        };
        let index = gump_art::pick(pictures, self.input.pointer_gump, &self.resources.gump_atlas)?;
        let item_index = index.position().checked_sub(1)?;
        let contents = self.world.authoritative.view.as_ref()?.contents.get(&container)?;
        let held = self
            .windows
            .item_drag
            .and_then(crate::windows::ItemDragTransaction::drag)
            .map(|drag| drag.item.serial);
        contents
            .iter()
            // The lifter's own client removes this optimistically. The shard
            // does not echo a `Remove` back to that same connection.
            .filter(|item| Some(item.serial) != held)
            .nth(item_index)
            .copied()
    }

    /// Refresh the one-frame-later hover tint from the same pictures hit tests use.
    pub(crate) fn hover_container_item(&mut self) -> bool {
        let hovered = self.container_item_under_pointer().map(|item| item.serial);
        if self.windows.hovered_container_item == hovered {
            return false;
        }
        self.windows.hovered_container_item = hovered;
        true
    }

    /// The worn item under the pointer in the paperdoll drawn last frame.
    pub(crate) fn paperdoll_item_under_pointer(&self) -> Option<(Serial, Equipment)> {
        let WindowSubject::Paperdoll(mobile) = self.window_under_pointer()? else {
            return None;
        };
        let Drawn::Paperdoll(doll) = self.drawn(WindowSubject::Paperdoll(mobile))? else {
            return None;
        };
        let index = gump_art::pick(
            &doll.pictures,
            self.input.pointer_gump,
            &self.resources.gump_atlas,
        )?;
        let layer = *doll.equipment_hits.get(&index)?;
        let view = self.world.authoritative.view.as_ref()?;
        let equipment = if view.player.serial == mobile {
            &view.player.equipment
        } else {
            &view.mobiles.get(&mobile)?.equipment
        };
        equipment
            .iter()
            .find(|item| item.layer == layer)
            .copied()
            .map(|item| (mobile, item))
    }

    /// Refresh the paperdoll's worn-item hover tint.
    pub(crate) fn hover_paperdoll_item(&mut self) -> bool {
        let hovered = self
            .paperdoll_item_under_pointer()
            .map(|(mobile, item)| (mobile, item.layer));
        let preview = self
            .windows
            .item_drag
            .and_then(crate::windows::ItemDragTransaction::drag)
            .and_then(|drag| match self.window_under_pointer() {
                Some(WindowSubject::Paperdoll(mobile))
                    if self
                        .world
                        .authoritative
                        .view
                        .as_ref()
                        .is_some_and(|view| view.player.serial == mobile) =>
                {
                    Some((mobile, drag.item))
                }
                _ => None,
            });
        let changed = self.windows.hovered_equipment != hovered || self.windows.preview_equipment != preview;
        self.windows.hovered_equipment = hovered;
        self.windows.preview_equipment = preview;
        changed
    }

    /// Turn a genuine pointer move into a lift. A press without this movement
    /// remains a click and can therefore use the item on a double-click.
    pub(crate) fn drag_container_item(&mut self) -> bool {
        let Some(crate::windows::ItemDragTransaction::Pressed(press)) = self.windows.item_drag else {
            return false;
        };
        const DRAG_SLOP: i32 = 3;
        if (self.input.pointer_gump.x - press.at.x).abs() <= DRAG_SLOP
            && (self.input.pointer_gump.y - press.at.y).abs() <= DRAG_SLOP
        {
            return false;
        }
        if self.input.shift_held && press.item.amount.0 > 1 {
            if !self.windows.split_pending {
                let Some(shell) = self.shell.as_mut() else {
                    return false;
                };
                shell.open_split(press.item.amount.0 - 1);
                self.windows.split_pending = true;
                self.windows.dragging = None;
            }
            return true;
        }
        let Some(link) = self.world.shard.link() else {
            return false;
        };
        link.pick_up_item(press.item.serial, press.item.amount);
        self.windows.item_drag = Some(crate::windows::ItemDragTransaction::Held(
            crate::windows::ItemDrag {
                item: press.item,
                origin: press.origin,
                grab: press.grab,
            },
        ));
        self.reproject_item_drag();
        self.windows.hovered_container_item = None;
        self.windows.dragging = None;
        true
    }

    /// Continue or cancel the Pressed transaction held while the amount prompt
    /// was open. A contained stack is placed back beside its remainder so the
    /// modal gesture finishes visibly; a ground stack remains on the cursor.
    pub(crate) fn finish_stack_split(&mut self, decision: crate::shell::SplitDecision) {
        self.windows.split_pending = false;
        let Some(crate::windows::ItemDragTransaction::Pressed(press)) = self.windows.item_drag else {
            return;
        };
        let crate::shell::SplitDecision::Confirm(amount) = decision else {
            self.windows.item_drag = None;
            return;
        };
        let Some(amount) = split_amount(press.item.amount.0, amount) else {
            self.windows.item_drag = None;
            return;
        };
        let Some(link) = self.world.shard.link() else {
            self.windows.item_drag = None;
            return;
        };
        let split_item = ContainedItem {
            amount: openshard_protocol::items::ItemAmount(amount),
            ..press.item
        };
        let drag = crate::windows::ItemDrag {
            item: split_item,
            origin: press.origin,
            grab: press.grab,
        };
        link.pick_up_item(press.item.serial, split_item.amount);
        self.windows.item_drag = match press.origin {
            crate::windows::DragOrigin::Container(container) => {
                // Finish a backpack split immediately. The lift path suppresses
                // Remove for its own client, so leaving the part on the cursor
                // after a modal prompt can hide the old serial indefinitely.
                // Dropping it back produces an Add for that serial; reopening
                // then replaces the whole list, including the new remainder.
                let at = GumpPoint::new(press.item.at.x + 18, press.item.at.y + 18);
                link.drop_into(split_item.serial, container, at);
                link.use_object(container);
                Some(crate::windows::ItemDragTransaction::Dropped {
                    drag,
                    destination: crate::windows::PendingDrop::Container { container, at },
                })
            }
            crate::windows::DragOrigin::Ground | crate::windows::DragOrigin::Equipment { .. } => {
                Some(crate::windows::ItemDragTransaction::Held(drag))
            }
        };
        self.reproject_item_drag();
        self.windows.hovered_container_item = None;
    }

    /// Commit a held item to an open bag or a picked world tile. Its local
    /// transaction stays in `Dropped` until the server confirms or rejects it.
    pub(crate) fn release_container_item(&mut self) -> bool {
        let Some(transaction) = self.windows.item_drag else {
            return false;
        };
        let Some(drag) = transaction.drag() else {
            return false;
        };
        if transaction.pending_drop().is_some() {
            return true;
        }
        self.windows.preview_equipment = None;
        let target = match self.window_under_pointer() {
            Some(WindowSubject::Paperdoll(mobile)) => {
                let layer = openshard_protocol::wire::Layer(
                    self.resources.tiledata.static_tile(drag.item.graphic.0).layer,
                );
                // Always finish through the shard. An occupied or invalid
                // slot is still an answer: `equip_item` sends DragCancel and
                // restores the remembered origin. Keeping that rejection
                // local used to strand this transaction in `Held`.
                if let Some(link) = self.world.shard.link() {
                    link.equip(drag.item.serial, layer, mobile);
                    self.windows.item_drag = Some(crate::windows::ItemDragTransaction::Dropped {
                        drag,
                        destination: crate::windows::PendingDrop::Equipment { mobile, layer },
                    });
                    self.reproject_item_drag();
                }
                return true;
            }
            Some(WindowSubject::Container(serial)) => serial,
            _ => {
                // Outside a gump the protocol's x/y/z are world coordinates,
                // not gump pixels. `pick_tile` already answers against the
                // frame the player released over.
                if let (Some(link), Some(tile)) =
                    (self.world.shard.link(), self.pick_tile(*self.control.camera()))
                {
                    link.drop_on_ground(
                        drag.item.serial,
                        openshard_protocol::world::Point::new(tile.at.x, tile.at.y, tile.stand_z.0),
                    );
                    self.windows.item_drag = Some(crate::windows::ItemDragTransaction::Dropped {
                        drag,
                        destination: crate::windows::PendingDrop::Ground(
                            openshard_protocol::world::Point::new(tile.at.x, tile.at.y, tile.stand_z.0),
                        ),
                    });
                    self.reproject_item_drag();
                }
                return true;
            }
        };
        let Some(at) = self
            .windows
            .own_windows
            .iter()
            .find(|window| window.subject == WindowSubject::Container(target))
            .map(|window| {
                GumpPoint::new(
                    self.input.pointer_gump.x - window.at.x - drag.grab.x,
                    self.input.pointer_gump.y - window.at.y - drag.grab.y,
                )
            })
        else {
            return true;
        };
        if let Some(link) = self.world.shard.link() {
            // This gesture targets the open container, never an icon drawn in
            // it.  Otherwise a floor item released above a nested bag silently
            // enters that bag and its coordinates are interpreted in the wrong
            // gump. Stack merging is a separate destination gesture.
            link.drop_into(drag.item.serial, target, at);
            self.windows.item_drag = Some(crate::windows::ItemDragTransaction::Dropped {
                drag,
                destination: crate::windows::PendingDrop::Container {
                    container: target,
                    at,
                },
            });
            self.reproject_item_drag();
        }
        true
    }

    /// Finish a click that never became a drag. The double-click decision was
    /// made on press, before this window can be raised or relaid out again.
    pub(crate) fn release_container_press(&mut self) -> bool {
        if self.windows.split_pending {
            return true;
        }
        matches!(
            self.windows.item_drag,
            Some(crate::windows::ItemDragTransaction::Pressed(_))
        ) && {
            self.windows.item_drag = None;
            true
        }
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
            if let Drawn::Vendor(vendor) = drawn {
                return vendor.contains(cursor).then_some(window.subject);
            }
            // A dialog's fields are the one part of a window that is a box
            // rather than a picture — see `gump::Field` — and a click in one is
            // still a click on the window. It sits over the background, which is
            // a picture, so this only matters for a field the layout hung
            // outside its own frame; asking is cheaper than being wrong there.
            if let Drawn::Dialog(laid_out) = drawn {
                if gump_art::field(&laid_out.art.fields, cursor).is_some() {
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
    ///
    /// The press while the hand is full is **not** asked about here any more:
    /// it is the manager's first question, ahead of every pane and of this —
    /// see `App::manager_gestures` and decision 7 in
    /// `docs/window_components.md`.
    pub(crate) fn press_on_own_window(&mut self) -> bool {
        if let Some(container) = self.stack_all_button_under_pointer() {
            self.start_stack_pass(container);
            self.windows.dragging = None;
            return true;
        }
        if let Some(container) = self.take_all_button_under_pointer() {
            self.take_all_from_container(container);
            self.windows.dragging = None;
            return true;
        }
        // A press that missed every window gives the keyboard back, and that is
        // the manager's own gesture now — see `App::manager_gestures`, which
        // runs it ahead of every pane and of this.
        let Some(subject) = self.window_under_pointer() else {
            return false;
        };
        self.raise_window(subject);
        // No vendor arm: a shop answers its own press in
        // `panes::vendor::VendorPane`, which the router offers the press to
        // before this is reached. A vendor window that gets this far has never
        // been drawn — there is no layout to hit-test — and the tail below
        // picks it up by whatever the player grabbed, which is what used to
        // happen when the hit test found nothing.
        if let WindowSubject::Container(container) = subject {
            if let Some(item) = self.container_item_under_pointer() {
                let window = self
                    .windows
                    .own_windows
                    .iter()
                    .find(|window| window.subject == WindowSubject::Container(container));
                if let Some(window) = window {
                    let icon = window.at.offset(GumpPixel::new(item.at.x, item.at.y));
                    let now = Instant::now();
                    let paired = self.windows.last_container_click.is_some_and(|(then, serial)| {
                        serial == item.serial && now.duration_since(then) <= DOUBLE_CLICK
                    });
                    self.windows.last_container_click = (!paired).then_some((now, item.serial));
                    if paired {
                        if let Some(link) = self.world.shard.link() {
                            // A katana's ordinary double-click is a wield: lift
                            // it and immediately place it in the tiledata slot,
                            // exactly as a drag onto the doll does.
                            if item.graphic == Graphic(0x13FF) {
                                if let Some(player) = self
                                    .world
                                    .authoritative
                                    .view
                                    .as_ref()
                                    .map(|view| view.player.serial)
                                {
                                    let layer = openshard_protocol::wire::Layer(
                                        self.resources.tiledata.static_tile(item.graphic.0).layer,
                                    );
                                    link.pick_up_item(item.serial, item.amount);
                                    link.equip(item.serial, layer, player);
                                }
                            } else {
                                link.use_object(item.serial);
                            }
                        }
                        self.windows.item_drag = None;
                        self.windows.dragging = None;
                        return true;
                    }
                    self.windows.item_drag = Some(crate::windows::ItemDragTransaction::Pressed(
                        crate::windows::ItemPress {
                            item,
                            origin: crate::windows::DragOrigin::Container(container),
                            at: self.input.pointer_gump,
                            grab: GumpPixel::new(
                                self.input.pointer_gump.x - icon.x,
                                self.input.pointer_gump.y - icon.y,
                            ),
                        },
                    ));
                    self.windows.dragging = None;
                    return true;
                }
            }
        }
        // No dialog arm either: a `0xB0` answers its own press in
        // `panes::dialog::DialogPane`, including the `{ nomove }` rule — a press
        // that is taken and does *not* grab — which was the one shape in this
        // function that the tail below could not express.
        //
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
            if let Some((mobile, item)) = self.paperdoll_item_under_pointer() {
                if self
                    .world
                    .authoritative
                    .view
                    .as_ref()
                    .is_some_and(|view| view.player.serial == mobile)
                {
                    self.windows.item_drag = Some(crate::windows::ItemDragTransaction::Pressed(
                        crate::windows::ItemPress {
                            item: ContainedItem {
                                serial: item.serial,
                                graphic: item.graphic,
                                amount: openshard_protocol::items::ItemAmount(1),
                                at: GumpPoint::new(0, 0),
                                grid: Default::default(),
                                hue: item.hue,
                            },
                            origin: crate::windows::DragOrigin::Equipment {
                                mobile,
                                layer: item.layer,
                            },
                            at: self.input.pointer_gump,
                            grab: self
                                .resources
                                .art
                                .static_art(item.graphic)
                                .ok()
                                .flatten()
                                .map(|art| {
                                    GumpPixel::new(i32::from(art.width()) / 2, i32::from(art.height()) / 2)
                                })
                                .unwrap_or_default(),
                        },
                    ));
                    self.windows.dragging = None;
                    return true;
                }
            }
        }
        // No skills arm either, for the vendor's reason above: the sheet
        // answers its own press in `panes::skills::SkillsPane`, which the
        // router offers it to before this is reached.
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

    /// The release that finishes a press on a paperdoll's button.
    ///
    /// Answers whether anything happened, so the caller can ask for a redraw:
    /// the button comes back up on the way out either way.
    ///
    /// The dialog half of this went with step 4 — `DialogPane::handle` answers
    /// its own release, and the reply a button produces is an
    /// [`Effect::Answer`](crate::panes::Effect::Answer), which is the `0xB1` and
    /// the close as one act.
    pub(crate) fn release_on_own_window(&mut self) -> bool {
        let Some((subject, button)) = self.windows.held_doll.take() else {
            return false;
        };
        // Only if the pointer is still on the same button. A press that slid off
        // one is not a click on it — the reference's own rule for every control
        // it draws — and it is not a click on whatever the finger landed on
        // either.
        if self.doll_button_under_pointer(subject) == Some(button) {
            self.doll_clicked(subject, button);
        }
        // True whatever it landed on: the button was drawn pressed and has to
        // come back up.
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
            // Asked of the window's own pane, because what a dialog answers with
            // is made of what the player set on it. The manager still decides
            // *that* it closes — both gestures that close one are its own, and
            // Escape closes the topmost window without ever pointing at it — so
            // this is the one door and the pane is the one that can fill in the
            // packet.
            let dismissal = self
                .windows
                .own_windows
                .iter()
                .find(|window| window.subject == subject)
                .and_then(|window| match &window.pane {
                    crate::panes::AnyPane::Dialog(pane) => pane.dismiss(&gump),
                    // A dialog window always holds a dialog pane — `AnyPane::of`
                    // is a `match` on the subject — so this is not a case, it is
                    // the compiler being told the same thing twice.
                    _ => None,
                });
            let Some(reply) = dismissal else {
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
                // The overlay is what says this is closed — see D2 in
                // `docs/client_window_state.md`. The line under it writes the
                // same fact into this thread's own view, which is redundant
                // for a container and is not for a vendor (the arm below sets
                // only the overlay); both are kept until that plan's backlog
                // settles which one stays. The link thread is told neither:
                // its `WorldView` is cloned across exactly once, at world
                // entry, so there is nothing there to go stale.
                self.windows.locally_closed.insert(subject);
                self.apply_close_window(link::CloseTarget::Container(serial));
            }
            // Nothing to forget by hand: what was chosen and how far down the
            // list the player had got are fields of the pane, and the `retain`
            // at the end of this drops the window and them with it. There used
            // to be two `remove` calls here, and a kind that was added without
            // them would have leaked its state for the life of the client.
            WindowSubject::Vendor(_) => {
                self.windows.locally_closed.insert(subject);
            }
            WindowSubject::Paperdoll(serial) => {
                self.windows.locally_closed.insert(subject);
                self.apply_close_window(link::CloseTarget::Paperdoll(serial));
            }
            // Nothing in the view to tell and so nothing to overlay: the
            // skills and the status numbers stay where they are, the way a
            // paperdoll's equipment does. What closing takes away is whatever
            // the window's pane held — a tree for one kind, nothing at all for
            // the other — and the `retain` at the end of this is what takes it.
            // That is deliberate and not a loss: the reference's windows do not
            // remember either, and a window with no memory is the backlog entry
            // every kind here shares.
            //
            // The status arm used to be `self.windows.status = false`, which is
            // the same close said twice: once here and once in a field. Step 3
            // deleted the field.
            WindowSubject::Skills | WindowSubject::Status => {}
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
        let Some(link) = self.world.shard.link() else {
            tracing::info!(%line, "nothing said: no shard is connected");
            return;
        };
        // Which channel decides which *packet*, not only which mode byte: a
        // guild or alliance line is `0xAD` speech with a different mode, and a
        // party line is not speech at all — it is `0xBF 0x06`, and putting it
        // through the speech path would say it out loud to the street.
        match self.chat.channel {
            chat::Channel::Say => link.say(line, TalkMode::Regular),
            chat::Channel::Guild => link.say(line, TalkMode::Guild),
            chat::Channel::Alliance => link.say(line, TalkMode::Alliance),
            chat::Channel::Party => link.say_to_party(line),
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
        if let Some(link) = self.world.shard.link() {
            link.answer_gump(reply);
            // The reply leaves on the wire and says nothing about the window
            // being done, so this end has to close it itself — here in this
            // thread's view, and in the overlay below.
            self.apply_close_window(link::CloseTarget::Gump(gump_id));
        }
        if self.world.authoritative.view.is_some() {
            self.windows.locally_closed.insert(WindowSubject::Dialog(gump_id));
        }
    }
}

#[cfg(test)]
mod stack_tests {
    use super::*;
    use openshard_protocol::containers::GridSlot;
    use openshard_protocol::items::ItemAmount;
    use openshard_protocol::wire::Hue;

    fn pile(serial: u32, amount: u16) -> ContainedItem {
        ContainedItem {
            serial: Serial::new(serial).expect("item serial"),
            graphic: GOLD_GRAPHIC,
            amount: ItemAmount(amount),
            at: GumpPoint::new(0, 0),
            grid: GridSlot(0),
            hue: Hue::NONE,
        }
    }

    #[test]
    fn stacking_prefers_a_whole_pile_that_fits() {
        let items = [
            pile(0x4000_0001, 50_000),
            pile(0x4000_0002, 20_000),
            pile(0x4000_0003, 5_000),
        ];
        assert_eq!(
            next_stack_step(&items),
            Some(StackStep {
                source: items[2].serial,
                target: items[0].serial,
                amount: 5_000,
            })
        );
    }

    #[test]
    fn stacking_splits_only_enough_to_fill_sixty_thousand() {
        let items = [pile(0x4000_0001, 55_000), pile(0x4000_0002, 20_000)];
        assert_eq!(next_stack_step(&items).expect("one pass").amount, 5_000);
    }

    #[test]
    fn full_piles_are_skipped_on_the_way_to_a_remainder() {
        let items = [
            pile(0x4000_0001, MAX_STACK),
            pile(0x4000_0002, 15_000),
            pile(0x4000_0003, 10_000),
        ];
        let step = next_stack_step(&items).expect("the remainder piles merge");
        assert_eq!(step.target, items[1].serial);
        assert_eq!(step.amount, 10_000);
    }

    #[test]
    fn a_split_never_takes_none_or_the_whole_stack() {
        assert_eq!(split_amount(10, 0), Some(1));
        assert_eq!(split_amount(10, 4), Some(4));
        assert_eq!(split_amount(10, 10), Some(9));
        assert_eq!(split_amount(1, 1), None);
    }
}
