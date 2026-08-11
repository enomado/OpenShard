//! What is drawn, and only that.
//!
//! [`App::draw`] and [`App::advance`] are the two halves of a frame — the
//! clock and the picture — and [`App::draw_from`] is the picture itself:
//! atlases grown, geometry assembled, every pass encoded. [`App::frame_facts`]
//! is the one place a pick happens, because the highlight and the tile marker
//! have to agree with what was actually drawn. [`assemble_geometry`] and the
//! free functions beside it are kept free rather than folded into
//! `draw_from` on purpose — see that method's own doc for the borrow-checker
//! reason.
//!
//! **A pure reader of command state.** Nothing here writes a walk target, a
//! gump's contents or anything a packet fills in — that is `net_command.rs`'s
//! and `ui_command.rs`'s and `own_windows.rs`'s job, upstream of a frame. What
//! this file *does* still mutate is purely presentational: animation clocks,
//! the atlases, the frame counters — state about how a picture is drawn, not
//! about what is true.

use std::sync::Arc;
use std::time::Instant;

use openshard_client_render::atlas::{AnimAtlas, AtlasError};
use openshard_client_render::blit::{self, Blit, ViewportRect};
use openshard_client_render::camera::{Camera, TileBounds, ViewPixel};
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::debug::View;
use openshard_client_render::frame::{self, Impostor};
use openshard_client_render::gbuffer::Gbuffer;
use openshard_client_render::gump::{self as gump_art, GumpPixel};
use openshard_client_render::items::{self};
use openshard_client_render::mobiles::{self, Mobile};
use openshard_client_render::outline::{self, Ring};
use openshard_client_render::renderer::{self, Target};
use openshard_client_render::select::{self, Selection};
use openshard_client_render::skills;
use openshard_client_render::solids;
use openshard_client_render::sprite::{SpriteQuad, split_corners};
use openshard_client_render::text::{self, GumpLabel, Label};
use openshard_client_render::{container, ground, light, paperdoll, statics};
use openshard_protocol::containers::ContainedItem;
use openshard_protocol::speech::Font;
use openshard_protocol::wire::Hue;
use openshard_uofiles::map::Map;
use openshard_uofiles::tiledata::TileData;

use crate::app::App;
use crate::crowd::{self, Crowd, Who};
use crate::picking::{self, Pick, SelectedIdentity};
use crate::window::{Atlases, Screen, Wanted, wanted_in};
use crate::windows::{Drawn, WindowSubject};
use crate::{
    CHAT_LINE_HEIGHT, CHAT_LINES, CHAT_MARGIN, clutter, desk, graphics, profile, resources,
    scaled_gump_quads, shell, windows, world,
};

/// The speech line: what has not been said yet, and whether the keyboard is
/// listening for it.
///
/// Lives on `App` rather than the HUD now — see `shell::Shell`'s old `typed`
/// field — because typing into it has to win the keyboard *before* a letter
/// is read as a hotkey or a walk key, which is a decision `App::window_event`
/// makes and the HUD no longer does.
#[derive(Default, Debug)]
pub(crate) struct Chat {
    /// What has been typed and not yet sent, in bytes: `fonts.mul` is drawn
    /// per byte (see `text::collect`), and every cursor and edit position here
    /// is a byte offset into this string for exactly that reason — a `char`
    /// index would have to be translated back at every glyph anyway.
    pub(crate) typed: String,
    /// Where the caret sits: a byte offset into `typed`, always on a `char`
    /// boundary.
    pub(crate) cursor: usize,
    /// Whether a keystroke that is not a hotkey reaches this line rather than
    /// the character. Opened by Enter, the reference client's own gesture —
    /// there is no mouse hit test for it, so nothing else about picking has
    /// to change for this to work.
    pub(crate) focused: bool,
}

impl Chat {
    /// Insert typed text at the caret and move the caret past it.
    pub(crate) fn insert(&mut self, text: &str) {
        self.typed.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    /// Delete the `char` before the caret, if any.
    pub(crate) fn backspace(&mut self) {
        let Some(before) = self.typed[..self.cursor].chars().next_back() else {
            return;
        };
        let start = self.cursor - before.len_utf8();
        self.typed.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// Delete the `char` after the caret, if any.
    pub(crate) fn delete(&mut self) {
        let Some(after) = self.typed[self.cursor..].chars().next() else {
            return;
        };
        let end = self.cursor + after.len_utf8();
        self.typed.replace_range(self.cursor..end, "");
    }

    /// Move the caret one `char` left, if it is not already at the start.
    pub(crate) fn left(&mut self) {
        if let Some(before) = self.typed[..self.cursor].chars().next_back() {
            self.cursor -= before.len_utf8();
        }
    }

    /// Move the caret one `char` right, if it is not already at the end.
    pub(crate) fn right(&mut self) {
        if let Some(after) = self.typed[self.cursor..].chars().next() {
            self.cursor += after.len_utf8();
        }
    }

    /// Take the typed line and close it back to empty, or `None` for a stray
    /// Enter on nothing worth sending — the same rule `shell::speech_line` had:
    /// an empty message is not silence worth sending, it is the server drawing
    /// nothing over the player's head.
    pub(crate) fn take(&mut self) -> Option<String> {
        let line = std::mem::take(&mut self.typed);
        self.cursor = 0;
        (!line.trim().is_empty()).then_some(line)
    }
}

/// The client's own map, unclutted by anything the shard has stood on it —
/// [`cluttered`] is what a step decision should actually ask.
///
/// A facade over [`resources::Resources::map`] and
/// [`resources::Resources::tiledata`], which travel together in every caller
/// that wants either: the split that put them in `Resources` and
/// [`world::WorldState::clutter`] in `WorldState` is about what changes
/// together (disk-read once against updated every packet), not about who
/// asks for them together, and this is the seam that reunites the two.
///
/// **A free function taking `&Resources`, not an `App` method taking
/// `&self`.** A method on `App` borrows the whole of it, so a caller that
/// also holds `&mut self.steer` beside the terrain — every arrow key and
/// every replanned step does — could no longer compile: the borrow checker
/// sees disjoint *fields* through a chain of `.` projections but not through
/// a method call, which is opaque to it. Passing `&self.resources` here is
/// the same projection the field access always was, just wrapped.
pub(crate) fn terrain(resources: &resources::Resources) -> openshard_movement::MapTerrain<&Map, &TileData> {
    openshard_movement::MapTerrain::new(resources.map.as_ref(), &resources.tiledata)
}

/// [`terrain`] with the shard's own items laid over it — what every step
/// decision on this end should actually ask. See [`clutter::Clutter::over`],
/// and `terrain`'s own docs for why this takes references rather than being
/// an `App` method.
pub(crate) fn cluttered<'a>(
    world: &'a world::WorldState,
    resources: &'a resources::Resources,
) -> clutter::Cluttered<'a, openshard_movement::MapTerrain<&'a Map, &'a TileData>> {
    world.clutter.over(&resources.map, &resources.tiledata)
}

/// The same, read as though every shut door on it stood open: what a route
/// may be *planned* through, and never what decides a step. See
/// [`clutter::Clutter::over_with_doors_open`].
pub(crate) fn cluttered_with_doors_open<'a>(
    world: &'a world::WorldState,
    resources: &'a resources::Resources,
) -> clutter::Cluttered<'a, openshard_movement::MapTerrain<&'a Map, &'a TileData>> {
    world
        .clutter
        .over_with_doors_open(&resources.map, &resources.tiledata)
}

/// Grows or, on eviction, wholly rebuilds `window`'s atlases so this frame's
/// picture has everywhere it needs already packed before anything reads them
/// — see `App::draw_from`'s Step three doc for where this call sits.
///
/// A free function and not a method on `App`: this is the one part of
/// presenting a frame that really does write `self` — `resources.anim`,
/// `graphics.covered`, `repacks` — and by taking exactly those fields rather
/// than `&mut self` it stays legible from its signature alone that nothing
/// else on `App` moves here. The same reasoning
/// [`picking::SelectedIdentity::as_static`]'s doc gives for being a free
/// function rather than a method.
///
/// Returns whether a full rebuild ran, for [`frames::Frame::repacked`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn ready_atlases(
    resources: &mut resources::Resources,
    graphics: &mut graphics::GraphicsSettings,
    world: &world::WorldState,
    repacks: &mut u64,
    window: &mut Screen,
    want: TileBounds,
    wanted: &Wanted,
    drawn: &[(Who, Mobile)],
) -> bool {
    // Set only on a successful rebuild — the counter `docs/camera.md`
    // asks for, so the frame that stalled for one can be told apart from
    // one that is merely heavy. See [`Frame::repacked`](frames::Frame).
    let mut repacked = false;
    // Full rebuild and not grow, either because the atlas just filled up
    // (the ordinary eviction) or because a debug edit changed a shape the
    // atlas already has packed and `grow` cannot see that on its own — it
    // only asks whether a graphic is packed *at all* (its own doc), so a
    // stair already on screen when its prism changes would never be
    // re-offered. Both land in the same rebuild, for the same reason: the
    // texture a bind group points at is the one the old atlas was
    // uploaded to.
    let evict = if resources.repack_forced {
        resources.repack_forced = false;
        true
    } else {
        // Grow rather than rebuild. What is new is added to the textures
        // already bound, a band of rows at a time, and a frame where the
        // camera stood still reads four `BTreeSet`s and touches no file
        // and no GPU.
        let grown = window.atlases.grow(
            &resources.art,
            &resources.texmaps,
            &resources.tiledata,
            &mut resources.anim,
            wanted,
        );
        // Whatever was packed is uploaded, including on the way out of a
        // failure: a growth that stopped part way still wrote pixels, and
        // pixels the device has not been told about are sampled as
        // whatever was there before. Cheap to do unconditionally — the
        // band is empty when nothing grew — and it is one fewer path
        // where an atlas and its texture can disagree.
        window.atlases.upload(
            &window.queue,
            &window.renderer,
            &window.statics,
            &window.mobile_pass,
        );
        match grown {
            Ok(()) => {
                graphics.covered = Some(want);
                false
            }
            Err(AtlasError::Full { .. }) => true,
            Err(error) => {
                eprintln!("growing the atlases: {error}");
                false
            }
        }
    };
    if evict {
        // Costly and rare on the ordinary path — where the old
        // arrangement paid it every few tiles — and it is the *only*
        // thing that reclaims space, so an atlas that only ever grew
        // would eventually stay full for ever. Cheap and deliberate on
        // the debug-edit path: one static's worth of texture, asked for
        // once per slider change.
        //
        // `covered` is cleared first: a rebuild forgets, so the next
        // frame may not assume anything about what the atlases hold. Set
        // again below only if the rebuild succeeds.
        graphics.covered = None;
        match Atlases::build(
            &resources.art,
            resources.surfaces.as_ref(),
            &resources.texmaps,
            &resources.tiledata,
            &mut resources.anim,
            &wanted_in(
                &resources.map,
                [want],
                &world.items,
                &drawn.iter().map(|(_, mobile)| mobile.clone()).collect::<Vec<_>>(),
                &world.tile_animations,
                &resources.equip_conv,
            ),
        ) {
            Ok(atlases) => {
                window.install_atlases(atlases, &resources.hue_ramp);
                graphics.covered = Some(want);
                repacked = true;
                *repacks += 1;
            }
            // One screen does not fit one atlas, which is a different
            // statement from "the atlas filled up": no eviction can help
            // and the frame draws with sprites missing. Named here rather
            // than hidden, and it is what the standing backlog item about
            // a failed repack is about.
            Err(error) => eprintln!("packing the art on screen: {error}"),
        }
    }
    repacked
}

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
        if let Some(view) = world.view.as_ref() {
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
                                |text, font| text::gump_width(text, font, &resources.font_atlas),
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

/// The speech line and the journal above it, over the finished picture and
/// under egui's — the same corner `shell::speech_line`'s `egui::Panel::bottom`
/// used to claim before this moved to the client's own rendering. Always
/// drawn, unlike [`draw_gump_windows`]: the font atlas needs no shard-sent
/// gump art to exist, so there is nothing here to be `None` until.
///
/// The plainest of this frame's free functions: every parameter is `&`, and
/// the one exception — `text_quads` — is appended to rather than replaced,
/// so the caller keeps owning the one instance buffer `GumpRenderer` has
/// room for (see the comment at this call's site in `App::draw_from`).
/// Nothing here is written back to `self` at all.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_chat_and_speech(
    resources: &resources::Resources,
    world: &world::WorldState,
    chat: &Chat,
    shell: Option<&shell::Shell>,
    window: &mut Screen,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    chat_style: desk::Chat,
    screen_speech: &[text::ScreenLabel<'_>],
    text_quads: &mut Vec<SpriteQuad>,
) {
    let scale = shell.map(|shell| shell.pixels_per_point()).unwrap_or(1.0);
    // The surface's size in gump pixels rather than real ones —
    // `Frame::scale`'s doc is what the one below multiplies out, and
    // this is that arithmetic done once for where the corner is
    // rather than for every quad in it.
    let canvas = GumpPixel::new(
        (window.config.width as f32 / scale) as i32,
        (window.config.height as f32 / scale) as i32,
    );
    let font = Font::DEFAULT;
    // The TrueType path draws at [`TTF_BASE_PIXEL_HEIGHT`] regardless
    // of [`desk::ChatScale`] — see [`scaled_gump_quads`]'s doc for
    // why an integer upscale is right for `fonts.mul` and wrong for an
    // antialiased face — so the line spacing only grows when the
    // glyphs it is spacing actually will.
    let line_height = match resources.ttf_font {
        Some(_) => CHAT_LINE_HEIGHT,
        None => CHAT_LINE_HEIGHT * chat_style.scale.raw() as i32,
    };
    let input_at = GumpPixel::new(CHAT_MARGIN, canvas.y - CHAT_MARGIN - line_height);

    // Owned before it is borrowed into `GumpLabel`s: the journal's own
    // strings are formatted here (name and text joined the way
    // `shell::Hud::said` used to) and the prompt is built from
    // the caller's own chat, so both need somewhere to live for the length of
    // `collect_gump`'s borrow.
    let mut rows: Vec<(GumpPixel, Hue, Font, String)> = Vec::new();
    if let Some(view) = world.view.as_ref() {
        for (row, line) in view.journal.iter().rev().take(CHAT_LINES).enumerate() {
            let at = GumpPixel::new(CHAT_MARGIN, input_at.y - line_height * (row as i32 + 1));
            let text = match line.name.is_empty() {
                true => line.text.clone(),
                false => format!("{}: {}", line.name, line.text),
            };
            rows.push((at, line.hue, line.font, text));
        }
    }
    let prompt = match chat.focused {
        true => format!("say: {}", chat.typed),
        // A hint and not an empty line: there is no mouse click to
        // discover this by any more (see `App::window_event`'s
        // `KeyCode::Enter` arm), so the one thing worth saying here is
        // the key that opens it.
        false => "[Enter] say".to_owned(),
    };
    let mut labels: Vec<GumpLabel<'_>> = rows
        .iter()
        .map(|(at, hue, font, text)| GumpLabel {
            at: *at,
            text,
            font: *font,
            hue: *hue,
            clip: None,
        })
        .collect();
    labels.push(GumpLabel {
        at: input_at,
        text: &prompt,
        font,
        hue: Hue(chat_style.hue),
        clip: None,
    });
    // The caret, a lone glyph rather than a new quad primitive: the
    // gump pass draws through an atlas of packed sprites and has
    // nothing that paints a solid rectangle, and `fonts.mul` already
    // has a `|` to stand in for one — as does every TrueType face,
    // `.notdef` or otherwise (`openshard_uofiles::ttf_font::TtfFont::glyph`'s
    // "never fails" doc). Blinks off wall-clock time rather than a
    // stored `Instant`, so nothing on `Chat` has to track when focus
    // began.
    let caret_text = "|";
    let blink_on = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_millis() / 500) % 2 == 0)
        .unwrap_or(true);
    // `fonts.mul` has no Cyrillic past `0xFF` — see `run`'s
    // `--ttf-font` doc — and this is the box a player actually reads
    // what they typed back from, so unlike the dialog captions
    // `text_quads` carries below, this switches to `App::ttf_font`
    // and `Screen::ttf_gump_pass` whenever one is set rather than
    // drawing a line nobody can read the second half of.
    if let Some(font) = &resources.ttf_font {
        let atlas = window
            .ttf_atlas
            .as_mut()
            .expect("create_window builds ttf_atlas whenever ttf_font is set");
        let wanted = labels
            .iter()
            .flat_map(|label| label.text.chars())
            .chain(std::iter::once('|'));
        if let Err(error) = atlas.add(font, wanted) {
            // Same corner as the speech line's own `atlas.add` above.
            eprintln!("packing ttf glyphs: {error}");
        }
        // `labels`' own positions are gump pixels, `rows`/`input_at`'s
        // space — real pixels only once here, not per glyph inside
        // `collect_gump_ttf`: see that function's doc for why the
        // earlier per-glyph version read soft and its baseline
        // sawtoothed.
        let to_real = |p: GumpPixel| {
            GumpPixel::new(
                (p.x as f32 * scale).round() as i32,
                (p.y as f32 * scale).round() as i32,
            )
        };
        let mut real_labels: Vec<GumpLabel<'_>> = labels
            .iter()
            .map(|label| GumpLabel {
                at: to_real(label.at),
                ..*label
            })
            .collect();
        let prefix_width = text::gump_width_ttf("say: ", atlas);
        if chat.focused && blink_on {
            let real_input_at = to_real(input_at);
            let caret_x = prefix_width + text::gump_width_ttf(&chat.typed[..chat.cursor], atlas);
            real_labels.push(GumpLabel {
                at: GumpPixel::new(real_input_at.x + caret_x, real_input_at.y),
                text: caret_text,
                font: Font::DEFAULT,
                hue: Hue(chat_style.hue),
                clip: None,
            });
        }
        let mut hud_quads = text::collect_gump_ttf(&real_labels, atlas);
        // Overhead speech's own quads, folded into this same list
        // rather than a render call of their own — `GumpRenderer::render`'s
        // doc is explicit that a second call the same frame does not
        // add a second draw, it *replaces* the first: the instances
        // live in one buffer written through `queue.write_buffer`,
        // which lands before either call's encoded draw actually
        // runs, so a first, separate `screen_speech` call earlier in
        // the frame was silently overwritten by this one and never
        // drew anything. One call, everything it should draw.
        hud_quads.extend(text::collect_screen_ttf(screen_speech, atlas));
        // Picks up this call's own `add` above and, the first time
        // through this frame, the speech line's — see
        // `Screen::upload_ttf_dirty`'s doc.
        window.upload_ttf_dirty();
        let timed = profile::begin(window.gpu.as_ref(), "ttf gump text", encoder);
        window
            .ttf_gump_pass
            .as_mut()
            .expect("create_window builds ttf_gump_pass whenever ttf_atlas is")
            .render(
                &window.device,
                &window.queue,
                encoder,
                gump_art::Frame {
                    target: view,
                    width: window.config.width,
                    height: window.config.height,
                    // Not `scale`: `hud_quads` are already in real
                    // pixels, so the shader's own multiply — the one
                    // `text_quads` below still needs, being in gump
                    // pixels — would double it.
                    scale: 1.0,
                },
                &hud_quads,
            );
        profile::end(window.gpu.as_ref(), encoder, timed);
    } else {
        let prefix_width = text::gump_width("say: ", font, &resources.font_atlas);
        if chat.focused && blink_on {
            let caret_x =
                prefix_width + text::gump_width(&chat.typed[..chat.cursor], font, &resources.font_atlas);
            labels.push(GumpLabel {
                at: GumpPixel::new(input_at.x + caret_x, input_at.y),
                text: caret_text,
                font,
                hue: Hue(chat_style.hue),
                clip: None,
            });
        }
        text_quads.extend(scaled_gump_quads(
            &labels,
            &resources.font_atlas,
            chat_style.scale.raw(),
        ));
    }
    // The one call, with the windows' lines already in front of the
    // chat's: painter's order inside a single pass, and the only order
    // there is — see `text_quads` for what a second call would cost.
    // Draws only the windows' captions when `App::ttf_font` is set:
    // the chat's own quads went through `ttf_gump_pass` above instead.
    let timed = profile::begin(window.gpu.as_ref(), "gump text", encoder);
    window.gump_text_pass.render(
        &window.device,
        &window.queue,
        encoder,
        gump_art::Frame {
            target: view,
            width: window.config.width,
            height: window.config.height,
            scale,
        },
        text_quads,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
}

/// Records every world-space pass into `encoder`, from the ground up to the
/// hover and held rings — the one part of presenting a frame that is
/// **only** drawing, in the sense the module docs on
/// [`crate::graphics::GraphicsSettings`] and [`crate::picking::Picking`]
/// argue for: a free function taking `&mut GraphicsSettings` for the one
/// pair of fields it writes (`solids_held`, `solids_drawn`, this frame's own
/// count of what the solids view was handed and drew) and `&Picking` for the
/// one it only reads. See [`assemble_geometry`]'s doc for the same shape one
/// step earlier in the frame.
///
/// `encoder` and `window` are threaded through rather than returned: the
/// gump windows, the chat line and egui all record into the same encoder
/// after this call returns, and `App::draw_from` is what owns that sequence.
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
    quads: &[ground::GroundQuad],
    static_instances: &openshard_client_render::sprite::InstanceRows,
    static_boxes: &[openshard_client_render::impostor::Volume],
    mesh_vertices: &[openshard_client_render::mesh_face::MeshFaceVertex],
    mesh_rows: &[openshard_client_render::mesh_face::MeshFaceRow],
    mobile_quads: &[SpriteQuad],
    text_quads: &[SpriteQuad],
    outline_quads: &[SpriteQuad],
    held_item_outline: &[SpriteQuad],
    mobile_outline: &[SpriteQuad],
    held_mobile_outline: &[SpriteQuad],
    select_quads: &[SpriteQuad],
    lighting: &light::Lighting,
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
        .render(&window.device, &window.queue, encoder, target, quads);
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
        &static_instances.rows,
        static_boxes,
        Some(static_instances.drawn),
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
        mesh_vertices,
        mesh_rows,
    );
    profile::end(window.gpu.as_ref(), encoder, timed);
    let timed = profile::begin(window.gpu.as_ref(), "mobiles", encoder);
    window.mobile_pass.render(
        &window.device,
        &window.queue,
        encoder,
        target,
        mobile_quads,
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
    let item_rings: Vec<&[SpriteQuad]> = outline_quads.iter().map(std::slice::from_ref).collect();
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
    if !mobile_outline.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "outline mask: mobiles", encoder);
        window.mobile_pass.render_mask(
            &window.device,
            &window.queue,
            encoder,
            target,
            &mask_view,
            &[mobile_outline],
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
    if !select_quads.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "select mask", encoder);
        window.statics.render_mask(
            &window.device,
            &window.queue,
            encoder,
            target,
            &select_view,
            &[select_quads],
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
    let held_item_rings: Vec<&[SpriteQuad]> = held_item_outline.iter().map(std::slice::from_ref).collect();
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
    if !held_mobile_outline.is_empty() {
        let timed = profile::begin(window.gpu.as_ref(), "held mask: mobiles", encoder);
        window.mobile_pass.render_mask(
            &window.device,
            &window.queue,
            encoder,
            target,
            &held_view,
            &[held_mobile_outline],
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
            lighting,
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
        let standing = openshard_client_render::solid::standing(&lighting.occlusion, solid_cut);
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
        .filter(|_| !select_quads.is_empty())
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
    if !held_item_outline.is_empty() || !held_mobile_outline.is_empty() {
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
    if !outline_quads.is_empty() || !mobile_outline.is_empty() {
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

/// Everything `frame::assemble` and its neighbours collected for one frame —
/// see [`assemble_geometry`]'s own doc.
pub(crate) struct FrameGeometry {
    /// The flames, the grid they are occluded by, the ambient, and the two
    /// per-fragment knobs the lighting pass reads — `frame::assemble`'s own.
    pub(crate) lighting: light::Lighting,
    /// The land, back to front.
    pub(crate) quads: Vec<ground::GroundQuad>,
    /// The map's furniture and the server's items, split so a corner static's
    /// two faces carry their own id — see `sprite::split_corners`.
    pub(crate) static_instances: openshard_client_render::sprite::InstanceRows,
    /// Every box every drawn static stands as — `docs/lighting_rebuild.md`
    /// phase 6.
    pub(crate) static_boxes: Vec<openshard_client_render::impostor::Volume>,
    /// Raw vertices for every visible climbable static's mesh.
    pub(crate) mesh_vertices: Vec<openshard_client_render::mesh_face::MeshFaceVertex>,
    /// One row per face represented in `mesh_vertices`.
    pub(crate) mesh_rows: Vec<openshard_client_render::mesh_face::MeshFaceRow>,
    /// What a click is holding, placed exactly as the picture placed it.
    pub(crate) select_quads: Vec<SpriteQuad>,
    /// The silhouette the hover ring is grown from.
    pub(crate) outline_quads: Vec<SpriteQuad>,
    /// The same, for what a click is holding.
    pub(crate) held_item_outline: Vec<SpriteQuad>,
    /// The creature silhouette the hover ring is grown from.
    pub(crate) mobile_outline: Vec<SpriteQuad>,
    /// The same, for what a click is holding.
    pub(crate) held_mobile_outline: Vec<SpriteQuad>,
    /// The crowd's own pictures.
    pub(crate) mobile_quads: Vec<SpriteQuad>,
    /// What the frame was asked for, in the same words `frame::Inputs::summary`
    /// gives — kept beside the pictures for the F12 dump. `None` unless a
    /// dump is armed.
    pub(crate) asked_for: Option<String>,
}

/// Everything the world's pictures are built from, out of `frame::assemble`
/// and the outline/mobile collectors beside it — the part of presenting a
/// frame that is genuinely **only** drawing: every parameter here is `&`
/// except `graphics`, which is handed over for the one field
/// `frame::Inputs::bake` writes through (`occlusion_bake`) rather than for
/// `self` as a whole. See [`ready_atlases`]'s doc for the same shape applied
/// to the atlases, and `App::draw_from`'s Step three doc for where this call
/// sits between them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_geometry(
    resources: &resources::Resources,
    graphics: &mut graphics::GraphicsSettings,
    world: &world::WorldState,
    picking: &picking::Picking,
    window: &Screen,
    camera: Camera,
    cutaway: &Cutaway,
    tuning: &light::Tuning,
    lit_item: Option<usize>,
    lit_mobile: Option<usize>,
    held_item: Option<usize>,
    held_mobile: Option<usize>,
    drawn: &[Mobile],
) -> FrameGeometry {
    // Three skies and not two: night, a daylight with a sun in it, and the
    // plain daylight that is the identity — the frame the blit has always
    // copied through untouched. The middle one is a key today; see
    // `App::sunlit`.
    let sky = match (graphics.night, graphics.sunlit) {
        (true, _) => Some(light::NIGHT),
        (false, true) => Some(light::SKYLIGHT),
        // Daylight, where the pass is a copy and no grid is built at all —
        // unless the solids view is on, and then the grid *is* the subject.
        // `Ambient::DAY` flattened is the identity, so the picture under the
        // boxes is the same daylight frame it was; what it buys is that the
        // list drawn is the one the shader would walk, out of the same bake,
        // rather than a second walk of the map made for the view. See
        // `docs/lighting.md` step 23.0.
        (false, false) => graphics.show_solids.then_some(light::Ambient::DAY),
    };
    // And whether a tile's share of it depends on what stands over the tile.
    // Off by default: see `App::sky_field`, and `light::Ambient::flattened`
    // for why the flat one is the baseline rather than a lesser version.
    let sky = match graphics.sky_field {
        true => sky,
        false => sky.map(light::Ambient::flattened),
    };
    // One pick (`lit_item`, at the top of the frame), two effects, and the
    // style decides which of them is asked for. `None` is how each is
    // switched off, so neither pass has a mode to branch on: the hue pass
    // draws an item that is not highlighted, and the silhouette pass is
    // handed an empty list.
    let hued = graphics.highlight_style.hues().then_some(lit_item).flatten();
    let ringed = graphics.highlight_style.rings().then_some(lit_item).flatten();

    // Where the player's own picture will land, asked before the statics
    // are collected rather than after: `frame::assemble` places a tree's
    // canopy against this rectangle, so a leaf that would be drawn over
    // the body is cut instead of hung over it — see
    // `cutaway::hides_foliage_over`. `None` only for the one frame the
    // atlas has not yet grown a frame for this body and group, same as
    // `mobiles::head_anchor`'s own gap.
    let player_rect = mobiles::screen_rect(&world.player, &camera, &window.atlases.mobiles);

    // **One assembly, and the client is a caller of it like any other** —
    // `docs/parity.md`, decision D1. This sequence used to be written out by
    // hand here and in six other places, every one of them free to pass a
    // different cutaway, a different grid or a different clock; each of them
    // did, and the difference was only ever found by reading. Everything a
    // caller may honestly differ on is a field of `frame::Inputs` now, so
    // what this frame is can be compared against what a tool's frame is
    // rather than pieced together from two call sites.
    let inputs = frame::Inputs {
        map: &resources.map,
        items: &world.items,
        camera: &camera,
        tiledata: &resources.tiledata,
        animations: &world.tile_animations,
        cutaway,
        land: &window.atlases.land,
        texmaps: &window.atlases.texmaps,
        // The pictures, which is where an occluder's *facing* comes from: a
        // wall stops a ray only where the ray crosses the side the wall
        // stands on, and only the art says which side that is. One atlas for
        // the grid and for both sprite passes, so they cannot be about two
        // different sets of sprites.
        statics: &window.atlases.statics,
        sky,
        // The sun is a property of the sky and not of the tiles, so it is an
        // input to the frame rather than something walked with them — and
        // never at night, where a second source lighting every roof would
        // undo the whole point of the dark. Where the Light tab put it,
        // which is `light::midday` until somebody moves a slider — see
        // `light::SunTuning`.
        sun: (graphics.sunlit && !graphics.night).then(|| tuning.sun.sun()),
        // And the flame in the player's own hand, which no walk of the map
        // could have found — see `light::carried`. The offset is where the
        // sprite is *actually* drawn this instant, past `at`'s tile, so the
        // pool glides with the walk instead of jumping once a step.
        carried: graphics.lantern.then_some((
            world.player.at,
            mobiles::walked_offset(&world.player),
            world.player.facing,
        )),
        tuning,
        flame_time: world.flame_clock.as_secs_f32(),
        // The blocks of the occlusion grid built for earlier frames. A
        // camera that has moved a tile wants the same five hundred and fifty
        // blocks it wanted last frame bar a handful — see `occlusion::bake`,
        // and `StaticAtlas::revision` for what makes this let go when the
        // atlas learns something new about a graphic.
        bake: Some(&mut graphics.occlusion_bake),
        highlight: hued,
        // The live client meets every sprite against its own boxes whenever
        // it has a grid at all. F10 is not this field: turning the lights off
        // takes the *sky* away, and a frame with no sky has no grid for
        // anything to be met against.
        impostor: Impostor::Met,
        // Which producers this frame draws — the World tab's own boxes. The
        // whole world unless somebody has ticked one off, and the lighting is
        // collected from all of it whatever they tick: see `frame::Draw`.
        draw: graphics.drawing,
        // The view is the looker's, not the world's: a diagnostic draws from
        // the values this frame was lit with, and in daylight those are the
        // ambient and the place attachment — which is exactly what a person
        // checking the place channel wants to see, without having to make it
        // night first.
        view: graphics.light_view,
        // `docs/combat.md`'s D9: the screen greys for the character this
        // client is, not for the offline placeholder — which has no view
        // and so is never a ghost.
        dead: world.view.as_ref().is_some_and(|view| view.player.dead),
        player_rect,
    };
    // **What the frame was asked for**, kept beside the pictures the dump
    // below writes. A picture on its own cannot be reproduced: two frames
    // that differ say nothing about *which* input differed, and the client's
    // arguments were readable until now only by reading this function. Only
    // when a dump is armed — `summary` walks every field and allocates.
    let asked_for = graphics.frame_dump.as_ref().map(|_| inputs.summary());
    let frame::Frame {
        lighting,
        ground: quads,
        statics:
            statics::StaticGeometry {
                quads: static_quads,
                mesh_vertices,
                mesh_rows,
                boxes: static_boxes,
            },
    } = frame::assemble(inputs);

    // What a click is holding, placed exactly as the picture placed it —
    // `statics::selected` is `statics::collect`'s own arithmetic — so the
    // mask lands on the wall's pixels rather than beside them. Empty on
    // every frame with nothing selected, which is what switches the pass off.
    let select_quads = statics::selected(
        &camera,
        &resources.tiledata,
        &world.tile_animations,
        &window.atlases.statics,
        cutaway,
        picking.selected.and_then(SelectedIdentity::as_static),
    );
    // The same quads as the picture's, so the ring lands on the sprite
    // rather than beside it — see `items::outlined`.
    let outline_quads = items::outlined(
        &world.items,
        &camera,
        &resources.tiledata,
        &world.tile_animations,
        &window.atlases.statics,
        cutaway,
        ringed,
    );
    // The held item's own silhouette, through the same function and for
    // the same reason — a second call rather than folding `held_item` into
    // `ringed` above, because the two are drawn with different [`Ring`]s
    // into different masks: this is what a click named, not what the
    // cursor is over.
    let held_item_outline = items::outlined(
        &world.items,
        &camera,
        &resources.tiledata,
        &world.tile_animations,
        &window.atlases.statics,
        cutaway,
        held_item,
    );
    // A corner static's two faces get their own id past this point — see
    // `docs/gbuffer.md` step 4 and `sprite::split_corners`'s own doc.
    let static_instances = split_corners(static_quads);
    // The same two effects for a creature, off the same style switch and
    // the same one-pick-a-frame rule: `lit_mobile` and `lit_item` are never
    // both `Some` (see where they are asked), so exactly one of the four
    // lists below is ever non-empty.
    let mobile_hued = graphics.highlight_style.hues().then_some(lit_mobile).flatten();
    let mobile_ringed = graphics.highlight_style.rings().then_some(lit_mobile).flatten();
    let mobile_outline = mobiles::outlined(
        drawn,
        &camera,
        &window.atlases.mobiles,
        cutaway,
        &resources.equip_conv,
        mobile_ringed,
    );
    // The held mobile's own silhouette — see `held_item_outline` above for
    // why this is a second call and not `mobile_ringed` itself.
    let held_mobile_outline = mobiles::outlined(
        drawn,
        &camera,
        &window.atlases.mobiles,
        cutaway,
        &resources.equip_conv,
        held_mobile,
    );
    let mobile_quads = mobiles::collect(
        drawn,
        &camera,
        &window.atlases.mobiles,
        cutaway,
        &resources.equip_conv,
        mobile_hued,
    );
    FrameGeometry {
        lighting,
        quads,
        static_instances,
        static_boxes,
        mesh_vertices,
        mesh_rows,
        select_quads,
        outline_quads,
        held_item_outline,
        mobile_outline,
        held_mobile_outline,
        mobile_quads,
        asked_for,
    }
}

/// One frame's own facts — see [`App::frame_facts`]'s doc for what makes this
/// worth a struct: every field is a pure question of `&self`, asked once
/// against one camera, and nothing here is written back except through the
/// three lines at `App::draw_from`'s call site that read `pick.static_`,
/// `on_mobile` and `on_item` back out again.
pub(crate) struct FrameFacts {
    /// Whether anybody is looking at the window at all — see [`App::watched`].
    pub(crate) watched: bool,
    /// The roof cutaway this frame's picks and picture are both drawn under.
    pub(crate) cutaway: Cutaway,
    /// What the cursor is over and what it lit — see [`Pick`].
    pub(crate) pick: Pick,
    /// The crowd as the mobile pass's own atlas already has it packed — the
    /// list `on_mobile` indexes into, and `None` before there is a window at
    /// all.
    pub(crate) drawn_mobiles: Option<Vec<(Who, Mobile)>>,
    /// The creature the cursor is over, indexing `drawn_mobiles` — the
    /// unfiltered form of [`Pick::mobile`], kept here because
    /// `App::draw_from` reads it back into `self.picking` regardless of the
    /// highlight mode: what a click selects is not a question about lighting.
    pub(crate) on_mobile: Option<usize>,
    /// The item the cursor is over, indexing `self.world.items` — the
    /// unfiltered form of [`Pick::item`], for the same reason.
    pub(crate) on_item: Option<usize>,
    /// What a click is holding, turned back into an index into
    /// `drawn_mobiles`.
    pub(crate) held_mobile: Option<usize>,
    /// What a click is holding, turned back into an index into
    /// `self.world.item_serials`.
    pub(crate) held_item: Option<usize>,
}

impl App {
    /// Everyone to draw, each beside the serial their clock is keyed by.
    ///
    /// Our own body first, and `None` for it while no shard has named us.
    ///
    /// The group is refreshed from the crowd here and not in
    /// [`App::advance_to_clocks`] alone, because this list is what *packs* the
    /// atlas as well as what draws from it — see [`App::wanted_in`]. `self.world.player`
    /// and `self.world.others` hold the group as of the last packet, and
    /// [`Crowd::advance`] changes it without one: a body that walked into view
    /// and then stopped is drawn standing while the packet-time list still says
    /// walking. Pack one group and draw another and [`mobiles::place`] finds no
    /// frame, so the body simply vanishes — and stays vanished for as long as it
    /// stands still, there being no further packet to correct the list with.
    pub(crate) fn drawn_mobiles(&self) -> Vec<(Who, Mobile)> {
        Self::everyone_drawn(
            &self.world.crowd,
            self.world.me(),
            &self.world.player,
            &self.world.others,
        )
    }

    /// [`App::drawn_mobiles`] over the four fields it reads, so a test can build
    /// the list the atlases are grown from without a window, a device or a
    /// shard.
    pub(crate) fn everyone_drawn(
        crowd: &Crowd,
        me: Who,
        player: &Mobile,
        others: &[(Who, Mobile)],
    ) -> Vec<(Who, Mobile)> {
        let mut mobiles = Vec::with_capacity(others.len() + 1);
        mobiles.push((me, player.clone()));
        mobiles.extend_from_slice(others);
        Self::advance_groups(crowd, &mut mobiles);
        mobiles
    }

    /// Refresh each body's animation group from the crowd's clock.
    ///
    /// Split out of [`App::advance_to_clocks`] because the group is the one part
    /// of a mobile that has to be right *before* the atlases are grown, and the
    /// growth happens with no atlas to ask for a frame count. Both paths go
    /// through here so there is one statement of "which group is playing".
    pub(crate) fn advance_groups(crowd: &Crowd, drawn: &mut [(Who, Mobile)]) {
        for (who, mobile) in drawn.iter_mut() {
            // `Crowd::advance` drops a walking body to standing on its own
            // timer, with nothing that looks like a packet to refresh
            // `mobile.group` from — a group read once and left stale plays the
            // walking sprite for ever, timed by a clock that has moved on to
            // the standing group's.
            if let Some(group) = crowd.group_for(*who) {
                mobile.group = group;
            }
        }
    }

    /// Fill in the three time-varying halves of every mobile from the crowd's
    /// clocks: which group is playing, which frame of it, where the body is
    /// drawn, and which tile the step is sorted at.
    ///
    /// An associated function taking the two fields it reads rather than a
    /// method, because both callers hold a borrow of one of `App`'s fields
    /// while they ask: the frame holds `self.window` mutably, and the pick
    /// holds it shared. A `&self` method would borrow all of `App` and neither
    /// could call it.
    ///
    /// `atlas` is asked for the frame *count*: a group's length is the
    /// animation's, and taking it from anywhere else makes "frame 7 of a
    /// 6-frame walk" expressible. Under the body the atlas packed — for a ghost
    /// the living body it borrows its pictures from — or a ghost counts zero
    /// frames, lands on frame 0 for ever and slides along standing still.
    pub(crate) fn advance_to_clocks(crowd: &Crowd, atlas: &AnimAtlas, drawn: &mut [(Who, Mobile)]) {
        // The group is read back first and not only the frame and the glide —
        // the frame count below is asked *under* it. Idempotent when the caller
        // is [`App::drawn_mobiles`], which is every caller today; here so this
        // function is right on its own terms rather than on its callers'.
        Self::advance_groups(crowd, drawn);
        for (who, mobile) in drawn.iter_mut() {
            let (direction, _) = openshard_uofiles::anim::facing(mobile.facing);
            let frame_count = atlas.frame_count(
                openshard_uofiles::anim::animation_body(mobile.body),
                mobile.group,
                direction,
            );
            mobile.frame = crowd.frame_for(*who, frame_count);
            if let Some(at) = crowd.drawn_for(*who) {
                mobile.drawn = at;
            }
            // And which tile it sorts at, which is a step's own clock too: the
            // crossing ends without a packet to say so, and a body still sorted
            // on the tile it left would keep drawing over the ground behind it.
            mobile.from = crowd.stepping_from(*who);
        }
    }

    /// Everyone as they are drawn *this instant*, clocks and all — the list
    /// [`mobiles::pick`] and [`mobiles::collect`] both index into.
    ///
    /// Built twice a frame, once for the pick and once for the picture, rather
    /// than threaded between them: the two happen either side of the atlas
    /// growth and of a mutable borrow of the window, and the work is a handful
    /// of map lookups over whoever is on screen. What matters is that the
    /// *order* is [`App::drawn_mobiles`]'s both times, so an index answered by
    /// the pick still names the same creature to the passes below.
    pub(crate) fn drawn_now(&self, atlas: &AnimAtlas) -> Vec<(Who, Mobile)> {
        let mut drawn = self.drawn_mobiles();
        Self::advance_to_clocks(&self.world.crowd, atlas, &mut drawn);
        drawn
    }

    pub(crate) fn draw(&mut self) {
        let started = Instant::now();
        // The frame boundary the flamegraph is cut on, put at the same place
        // `started` is sampled so that a frame in `puffin_viewer` and a frame in
        // the `frames` panel are the same span of time. Free when nobody is
        // recording — see [`profile`].
        profile::frame();
        puffin::profile_scope!("draw");
        // What the shard has opened, and what it has taken away: the view is
        // filled by `client/net`, which knows nothing about screens, so a
        // window appearing is this end noticing.
        self.sync_own_windows();
        // # The frame is three steps, and this is the first of them
        //
        // Everything that writes runs in `Self::advance`, before anything
        // reads — see that method's own doc for why the clock and the eye
        // move there and not here. After it returns, nothing in the frame
        // moves the world or the camera again; the snapshot below is what
        // every reader from here on is handed.
        self.advance(started);
        let camera = *self.control.camera();
        self.draw_from(started, camera);
    }

    /// **Step one of three**: everything that writes. What the shell asked
    /// for last frame, then every clock, then the eye.
    ///
    /// The animation clock moves here, at the top of the frame that is about
    /// to show its answer — not when the timer that asked for this frame
    /// fired.
    ///
    /// A glide is a position read off a clock, so the moment that clock is
    /// read has to be the moment the picture is built or the walk judders:
    /// the timer fires, the loop then lays out the UI, grows an atlas and
    /// waits on the swapchain, and however long that took is error in the
    /// body's position — error that varies frame to frame, which is exactly
    /// what an eye reads as a stutter. It also puts the sampling back in step
    /// with the display: `WaitUntil` is a floor, the timer's 16ms beats
    /// against a 60Hz refresh, and a frame drawn from the previous tick's
    /// clock lands on the wrong side of that beat every second or so.
    ///
    /// Whatever really passed — see `App::last_advance`. A stall longer than
    /// a frame, the window minimised or the machine asleep, moves the clock
    /// the whole way rather than queuing a burst of catch-up frames for time
    /// nobody watched: a body that was walking through it has long since
    /// arrived.
    ///
    /// The defect this staging is written against: the HUD used to be built
    /// at the top of the frame and the eye moved a few lines further down, so
    /// the overlay egui laid out — the tile highlight, the hover, the walk
    /// goal — was drawn against the *previous* frame's camera while the world
    /// pass below drew from this one's. The gap between them is one frame of
    /// camera motion, which is not a constant: it is whatever the display
    /// gave this frame, so the markers shivered against the ground they were
    /// meant to be lying on, and every missed interval made them jump.
    /// Reordering two calls would have fixed today's version of it and left
    /// the shape that produced it, which is a second reader picking the
    /// camera up at a different moment. So the frame is staged instead.
    pub(crate) fn advance(&mut self, started: Instant) {
        let elapsed = started.saturating_duration_since(self.last_advance);
        let asked = std::mem::take(&mut self.pending);
        self.apply(asked);
        // The viewport the last frame's layout left free — `Shell` holds it
        // between frames for exactly this. It has to be settled before the eye
        // is, because it is what decides how much world a camera can see.
        if let Some(shell) = self.shell.as_ref() {
            let viewport = shell.viewport();
            self.control.resize(viewport.width, viewport.height);
        }
        self.world.crowd.advance(elapsed);
        // The statics that move on their own, on the same span as everybody
        // else. Its own clock inside — a fire's cycle has nothing to do with a
        // walk's — and one *sample*, which is the whole rule: two clocks read
        // from two `Instant::now()`s a few hundred microseconds apart would put
        // a torch and the body that walks past it on two different instants.
        self.world.tile_animations.advance(elapsed);
        // And the flames, off the same span: a fire's animation frame and the
        // brightness of the pool it casts are two clocks describing one fire,
        // and they are advanced together or they describe two.
        self.world.flame_clock += elapsed;
        self.last_advance = started;
        // Whatever scenario is being walked delivers its knots for the span that
        // just passed, before the eye is asked where the body is: a step that
        // arrived this frame is one the camera has to answer this frame.
        self.advance_replay(elapsed);
        // A viewport that grew may have taken the world texture past what the
        // device allows, which no zoom step asked for.
        self.fit_zoom_to_device();
        // And the eye goes where the body is *this frame*: a step arrives once
        // and is then walked across for the next 400ms, so every frame in
        // between has a different answer.
        self.follow_player(elapsed);
    }

    /// **Step two**: one snapshot, and it is a value.
    ///
    /// Every question this frame's picture and HUD are built from, asked
    /// once against one camera and one cutaway and answered as a plain
    /// value — purely a function of `&self`, so a caller cannot mistake this
    /// for a place the frame's state changes. It has none: `on_static`,
    /// `on_mobile` and `on_item` still have to land in `self.picking` for the
    /// click to read next frame, but that write happens in the three lines at
    /// `draw_from`'s call site instead, which is the "mutations applied
    /// separately" half of the shape `Self::advance` set up for the first
    /// step.
    pub(crate) fn frame_facts(&self, camera: Camera) -> FrameFacts {
        // Read before the window is borrowed below, for the same reason the
        // pacing at the foot of the frame is a fact about the whole app
        // rather than about it.
        let watched = self.watched();
        // The same, for the two the item highlight needs — both are questions
        // about the whole of `self` and are asked once, here.
        let owns_pointer = self.world_owns_pointer();
        let cursor = self.control.cursor();

        // What this frame does not draw, read once from the tile the player is
        // standing on. Once, and from the *player's* tile rather than the
        // camera's: a free camera looking at a rooftop three streets away has
        // not walked indoors, and the client's rule is about where the body is.
        // See `openshard_client_render::cutaway`.
        //
        // `self.world.cutaway_at`, not `self.world.player.at`: the latter is this end's
        // own unconfirmed prediction, which for one frame can be a tile a
        // held direction was refused on — see the field's own doc.
        //
        // Here, in the snapshot, and not beside the passes that draw from it:
        // the item pick below needs it, and the pick has to be answered before
        // the HUD is built — see the next paragraph.
        let cutaway = Cutaway::at(
            &self.resources.map,
            &self.resources.tiledata,
            self.world.cutaway_at,
            true,
        );
        // The ground tile under the cursor, and its ring — asked here beside
        // the picks below rather than a second time when the HUD is built:
        // this used to be `App::hud`'s own call to `Self::pick_tile`, a second
        // "what is the cursor over" answered from a *different* camera in
        // spirit even when it happened to be the same value in practice. One
        // frame's worth of picks belongs in one place — this function — same
        // as `on_mobile`/`on_item`/`on_static` below.
        let hover = owns_pointer.then(|| self.pick_tile(camera)).flatten();
        let neighbours = hover.as_ref().map_or_else(Vec::new, |tile| self.tile_ring(tile));
        // What the cursor is over, asked here rather than remembered from the
        // last click: the picture moves under a still mouse — the body walks,
        // the camera follows, a door swings — so where the cursor is pointing is
        // a question about *this* frame's picture and has to be asked against
        // this frame's camera. The same `items::pick` a double-click asks, so
        // what is lit is what would be used.
        //
        // Asked once and answered to three readers: the hue the picture is drawn
        // in, the silhouette the ring is grown from, and whether the HUD marks
        // the tile under the cursor at all. Two picks would be two chances to
        // disagree about what the cursor is on, and the visible form of that
        // disagreement is a barrel ringed with the ground under it diamonded.
        //
        // Against the atlas as it stands *before* this frame grows it, which is
        // the one thing given up by asking this early. An item that came on
        // screen this very frame has no sprite packed yet and so no rectangle to
        // be pointed at, and is pickable a frame later; the alternative was a
        // tile marker that decides whether to draw itself from the previous
        // frame's answer, which flickers along every item's edge.
        // **The picks are the frame's *facts*, and the mode decides only what is
        // drawn from them.** They used to be skipped under
        // `HighlightTarget::Tiles`, which folded two questions into one field:
        // "what is the cursor on" and "what may light up". A click reads the
        // first — see the `MouseInput` arm — so with the two folded together a
        // player who had pinned the highlight to tiles could not select a wall at
        // all, and the reason was invisible. The mode is applied to `lit_*`
        // below instead, where it is about lighting and nothing else.
        //
        // Creatures are asked first and they win: a mobile stands *on* the
        // clutter of its tile — it is sorted above whatever is lying there, and
        // it is what a player pointing at a shopkeeper standing on a rug means.
        // Then the server's items, then the map's own furniture. One chain, and
        // every later question is asked only where the earlier ones found
        // nothing — so "what is under the cursor" has exactly one answer and the
        // ring, the wash, the tile marker and the click cannot disagree about it.
        // Kept whole, and not just picked from: the click reads a mobile back by
        // [`Who`] rather than by this index, which is only ever good for this
        // one frame's own `Vec` — see `FrameFacts::on_mobile`.
        let drawn_mobiles = self
            .window
            .as_ref()
            .map(|window| self.drawn_now(&window.atlases.mobiles));
        let on_mobile = match (owns_pointer, self.window.as_ref(), &drawn_mobiles) {
            (true, Some(window), Some(drawn)) => mobiles::pick(
                &drawn.iter().map(|(_, mobile)| mobile.clone()).collect::<Vec<_>>(),
                &camera,
                &window.atlases.mobiles,
                &cutaway,
                &self.resources.equip_conv,
                cursor,
            ),
            _ => None,
        };
        let on_item = match owns_pointer && on_mobile.is_none() {
            true => self.window.as_ref().and_then(|window| {
                items::pick(
                    &self.world.items,
                    &camera,
                    &self.resources.tiledata,
                    &self.world.tile_animations,
                    &window.atlases.statics,
                    &cutaway,
                    cursor,
                )
            }),
            false => None,
        };
        // And the map's own furniture last, which is the one a wall is: it has no
        // serial and cannot be used, so it loses to anything that can. Asked
        // every frame rather than at the click, because it is what the *tile
        // marker* has to know — a wall under the cursor takes the highlight, and
        // the diamond drawn on the ground behind it was the client answering the
        // same question twice with two different tiles.
        //
        // This is the one pick that walks the map: `statics::pick` covers the
        // cells `statics::collect` is about to draw. It is a second walk of them
        // per frame with the pointer over the world, and the placement it does
        // per static is the collector's own — see the Frames tab if it ever
        // shows.
        let on_static = match owns_pointer && on_mobile.is_none() && on_item.is_none() {
            true => self.window.as_ref().and_then(|window| {
                statics::pick(
                    &self.resources.map,
                    &camera,
                    &self.resources.tiledata,
                    &self.world.tile_animations,
                    &window.atlases.statics,
                    &cutaway,
                    cursor,
                )
            }),
            false => None,
        };
        // What the mode allows to light up. `Tiles` lights neither, which is the
        // whole of that setting; the facts above are unchanged by it.
        let lit_mobile = on_mobile.filter(|_| self.graphics.highlight != shell::HighlightTarget::Tiles);
        let lit_item = on_item.filter(|_| self.graphics.highlight != shell::HighlightTarget::Tiles);

        // What a click is *holding*, turned from identity back into this
        // frame's index — the reverse of `on_mobile`/`on_item` just above.
        // This is the held ring's own pick, asked once here rather than at
        // every reader, for the reason `lit_item`'s doc gives for asking
        // `on_item` once: two lookups are two chances to disagree about which
        // creature a `Who` still names.
        //
        // Valid only while the crowd is actually drawn: `drawn` below is
        // emptied whole when `self.graphics.drawing.mobiles` is off, and an index into
        // `drawn_mobiles` would then point at a `Vec` the held ring never
        // sees.
        let held_mobile = self
            .picking
            .selected
            .and_then(SelectedIdentity::as_mobile)
            .filter(|_| self.graphics.drawing.mobiles)
            .and_then(|who| {
                drawn_mobiles
                    .as_ref()
                    .and_then(|drawn| drawn.iter().position(|(candidate, _)| *candidate == who))
            });
        let held_item = self
            .picking
            .selected
            .and_then(SelectedIdentity::as_item)
            .and_then(|serial| {
                self.world
                    .item_serials
                    .iter()
                    .position(|candidate| *candidate == serial)
            });
        FrameFacts {
            watched,
            cutaway,
            pick: Pick {
                tile: hover,
                neighbours,
                static_: on_static,
                mobile: lit_mobile,
                item: lit_item,
            },
            drawn_mobiles,
            on_mobile,
            on_item,
            held_mobile,
            held_item,
        }
    }

    /// **Steps two and three**: the frame `Self::advance` staged for. Takes
    /// the camera as a parameter rather than reading `self.control` again —
    /// a `&Camera` handed to five collectors is five reads of a field that
    /// something between them might have moved, which is the defect
    /// `Self::advance`'s doc is written against, expressed as a borrow. A
    /// `Camera` is `Copy`, so the one read in `draw` costs nothing and cannot
    /// be stale in one place and fresh in another.
    pub(crate) fn draw_from(&mut self, started: Instant, camera: Camera) {
        // # Step two: one snapshot, and it is a value
        //
        // `Self::frame_facts` asks every question this frame's picture and HUD
        // are built from, purely against `&self` — and answers three of them,
        // `on_static`/`on_mobile`/`on_item`, into `self.picking` right here,
        // which is the "mutations applied separately" half of the shape
        // `Self::advance` set up for the first: a function that only *asks*
        // stays a function that only asks, and the one write this frame still
        // owes `self.picking` is named where it happens rather than folded into
        // the asking.
        let facts = self.frame_facts(camera);
        self.picking.on_static = facts.pick.static_;
        self.picking.on_mobile = facts.on_mobile.and_then(|index| {
            facts
                .drawn_mobiles
                .as_ref()
                .and_then(|drawn| drawn.get(index))
                .map(|(who, _)| *who)
        });
        self.picking.on_item = facts.on_item.map(|index| self.world.item_serials[index]);
        let FrameFacts {
            watched,
            cutaway,
            pick,
            drawn_mobiles: _,
            on_mobile: _,
            on_item: _,
            held_mobile,
            held_item,
        } = facts;

        // # Step three: present. Nothing below this line writes the world.
        //
        // The UI first, because it is what the surface is composited from
        // bottom-up and because its layout is what next frame's viewport comes
        // from. Its request is *held* rather than applied — see [`App::pending`].
        //
        // Timed, and separately from the world below: the two halves of a frame
        // are built by two things that grow for different reasons, and a single
        // build time cannot say which of them ate the frame. See [`frames`].
        //
        // The `Instant`s from here down are instrumentation and not a clock the
        // picture depends on: they measure what this frame cost, and no position
        // in it is a function of them. The one sampling of time that the frame is
        // built from is `started`, at the top.
        let ui_started = Instant::now();
        let hud = self.hud(camera, &pick, &cutaway);
        let painting = self.window.as_ref().map(|screen| Arc::clone(&screen.window));
        let ui = match (self.shell.as_mut(), painting.as_ref()) {
            (Some(shell), Some(window)) => {
                let (request, output) = shell.run(window, &hud, camera, &self.world);
                let viewport = shell.viewport();
                Some((request, output, viewport))
            }
            _ => None,
        };
        let mut ui_cost = ui_started.elapsed();
        if let Some((request, _, _)) = &ui {
            self.pending = request.clone();
        }

        // The Light tab's own numbers, which live in the shell — read here,
        // once for the whole frame: the flames, the ambient and the sun below
        // are all turned by them, and so is `want` just below, since the
        // atlases have to be grown for the same bound `light::collect` reads
        // them over.
        let tuning = self.tuning();
        // The Chat tab's own numbers, the same reason and the same place:
        // gathered before the window is borrowed below, since `App::chat_style`
        // also reads the whole of `self`.
        let chat_style = self.chat_style();
        // What the camera has walked onto since the atlases were last grown.
        // Gathered before the window is borrowed, and not inside the borrow: it
        // reads the whole of `self`, and the window is part of it.
        let want = light::lit_tiles(&camera, &tuning);
        let wanted = self.wanted_since(camera, &tuning, self.graphics.covered);
        let mut drawn = self.drawn_mobiles();
        // Likewise: the cut the solids view is drawn under reads the player, and
        // the pass that uses it runs inside the window's borrow.
        let solid_cut = self.solid_cut();

        let Some(window) = self.window.as_mut() else {
            return;
        };
        let repacked = ready_atlases(
            &mut self.resources,
            &mut self.graphics,
            &self.world,
            &mut self.repacks,
            window,
            want,
            &wanted,
            &drawn,
        );

        // Three time-varying halves of a mobile, filled in per frame rather
        // than per packet: the crowd is the only thing that knows what a
        // clock — and a group — has done since the `0x77` landed, and
        // `self.world.player`/`self.world.others` were built when it did. Against the atlas
        // as it stands *after* this frame's growth, which is the one the
        // picture below is drawn from.
        Self::advance_to_clocks(&self.world.crowd, &window.atlases.mobiles, &mut drawn);
        // Whoever the crowd is still holding a line for, hung above whichever
        // of `drawn`'s mobiles their serial belongs to. Read out here, before
        // `who` is dropped below: a label with no mobile to anchor to has
        // nothing to draw either way, so the two share the same "still on
        // screen" question `mobiles::head_anchor` answers.
        let speech: Vec<(ViewPixel, String, Font, Hue)> = drawn
            .iter()
            .filter_map(|(who, mobile)| {
                let (text, font, hue) = self.world.crowd.speaking(*who)?;
                let anchor = mobiles::head_anchor(mobile, &camera, &window.atlases.mobiles)?;
                Some((anchor, text.to_string(), font, hue))
            })
            .collect();
        // **The crowd, or none of it** — `frame::Draw::mobiles`, which this
        // function honours because `frame::assemble` does not collect mobiles at
        // all. Emptied here and not at each of the three uses below, so that the
        // picture, the ring and the outline cannot disagree about who is in the
        // frame: a body left out of the world image and still ringed would be a
        // halo round nothing.
        //
        // The speech above is deliberately *not* filtered by it. A label is not a
        // thing standing in the street — `Kind::Nothing`, see `crate::place::Kind`
        // — and turning the crowd off to look at a wall is not a request to go
        // deaf.
        let drawn: Vec<Mobile> = match self.graphics.drawing.mobiles {
            true => drawn.into_iter().map(|(_, mobile)| mobile).collect(),
            false => Vec::new(),
        };

        // The vsync wait, and the reason it is timed on its own: under
        // `PresentMode::Fifo` this call blocks until the display has taken the
        // frame before it, which on an idle client is most of the interval.
        // Counted as build time it would report a client that is asleep as one
        // at full load, and the panel exists to tell those two apart.
        let acquire_started = Instant::now();
        let frame = match window.surface.get_current_texture() {
            // Suboptimal still draws: the surface wants reconfiguring, and the
            // next resize event will do it.
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                frame
            }
            // The swapchain no longer matches the window. Rebuild it and let the
            // next redraw use it; drawing into a stale one is a crash on some
            // backends and a stretched frame on others.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                window.surface.configure(&window.device, &window.config);
                return;
            }
            // Nothing was acquired and nothing is wrong: the window is hidden,
            // or the compositor took too long. Skipping the frame is the answer.
            other => {
                if !matches!(
                    other,
                    wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded
                ) {
                    eprintln!("acquiring a frame: {other:?}");
                }
                return;
            }
        };
        let wait = acquire_started.elapsed();
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Where the world goes on the surface: the rect the panels left free, so
        // a docked panel shrinks the world rather than covering it.
        let viewport = ui.as_ref().map_or(
            ViewportRect {
                x: 0,
                y: 0,
                width: window.config.width,
                height: window.config.height,
            },
            |(_, _, viewport)| *viewport,
        );

        // The image the world is drawn into. Its size is the camera's, so a
        // resize and a zoom step are the same event here — and recreating it is
        // the only thing either of them costs.
        //
        // Magnified it is the *viewport's* size and the magnification rides in
        // the vertex transform, so the world is drawn at the display's own
        // resolution and the blit below is a copy; minified it is the world's
        // own larger extent and the blit shrinks it. `docs/camera.md` D11 is the
        // argument, and the short of it is that an image of virtual resolution
        // cannot express an offset of one real pixel — which is the whole of
        // what made a magnified scroll coarser than the screen it was on.
        let (render_width, render_height) = camera.image_size();
        if window.world.width() != render_width || window.world.height() != render_height {
            window.world = blit::world_texture(&window.device, render_width, render_height);
            // Tested pixel for pixel against that image, so it is exactly its
            // size or it is nothing.
            window.depth = renderer::depth_texture(&window.device, render_width, render_height);
            // And the mask with it: it is the colour attachment of a pass whose
            // depth attachment is that buffer, and wgpu requires the two to be
            // one size.
            window.outline_mask = outline::mask_texture(&window.device, render_width, render_height);
            window.select_mask = outline::mask_texture(&window.device, render_width, render_height);
            window.held_mask = outline::mask_texture(&window.device, render_width, render_height);
            // And the G-buffer, whose planes are attachments of those same
            // passes and are read texel for texel against that image.
            window.gbuffer = Gbuffer::new(&window.device, render_width, render_height);
        }
        let world_view = window.world.create_view(&wgpu::TextureViewDescriptor::default());

        // **The frame's occluders are built before its pictures are collected**,
        // and that ordering is `docs/lighting_height.md` phase 3's one real cost.
        // A static's drawn row now carries the number this grid gave it
        // (`occlusion::Occlusion::owner_at`), so that a fragment of it can say
        // which occluder it is a point of instead of having that guessed from its
        // height; collecting the pictures first would stamp numbers off the grid
        // of the frame before. Nothing else about either step changed — the
        // statics used to go first for no reason anyone recorded.
        //
        // The lights come from the same camera, cutaway and item list the passes
        // below draw from, so a torch that was not drawn casts nothing and a
        // torch that was is lighting the pixels it is standing in rather than the
        // pixels it stood in last frame.
        //
        // `assemble_geometry` is a free function for the same reason
        // `ready_atlases` is one: it is handed `&mut self.graphics` only for
        // the one field it writes (`occlusion_bake`, through
        // `frame::Inputs::bake`), and every other field it reads is a plain
        // `&`, so the signature alone says this is not a place `self.world`
        // or `self.resources` change.
        let geometry = assemble_geometry(
            &self.resources,
            &mut self.graphics,
            &self.world,
            &self.picking,
            window,
            camera,
            &cutaway,
            &tuning,
            pick.item,
            pick.mobile,
            held_item,
            held_mobile,
            &drawn,
        );
        let FrameGeometry {
            lighting,
            quads,
            static_instances,
            static_boxes,
            mesh_vertices,
            mesh_rows,
            select_quads,
            outline_quads,
            held_item_outline,
            mobile_outline,
            held_mobile_outline,
            mobile_quads,
            asked_for,
        } = geometry;
        // `fonts.mul` or the operator-supplied TrueType face, never a mix
        // within one frame — see `run`'s doc for why `ttf_font` is an
        // all-or-nothing switch. `fonts.mul` still draws into the world
        // image, at the world's own camera-scaled zoom — a bitmap font's
        // blocky nearest-sampled magnification is the look every other
        // sprite already has. A TrueType face does not go through the world
        // passes at all any more: `screen_speech` is collected in real
        // screen pixels instead, held until the HUD block further down folds
        // it into `hud_quads` for `Screen::ttf_gump_pass`'s one call — see
        // `text::ScreenLabel`'s doc for why the pass and `hud_quads`'s own
        // comment for why it has to be one call.
        let (text_quads, screen_speech): (Vec<SpriteQuad>, Vec<text::ScreenLabel<'_>>) = match &self
            .resources
            .ttf_font
        {
            Some(font) => {
                let atlas = window
                    .ttf_atlas
                    .as_mut()
                    .expect("create_window builds ttf_atlas whenever ttf_font is set");
                // Unlike `font_atlas`, `ttf_atlas` is grown a line at a time:
                // there is no bounded "whole file" to pack up front for a
                // face that answers to all of Unicode, so this asks it to
                // rasterize whatever of this frame's speech it has not seen
                // yet, the way `window.atlases` grows for graphics newly on
                // screen.
                if let Err(error) = atlas.add(font, speech.iter().flat_map(|(.., line, _, _)| line.chars())) {
                    // `eprintln!` and a frame that draws anyway, the same
                    // corner `AtlasError::Full` already cuts for the map's
                    // own atlases — see docs/client.md. Unreachable in
                    // practice: a shard's whole spoken character set is a
                    // few hundred glyphs at most, nowhere near one 2048
                    // texture.
                    eprintln!("packing ttf glyphs: {error}");
                }
                let screen_speech = speech
                    .iter()
                    .map(|(anchor, line, _font, hue)| {
                        // `to_viewport` and not the projection directly:
                        // it is the one place that already undoes both a
                        // magnifying zoom's vertex-shader scale *and* a
                        // minifying one's blit-shrink with the same number
                        // — see its own doc. `viewport`'s own corner is
                        // added because `to_viewport` answers in pixels of
                        // the rect the world goes into, not the surface.
                        let real = camera.to_viewport(*anchor);
                        text::ScreenLabel {
                            anchor: GumpPixel::new(
                                viewport.x as i32 + real.x.round() as i32,
                                viewport.y as i32 + real.y.round() as i32,
                            ),
                            text: line.as_str(),
                            hue: *hue,
                        }
                    })
                    .collect();
                (Vec::new(), screen_speech)
            }
            None => {
                let labels: Vec<Label<'_>> = speech
                    .iter()
                    .map(|(anchor, line, font, hue)| Label {
                        anchor: *anchor,
                        text: line.as_str(),
                        font: *font,
                        hue: *hue,
                        // Nearer than anything the world draws, rather than
                        // an `Order` of its own: speech reads as an overlay
                        // above whoever said it in every reference client,
                        // and there is no real case here of a wall in front
                        // of the speaker hiding it that a viewer would want
                        // honoured. Worth revisiting with a
                        // `depth::text_priority_z` alongside the mobile's
                        // own if that ever stops being true.
                        depth: 0.0,
                    })
                    .collect();
                (text::collect(&labels, &self.resources.font_atlas), Vec::new())
            }
        };
        // Uploads whatever the `add` above (and the HUD's own, further down
        // this frame) packed fresh — see `Screen::upload_ttf_dirty`'s doc for
        // why this is the one place both call through rather than each taking
        // `TtfAtlas::take_dirty` for itself.
        window.upload_ttf_dirty();
        let depth_view = window.depth.create_view(&wgpu::TextureViewDescriptor::default());
        let gbuffer_views = window.gbuffer.views();
        let target = Target {
            view: &world_view,
            depth: &depth_view,
            gbuffer: &gbuffer_views,
            width: render_width,
            height: render_height,
            projection: camera.projection(),
        };
        let mut encoder = window
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        // `encode_world_passes` is a free function for the same reason
        // `assemble_geometry` is one: every world pass it records is drawn
        // from the values handed to it, and the one thing on `self` it
        // writes — `graphics.solids_held`/`graphics.solids_drawn` — is
        // written through the `&mut GraphicsSettings` its signature already
        // names, not through a `&mut self` that would let it touch anything
        // else too.
        encode_world_passes(
            &mut self.graphics,
            &self.picking,
            window,
            &mut encoder,
            target,
            &view,
            &world_view,
            &gbuffer_views,
            viewport,
            camera,
            solid_cut,
            &quads,
            &static_instances,
            &static_boxes,
            &mesh_vertices,
            &mesh_rows,
            &mobile_quads,
            &text_quads,
            &outline_quads,
            &held_item_outline,
            &mobile_outline,
            &held_mobile_outline,
            &select_quads,
            &lighting,
            render_width,
            render_height,
        );
        // The shard's dialogs, in the client's own art, over the finished
        // picture and under egui's.
        //
        // Under egui and not over it, deliberately: the widgets that *answer* a
        // gump are still egui's, laid out at the same coordinates in the same
        // units — one gump pixel is one egui point, and the scale below is the
        // window's own scale factor, which is what makes those two spaces the
        // same one. So the art draws the window and egui's transparent widgets
        // sit exactly on it. See `client/app/src/gump.rs`.
        //
        // The atlas grows here rather than when the packet arrived: a page
        // button flips pages inside the client, so what a window needs is every
        // page's art and not the showing one's — `gump::art_of` is that list,
        // and it is asked for on the frame the window is drawn on because that
        // is the frame that knows the window is open at all.
        // `draw_gump_windows` is a free function for the same reason as its
        // neighbours above: `resources.gump_atlas` and `windows.drawn_windows`
        // are the two things on `self` it really writes, and both are named
        // in its signature rather than reached through `&mut self`.
        draw_gump_windows(
            &mut self.resources,
            &self.world,
            &mut self.windows,
            self.shell.as_ref(),
            window,
            &mut encoder,
            &view,
        );
        // Every line of gump-space text this frame, from both the blocks below.
        //
        // **One list because there is one pass.** `GumpRenderer` holds a single
        // instance buffer, and `queue.write_buffer` lands before the encoder is
        // submitted — so two `render` calls in a frame do not draw twice, they
        // draw the *second* call's instances twice and lose the first's. That
        // is what happened to every window's text for as long as there was a
        // line in the journal to overwrite it with: a paperdoll's name plate
        // was written, cut, submitted, and then quietly replaced by the chat.
        let mut text_quads: Vec<SpriteQuad> = Vec::new();
        // What the windows have written on them: a dialog's captions and
        // fields, and a paperdoll's name.
        //
        // A second pass over the same surface rather than more quads in the one
        // above, because a draw call binds one texture and a glyph lives in the
        // font atlas. After the art and therefore over it, which is the order
        // the reference draws a window's text controls in — and after the block
        // above rather than inside it, because that one holds the gump pass
        // mutably and this needs the text pass.
        //
        // Read off `drawn_windows` — the list the frame was just drawn from and
        // the one the pointer will be tested against — so a caption cannot end
        // up on a window laid out differently from the pictures under it.
        {
            let mut labels: Vec<openshard_client_render::text::GumpLabel<'_>> = Vec::new();
            // The lines that are cut to a box before they are drawn, already
            // quads: they cannot travel with the labels above, because what
            // cuts them is the window they belong to and a label does not carry
            // one. Extended onto the same draw call, which is the point — one
            // texture, one pass, whatever produced the quads.
            let mut cut: Vec<SpriteQuad> = Vec::new();
            for (subject, drawn) in &self.windows.drawn_windows {
                match (subject, drawn) {
                    (WindowSubject::Dialog(gump_id), Drawn::Dialog(laid_out)) => {
                        if let Some(gump) = self
                            .world
                            .view
                            .as_ref()
                            .and_then(|view| view.gumps.iter().find(|gump| gump.gump_id == *gump_id))
                        {
                            labels.extend(self.windows.dialogs.lines(
                                gump,
                                laid_out,
                                &self.resources.font_atlas,
                                self.resources.cliloc.as_ref(),
                            ));
                        }
                    }
                    (WindowSubject::Paperdoll(serial), Drawn::Paperdoll(_)) => {
                        // The name the `0x88` carried, which is the only place
                        // this string exists — see `view::Paperdoll::name`. The
                        // window's own origin comes off `own_windows` rather
                        // than the doll, for the reason the plate is part of the
                        // frame: it moves with the window and not with the body.
                        let at = self
                            .windows
                            .own_windows
                            .iter()
                            .find(|window| window.subject == *subject)
                            .map(|window| window.at);
                        if let (Some(at), Some(doll)) = (
                            at,
                            self.world
                                .view
                                .as_ref()
                                .and_then(|view| view.paperdolls.get(serial)),
                        ) {
                            labels.push(paperdoll::name(&doll.name, at));
                        }
                    }
                    // The skill window writes its own lines — a heading, a
                    // name, a value, the total — and they are the one text in
                    // this client that is *cut*: a row half out of the viewport
                    // is drawn half, which is what a bar the player drags means.
                    // Collected and cut here rather than added to `labels`,
                    // because a glyph is a quad in the font atlas and has no
                    // picture to carry a box on — see `Scissor::cut`.
                    (WindowSubject::Skills, Drawn::Skills(sheet)) => {
                        for line in &sheet.lines {
                            let mut quads = openshard_client_render::text::collect_gump(
                                &[line.label()],
                                &self.resources.font_atlas,
                            );
                            // Its own box, and not the window's: the rows are cut
                            // to the list and the total written under them is not
                            // — see `skills::Line::scissor`, which is a
                            // difference this found out by drawing it wrong.
                            if let Some(scissor) = line.scissor {
                                scissor.cut(&mut quads);
                            }
                            cut.extend(quads);
                        }
                    }
                    _ => {}
                }
            }
            text_quads.extend(openshard_client_render::text::collect_gump(
                &labels,
                &self.resources.font_atlas,
            ));
            text_quads.extend(cut);
        }
        // `draw_chat_and_speech` is a free function like its neighbours
        // above, though a plainer one: nothing it is handed is written back
        // to `self` at all, only appended to the caller's own `text_quads`.
        draw_chat_and_speech(
            &self.resources,
            &self.world,
            &self.chat,
            self.shell.as_ref(),
            window,
            &mut encoder,
            &view,
            chat_style,
            &screen_speech,
            &mut text_quads,
        );
        // The UI over it, with no depth attachment: the world's depth buffer
        // ordered the world, and this is drawn on the result.
        if let (Some(shell), Some((_, output, _))) = (self.shell.as_mut(), ui) {
            let painting = Instant::now();
            let timed = profile::begin(window.gpu.as_ref(), "egui", &mut encoder);
            shell.paint(
                &window.device,
                &window.queue,
                &mut encoder,
                &view,
                output,
                [window.config.width, window.config.height],
            );
            profile::end(window.gpu.as_ref(), &mut encoder, timed);
            ui_cost += painting.elapsed();
        }
        // Every query closed above, copied out of its set and into the buffer
        // the next frame will map — recorded into this encoder, so it has to
        // happen before the submit and after the last `profile::end`.
        if let Some(gpu) = window.gpu.as_mut() {
            gpu.resolve(&mut encoder);
        }
        window.queue.submit([encoder.finish()]);
        // And the frame closed, which is what makes those buffers eligible to be
        // mapped. What comes back is an older frame's timings — see [`profile`]
        // for why that is the right trade and not a defect.
        if let Some(gpu) = window.gpu.as_mut() {
            gpu.end_frame(&window.device, &window.queue);
        }
        // **This frame, written out** — F12, and `docs/parity.md`'s first
        // backlog item. After the submit above and not beside the blit, because
        // what is read back has to be pixels the device has actually been given
        // the commands for; the world image, the G-buffer and the instance
        // buffers all still hold this frame's own, since nothing writes them
        // again until the next one.
        //
        // Not the surface: what is presented has the HUD, the panels and the
        // solids overlay on top of it, and a tool's frame has none of those.
        // What a comparison wants is the world as the blit left it, so the blit
        // is run again into a texture of its own — the same pass, the same
        // lighting, the same rect — once per plane. `docs/parity.md` D5.
        if let Some(into) = self.graphics.frame_dump.take() {
            let dump = window.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame dump"),
                size: wgpu::Extent3d {
                    width: window.config.width,
                    height: window.config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // **The world's format and not the surface's**, which is what
                // the first press of F12 found: a surface is whatever the
                // compositor offered — here `Rgba16Float`, eight bytes a texel
                // — and reading it back as RGBA8 is a copy `wgpu` refuses. Even
                // where it is four (`Bgra8Unorm`) it is the wrong four: the
                // picture would come out with its red and blue swapped, and
                // nothing would say so. `isolated_scene` has always drawn into
                // this format, and a dump exists to be compared with that one.
                format: blit::WORLD_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let dump_view = dump.create_view(&wgpu::TextureViewDescriptor::default());
            // A second pipeline for that format, built here and dropped with the
            // dump: `Screen::blit` is bound to the surface's format and cannot
            // draw into this target. The same shader and the same uniforms — the
            // format is the whole of the difference — and it is built per press
            // rather than kept, because every frame that is not being dumped
            // would otherwise carry a pipeline nobody draws with.
            let mut dump_blit = Blit::new(&window.device, blit::WORLD_FORMAT);
            let planes = openshard_client_render::dump::planes(
                &window.device,
                &window.queue,
                &mut dump_blit,
                &dump,
                blit::Frame {
                    // A view of `dump`, made one line above it: `dump::planes`'s
                    // own contract, and the whole of what this call site has to
                    // keep true.
                    target: &dump_view,
                    world: &world_view,
                    gbuffer: &gbuffer_views,
                    face_instances: window.statics.instances_buffer(),
                    mobile_instances: window.mobile_pass.instances_buffer(),
                    mesh_instances: window.mesh_pass.rows_buffer(),
                    ground_instances: window.renderer.instances_buffer(),
                    zoom: camera.zoom(),
                    rect: viewport,
                },
                &lighting,
                // Every one of them. A dump taken because something looked wrong
                // is taken once, and a plane left out is a plane somebody has to
                // reproduce the moment they want it — by which time the frame is
                // gone.
                &View::ALL,
            );
            match write_frame_dump(
                &into,
                &planes,
                asked_for
                    .as_deref()
                    .expect("the summary is taken whenever a dump is armed, above"),
            ) {
                Ok(()) => tracing::info!(
                    into = %into.display(),
                    planes = planes.len(),
                    "frame dumped",
                ),
                // A dump that could not be written is a diagnostic that failed,
                // not a frame that failed: the client goes on drawing.
                Err(error) => tracing::warn!(into = %into.display(), %error, "dumping the frame"),
            }
        }
        // Presentation moved onto the queue in wgpu 30; the texture is consumed.
        window.queue.present(frame);
        // And the next frame is asked for here rather than through the timer,
        // unconditionally while somebody is watching. This is the pacer: the
        // surface presents in FIFO, so `get_current_texture` above blocks the
        // next frame until the display has taken this one, and asking again
        // straight away runs the loop at the display's own rate instead of at a
        // 16ms timer that beats against it.
        //
        // Every frame and not only the gliding ones, which is the change: a
        // client that only redrew when something moved dropped to 12.5 frames a
        // second the moment the player stood still, and however correct the
        // reason was, what it looked like was a stall. The timer stays for the
        // window nobody is looking at — see [`App::pacing`].
        if watched {
            window.window.request_redraw();
        }
        let took = started.elapsed();
        // The interval between two *drawn* frames, and where this one's time
        // went: the pacing and the price, which are the two things a drop in
        // frame rate can be — and the price split between the panels and the
        // world, which are the two things the price can be. See [`frames`].
        //
        // The scene is what is left after the UI and the wait rather than a
        // fourth clock, so the three always add up to the frame exactly: a
        // fourth `Instant` would leave a remainder nobody could account for.
        let scene = took.saturating_sub(ui_cost).saturating_sub(wait);
        // The device's own number, which is *not* about this frame: it is
        // whichever frame the timestamps have come back for, two or three ago.
        // Recorded against this one anyway, because what it answers — "is the
        // wait above slack or a stall" — is a question about a standing cost and
        // not about one frame's spike. See [`profile`].
        let gpu = self
            .window
            .as_ref()
            .and_then(|window| window.gpu.as_ref())
            .map(profile::Gpu::total);
        self.frames.record(
            started.saturating_duration_since(self.last_frame),
            ui_cost,
            scene,
            wait,
            gpu,
            repacked,
        );
        self.last_frame = started;
    }
}

/// Where a frame dump goes: `OPENSHARD_FRAME_DUMP_DIR`, or a directory of our
/// own under the system temp.
///
/// Never the source tree — one dump is thirteen uncompressed pictures and none
/// of them belongs in a diff. The same rule, and the same shape, as
/// [`dst::dump_dir`](crate::dst)'s.
///
/// **Not `OPENSHARD_FRAME_DUMP`**, which the render crate's own tools already
/// read as the *file* their one picture is written to
/// (`examples/isolated_scene.rs`, `tests/cost.rs`). One name meaning a file to
/// one caller and a directory to another is precisely the quiet difference
/// `docs/parity.md` exists to stop, so the client's knob is its own name — and a
/// directory, because what the client has to dump is every plane at once plus
/// the inputs they came from.
pub(crate) fn frame_dump_root() -> std::path::PathBuf {
    std::env::var_os("OPENSHARD_FRAME_DUMP_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("openshard-frame"))
}

/// One dump: a directory holding a picture per plane, named for the plane, and
/// the inputs the frame was assembled from.
///
/// The summary is written last on purpose — a directory that has `inputs.txt` in
/// it has every picture beside it, so a reader never compares a half-written
/// dump against a whole one.
///
/// **`inputs.txt` is written verbatim and gets no line of its own from here**,
/// which is worth stating because one line of it reads oddly beside the
/// directory: `view` is what the *window* was showing when the key was pressed,
/// while each picture beside it is named for the plane it actually is. Adding a
/// note to explain that would be a line the tool's own summary does not have,
/// and the two are written to be diffed — an extra line here is a difference in
/// every comparison, forever, to save one sentence of documentation.
pub(crate) fn write_frame_dump(
    into: &std::path::Path,
    planes: &[(View, Vec<u8>)],
    asked_for: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(into)?;
    for (view, png) in planes {
        std::fs::write(into.join(format!("{}.png", view.name())), png)?;
    }
    std::fs::write(into.join("inputs.txt"), asked_for)
}
