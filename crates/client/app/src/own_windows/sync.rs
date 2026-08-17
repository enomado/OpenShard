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
            // The local windows included, and that is now the *same* line
            // rather than a line each: what a skill sheet or a status frame
            // knows is a field of its pane, and the clear drops both windows
            // and both panes. There used to be a `status = false` under this,
            // and before step 2 a `skills = None` beside it.
            self.windows.own_windows.clear();
            self.windows.keyboard = None;
            self.windows.stack_pass = None;
            // The press no window holds, and the modal that may be standing
            // over it: both are the manager's, so the clear that drops every
            // window has to name them. A press a *pane* was holding went with
            // its window in the line above.
            self.windows.world_press = None;
            self.windows.prompt = None;
            return;
        };
        reconcile_own_windows(
            view,
            &mut self.windows.own_windows,
            &mut self.windows.locally_closed,
        );
        // The keyboard belongs to a window only while that window is open, and
        // this is the same predicate every reader of the field already applies —
        // written back so that a dialog the shard has taken away does not leave
        // a name behind. It replaces `Dialogs::sync`'s three lines of the same
        // idea: what a dialog held is a field of its pane now, and the `retain`
        // above drops both together.
        self.windows.keyboard = self.keyboard_window();
        self.advance_stack_pass();
    }
}
