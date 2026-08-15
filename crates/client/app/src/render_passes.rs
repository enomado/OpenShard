//! The GPU passes a frame's already-collected geometry is recorded through:
//! [`encode_world_passes`] is the world — ground, statics, mobiles, the
//! masks and rings — and [`draw_gump_windows`] is this client's own dialogs,
//! drawn over it in the client's own art. Neither decides what is drawn;
//! `crate::frame_geometry` and `App::draw_from` do that, and hand the
//! answer here to be recorded.

use openshard_client_render::blit::{self, ViewportRect};
use std::collections::BTreeSet;

use openshard_client_render::camera::Camera;
use openshard_client_render::composite::{
    CompositeProducerJob, CompositeQuad, ImmutableRevision, MapBlock, MapBlockBounds,
};
use openshard_client_render::geometry::Rect;
use openshard_client_render::gump::{self as gump_art};
use openshard_client_render::lod::BlockLod;
use openshard_client_render::outline::{self, Ring};
use openshard_client_render::renderer::Target;
use openshard_client_render::select::{self, Selection};
use openshard_client_render::sprite::SpriteQuad;
use openshard_client_render::{container, paperdoll, skills, solids, status, vendor};
use openshard_protocol::containers::ContainedItem;
use std::time::{Duration, Instant};

use crate::frame_geometry::FrameGeometry;
use crate::picking::{self, SelectedIdentity};
use crate::window::Screen;

fn container_hover_text(item: &ContainedItem, name: &str) -> String {
    format!("{name} ({})", item.amount.0)
}

/// Facts from the one world-pass encoding that a GPU dump can later compare
/// with its attachments. Keeping these numbers beside the exact frame closes
/// the gap between a requested LOD policy and the list the ground renderer was
/// actually handed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldPassAudit {
    pub(crate) requested_lod: BlockLod,
    pub(crate) composite_revision: ImmutableRevision,
    pub(crate) ready_blocks: usize,
    pub(crate) live_ground_quads: usize,
    pub(crate) full_ground_quads: usize,
    /// CPU command-recording costs within the world encoder. These are kept
    /// beside the frame's aggregate `encode` time so a far-zoom trace can
    /// distinguish bind/upload work from the later overlays and UI passes.
    pub(crate) cpu_ground: Duration,
    pub(crate) cpu_composites: Duration,
    pub(crate) cpu_ground_detail: Duration,
    pub(crate) ground_detail_cpu_uniforms: Duration,
    pub(crate) ground_detail_cpu_serialize: Duration,
    pub(crate) ground_detail_cpu_upload: Duration,
    pub(crate) ground_detail_cpu_pass: Duration,
    pub(crate) cpu_statics: Duration,
    pub(crate) cpu_items: Duration,
    pub(crate) composite_bindings_created: usize,
    pub(crate) composite_bindings_reused: usize,
    pub(crate) composite_cpu_upload: Duration,
    pub(crate) composite_cpu_bindings: Duration,
    pub(crate) composite_cpu_pass: Duration,
}
use crate::windows::{Drawn, WindowSubject};
use crate::{crowd, graphics, profile, resources, shell, windows, world};

/// The shard's dialogs, in the client's own art, packed and drawn — a
/// container, a paperdoll, the skill sheet, all three through one machinery.
/// None of them is an egui window: their position, their drag, their
/// z-order and their hit test are this client's, in gump pixels, which is
/// decision 5 in `docs/client.md`. See `own_windows`, `crate::gump`,
/// `openshard_client_render::container` and
/// `openshard_client_render::paperdoll`.
///
/// A free function for the same reason [`encode_world_passes`] is one:
/// `resources.gump_atlas` grows and `windows.drawn_windows` is written here,
/// and both are named in the signature rather than reached through
/// `&mut self`. Does nothing when there is no gump file or no pass to draw
/// through — an offline run with neither.
// Named individually on purpose — see the doc above: reaching them through
// `&mut self` is what this function exists to avoid.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gump_windows(
    resources: &mut resources::Resources,
    world: &world::WorldState,
    windows: &mut windows::Windows,
    cursor: gump_art::GumpPixel,
    shell: Option<&shell::Shell>,
    window: &mut Screen,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) {
    // `drawn_windows` is both the previous frame's hit-test map and the
    // source of the text overlay assembled later in `App::draw_from`.  It
    // must therefore describe *this* frame's art pass, including the frame in
    // which the pass cannot run.  Keeping yesterday's layout here used to
    // leave an invisible window intercepting clicks (and its labels still
    // eligible for the text pass) after gump assets or their GPU pass were
    // unavailable.
    // Once each, not once a frame: at sixty frames a second these would drown
    // the lines that matter.
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static SAID: AtomicBool = AtomicBool::new(false);
        if (resources.gumps.is_none() || window.gump_pass.is_none()) && !SAID.swap(true, Ordering::Relaxed) {
            eprintln!(
                "PROBE draw: art={} pass={} — no dialog can ever draw",
                resources.gumps.is_some(),
                window.gump_pass.is_some()
            );
        }
    }
    if let (Some(files), Some(pass)) = (resources.gumps.as_ref(), window.gump_pass.as_mut()) {
        // Every open dialog's art, packed before anything is laid out.
        //
        // Before, and not on the way, for two reasons. A `{ resizepic }`
        // cannot be placed until its nine pieces have been packed — where
        // its edges go is decided by how big its corners turned out to be —
        // and a page button flips pages inside the client, so what a window
        // needs is *every* page's art rather than the showing one's.
        // `gump::art_of` is that list, asked for on the frame the window is
        // drawn on because that is the frame that knows it is open at all.
        let open = world
            .authoritative
            .view
            .as_ref()
            .map(|view| view.gumps.as_slice())
            .unwrap_or_default();
        let mut pictures = Vec::new();
        for gump in open {
            let art_files = gump_art::ArtFiles {
                gumps: files,
                items: &resources.art,
            };
            if let Err(error) = resources
                .gump_atlas
                .add(art_files, gump_art::art_of(&gump.elements))
            {
                // Said once per window and then drawn without whatever is
                // missing: a dialog with a hole in it is still a dialog the
                // player can read, and a client that refused to draw one
                // would take the shard's staff commands down with it.
                eprintln!("packing gump art for {:?}: {error}", gump.gump_id);
            }
        }
        // This client's own windows — a dialog, a container, a paperdoll —
        // all three through one machinery.
        //
        // Bottom to top, which is the list's own order: the pass has no
        // depth, so later is over.
        //
        // The layouts are built before the loop that packs them, so that
        // nothing borrows the view while the atlas is being grown.
        // Paired with their subjects rather than left parallel to
        // `own_windows`: a container whose entry has gone from the view is
        // skipped below, and an index into one list would then name the
        // wrong window in the other. This list is what the pointer is
        // tested against next frame — see `windows::Windows::drawn_windows`.
        let mut drawn_windows: Vec<(WindowSubject, Drawn)> = Vec::new();
        if let Some(view) = world.authoritative.view.as_ref() {
            let art_files = gump_art::ArtFiles {
                gumps: files,
                items: &resources.art,
            };
            if let Err(error) = resources.gump_atlas.add(art_files, vendor::art_of()) {
                eprintln!("packing vendor gump art: {error}");
            }
            for open in &windows.own_windows {
                let WindowSubject::Container(serial) = open.subject else {
                    continue;
                };
                let Some(gump) = view.containers.get(&serial).copied() else {
                    continue;
                };
                let contents_serial = view
                    .vendor_buys
                    .get(&serial)
                    .map_or(serial, |catalogue| catalogue.container);
                let contents = view
                    .contents
                    .get(&contents_serial)
                    .map_or(&[] as &[ContainedItem], Vec::as_slice);
                if let Err(error) = resources
                    .gump_atlas
                    .add(art_files, container::art_of(gump, contents))
                {
                    eprintln!("packing container art for {serial}: {error}");
                }
            }
            for open in &windows.own_windows {
                match open.subject {
                    WindowSubject::Vendor(serial) => {
                        let Some(catalogue) = view.vendor_buys.get(&serial) else {
                            let Some(catalogue) = view.vendor_sells.get(&serial) else {
                                continue;
                            };
                            let amounts = windows
                                .vendor_amounts
                                .entry(serial)
                                .or_insert_with(|| vec![0; catalogue.lines.len()]);
                            if amounts.len() != catalogue.lines.len() {
                                amounts.resize(catalogue.lines.len(), 0);
                            }
                            let scroll = *windows.vendor_scrolls.entry(serial).or_default();
                            drawn_windows.push((
                                open.subject,
                                Drawn::Vendor(vendor::sell(
                                    serial,
                                    &catalogue.lines,
                                    amounts,
                                    scroll,
                                    open.at,
                                    cursor,
                                    &resources.gump_atlas,
                                )),
                            ));
                            continue;
                        };
                        let amounts = windows
                            .vendor_amounts
                            .entry(serial)
                            .or_insert_with(|| vec![0; catalogue.lines.len()]);
                        if amounts.len() != catalogue.lines.len() {
                            amounts.resize(catalogue.lines.len(), 0);
                        }
                        let scroll = *windows.vendor_scrolls.entry(serial).or_default();
                        drawn_windows.push((
                            open.subject,
                            Drawn::Vendor(vendor::buy(
                                serial,
                                &catalogue.lines,
                                view.contents.get(&catalogue.container).map_or(&[], Vec::as_slice),
                                amounts,
                                scroll,
                                open.at,
                                cursor,
                                &resources.gump_atlas,
                            )),
                        ));
                    }
                    WindowSubject::Dialog(gump_id) => {
                        let Some(gump) = view.gumps.iter().find(|gump| gump.gump_id == gump_id) else {
                            eprintln!("PROBE draw: a window for {gump_id:?} but the view has no such gump");
                            continue;
                        };
                        // Once per window, not once a frame.
                        {
                            use std::sync::atomic::{AtomicU32, Ordering};
                            static LAST: AtomicU32 = AtomicU32::new(u32::MAX);
                            if LAST.swap(gump_id.0, Ordering::Relaxed) != gump_id.0 {
                                eprintln!("PROBE draw: laying out {gump_id:?} at {:?}", open.at);
                            }
                        }
                        // The page, the switches and the pressed button are
                        // the three things the wire does not carry, and all
                        // three come out of `Dialogs` — which is also what
                        // the press that set them wrote to, so what is drawn
                        // pressed is what the release will act on.
                        drawn_windows.push((
                            open.subject,
                            Drawn::Dialog(windows.dialogs.layout(gump, open.at, &resources.gump_atlas)),
                        ));
                    }
                    WindowSubject::Skills => {
                        let Some(tree) = windows.skills.as_ref() else {
                            continue;
                        };
                        // The three sources meet here and nowhere else: the
                        // names and the tree out of the client's files, the
                        // numbers out of the view, and how wide a string
                        // came out of the font atlas — which is what puts a
                        // value's right edge where it belongs and starts a
                        // heading's rule at the end of its name.
                        //
                        // The wire's `u8` becomes a `SkillId` here, at the
                        // one seam that holds both: `view::Player::skills`
                        // is keyed by what the shard said, and the files'
                        // numbering is the type the window is written
                        // against. A skill the files do not name has no row
                        // to put a number on, and is dropped by `names.get`
                        // inside the layout rather than here.
                        drawn_windows.push((
                            open.subject,
                            Drawn::Skills(skills::window(
                                &resources.skill_names,
                                &resources.skill_groups,
                                tree,
                                |id| {
                                    view.player.skills.get(&id.0).map(|line| skills::Standing {
                                        skill: *line,
                                        // The player's own click, held over
                                        // the shard's line — see
                                        // `Tree::lock_of`'s doc.
                                        lock: tree.lock_of(id, line.lock),
                                    })
                                },
                                |text, font| {
                                    openshard_client_render::text::gump_width(
                                        text,
                                        font,
                                        &resources.font_atlas,
                                    )
                                },
                                open.at,
                            )),
                        ));
                    }
                    WindowSubject::Status => {
                        let (Some(status), Some(hits)) = (view.player.status.as_ref(), view.player.hits)
                        else {
                            continue;
                        };
                        drawn_windows
                            .push((open.subject, Drawn::Status(status::window(status, hits, open.at))));
                    }
                    WindowSubject::Container(serial) => {
                        let Some(gump) = view.containers.get(&serial).copied() else {
                            continue;
                        };
                        // A vendor window is keyed by the vendor mobile, while
                        // its displayed stock lives in the crate named by the
                        // buy catalogue.  Ordinary bags use their own serial
                        // for both.  Draw the stock in the classic shop gump;
                        // purchases themselves still go through the vendor
                        // controls rather than the normal item-drag path.
                        let contents_serial = view
                            .vendor_buys
                            .get(&serial)
                            .map_or(serial, |catalogue| catalogue.container);
                        // A successful lift is not echoed as `Remove` to the
                        // player holding it: that client already took the icon
                        // out of its own gump. Mirror that rule locally until
                        // the drag is completed or refused.
                        let transaction = windows.item_drag;
                        let held = transaction.and_then(crate::windows::ItemDragTransaction::drag);
                        let mut contents: Vec<ContainedItem> = view
                            .contents
                            .get(&contents_serial)
                            .into_iter()
                            .flatten()
                            .filter(|item| Some(item.serial) != held.map(|drag| drag.item.serial))
                            .copied()
                            .collect();
                        if let (Some(drag), Some(crate::windows::PendingDrop::Container { container, at })) = (
                            held,
                            transaction.and_then(crate::windows::ItemDragTransaction::pending_drop),
                        ) {
                            if container == serial {
                                contents.push(ContainedItem { at, ..drag.item });
                            }
                        }
                        drawn_windows.push((
                            open.subject,
                            Drawn::Container(container::window_highlighted(
                                gump,
                                &contents,
                                open.at,
                                windows.hovered_container_item,
                            )),
                        ));
                    }
                    WindowSubject::Paperdoll(serial) => {
                        // Whose body and whose equipment, read off the view
                        // inline rather than through a method: the
                        // surface's window is held mutably across this
                        // loop, and a `&self` call would borrow all of it.
                        // Nothing else asks these questions — the hit test
                        // reads the list this builds (`drawn_windows`)
                        // rather than working out the body a second time,
                        // which is what used to make a paperdoll whose two
                        // answers disagreed a window that could not be
                        // closed.
                        let own = view.player.serial == serial;
                        let body = match own {
                            true => Some((view.player.body, view.player.hue)),
                            // A paperdoll of a mobile this client has never
                            // been told the body of: the frame is drawn and
                            // the doll is not, until the `0x77` arrives.
                            false => view.mobiles.get(&serial).map(|m| (m.body, m.hue)),
                        };
                        // The `0x88` carries no equipment — see
                        // `WorldView::paperdolls` — so it is read off the
                        // body the window names.
                        let equipment = match own {
                            true => crowd::worn(&view.player.equipment, &resources.tiledata),
                            false => match view.mobiles.get(&serial) {
                                Some(mobile) => crowd::worn(&mobile.equipment, &resources.tiledata),
                                None => Vec::new(),
                            },
                        };
                        let equipment: Vec<_> = equipment
                            .into_iter()
                            .filter(|item| {
                                windows
                                    .item_drag
                                    .and_then(crate::windows::ItemDragTransaction::drag)
                                    .is_none_or(|drag| {
                                        drag.origin
                                            != crate::windows::DragOrigin::Equipment {
                                                mobile: serial,
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
                        // The stance, off the player and not off the `0x88`
                        // the window opened on: a `0x72` moves it while
                        // that packet stands still, and the toggle is the
                        // one picture on the frame that has to follow. See
                        // `WorldView::player`'s `war`, which is where both
                        // packets are folded to.
                        let whose = match own {
                            true => paperdoll::Whose::Own { war: view.player.war },
                            false => paperdoll::Whose::Another,
                        };
                        // Which button the finger is on, if it is on one of
                        // this window's. Held per window rather than per
                        // client: two dolls can be open and only the pressed
                        // one draws a pressed picture.
                        let held = windows
                            .held_doll
                            .filter(|(window, _)| *window == open.subject)
                            .map(|(_, button)| button);
                        let preview = windows
                            .preview_equipment
                            .filter(|(mobile, _)| *mobile == serial)
                            .and_then(|(_, item)| {
                                let layer = openshard_protocol::wire::Layer(
                                    resources.tiledata.static_tile(item.graphic.0).layer,
                                );
                                (own && layer.0 > 0
                                    && layer.0 <= 25
                                    && !equipment.iter().any(|worn| worn.layer == layer))
                                .then_some(openshard_client_render::mobiles::EquipmentLayer {
                                    graphic: resources.tiledata.static_tile(item.graphic.0).anim_id,
                                    hue: item.hue,
                                    layer,
                                })
                            });
                        drawn_windows.push((
                            open.subject,
                            Drawn::Paperdoll(paperdoll::window(
                                wearer.as_ref(),
                                whose,
                                held,
                                windows
                                    .hovered_equipment
                                    .and_then(|(mobile, layer)| (mobile == serial).then_some(layer)),
                                preview,
                                &resources.equip_conv,
                                files,
                                open.at,
                            )),
                        ));
                    }
                }
            }
        }
        for (_, window) in &drawn_windows {
            let art_files = gump_art::ArtFiles {
                gumps: files,
                items: &resources.art,
            };
            // Everything the window will draw, packed before it is drawn —
            // a picture the atlas grew on the *next* frame would draw the
            // window with a hole in it once. Said and drawn anyway on a
            // failure, for `gump::art_of`'s reason above.
            if let Err(error) = resources
                .gump_atlas
                .add(art_files, paperdoll::art_of(window.pictures()))
            {
                eprintln!("packing window art: {error}");
            }
        }
        // The item follows the pointer above every window while the shard has
        // it on the cursor. It intentionally is not added to `drawn_windows`:
        // it is a cursor preview, not a window that can intercept a drop.
        if let Some(drag) = windows
            .item_drag
            .filter(|transaction| transaction.pending_drop().is_none())
            .and_then(crate::windows::ItemDragTransaction::drag)
        {
            let art_files = gump_art::ArtFiles {
                gumps: files,
                items: &resources.art,
            };
            if let Err(error) = resources.gump_atlas.add(
                art_files,
                [gump_art::GumpArt::Item(
                    openshard_client_render::items::displayed_graphic(drag.item.graphic, drag.item.amount),
                )],
            ) {
                eprintln!("packing dragged item art: {error}");
            }
            pictures.push(
                gump_art::Picture::plain(
                    gump_art::GumpArt::Item(openshard_client_render::items::displayed_graphic(
                        drag.item.graphic,
                        drag.item.amount,
                    )),
                    gump_art::GumpPixel::new(cursor.x - drag.grab.x, cursor.y - drag.grab.y),
                )
                .hued(drag.item.hue),
            );
        }
        // What the pointer is tested against from here on, and the atlas it
        // is tested in is the one just grown for it: the hit test and the
        // frame are now the same list. Kept even when it is empty — the
        // windows this frame drew none of are windows nothing can click.
        windows.drawn_windows = drawn_windows;
        if let Some(rows) = resources.gump_atlas.take_dirty() {
            pass.upload_rows(&window.queue, resources.gump_atlas.pixels(), rows);
        }
        let frame = gump_art::Frame {
            target: view,
            width: window.config.width,
            height: window.config.height,
            scale: shell.map(|shell| shell.pixels_per_point()).unwrap_or(1.0),
        };
        // A window is one painter layer: its frame and icons, then the text
        // belonging to that frame, before the next window is allowed to cover
        // it.  A global text pass cannot express this ordering.
        for (subject, drawn) in &windows.drawn_windows {
            let art = gump_art::collect(drawn.pictures(), &resources.gump_atlas);
            pass.render_layer(&window.device, &window.queue, encoder, frame, &art);
            let mut labels = Vec::new();
            let mut cut = Vec::new();
            let container_hover = match subject {
                WindowSubject::Container(serial) => windows.hovered_container_item.and_then(|hovered| {
                    let item = world
                        .authoritative
                        .view
                        .as_ref()?
                        .contents
                        .get(serial)?
                        .iter()
                        .find(|item| item.serial == hovered)?;
                    Some(container_hover_text(
                        item,
                        &resources.tiledata.static_tile(item.graphic.0).name,
                    ))
                }),
                _ => None,
            };
            match (subject, drawn) {
                (WindowSubject::Dialog(gump_id), Drawn::Dialog(laid_out)) => {
                    if let Some(gump) = world
                        .authoritative
                        .view
                        .as_ref()
                        .and_then(|view| view.gumps.iter().find(|gump| gump.gump_id == *gump_id))
                    {
                        labels.extend(windows.dialogs.lines(
                            gump,
                            laid_out,
                            &resources.font_atlas,
                            resources.cliloc.as_ref(),
                        ));
                    }
                }
                (WindowSubject::Paperdoll(serial), Drawn::Paperdoll(_)) => {
                    if let (Some(at), Some(doll)) = (
                        windows
                            .own_windows
                            .iter()
                            .find(|window| window.subject == *subject)
                            .map(|window| window.at),
                        world
                            .authoritative
                            .view
                            .as_ref()
                            .and_then(|view| view.paperdolls.get(serial)),
                    ) {
                        labels.push(paperdoll::name(&doll.name, at));
                    }
                    if let Some((mobile, layer)) = windows.hovered_equipment {
                        if mobile == *serial {
                            let item = world.authoritative.view.as_ref().and_then(|view| {
                                if view.player.serial == mobile {
                                    view.player.equipment.iter().find(|item| item.layer == layer)
                                } else {
                                    view.mobiles
                                        .get(&mobile)?
                                        .equipment
                                        .iter()
                                        .find(|item| item.layer == layer)
                                }
                            });
                            if let Some(item) = item {
                                labels.push(openshard_client_render::text::GumpLabel {
                                    at: cursor.offset(gump_art::GumpPixel::new(14, 18)),
                                    text: &resources.tiledata.static_tile(item.graphic.0).name,
                                    font: openshard_protocol::speech::Font(1),
                                    hue: openshard_protocol::wire::Hue::LABEL,
                                    clip: None,
                                });
                            }
                        }
                    }
                }
                (WindowSubject::Skills, Drawn::Skills(sheet)) => {
                    for line in &sheet.lines {
                        let mut quads = openshard_client_render::text::collect_gump(
                            &[line.label()],
                            &resources.font_atlas,
                        );
                        if let Some(scissor) = line.scissor {
                            scissor.cut(&mut quads);
                        }
                        cut.extend(quads);
                    }
                }
                (WindowSubject::Status, Drawn::Status(status)) => {
                    labels.extend(status.lines.iter().map(|line| line.label()));
                }
                (WindowSubject::Vendor(_), Drawn::Vendor(vendor)) => {
                    labels.extend(vendor.lines.iter().map(|line| line.label()));
                }
                (WindowSubject::Container(serial), Drawn::Container(_)) => {
                    let backpack = world.authoritative.view.as_ref().and_then(|view| {
                        view.player
                            .equipment
                            .iter()
                            .find(|item| item.layer == openshard_protocol::wire::Layer::BACKPACK)
                            .map(|item| item.serial)
                    });
                    if let (Some(view), Some(open), Some(gump)) = (
                        world.authoritative.view.as_ref(),
                        windows
                            .own_windows
                            .iter()
                            .find(|window| window.subject == *subject),
                        world
                            .authoritative
                            .view
                            .as_ref()
                            .and_then(|view| view.containers.get(serial)),
                    ) {
                        if !view.vendor_buys.contains_key(serial) {
                            if backpack != Some(*serial) {
                                if let Some(button) =
                                    container::take_all_button(&resources.gump_atlas, *gump, open.at)
                                {
                                    labels.push(openshard_client_render::text::GumpLabel {
                                        at: button.label_at(),
                                        text: container::TAKE_ALL_LABEL,
                                        font: openshard_protocol::speech::Font(1),
                                        hue: openshard_protocol::wire::Hue::LABEL,
                                        clip: None,
                                    });
                                }
                            }
                            if backpack == Some(*serial) {
                                if let Some(button) =
                                    container::stack_all_button(&resources.gump_atlas, *gump, open.at)
                                {
                                    labels.push(openshard_client_render::text::GumpLabel {
                                        at: button.label_at(),
                                        text: container::STACK_ALL_LABEL,
                                        font: openshard_protocol::speech::Font(1),
                                        hue: openshard_protocol::wire::Hue::LABEL,
                                        clip: None,
                                    });
                                }
                            }
                        }
                    }
                    if let Some(text) = container_hover.as_deref() {
                        labels.push(openshard_client_render::text::GumpLabel {
                            at: cursor.offset(gump_art::GumpPixel::new(14, 18)),
                            text,
                            font: openshard_protocol::speech::Font(1),
                            hue: openshard_protocol::wire::Hue::LABEL,
                            clip: None,
                        });
                    }
                }
                _ => {}
            }
            let mut text = openshard_client_render::text::collect_gump(&labels, &resources.font_atlas);
            text.extend(cut);
            window
                .gump_text_pass
                .render_layer(&window.device, &window.queue, encoder, frame, &text);
        }
        let dragged = gump_art::collect(&pictures, &resources.gump_atlas);
        pass.render_layer(&window.device, &window.queue, encoder, frame, &dragged);
    } else {
        windows.drawn_windows.clear();
    }
}

/// Records every world-space pass into `encoder`, from the ground up to the
/// hover and held rings — the one part of presenting a frame that is
/// **only** drawing, in the sense the module docs on
/// [`crate::graphics::GraphicsSettings`] and [`crate::picking::Picking`]
/// argue for: a free function taking `&mut GraphicsSettings` for the one
/// pair of fields it writes (`solids_held`, `solids_drawn`, this frame's own
/// count of what the solids view was handed and drew) and `&Picking` for the
/// one it only reads. See `crate::frame_geometry::assemble_geometry`'s doc
/// for the same shape one step earlier in the frame.
///
/// `encoder` and `window` are threaded through rather than returned: the
/// gump windows, the chat line and egui all record into the same encoder
/// after this call returns, and `App::draw_from` is what owns that sequence.
///
/// `geometry` is taken whole and not exploded into its fields: it is
/// `assemble_geometry`'s own output, still in the shape that function built
/// it in, and every field this pass reads (all but `asked_for`, which is the
/// F12 dump's) comes straight off it. `text_quads` is the one thing drawn
/// here that is not part of it — the overhead speech quads, collected
/// separately after the geometry is assembled — so it stays its own
/// parameter rather than a field something would have to graft onto
/// [`FrameGeometry`] just to be threaded through in the same breath.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_world_passes(
    graphics: &mut graphics::GraphicsSettings,
    picking: &picking::Picking,
    window: &mut Screen,
    encoder: &mut wgpu::CommandEncoder,
    target: Target<'_>,
    view: &wgpu::TextureView,
    world_view: &wgpu::TextureView,
    gbuffer_views: &openshard_client_render::gbuffer::Views,
    cutaway_world_view: &wgpu::TextureView,
    cutaway_gbuffer_views: &openshard_client_render::gbuffer::Views,
    viewport: ViewportRect,
    camera: Camera,
    solid_cut: openshard_client_render::solid::Cut,
    geometry: &FrameGeometry,
    text_quads: &[SpriteQuad],
    render_width: u32,
    render_height: u32,
    composite_lod: BlockLod,
    composite_revision: ImmutableRevision,
    composite_visible: Option<MapBlockBounds>,
) -> WorldPassAudit {
    // Ground first, because it clears; statics after, into what it left.
    // Which covers which is decided by the depth they share, not by this
    // order — the order only decides who clears.
    //
    // Every pass from here to the submit is bracketed by `profile::begin`
    // and `profile::end`: the GPU's own timestamps, which are the one half
    // of a frame's cost no clock on this thread can see. See [`profile`] for
    // why that is so and why the bracket is a pair of calls rather than a
    // scope guard. Nothing when the adapter has no timestamp queries.
    let ready: Vec<(
        MapBlock,
        Rect,
        &openshard_client_render::composite::CompositeTexture,
    )> = (geometry.cutaway_instances.drawn == 0)
        .then_some(composite_visible)
        .flatten()
        .into_iter()
        .flat_map(MapBlockBounds::blocks)
        .filter_map(|block| {
            let texture =
                window
                    .composites
                    .selected_or_more_detailed(block, composite_lod, composite_revision)?;
            if !texture.has_deferred() {
                return None;
            }
            debug_assert_eq!(texture.ground().block(), block);
            let rect = texture.rect_in(camera);
            debug_assert_eq!(
                rect,
                CompositeProducerJob::for_flat_ground(texture.key(), texture.ground()).rect_in(camera),
                "the cached texture must restore through its producer transform"
            );
            Some((block, rect, texture))
        })
        .collect();
    let cached_blocks: BTreeSet<_> = ready.iter().map(|(block, _, _)| *block).collect();
    let ground = geometry.detail_ground(&cached_blocks);
    let mut audit = WorldPassAudit {
        requested_lod: composite_lod,
        composite_revision,
        ready_blocks: ready.len(),
        live_ground_quads: ground.len(),
        full_ground_quads: geometry.quads.len(),
        cpu_ground: Duration::ZERO,
        cpu_composites: Duration::ZERO,
        cpu_ground_detail: Duration::ZERO,
        ground_detail_cpu_uniforms: Duration::ZERO,
        ground_detail_cpu_serialize: Duration::ZERO,
        ground_detail_cpu_upload: Duration::ZERO,
        ground_detail_cpu_pass: Duration::ZERO,
        cpu_statics: Duration::ZERO,
        cpu_items: Duration::ZERO,
        composite_bindings_created: 0,
        composite_bindings_reused: 0,
        composite_cpu_upload: Duration::ZERO,
        composite_cpu_bindings: Duration::ZERO,
        composite_cpu_pass: Duration::ZERO,
    };
    if composite_lod == BlockLod::Lod0 && !ready.is_empty() {
        tracing::error!(?audit, "LOD0 world pass selected cached map blocks");
    }
    let (map_static_rows, map_statics_drawn) = geometry.detail_map_statics(&cached_blocks);
    // The ordinary all-live path clears and draws at once. With a cache, clear
    // first but defer the live slope/rim rows until after restore: their exact
    // current-frame depth must be able to beat a neighbouring cached flat tile
    // where the two diamonds overlap.
    let ground_started = Instant::now();
    if ready.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "ground", encoder);
        window
            .renderer
            .render(&window.device, &window.queue, encoder, target, &ground);
        profile::end(window.gpu.as_ref(), encoder, timed);
    } else {
        window
            .renderer
            .render(&window.device, &window.queue, encoder, target, &[]);
    }
    let cpu_ground = ground_started.elapsed();
    // Cached flat ground is restored before all live sprite passes. Map statics
    // deliberately remain live: their art can overhang an 8×8 ground block by
    // more than the bounded composite margin, while this shared depth buffer
    // still orders them against every cached ground pixel.
    let composite_blocks: Vec<_> = ready
        .iter()
        .map(|(_, rect, texture)| CompositeQuad { texture, rect: *rect })
        .collect();
    let (eye_x, eye_y) = camera.eye_tile();
    let current_depth_base = openshard_client_render::depth::base_for(eye_x, eye_y);
    let composites_started = Instant::now();
    let timed = profile::begin(window.gpu.as_ref(), "map composites", encoder);
    // `render_deferred` keeps the depth-base correction per block instance, so
    // the full far-zoom set is one upload/call. Its own per-call batch remains
    // immutable until this encoder is submitted, preserving the artifact fix
    // for multiple deferred calls in one command buffer.
    window.composite_pass.begin_frame();
    window.composite_pass.render_deferred_rebased(
        &window.device,
        &window.queue,
        encoder,
        target,
        current_depth_base,
        &composite_blocks,
    );
    (audit.composite_bindings_created, audit.composite_bindings_reused) =
        window.composite_pass.deferred_binding_stats();
    let composite_cpu = window.composite_pass.deferred_cpu_costs();
    audit.composite_cpu_upload = composite_cpu.upload;
    audit.composite_cpu_bindings = composite_cpu.bindings;
    audit.composite_cpu_pass = composite_cpu.pass;
    profile::end(window.gpu.as_ref(), encoder, timed);
    let ground_detail_started = Instant::now();
    if !ready.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "ground detail", encoder);
        window
            .renderer
            .render_loaded(&window.device, &window.queue, encoder, target, &ground);
        let ground_detail_cpu = window.renderer.last_cpu_costs();
        audit.ground_detail_cpu_uniforms = ground_detail_cpu.uniforms;
        audit.ground_detail_cpu_serialize = ground_detail_cpu.serialize;
        audit.ground_detail_cpu_upload = ground_detail_cpu.upload;
        audit.ground_detail_cpu_pass = ground_detail_cpu.pass;
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    audit.cpu_ground_detail = ground_detail_started.elapsed();
    let cpu_composites = composites_started.elapsed();
    // Handed over every frame rather than on the key, because the key does
    // not have the window: `graphics.fringe` is the switch and the pass is where
    // it is read, and a state pushed once at start-up would leave F2 silent.
    window.statics.set_fringe(graphics.fringe);
    window.items_pass.set_fringe(graphics.fringe);
    let statics_started = Instant::now();
    let timed = profile::begin(window.gpu.as_ref(), "statics", encoder);
    window.statics.render(
        &window.device,
        &window.queue,
        encoder,
        target,
        map_static_rows,
        &geometry.mesh.boxes,
        Some(map_statics_drawn),
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    let cpu_statics = statics_started.elapsed();
    // Composite entries are potentially eight RGBA-sized planes each.  Keep a
    // bounded LRU tail, but protect one block outside the current viewport so
    // a small pan cannot turn into an upload/pan-back loop.  This maintenance
    // never builds pixels and therefore cannot add a synchronous camera-frame
    // composition cost.
    let _evicted_composites = window.composites.evict_lru_outside_viewport(composite_visible);
    // Server items intentionally run after immutable map statics.  The shared
    // depth buffer preserves their historical interleaving, while keeping this
    // buffer free of map rows is what lets a cached map composite keep a stable
    // G-buffer identity without making a dynamic item point at a stale row.
    let items_started = Instant::now();
    let timed = profile::begin(window.gpu.as_ref(), "items", encoder);
    window.items_pass.render_with_id_bits(
        &window.device,
        &window.queue,
        encoder,
        target,
        &geometry.item_instances.rows,
        &[],
        Some(geometry.item_instances.drawn),
        openshard_client_render::gbuffer::IDS_DYNAMIC_ITEM,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    let cpu_items = items_started.elapsed();
    audit.cpu_ground = cpu_ground;
    audit.cpu_composites = cpu_composites;
    audit.cpu_statics = cpu_statics;
    audit.cpu_items = cpu_items;
    // Right after statics, into the same static's own pixels its
    // billboard sprite just drew — `docs/gbuffer.md` step 4c. Depth and
    // place only, never colour: this only gives a climbable static's
    // pixels a more honest per-face normal than one blended stance could.
    let timed = profile::begin(window.gpu.as_ref(), "mesh faces", encoder);
    window.mesh_pass.render(
        &window.device,
        &window.queue,
        encoder,
        target,
        &geometry.mesh.mesh_vertices,
        &geometry.mesh.mesh_rows,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    let timed = profile::begin(window.gpu.as_ref(), "mobiles", encoder);
    window.mobile_pass.render(
        &window.device,
        &window.queue,
        encoder,
        target,
        &geometry.mobile_quads,
        // A mobile has no volume — `docs/lighting_rebuild.md` says so in as
        // many words, and phase 7 is what gives a billboard a normal.
        &[],
        None,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    // The architectural cutaway comes after mobiles so its depth test sees the
    // settled opaque world. It writes its *own* picture and G-buffer but reads
    // the main depth without writing it; its separately lit colour is composed
    // after the main deferred blit below.
    if geometry.cutaway_instances.drawn != 0 {
        let timed = profile::begin(window.gpu.as_ref(), "cutaway geometry", encoder);
        let cutaway_target = Target {
            view: cutaway_world_view,
            depth: target.depth,
            gbuffer: cutaway_gbuffer_views,
            width: target.width,
            height: target.height,
            projection: target.projection,
        };
        window.statics.render_cutaway(
            &window.device,
            &window.queue,
            encoder,
            cutaway_target,
            &geometry.cutaway_instances.rows,
            &geometry.cutaway_boxes,
            geometry.cutaway_instances.drawn,
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    // The silhouettes, here and not later: the mask is depth-tested against
    // what the three world passes have drawn, so a barrel behind a wall is
    // kept out of it — and the text pass below writes depth at the near
    // plane over everything, which would punch the mask through.
    let mask_view = window
        .outline_mask
        .create_view(&wgpu::TextureViewDescriptor::default());
    // One item is one ring; the pass numbers groups, so each quad is a group
    // of its own — see `SpriteRenderer::render_mask`.
    let item_rings: Vec<&[SpriteQuad]> = geometry.outline_quads.iter().map(std::slice::from_ref).collect();
    let timed = profile::begin(window.gpu.as_ref(), "outline mask: items", encoder);
    window.statics.render_mask(
        &window.device,
        &window.queue,
        encoder,
        target,
        &mask_view,
        &item_rings,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    // And a creature through its own atlas, in *one* group: a body and
    // everything it wears is one thing being pointed at, and one ring goes
    // round the lot. This pass clears the mask too, which is why it is
    // skipped when nothing is ringed — the items' pass above has already
    // written the frame's answer, and a second clear would erase it.
    if !geometry.mobile_outline.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "outline mask: mobiles", encoder);
        window.mobile_pass.render_mask(
            &window.device,
            &window.queue,
            encoder,
            target,
            &mask_view,
            &[&geometry.mobile_outline],
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    // And the held selection into its own mask, through the same pass and
    // the same depth buffer: what is washed is what is *visible* of the
    // selected static, so a wall the player has walked behind is not painted
    // over the thing now in front of it. One group, because a selection is
    // one thing — the pass numbers groups for the ring's sake and the wash
    // reads only "is this texel nought".
    let select_view = window
        .select_mask
        .create_view(&wgpu::TextureViewDescriptor::default());
    if !geometry.select_quads.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "select mask", encoder);
        window.statics.render_mask(
            &window.device,
            &window.queue,
            encoder,
            target,
            &select_view,
            &[&geometry.select_quads],
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    // And the combat target (or, with no target, the held mobile/item) into
    // `Screen::held_mask` — the same two-pass shape as the hover ring
    // above (items first, unconditionally, so an empty frame still clears
    // the mask; the mobile pass gated because it clears the mask too).
    // Not folded into the hover mask above: a click's ring must survive
    // the cursor moving off the thing, and that mask is overwritten fresh
    // every frame from whatever the cursor is over *this* frame alone.
    let held_view = window
        .held_mask
        .create_view(&wgpu::TextureViewDescriptor::default());
    let held_item_rings: Vec<&[SpriteQuad]> = geometry
        .held_item_outline
        .iter()
        .map(std::slice::from_ref)
        .collect();
    let timed = profile::begin(window.gpu.as_ref(), "held mask: items", encoder);
    window.statics.render_mask(
        &window.device,
        &window.queue,
        encoder,
        target,
        &held_view,
        &held_item_rings,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    if !geometry.held_mobile_outline.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "held mask: mobiles", encoder);
        window.mobile_pass.render_mask(
            &window.device,
            &window.queue,
            encoder,
            target,
            &held_view,
            &[&geometry.held_mobile_outline],
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    // Always `text_pass`, `fonts.mul`'s own: `text_quads` is empty
    // whenever `App::ttf_font` is set, since a TrueType face's speech
    // draws after the blit instead — see `screen_speech`'s own comment
    // above and the render call after it, below.
    let timed = profile::begin(window.gpu.as_ref(), "overhead text", encoder);
    window.text_pass.render(
        &window.device,
        &window.queue,
        encoder,
        target,
        text_quads,
        &[],
        None,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    // And the world image onto the surface, into the rect the panels left
    // free. Magnified this is a copy — the image is already the viewport's
    // size and the magnification happened in the vertex transform — and
    // minified it is where the shrinking happens, which is why the zoom is
    // still what picks the sampler.
    //
    // The lighting — the flames, the sun, the lantern in the player's hand
    // and which of the pass's own values is drawn — was assembled at the top
    // of the frame, out of `frame::Inputs`. Nothing between there and here
    // may touch it: a frame this client draws and a frame a tool dumps are
    // the same frame only for as long as neither of them has an adjustment
    // of its own afterwards. `docs/parity.md`.
    //
    // **Solids alone**, `App::solids_only`: the surface is cleared and the
    // world image is not drawn onto it at all, so the boxes below stand
    // over nothing that could be mistaken for their own shape. `lighting`
    // is unaffected either way — it is what the solids pass reads its grid
    // from, and it was already built above whichever branch runs here.
    if graphics.solids_only && graphics.show_solids {
        let timed = profile::begin(window.gpu.as_ref(), "solids-only clear", encoder);
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("solids-only clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(openshard_client_render::renderer::CLEAR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        profile::end(window.gpu.as_ref(), encoder, timed);
    } else {
        // **The pass to watch.** Deferred shading over the whole viewport:
        // every light in range walked per fragment, the sun, the occlusion
        // grid. `tests/cost.rs` measures it offline; this is the same pass
        // on the frame as played.
        let timed = profile::begin(window.gpu.as_ref(), "blit: lighting", encoder);
        window.blit.render(
            &window.device,
            &window.queue,
            encoder,
            blit::Frame {
                target: view,
                world: world_view,
                gbuffer: gbuffer_views,
                face_instances: window.statics.instances_buffer(),
                item_instances: window.items_pass.instances_buffer(),
                mobile_instances: window.mobile_pass.instances_buffer(),
                mesh_instances: window.mesh_pass.rows_buffer(),
                ground_instances: window.renderer.instances_buffer(),
                zoom: camera.zoom(),
                rect: viewport,
            },
            &geometry.lighting,
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
        if geometry.cutaway_instances.drawn != 0 {
            let timed = profile::begin(window.gpu.as_ref(), "cutaway lighting", encoder);
            window.blit.render_cutaway(
                &window.device,
                &window.queue,
                encoder,
                blit::Frame {
                    target: view,
                    world: cutaway_world_view,
                    gbuffer: cutaway_gbuffer_views,
                    face_instances: window.statics.cutaway_instances_buffer(),
                    item_instances: window.items_pass.instances_buffer(),
                    mobile_instances: window.mobile_pass.instances_buffer(),
                    mesh_instances: window.mesh_pass.rows_buffer(),
                    ground_instances: window.renderer.instances_buffer(),
                    zoom: camera.zoom(),
                    rect: viewport,
                },
                &geometry.lighting,
                1.0,
            );
            profile::end(window.gpu.as_ref(), encoder, timed);
        }
    }
    // The occlusion grid as solids, when somebody asked for it — step 23.0.
    // First of what is drawn over the lit picture, so the highlights stay on
    // top of it: a diagnostic must not hide the thing the cursor is naming.
    //
    // The grid drawn is the frame's **own** — `lighting.occlusion`, which is
    // the list the shader is walking this same frame — and not a second walk
    // of the map. A picture of a grid rebuilt beside the one in force would
    // be a claim about a grid nothing rendered.
    if graphics.show_solids {
        let standing = openshard_client_render::solid::standing(&geometry.lighting.occlusion, solid_cut);
        graphics.solids_held = standing.len();
        let timed = profile::begin(window.gpu.as_ref(), "solids", encoder);
        graphics.solids_drawn = window.solids.render(
            &window.device,
            &window.queue,
            encoder,
            solids::Frame {
                target: view,
                size: (window.config.width, window.config.height),
                rect: viewport,
            },
            &camera,
            &standing,
            solids::Style {
                opaque: graphics.solids_opaque,
                ..solids::Style::default()
            },
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    // The held selection's wash, first of the two things drawn over the lit
    // picture: the wall the click named and the ground it stands on. Under
    // the ring rather than over it, because they answer different questions
    // — the wash is what is *held* and the ring is what the cursor is on —
    // and the live one has to stay readable while it passes over the held
    // one.
    //
    // Skipped when nothing is selected, and the whole cost of a frame with
    // nothing selected is that comparison: the mask is not drawn either.
    if let Some(picked) = picking
        .selected
        .and_then(SelectedIdentity::as_static)
        .filter(|_| !geometry.select_quads.is_empty())
    {
        let timed = profile::begin(window.gpu.as_ref(), "selection wash", encoder);
        window.select.render(
            &window.device,
            &window.queue,
            encoder,
            select::Frame {
                target: view,
                mask: &select_view,
                ids: &gbuffer_views.ids,
                face_instances: window.statics.instances_buffer(),
                ground_instances: window.renderer.instances_buffer(),
                size: (render_width, render_height),
                rect: viewport,
            },
            // The tile the *static* stands on, and not `selected_tile`: the
            // ground being washed is the ground under the thing that was
            // picked, which is the whole of "and the tile it stands on". The
            // two are usually different tiles — a wall's picture stands up
            // the screen from its own cell, so the ground under the cursor is
            // the cell behind it.
            Selection::DEFAULT.on(openshard_movement::Tile::new(picked.at.x, picked.at.y)),
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    // And the ring on top of that, over the same rectangle — after the blit
    // so it is drawn in screen pixels and unlit: a highlight that dimmed at
    // night would stop working exactly when the picture is hardest to read.
    // Skipped entirely on the ordinary frame, where nothing is under the
    // cursor and the mask is empty. **Both silhouette lists**, or a ringed
    // creature draws its mask into a texture no pass ever reads and the
    // highlight is simply absent — which is what an item-only test of this
    // condition looked like from the outside.
    // The held ring, drawn first of the two so the live hover ring stays
    // on top and readable when the cursor is over the very thing that is
    // selected — the same ordering the wash and the hover ring keep,
    // and for the same reason. `Ring::SELECTED`'s own pipeline call: one
    // [`Ring`] per `Outline::render`, so the held ring's colour cannot be
    // the hover ring's even for one frame.
    if !geometry.held_item_outline.is_empty() || !geometry.held_mobile_outline.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "held ring", encoder);
        window.outline.render(
            &window.device,
            &window.queue,
            encoder,
            outline::Frame {
                target: view,
                mask: &held_view,
                mask_size: (render_width, render_height),
                rect: viewport,
            },
            Ring::SELECTED.for_zoom(camera.zoom()),
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    if !geometry.outline_quads.is_empty() || !geometry.mobile_outline.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "outline ring", encoder);
        window.outline.render(
            &window.device,
            &window.queue,
            encoder,
            outline::Frame {
                target: view,
                mask: &mask_view,
                mask_size: (render_width, render_height),
                rect: viewport,
            },
            // The soft ring — an edge with a glow behind it — widened when
            // the world is minified, where one mask texel is less than one
            // screen pixel and a hairline breaks into a dashed line. See
            // `Ring::for_zoom`.
            Ring::SOFT.for_zoom(camera.zoom()),
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
    }
    audit
}

#[cfg(test)]
mod tests {
    use super::container_hover_text;
    use openshard_protocol::containers::{ContainedItem, GridSlot};
    use openshard_protocol::gump::GumpPoint;
    use openshard_protocol::items::ItemAmount;
    use openshard_protocol::serial::Serial;
    use openshard_protocol::wire::{Graphic, Hue};

    #[test]
    fn a_container_hover_names_the_item_and_its_amount() {
        let gold = ContainedItem {
            serial: Serial::new(0x4000_0001).expect("an item serial"),
            graphic: Graphic(0x0EED),
            amount: ItemAmount(137),
            at: GumpPoint::new(60, 60),
            grid: GridSlot(0),
            hue: Hue(0),
        };

        assert_eq!(container_hover_text(&gold, "gold coins"), "gold coins (137)");
    }
}
