//! One frame's geometry and facts, assembled before anything is drawn:
//! [`assemble_geometry`] is `frame::assemble` and the outline/mobile
//! collectors beside it, folded into one [`FrameGeometry`]; [`FrameFacts`] is
//! [`crate::app::App::frame_facts`]'s own answer, the frame's picks and
//! whether anybody is watching. Neither writes a picture — [`crate::window`]'s
//! atlases and [`crate::presentation`]'s passes do that from what is
//! collected here.

use openshard_client_render::camera::Camera;
use openshard_client_render::cutaway::Cutaway;
use openshard_client_render::frame::{self, Impostor};
use openshard_client_render::mobiles::Mobile;
use openshard_client_render::sprite::{SpriteQuad, split_corners};
use openshard_client_render::{ground, items, light, mobiles, statics};

use crate::crowd::Who;
use crate::picking::{self, Pick, SelectedIdentity};
use crate::window::Screen;
use crate::{graphics, resources, world};

/// Everything `frame::assemble` and its neighbours collected for one frame —
/// see [`assemble_geometry`]'s own doc.
pub(crate) struct FrameGeometry {
    /// The flames, the grid they are occluded by, the ambient, and the two
    /// per-fragment knobs the lighting pass reads — `frame::assemble`'s own.
    pub(crate) lighting: light::Lighting,
    /// The land, back to front.
    pub(crate) quads: Vec<ground::GroundQuad>,
    /// The map's furniture and the server's items, split so a corner static's
    /// two faces carry their own id — see `sprite::split_corners`.
    pub(crate) static_instances: openshard_client_render::sprite::InstanceRows,
    /// The other three quarters of `frame::assemble`'s own
    /// `statics::StaticGeometry` — everything about the map's statics and the
    /// server's items beside their quads, which `static_instances` above
    /// already carries in split form. [`statics::StaticMesh`] and not
    /// `StaticGeometry` itself: that type's own `quads` was spent building
    /// `static_instances`, and a second field here claiming to hold quads
    /// that are not there would be the same untruth `StaticMesh`'s own doc
    /// argues against.
    pub(crate) mesh: statics::StaticMesh,
    /// What a click is holding, placed exactly as the picture placed it.
    pub(crate) select_quads: Vec<SpriteQuad>,
    /// The silhouette the hover ring is grown from.
    pub(crate) outline_quads: Vec<SpriteQuad>,
    /// The same, for what a click is holding.
    pub(crate) held_item_outline: Vec<SpriteQuad>,
    /// The creature silhouette the hover ring is grown from.
    pub(crate) mobile_outline: Vec<SpriteQuad>,
    /// The same, for what a click is holding.
    pub(crate) held_mobile_outline: Vec<SpriteQuad>,
    /// The crowd's own pictures.
    pub(crate) mobile_quads: Vec<SpriteQuad>,
    /// What the frame was asked for, in the same words `frame::Inputs::summary`
    /// gives — kept beside the pictures for the F12 dump. `None` unless a
    /// dump is armed.
    pub(crate) asked_for: Option<String>,
}

/// Everything the world's pictures are built from, out of `frame::assemble`
/// and the outline/mobile collectors beside it — the part of presenting a
/// frame that is genuinely **only** drawing: every parameter here is `&`
/// except `graphics`, which is handed over for the one field
/// `frame::Inputs::bake` writes through (`occlusion_bake`) rather than for
/// `self` as a whole. See `crate::window::ready_atlases`'s doc for the same
/// shape applied to the atlases, and `App::draw_from`'s Step three doc for
/// where this call sits between them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_geometry(
    resources: &resources::Resources,
    graphics: &mut graphics::GraphicsSettings,
    world: &world::WorldState,
    picking: &picking::Picking,
    window: &Screen,
    camera: Camera,
    cutaway: &Cutaway,
    tuning: &light::Tuning,
    lit_item: Option<usize>,
    lit_mobile: Option<usize>,
    held_item: Option<usize>,
    held_mobile: Option<usize>,
    drawn: &[Mobile],
) -> FrameGeometry {
    // Three skies and not two: night, a daylight with a sun in it, and the
    // plain daylight that is the identity — the frame the blit has always
    // copied through untouched. The middle one is a key today; see
    // `App::sunlit`.
    let sky = match (graphics.night, graphics.sunlit) {
        (true, _) => Some(light::NIGHT),
        (false, true) => Some(light::SKYLIGHT),
        // Daylight, where the pass is a copy and no grid is built at all —
        // unless the solids view is on, and then the grid *is* the subject.
        // `Ambient::DAY` flattened is the identity, so the picture under the
        // boxes is the same daylight frame it was; what it buys is that the
        // list drawn is the one the shader would walk, out of the same bake,
        // rather than a second walk of the map made for the view. See
        // `docs/lighting.md` step 23.0.
        (false, false) => graphics.show_solids.then_some(light::Ambient::DAY),
    };
    // And whether a tile's share of it depends on what stands over the tile.
    // Off by default: see `App::sky_field`, and `light::Ambient::flattened`
    // for why the flat one is the baseline rather than a lesser version.
    let sky = match graphics.sky_field {
        true => sky,
        false => sky.map(light::Ambient::flattened),
    };
    // One pick (`lit_item`, at the top of the frame), two effects, and the
    // style decides which of them is asked for. `None` is how each is
    // switched off, so neither pass has a mode to branch on: the hue pass
    // draws an item that is not highlighted, and the silhouette pass is
    // handed an empty list.
    let hued = graphics.highlight_style.hues().then_some(lit_item).flatten();
    let ringed = graphics.highlight_style.rings().then_some(lit_item).flatten();

    // Where the player's own picture will land, asked before the statics
    // are collected rather than after: `frame::assemble` places a tree's
    // canopy against this rectangle, so a leaf that would be drawn over
    // the body is cut instead of hung over it — see
    // `cutaway::hides_foliage_over`. `None` only for the one frame the
    // atlas has not yet grown a frame for this body and group, same as
    // `mobiles::head_anchor`'s own gap.
    let player_rect = mobiles::screen_rect(&world.player, &camera, &window.atlases.mobiles);

    // **One assembly, and the client is a caller of it like any other** —
    // `docs/parity.md`, decision D1. This sequence used to be written out by
    // hand here and in six other places, every one of them free to pass a
    // different cutaway, a different grid or a different clock; each of them
    // did, and the difference was only ever found by reading. Everything a
    // caller may honestly differ on is a field of `frame::Inputs` now, so
    // what this frame is can be compared against what a tool's frame is
    // rather than pieced together from two call sites.
    let inputs = frame::Inputs {
        map: &resources.map,
        items: &world.items,
        camera: &camera,
        tiledata: &resources.tiledata,
        animations: &world.tile_animations,
        cutaway,
        land: &window.atlases.land,
        texmaps: &window.atlases.texmaps,
        // The pictures, which is where an occluder's *facing* comes from: a
        // wall stops a ray only where the ray crosses the side the wall
        // stands on, and only the art says which side that is. One atlas for
        // the grid and for both sprite passes, so they cannot be about two
        // different sets of sprites.
        statics: &window.atlases.statics,
        sky,
        // The sun is a property of the sky and not of the tiles, so it is an
        // input to the frame rather than something walked with them — and
        // never at night, where a second source lighting every roof would
        // undo the whole point of the dark. Where the Light tab put it,
        // which is `light::midday` until somebody moves a slider — see
        // `light::SunTuning`.
        sun: (graphics.sunlit && !graphics.night).then(|| tuning.sun.sun()),
        // And the flame in the player's own hand, which no walk of the map
        // could have found — see `light::carried`. The offset is where the
        // sprite is *actually* drawn this instant, past `at`'s tile, so the
        // pool glides with the walk instead of jumping once a step.
        carried: graphics.lantern.then_some((
            world.player.at,
            mobiles::walked_offset(&world.player),
            world.player.facing,
        )),
        tuning,
        flame_time: world.flame_clock.as_secs_f32(),
        // The blocks of the occlusion grid built for earlier frames. A
        // camera that has moved a tile wants the same five hundred and fifty
        // blocks it wanted last frame bar a handful — see `occlusion::bake`,
        // and `StaticAtlas::revision` for what makes this let go when the
        // atlas learns something new about a graphic.
        bake: Some(&mut graphics.occlusion_bake),
        highlight: hued,
        // The live client meets every sprite against its own boxes whenever
        // it has a grid at all. F10 is not this field: turning the lights off
        // takes the *sky* away, and a frame with no sky has no grid for
        // anything to be met against.
        impostor: Impostor::Met,
        // Which producers this frame draws — the World tab's own boxes. The
        // whole world unless somebody has ticked one off, and the lighting is
        // collected from all of it whatever they tick: see `frame::Draw`.
        draw: graphics.drawing,
        // The view is the looker's, not the world's: a diagnostic draws from
        // the values this frame was lit with, and in daylight those are the
        // ambient and the place attachment — which is exactly what a person
        // checking the place channel wants to see, without having to make it
        // night first.
        view: graphics.light_view,
        // `docs/combat.md`'s D9: the screen greys for the character this
        // client is, not for the offline placeholder — which has no view
        // and so is never a ghost.
        dead: world.view.as_ref().is_some_and(|view| view.player.dead),
        player_rect,
    };
    // **What the frame was asked for**, kept beside the pictures the dump
    // below writes. A picture on its own cannot be reproduced: two frames
    // that differ say nothing about *which* input differed, and the client's
    // arguments were readable until now only by reading this function. Only
    // when a dump is armed — `summary` walks every field and allocates.
    let asked_for = graphics.frame_dump.as_ref().map(|_| inputs.summary());
    let frame::Frame {
        lighting,
        ground: quads,
        statics:
            statics::StaticGeometry {
                quads: static_quads,
                mesh_vertices,
                mesh_rows,
                boxes,
            },
    } = frame::assemble(inputs);
    let mesh = statics::StaticMesh {
        mesh_vertices,
        mesh_rows,
        boxes,
    };

    // What a click is holding, placed exactly as the picture placed it —
    // `statics::selected` is `statics::collect`'s own arithmetic — so the
    // mask lands on the wall's pixels rather than beside them. Empty on
    // every frame with nothing selected, which is what switches the pass off.
    let select_quads = statics::selected(
        &camera,
        &resources.tiledata,
        &world.tile_animations,
        &window.atlases.statics,
        cutaway,
        picking.selected.and_then(SelectedIdentity::as_static),
    );
    // The same quads as the picture's, so the ring lands on the sprite
    // rather than beside it — see `items::outlined`.
    let outline_quads = items::outlined(
        &world.items,
        &camera,
        &resources.tiledata,
        &world.tile_animations,
        &window.atlases.statics,
        cutaway,
        ringed,
    );
    // The held item's own silhouette, through the same function and for
    // the same reason — a second call rather than folding `held_item` into
    // `ringed` above, because the two are drawn with different [`Ring`]s
    // into different masks: this is what a click named, not what the
    // cursor is over.
    let held_item_outline = items::outlined(
        &world.items,
        &camera,
        &resources.tiledata,
        &world.tile_animations,
        &window.atlases.statics,
        cutaway,
        held_item,
    );
    // A corner static's two faces get their own id past this point — see
    // `docs/gbuffer.md` step 4 and `sprite::split_corners`'s own doc.
    let static_instances = split_corners(static_quads);
    // The same two effects for a creature, off the same style switch and
    // the same one-pick-a-frame rule: `lit_mobile` and `lit_item` are never
    // both `Some` (see where they are asked), so exactly one of the four
    // lists below is ever non-empty.
    let mobile_hued = graphics.highlight_style.hues().then_some(lit_mobile).flatten();
    let mobile_ringed = graphics.highlight_style.rings().then_some(lit_mobile).flatten();
    let mobile_outline = mobiles::outlined(
        drawn,
        &camera,
        &window.atlases.mobiles,
        cutaway,
        &resources.equip_conv,
        mobile_ringed,
    );
    // The held mobile's own silhouette — see `held_item_outline` above for
    // why this is a second call and not `mobile_ringed` itself.
    let held_mobile_outline = mobiles::outlined(
        drawn,
        &camera,
        &window.atlases.mobiles,
        cutaway,
        &resources.equip_conv,
        held_mobile,
    );
    let mobile_quads = mobiles::collect(
        drawn,
        &camera,
        &window.atlases.mobiles,
        cutaway,
        &resources.equip_conv,
        mobile_hued,
    );
    FrameGeometry {
        lighting,
        quads,
        static_instances,
        mesh,
        select_quads,
        outline_quads,
        held_item_outline,
        mobile_outline,
        held_mobile_outline,
        mobile_quads,
        asked_for,
    }
}

/// One frame's own facts — see [`crate::app::App::frame_facts`]'s doc for
/// what makes this worth a struct: every field is a pure question of
/// `&self`, asked once against one camera, and nothing here is written back
/// except through the three lines at `App::draw_from`'s call site that read
/// `pick.static_`, `on_mobile` and `on_item` back out again.
pub(crate) struct FrameFacts {
    /// Whether anybody is looking at the window at all — see `App::watched`.
    pub(crate) watched: bool,
    /// The roof cutaway this frame's picks and picture are both drawn under.
    pub(crate) cutaway: Cutaway,
    /// What the cursor is over and what it lit — see [`Pick`].
    pub(crate) pick: Pick,
    /// The crowd as the mobile pass's own atlas already has it packed — the
    /// list `on_mobile` indexes into, and `None` before there is a window at
    /// all.
    pub(crate) drawn_mobiles: Option<Vec<(Who, Mobile)>>,
    /// The creature the cursor is over, indexing `drawn_mobiles` — the
    /// unfiltered form of [`Pick::mobile`], kept here because
    /// `App::draw_from` reads it back into `self.picking` regardless of the
    /// highlight mode: what a click selects is not a question about lighting.
    pub(crate) on_mobile: Option<usize>,
    /// The item the cursor is over, indexing `self.world.items` — the
    /// unfiltered form of [`Pick::item`], for the same reason.
    pub(crate) on_item: Option<usize>,
    /// What a click is holding, turned back into an index into
    /// `drawn_mobiles`.
    pub(crate) held_mobile: Option<usize>,
    /// What a click is holding, turned back into an index into
    /// `self.world.item_serials`.
    pub(crate) held_item: Option<usize>,
}
