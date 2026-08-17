//! Input handling specific to paperdoll controls.

use std::time::Instant;

use openshard_client_render::{gump as gump_art, paperdoll};
use openshard_protocol::mobile::Equipment;
use openshard_protocol::wire::Layer;

use crate::app::App;
use crate::scroll_pairs;
use crate::windows::{Drawn, WindowSubject};

impl App {
    /// Return the interactive paperdoll picture under the cursor, if any.
    pub(crate) fn doll_button_under_pointer(&self, subject: WindowSubject) -> Option<paperdoll::DollButton> {
        let Some(Drawn::Paperdoll(doll)) = self.drawn(subject) else {
            return None;
        };
        let index = gump_art::pick(
            &doll.pictures,
            self.input.pointer_gump,
            &self.resources.gump_atlas,
        )?;
        doll.hits.get(&index).copied()
    }

    /// Send the request named by a paperdoll control.
    pub(crate) fn doll_clicked(&mut self, subject: WindowSubject, button: paperdoll::DollButton) {
        let WindowSubject::Paperdoll(mobile) = subject else {
            return;
        };
        let Some(view) = self.world.authoritative.view.as_ref() else {
            return;
        };
        let own = view.player.serial == mobile;
        let war = view.player.war;
        let equipment: &[Equipment] = match own {
            true => &view.player.equipment,
            false => view
                .mobiles
                .get(&mobile)
                .map_or(&[] as &[Equipment], |mobile| mobile.equipment.as_slice()),
        };
        let backpack = equipment
            .iter()
            .find(|item| item.layer == Layer::BACKPACK)
            .map(|item| item.serial);
        let paired = match button {
            paperdoll::DollButton::Profile
            | paperdoll::DollButton::Party
            | paperdoll::DollButton::Virtue
            | paperdoll::DollButton::Backpack => self.scroll_paired(subject, button),
            _ => false,
        };
        let Some(link) = self.world.shard.link() else {
            return;
        };
        let mut opened_skills = false;
        let mut opened_status = false;
        match button {
            paperdoll::DollButton::WarMode => link.war_mode(!war),
            paperdoll::DollButton::LogOut => link.log_out(),
            paperdoll::DollButton::Quests => link.quest_log(),
            paperdoll::DollButton::Guild => link.guild_menu(),
            // Status packets describe this connection's player, so a stranger's
            // Status control must not open a misleading local window.
            paperdoll::DollButton::Status if own => {
                link.status(mobile);
                opened_status = true;
            }
            // Window visibility is local intent; 0x3A merely refreshes data.
            paperdoll::DollButton::Skills if own => {
                link.skills(mobile);
                opened_skills = true;
            }
            paperdoll::DollButton::Virtue if paired => link.virtue(mobile),
            paperdoll::DollButton::Backpack if paired => {
                if let Some(serial) = backpack {
                    link.use_object(serial);
                }
            }
            _ => {}
        }
        if opened_skills {
            // The same door `Effect::Open(LocalWindow::Skills)` goes through,
            // and it will *be* that effect once the paperdoll is a pane of its
            // own (step 5). Idempotent, so a second press leaves the sheet's
            // scroll and shut headings where the player left them.
            crate::windows::open_local_window(&mut self.windows.own_windows, WindowSubject::Skills);
        }
        if opened_status {
            self.windows.status = true;
        }
    }

    /// Whether this was the second click on the same paperdoll scroll.
    pub(crate) fn scroll_paired(&mut self, subject: WindowSubject, button: paperdoll::DollButton) -> bool {
        let now = Instant::now();
        let paired = scroll_pairs(self.windows.last_scroll, now, subject, button);
        self.windows.last_scroll = (!paired).then_some((now, subject, button));
        paired
    }
}
