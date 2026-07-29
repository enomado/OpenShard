//! Generic gumps: the server-driven dialog (`0xB0`) and the client's reply
//! (`0xB1`).
//!
//! A gump is UO's windowing primitive — a paperdoll, a shopkeeper, a book, and
//! the staff menus this shard needs are all gumps. The server sends a *layout*: a
//! little command language (`{ resizepic ... }`, `{ button ... }`, `{ page N }`)
//! plus a list of text strings the layout refers to by index. The client renders
//! it and, when the player clicks a button, sends back the button id in a `0xB1`.
//!
//! This is the uncompressed `0xB0` form. Clients from 5.0 also accept a zlib-packed
//! `0xDD`, and ClassicUO and the modern 2D client both still render `0xB0`, so the
//! compression is a later optimisation, not a requirement.

use crate::codec::{PacketReader, PacketWriter};
use crate::error::DecodeError;
use crate::packet::{DecodePacket, EncodePacket, PacketLength};
use crate::version::ClientVersion;

/// The client's white, in the 15-bit colour a gump element takes.
pub const GUMP_WHITE: u32 = 0x7FFF;
/// ServUO's `BaseQuestGump.DarkGreen` — the colour a quest title is drawn in.
pub const GUMP_DARK_GREEN: u32 = 10_000;
/// ServUO's `BaseQuestGump.LightGreen` — body text and objective labels.
pub const GUMP_LIGHT_GREEN: u32 = 90_000;
/// The red a failed quest is drawn in (ServUO passes `0x3C00` literally).
pub const GUMP_RED: u32 = 0x3C00;

/// Expand one of the client's 15-bit colours to the 24-bit RGB an HTML
/// `<BASEFONT COLOR=#RRGGBB>` tag wants. ServUO's `BaseQuestGump.C16232`.
#[must_use]
pub const fn gump_color_rgb(c16: u32) -> u32 {
    let c16 = c16 & 0x7FFF;
    let r = ((c16 >> 10) & 0x1F) << 3;
    let g = ((c16 >> 5) & 0x1F) << 3;
    let b = (c16 & 0x1F) << 3;
    (r << 16) | (g << 8) | b
}

/// What a gump button does when it is clicked.
///
/// `Reply` closes the gump and sends a `0xB1`; `Page` is handled entirely by the
/// client, flipping to another page of the same gump without a round trip.
/// ServUO's `GumpButtonType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GumpButton {
    /// Send the id back to the server.
    Reply,
    /// Flip to another page, client-side.
    Page,
}

impl GumpButton {
    const fn code(self) -> i64 {
        match self {
            Self::Page => 0,
            Self::Reply => 1,
        }
    }
}

/// A gump layout under construction: the command string, plus the text lines it
/// refers to by index.
///
/// The wire format is a little command language (`{ resizepic 0 0 5054 200 200 }`)
/// and a separate table of strings; an element that shows text stores it in the
/// table and names its index. Writing that by hand works for a two-button dialog
/// and does not survive a real one — the quest log is forty-odd elements, and one
/// mistyped keyword renders as an empty window with nothing to debug. So the
/// elements are methods, and each one is ported from the matching
/// `Server/Gumps/Gump*.cs` in ServUO: the keyword *and* the argument order, both
/// of which the client reads positionally.
///
/// Call [`finish`](Self::finish) and hand the two halves to
/// [`GumpDisplay`].
#[derive(Clone, Default, Debug)]
pub struct GumpLayout {
    layout: String,
    lines: Vec<String>,
}

// A gump element's arguments are the client's, in the client's order — grouping
// them into structs would hide exactly the thing that has to be checked against
// ServUO line by line. Long argument lists are the format, not a design choice.
#[allow(clippy::too_many_arguments)]
impl GumpLayout {
    /// An empty layout.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a text line and return the index the layout refers to it by.
    ///
    /// Identical lines are *not* pooled: ServUO does not pool them either, and an
    /// index that silently aliases another element's text is a worse surprise than
    /// a few duplicated strings.
    pub fn text(&mut self, line: impl Into<String>) -> usize {
        self.lines.push(line.into());
        self.lines.len() - 1
    }

    /// The window may not be dragged.
    pub fn no_move(&mut self) {
        self.layout.push_str("{ nomove }");
    }

    /// The window has no close box — the player must answer with a button.
    pub fn no_close(&mut self) {
        self.layout.push_str("{ noclose }");
    }

    /// The window cannot be dismissed with right-click.
    pub fn no_dispose(&mut self) {
        self.layout.push_str("{ nodispose }");
    }

    /// The window cannot be resized.
    pub fn no_resize(&mut self) {
        self.layout.push_str("{ noresize }");
    }

    /// Start a page. Page `0` is drawn on every page; the rest are switched
    /// between by a [`GumpButton::Page`] button, client-side.
    pub fn page(&mut self, page: u32) {
        self.element("page", &[i64::from(page)]);
    }

    /// A gump-art image.
    pub fn image(&mut self, x: i32, y: i32, gump: u32) {
        self.element("gumppic", &[i64::from(x), i64::from(y), i64::from(gump)]);
    }

    /// A gump-art image in a hue. The hue rides as a `hue=` suffix, not a plain
    /// argument — the client parses the two forms differently.
    pub fn image_hued(&mut self, x: i32, y: i32, gump: u32, hue: u32) {
        self.element("gumppic", &[i64::from(x), i64::from(y), i64::from(gump)]);
        // Re-open the element the way ServUO's `GumpImage` does: the suffix is
        // appended inside the braces, after the id.
        let brace = self.layout.pop();
        debug_assert_eq!(brace, Some('}'));
        self.layout.pop(); // the space before it
        self.layout.push_str(&format!(" hue={hue} }}"));
    }

    /// An image tiled to fill a rectangle.
    pub fn image_tiled(&mut self, x: i32, y: i32, width: i32, height: i32, gump: u32) {
        self.element(
            "gumppictiled",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(width),
                i64::from(height),
                i64::from(gump),
            ],
        );
    }

    /// A darkened, semi-transparent rectangle.
    pub fn alpha_region(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.element(
            "checkertrans",
            &[i64::from(x), i64::from(y), i64::from(width), i64::from(height)],
        );
    }

    /// A stretched background frame. Note the order: the gump id comes *before*
    /// the size, unlike every other element.
    pub fn background(&mut self, x: i32, y: i32, width: i32, height: i32, gump: u32) {
        self.element(
            "resizepic",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(gump),
                i64::from(width),
                i64::from(height),
            ],
        );
    }

    /// A button. `normal` and `pressed` are gump art; `id` is what comes back in
    /// the `0xB1` (for [`GumpButton::Reply`]) or the page to flip to (for
    /// [`GumpButton::Page`], where `param` carries the page instead).
    pub fn button(
        &mut self,
        x: i32,
        y: i32,
        normal: u32,
        pressed: u32,
        kind: GumpButton,
        param: u32,
        id: u32,
    ) {
        self.element(
            "button",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(normal),
                i64::from(pressed),
                kind.code(),
                i64::from(param),
                i64::from(id),
            ],
        );
    }

    /// A radio button. Only one per group may be on; the client returns the
    /// `switch_id` of whichever is set when a reply button is pressed.
    pub fn radio(&mut self, x: i32, y: i32, off: u32, on: u32, initial: bool, switch_id: u32) {
        self.element(
            "radio",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(off),
                i64::from(on),
                i64::from(initial),
                i64::from(switch_id),
            ],
        );
    }

    /// A checkbox. Like [`radio`](Self::radio), but independent of its neighbours.
    pub fn check(&mut self, x: i32, y: i32, off: u32, on: u32, initial: bool, switch_id: u32) {
        self.element(
            "checkbox",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(off),
                i64::from(on),
                i64::from(initial),
                i64::from(switch_id),
            ],
        );
    }

    /// A one-line label in a hue.
    pub fn label(&mut self, x: i32, y: i32, hue: u32, text: impl Into<String>) {
        let line = self.text(text);
        self.element("text", &[i64::from(x), i64::from(y), i64::from(hue), line as i64]);
    }

    /// A label clipped to a box rather than overflowing it.
    pub fn cropped_label(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        hue: u32,
        text: impl Into<String>,
    ) {
        let line = self.text(text);
        self.element(
            "croppedtext",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(width),
                i64::from(height),
                i64::from(hue),
                line as i64,
            ],
        );
    }

    /// A block of HTML text.
    pub fn html(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        text: impl Into<String>,
        background: bool,
        scrollbar: bool,
    ) {
        let line = self.text(text);
        self.element(
            "htmlgump",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(width),
                i64::from(height),
                line as i64,
                i64::from(background),
                i64::from(scrollbar),
            ],
        );
    }

    /// A block of HTML text in one of the client's 15-bit colours.
    ///
    /// There is no colour argument on `htmlgump`: ServUO wraps the text in a
    /// `<BASEFONT>` tag instead (`BaseQuestGump.Color`), which is why this takes
    /// the same colour constants the localized form does and converts.
    pub fn html_colored(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        text: &str,
        color: u32,
        background: bool,
        scrollbar: bool,
    ) {
        let rgb = gump_color_rgb(color);
        let wrapped = format!("<BASEFONT COLOR=#{rgb:06X}>{text}</BASEFONT>");
        self.html(x, y, width, height, wrapped, background, scrollbar);
    }

    /// A localized string — the client looks the number up in its own `cliloc`
    /// file, so no text travels.
    pub fn html_localized(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        cliloc: u32,
        background: bool,
        scrollbar: bool,
    ) {
        self.element(
            "xmfhtmlgump",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(width),
                i64::from(height),
                i64::from(cliloc),
                i64::from(background),
                i64::from(scrollbar),
            ],
        );
    }

    /// A localized string in a colour.
    pub fn html_localized_colored(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        cliloc: u32,
        color: u32,
        background: bool,
        scrollbar: bool,
    ) {
        self.element(
            "xmfhtmlgumpcolor",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(width),
                i64::from(height),
                i64::from(cliloc),
                i64::from(background),
                i64::from(scrollbar),
                i64::from(color),
            ],
        );
    }

    /// A localized string with `~1_VAL~`-style arguments filled in, tab-separated.
    ///
    /// Note the argument order: unlike every other localized form, the cliloc
    /// comes *after* the flags and colour. ServUO's `xmfhtmltok`.
    pub fn html_localized_args(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        cliloc: u32,
        args: &str,
        color: u32,
        background: bool,
        scrollbar: bool,
    ) {
        self.element(
            "xmfhtmltok",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(width),
                i64::from(height),
                i64::from(background),
                i64::from(scrollbar),
                i64::from(color),
                i64::from(cliloc),
            ],
        );
        let brace = self.layout.pop();
        debug_assert_eq!(brace, Some('}'));
        self.layout.pop();
        self.layout.push_str(&format!(" @{args}@ }}"));
    }

    /// An item, drawn from the art the world uses rather than the gump art.
    pub fn item(&mut self, x: i32, y: i32, graphic: u32, hue: u32) {
        if hue == 0 {
            self.element("tilepic", &[i64::from(x), i64::from(y), i64::from(graphic)]);
        } else {
            self.element(
                "tilepichue",
                &[i64::from(x), i64::from(y), i64::from(graphic), i64::from(hue)],
            );
        }
    }

    /// A text field the player types into; its contents come back in the `0xB1`
    /// under `entry_id`.
    pub fn text_entry(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        hue: u32,
        entry_id: u32,
        initial: impl Into<String>,
    ) {
        let line = self.text(initial);
        self.element(
            "textentry",
            &[
                i64::from(x),
                i64::from(y),
                i64::from(width),
                i64::from(height),
                i64::from(hue),
                i64::from(entry_id),
                line as i64,
            ],
        );
    }

    /// The finished layout string and its text table.
    #[must_use]
    pub fn finish(&self) -> (&str, &[String]) {
        (&self.layout, &self.lines)
    }

    /// One `{ keyword arg arg ... }` element.
    fn element(&mut self, keyword: &str, args: &[i64]) {
        self.layout.push_str("{ ");
        self.layout.push_str(keyword);
        for arg in args {
            self.layout.push(' ');
            self.layout.push_str(&arg.to_string());
        }
        self.layout.push_str(" }");
    }
}

/// `0xBF` subcommand `0x04` — close an open gump on the client. Fixed 13 bytes.
///
/// The server sends this before re-sending a dialog that replaces itself (the
/// quest gump's own page chain does exactly that); without it the client stacks
/// the new window on the old one. `button` is what the closed gump reports as its
/// answer — ServUO passes `0` for a silent close. Ported from ServUO's
/// `CloseGump`.
///
/// Fixed despite living under `0xBF`, for the same reason [`crate::mobile::StatLocks`]
/// is: the body never varies, so the constant `u16(13)` is written here by hand,
/// since [`crate::packet::frame_body`] only back-patches a length for
/// [`PacketLength::Variable`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CloseGump {
    /// Which dialog to close.
    pub gump_id: u32,
    /// What the closed gump reports as its answer. ServUO passes `0`.
    pub button: u32,
}

impl EncodePacket for CloseGump {
    const ID: u8 = 0xBF;
    const LENGTH: PacketLength = PacketLength::Fixed(13);

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u16(13);
        out.u16(0x04);
        out.u32(self.gump_id);
        out.u32(self.button);
    }
}

/// `0xB0` — display a generic gump. Variable length.
///
/// `serial` is the context the gump belongs to (a mobile, an item, or `0` for a
/// standalone dialog); `gump_id` names *which* dialog, and the client echoes both
/// back in its `0xB1` so the server knows what was answered. `layout` is the gump
/// command string; `lines` are the strings it references by index (`{ text X Y
/// LINE }` picks `lines[LINE]`), sent as big-endian UTF-16.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GumpDisplay {
    /// The context the gump belongs to (a mobile, an item, or `0` standalone).
    pub serial: u32,
    /// Which dialog — echoed back in the `0xB1`.
    pub gump_id: u32,
    /// Screen x.
    pub x: i32,
    /// Screen y.
    pub y: i32,
    /// The gump command string — see [`GumpLayout::finish`].
    pub layout: String,
    /// The strings the layout refers to by index.
    pub lines: Vec<String>,
}

impl EncodePacket for GumpDisplay {
    const ID: u8 = 0xB0;
    const LENGTH: PacketLength = PacketLength::Variable;

    fn encode_body(&self, out: &mut PacketWriter, _version: ClientVersion) {
        out.u32(self.serial);
        out.u32(self.gump_id);
        out.u32(self.x as u32);
        out.u32(self.y as u32);

        // The layout is ASCII with a byte-count prefix; the client reads exactly
        // that many bytes, so no terminator is needed.
        let layout = self.layout.as_bytes();
        out.u16(u16::try_from(layout.len()).unwrap_or(u16::MAX));
        out.bytes(layout);

        // The text table: a count, then each line as its own UTF-16 run.
        out.u16(u16::try_from(self.lines.len()).unwrap_or(u16::MAX));
        for line in &self.lines {
            let units: Vec<u16> = line.encode_utf16().collect();
            out.u16(u16::try_from(units.len()).unwrap_or(u16::MAX));
            for unit in units {
                out.u16(unit); // big-endian, like every u16 on the wire
            }
        }
    }
}

/// `0xB1` — the client's answer to a gump: which button was pressed, plus the
/// state of any switches (checkboxes, radios) and text fields.
///
/// A button of `0` is the close box — the player dismissed the gump without
/// choosing. Otherwise it is the id the layout gave the button that was clicked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GumpResponse {
    /// The context serial the gump was opened on.
    pub serial: u32,
    /// Which dialog answered — the `gump_id` the server sent.
    pub gump_id: u32,
    /// The button pressed; `0` means the gump was closed without a choice.
    pub button: u32,
    /// The ids of the switches (checkboxes, radio buttons) left *on*.
    pub switches: Vec<u32>,
    /// Text fields, as `(field id, contents)`.
    pub text_entries: Vec<(u16, String)>,
}

impl DecodePacket for GumpResponse {
    const ID: u8 = 0xB1;

    /// `decode_packet` has already skipped the length field for a `Variable`
    /// id — see its doc comment — so this starts straight at the serial.
    fn decode_body(reader: &mut PacketReader<'_>, _version: ClientVersion) -> Result<Self, DecodeError> {
        let serial = reader.u32()?;
        let gump_id = reader.u32()?;
        let button = reader.u32()?;

        let switch_count = reader.u32()? as usize;
        let mut switches = Vec::with_capacity(switch_count.min(64));
        for _ in 0..switch_count {
            switches.push(reader.u32()?);
        }

        let text_count = reader.u32()? as usize;
        let mut text_entries = Vec::with_capacity(text_count.min(64));
        for _ in 0..text_count {
            let id = reader.u16()?;
            let len = reader.u16()? as usize;
            let mut units = Vec::with_capacity(len);
            for _ in 0..len {
                units.push(reader.u16()?);
            }
            text_entries.push((id, String::from_utf16_lossy(&units)));
        }

        Ok(Self {
            serial,
            gump_id,
            button,
            switches,
            text_entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{decode_packet, encode_packet};

    fn version() -> ClientVersion {
        ClientVersion::new(7, 0, 45, 65)
    }

    #[test]
    fn a_built_element_matches_the_string_it_replaces() {
        // The exact bytes the admin gump has been hand-writing, from the builder.
        let mut layout = GumpLayout::new();
        layout.background(0, 0, 200, 200, 5054);
        layout.button(10, 10, 4005, 4007, GumpButton::Reply, 0, 1);
        assert_eq!(
            layout.finish().0,
            "{ resizepic 0 0 5054 200 200 }{ button 10 10 4005 4007 1 0 1 }"
        );
    }

    #[test]
    fn a_negative_coordinate_stays_negative() {
        // The quest frame puts an element at x = -16. Formatting it through an
        // unsigned type would send 4294967280 and the client would drop the whole
        // layout, which looks like a gump that simply never opens.
        let mut layout = GumpLayout::new();
        layout.image(-16, 285, 0x28A2);
        assert_eq!(layout.finish().0, "{ gumppic -16 285 10402 }");
    }

    #[test]
    fn a_label_interns_its_text_and_names_the_index() {
        let mut layout = GumpLayout::new();
        layout.label(20, 18, 1153, "Quest Log");
        layout.label(20, 40, 1153, "A Plague of Rats");
        let (string, lines) = layout.finish();
        assert_eq!(string, "{ text 20 18 1153 0 }{ text 20 40 1153 1 }");
        assert_eq!(lines, ["Quest Log", "A Plague of Rats"]);
    }

    #[test]
    fn window_flags_come_before_the_elements() {
        let mut layout = GumpLayout::new();
        layout.no_close();
        layout.no_resize();
        layout.page(0);
        assert_eq!(layout.finish().0, "{ noclose }{ noresize }{ page 0 }");
    }

    #[test]
    fn a_hued_image_carries_its_hue_as_a_suffix() {
        // `hue=` is parsed differently from a positional argument; a plain fifth
        // number is silently ignored.
        let mut layout = GumpLayout::new();
        layout.image_hued(5, 6, 100, 33);
        assert_eq!(layout.finish().0, "{ gumppic 5 6 100 hue=33 }");
    }

    #[test]
    fn coloured_html_wraps_the_text_in_a_basefont_tag() {
        let mut layout = GumpLayout::new();
        layout.html_colored(
            98,
            156,
            312,
            180,
            "Slay five rats.",
            GUMP_LIGHT_GREEN,
            false,
            true,
        );
        let (string, lines) = layout.finish();
        assert_eq!(string, "{ htmlgump 98 156 312 180 0 0 1 }");
        assert_eq!(lines, ["<BASEFONT COLOR=#B8E080>Slay five rats.</BASEFONT>"]);
    }

    #[test]
    fn a_localized_line_with_arguments_ends_in_its_argument_run() {
        let mut layout = GumpLayout::new();
        layout.html_localized_args(98, 140, 312, 16, 1_074_782, "Britain", GUMP_WHITE, false, false);
        assert_eq!(
            layout.finish().0,
            "{ xmfhtmltok 98 140 312 16 0 0 32767 1074782 @Britain@ }"
        );
    }

    #[test]
    fn a_close_gump_names_its_dialog_and_button() {
        let bytes = encode_packet(
            &CloseGump {
                gump_id: 0x0051_0001,
                button: 0,
            },
            version(),
        );
        assert_eq!(bytes.len(), 13, "ServUO's EnsureCapacity(13)");
        assert_eq!(bytes[0], 0xBF);
        assert_eq!(u16::from_be_bytes([bytes[1], bytes[2]]), 13);
        assert_eq!(u16::from_be_bytes([bytes[3], bytes[4]]), 0x04);
        assert_eq!(
            u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]),
            0x0051_0001
        );
    }

    #[test]
    fn a_built_layout_round_trips_through_the_display_packet() {
        let mut layout = GumpLayout::new();
        layout.background(0, 0, 340, 220, 5054);
        layout.label(20, 18, 1153, "A Plague of Rats");
        let (string, lines) = layout.finish();
        let bytes = encode_packet(
            &GumpDisplay {
                serial: 0,
                gump_id: 0x0051_0001,
                x: 75,
                y: 25,
                layout: string.to_owned(),
                lines: lines.to_vec(),
            },
            version(),
        );
        let declared = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
        assert_eq!(declared, bytes.len());
    }

    #[test]
    fn a_radio_reply_carries_its_switches() {
        // The resign dialog is radios plus one OK button: without the switch ids
        // the answer is "OK" with no idea what was chosen.
        let mut layout = GumpLayout::new();
        layout.radio(100, 100, 0x25F8, 0x25FB, false, 0);
        layout.radio(100, 120, 0x25F8, 0x25FB, true, 1);
        assert_eq!(
            layout.finish().0,
            "{ radio 100 100 9720 9723 0 0 }{ radio 100 120 9720 9723 1 1 }"
        );

        let mut p = vec![0xB1u8, 0, 0];
        p.extend_from_slice(&0u32.to_be_bytes());
        p.extend_from_slice(&0x0051_0002u32.to_be_bytes());
        p.extend_from_slice(&1u32.to_be_bytes()); // the OK button
        p.extend_from_slice(&1u32.to_be_bytes()); // one switch on
        p.extend_from_slice(&1u32.to_be_bytes()); // ...its id
        p.extend_from_slice(&0u32.to_be_bytes()); // no text
        let len = u16::try_from(p.len()).unwrap();
        p[1..3].copy_from_slice(&len.to_be_bytes());

        let got: GumpResponse = decode_packet(&p, version()).unwrap();
        assert_eq!(got.button, 1);
        assert_eq!(got.switches, vec![1], "which radio was set");
    }

    #[test]
    fn a_gump_declares_its_own_length_and_id() {
        let bytes = encode_packet(
            &GumpDisplay {
                serial: 0,
                gump_id: 0x00AD_0001,
                x: 50,
                y: 40,
                layout: "{ resizepic 0 0 5054 200 200 }{ button 10 10 4005 4007 1 0 1 }".to_owned(),
                lines: vec!["Spawn".to_owned()],
            },
            version(),
        );
        assert_eq!(bytes[0], 0xB0);
        let declared = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
        assert_eq!(declared, bytes.len(), "the length word must match the bytes");
        assert_eq!(
            u32::from_be_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]),
            0x00AD_0001,
            "the gump id round-trips"
        );
    }

    #[test]
    fn a_line_rides_as_big_endian_utf16() {
        // "Ok" is two code units; each is a big-endian u16.
        let bytes = encode_packet(
            &GumpDisplay {
                serial: 0,
                gump_id: 1,
                x: 0,
                y: 0,
                layout: "{ page 0 }".to_owned(),
                lines: vec!["Ok".to_owned()],
            },
            version(),
        );
        // Find the text table: it is the tail, `count(2) len(2) 'O'(2) 'k'(2)`.
        let tail = &bytes[bytes.len() - 8..];
        assert_eq!(u16::from_be_bytes([tail[0], tail[1]]), 1, "one line");
        assert_eq!(u16::from_be_bytes([tail[2], tail[3]]), 2, "two chars");
        assert_eq!(u16::from_be_bytes([tail[4], tail[5]]), b'O' as u16);
        assert_eq!(u16::from_be_bytes([tail[6], tail[7]]), b'k' as u16);
    }

    #[test]
    fn a_response_reads_the_button_and_its_fields() {
        // 0xB1: len, serial, gumpId, button=3, 1 switch (id 7), 1 text (id 2, "Hi").
        let mut p = vec![0xB1u8, 0, 0];
        p.extend_from_slice(&0x1234u32.to_be_bytes());
        p.extend_from_slice(&0x00AD_0001u32.to_be_bytes());
        p.extend_from_slice(&3u32.to_be_bytes());
        p.extend_from_slice(&1u32.to_be_bytes()); // switch count
        p.extend_from_slice(&7u32.to_be_bytes()); // switch id
        p.extend_from_slice(&1u32.to_be_bytes()); // text count
        p.extend_from_slice(&2u16.to_be_bytes()); // text id
        p.extend_from_slice(&2u16.to_be_bytes()); // char count
        p.extend_from_slice(&(b'H' as u16).to_be_bytes());
        p.extend_from_slice(&(b'i' as u16).to_be_bytes());
        let len = u16::try_from(p.len()).unwrap();
        p[1..3].copy_from_slice(&len.to_be_bytes());

        let got: GumpResponse = decode_packet(&p, version()).unwrap();
        assert_eq!(got.serial, 0x1234);
        assert_eq!(got.gump_id, 0x00AD_0001);
        assert_eq!(got.button, 3);
        assert_eq!(got.switches, vec![7]);
        assert_eq!(got.text_entries, vec![(2, "Hi".to_owned())]);
    }

    #[test]
    fn a_closed_gump_is_button_zero() {
        let mut p = vec![0xB1u8, 0, 0];
        p.extend_from_slice(&0u32.to_be_bytes()); // serial
        p.extend_from_slice(&5u32.to_be_bytes()); // gumpId
        p.extend_from_slice(&0u32.to_be_bytes()); // button 0 = closed
        p.extend_from_slice(&0u32.to_be_bytes()); // no switches
        p.extend_from_slice(&0u32.to_be_bytes()); // no text
        let len = u16::try_from(p.len()).unwrap();
        p[1..3].copy_from_slice(&len.to_be_bytes());

        let got: GumpResponse = decode_packet(&p, version()).unwrap();
        assert_eq!(got.button, 0, "the close box");
        assert!(got.switches.is_empty());
    }
}
