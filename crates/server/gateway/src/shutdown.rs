//! The one signal that stops a shard.

use tokio::sync::watch;

/// A request to stop, shared by everything that would otherwise run forever.
///
/// Three loops never end on their own: the accept loop, one read/write pair per
/// connection, and the tick. Each of them has to be told, and each of them has
/// to be told by *the same* thing, or a shard stops in pieces — a tick that has
/// saved and gone while a socket is still being read, or a listener that keeps
/// handing out connections to a world that is no longer there. So this is a
/// value that is cloned and carried, not a signal handler and not a flag some
/// module owns: Ctrl-C in the binary and a handle in a test produce the same
/// stop, on the same paths.
///
/// # Why it lives in the gateway
///
/// Because the door is what must close first, and because this is the lowest
/// crate that everything with a loop to stop already depends on. Putting it
/// under `crates/common` would make it something both ends of the wire agree
/// on, which it is not — a client has no shard to stop.
///
/// # Level-triggered, and that is the whole design
///
/// [`Shutdown::requested`] resolves the moment the stop *has been asked for*,
/// not the moment it is asked for. A task that starts awaiting after the stop
/// went out does not wait for an edge that has already passed — which is the
/// bug this shape exists to make impossible, because a connection accepted one
/// instant before the stop would otherwise be served forever by a shard that is
/// already gone.
///
/// It is also why `requested` may be awaited in a `select!` loop: it builds a
/// fresh waiter each call, and cancelling one loses nothing, because the fact
/// it is waiting for is a state and not an event.
///
/// Dropping a `Shutdown` stops nothing. It is a handle on a shared fact, and
/// letting one go only means this holder will not be the one to ask.
#[derive(Clone, Debug)]
pub struct Shutdown(watch::Sender<bool>);

impl Shutdown {
    /// A stop that has not been asked for.
    pub fn new() -> Self {
        Self(watch::channel(false).0)
    }

    /// Ask everything holding a clone of this to stop.
    ///
    /// Returns immediately: what "stopped" means belongs to each loop that
    /// hears it — the tick's last save takes as long as it takes. Asking twice
    /// is asking once.
    pub fn stop(&self) {
        // `send_replace` rather than `send`, which reports an error when no
        // receiver exists. Nobody listening is not a failure here: a shard with
        // no loops left to stop is a shard that has stopped.
        self.0.send_replace(true);
    }

    /// Whether a stop has been asked for.
    ///
    /// For a caller that has something in hand at this instant and wants to
    /// know whether it is still worth doing. Anything that would otherwise
    /// wait should await [`Shutdown::requested`] instead of polling this.
    pub fn is_stopping(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolve once a stop has been asked for, now or earlier.
    pub async fn requested(&self) {
        let mut watcher = self.0.subscribe();
        // This handle owns the sending half, so the channel cannot close while
        // the call is alive, and `wait_for` returns without waiting when the
        // value already satisfies the predicate.
        let _ = watcher
            .wait_for(|stopping| *stopping)
            .await
            .expect("this handle holds the sender, so the channel cannot close");
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_stop_is_heard() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_stopping(), "nothing has been asked for yet");

        let waiting = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.requested().await }
        });
        shutdown.stop();

        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the waiter was woken")
            .expect("and it did not panic");
        assert!(shutdown.is_stopping());
    }

    #[tokio::test]
    async fn a_late_waiter_does_not_wait_for_an_edge_that_has_passed() {
        // The property the whole type rests on. A connection task spawned in the
        // same breath as the stop starts awaiting *after* it went out, and a
        // notify-style signal would leave that one task serving a client for a
        // world that has already saved and gone.
        let shutdown = Shutdown::new();
        shutdown.stop();

        tokio::time::timeout(Duration::from_secs(5), shutdown.requested())
            .await
            .expect("a stop that has already happened is not something to wait for");
    }

    #[tokio::test]
    async fn any_clone_can_ask_and_every_clone_hears() {
        // Which is what makes one value carried down the call tree the same
        // stop everywhere: the binary holds one, the gate holds one, and each
        // connection task holds one more.
        let shutdown = Shutdown::new();
        let held = shutdown.clone();
        drop(shutdown);

        let heard = held.clone();
        held.stop();

        tokio::time::timeout(Duration::from_secs(5), heard.requested())
            .await
            .expect("the clone heard what another clone asked for");
    }

    #[tokio::test]
    async fn dropping_a_handle_stops_nothing() {
        // Dropping a `Shutdown` is not a stop, and it matters: the gate holds
        // one for the life of the process and a test may well let go of its own
        // the moment it has passed it on.
        let shutdown = Shutdown::new();
        drop(shutdown.clone());

        assert!(!shutdown.is_stopping());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), shutdown.requested())
                .await
                .is_err(),
            "a dropped handle asked for nothing"
        );
    }
}
