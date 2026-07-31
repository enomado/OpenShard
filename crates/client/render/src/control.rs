//! What the mouse, the wheel and the lock key do to a [`Camera`].
//!
//! The arithmetic between an input event and an eye: a drag's fractional
//! remainder, a wheel notch's anchor, the device's refusal to allocate the image
//! a zoom asks for, and whether the eye is the body's or the mouse's. All of it
//! used to live in `client/app`, where it could not be reached from a test
//! because the thing that owned it also owned a window, a GPU and a `Map`. None
//! of it needs any of the three.
//!
//! What is deliberately *not* here: any decision about what happens next. This
//! answers whether something moved and reports what a device refused; asking for
//! a redraw and printing the refusal are the caller's, because a renderer with a
//! stderr is a renderer that cannot be run twice in one process.

use openshard_protocol::world::Point;

use crate::camera::{Camera, WorldPixel, Zoom};

/// Whether the camera is tied to the body or the mouse.
///
/// It lives beside the camera and not inside it: the camera does not know what a
/// player is, and giving it one would put `client/net` inside `client/render`.
/// What it *is* is a rule about who may move the eye, which is exactly what
/// [`Control`] arbitrates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Follow {
    /// The eye is the body's, and the server moves it.
    Body,
    /// The eye is the mouse's, and the body may walk off screen.
    Free,
}

/// What the mouse is doing to the camera.
///
/// The fraction is the reason this is a struct and not a flag. At zoom 2 a
/// one-pixel drag is half a world pixel, and an eye that carried the fraction
/// would put every sprite on a half-texel boundary for half of all camera
/// positions — the same class of defect as the half-texel inset the atlases
/// apply, spread across the whole frame instead of one edge. So the remainder is
/// accumulated here, in the input handler, and only whole world pixels reach
/// [`Camera::look_at_pixel`].
#[derive(Clone, Copy, Default, Debug)]
struct Drag {
    /// Where the cursor was last seen, in physical pixels from the viewport's
    /// top-left. Needed by the wheel, which is told a delta and not a position.
    cursor: (i32, i32),
    /// Whether the middle button is down.
    panning: bool,
    /// Viewport pixels dragged and not yet spent, numerator over the zoom's.
    remainder: (i32, i32),
}

/// A zoom the device would not allocate the offscreen image for.
///
/// Returned rather than printed, and it carries the numbers because the message
/// worth writing names all of them: a silently truncated target draws a smaller
/// world into a larger rect, which looks exactly like a bug in the projection, so
/// whoever hits this has to be able to say what happened and why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TooLarge {
    /// The zoom that was asked for and refused.
    pub wanted: Zoom,
    /// The image it would have wanted, in pixels.
    pub width: u32,
    /// Likewise.
    pub height: u32,
    /// What this device allows in either dimension.
    pub max: u32,
    /// The zoom in force now — the old one after a refusal, a tighter one after
    /// [`Control::fit_to_device`] stepped in.
    pub settled: Zoom,
}

/// A camera, who is allowed to move it, and what the mouse has not yet spent.
#[derive(Clone, Copy, Debug)]
pub struct Control {
    camera: Camera,
    follow: Follow,
    drag: Drag,
    /// How far the ladder may be walked down before the offscreen texture is
    /// larger than the GPU allows.
    ///
    /// WebGL2 guarantees only 2048 in each dimension and a 1080p window at `1/2`
    /// wants more, so the ladder has a runtime end that depends on the device.
    max_texture: u32,
}

impl Control {
    /// A locked camera on this device.
    pub fn new(camera: Camera, max_texture: u32) -> Self {
        Self {
            camera,
            follow: Follow::Body,
            drag: Drag::default(),
            max_texture,
        }
    }

    /// The camera, for everything that draws from it.
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// Whether the eye is the body's or the mouse's.
    pub fn follow(&self) -> Follow {
        self.follow
    }

    /// Whether the middle button is held.
    pub fn panning(&self) -> bool {
        self.drag.panning
    }

    /// Where the cursor was last seen, in viewport pixels.
    pub fn cursor(&self) -> (i32, i32) {
        self.drag.cursor
    }

    /// The viewport changed size — a window resize, or a panel that grew.
    ///
    /// Zero is a minimised window rather than an error, and a texture of zero
    /// width is, so both are floored at one.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.camera.width = width.max(1);
        self.camera.height = height.max(1);
    }

    /// What this device will allocate, once there is a device to ask.
    pub fn set_max_texture(&mut self, max: u32) {
        self.max_texture = max;
    }

    /// Put the eye back on the body and lock it there.
    ///
    /// Snaps rather than eases. Easing wants a per-frame clock over a mobile that
    /// survives between frames, which is what the "everything stands" backlog
    /// item in `docs/client.md` is waiting for; both should be built once.
    pub fn relock(&mut self, at: Point) {
        self.follow = Follow::Body;
        self.camera.look_at(at);
    }

    /// Let the body walk off screen: the eye stops following it.
    pub fn unlock(&mut self) {
        self.follow = Follow::Free;
    }

    /// The body moved. The eye follows it only while locked.
    ///
    /// The one rule the lock exists for, and the reason it is a method rather
    /// than an `if` at each call site: `App::step` and `App::entered` are two
    /// writers of the same eye, and a third would forget.
    pub fn follow_body(&mut self, at: Point) {
        if self.follow == Follow::Body {
            self.camera.look_at(at);
        }
    }

    /// The middle button went down or came up.
    ///
    /// Whatever a previous drag was saving up is not owed to this one, so the
    /// remainder is dropped on either edge.
    pub fn set_panning(&mut self, down: bool) {
        self.drag.panning = down;
        self.drag.remainder = (0, 0);
    }

    /// The cursor moved to a viewport pixel, panning if the button is down.
    ///
    /// Answers whether the eye actually moved: at zoom 4 most one-pixel drags
    /// move nothing, and asking for a redraw for each of them would be a frame
    /// per mouse report showing the same picture.
    pub fn cursor_moved(&mut self, x: i32, y: i32) -> bool {
        let (dx, dy) = (x - self.drag.cursor.0, y - self.drag.cursor.1);
        self.drag.cursor = (x, y);
        self.drag.panning && self.pan(dx, dy)
    }

    /// Move the eye by a drag, in viewport pixels, spending whole world pixels.
    ///
    /// Unlocks: a hand on the camera outranks the server, until `Home` says
    /// otherwise. Answers whether the eye moved.
    pub fn pan(&mut self, dx: i32, dy: i32) -> bool {
        self.follow = Follow::Free;
        let num = self.camera.zoom().numerator() as i32;
        let den = self.camera.zoom().denominator() as i32;
        // Viewport pixels times the denominator, kept as a numerator over `num`:
        // the fraction stays here rather than in the eye.
        let owed_x = self.drag.remainder.0 + dx * den;
        let owed_y = self.drag.remainder.1 + dy * den;
        // Towards zero, so the remainder keeps the sign of the debt and a drag
        // back and forth ends where it started.
        let (whole_x, whole_y) = (owed_x / num, owed_y / num);
        self.drag.remainder = (owed_x - whole_x * num, owed_y - whole_y * num);
        if whole_x == 0 && whole_y == 0 {
            return false;
        }
        let eye = self.camera.eye();
        // The world follows the cursor, so the eye goes the other way.
        self.camera.look_at_pixel(WorldPixel {
            x: eye.x - whole_x,
            y: eye.y - whole_y,
        });
        true
    }

    /// One notch of the wheel.
    ///
    /// `Ok(true)` when something changed, `Ok(false)` at either end of the
    /// ladder, and `Err` when the device would not hold the image the new zoom
    /// wants — refused rather than truncated, because a smaller world drawn into
    /// a larger rect looks like a projection bug and reads as one.
    pub fn zoom(&mut self, inwards: bool) -> Result<bool, TooLarge> {
        let wanted = if inwards {
            self.camera.zoom().scale_up()
        } else {
            self.camera.zoom().scale_down()
        };
        if wanted == self.camera.zoom() {
            return Ok(false);
        }
        // Locked to the body, the zoom is about the middle: an eye held to the
        // cursor would be moved here and moved back by the next `WorldView`,
        // which is a fight rather than a camera. Unlocked, it is about the
        // cursor, which is the difference between a camera that feels placed and
        // one that feels shoved.
        let (anchor_x, anchor_y) = match self.follow {
            Follow::Body => (self.camera.width as i32 / 2, self.camera.height as i32 / 2),
            Follow::Free => self.drag.cursor,
        };
        let mut probe = self.camera;
        probe.zoom_about(anchor_x, anchor_y, wanted);
        if let Some(refusal) = self.refuses(&probe) {
            return Err(refusal);
        }
        self.camera = probe;
        // The fraction a drag was saving up belongs to the old zoom.
        self.drag.remainder = (0, 0);
        Ok(true)
    }

    /// Step the zoom back in until the world texture fits this device.
    ///
    /// [`Control::zoom`] refuses a step that would not fit, and that is not the
    /// whole of it: the offscreen image is `viewport / zoom`, so *growing the
    /// window* at a zoom that fitted asks for a texture that does not. Nobody
    /// zooms in that path, so the check has to live where the size is used and
    /// not only where the zoom changes — without this, dragging a window wider at
    /// `1/2` is a validation error from the texture rather than a camera that
    /// stops zooming out.
    ///
    /// Answers with what did not fit, once, whatever the number of rungs it had
    /// to climb: the caller wants to say it, not to say it repeatedly.
    pub fn fit_to_device(&mut self) -> Option<TooLarge> {
        let mut refusal: Option<TooLarge> = None;
        while let Some(too_large) = self.refuses(&self.camera) {
            let tighter = self.camera.zoom().scale_up();
            if tighter == self.camera.zoom() {
                // The top of the ladder and still too large, which means a
                // viewport bigger than the device's limit — nothing a zoom can
                // answer. Reported rather than looped over.
                break;
            }
            refusal.get_or_insert(too_large);
            // About the middle: this is not somebody's wheel, it is the device
            // saying no, and there is no cursor that asked for it.
            let (cx, cy) = (self.camera.width as i32 / 2, self.camera.height as i32 / 2);
            self.camera.zoom_about(cx, cy, tighter);
        }
        // The zoom in force is the one this settled on, not the one the first
        // rung refused.
        refusal.map(|first| TooLarge {
            settled: self.camera.zoom(),
            ..first
        })
    }

    /// Whether the offscreen image this camera wants is larger than the device
    /// allows, and by how much.
    fn refuses(&self, camera: &Camera) -> Option<TooLarge> {
        let (width, height) = (camera.render_width(), camera.render_height());
        if width <= self.max_texture && height <= self.max_texture {
            return None;
        }
        Some(TooLarge {
            wanted: camera.zoom(),
            width,
            height,
            max: self.max_texture,
            // Overwritten by `fit_to_device`, which knows where it stopped. On a
            // refusal from `zoom` the camera did not move, so this is already it.
            settled: self.camera.zoom(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device that will hold anything, so a test about panning is not also a
    /// test about texture limits.
    const HUGE: u32 = 1 << 20;

    fn control() -> Control {
        Control::new(Camera::new(Point::new(100, 100, 0), 800, 600), HUGE)
    }

    /// Zooming out four rungs from `1:1` and back in four lands on the same
    /// zoom, which is the ladder's whole promise.
    #[test]
    fn the_ladder_ends_rather_than_wrapping() {
        let mut control = control();
        let mut out = 0;
        while control.zoom(false).unwrap() {
            out += 1;
            assert!(out < 100, "the ladder has no bottom");
        }
        assert_eq!(control.camera().zoom().to_string(), "1/2x");
        let mut back = 0;
        while control.zoom(true).unwrap() {
            back += 1;
            assert!(back < 100, "the ladder has no top");
        }
        assert_eq!(control.camera().zoom().to_string(), "4x");
        assert_eq!((out, back), (3, 8), "nine rungs, with 1:1 the fourth");
    }

    /// The remainder is the point of `Drag`, and this is what it buys: at zoom 4
    /// a single viewport pixel is a quarter of a world pixel, so three drags
    /// move nothing and the fourth moves one.
    #[test]
    fn a_drag_finer_than_a_world_pixel_accumulates() {
        let mut control = control();
        for _ in 0..5 {
            assert!(control.zoom(true).unwrap());
        }
        assert_eq!(control.camera().zoom().to_string(), "4x");
        let before = control.camera().eye();
        assert!(!control.pan(1, 0), "a quarter pixel is not a pixel");
        assert!(!control.pan(1, 0));
        assert!(!control.pan(1, 0));
        assert!(control.pan(1, 0), "four quarters are");
        assert_eq!(control.camera().eye().x, before.x - 1);
    }

    /// And the direction of the rounding: a drag out and back ends where it
    /// started rather than a pixel to one side, which is what "towards zero"
    /// with a signed remainder is for.
    #[test]
    fn a_drag_out_and_back_ends_where_it_started() {
        let mut control = control();
        assert!(control.zoom(true).unwrap());
        let before = control.camera().eye();
        for _ in 0..7 {
            control.pan(1, 1);
        }
        for _ in 0..7 {
            control.pan(-1, -1);
        }
        assert_eq!(control.camera().eye(), before);
    }

    /// Three quarters of a world pixel owed at 4x, and the zoom that leaves.
    ///
    /// The remainder belongs to the zoom that produced it: three quarters is not
    /// three thirds, and a remainder carried across would pay for a pixel the
    /// drag never asked for — the next single-pixel drag at 3x would move the eye
    /// instead of saving up like the two after it.
    #[test]
    fn a_zoom_forgets_what_a_drag_was_saving_up() {
        let mut control = control();
        for _ in 0..5 {
            assert!(control.zoom(true).unwrap());
        }
        for _ in 0..3 {
            assert!(!control.pan(1, 0), "three quarters owed");
        }
        assert!(control.zoom(false).unwrap());
        assert_eq!(control.camera().zoom().to_string(), "3x");
        assert!(!control.pan(1, 0), "a third, not a third plus three quarters");
        assert!(!control.pan(1, 0));
        let before = control.camera().eye();
        assert!(control.pan(1, 0), "three thirds");
        assert_eq!(control.camera().eye().x, before.x - 1);
    }

    /// A press or a release does the same, for the same reason: the fraction one
    /// drag left over is not owed to the next one.
    #[test]
    fn a_new_drag_does_not_inherit_a_remainder() {
        let mut control = control();
        for _ in 0..5 {
            assert!(control.zoom(true).unwrap());
        }
        for _ in 0..3 {
            assert!(!control.pan(1, 0));
        }
        control.set_panning(true);
        for _ in 0..3 {
            assert!(
                !control.pan(1, 0),
                "the owed three quarters went with the old drag"
            );
        }
        let before = control.camera().eye();
        assert!(control.pan(1, 0));
        assert_eq!(control.camera().eye().x, before.x - 1);
    }

    /// `cursor_moved` pans only while the button is down, and it pans by the
    /// delta rather than by the position — the first cursor report after the
    /// window opens is a jump from wherever the origin happens to be.
    #[test]
    fn the_cursor_moves_the_eye_only_while_the_button_is_down() {
        let mut control = control();
        assert!(!control.cursor_moved(400, 300), "no button, no pan");
        assert_eq!(control.cursor(), (400, 300));
        let before = control.camera().eye();
        control.set_panning(true);
        assert!(control.cursor_moved(410, 300));
        assert_eq!(control.camera().eye().x, before.x - 10);
    }

    /// The lock is the rule two writers share: while it holds, the body moves
    /// the eye; once a drag has unlocked it, the body is free to walk off screen.
    #[test]
    fn the_body_moves_the_eye_only_while_locked() {
        let mut control = control();
        assert_eq!(control.follow(), Follow::Body);
        control.follow_body(Point::new(200, 200, 0));
        assert_eq!(
            control.camera().eye(),
            crate::camera::project(Point::new(200, 200, 0))
        );

        control.pan(30, 30);
        assert_eq!(control.follow(), Follow::Free, "a hand on the camera unlocks it");
        let free = control.camera().eye();
        control.follow_body(Point::new(300, 300, 0));
        assert_eq!(control.camera().eye(), free, "the body no longer drags the eye");

        control.relock(Point::new(300, 300, 0));
        assert_eq!(control.follow(), Follow::Body);
        assert_eq!(
            control.camera().eye(),
            crate::camera::project(Point::new(300, 300, 0))
        );
    }

    /// Locked, the wheel zooms about the middle rather than the cursor: an eye
    /// pinned to the cursor would be moved by the zoom and moved straight back by
    /// the next `WorldView`, which is a fight rather than a camera.
    #[test]
    fn a_locked_zoom_is_about_the_middle() {
        let mut locked = control();
        locked.set_panning(false);
        locked.cursor_moved(0, 0);
        let eye = locked.camera().eye();
        assert!(locked.zoom(true).unwrap());
        assert_eq!(locked.camera().eye(), eye, "the middle does not move");

        // The same notch with the same cursor, unlocked, holds the *cursor*
        // still instead — which moves the eye.
        let mut free = control();
        free.unlock();
        free.cursor_moved(0, 0);
        let under_cursor = free.camera().pick(0, 0);
        assert!(free.zoom(true).unwrap());
        assert_eq!(free.camera().pick(0, 0), under_cursor);
        assert_ne!(free.camera().eye(), eye);
    }

    /// The device's refusal is a value and not a truncation: the zoom does not
    /// change, and the numbers needed to explain it come back.
    #[test]
    fn a_device_that_cannot_hold_the_image_refuses_the_zoom() {
        // 800 wide at 3/4 wants 1068, which this device will not hold.
        let mut control = Control::new(Camera::new(Point::new(100, 100, 0), 800, 600), 1024);
        let before = *control.camera();
        let refusal = control.zoom(false).unwrap_err();
        assert_eq!(*control.camera(), before, "a refusal moves nothing");
        assert_eq!(refusal.max, 1024);
        assert_eq!(refusal.width, 1068);
        assert_eq!(refusal.settled, before.zoom());
        assert_eq!(refusal.wanted.to_string(), "3/4x");
    }

    /// And the path no wheel takes: a window dragged wider at a zoom that fitted
    /// asks for a texture that does not, so the fit is checked where the size is
    /// used. Without this the next frame is a validation error.
    #[test]
    fn growing_the_viewport_steps_the_zoom_back_in() {
        let mut control = Control::new(Camera::new(Point::new(100, 100, 0), 400, 300), 1024);
        assert!(control.zoom(false).unwrap(), "536 fits in 1024");
        assert_eq!(control.camera().zoom().to_string(), "3/4x");
        assert_eq!(control.fit_to_device(), None);

        control.resize(1000, 300);
        let refusal = control.fit_to_device().expect("1336 is past 1024");
        assert_eq!(refusal.width, 1336);
        assert_eq!(refusal.settled, control.camera().zoom());
        assert_eq!(control.camera().zoom(), Zoom::ONE);
        assert!(control.camera().render_width() <= 1024);
        assert_eq!(control.fit_to_device(), None, "once it fits it stays fitted");
    }

    /// Several rungs at once, and the answer is still one refusal — the caller
    /// prints it, and a caller that printed one per rung would be shouting.
    #[test]
    fn a_fit_that_climbs_several_rungs_reports_once() {
        let mut control = Control::new(Camera::new(Point::new(100, 100, 0), 400, 300), 4096);
        assert!(control.zoom(false).unwrap());
        control.set_max_texture(512);
        control.resize(1200, 900);
        let refusal = control.fit_to_device().expect("1200 at 3/4 is 1600");
        assert_eq!(refusal.wanted.to_string(), "3/4x", "the rung that first refused");
        assert_eq!(refusal.settled, control.camera().zoom());
        assert!(control.camera().render_width() <= 512);
    }

    /// A viewport larger than the device's limit is not a zoom problem, and the
    /// loop that steps in has to stop rather than spin at the top of the ladder.
    #[test]
    fn a_viewport_past_the_limit_stops_at_the_top_of_the_ladder() {
        let mut control = Control::new(Camera::new(Point::new(100, 100, 0), 8192, 8192), 1024);
        let refusal = control.fit_to_device().expect("nothing here fits");
        assert_eq!(refusal.settled.to_string(), "4x", "climbed as far as it could");
        assert_eq!(control.camera().zoom().to_string(), "4x");
    }

    /// The viewport is floored rather than allowed to be zero: a minimised
    /// window is not an error and a texture of zero width is.
    #[test]
    fn a_minimised_window_is_one_pixel_wide() {
        let mut control = control();
        control.resize(0, 0);
        assert_eq!(control.camera().width, 1);
        assert_eq!(control.camera().render_width(), 1);
    }
}
