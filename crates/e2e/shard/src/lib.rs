//! End-to-end tests, and nothing else.
//!
//! # Why this crate exists at all
//!
//! `server/*` and `client/*` never depend on each other — that rule is what
//! keeps the wire the only thing they agree on, and it is worth keeping. But a
//! test that a *client* can log in to a *shard* needs both ends in one process,
//! and putting it on either side would make that side depend on the other,
//! dev-dependency or not.
//!
//! So it lives outside both. This crate is the only place in the workspace
//! allowed to name both ends, it ships no code — `tests/` is the whole point of
//! it — and nothing depends on it.
//!
//! # What belongs here
//!
//! Only what cannot be tested on one side alone. The gateway's framing, the
//! client's login machine and the world's tick all have their own tests, and
//! those are better tests: pure state machines, no ports, no timing. What is
//! left for this crate is the seam — that the two ends, each correct, actually
//! agree.
