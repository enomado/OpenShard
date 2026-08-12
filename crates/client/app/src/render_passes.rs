//! The GPU passes a frame's already-collected geometry is recorded through:
//! [`encode_world_passes`] is the world — ground, statics, mobiles, the
//! masks and rings — and [`draw_gump_windows`] is this client's own dialogs,
//! drawn over it in the client's own art. Neither decides what is drawn;
//! `crate::frame_geometry` and `App::draw_from` do that, and hand the
//! answer here to be recorded.

use openshard_client_render::blit::{self, ViewportRect};
use openshard_client_render::camera::Camera;
use openshard_client_render::gump::{self as gump_art};
use openshard_client_render::outline::{self, Ring};
use openshard_client_render::renderer::Target;
use openshard_client_render::select::{self, Selection};
use openshard_client_render::sprite::SpriteQuad;
use openshard_client_render::{container, paperdoll, skills, solids, status};
use openshard_protocol::containers::ContainedItem;

use crate::frame_geometry::FrameGeometry;
use crate::picking::{self, SelectedIdentity};
use crate::window::Screen;
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
pub(crate) fn draw_gump_windows(
    resources: &mut resources::Resources,
    world: &world::WorldState,
    windows: &mut windows::Windows,
    shell: Option<&shell::Shell>,
    window: &mut Screen,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) {
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
            for open in &windows.own_windows {
                match open.subject {
                    WindowSubject::Dialog(gump_id) => {
                        let Some(gump) = view.gumps.iter().find(|gump| gump.gump_id == gump_id) else {
                            continue;
                        };
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
                                        value: line.value,
                                        cap: line.cap,
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
                        drawn_windows.push((
                            open.subject,
                            Drawn::Status(status::window(
                                status::Standing {
                                    name: &status.name,
                                    female: status.female,
                                    strength: status.strength,
                                    dexterity: status.dexterity,
                                    intelligence: status.intelligence,
                                    hits,
                                    stamina: status.stamina,
                                    mana: status.mana,
                                    gold: status.gold,
                                    armor: status.armor,
                                    weight: status.weight,
                                    max_weight: status.max_weight,
                                },
                                open.at,
                            )),
                        ));
                    }
                    WindowSubject::Container(serial) => {
                        let Some(gump) = view.containers.get(&serial).copied() else {
                            continue;
                        };
                        let contents: Vec<ContainedItem> =
                            view.contents.get(&serial).cloned().unwrap_or_default();
                        drawn_windows.push((
                            open.subject,
                            Drawn::Container(container::window(gump, &contents, open.at)),
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
                        drawn_windows.push((
                            open.subject,
                            Drawn::Paperdoll(paperdoll::window(
                                wearer.as_ref(),
                                whose,
                                held,
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
            pictures.extend(window.pictures().iter().copied());
        }
        // What the pointer is tested against from here on, and the atlas it
        // is tested in is the one just grown for it: the hit test and the
        // frame are now the same list. Kept even when it is empty — the
        // windows this frame drew none of are windows nothing can click.
        windows.drawn_windows = drawn_windows;
        if let Some(rows) = resources.gump_atlas.take_dirty() {
            pass.upload_rows(&window.queue, resources.gump_atlas.pixels(), rows);
        }
        let quads = gump_art::collect(&pictures, &resources.gump_atlas);
        let timed = profile::begin(window.gpu.as_ref(), "gump art", encoder);
        pass.render(
            &window.device,
            &window.queue,
            encoder,
            gump_art::Frame {
                target: view,
                width: window.config.width,
                height: window.config.height,
                // A whole number, and the same one egui is laying its
                // widgets out at: gump art is five-bit pixel art sampled
                // with Nearest, and a fractional scale doubles some of its
                // rows and not others.
                // egui's own, and not the window's scale factor rounded:
                // the art is placed at coordinates egui laid out in
                // points, so any other number here slides a window's
                // pictures off its buttons.
                scale: shell.map(|shell| shell.pixels_per_point()).unwrap_or(1.0),
            },
            &quads,
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
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
    viewport: ViewportRect,
    camera: Camera,
    solid_cut: openshard_client_render::solid::Cut,
    geometry: &FrameGeometry,
    text_quads: &[SpriteQuad],
    render_width: u32,
    render_height: u32,
) {
    // Ground first, because it clears; statics after, into what it left.
    // Which covers which is decided by the depth they share, not by this
    // order — the order only decides who clears.
    //
    // Every pass from here to the submit is bracketed by `profile::begin`
    // and `profile::end`: the GPU's own timestamps, which are the one half
    // of a frame's cost no clock on this thread can see. See [`profile`] for
    // why that is so and why the bracket is a pair of calls rather than a
    // scope guard. Nothing when the adapter has no timestamp queries.
    let timed = profile::begin(window.gpu.as_ref(), "ground", encoder);
    window
        .renderer
        .render(&window.device, &window.queue, encoder, target, &geometry.quads);
    profile::end(window.gpu.as_ref(), encoder, timed);
    // Handed over every frame rather than on the key, because the key does
    // not have the window: `graphics.fringe` is the switch and the pass is where
    // it is read, and a state pushed once at start-up would leave F2 silent.
    window.statics.set_fringe(graphics.fringe);
    let timed = profile::begin(window.gpu.as_ref(), "statics", encoder);
    window.statics.render(
        &window.device,
        &window.queue,
        encoder,
        target,
        &geometry.static_instances.rows,
        &geometry.mesh.boxes,
        Some(geometry.static_instances.drawn),
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
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
    // And the held mobile or item's own ring silhouette, into
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
                mobile_instances: window.mobile_pass.instances_buffer(),
                mesh_instances: window.mesh_pass.rows_buffer(),
                ground_instances: window.renderer.instances_buffer(),
                zoom: camera.zoom(),
                rect: viewport,
            },
            &geometry.lighting,
        );
        profile::end(window.gpu.as_ref(), encoder, timed);
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
            Selection::DEFAULT.on((picked.at.x, picked.at.y)),
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
}
