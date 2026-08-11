//! The typed line and its rendering, together: [`Chat`] is what has not been
//! sent yet, and [`draw_chat_and_speech`] is the speech line and the journal
//! above it, over the finished picture and under egui's.

use openshard_client_render::gump::{self as gump_art, GumpPixel};
use openshard_client_render::sprite::SpriteQuad;
use openshard_client_render::text::{self, GumpLabel};
use openshard_protocol::speech::Font;
use openshard_protocol::wire::Hue;

use crate::window::Screen;
use crate::{
    CHAT_LINE_HEIGHT, CHAT_LINES, CHAT_MARGIN, desk, profile, resources, scaled_gump_quads, shell, world,
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

/// The speech line and the journal above it, over the finished picture and
/// under egui's — the same corner `shell::speech_line`'s `egui::Panel::bottom`
/// used to claim before this moved to the client's own rendering. Always
/// drawn, unlike `crate::presentation::draw_gump_windows`: the font atlas
/// needs no shard-sent gump art to exist, so there is nothing here to be
/// `None` until.
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
