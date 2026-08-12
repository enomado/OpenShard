//! Input handling specific to the local skills window.

use openshard_client_render::skills;
use openshard_protocol::skill::SkillLock;
use openshard_protocol::wire::RawSkillId;

use crate::app::App;
use crate::windows::{Drawn, WindowSubject};

impl App {
    /// Which of the skill window's last-drawn pictures is an interactive hit.
    pub(crate) fn skill_hit_under_pointer(&self) -> Option<skills::Hit> {
        let Some(Drawn::Skills(sheet)) = self.drawn(WindowSubject::Skills) else {
            return None;
        };
        sheet.hit(self.input.pointer_gump, &self.resources.gump_atlas)
    }

    /// How tall the current skill tree is; every scroll action clamps to it.
    pub(crate) fn skill_content(&self) -> i32 {
        self.windows.skills.as_ref().map_or(0, |tree| {
            skills::content_height(&self.resources.skill_names, &self.resources.skill_groups, tree)
        })
    }

    /// Apply a click on the skills window.
    pub(crate) fn skill_clicked(&mut self, hit: skills::Hit) {
        let content = self.skill_content();
        let jumped = match hit {
            skills::Hit::Track => match self.drawn(WindowSubject::Skills) {
                Some(Drawn::Skills(sheet)) => Some(sheet.offset_at(self.input.pointer_gump, content)),
                _ => None,
            },
            _ => None,
        };
        let Some(tree) = self.windows.skills.as_mut() else {
            return;
        };
        match hit {
            skills::Hit::Heading(group) => {
                tree.toggle(group);
                let content =
                    skills::content_height(&self.resources.skill_names, &self.resources.skill_groups, tree);
                tree.scroll_to(tree.offset(), content);
            }
            skills::Hit::Up => tree.scroll_by(-skills::STEP, content),
            skills::Hit::Down => tree.scroll_by(skills::STEP, content),
            skills::Hit::Track => {
                if let Some(offset) = jumped {
                    tree.scroll_to(offset, content);
                }
            }
            skills::Hit::Thumb => {}
            skills::Hit::Lock(id) => {
                let shard = self
                    .world
                    .authoritative
                    .view
                    .as_ref()
                    .and_then(|view| view.player.skills.get(&id.0))
                    .map(|line| line.lock)
                    .unwrap_or_default();
                let next = match tree.lock_of(id, shard) {
                    SkillLock::Up => SkillLock::Down,
                    SkillLock::Down => SkillLock::Locked,
                    SkillLock::Locked => SkillLock::Up,
                };
                tree.set_lock(id, next);
                if let Some(link) = self.world.link.as_ref() {
                    link.set_skill_lock(RawSkillId(id.0), next);
                }
            }
            skills::Hit::Use(id) => {
                if let Some(link) = self.world.link.as_ref() {
                    link.use_skill(RawSkillId(id.0));
                }
            }
        }
    }

    /// Follow the pointer with a held scroll thumb.
    pub(crate) fn drag_thumb(&mut self) -> bool {
        if self.windows.held_skill != Some(skills::Hit::Thumb) {
            return false;
        }
        let content = self.skill_content();
        let Some(Drawn::Skills(sheet)) = self.drawn(WindowSubject::Skills) else {
            return false;
        };
        let offset = sheet.offset_at(self.input.pointer_gump, content);
        let Some(tree) = self.windows.skills.as_mut() else {
            return false;
        };
        let before = tree.offset();
        tree.scroll_to(offset, content);
        tree.offset() != before
    }

    /// Take a wheel notch over the skills window, even at either scroll end.
    pub(crate) fn scroll_skills(&mut self, notches: f32) -> bool {
        if self.window_under_pointer() != Some(WindowSubject::Skills) {
            return false;
        }
        let content = self.skill_content();
        if let Some(tree) = self.windows.skills.as_mut() {
            let step = if notches > 0.0 {
                -skills::STEP
            } else {
                skills::STEP
            };
            tree.scroll_by(step, content);
        }
        true
    }
}
