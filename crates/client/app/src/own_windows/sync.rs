//! Reconciliation between the authoritative view and the client-owned window layer.

use crate::app::App;
use crate::windows::reconcile_own_windows;

impl App {
    /// Open a window for everything the shard has opened and this client has
    /// not placed yet, and drop the windows whose subject is gone.
    ///
    /// Run once a frame rather than when the packet arrived, and idempotent for
    /// that reason: the `0x24` and the `0x88` are folded into the [`WorldView`]
    /// by `client/net`, which knows nothing about screens, so the window is
    /// this end noticing that the view has grown something it has nowhere to
    /// put.
    ///
    /// The drop is the other direction of the same idea: a container removed
    /// from the world — or a mobile destroyed — takes its entry in the view
    /// with it (see `WorldView::apply`'s `Remove` arm), and a window over
    /// nothing must not outlive it.
    pub(crate) fn sync_own_windows(&mut self) {
        let Some(view) = self.world.authoritative.view.as_ref() else {
            // No world, no windows: a map viewer has no shard to have opened
            // one, and anything left over is from a session that has ended.
            self.windows.own_windows.clear();
            // Including local windows, whose state must not leak to a login.
            self.windows.skills = None;
            self.windows.held_skill = None;
            self.windows.status = false;
            return;
        };
        // The state a dialog holds that no packet does, kept in step with the
        // same list: a window the shard has taken away forgets its page, its
        // switches and the finger on it — see `gump::Dialogs::sync`.
        self.windows.dialogs.sync(&view.gumps);
        reconcile_own_windows(
            view,
            &mut self.windows.own_windows,
            &mut self.windows.locally_closed,
            self.windows.skills.is_some(),
            self.windows.status,
        );
    }
}
