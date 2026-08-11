//! What the window system and the mouse have last said: [`Input`].
//!
//! Every field here is written by exactly one `WindowEvent` arm and read back
//! by whichever method needs to know what a key or the pointer is doing right
//! now — none of it is a fact about the world or about a person's own
//! settings, see [`crate::world::WorldState`] and
//! [`crate::graphics::GraphicsSettings`] for those. Pulled out of
//! [`crate::App`] for the same reason those were, even though — unlike
//! them — the fields here are mostly written and read one at a time rather
//! than together: each is its own small fact about the OS/input layer, and
//! grouping them says what they are, not that they change in step.

use std::time::Instant;

use openshard_client_render::gump::GumpPixel;

/// What the window system and the mouse have last said — see the module
/// docs.
pub struct Input {
    /// Whether the right button is down, which is what makes dragging steer:
    /// a heading (or, with Ctrl, a destination) is restated on every cursor
    /// move while it is.
    pub aiming: bool,
    /// Whether Ctrl is held, which is what turns the right-hold from a
    /// heading — the default "run toward the cursor" idiom, no map involved
    /// — into a move order that plans a route with `find_path`. See
    /// `steer.rs`'s module docs.
    pub ctrl_held: bool,
    /// When the last left click landed, or `None` when the one before it
    /// already made a pair.
    ///
    /// The whole of this client's double-click detection, and the reason it
    /// is here rather than asked of the window system: the world's clicks do
    /// not go through egui — see the `MouseInput` arm — and `winit` reports
    /// presses, not gestures. Cleared when a pair fires, which is what stops
    /// three clicks from being two double-clicks; ClassicUO's
    /// `GameController` zeroes its own `lastClickTime` in the same place and
    /// for the same reason.
    pub last_click: Option<Instant>,
    /// Whether the cursor is inside the window at all.
    ///
    /// The other half of "does the world own the mouse", and the half no
    /// egui state can answer: a cursor that has left the window stops
    /// sending positions, so the last one it sent stays true for ever and
    /// the highlight it picked sits on the ground with nobody pointing at
    /// it. `CursorLeft` is the only event that says so.
    pub pointer_inside: bool,
    /// Where the cursor is in *gump* pixels — measured from the surface's
    /// own top left, not the viewport's.
    ///
    /// A second cursor and not the one [`control`](crate::App::control)
    /// keeps, because the two are measured from different corners: the
    /// world's is relative to the viewport, so that the camera zooms about
    /// the picture's centre and not the window's, and an interface has no
    /// viewport at all. Converting one into the other at each use is the
    /// arithmetic the two pixel types exist to stop being done wrong once.
    pub pointer_gump: GumpPixel,
    /// Whether the window has the keyboard.
    ///
    /// Half of [`watched`](crate::App::watched), and true at construction: a
    /// window is mapped focused and winit sends no event to say the thing it
    /// has just done.
    pub focused: bool,
    /// Whether the compositor says the window is entirely covered.
    ///
    /// The other half of [`watched`](crate::App::watched). Its own field
    /// rather than folded into the first, because the two arrive as two
    /// events in an order nothing promises, and one `bool` written by both
    /// would read the second one's answer to the first one's question.
    pub occluded: bool,
}
