//! The walk sequence: the handshake under every step a client takes.

use openshard_protocol::world::RawStepSequence;

/// Tracks where a client is in the walk sequence.
///
/// # What the sequence is for
///
/// Every `0x02` walk request carries a byte. The server echoes it in the `0x22`
/// ack, and the client uses that to match the ack to the step it asked for —
/// several can be in flight at once over a slow link. A `0x21` reject snaps the
/// client back and resets the count.
///
/// # Sphere's rules, and they are not obvious
///
/// From `PacketMovementReq::onReceive`:
///
/// - **Zero means "fresh".** A connection starts expecting 0. If a client's
///   first step is anything else, it is out of step with the server and the
///   walk is refused.
/// - **255 wraps to 1, not 0.** `if (sequence == UINT8_MAX) sequence = 0;` then
///   `++sequence`. Zero is skipped on wrap because zero is reserved for a fresh
///   connection — a wrap through it would look like a client that had just
///   reconnected.
/// - **A reject resets to zero.** Both ends: the server expects 0 next, and the
///   client sends 0 next.
///
/// # What is deliberately not checked
///
/// That the sequence *matches* what the server expects. Sphere checks only the
/// fresh-connection case and lets everything else through, and this does too.
/// Being stricter would be easy and wrong: clients drift out of step for
/// perfectly ordinary reasons — a dropped ack, a reject in flight — and a
/// server that refuses on mismatch turns a hiccup into a client that cannot
/// walk until it reconnects. The sequence is an echo tag, not a nonce.
///
/// ```
/// use openshard_movement::WalkSequence;
/// use openshard_protocol::world::RawStepSequence;
///
/// let mut sequence = WalkSequence::new();
///
/// // A fresh connection must open with zero.
/// assert!(sequence.accept(RawStepSequence(0)).is_ok());
/// assert!(sequence.accept(RawStepSequence(1)).is_ok());
///
/// // A reject puts both ends back to zero.
/// sequence.reset();
/// assert!(sequence.accept(RawStepSequence(5)).is_err(), "the client must restart at zero");
/// assert!(sequence.accept(RawStepSequence(0)).is_ok());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WalkSequence {
    /// What the server expects next. Zero means "fresh, or just reset".
    expected: u8,
}

/// A walk request came in out of step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutOfSequence {
    /// What the client sent.
    pub got: RawStepSequence,
}

impl WalkSequence {
    /// A sequence expecting a client's first step.
    pub const fn new() -> Self {
        Self { expected: 0 }
    }

    /// What the server expects next.
    pub const fn expected(self) -> u8 {
        self.expected
    }

    /// Whether the next step must be a zero.
    pub const fn is_fresh(self) -> bool {
        self.expected == 0
    }

    /// Take a walk request's sequence byte, advancing on success.
    ///
    /// Fails only when a fresh connection opens with something other than zero.
    /// The caller should answer `0x21` and call [`WalkSequence::reset`].
    pub fn accept(&mut self, sequence: RawStepSequence) -> Result<(), OutOfSequence> {
        if self.expected == 0 && sequence.0 != 0 {
            return Err(OutOfSequence { got: sequence });
        }
        // 255 wraps to 1: zero is reserved for a fresh connection, so a wrap
        // through it would be indistinguishable from a reconnect.
        self.expected = if sequence.0 == u8::MAX { 1 } else { sequence.0 + 1 };
        Ok(())
    }

    /// Go back to fresh, so the next step may carry any sequence.
    ///
    /// Two things ask for this and they are the same thing from the client's
    /// side: a refused step (it resets its own count when it sees `0x21`) and a
    /// jump it did not predict — a teleport or a facet change, after which it
    /// starts counting from zero again. Either way both ends have to agree that
    /// the next step is a zero, or the server rejects a step that was correct
    /// and the client asks for a resync it will not get.
    pub fn reset(&mut self) {
        self.expected = 0;
    }
}

/// The other side of [`WalkSequence`]: what a *client* puts in its next `0x02`.
///
/// The two halves of one rule, which is why they live in the same file. Every
/// sentence in [`WalkSequence`]'s docs about what the server will accept is a
/// sentence about what this has to emit — open at zero, skip zero on the wrap,
/// go back to zero when a `0x21` arrives — and a copy of that rule written
/// somewhere else would drift silently: the drift only shows up on the 256th
/// step of a session, or on the step after a refusal.
///
/// ```
/// use openshard_movement::{StepCounter, WalkSequence};
/// use openshard_protocol::world::RawStepSequence;
///
/// let mut counter = StepCounter::new();
/// let mut server = WalkSequence::new();
///
/// // A fresh connection opens at zero, which is the one thing the server insists on.
/// assert_eq!(counter.take(), 0);
/// assert_eq!(counter.take(), 1);
///
/// // A `0x21` puts both ends back to the start.
/// counter.reset();
/// server.reset();
/// assert!(server.accept(RawStepSequence(counter.take())).is_ok());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StepCounter {
    /// What the next `0x02` will carry. Zero means "fresh, or just reset".
    next: u8,
}

impl StepCounter {
    /// A counter whose first step is a client's first step.
    pub const fn new() -> Self {
        Self { next: 0 }
    }

    /// What the next `0x02` will carry, without spending it.
    pub const fn peek(self) -> u8 {
        self.next
    }

    /// Take the sequence byte for one `0x02`, advancing past it.
    pub fn take(&mut self) -> u8 {
        let sending = self.next;
        // The wrap the server also makes: 255 goes to 1, because zero says
        // "fresh connection" and a wrap through it would ask the server to
        // treat a walk in progress as a reconnect.
        self.next = if sending == u8::MAX { 1 } else { sending + 1 };
        sending
    }

    /// Go back to zero, because a `0x21` arrived or the world moved us.
    ///
    /// The mirror of [`WalkSequence::reset`], and it has to happen on the same
    /// events: a refused step, a teleport, a facet change.
    pub fn reset(&mut self) {
        self.next = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(value: u8) -> RawStepSequence {
        RawStepSequence(value)
    }

    #[test]
    fn a_fresh_connection_expects_zero() {
        let sequence = WalkSequence::new();
        assert_eq!(sequence.expected(), 0);
        assert!(sequence.is_fresh());
    }

    #[test]
    fn a_fresh_connection_that_does_not_open_with_zero_is_refused() {
        // Sphere: `if (net->m_sequence == 0 && sequence != 0) direction = DIR_QTY`
        // — an invalid direction, to reject on purpose.
        for opening in 1..=u8::MAX {
            let mut sequence = WalkSequence::new();
            assert_eq!(
                sequence.accept(raw(opening)),
                Err(OutOfSequence { got: raw(opening) }),
                "a fresh connection must not accept {opening}"
            );
            assert!(sequence.is_fresh(), "a refusal leaves it fresh");
        }
    }

    #[test]
    fn steps_advance_one_at_a_time() {
        let mut sequence = WalkSequence::new();
        for step in 0..100u8 {
            assert!(sequence.accept(raw(step)).is_ok());
            assert_eq!(sequence.expected(), step + 1);
        }
    }

    #[test]
    fn two_hundred_and_fifty_five_wraps_to_one_not_zero() {
        // The rule that is easy to get wrong, and wrong invisibly: a naive
        // `wrapping_add(1)` gives 0, which reads as a fresh connection. The next
        // step would then have to be 0 as well, and a client that dutifully sent
        // 1 would be refused — walking would break once every 256 steps.
        let mut sequence = WalkSequence::new();
        sequence.accept(raw(0)).unwrap();
        sequence.accept(raw(254)).unwrap();
        assert_eq!(sequence.expected(), 255);

        sequence.accept(raw(255)).unwrap();
        assert_eq!(sequence.expected(), 1, "zero is skipped on the wrap");
        assert!(!sequence.is_fresh(), "a wrap is not a reconnect");
    }

    #[test]
    fn accepting_the_wrap_does_not_overflow() {
        // `sequence + 1` on a u8 of 255 would panic in debug and wrap in
        // release. The wrap rule is what stops it, but it has to actually run.
        let mut sequence = WalkSequence::new();
        sequence.accept(raw(0)).unwrap();
        for _ in 0..1000 {
            sequence.accept(raw(255)).unwrap();
            assert_eq!(sequence.expected(), 1);
        }
    }

    #[test]
    fn a_reject_puts_the_sequence_back_to_zero() {
        let mut sequence = WalkSequence::new();
        sequence.accept(raw(0)).unwrap();
        sequence.accept(raw(1)).unwrap();
        assert!(!sequence.is_fresh());

        sequence.reset();
        assert!(sequence.is_fresh(), "the client resets too, on seeing 0x21");
        assert!(sequence.accept(raw(0)).is_ok());
    }

    #[test]
    fn after_a_reject_the_client_must_restart_at_zero() {
        let mut sequence = WalkSequence::new();
        sequence.accept(raw(0)).unwrap();
        sequence.accept(raw(1)).unwrap();
        sequence.reset();

        assert_eq!(sequence.accept(raw(2)), Err(OutOfSequence { got: raw(2) }));
        assert!(sequence.accept(raw(0)).is_ok());
    }

    #[test]
    fn a_mismatched_sequence_mid_walk_is_accepted() {
        // Deliberate, and matching Sphere. A client drifts out of step for
        // ordinary reasons — a dropped ack, a reject still in flight — and a
        // server that refused on mismatch would turn a hiccup into a client that
        // cannot walk until it reconnects. The byte is an echo tag, not a nonce.
        let mut sequence = WalkSequence::new();
        sequence.accept(raw(0)).unwrap();
        assert_eq!(sequence.expected(), 1);

        assert!(sequence.accept(raw(200)).is_ok(), "not our place to refuse");
        assert_eq!(sequence.expected(), 201, "follow the client");
    }

    #[test]
    fn a_full_lap_never_returns_to_fresh() {
        // Walking 300 steps must not, at any point, look like a reconnect — and
        // the client half is driven by the real `StepCounter` rather than a copy
        // of its rule written here, so the two cannot drift apart without this
        // failing.
        let mut sequence = WalkSequence::new();
        let mut counter = StepCounter::new();
        for step in 0..300 {
            let sending = counter.take();
            assert!(
                sequence.accept(raw(sending)).is_ok(),
                "step {step} with sequence {sending}"
            );
            assert!(
                !sequence.is_fresh(),
                "step {step} made a live connection look fresh"
            );
        }
    }

    #[test]
    fn a_counter_opens_at_zero_and_skips_it_on_the_wrap() {
        let mut counter = StepCounter::new();
        assert_eq!(counter.peek(), 0, "a fresh connection must open at zero");
        assert_eq!(counter.take(), 0);
        assert_eq!(counter.take(), 1);

        // Straight to the wrap.
        let mut counter = StepCounter::new();
        for _ in 0..=u8::MAX {
            counter.take();
        }
        assert_eq!(counter.peek(), 1, "255 goes to 1, not back to zero");
    }

    #[test]
    fn a_counter_that_reset_is_accepted_by_a_sequence_that_reset() {
        // The step after a `0x21`. Both ends reset on it, and a client that
        // carried on counting would be refused for the rest of the session:
        // the server is fresh, and a fresh server takes nothing but zero.
        let mut sequence = WalkSequence::new();
        let mut counter = StepCounter::new();
        for _ in 0..5 {
            sequence.accept(raw(counter.take())).unwrap();
        }

        sequence.reset();
        counter.reset();
        assert!(sequence.accept(raw(counter.take())).is_ok());
    }
}
