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

use openshard_client_render::vendor::Hit as VendorHit;
use openshard_client_render::{
    container,
    gump::{self as gump_art, GumpPixel},
};
use openshard_protocol::containers::ContainedItem;
use openshard_protocol::gump::GumpId;
use openshard_protocol::gump::GumpPoint;
use openshard_protocol::mobile::Equipment;
use openshard_protocol::serial::Serial;
use openshard_protocol::wire::{Graphic, Layer};

use crate::app::App;
use crate::windows::{Drawn, WindowSubject};
use crate::{DOUBLE_CLICK, gump, link};

mod paperdoll;
mod skills;
mod sync;

impl App {
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

    /// Scroll the catalogue under the pointer by one row per wheel gesture.
    ///
    /// Vendor packets can contain an entire restock, so their window has a
    /// fixed viewport rather than growing past the screen.  The row offset is
    /// local UI state, like a skill tree's scroll position; stock and selected
    /// quantities remain authoritative on the shard and in `WorldView`.
    pub(crate) fn scroll_vendor(&mut self, notches: f32) -> bool {
        let Some(WindowSubject::Vendor(vendor)) = self.window_under_pointer() else {
            return false;
        };
        let Some(Drawn::Vendor(window)) = self.drawn(WindowSubject::Vendor(vendor)) else {
            return false;
        };
        if !window.catalogue_contains(self.input.pointer_gump) {
            return false;
        }
        let rows = self
            .world
            .authoritative
            .view
            .as_ref()
            .and_then(|view| {
                view.vendor_buys
                    .get(&vendor)
                    .map(|catalogue| catalogue.lines.len())
                    .or_else(|| {
                        view.vendor_sells
                            .get(&vendor)
                            .map(|catalogue| catalogue.lines.len())
                    })
            })
            .unwrap_or_default();
        let offset = self.windows.vendor_scrolls.entry(vendor).or_default();
        let before = *offset;
        let maximum = rows.saturating_sub(openshard_client_render::vendor::VISIBLE_ROWS);
        if notches > 0.0 {
            *offset = offset.saturating_sub(1);
        } else {
            *offset = (*offset + 1).min(maximum);
        }
        *offset != before
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
                    amount: item.amount.0,
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
                let Some(view) = self.world.authoritative.view.as_ref() else {
                    return true;
                };
                // This is the same local gate ClassicUO uses for its ghost:
                // a wearable can land only on our own doll and only in an
                // empty slot.  The shard remains authoritative, but sending
                // an equip it will certainly reject leaves a misleading drag.
                let slot_taken = view.player.serial != mobile
                    || layer.0 == 0
                    || layer.0 > 25
                    || view
                        .player
                        .equipment
                        .iter()
                        .any(|worn| worn.layer == layer && worn.serial != drag.item.serial);
                if slot_taken {
                    return true;
                }
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
        if let Some(container) = self.take_all_button_under_pointer() {
            self.take_all_from_container(container);
            self.windows.dragging = None;
            return true;
        }
        let Some(subject) = self.window_under_pointer() else {
            // A press that missed every window gives the keyboard back: a field
            // stays focused only while the player is still in the dialog.
            self.windows.dialogs.unfocus();
            return false;
        };
        self.raise_window(subject);
        if let WindowSubject::Vendor(vendor) = subject {
            let hit = self.drawn(subject).and_then(|drawn| match drawn {
                Drawn::Vendor(window) => window.hit(self.input.pointer_gump),
                _ => None,
            });
            match hit {
                Some(VendorHit::Row(row)) => {
                    let limit = self
                        .world
                        .authoritative
                        .view
                        .as_ref()
                        .map(|view| {
                            if let Some(sale) = view.vendor_sells.get(&vendor) {
                                sale.lines.get(row).map_or(0, |line| line.amount)
                            } else {
                                view.contents
                                    .get(&view.vendor_buys[&vendor].container)
                                    .and_then(|items| items.get(row))
                                    .map_or(0, |item| item.amount)
                            }
                        })
                        .unwrap_or(0);
                    let amounts = self.windows.vendor_amounts.entry(vendor).or_default();
                    if amounts.len() <= row {
                        amounts.resize(row + 1, 0);
                    }
                    amounts[row] = if amounts[row] >= limit {
                        0
                    } else {
                        amounts[row] + 1
                    };
                    self.windows.dragging = None;
                    return true;
                }
                Some(VendorHit::Confirm) => {
                    self.confirm_vendor(vendor);
                    return true;
                }
                Some(VendorHit::Remove(row)) => {
                    if let Some(amount) = self
                        .windows
                        .vendor_amounts
                        .get_mut(&vendor)
                        .and_then(|amounts| amounts.get_mut(row))
                    {
                        *amount = amount.saturating_sub(1);
                    }
                    self.windows.dragging = None;
                    return true;
                }
                Some(VendorHit::Clear) => {
                    if let Some(amounts) = self.windows.vendor_amounts.get_mut(&vendor) {
                        amounts.fill(0);
                    }
                    self.windows.dragging = None;
                    return true;
                }
                // The art's header and empty parchment are deliberately not
                // actions.  Let the normal window path below pick the gump up
                // from either of them.
                None => {}
            }
        }
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
                                amount: 1,
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
            WindowSubject::Vendor(serial) => {
                self.windows.vendor_amounts.remove(&serial);
                self.windows.vendor_scrolls.remove(&serial);
                self.windows.locally_closed.insert(subject);
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

    /// Send one atomic catalogue answer. Selecting rows is purely local; only
    /// this button crosses the wire, so the server remains the authority for
    /// stock, money and the contents of the player's backpack.
    fn confirm_vendor(&mut self, vendor: openshard_protocol::serial::Serial) {
        let Some(view) = self.world.authoritative.view.as_ref() else {
            return;
        };
        let amounts = self
            .windows
            .vendor_amounts
            .get(&vendor)
            .cloned()
            .unwrap_or_default();
        if let Some(catalogue) = view.vendor_sells.get(&vendor) {
            let sales = catalogue
                .lines
                .iter()
                .zip(amounts)
                .filter_map(|(line, amount)| (amount > 0).then_some((line.serial, amount)))
                .collect();
            if let Some(link) = self.world.shard.link() {
                link.sell(vendor, sales);
            }
        } else if let Some(catalogue) = view.vendor_buys.get(&vendor) {
            let purchases = view
                .contents
                .get(&catalogue.container)
                .into_iter()
                .flatten()
                .zip(amounts)
                .filter_map(|(item, amount)| (amount > 0).then_some((item.serial, amount)))
                .collect();
            if let Some(link) = self.world.shard.link() {
                link.buy(vendor, purchases);
            }
        }
        self.close_window(WindowSubject::Vendor(vendor));
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
        match self.world.shard.link() {
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
        if let Some(link) = self.world.shard.link() {
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
